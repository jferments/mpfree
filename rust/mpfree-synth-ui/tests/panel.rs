//! The Synth tab end to end: touches, Q-Links and the wheel on a Ui laid out
//! at the MPC One's 1280x800, with a real Poly taking the actions.

use mpfree_node::{Curve, Node};
use mpfree_synth::params as pp;
use mpfree_synth::Poly;
use mpfree_synth_ui::sample::{self, app_ui, poly, poly_presets, preset, Example, PAGE};
use mpfree_synth_ui::view::{LAYOUT, PLAIN_LAYOUT, PRESETS, QLINKS, TILES, TITLE, VISUALS};
use mpfree_synth_ui::widgets::{QLinkStrip, TileGrid, TileKind, TitleBar, Visuals};
use mpfree_synth_ui::{SynthAction, SynthPanel, SynthState};
use mpfree_ui::{Color, Fonts, Rect, SoftCanvas, Touch, Ui};

struct Rig {
    ui: Ui,
    panel: SynthPanel,
    node: Poly,
}

fn rig(preset_name: &str) -> Rig {
    let node = poly(preset(preset_name), &[]);
    let st = SynthState::from_node(&node, "Bass", Color::rgb(0x3DDC97), poly_presets(), Some(preset(preset_name)));
    let mut ui = app_ui(Fonts::embedded(), "");
    let panel = SynthPanel::add(&mut ui, PAGE, st).unwrap();
    ui.show(PAGE).unwrap();
    Rig { ui, panel, node }
}

impl Rig {
    /// A finger down, moved, and lifted; every action is applied to the node.
    fn gesture(&mut self, path: &[(i32, i32)]) -> Vec<SynthAction> {
        let mut out = Vec::new();
        for (k, &(x, y)) in path.iter().enumerate() {
            let phase = if k == 0 { Touch::Down } else if k == path.len() - 1 { Touch::Up } else { Touch::Move };
            if let Some(ev) = self.ui.touch(1, phase, x, y) {
                if let Some(a) = self.panel.event(&mut self.ui, &ev) {
                    self.apply(&a);
                    out.push(a);
                }
            }
        }
        out
    }

    fn tap(&mut self, x: i32, y: i32) -> Vec<SynthAction> {
        self.gesture(&[(x, y), (x, y)])
    }

    fn apply(&mut self, a: &SynthAction) {
        match a {
            SynthAction::Set { id, value } => self.node.set_param(*id, *value),
            SynthAction::Preset(i) => self.node.load_preset(&Poly::presets()[*i]),
        }
    }

    fn page(&mut self, name: &str) {
        let i = self.panel.pages().iter().position(|p| p.name == name).unwrap();
        let r = self.ui.rect_of(TITLE).unwrap();
        let tab = self.ui.get::<TitleBar>(TITLE).unwrap().tab_rect(r, i);
        self.tap(tab.x + tab.w / 2, tab.y + tab.h / 2);
        assert_eq!(self.panel.page(), i);
    }

