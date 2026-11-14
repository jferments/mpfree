//! Effects with modes: near-duplicate effects shown as one effect whose
//! Mode picks the sound (Jay, 14 Nov 2026). Reverb's Room, Plate and
//! Spring are three engines, its Hall, Cloud and Echoes one engine (the
//! Cloud reverb) with its Character set; Drive's ten modes are the Drive
//! and Character engines with their Mode set.
//!
//! Nothing underneath changes: every engine keeps its node id, its
//! parameters and its presets, an insert saves its engine's id as before,
//! and two inserts of one family (a Room, then a Plate) are two separate
//! effects. A mode is an engine plus a few fixed values, which the chain
//! editor hides, since the Mode chooser stands for them.
//!
//! Switching modes on an insert: within one engine (Hall to Cloud, Tube
//! to Fuzz) only the fixed values change; across engines (Room to Plate)
//! the app builds the new engine with [`switch_values`], which keeps the
//! controls the two share by name.

use mpfree_node::{Curve, Node, ParamInfo};

use crate::ids::{character, cloud, drive, tremolo};
use crate::registry::{self, Category, FxEntry};
use crate::Preset;

/// One mode of a family: an engine and the values that make it this mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub label: &'static str,
    /// The engine's node id, in the registry.
    pub id: &'static str,
    /// Parameters the mode sets and the chain editor hides.
    pub fixed: &'static [(u16, f32)],
}

/// Near-duplicate effects shown as one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Family {
    pub name: &'static str,
    /// The picker's line about it.
    pub about: &'static str,
    /// In the order the Mode chooser shows them. An engine's modes are
    /// listed together, the one its defaults make first, so a new insert
    /// of the engine (or a preset that leaves the fixed values alone) is
    /// that mode.
    pub modes: &'static [Mode],
}

const fn m(label: &'static str, id: &'static str, fixed: &'static [(u16, f32)]) -> Mode {
    Mode { label, id, fixed }
}

const ROOM: &str = "mpfree.fx.room";
const PLATE: &str = "mpfree.fx.plate";
const SPRING: &str = "mpfree.fx.spring";
const CLOUD: &str = "mpfree.fx.cloud";
const DRIVE: &str = "mpfree.fx.drive";
const CHARACTER: &str = "mpfree.fx.character";
const TREMOLO: &str = "mpfree.fx.tremolo";

/// Every family, in no particular order (the picker shows a family where
/// its first engine sits in the registry).
pub static FAMILIES: &[Family] = &[
    Family {
        name: "Reverb",
        about: "Room, plate, spring, hall, cloud",
        modes: &[
            m("Room", ROOM, &[]),
            m("Plate", PLATE, &[]),
            m("Spring", SPRING, &[]),
            m("Hall", CLOUD, &[(cloud::CHARACTER, 2.0)]),
            m("Cloud", CLOUD, &[(cloud::CHARACTER, 3.0)]),
            m("Echoes", CLOUD, &[(cloud::CHARACTER, 4.0)]),
        ],
    },
    Family {
        name: "Delay",
        about: "Digital, tape or four taps",
        modes: &[
            m("Digital", "mpfree.fx.delay", &[]),
            m("Tape", "mpfree.fx.tape_delay", &[]),
            m("Multi-tap", "mpfree.fx.multitap", &[]),
        ],
    },
    Family {
        name: "Drive",
        about: "Soft to fuzz, console to tape",
        modes: &[
            m("Soft", DRIVE, &[(drive::MODE, 0.0)]),
            m("Hard", DRIVE, &[(drive::MODE, 1.0)]),
            m("Tube", DRIVE, &[(drive::MODE, 2.0)]),
            m("Foldback", DRIVE, &[(drive::MODE, 3.0)]),
            m("Fuzz", DRIVE, &[(drive::MODE, 4.0)]),
            m("Rectify", DRIVE, &[(drive::MODE, 5.0)]),
            m(
                "Console",
                CHARACTER,
                &[(character::MODE, character::CONSOLE)],
            ),
            m("Tape", CHARACTER, &[(character::MODE, character::TAPE)]),
            m("Iron", CHARACTER, &[(character::MODE, character::IRON)]),
            m("Soften", CHARACTER, &[(character::MODE, character::SOFTEN)]),
        ],
    },
    Family {
        name: "Filter",
        about: "Low, high, band, notch, or one knob",
        modes: &[
            m("Multi", "mpfree.fx.filter", &[]),
            m("DJ", "mpfree.fx.dj_filter", &[]),
        ],
    },
    Family {
        name: "Tremolo",
        about: "Wobble, chop or side to side",
        modes: &[
            m("Volume", TREMOLO, &[(tremolo::MODE, 0.0)]),
            m("Harmonic", TREMOLO, &[(tremolo::MODE, 1.0)]),
            m("Auto-pan", "mpfree.fx.autopan", &[]),
        ],
    },
];

