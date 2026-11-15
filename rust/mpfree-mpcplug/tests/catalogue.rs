//! Every MPFree effect and synth could become an MPC plugin: its names
//! must be unique (the MPC's skins find parameters by name) and fit the
//! host's string buffers, and its pages must fit the MPC's limits.

use mpfree_mpcplug::skin::{MAX_PAGES, PAGE_CONTROLS, PAGE_NAME_CHARS};
use mpfree_mpcplug::{host_names, host_params, ui_json};
use mpfree_node::Node;

fn every_node() -> Vec<Box<dyn Node>> {
    let mut nodes: Vec<Box<dyn Node>> = mpfree_fx::registry::EFFECTS
        .iter()
        .map(|e| (e.make)())
        .collect();
    let r = 44_100;
    nodes.push(Box::new(mpfree_synth::Analog::new(r)));
    nodes.push(Box::new(mpfree_synth::Drawbar::new(r)));
    nodes.push(Box::new(mpfree_synth::Drums::new(r)));
    nodes.push(Box::new(mpfree_synth::Fm::new(r)));
    nodes.push(Box::new(mpfree_synth::Keys::new(r)));
    nodes.push(Box::new(mpfree_synth::Pluck::new(r)));
    nodes.push(Box::new(mpfree_synth::Poly::new(r)));
    nodes.push(Box::new(mpfree_synth::Squelch::new(r)));
    nodes
}

#[test]
fn names_are_unique_and_fit() {
    for node in every_node() {
        let params = host_params(node.as_ref());
        let names = host_names(&params);
        let id = node.info().id;
        for (i, n) in names.iter().enumerate() {
            assert!(!n.is_empty() && n.len() < 32, "{id}: name {n:?}");
            assert!(!names[..i].contains(n), "{id}: {n:?} twice");
        }
    }
}

#[test]
fn pages_fit_the_mpc() {
    for node in every_node() {
        let id = node.info().id;
        let ui = ui_json(node.as_ref());
        let pages = ui.matches("\"name\":").count();
        assert!((1..=MAX_PAGES).contains(&pages), "{id}: {pages} pages");
        for page in ui.split("{ \"name\": ").skip(1) {
            let label = page.split('"').nth(1).unwrap();
            assert!(
                label.chars().count() <= PAGE_NAME_CHARS,
                "{id}: page {label:?}"
            );
            let controls = page.matches("\"param\":").count();
            assert!(
                (1..=PAGE_CONTROLS).contains(&controls),
                "{id}: page {label:?} has {controls}"
            );
        }
        // Every shown parameter is on a page unless the pages ran out.
        let shown = ui.matches("\"param\":").count();
        let params = host_params(node.as_ref()).len();
        assert!(
            shown == params || pages == MAX_PAGES,
            "{id}: {shown} of {params} on pages"
        );
    }
}
