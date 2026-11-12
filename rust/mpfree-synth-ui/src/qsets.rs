//! Q-Link sets: named sets of four sound controls that a sound carries, so
//! the four Q-Links play it from any screen without opening its editor.
//!
//! A [`QSet`] is a name and four parameter ids (`None` for a knob left
//! free). [`factory`] gives each engine's curated sets, in the order a
//! player reaches for them (Poly: Tone, Env, Filter env, Motion, Mix, Osc).
//! Any other node, such as a hosted plugin, gets one set per Synth tab
//! page, holding what that page's Q-Links turn.
//!
//! **Drums** follow the pad hit last: their sets name pad 1's parameters,
//! and [`on_pad`] moves them to another pad. A knob learned on another pad
//! is kept for pad 1 ([`pad_relative`]), so it follows the pad too.
//!
//! **Saved with the sound:** [`save`] writes one `qset = ...` line per set
//! for a preset file, and [`load`] reads them back from a whole preset
//! file, ignoring its other lines. An id the node no longer has (or one
//! that is a meter) loads as a free knob, so a preset outlives a parameter
//! taken away.
//!
//! **On the Q-Links:** [`to_banks`] makes the sets into `mpfree-mod` banks
//! for `Bindings::set_page_sets`, and after a learn
//! (`Bindings::take_sets_edited`) [`from_banks`] reads them back to save
//! with the sound. Which button steps them and where the cards sit is the
//! app's layout.

use mpfree_mod::{Bank, Binding};
use mpfree_node::{NodeInfo, ParamInfo};
use mpfree_param::{Owner, ParamAddr};

use crate::pages::{plan, qlink_targets};

/// Knobs in a set: the MPC One's four Q-Links.
pub const KNOBS: usize = 4;
/// The most sets a sound carries.
pub const MAX_SETS: usize = 8;
/// The longest set name, in characters, so "Poly · Filter env" fits the
/// column's title.
pub const MAX_NAME: usize = 12;
/// A preset file's key for one set.
pub const KEY: &str = "qset";

/// One set: its name and what each knob turns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QSet {
    pub name: String,
    /// Parameter ids, knob 1 first; `None` leaves the knob free.
    pub params: [Option<u16>; KNOBS],
}

impl QSet {
    pub fn new(name: &str, params: [u16; KNOBS]) -> QSet {
        QSet {
            name: name.to_string(),
            params: params.map(Some),
        }
    }
}

/// A curated set in an engine's table.
struct Curated {
    name: &'static str,
    params: [u16; KNOBS],
}

const fn set(name: &'static str, params: [u16; KNOBS]) -> Curated {
    Curated { name, params }
}

use mpfree_synth::params as pp;

/// Poly: the filter first (what a player turns most), then the envelopes,
/// the movement and the oscillators.
const POLY: &[Curated] = &[
    set(
        "Tone",
        [pp::CUTOFF, pp::RESONANCE, pp::FENV_AMOUNT, pp::DRIVE],
    ),
    set("Env", [pp::AMP_A, pp::AMP_D, pp::AMP_S, pp::AMP_R]),
    set(
        "Filter env",
        [pp::FENV_A, pp::FENV_D, pp::FENV_S, pp::FENV_R],
    ),
    set(
        "Motion",
        [
            pp::LFO_RATE,
            pp::LFO_CUTOFF,
            pp::LFO_PITCH,
            pp::UNISON_DETUNE,
        ],
    ),
    set(
        "Mix",
        [
            pp::OSC1_LEVEL,
            pp::OSC2_LEVEL,
            pp::SUB_LEVEL,
            pp::NOISE_LEVEL,
        ],
    ),
    set(
        "Osc",
        [pp::OSC2_SEMI, pp::OSC2_FINE, pp::OSC1_PW, pp::OSC2_PW],
    ),
];

use mpfree_synth::fm_params::{self as fp, op, op_id};

