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
    pg("Voice", &["Voice", "Output"], &[pp::GLIDE, pp::BEND_RANGE, pp::LEVEL, pp::PAN]),
    // Pressure: the threshold, where it's full, and the first two slots'
    // depths are what a player tunes while playing.
    pg("Pressure", &["Pressure"], &[pp::PRESS_THRESHOLD, pp::PRESS_FULL_AT, pp::press_depth(0), pp::press_depth(1)]),
];

use mpfree_synth::fm_params::{self as fp, op, op_id};

/// FM: the algorithm and voice, the modulation (pitch envelope and LFO), a
/// page per operator, and the operators' keyboard scaling two to a page.
const FM: &[PlanPage] = &[
    pg("Algo", &["Algo", "Voice", "Output"], &[fp::ALGORITHM, fp::FEEDBACK, fp::TRANSPOSE, fp::LEVEL]),
    pg("Mod", &["LFO", "Pitch env"], &[fp::LFO_SPEED, fp::LFO_PMD, fp::LFO_AMD, fp::LFO_DELAY]),
    pg("Pressure", &["Pressure"], &[fp::PRESS_THRESHOLD, fp::PRESS_FULL_AT, fp::press_depth(0), fp::press_depth(1)]),
    pg("Op 1", &["Op 1"], &FM_OP_Q[0]),
    pg("Op 2", &["Op 2"], &FM_OP_Q[1]),
    pg("Op 3", &["Op 3"], &FM_OP_Q[2]),
    pg("Op 4", &["Op 4"], &FM_OP_Q[3]),
    pg("Op 5", &["Op 5"], &FM_OP_Q[4]),
    pg("Op 6", &["Op 6"], &FM_OP_Q[5]),
    pg("Kbd 1-2", &["Op 1 kbd", "Op 2 kbd"], &[]),
    pg("Kbd 3-4", &["Op 3 kbd", "Op 4 kbd"], &[]),
    pg("Kbd 5-6", &["Op 5 kbd", "Op 6 kbd"], &[]),
];

/// An operator page's Q-Links: output level, coarse, fine and velocity.
const FM_OP_Q: [[u16; 4]; 6] = {
    let mut q = [[0u16; 4]; 6];
    let mut n = 0;
    while n < 6 {
        q[n] = [op_id(n + 1, op::LEVEL), op_id(n + 1, op::COARSE), op_id(n + 1, op::FINE), op_id(n + 1, op::VEL_SENS)];
        n += 1;
    }
    q
};

use mpfree_synth::wavetable::params::{self as wp, osc_id};

/// Wavetable: a page per oscillator (Q-Links on table position, warp,
/// fine and level, the knobs played live), then the mix, filter, mod
/// envelope, LFO, amp envelope, voice and pressure.
const WT: &[PlanPage] = &[
    pg("Osc A", &["Osc A"], &[osc_id(0, wp::POSITION), osc_id(0, wp::WARP), osc_id(0, wp::FINE), osc_id(0, wp::OSC_LEVEL)]),
    pg("Osc B", &["Osc B"], &[osc_id(1, wp::POSITION), osc_id(1, wp::WARP), osc_id(1, wp::FINE), osc_id(1, wp::OSC_LEVEL)]),
    pg("Mix", &["Mix", "Unison"], &[wp::SUB_LEVEL, wp::NOISE_LEVEL, wp::UNISON_DETUNE, wp::UNISON_SPREAD]),
    pg("Filter", &["Filter"], &[]),
    pg("Mod", &["Mod env"], &[wp::MENV_POS_A, wp::MENV_WARP, wp::MENV_D, wp::VEL_POS]),
    pg("LFO", &["LFO"], &[wp::LFO_RATE, wp::LFO_POS_A, wp::LFO_POS_B, wp::LFO_PITCH]),
    pg("Env", &["Amp env"], &[]),
    pg("Voice", &["Voice", "Output"], &[wp::GLIDE, wp::BEND_RANGE, wp::LEVEL, wp::PAN]),
    pg("Pressure", &["Pressure"], &[wp::PRESS_THRESHOLD, wp::PRESS_FULL_AT, wp::press_depth(0), wp::press_depth(1)]),
];

use mpfree_synth::drum_params::{field as df, pad_id, LEVEL as DRUM_LEVEL, BASE_NOTE};

/// The drum synth's Q-Links on pad `n`'s page: tune, decay, tone, snap.
const fn drum_q(n: usize) -> [u16; 4] { [pad_id(n, df::TUNE), pad_id(n, df::DECAY), pad_id(n, df::TONE), pad_id(n, df::SNAP)] }
const DRUM_Q: [[u16; 4]; 16] = {
    let mut q = [[0u16; 4]; 16];
    let mut n = 0;
    while n < 16 {
        q[n] = drum_q(n);
        n += 1;
    }
    q
};