    /// The tile showing parameter `id` on the current page.
    fn tile(&self, id: u16) -> Rect {
        let page = &self.panel.pages()[self.panel.page()];
        let k = page.params.iter().position(|&i| self.panel.state.params[i].id == id).expect("on this page");
        self.ui.get::<TileGrid>(TILES).unwrap().tile_rect(self.ui.rect_of(TILES).unwrap(), k)
    }
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

#[test]
fn dragging_a_knob_up_turns_it_up_on_the_node() {
    let mut r = rig("Grit Bass");
    r.page("Filter");
    let before = r.node.param(pp::CUTOFF);
    let (x, y) = centre(r.tile(pp::CUTOFF));
    let acts = r.gesture(&[(x, y), (x, y - 40), (x, y - 80), (x, y - 80)]);
    assert!(!acts.is_empty());
    let after = r.node.param(pp::CUTOFF);
    assert!(after > before * 1.5, "{before} -> {after}");
    assert_eq!(r.panel.focus(), Some(pp::CUTOFF));
    assert_eq!(r.panel.state.value(pp::CUTOFF), after, "the panel's copy follows");
    assert!(r.panel.state.edited);
    // Dragging down past the bottom stops at the minimum.
    r.gesture(&[(x, y), (x, y + 2000), (x, y + 2000)]);
    assert_eq!(r.node.param(pp::CUTOFF), 20.0);
}

#[test]
fn choices_and_switches_change_on_a_tap() {
    let mut r = rig("Grit Bass");
    // Voice mode's three choices fit as buttons: a tap picks one.
    r.page("Voice");
    let parts = TileGrid::choice_rects(r.tile(pp::VOICE_MODE), 3);
    assert_eq!(parts.len(), 3);
    let (x, y) = centre(parts[0]);
    assert_eq!(r.tap(x, y), vec![SynthAction::Set { id: pp::VOICE_MODE, value: 0.0 }]);
    // Tapping the chosen one again changes nothing.
    assert!(r.tap(x, y).is_empty());
    // The filter type's four don't fit across its tile, so it has arrows.
    r.page("Filter");
    let parts = TileGrid::choice_rects(r.tile(pp::FILTER_MODE), 4);
    assert_eq!(parts.len(), 2);
    let (x, y) = centre(parts[1]);
    assert_eq!(r.tap(x, y), vec![SynthAction::Set { id: pp::FILTER_MODE, value: 1.0 }]);

    r.page("LFO");
    let (x, y) = centre(r.tile(pp::LFO_RETRIG));
    assert_eq!(r.tap(x, y), vec![SynthAction::Set { id: pp::LFO_RETRIG, value: 1.0 }]);
    assert_eq!(r.tap(x, y), vec![SynthAction::Set { id: pp::LFO_RETRIG, value: 0.0 }]);
    // Sync has eleven choices, so it has arrows.
    let sync = r.tile(pp::LFO_SYNC);
    let arrows = TileGrid::choice_rects(sync, pp::LFO_SYNCS.len());
    let (x, y) = centre(arrows[1]);
    assert_eq!(r.tap(x, y), vec![SynthAction::Set { id: pp::LFO_SYNC, value: 1.0 }]);
    let (x, y) = centre(arrows[0]);
    assert_eq!(r.tap(x, y), vec![SynthAction::Set { id: pp::LFO_SYNC, value: 0.0 }]);
    assert!(r.tap(x, y).is_empty(), "already at the first choice");
}

#[test]
fn a_finger_that_slides_off_a_choice_does_not_change_it() {
    let mut r = rig("Init");
    r.page("Voice");
    let parts = TileGrid::choice_rects(r.tile(pp::VOICE_MODE), 3);
    let (x, y) = centre(parts[2]);
    let far = centre(r.tile(pp::GLIDE));
    assert!(r.gesture(&[(x, y), far, far]).is_empty());
    assert_eq!(r.node.param(pp::VOICE_MODE), 0.0);
}

#[test]
fn qlinks_and_the_wheel_turn_in_detents() {
    let mut r = rig("Init");
    r.page("Filter");
    let p = *pp::info(pp::CUTOFF).unwrap();
    let start = r.panel.state.value(pp::CUTOFF);
    // Q-Link 1 is Cutoff on the Filter page: ten detents is a tenth of the range.
    let a = r.panel.qlink(&mut r.ui, 0, 10).unwrap();
    r.apply(&a);
    let SynthAction::Set { id, value } = a else { panic!() };
    assert_eq!(id, pp::CUTOFF);
    assert!((p.to_unit(value) - p.to_unit(start) - 0.1).abs() < 1e-4);
    // The wheel turns the selected tile, here Drive after a tap.
    let (x, y) = centre(r.tile(pp::DRIVE));
    r.tap(x, y);
    let a = r.panel.wheel(&mut r.ui, 5).unwrap();
    assert!(matches!(a, SynthAction::Set { id: pp::DRIVE, .. }));
    // A stepped parameter moves one step a detent.
    r.page("Voice");
    let (x, y) = centre(r.tile(pp::POLYPHONY));
    r.tap(x, y);
    assert_eq!(r.panel.wheel(&mut r.ui, 1), Some(SynthAction::Set { id: pp::POLYPHONY, value: 9.0 }));
    assert_eq!(r.panel.wheel(&mut r.ui, -3), Some(SynthAction::Set { id: pp::POLYPHONY, value: 6.0 }));
    // Past the end, nothing more.
    assert!(r.panel.wheel(&mut r.ui, 100).is_some());
    assert_eq!(r.panel.wheel(&mut r.ui, 1), None);
    // A Q-Link the page doesn't use does nothing.
    assert_eq!(r.panel.qlink(&mut r.ui, 7, 1), None);
}

#[test]
fn the_qlink_strip_shows_what_the_qlinks_turn() {
    let mut r = rig("Grit Bass");
    r.page("Osc");
    let labels: Vec<String> = r.ui.get::<QLinkStrip>(QLINKS).unwrap().cells.iter().map(|c| c.label.clone()).collect();
    assert_eq!(labels, ["Fine", "Level", "Sub", "Detune"]);
    let tiles = &r.ui.get::<TileGrid>(TILES).unwrap().tiles;
    let marked: Vec<usize> = tiles.iter().filter_map(|t| t.qlink).collect();
    assert_eq!(marked.len(), 4);
    // Tapping a Q-Link cell selects its tile, so the wheel turns it.
    let strip = r.ui.rect_of(QLINKS).unwrap();
    let (x, y) = centre(QLinkStrip::cell_rect(strip, 2));
    r.tap(x, y);
    assert_eq!(r.panel.focus(), Some(pp::SUB_LEVEL));
}

#[test]
fn presets_load_through_the_app() {
    let mut r = rig("Init");
    let list = r.ui.rect_of(PRESETS).unwrap();
    // Rows are 46 px under the 34 px title, inside 14 px of padding.
    let row = |k: i32| (list.x + 60, list.y + 14 + 34 + k * 46 + 20);
    let (x, y) = row(2);
    assert_eq!(r.tap(x, y), vec![SynthAction::Preset(2)]);
    assert_eq!(r.node.param(pp::VOICE_MODE), Poly::presets()[2].values.iter().find(|v| v.0 == pp::VOICE_MODE).unwrap().1);
    // The app reloads the panel from the node after a preset.
    let st = SynthState::from_node(&r.node, "Bass", Color::rgb(0x3DDC97), poly_presets(), Some(2));
    r.panel.load(&mut r.ui, st).unwrap();
    assert!(r.ui.get::<TitleBar>(TITLE).unwrap().title.ends_with(Poly::presets()[2].name));
}

#[test]
fn every_poly_parameter_can_be_reached_and_set() {
    let mut r = rig("Init");
    for (i, p) in pp::PARAMS.iter().enumerate() {
        let page = r.panel.pages().iter().position(|pg| pg.params.contains(&i)).expect("on a page");
        r.panel.show_page(&mut r.ui, page);
        let (x, y) = centre(r.tile(p.id));
        r.tap(x, y);
        let turned = r.panel.wheel(&mut r.ui, 1).or_else(|| r.panel.wheel(&mut r.ui, -1));
        if p.readonly {
            assert_eq!(turned, None, "{} is a meter", p.name);
            let tile = r.ui.get::<TileGrid>(TILES).unwrap().tiles.iter().find(|t| t.kind == TileKind::Meter).cloned();
            assert!(tile.is_some());
        } else {
            let Some(SynthAction::Set { id, value }) = turned else { panic!("{} did not turn", p.name) };
            assert_eq!(id, p.id);
            assert_eq!(value, p.clamp(value));
            r.apply(&SynthAction::Set { id, value });
            assert_eq!(r.node.param(id), value, "{}", p.name);
        }
    }
}

#[test]
fn the_cards_follow_the_page_and_open_their_page() {
    let mut r = rig("Grit Bass");
    r.page("Env");
    let cards = &r.ui.get::<Visuals>(VISUALS).unwrap().cards;
    assert_eq!(cards.iter().filter(|c| c.active).map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Env"]);
    let strip = r.ui.rect_of(VISUALS).unwrap();
    let card = r.ui.get::<Visuals>(VISUALS).unwrap().card_rect(strip, 3);
    let (x, y) = centre(card);
    r.tap(x, y);
    assert_eq!(r.panel.pages()[r.panel.page()].name, "LFO");
    // SHIFT + wheel steps through the pages and wraps.
    r.panel.step_page(&mut r.ui, 1);
    assert_eq!(r.panel.pages()[r.panel.page()].name, "Voice");
    r.panel.step_page(&mut r.ui, 1);
    assert_eq!(r.panel.pages()[r.panel.page()].name, "Osc");
}

#[test]
fn a_node_without_a_plan_gets_the_plain_page() {
    let node = Example::new();
    let st = SynthState::from_node(&node, "", Color::rgb(0x6EA8FE), Vec::new(), None);
    let mut ui = app_ui(Fonts::embedded(), "");
    let panel = SynthPanel::add(&mut ui, PAGE, st).unwrap();
    ui.show(PAGE).unwrap();
    assert_eq!(panel.layout(), PLAIN_LAYOUT);
    assert!(ui.rect_of(VISUALS).is_none());
    let names: Vec<&str> = panel.pages().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Delay", "Output"]);
    let mut c = SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
}

#[test]
fn every_sample_lays_out_inside_the_screen_and_paints() {
    let screen = Rect::new(0, 0, sample::WIDTH, sample::HEIGHT);
    for mut s in sample::samples(Fonts::embedded).unwrap() {
        assert!(s.panel.layout() == LAYOUT || s.panel.layout() == PLAIN_LAYOUT);
        let tiles_r = s.ui.rect_of(TILES).unwrap();
        assert_eq!(tiles_r.intersect(screen), tiles_r);
        let grid = s.ui.get::<TileGrid>(TILES).unwrap();
        for k in 0..grid.tiles.len() {
            let t = grid.tile_rect(tiles_r, k);
            assert_eq!(t.intersect(tiles_r), t, "{}: tile {k} spills out", s.name);
            assert!(t.h >= 56, "{}: tile {k} is {} px tall", s.name, t.h);
        }
        let mut c = SoftCanvas::new(sample::WIDTH, sample::HEIGHT);
        assert_eq!(s.ui.paint(&mut c), vec![screen]);
        assert!(!s.ui.needs_paint(), "{}: nothing animates", s.name);
        // A turn redraws only the panel's widgets, not the app's header.
        if let Some(a) = s.panel.wheel(&mut s.ui, 1) {
            let _ = a;
            let damage = s.ui.paint(&mut c);
            let header = s.ui.rect_of("header").unwrap();
            assert!(damage.iter().all(|d| d.intersect(header).is_empty()), "{}", s.name);
        }
    }
}

#[test]
fn stepped_and_choice_values_are_whole_numbers_after_a_drag() {
    let mut r = rig("Init");
    r.page("Voice");
    let (x, y) = centre(r.tile(pp::POLYPHONY));
    for a in r.gesture(&[(x, y), (x, y - 33), (x, y - 71), (x, y - 71)]) {
        let SynthAction::Set { id, value } = a else { panic!() };
        let p = pp::info(id).unwrap();
        assert!(matches!(p.curve, Curve::Stepped));
        assert_eq!(value, value.round());
    }
}
