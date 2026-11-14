//! Every control of every built-in effect can be reached in the chain
//! editor: its pages (a Main page first for an effect with one, then its
//! groups) together show all of its parameters, none over
//! `KNOBS_PER_PAGE`. The editor splits a group over a page ("EQ 2"), but
//! a built-in effect keeps each group within one, so its tabs are the
//! groups it names.

use mpfree_chain_ui::state::{pages, KNOBS_PER_PAGE};
use mpfree_fx::main_page::main_page;
use mpfree_fx::registry::EFFECTS;

#[test]
fn every_effect_control_is_on_a_page() {
    let mut hidden = Vec::new();
    let mut split = Vec::new();
    for e in EFFECTS {
        let params = (e.make)().params();
        let ps = pages(params, &[], main_page(e.id));
        let mut shown: Vec<u16> = Vec::new();
        for p in &ps {
            assert!(p.ids.len() <= KNOBS_PER_PAGE, "{} {}", e.name, p.label);
            if p.label != "Main" && !params.iter().any(|q| q.group == p.label) {
                split.push(format!("{}: {}", e.name, p.label));
            }
            shown.extend(&p.ids);
        }
        for p in params.iter().filter(|p| !p.readonly) {
            if !shown.contains(&p.id) {
                hidden.push(format!("{}: {} ({})", e.name, p.name, p.group));
            }
        }
    }
    assert!(hidden.is_empty(), "controls no page shows: {hidden:#?}");
    assert!(split.is_empty(), "groups over a page: {split:#?}");
}