/// The family an engine belongs to.
pub fn family(id: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.modes.iter().any(|m| m.id == id))
}

/// Which family and mode an insert of engine `id` is, reading its values
/// with `value`. The mode is `None` when its values make none of the
/// family's modes (a Cloud reverb from before families, set to its own
/// Plate or Room).
pub fn mode_of(id: &str, value: impl Fn(u16) -> f32) -> Option<(&'static Family, Option<usize>)> {
    let f = family(id)?;
    let mode = f
        .modes
        .iter()
        .position(|m| m.id == id && m.fixed.iter().all(|&(p, v)| (value(p) - v).abs() < 0.5));
    Some((f, mode))
}

/// [`mode_of`] for a node.
pub fn node_mode(n: &dyn Node) -> Option<(&'static Family, Option<usize>)> {
    mode_of(n.info().id, |p| n.param(p))
}

/// The parameters the chain editor hides on this insert: its mode's fixed
/// ones (none when it is in no mode, so nothing is out of reach).
pub fn hidden(id: &str, value: impl Fn(u16) -> f32) -> &'static [(u16, f32)] {
    match mode_of(id, value) {
        Some((f, Some(i))) => f.modes[i].fixed,
        _ => &[],
    }
}

/// What a musician calls an insert: "Reverb · Plate" for one in a family's
/// mode, else the engine's own name ("Cloud reverb", "Compressor").
pub fn insert_name(n: &dyn Node) -> String {
    match node_mode(n) {
        Some((f, Some(i))) => format!("{} · {}", f.name, f.modes[i].label),
        _ => n.info().name.to_string(),
    }
}

/// The id the picker and the favourites and recent lists use for an
/// engine: its family's first engine, so every member shows as the one
/// family tile; any other id as it is.
pub fn tile_id(id: &str) -> &str {
    match family(id) {
        Some(f) => f.modes[0].id,
        None => id,
    }
}

/// One tile of the picker: a family, or an effect in none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    /// What adding it makes: the family's first mode, or the effect.
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub category: Category,
}

/// The tiles a strip's picker offers (Master safety only on the master):
/// the registry's effects in its order, each family once, where its first
/// engine in that order sits.
pub fn tiles(master: bool) -> Vec<Tile> {
    let mut out: Vec<Tile> = Vec::new();
    let mut seen: Vec<&'static str> = Vec::new();
    for e in registry::offered(master) {
        match family(e.id) {
            Some(f) if seen.contains(&f.name) => {}
            Some(f) => {
                seen.push(f.name);
                out.push(Tile {
                    id: f.modes[0].id,
                    name: f.name,
                    about: f.about,
                    category: e.category,
                });
            }
            None => out.push(Tile {
                id: e.id,
                name: e.name,
                about: e.about,
                category: e.category,
            }),
        }
    }
    out
}

/// One row of a family's presets list: the mode it plays in, and the
/// preset as the picker adds it (`id` and its index in that engine's
/// presets).
#[derive(Clone, Copy)]
pub struct FamilyPreset {
    pub mode: usize,
    pub id: &'static str,
    pub index: usize,
    pub preset: &'static Preset,
}

/// A family's factory presets under its modes, in mode order. A preset
/// that leaves a fixed value alone plays in its engine's first mode (the
/// engine's default); one that makes no mode (the Cloud reverb's own
/// Plate and Room) is left out, since the family's Plate and Room modes
/// have their own.
pub fn presets(f: &Family) -> Vec<FamilyPreset> {
    let mut out = Vec::new();
    for (mi, mode) in f.modes.iter().enumerate() {
        let first = f.modes.iter().position(|m| m.id == mode.id) == Some(mi);
        let entry: Option<&'static FxEntry> = registry::entry(mode.id);
        for (index, preset) in entry.map_or(&[][..], |e| e.presets).iter().enumerate() {
            let fits = mode.fixed.iter().all(|&(p, v)| {
                match preset.values.iter().find(|(q, _)| *q == p) {
                    Some(&(_, pv)) => (pv - v).abs() < 0.5,
                    None => first,
                }
            });
            if fits {
                out.push(FamilyPreset {
                    mode: mi,
                    id: mode.id,
                    index,
                    preset,
                });
            }
        }
    }
    out
}