/// FM: the algorithm and feedback, then the operators' output levels (the
/// brightness of a modulator, the loudness of a carrier) and frequency
/// ratios, then the LFO.
const FM: &[Curated] = &[
    set(
        "Algo",
        [fp::ALGORITHM, fp::FEEDBACK, fp::TRANSPOSE, fp::LEVEL],
    ),
    set(
        "Levels 1-4",
        [
            op_id(1, op::LEVEL),
            op_id(2, op::LEVEL),
            op_id(3, op::LEVEL),
            op_id(4, op::LEVEL),
        ],
    ),
    set(
        "Ops 5-6",
        [
            op_id(5, op::LEVEL),
            op_id(6, op::LEVEL),
            op_id(5, op::COARSE),
            op_id(6, op::COARSE),
        ],
    ),
    set(
        "Ratios 1-4",
        [
            op_id(1, op::COARSE),
            op_id(2, op::COARSE),
            op_id(3, op::COARSE),
            op_id(4, op::COARSE),
        ],
    ),
    set(
        "Mod",
        [fp::LFO_SPEED, fp::LFO_PMD, fp::LFO_AMD, fp::LFO_DELAY],
    ),
];

use mpfree_synth::wavetable::params::{self as wp, osc_id};

/// Wavetable: where each oscillator reads its table first, then the
/// filter, the envelopes, the movement and the mix.
const WT: &[Curated] = &[
    set(
        "Tables",
        [
            osc_id(0, wp::POSITION),
            osc_id(1, wp::POSITION),
            osc_id(0, wp::WARP),
            osc_id(1, wp::WARP),
        ],
    ),
    set(
        "Tone",
        [wp::CUTOFF, wp::RESONANCE, wp::MENV_CUTOFF, wp::DRIVE],
    ),
    set("Env", [wp::AMP_A, wp::AMP_D, wp::AMP_S, wp::AMP_R]),
    set(
        "Mod env",
        [wp::MENV_A, wp::MENV_D, wp::MENV_POS_A, wp::MENV_POS_B],
    ),
    set(
        "Motion",
        [wp::LFO_RATE, wp::LFO_POS_A, wp::LFO_POS_B, wp::LFO_CUTOFF],
    ),
    set(
        "Mix",
        [
            osc_id(0, wp::OSC_LEVEL),
            osc_id(1, wp::OSC_LEVEL),
            wp::SUB_LEVEL,
            wp::UNISON_DETUNE,
        ],
    ),
];

use mpfree_synth::pluck_params as kp;

/// Pluck: the string, the mallet (for the mallet model), and how it plays.
const PLUCK: &[Curated] = &[
    set(
        "String",
        [kp::DECAY, kp::BRIGHTNESS, kp::PICK, kp::POSITION],
    ),
    set(
        "Mallet",
        [kp::HARDNESS, kp::STRIKE, kp::STRETCH, kp::HIGH_DAMP],
    ),
    set("Play", [kp::RELEASE, kp::VEL_TONE, kp::SPREAD, kp::LEVEL]),
];

use mpfree_synth::drum_params::{field as df, pad_id, PADS, STRIDE};

/// Drums, on pad 1 ([`on_pad`] moves them): the hit, its body, its mix.
const DRUMS: &[Curated] = &[
    set(
        "Hit",
        [
            pad_id(0, df::TUNE),
            pad_id(0, df::DECAY),
            pad_id(0, df::TONE),
            pad_id(0, df::SNAP),
        ],
    ),
    set(
        "Body",
        [
            pad_id(0, df::NOISE),
            pad_id(0, df::SWEEP),
            pad_id(0, df::SWEEP_TIME),
            pad_id(0, df::DRIVE),
        ],
    ),
    set(
        "Mix",
        [
            pad_id(0, df::LEVEL),
            pad_id(0, df::PAN),
            pad_id(0, df::VEL_TONE),
            pad_id(0, df::VEL_LEVEL),
        ],
    ),
];

const DRUMS_ID: &str = "mpfree.drums";

fn curated(id: &str) -> Option<&'static [Curated]> {
    match id {
        "mpfree.poly" => Some(POLY),
        "mpfree.fm" => Some(FM),
        "mpfree.wavetable" => Some(WT),
        "mpfree.pluck" => Some(PLUCK),
        DRUMS_ID => Some(DRUMS),
        _ => None,
    }
}

