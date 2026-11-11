//! Which parameters go on which page of the Synth tab.
//!
//! Any node gets pages from its [`ParamInfo`] groups: consecutive groups are
//! packed onto one page while they fit, and the page takes the first group's
//! name. That is what a hosted plugin or an effect will show. A native
//! engine can name its own pages ([`plan`] knows Poly's), so related groups
//! sit together the way a player expects (the filter with its envelope).

use mpfree_node::{NodeInfo, ParamInfo};

/// The most tiles one page shows (four rows of four).
pub const MAX_TILES: usize = 16;
/// Generic pages stop adding groups at this many tiles, so most pages keep
/// big tiles (three rows of four).
pub const SOFT_TILES: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub name: String,
    /// Indexes into the node's parameter list, in page order.
    pub params: Vec<usize>,
    /// What Q-Links 1 to 4 turn, if the engine chose (indexes, on this page).
    pub qlinks: Vec<usize>,
}

/// A native engine's own page: its name, the groups on it, and the
/// parameters its Q-Links turn (by id; empty for the first four knobs).
struct PlanPage {
    name: &'static str,
    groups: &'static [&'static str],
    qlinks: &'static [u16],
}

const fn pg(name: &'static str, groups: &'static [&'static str], qlinks: &'static [u16]) -> PlanPage {
    PlanPage { name, groups, qlinks }
}

use mpfree_synth::params as pp;

const POLY: &[PlanPage] = &[
    // Osc: the detune, the second oscillator's level, the sub and the unison
    // spread are what players reach for; octaves and waves are set once.
    pg("Osc", &["Osc 1", "Osc 2", "Mix", "Unison"], &[pp::OSC2_FINE, pp::OSC2_LEVEL, pp::SUB_LEVEL, pp::UNISON_DETUNE]),
    pg("Filter", &["Filter", "Filter env"], &[]),
    pg("Env", &["Amp env"], &[]),
    pg("LFO", &["LFO"], &[]),
    pg("Voice", &["Voice", "Output"], &[pp::GLIDE, pp::PRESSURE_CUTOFF, pp::LEVEL, pp::PAN]),
];

fn known(id: &str) -> Option<&'static [PlanPage]> {
    match id {
        "mpfree.poly" => Some(POLY),
        _ => None,
    }
}

/// The pages for a node. Every parameter is on exactly one page; a group the
/// engine's own plan doesn't name goes on generic pages after it.
pub fn plan(info: &NodeInfo, params: &[ParamInfo]) -> Vec<Page> {
    let mut placed = vec![false; params.len()];
    let mut pages = Vec::new();
    if let Some(plan) = known(info.id) {
        for PlanPage { name, groups, qlinks } in plan {
            let idx: Vec<usize> = groups
                .iter()
                .flat_map(|g| (0..params.len()).filter(move |&i| params[i].group == *g))
                .filter(|&i| !placed[i])
                .collect();
            for &i in &idx {
                placed[i] = true;
            }
            for (k, chunk) in idx.chunks(MAX_TILES).enumerate() {
                let name = if k == 0 { name.to_string() } else { format!("{name} {}", k + 1) };
                let q = qlinks
                    .iter()
                    .filter_map(|id| chunk.iter().copied().find(|&i| params[i].id == *id))
                    .take(4)
                    .collect();
                pages.push(Page { name, params: chunk.to_vec(), qlinks: q });
            }
        }
    }
    pages.extend(generic(params, &placed));
    pages
}

/// Groups in the order they first appear, packed onto pages.
fn generic(params: &[ParamInfo], placed: &[bool]) -> Vec<Page> {
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (i, p) in params.iter().enumerate() {
        if placed[i] {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == p.group) {
            Some((_, v)) => v.push(i),
            None => groups.push((p.group, vec![i])),
        }
    }
    let mut pages: Vec<Page> = Vec::new();
    for (group, idx) in groups {
        let name = if group.is_empty() { "Main" } else { group };
        let fits = pages.last().is_some_and(|p| p.params.len() + idx.len() <= SOFT_TILES);
        if fits {
            pages.last_mut().unwrap().params.extend(idx);
            continue;
        }
        for (k, chunk) in idx.chunks(MAX_TILES).enumerate() {
            let name = if k == 0 { name.to_string() } else { format!("{name} {}", k + 1) };
            pages.push(Page { name, params: chunk.to_vec(), qlinks: Vec::new() });
        }
    }
    pages
}