/// The values a new engine starts with when an insert switches to `to`:
/// the controls it shares with the old engine (`from`, read with `value`)
/// by name, unit and kind (a choice only between identical lists), each
/// clamped to its new range, then the mode's fixed values. The old
/// engine's fixed values are not carried.
pub fn switch_values(
    from: &[ParamInfo],
    value: impl Fn(u16) -> f32,
    from_fixed: &[(u16, f32)],
    to: &[ParamInfo],
    mode: &Mode,
) -> Vec<(u16, f32)> {
    let mut out: Vec<(u16, f32)> = Vec::new();
    for t in to.iter().filter(|t| !t.readonly) {
        if mode.fixed.iter().any(|&(p, _)| p == t.id) {
            continue;
        }
        let Some(f) = from
            .iter()
            .find(|f| carries(f, t) && !from_fixed.iter().any(|&(p, _)| p == f.id))
        else {
            continue;
        };
        out.push((t.id, t.clamp(value(f.id))));
    }
    out.extend_from_slice(mode.fixed);
    out
}

/// The parameter of `to` that a switch carries `from`'s parameter `id`
/// into, by [`switch_values`]' rule: where a macro or a mod row on it goes.
pub fn carried_id(from: &[ParamInfo], id: u16, to: &[ParamInfo]) -> Option<u16> {
    let f = from.iter().find(|f| f.id == id)?;
    to.iter().find(|t| carries(f, t)).map(|t| t.id)
}

fn carries(f: &ParamInfo, t: &ParamInfo) -> bool {
    !f.readonly
        && !t.readonly
        && f.name == t.name
        && f.unit == t.unit
        && same_kind(f.curve, t.curve)
}

/// A switch across engines, as the app hands it to whatever builds the
/// new engine (off the audio thread): what it switches from, and the
/// values the new engine had when this insert last left it, if it did.
#[derive(Clone, Debug)]
pub struct Switch {
    pub from: &'static [ParamInfo],
    /// The old engine's values, every parameter.
    pub values: Vec<(u16, f32)>,
    pub from_fixed: &'static [(u16, f32)],
    pub mode: &'static Mode,
    /// The new engine's values from before (switching back restores
    /// them); its mode's fixed values still win.
    pub remembered: Option<Vec<(u16, f32)>>,
}

impl Switch {
    /// Leaving `n` (in its family's mode `from_fixed` is the fixed values
    /// of) for `mode`.
    pub fn new(n: &dyn Node, from_fixed: &'static [(u16, f32)], mode: &'static Mode) -> Switch {
        Switch {
            from: n.params(),
            values: values(n),
            from_fixed,
            mode,
            remembered: None,
        }
    }

    /// Sets the new engine up: the remembered values, or the controls it
    /// shares with the old one, then the mode's fixed values.
    pub fn apply(&self, to: &mut dyn Node) {
        let set: Vec<(u16, f32)> = match &self.remembered {
            Some(r) => r.iter().chain(self.mode.fixed).copied().collect(),
            None => {
                let value = |p: u16| {
                    self.values
                        .iter()
                        .find(|(q, _)| *q == p)
                        .map_or(0.0, |v| v.1)
                };
                switch_values(self.from, value, self.from_fixed, to.params(), self.mode)
            }
        };
        for (p, v) in set {
            to.set_param(p, v);
        }
    }
}

/// Every settable parameter's value, to remember or carry.
pub fn values(n: &dyn Node) -> Vec<(u16, f32)> {
    n.params()
        .iter()
        .filter(|p| !p.readonly)
        .map(|p| (p.id, n.param(p.id)))
        .collect()
}

