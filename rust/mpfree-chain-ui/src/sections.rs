//! The selected effect's page as labelled sections (the effect page
//! redesign, Jay 14 Nov 2026: "Build it as drawn"), for the chain editor to
//! draw with the Synth tab's section widget
//! (`mpfree_synth_ui::section_view`).
//!
//! A built-in effect follows its plan (`mpfree_fx::page_plan`): its pages
//! (one for most effects), each page's sections in their job's colour with
//! their controls sized by tier, the effect's drawings by card title, live
//! meters, a stage's switch in its heading, and the Q-Links the plan picks
//! (on every page; a synced Time's Q-Link turns Sync). A control another
//! one makes do nothing is dimmed, and its section says why. A family
//! mode's fixed controls are left out, since the Mode chooser stands for
//! them.
//!
//! An effect with no plan (a hosted plugin) keeps its pages of up to eight
//! controls by group ([`crate::state::pages`]), each shown as a section
//! per group (`mpfree_synth_ui::sections::by_group`), its Q-Links the
//! page's first four.

use mpfree_fx::mpfree_node::Node;
use mpfree_fx::page_plan::{self, Plan};
use mpfree_mod::ModEngine;
use mpfree_param::Owner;
use mpfree_synth_ui::sections::{by_group, Job, Tier};

use crate::state::{pages, param_view, ParamView};

/// One control of a section.
#[derive(Clone, Debug, PartialEq)]
pub struct CtlState {
    /// Its knob: an index into [`PageView::params`].
    pub param: usize,
    /// What the section prints for it ("Threshold" in the Gate section).
    pub label: String,
    pub tier: Tier,
    /// Drawn as a vertical fader.
    pub fader: bool,
    /// Q-Link 1 to 4 turns it (0 to 3).
    pub qlink: Option<usize>,
    /// Why it does nothing as the effect is set, if it doesn't.
    pub dim: Option<&'static str>,
}

/// One section of the page shown.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionState {
    /// Its slot in [`PageView::spec`].
    pub key: String,
    pub title: String,
    pub job: Job,
    /// Beside the title: why its first dimmed control is dim ("Time · Sync
    /// sets it"), else the plan's note.
    pub note: String,
    /// The page a tap on its title opens (an index into
    /// [`PageView::pages`]).
    pub link: Option<usize>,
    /// The effect's drawings it shows, by card title.
    pub drawings: Vec<String>,
    /// Its controls two to a line (every one of them is a choice, a switch
    /// or a cut).
    pub rows: bool,
    /// Its controls, then its meters.
    pub controls: Vec<CtlState>,
    /// The switch in its heading (a stage of the Channel strip).
    pub switch: Option<CtlState>,
}

/// The selected effect's page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageView {
    /// Its pages' names: a tab each when there are two or more.
    pub pages: Vec<String>,
    /// The page shown.
    pub page: usize,
    /// The sections' layout over their keys (`mpfree-ui` syntax).
    pub spec: String,
    /// Every control, switch and meter on the page shown, each once.
    pub params: Vec<ParamView>,
    pub sections: Vec<SectionState>,
    /// What Q-Links 1 to 4 turn, the same on every page.
    pub qlinks: Vec<ParamView>,
}

/// Insert `n`'s page `page` (of `owner`, modulated by `engine`), leaving
/// out `hide` (its family mode's fixed controls).
pub fn page_view(
    n: &dyn Node,
    owner: Owner,
    engine: &ModEngine,
    hide: &[(u16, f32)],
    page: usize,
) -> PageView {
    match page_plan::plan(n.info().id) {
        Some(plan) => planned(n, owner, engine, hide, page, plan),
        None => by_groups(n, owner, engine, hide, page),
    }
}

fn job(j: page_plan::Job) -> Job {
    match j {
        page_plan::Job::Source => Job::Source,
        page_plan::Job::Tone => Job::Tone,
        page_plan::Job::Shape => Job::Shape,
        page_plan::Job::Motion => Job::Motion,
        page_plan::Job::Voice => Job::Voice,
    }
}