/// A sound's sets as the factory made them: the engine's curated sets, or
/// one per Synth tab page for any other node (the first [`MAX_SETS`]
/// pages that have a knob to turn).
pub fn factory(info: &NodeInfo, params: &[ParamInfo]) -> Vec<QSet> {
    if let Some(sets) = curated(info.id) {
        return sets.iter().map(|c| QSet::new(c.name, c.params)).collect();
    }
    plan(info, params)
        .iter()
        .filter_map(|page| {
            let mut q = [None; KNOBS];
            for (k, &i) in qlink_targets(page, params).iter().take(KNOBS).enumerate() {
                q[k] = Some(params[i].id);
            }
            q.iter().any(Option::is_some).then(|| QSet {
                name: clip_name(&page.name),
                params: q,
            })
        })
        .take(MAX_SETS)
        .collect()
}

/// The sets on pad `pad` (0 to 15) for Drums, which name pad 1's
/// parameters; any other node's sets as they are.
pub fn on_pad(info: &NodeInfo, sets: &[QSet], pad: usize) -> Vec<QSet> {
    if info.id != DRUMS_ID {
        return sets.to_vec();
    }
    let pad = pad.min(PADS - 1);
    sets.iter()
        .map(|s| QSet {
            name: s.name.clone(),
            params: s
                .params
                .map(|p| p.map(|id| if id < STRIDE { pad_id(pad, id) } else { id })),
        })
        .collect()
}

/// The id to keep in a set for a parameter learned on screen: for Drums,
/// a pad's parameter as pad 1's (so the knob follows the pad hit last);
/// any other id as it is.
pub fn pad_relative(info: &NodeInfo, id: u16) -> u16 {
    if info.id == DRUMS_ID && usize::from(id) < PADS * usize::from(STRIDE) {
        id % STRIDE
    } else {
        id
    }
}

/// The sets as preset-file lines, one per set: `qset = 31 32 - 33 Tone`
/// (the four ids, `-` for a free knob, then the name).
pub fn save(sets: &[QSet]) -> String {
    let mut s = String::new();
    for set in sets.iter().take(MAX_SETS) {
        s.push_str(KEY);
        s.push_str(" =");
        for p in set.params {
            match p {
                Some(id) => s.push_str(&format!(" {id}")),
                None => s.push_str(" -"),
            }
        }
        let name = clip_name(&set.name);
        if !name.is_empty() {
            s.push(' ');
            s.push_str(&name);
        }
        s.push('\n');
    }
    s
}

/// The sets in a preset file's text (other lines are skipped), checked
/// against the node's parameters: an id it doesn't have, or a meter,
/// loads as a free knob, and a set with no name is called "Set N". `None`
/// when the text has no `qset` lines (use [`factory`]).
pub fn load(text: &str, params: &[ParamInfo]) -> Option<Vec<QSet>> {
    let mut sets = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != KEY {
            continue;
        }
        if sets.len() == MAX_SETS {
            break;
        }
        let mut rest = value.trim();
        let mut q = [None; KNOBS];
        for slot in &mut q {
            let (word, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            rest = after.trim_start();
            *slot = word
                .parse::<u16>()
                .ok()
                .filter(|&id| params.iter().any(|p| p.id == id && !p.readonly));
        }
        let name = clip_name(rest);
        sets.push(QSet {
            name: if name.is_empty() {
                format!("Set {}", sets.len() + 1)
            } else {
                name
            },
            params: q,
        });
    }
    (!sets.is_empty()).then_some(sets)
}

/// The sets as Q-Link banks for `Bindings::set_page_sets`: one bank per
/// set, with the set's name, each knob turning its parameter on `owner`
/// (the sound's instrument, `Owner::Inst(track)`) over its whole range and
/// named as the parameter is. A free knob, an id the node doesn't have and
/// a meter are free knobs. For Drums, pass the sets [`on_pad`] gives for
/// the pad hit last.
pub fn to_banks(sets: &[QSet], params: &[ParamInfo], owner: Owner) -> Vec<Bank> {
    sets.iter()
        .take(MAX_SETS)
        .map(|s| Bank {
            name: s.name.clone(),
            knobs: s.params.map(|p| {
                let id = p?;
                let info = params.iter().find(|i| i.id == id && !i.readonly)?;
                Some(Binding::new(ParamAddr::new(owner, id), *info, info.name))
            }),
        })
        .collect()
}