fn same_kind(a: Curve, b: Curve) -> bool {
    match (a, b) {
        (Curve::Choice(x), Curve::Choice(y)) => x == y,
        (Curve::Choice(_), _) | (_, Curve::Choice(_)) => false,
        (Curve::Toggle, Curve::Toggle) => true,
        (Curve::Toggle, _) | (_, Curve::Toggle) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(id: &str) -> Box<dyn Node> {
        registry::make(id).expect(id)
    }

    #[test]
    fn every_mode_names_a_registry_engine_in_one_category() {
        let mut engines: Vec<&str> = Vec::new();
        for f in FAMILIES {
            let cat = registry::entry(f.modes[0].id).unwrap().category;
            for m in f.modes {
                let e = registry::entry(m.id).expect(m.id);
                assert_eq!(e.category, cat, "{} {}", f.name, m.label);
                assert!(!e.master_only);
                let n = defaults(m.id);
                for &(p, v) in m.fixed {
                    let info = n.params().iter().find(|q| q.id == p).expect("fixed id");
                    assert_eq!(info.clamp(v), v, "{} {}", f.name, m.label);
                }
            }
            // An engine's modes sit together, and no engine is in two
            // families.
            let mut last = "";
            for m in f.modes {
                if m.id != last {
                    assert!(!engines.contains(&m.id), "{} split or shared", m.id);
                    engines.push(m.id);
                    last = m.id;
                }
            }
        }
    }

    #[test]
    fn a_new_engine_is_its_first_mode() {
        for f in FAMILIES {
            for (i, m) in f.modes.iter().enumerate() {
                if f.modes.iter().position(|o| o.id == m.id) != Some(i) {
                    continue;
                }
                let n = defaults(m.id);
                let (g, mode) = node_mode(n.as_ref()).unwrap();
                assert_eq!(g.name, f.name);
                assert_eq!(mode, Some(i), "{} {}", f.name, m.label);
            }
        }
    }

    #[test]
    fn each_mode_is_found_from_its_fixed_values() {
        for f in FAMILIES {
            for (i, m) in f.modes.iter().enumerate() {
                let mut n = defaults(m.id);
                for &(p, v) in m.fixed {
                    n.set_param(p, v);
                }
                assert_eq!(node_mode(n.as_ref()).unwrap().1, Some(i));
                let name = insert_name(n.as_ref());
                assert_eq!(name, format!("{} · {}", f.name, m.label));
            }
        }
        // The Cloud reverb's own Plate is in no mode, and hides nothing.
        let mut c = defaults(CLOUD);
        c.set_param(cloud::CHARACTER, 0.0);
        assert_eq!(node_mode(c.as_ref()).unwrap().1, None);
        assert!(hidden(CLOUD, |p| c.param(p)).is_empty());
        assert_eq!(insert_name(c.as_ref()), "Cloud reverb");
        assert!(mode_of("mpfree.fx.compressor", |_| 0.0).is_none());
    }

    #[test]
    fn the_picker_shows_each_family_once() {
        let t = tiles(false);
        let all = registry::offered(false).count();
        let members: usize = FAMILIES
            .iter()
            .map(|f| {
                let mut ids: Vec<&str> = f.modes.iter().map(|m| m.id).collect();
                ids.dedup();
                ids.len()
            })
            .sum();
        assert_eq!(t.len(), all - members + FAMILIES.len());
        assert_eq!(t.len(), 35);
        for f in FAMILIES {
            assert_eq!(t.iter().filter(|x| x.name == f.name).count(), 1);
            for m in f.modes {
                assert_eq!(tile_id(m.id), f.modes[0].id);
            }
        }
        // Each tile adds something the registry makes, and the reverb
        // sits where the first reverb did.
        assert!(t.iter().all(|x| registry::entry(x.id).is_some()));
        let reverb = t.iter().position(|x| x.name == "Reverb").unwrap();
        assert_eq!(t[reverb - 1].name, "Delay");
        assert_eq!(tile_id("mpfree.fx.gate"), "mpfree.fx.gate");
    }

    #[test]
    fn presets_land_under_the_mode_they_play() {
        for f in FAMILIES {
            let list = presets(f);
            assert!(list.windows(2).all(|w| w[0].mode <= w[1].mode));
            for p in &list {
                let mut n = defaults(p.id);
                for &(q, v) in p.preset.values {
                    n.set_param(q, v);
                }
                assert_eq!(
                    node_mode(n.as_ref()).unwrap().1,
                    Some(p.mode),
                    "{}",
                    p.preset.name
                );
                assert!(std::ptr::eq(
                    &registry::entry(p.id).unwrap().presets[p.index],
                    p.preset
                ));
            }
        }
        let reverb = presets(&FAMILIES[0]);
        let names: Vec<&str> = reverb.iter().map(|p| p.preset.name).collect();
        assert!(names.contains(&"Big hall") && names.contains(&"Gated snare"));
        assert!(
            !names.contains(&"Small room"),
            "the Cloud's own Room is left out"
        );
    }

    #[test]
    fn a_switch_sets_up_the_new_engine_and_switching_back_restores() {
        let ids = |n: &dyn Node, name: &str| n.params().iter().find(|p| p.name == name).unwrap().id;
        let mut room = defaults(ROOM);
        let mix = ids(room.as_ref(), "Mix");
        room.set_param(mix, 40.0);
        room.set_param(ids(room.as_ref(), "Early"), 90.0);
        let plate_mode = &FAMILIES[0].modes[1];
        let mut plate = defaults(PLATE);
        Switch::new(room.as_ref(), &[], plate_mode).apply(plate.as_mut());
        assert_eq!(plate.param(ids(plate.as_ref(), "Mix")), 40.0);
        assert_eq!(
            carried_id(room.params(), mix, plate.params()),
            Some(ids(plate.as_ref(), "Mix"))
        );
        assert_eq!(
            carried_id(room.params(), ids(room.as_ref(), "Early"), plate.params()),
            None
        );
        // Back to the room with what it had: Early comes back too.
        let mut back = defaults(ROOM);
        let mut sw = Switch::new(plate.as_ref(), &[], &FAMILIES[0].modes[0]);
        sw.remembered = Some(values(room.as_ref()));
        sw.apply(back.as_mut());
        assert_eq!(values(back.as_ref()), values(room.as_ref()));
        // Remembered Drive values, back in Fuzz: the mode's value wins.
        let mut d = defaults(DRIVE);
        d.set_param(drive::DRIVE, 20.0);
        let fuzz = &FAMILIES[2].modes[4];
        let mut again = defaults(DRIVE);
        let c = defaults(CHARACTER);
        let mut sw = Switch::new(c.as_ref(), FAMILIES[2].modes[6].fixed, fuzz);
        sw.remembered = Some(values(d.as_ref()));
        sw.apply(again.as_mut());
        assert_eq!(again.param(drive::DRIVE), 20.0);
        assert_eq!(node_mode(again.as_ref()).unwrap().1, Some(4));
    }

    #[test]
    fn a_switch_keeps_what_the_engines_share() {
        let mut room = defaults(ROOM);
        let ids = |n: &dyn Node, name: &str| n.params().iter().find(|p| p.name == name).unwrap().id;
        room.set_param(ids(room.as_ref(), "Mix"), 40.0);
        room.set_param(ids(room.as_ref(), "Pre-delay"), 80.0);
        room.set_param(ids(room.as_ref(), "Early"), 90.0);
        let plate = defaults(PLATE);
        let v = switch_values(
            room.params(),
            |p| room.param(p),
            &[],
            plate.params(),
            &FAMILIES[0].modes[1],
        );
        let get = |name: &str| {
            v.iter()
                .find(|(p, _)| *p == ids(plate.as_ref(), name))
                .map(|x| x.1)
        };
        assert_eq!(get("Mix"), Some(40.0));
        assert_eq!(get("Pre-delay"), Some(80.0));
        assert_eq!(get("Tone"), None, "the room has no Tone");
        // To the Cloud's Hall: its Character is the mode's, never carried.
        let cloud = defaults(CLOUD);
        let hall = &FAMILIES[0].modes[3];
        let v = switch_values(
            plate.params(),
            |p| plate.param(p),
            &[],
            cloud.params(),
            hall,
        );
        assert_eq!(v.iter().filter(|(p, _)| *p == cloud::CHARACTER).count(), 1);
        assert_eq!(v.last(), Some(&(cloud::CHARACTER, 2.0)));
        // Drive's Mode and Character's Mode are different lists: not
        // carried, the new mode's value is.
        let d = defaults(DRIVE);
        let console = &FAMILIES[2].modes[6];
        let c = defaults(CHARACTER);
        let v = switch_values(
            d.params(),
            |p| d.param(p),
            FAMILIES[2].modes[0].fixed,
            c.params(),
            console,
        );
        assert_eq!(v.iter().filter(|(p, _)| *p == character::MODE).count(), 1);
        // Values are clamped to the new range: the plate's pre-delay goes
        // to 250 ms, the room's to 100.
        let mut p2 = defaults(PLATE);
        p2.set_param(ids(p2.as_ref(), "Pre-delay"), 200.0);
        let v = switch_values(
            p2.params(),
            |p| p2.param(p),
            &[],
            room.params(),
            &FAMILIES[0].modes[0],
        );
        let pre = v
            .iter()
            .find(|(p, _)| *p == ids(room.as_ref(), "Pre-delay"))
            .unwrap()
            .1;
        assert_eq!(pre, 100.0);
    }
}
