//! Every control of every built-in effect can be reached in the chain
//! editor: its group tabs together show all of its parameters. A group
//! over `KNOBS_PER_PAGE` would hide the ones past the page.

use mpfree_chain_ui::state::{page, KNOBS_PER_PAGE};
use mpfree_fx::registry::EFFECTS;

#[test]
fn every_effect_control_is_on_a_page() {
    let mut hidden = Vec::new();
    for e in EFFECTS {
        let params = (e.make)().params();
        let (groups, _) = page(params, 0);
        let mut shown: Vec<u16> = Vec::new();
        for g in 0..groups.len() {
            let (_, ps) = page(params, g);
            assert!(ps.len() <= KNOBS_PER_PAGE);
            shown.extend(ps.iter().map(|p| p.id));
        }
        for p in params.iter().filter(|p| !p.readonly) {
            if !shown.contains(&p.id) {
                hidden.push(format!("{}: {} ({})", e.name, p.name, p.group));
            }
        }
    }
    assert!(hidden.is_empty(), "controls no page shows: {hidden:#?}");
}