fn tier(t: page_plan::Tier) -> Tier {
    match t {
        page_plan::Tier::Trim => Tier::Trim,
        page_plan::Tier::Std => Tier::Std,
        page_plan::Tier::Main => Tier::Main,
        page_plan::Tier::Hero => Tier::Hero,
    }
}

/// The index of parameter `id`'s knob in `params`, added if it isn't there.
fn knob(
    params: &mut Vec<ParamView>,
    n: &dyn Node,
    owner: Owner,
    engine: &ModEngine,
    id: u16,
) -> Option<usize> {
    if let Some(k) = params.iter().position(|p| p.info.id == id) {
        return Some(k);
    }
    params.push(param_view(n, owner, engine, id)?);
    Some(params.len() - 1)
}

fn planned(
    n: &dyn Node,
    owner: Owner,
    engine: &ModEngine,
    hide: &[(u16, f32)],
    page: usize,
    plan: &'static Plan,
) -> PageView {
    let infos = n.params();
    let find = |name: &str| mpfree_fx::main_page::find(infos, name);
    let value = |id: u16| {
        engine
            .value(mpfree_param::ParamAddr::new(owner, id))
            .unwrap_or_else(|| n.param(id))
    };
    let hidden = |id: u16| hide.iter().any(|&(h, _)| h == id);
    // What each Q-Link turns now: the plan's pick, or what a swap moves it
    // to (Sync, while Time is synced).
    let turns: Vec<u16> = plan
        .qlinks
        .iter()
        .filter_map(|q| find(plan.qlink_turns(infos, q, value)).map(|p| p.id))
        .collect();
    let page = page.min(plan.pages.len() - 1);
    let pg = &plan.pages[page];
    let mut params: Vec<ParamView> = Vec::new();
    let mut sections = Vec::new();
    for sec in pg.sections {
        let mut why = None;
        let mut ctl = |params: &mut Vec<ParamView>, name: &str| -> Option<CtlState> {
            let info = find(name)?;
            if hidden(info.id) {
                return None;
            }
            let k = knob(params, n, owner, engine, info.id)?;
            let qlink = turns.iter().position(|&t| t == info.id);
            let label = sec.label(name).to_string();
            let dim = if info.readonly {
                None
            } else {
                plan.dimmed(infos, name, value)
            };
            if let (None, Some(r)) = (&why, dim) {
                why = Some(format!("{label} · {r}"));
            }
            Some(CtlState {
                param: k,
                label,
                tier: tier(sec.tier(name, qlink.is_some())),
                fader: sec.faders.contains(&name),
                qlink,
                dim,
            })
        };
        let switch = sec.switch.and_then(|sw| ctl(&mut params, sw));
        let mut controls: Vec<CtlState> = Vec::new();
        for name in sec.params.iter().chain(sec.meters) {
            if let Some(c) = ctl(&mut params, name) {
                controls.push(c);
            }
        }
        if controls.is_empty() && sec.drawings.is_empty() {
            continue;
        }
        let rows = !sec.params.is_empty() && sec.params.iter().all(|p| sec.rows.contains(p));
        sections.push(SectionState {
            key: sec.key.to_string(),
            title: sec.name.to_string(),
            job: job(sec.job),
            note: why
                .or_else(|| sec.note.map(str::to_string))
                .unwrap_or_default(),
            link: sec
                .link
                .and_then(|l| plan.pages.iter().position(|p| p.name == l)),
            drawings: sec.drawings.iter().map(|d| d.to_string()).collect(),
            rows,
            controls,
            switch,
        });
    }
    let qlinks = turns
        .iter()
        .filter_map(|&id| param_view(n, owner, engine, id))
        .collect();
    PageView {
        pages: plan.pages.iter().map(|p| p.name.to_string()).collect(),
        page,
        spec: pg.spec.to_string(),
        params,
        sections,
        qlinks,
    }
}