/// A tile's label: the parameter's name, with its group when the group is
/// not the page's own ("Env · Attack" on the Filter page, "Osc 2 · Wave").
pub fn tile_label(page: &str, p: &ParamInfo) -> String {
    if p.group == page || p.group.is_empty() || p.group == p.name {
        return p.name.to_string();
    }
    // "Filter env" on the Filter page is the page's env; "Osc 2" on the Osc
    // page is its own thing and keeps its whole name.
    match p.group.strip_prefix(page).and_then(|r| r.strip_prefix(' ')) {
        Some(rest) if rest.starts_with(|c: char| c.is_alphabetic()) => format!("{} · {}", cap(rest), p.name),
        _ => format!("{} · {}", p.group, p.name),
    }
}

/// What Q-Links 1 to 4 turn on a page: the engine's choice, or else the
/// first four knobs (a choice, a switch or a meter is left to the touch
/// screen), then whatever is left.
pub fn qlink_targets(page: &Page, params: &[ParamInfo]) -> Vec<usize> {
    if !page.qlinks.is_empty() {
        return page.qlinks.clone();
    }
    let knob = |i: &usize| {
        let p = &params[*i];
        !p.readonly && !matches!(p.curve, mpfree_node::Curve::Choice(_) | mpfree_node::Curve::Toggle)
    };
    let mut v: Vec<usize> = page.params.iter().copied().filter(knob).take(4).collect();
    for &i in &page.params {
        if v.len() == 4 {
            break;
        }
        if !v.contains(&i) && !params[i].readonly {
            v.push(i);
        }
    }
    v
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{Curve, Kind, Unit};
    use mpfree_synth::params::PARAMS;

    static POLY_INFO: NodeInfo = NodeInfo { id: "mpfree.poly", name: "Poly", kind: Kind::Instrument };
    static OTHER: NodeInfo = NodeInfo { id: "test.other", name: "Other", kind: Kind::Effect };

    fn every_param_once(pages: &[Page], n: usize) {
        let mut seen = vec![0; n];
        for p in pages {
            assert!(!p.params.is_empty(), "page {} is empty", p.name);
            assert!(p.params.len() <= MAX_TILES, "page {} has {} tiles", p.name, p.params.len());
            for &i in &p.params {
                seen[i] += 1;
            }
        }
        assert!(seen.iter().all(|&s| s == 1), "{seen:?}");
    }

    #[test]
    fn poly_has_its_own_pages_and_every_param_once() {
        let pages = plan(&POLY_INFO, PARAMS);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Osc", "Filter", "Env", "LFO", "Voice"]);
        every_param_once(&pages, PARAMS.len());
        // The Q-Links turn the filter page's first four knobs, not its Type.
        let q: Vec<&str> = qlink_targets(&pages[1], PARAMS).iter().map(|&i| PARAMS[i].name).collect();
        assert_eq!(q, ["Cutoff", "Resonance", "Drive", "Env amount"]);
        // A page of choices still gets four.
        let lfo = qlink_targets(&pages[3], PARAMS);
        assert_eq!(lfo.len(), 4);
        assert!(lfo.iter().all(|&i| !PARAMS[i].readonly));
        // The engine's own choice, where it made one.
        let osc: Vec<&str> = qlink_targets(&pages[0], PARAMS).iter().map(|&i| PARAMS[i].name).collect();
        assert_eq!(osc, ["Fine", "Level", "Sub", "Detune"]);
        for page in &pages {
            assert!(page.qlinks.iter().all(|i| page.params.contains(i)), "{}", page.name);
        }
    }

    #[test]
    fn any_node_gets_generic_pages() {
        let pages = plan(&OTHER, PARAMS);
        every_param_once(&pages, PARAMS.len());
        assert_eq!(pages[0].name, "Osc 1");
    }

    #[test]
    fn a_big_group_is_split() {
        const fn q(id: u16) -> ParamInfo {
            ParamInfo { id, name: "P", group: "Big", min: 0.0, max: 1.0, default: 0.0, unit: Unit::None,
                        curve: Curve::Linear, readonly: false }
        }
        let params: Vec<ParamInfo> = (0..40).map(q).collect();
        let pages = plan(&OTHER, &params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Big", "Big 2", "Big 3"]);
        every_param_once(&pages, 40);
    }

    #[test]
    fn labels_name_the_group_when_it_is_not_the_page() {
        let p = |g: &'static str, n: &'static str| ParamInfo {
            id: 0, name: n, group: g, min: 0.0, max: 1.0, default: 0.0, unit: Unit::None, curve: Curve::Linear,
            readonly: false,
        };
        assert_eq!(tile_label("Filter", &p("Filter", "Cutoff")), "Cutoff");
        assert_eq!(tile_label("Filter", &p("Filter env", "Attack")), "Env · Attack");
        assert_eq!(tile_label("Osc", &p("Osc 2", "Wave")), "Osc 2 · Wave");
        assert_eq!(tile_label("Osc", &p("Mix", "Sub")), "Mix · Sub");
        assert_eq!(tile_label("Osc", &p("Unison", "Unison")), "Unison");
    }
}