/// The banks back as sets, to save with the sound after a learn changed
/// them (`Bindings::take_sets_edited`): the inverse of [`to_banks`]. Only
/// the sound's own parameters travel with its preset, so a knob learned to
/// anything not on `owner` (an effect, a macro, another track's sound), or
/// to an id the node doesn't have, is saved free. A Drums pad's parameter
/// is kept as pad 1's ([`pad_relative`]), so it follows the pad. A knob's
/// part-range is not saved: it loads over the whole range.
pub fn from_banks(
    info: &NodeInfo,
    banks: &[Bank],
    params: &[ParamInfo],
    owner: Owner,
) -> Vec<QSet> {
    banks
        .iter()
        .take(MAX_SETS)
        .enumerate()
        .map(|(n, b)| {
            let name = clip_name(&b.name);
            QSet {
                name: if name.is_empty() {
                    format!("Set {}", n + 1)
                } else {
                    name
                },
                params: b.knobs.each_ref().map(|k| {
                    let t = k.as_ref()?.target;
                    let ok =
                        t.owner == owner && params.iter().any(|i| i.id == t.param && !i.readonly);
                    ok.then(|| pad_relative(info, t.param))
                }),
            }
        })
        .collect()
}

/// A name a set can carry: one line of printable characters, trimmed, at
/// most [`MAX_NAME`] characters.
fn clip_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clean
        .trim()
        .chars()
        .take(MAX_NAME)
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_mod::Bindings;
    use mpfree_node::{Curve, Kind, Node, Unit};
    use mpfree_synth::{Drums, Fm, Pluck, Poly, Wavetable};

    fn engines() -> Vec<Box<dyn Node>> {
        vec![
            Box::new(Poly::new(48_000)),
            Box::new(Fm::new(48_000)),
            Box::new(Wavetable::new(48_000)),
            Box::new(Pluck::new(48_000)),
            Box::new(Drums::new(48_000)),
        ]
    }

    #[test]
    fn every_engine_has_curated_sets_of_real_knobs() {
        for e in engines() {
            let info = e.info();
            let params = e.params();
            let sets = factory(info, params);
            assert!(
                (3..=MAX_SETS).contains(&sets.len()),
                "{}: {} sets",
                info.name,
                sets.len()
            );
            for s in &sets {
                assert!(
                    !s.name.is_empty() && s.name.chars().count() <= MAX_NAME,
                    "{}: name {:?}",
                    info.name,
                    s.name
                );
                for (k, p) in s.params.iter().enumerate() {
                    let id = p.unwrap_or_else(|| panic!("{} {}: knob {k} free", info.name, s.name));
                    let p = params
                        .iter()
                        .find(|p| p.id == id)
                        .unwrap_or_else(|| panic!("{} {}: no id {id}", info.name, s.name));
                    assert!(
                        !p.readonly,
                        "{} {}: {} is a meter",
                        info.name, s.name, p.name
                    );
                    assert!(
                        !matches!(p.curve, Curve::Toggle | Curve::Choice(_)),
                        "{} {}: {} is a switch or a choice, for the screen",
                        info.name,
                        s.name,
                        p.name
                    );
                    assert!(
                        s.params[..k].iter().all(|q| *q != Some(id)),
                        "{} {}: {} twice",
                        info.name,
                        s.name,
                        p.name
                    );
                }
            }
            let mut names: Vec<&str> = sets.iter().map(|s| s.name.as_str()).collect();
            names.sort();
            names.dedup();
            assert_eq!(
                names.len(),
                sets.len(),
                "{}: two sets share a name",
                info.name
            );
        }
    }

    #[test]
    fn poly_starts_on_its_filter() {
        let p = Poly::new(48_000);
        let sets = factory(p.info(), p.params());
        let names: Vec<&str> = sets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Tone", "Env", "Filter env", "Motion", "Mix", "Osc"]);
        assert_eq!(
            sets[0].params,
            [pp::CUTOFF, pp::RESONANCE, pp::FENV_AMOUNT, pp::DRIVE].map(Some)
        );
    }

    #[test]
    fn drum_sets_follow_the_pad() {
        let d = Drums::new(48_000);
        let info = d.info();
        let sets = factory(info, d.params());
        let pad6 = on_pad(info, &sets, 5);
        assert_eq!(pad6[0].params[1], Some(pad_id(5, df::DECAY)));
        assert_eq!(
            on_pad(info, &sets, 99)[0].params[0],
            Some(pad_id(15, df::TUNE))
        );
        // The kit's own controls stay put, and so does a free knob.
        let mut kit = sets[0].clone();
        kit.params[2] = Some(mpfree_synth::drum_params::LEVEL);
        kit.params[3] = None;
        let moved = &on_pad(info, &[kit], 7)[0];
        assert_eq!(moved.params[2], Some(mpfree_synth::drum_params::LEVEL));
        assert_eq!(moved.params[3], None);
        // A knob learned on pad 9 is kept as pad 1's and follows the pad.
        assert_eq!(pad_relative(info, pad_id(8, df::DRIVE)), df::DRIVE);
        assert_eq!(
            pad_relative(info, mpfree_synth::drum_params::BASE_NOTE),
            mpfree_synth::drum_params::BASE_NOTE
        );
        // Other engines' ids never move.
        let p = Poly::new(48_000);
        assert_eq!(pad_relative(p.info(), 5), 5);
        let ps = factory(p.info(), p.params());
        assert_eq!(on_pad(p.info(), &ps, 3), ps);
    }

    #[test]
    fn sets_save_and_load_with_a_preset() {
        for e in engines() {
            let (info, params) = (e.info(), e.params());
            let sets = factory(info, params);
            let text = format!(
                "engine = {}\nname = Test\n{}cutoff = 0.5\n",
                info.id,
                save(&sets)
            );
            assert_eq!(load(&text, params), Some(sets), "{}", info.name);
        }
        assert_eq!(
            load(
                "engine = mpfree.poly\nname = Bare\n",
                Poly::new(48_000).params()
            ),
            None
        );
    }

    #[test]
    fn a_rebound_set_keeps_its_free_knobs_and_name() {
        let p = Poly::new(48_000);
        let mut sets = factory(p.info(), p.params());
        sets[1].name = "My  env".to_string();
        sets[1].params[2] = None;
        sets[1].params[3] = Some(pp::GLIDE);
        let text = save(&sets);
        assert!(text.contains("qset = 50 51 - 72 My  env\n"), "{text}");
        assert_eq!(load(&text, p.params()), Some(sets));
    }

    #[test]
    fn loading_tolerates_old_and_odd_presets() {
        let params = Poly::new(48_000).params();
        let text = "qset = 31 999 90 x\n\
                    qset=32\n\
                    qset = 31 32 33 34 A name far too long for the column\n\
                    qset = 31 32 33 34 Bad\u{7}name\n";
        let sets = load(text, params).unwrap();
        // 999 is not a Poly id, 90 is its voice meter, x is not a number.
        assert_eq!(sets[0].params, [Some(pp::CUTOFF), None, None, None]);
        assert_eq!(sets[0].name, "Set 1");
        assert_eq!(sets[1].params, [Some(pp::RESONANCE), None, None, None]);
        assert_eq!(sets[1].name, "Set 2");
        assert_eq!(sets[2].name, "A name far t");
        assert_eq!(sets[3].name, "Bad name");
        let many: String = (0..20)
            .map(|i| format!("qset = 31 32 33 34 S{i}\n"))
            .collect();
        assert_eq!(load(&many, params).unwrap().len(), MAX_SETS);
    }

    #[test]
    fn other_nodes_get_a_set_per_page() {
        static INFO: NodeInfo = NodeInfo {
            id: "test.plugin",
            name: "Plugin",
            kind: Kind::Instrument,
        };
        let knob = |id: u16, name: &'static str, group: &'static str| ParamInfo {
            id,
            name,
            group,
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: Unit::None,
            curve: Curve::Linear,
            readonly: false,
        };
        let mut params: Vec<ParamInfo> = (0..6).map(|i| knob(i, "Knob", "Main")).collect();
        params.extend((10..30).map(|i| knob(i, "Fx", "Effects")));
        params.push(ParamInfo {
            readonly: true,
            ..knob(40, "Meter", "Meters")
        });
        let sets = factory(&INFO, &params);
        assert_eq!(sets[0].name, "Main");
        assert_eq!(sets[0].params, [0, 1, 2, 3].map(Some));
        assert!(
            sets.iter().all(|s| s.name != "Meters"),
            "a page of meters gives no set"
        );
        assert!(sets.len() <= MAX_SETS);
    }

    #[test]
    fn sets_go_to_the_qlinks_and_come_back() {
        let owner = Owner::Inst(2);
        for e in engines() {
            let (info, params) = (e.info(), e.params());
            let sets = factory(info, params);
            let banks = to_banks(&sets, params, owner);
            assert_eq!(banks.len(), sets.len(), "{}", info.name);
            for (s, b) in sets.iter().zip(&banks) {
                assert_eq!(b.name, s.name, "{}", info.name);
                for (p, k) in s.params.iter().zip(&b.knobs) {
                    let pi = params.iter().find(|i| Some(i.id) == *p).unwrap();
                    let k = k.as_ref().expect("a curated knob is bound");
                    assert_eq!(k.target, ParamAddr::new(owner, pi.id));
                    assert_eq!((k.info, k.label.as_str()), (*pi, pi.name));
                    assert_eq!((k.from, k.to, k.colour), (0.0, 1.0, None));
                }
            }
            assert_eq!(
                from_banks(info, &banks, params, owner),
                sets,
                "{}",
                info.name
            );
        }
        // A free knob and an id the node doesn't have are free knobs.
        let p = Poly::new(48_000);
        let odd = QSet {
            name: "Odd".to_string(),
            params: [Some(9_999), None, Some(pp::CUTOFF), None],
        };
        let banks = to_banks(&[odd], p.params(), owner);
        let bound: Vec<bool> = banks[0].knobs.iter().map(Option::is_some).collect();
        assert_eq!(bound, [false, false, true, false]);
        // Drums: the sets on pad 6 come back as pad 1's.
        let d = Drums::new(48_000);
        let sets = factory(d.info(), d.params());
        let banks = to_banks(&on_pad(d.info(), &sets, 5), d.params(), owner);
        assert_eq!(
            banks[0].knobs[0].as_ref().map(|k| k.target.param),
            Some(pad_id(5, df::TUNE))
        );
        assert_eq!(from_banks(d.info(), &banks, d.params(), owner), sets);
    }

    #[test]
    fn a_learn_on_the_qlinks_saves_with_the_sound_when_it_can() {
        let p = Poly::new(48_000);
        let (info, params) = (p.info(), p.params());
        let owner = Owner::Inst(0);
        let sets = factory(info, params);
        let mut b = Bindings::with_macros(&[]);
        b.set_page_sets("synth", "Poly", to_banks(&sets, params, owner));
        assert_eq!(b.title(), "Poly · Tone");
        let w = b.turn(0, 1, false).expect("knob 1 moves the cutoff");
        assert_eq!(w.addr, ParamAddr::new(owner, pp::CUTOFF));
        // Glide learned to knob 4 of Env goes with the sound.
        b.next_set();
        let glide = *params.iter().find(|i| i.id == pp::GLIDE).unwrap();
        b.learn(ParamAddr::new(owner, pp::GLIDE), glide, glide.name);
        assert_eq!(b.turn(3, 1, false), None);
        assert!(b.take_sets_edited());
        let mut want = sets.clone();
        want[1].params[3] = Some(pp::GLIDE);
        let saved = from_banks(info, b.page_sets(), params, owner);
        assert_eq!(saved, want);
        assert_eq!(load(&save(&saved), params), Some(want.clone()));
        // Another track's sound or a macro can't travel with the preset.
        b.learn(ParamAddr::new(Owner::Inst(1), pp::CUTOFF), glide, "Cutoff");
        b.turn(1, 1, false);
        b.learn(ParamAddr::macro_value(0), glide, "Wash");
        b.turn(2, 1, false);
        assert!(b.take_sets_edited());
        want[1].params[1] = None;
        want[1].params[2] = None;
        assert_eq!(from_banks(info, b.page_sets(), params, owner), want);
    }
}