/// Drums: a page per pad (its 14 knobs), then the kit.
const DRUMS: &[PlanPage] = &[
    pg("Pad 1", &["Pad 1"], &DRUM_Q[0]), pg("Pad 2", &["Pad 2"], &DRUM_Q[1]), pg("Pad 3", &["Pad 3"], &DRUM_Q[2]), pg("Pad 4", &["Pad 4"], &DRUM_Q[3]),
    pg("Pad 5", &["Pad 5"], &DRUM_Q[4]), pg("Pad 6", &["Pad 6"], &DRUM_Q[5]), pg("Pad 7", &["Pad 7"], &DRUM_Q[6]), pg("Pad 8", &["Pad 8"], &DRUM_Q[7]),
    pg("Pad 9", &["Pad 9"], &DRUM_Q[8]), pg("Pad 10", &["Pad 10"], &DRUM_Q[9]), pg("Pad 11", &["Pad 11"], &DRUM_Q[10]), pg("Pad 12", &["Pad 12"], &DRUM_Q[11]),
    pg("Pad 13", &["Pad 13"], &DRUM_Q[12]), pg("Pad 14", &["Pad 14"], &DRUM_Q[13]), pg("Pad 15", &["Pad 15"], &DRUM_Q[14]), pg("Pad 16", &["Pad 16"], &DRUM_Q[15]),
    pg("Kit", &["Kit"], &[DRUM_LEVEL, BASE_NOTE]),
];

/// The page that holds pad `pad`'s knobs (0 to 15), so an app can follow
/// the last pad hit.
pub fn drum_page(pad: usize) -> usize { pad.min(15) }

use mpfree_synth::pluck_params as kp;

/// Pluck: the string (with the model, decay and release), the mallet, and
/// the voice.
const PLUCK: &[PlanPage] = &[
    pg("String", &["Model", "String"], &[kp::DECAY, kp::BRIGHTNESS, kp::PICK, kp::POSITION]),
    pg("Mallet", &["Mallet"], &[kp::HARDNESS, kp::STRIKE, kp::STRETCH, kp::HIGH_DAMP]),
    pg("Voice", &["Voice", "Output"], &[kp::VEL_TONE, kp::BEND_RANGE, kp::LEVEL, kp::SPREAD]),
];

fn known(id: &str) -> Option<&'static [PlanPage]> {
    match id {
        "mpfree.poly" => Some(POLY),
        "mpfree.fm" => Some(FM),
        "mpfree.wavetable" => Some(WT),
        "mpfree.drums" => Some(DRUMS),
        "mpfree.pluck" => Some(PLUCK),
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
        assert_eq!(names, ["Osc", "Filter", "Env", "LFO", "Voice", "Pressure"]);
        every_param_once(&pages, PARAMS.len());
        // Pressure: the 8 shaper settings and 4 slots fill one page.
        assert_eq!(pages[5].params.len(), 16);
        let q: Vec<&str> = qlink_targets(&pages[5], PARAMS).iter().map(|&i| PARAMS[i].name).collect();
        assert_eq!(q, ["Threshold", "Full at", "Depth 1", "Depth 2"]);
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
    fn fm_has_its_own_pages_and_every_param_once() {
        use mpfree_node::Node;
        let fm = mpfree_synth::Fm::new(48_000);
        let params = fm.params();
        let pages = plan(fm.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Algo", "Mod", "Pressure", "Op 1", "Op 2", "Op 3", "Op 4", "Op 5", "Op 6", "Kbd 1-2", "Kbd 3-4", "Kbd 5-6"]);
        assert_eq!(pages[2].params.len(), 16);
        let q: Vec<&str> = qlink_targets(&pages[2], params).iter().map(|&i| params[i].name).collect();
        assert_eq!(q, ["Threshold", "Full at", "Depth 1", "Depth 2"]);
        let mut seen = vec![0; params.len()];
        for p in &pages {
            assert!(p.params.len() <= MAX_TILES, "{}: {} tiles", p.name, p.params.len());
            for &i in &p.params {
                seen[i] += 1;
            }
        }
        assert!(seen.iter().all(|&n| n == 1), "every parameter on exactly one page");
        for p in &pages[3..9] {
            assert_eq!(p.params.len(), 16);
            let q: Vec<&str> = qlink_targets(p, params).iter().map(|&i| params[i].name).collect();
            assert_eq!(q, ["Output", "Coarse", "Fine", "Velocity"], "{}", p.name);
        }
        // Labels stay short enough for a tile.
        for p in &pages {
            for &i in &p.params {
                let l = tile_label(&p.name, &params[i]);
                assert!(l.len() <= 20, "{}: {l}", p.name);
            }
        }
    }

    #[test]
    fn drums_get_a_page_per_pad_and_the_kit() {
        use mpfree_node::Node;
        let d = mpfree_synth::Drums::new(48_000);
        let params = d.params();
        let pages = plan(d.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        let mut want: Vec<String> = (1..=16).map(|n| format!("Pad {n}")).collect();
        want.push("Kit".into());
        assert_eq!(names, want);
        every_param_once(&pages, params.len());
        for (n, p) in pages[..16].iter().enumerate() {
            assert_eq!(p.params.len(), 14);
            let q: Vec<&str> = qlink_targets(p, params).iter().map(|&i| params[i].name).collect();
            assert_eq!(q, ["Tune", "Decay", "Tone", "Snap"], "{}", p.name);
            assert_eq!(drum_page(n), n);
            for &i in &p.params {
                assert!(tile_label(&p.name, &params[i]).len() <= 20);
            }
        }
    }

    #[test]
    fn pluck_has_its_own_pages() {
        use mpfree_node::Node;
        let s = mpfree_synth::Pluck::new(48_000);
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["String", "Mallet", "Voice"]);
        every_param_once(&pages, params.len());
        let q: Vec<&str> = qlink_targets(&pages[0], params).iter().map(|&i| params[i].name).collect();
        assert_eq!(q, ["Decay", "Brightness", "Pick", "Position"]);
        let q: Vec<&str> = qlink_targets(&pages[1], params).iter().map(|&i| params[i].name).collect();
        assert_eq!(q, ["Hardness", "Strike", "Stretch", "High damp"]);
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