fn by_groups(
    n: &dyn Node,
    owner: Owner,
    engine: &ModEngine,
    hide: &[(u16, f32)],
    page: usize,
) -> PageView {
    let infos = n.params();
    let ps = pages(infos, hide, None);
    if ps.is_empty() {
        return PageView::default();
    }
    let page = page.min(ps.len() - 1);
    let index = |id: u16| infos.iter().position(|p| p.id == id);
    let shown = mpfree_synth_ui::pages::Page {
        name: ps[page].label.clone(),
        params: ps[page].ids.iter().filter_map(|&id| index(id)).collect(),
        qlinks: Vec::new(),
    };
    let a = by_group(infos, &shown);
    let mut params: Vec<ParamView> = Vec::new();
    let mut sections = Vec::new();
    for sec in &a.sections {
        let mut controls = Vec::new();
        for &i in &sec.params {
            let info = &infos[i];
            let Some(k) = knob(&mut params, n, owner, engine, info.id) else {
                continue;
            };
            let qlink = shown.params.iter().take(4).position(|&q| q == i);
            controls.push(CtlState {
                param: k,
                label: sec.label(i, info),
                tier: sec.tier(i, qlink.is_some()),
                fader: sec.faders.contains(&i),
                qlink,
                dim: None,
            });
        }
        sections.push(SectionState {
            key: sec.key.clone(),
            title: sec.name.clone(),
            job: sec.job,
            note: String::new(),
            link: None,
            drawings: Vec::new(),
            rows: sec.rows,
            controls,
            switch: None,
        });
    }
    let qlinks = ps[page]
        .ids
        .iter()
        .take(4)
        .filter_map(|&id| param_view(n, owner, engine, id))
        .collect();
    PageView {
        pages: ps.iter().map(|p| p.label.clone()).collect(),
        page,
        spec: a.spec,
        params,
        sections,
        qlinks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_fx::registry::{self, EFFECTS};
    use mpfree_mod::DEFAULT_TICK;
    use mpfree_param::Strip;

    const OWNER: Owner = Owner::Insert(Strip::Master, 1);

    fn engine() -> ModEngine {
        ModEngine::new(48_000, DEFAULT_TICK)
    }

    fn hide(n: &dyn Node) -> &'static [(u16, f32)] {
        mpfree_fx::families::hidden(n.info().id, |p| n.param(p))
    }

    fn slots(spec: &str) -> Vec<String> {
        let mut s = mpfree_ui::layout::slots(&mpfree_ui::layout::parse(spec).unwrap());
        s.sort();
        s
    }

    /// Every built-in effect's every page: each settable control it shows
    /// once, every section in the layout, every knob used, the Q-Links
    /// badged where they are on the page.
    #[test]
    fn every_page_of_every_effect_shows_its_controls_once() {
        let e = engine();
        for fx in EFFECTS {
            let n = registry::make(fx.id).unwrap();
            let plan = page_plan::plan(fx.id).unwrap();
            for p in 0..plan.pages.len() {
                let v = page_view(n.as_ref(), OWNER, &e, hide(n.as_ref()), p);
                assert_eq!(v.page, p);
                let mut keys: Vec<String> = v.sections.iter().map(|s| s.key.clone()).collect();
                keys.sort();
                assert_eq!(slots(&v.spec), keys, "{} page {p}", fx.name);
                let mut used: Vec<usize> = v
                    .sections
                    .iter()
                    .flat_map(|s| s.controls.iter().chain(&s.switch).map(|c| c.param))
                    .collect();
                let all = used.len();
                used.sort_unstable();
                used.dedup();
                assert_eq!(used.len(), all, "{} page {p}: a control twice", fx.name);
                assert_eq!(used, (0..v.params.len()).collect::<Vec<_>>(), "{}", fx.name);
                for c in v.sections.iter().flat_map(|s| s.controls.iter()) {
                    if let Some(q) = c.qlink {
                        assert_eq!(v.qlinks[q].addr, v.params[c.param].addr, "{}", fx.name);
                    }
                }
                for h in hide(n.as_ref()) {
                    assert!(!v.params.iter().any(|k| k.info.id == h.0), "{}", fx.name);
                }
            }
            let v = page_view(n.as_ref(), OWNER, &e, hide(n.as_ref()), 0);
            assert_eq!(v.qlinks.len(), plan.qlinks.len(), "{}", fx.name);
            let badged = v
                .sections
                .iter()
                .flat_map(|s| s.controls.iter())
                .filter(|c| c.qlink.is_some())
                .count();
            assert_eq!(badged, plan.qlinks.len(), "{}: Q-Link badges", fx.name);
        }
    }

    #[test]
    fn a_synced_delay_dims_time_and_its_q_link_turns_sync() {
        let e = engine();
        let mut n = registry::make("mpfree.fx.delay").unwrap();
        let sync = mpfree_fx::main_page::find(n.params(), "Sync").unwrap().id;
        n.set_param(sync, 0.0);
        let free = page_view(n.as_ref(), OWNER, &e, &[], 0);
        assert_eq!(free.qlinks[0].info.name, "Time");
        n.set_param(sync, 4.0);
        let v = page_view(n.as_ref(), OWNER, &e, &[], 0);
        assert_eq!(v.qlinks[0].info.name, "Sync");
        let echo = &v.sections[0];
        let time = echo
            .controls
            .iter()
            .find(|c| v.params[c.param].info.name == "Time")
            .unwrap();
        assert_eq!(time.dim, Some("Sync sets it"));
        assert_eq!(time.qlink, None);
        assert_eq!(echo.note, "Time · Sync sets it");
        let sync_ctl = echo
            .controls
            .iter()
            .find(|c| v.params[c.param].info.name == "Sync")
            .unwrap();
        assert_eq!(sync_ctl.qlink, Some(0));
    }

    #[test]
    fn the_channel_strip_has_a_main_page_and_a_page_per_stage() {
        let e = engine();
        let n = registry::make("mpfree.fx.channel_strip").unwrap();
        let v = page_view(n.as_ref(), OWNER, &e, &[], 0);
        assert_eq!(v.pages, ["Main", "Gate", "Compressor", "EQ"]);
        let comp = v.sections.iter().find(|s| s.key == "comp").unwrap();
        assert_eq!(comp.link, Some(2));
        let sw = comp.switch.as_ref().unwrap();
        assert_eq!(v.params[sw.param].info.name, "Comp");
        assert_eq!(comp.controls[0].label, "Threshold");
        assert_eq!(comp.controls[0].tier, Tier::Hero);
        let gate = page_view(n.as_ref(), OWNER, &e, &[], 1);
        assert_eq!(gate.page, 1);
        assert_eq!(gate.sections[0].controls.len(), 5, "4 controls and a meter");
        // The Q-Links stay on Main's four on every page.
        assert_eq!(gate.qlinks, v.qlinks);
        assert_eq!(page_view(n.as_ref(), OWNER, &e, &[], 9).page, 3);
    }

    #[test]
    fn a_drive_in_a_mode_leaves_its_mode_to_the_chooser() {
        let e = engine();
        let n = registry::make("mpfree.fx.drive").unwrap();
        let v = page_view(n.as_ref(), OWNER, &e, hide(n.as_ref()), 0);
        assert!(!v.params.iter().any(|p| p.info.name == "Mode"));
        let drive = &v.sections[0];
        assert_eq!(drive.drawings, ["Curve", "A sine through it"]);
        assert_eq!(drive.controls[0].tier, Tier::Hero);
        let meters: Vec<&str> = v
            .params
            .iter()
            .filter(|p| p.info.readonly)
            .map(|p| p.info.name)
            .collect();
        assert_eq!(meters, ["Level change"]);
    }

    #[test]
    fn an_effect_with_no_plan_gets_pages_of_eight_by_group() {
        let e = engine();
        let n = registry::make("mpfree.fx.multiband").unwrap();
        let v = by_groups(n.as_ref(), OWNER, &e, &[], 0);
        assert_eq!(v.pages, ["Bands", "Low", "Mid", "High"]);
        assert!(v.params.len() <= 8);
        assert_eq!(v.qlinks.len(), 4);
        assert_eq!(
            v.sections
                .iter()
                .flat_map(|s| &s.controls)
                .filter(|c| c.qlink.is_some())
                .count(),
            4
        );
        mpfree_ui::layout::parse(&v.spec).unwrap();
    }
}
