//! Every control of every built-in effect can be reached in the chain
//! editor: its pages ([`mpfree_chain_ui::sections::page_view`], from its
//! plan) together show each of its settable parameters, but for those its
//! family mode fixes (the Mode chooser stands for them).

use mpfree_chain_ui::sections::page_view;
use mpfree_fx::families;
use mpfree_fx::page_plan::plan;
use mpfree_fx::registry::EFFECTS;
use mpfree_mod::{ModEngine, DEFAULT_TICK};
use mpfree_param::{Owner, Strip};

#[test]
fn every_effect_control_is_on_a_page() {
    let engine = ModEngine::new(48_000, DEFAULT_TICK);
    let owner = Owner::Insert(Strip::Master, 1);
    let mut hidden = Vec::new();
    for e in EFFECTS {
        let n = (e.make)();
        let hide = families::hidden(e.id, |p| n.param(p));
        let pages = plan(e.id).map_or(1, |p| p.pages.len());
        let mut shown: Vec<u16> = Vec::new();
        for page in 0..pages {
            let v = page_view(n.as_ref(), owner, &engine, hide, page);
            shown.extend(v.params.iter().map(|p| p.info.id));
        }
        for p in n.params().iter().filter(|p| !p.readonly) {
            if !shown.contains(&p.id) && !hide.iter().any(|&(h, _)| h == p.id) {
                hidden.push(format!("{}: {} ({})", e.name, p.name, p.group));
            }
        }
    }
    assert!(hidden.is_empty(), "controls no page shows: {hidden:#?}");
}
