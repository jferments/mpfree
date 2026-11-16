//! The effects' page plans (`mpfree_fx::page_plan`) against the screen:
//! every page's layout parses, places each section once in the 844 by 540
//! pixels the page has, and gives each section room for its controls; and
//! every drawing a plan places is one the effect draws (or one the new
//! pages add), with none of today's left out.

use mpfree_fx::main_page::find;
use mpfree_fx::mpfree_node::{Curve, ParamInfo};
use mpfree_fx::page_plan::{Page, Plan, Section, Tier, PLANS, TO_DRAW};
use mpfree_fx::registry;
use mpfree_fx_ui::state::{FxState, Scratch};
use mpfree_fx_ui::visual;
use mpfree_ui::layout;
use mpfree_ui::{Rect, Theme};

/// The page's area: the column between the effects list and the Q-Links,
/// under the header and over the Modulate row.
const PAGE: Rect = Rect {
    x: 0,
    y: 0,
    w: 844,
    h: 540,
};
const GAP: i32 = 12;
/// A section's padding and heading.
const PAD: i32 = 14;
const HEAD: i32 = 24;

/// The room a control takes, by what it is drawn as: (width, height).
fn footprint(plan: &Plan, sec: &Section, name: &str, info: &ParamInfo, inner_w: i32) -> (i32, i32) {
    if sec.faders.contains(&name) {
        (64, 180)
    } else if sec.rows.contains(&name) {
        (inner_w / 2, 48)
    } else if matches!(info.curve, Curve::Choice(_) | Curve::Toggle) {
        (inner_w.min(200), 56)
    } else {
        // A knob with its value and label under it.
        match sec.tier(name, plan.qlinked(name)) {
            Tier::Hero => (136, 180),
            Tier::Main => (96, 130),
            Tier::Std => (80, 104),
            Tier::Trim => (64, 88),
        }
    }
}

fn check_page(plan: &Plan, page: &Page, params: &[ParamInfo], bad: &mut Vec<String>) {
    let at = format!("{} {}", plan.id, page.name);
    let node = layout::parse(page.spec).unwrap_or_else(|e| panic!("{at}: {e}"));
    let mut slots = layout::slots(&node);
    let mut keys: Vec<String> = page.sections.iter().map(|s| s.key.to_string()).collect();
    slots.sort();
    keys.sort();
    assert_eq!(slots, keys, "{at}: the layout's slots are its sections");
    for (key, r) in layout::solve(&node, PAGE, GAP) {
        let sec = page.sections.iter().find(|s| s.key == key).unwrap();
        let (w, h) = (r.w - 2 * PAD, r.h - 2 * PAD - HEAD - GAP);
        if w < 80 || h < 60 {
            bad.push(format!("{at}: {key} is {}x{}", r.w, r.h));
            continue;
        }
        let mut room = (w * h) as i64;
        if !sec.drawings.is_empty() {
            if sec.params.is_empty() {
                continue;
            }
            // The drawings take a band across the section.
            room = (w * (h - (h * 45 / 100).clamp(64, 240))) as i64;
        }
        let mut need = 0i64;
        for name in sec.params {
            let info = find(params, name).unwrap();
            let (cw, ch) = footprint(plan, sec, name, info, w);
            if cw > w || ch > h {
                bad.push(format!("{at}: {name} ({cw}x{ch}) in {key} ({w}x{h})"));
            }
            need += (cw * ch) as i64;
        }
        if need > room {
            bad.push(format!(
                "{at}: {key} needs {need} px² for its controls and has {room} ({}x{})",
                r.w, r.h
            ));
        }
    }
}

#[test]
fn every_page_lays_out_with_room_for_its_controls() {
    let mut bad = Vec::new();
    for plan in PLANS {
        let n = registry::make(plan.id).unwrap();
        for page in plan.pages {
            check_page(plan, page, n.params(), &mut bad);
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The titles of the drawings the effect pages draw for engine `id` today.
fn cards(id: &str) -> Vec<String> {
    if !visual::has_visuals(id) {
        return Vec::new();
    }
    let t = Theme::default();
    let n = registry::make(id).unwrap();
    let st = FxState::from_node(n.as_ref(), "", t.accent, 120.0, 48_000);
    let scratch = Scratch::new(&st);
    visual::strip(&st, "", None, &scratch, &t)
        .cards
        .into_iter()
        .map(|c| c.title)
        .collect()
}

#[test]
fn drawings_are_the_effects_own_and_none_is_left_out() {
    for plan in PLANS {
        let have = cards(plan.id);
        let to_draw: Vec<&str> = TO_DRAW
            .iter()
            .filter(|(id, _)| *id == plan.id)
            .map(|(_, t)| *t)
            .collect();
        for t in &to_draw {
            assert!(
                !have.iter().any(|h| h == t),
                "{}: {t} is drawn now, take it off TO_DRAW",
                plan.id
            );
        }
        let placed: Vec<&str> = plan
            .pages
            .iter()
            .flat_map(|p| p.sections.iter())
            .flat_map(|s| s.drawings.iter().copied())
            .collect();
        for d in &placed {
            assert!(
                have.iter().any(|h| h == d) || to_draw.contains(d),
                "{}: no drawing {d}",
                plan.id
            );
        }
        // Every drawing the effect has is on one of its pages.
        for h in &have {
            assert!(
                placed.contains(&h.as_str()),
                "{}: {h} isn't on any page",
                plan.id
            );
        }
        for t in &to_draw {
            assert!(
                placed.contains(t),
                "{}: {t} on TO_DRAW but not placed",
                plan.id
            );
        }
    }
}
