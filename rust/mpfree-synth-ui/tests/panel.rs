//! The Synth tab end to end: touches, Q-Links and the wheel on a Ui laid out
//! at the MPC One's 1280x800, with a real Poly taking the actions.

use mpfree_node::{Curve, Node};
use mpfree_synth::params as pp;
use mpfree_synth::Poly;
use mpfree_synth_ui::sample::{self, app_ui, poly, poly_presets, preset, Example, PAGE};
use mpfree_synth_ui::section_view::{
    plot_rect, wells, CtlGeom, Geom, Kind, SectionBox, SectionView,
};
use mpfree_synth_ui::view::{INST, LAYOUT, PRESETS, QLINK_COLUMN, TILES, TITLE};
use mpfree_synth_ui::widgets::{node_rect, Card, TitleBar};
use mpfree_synth_ui::{SynthAction, SynthPanel, SynthState};
use mpfree_ui::text::FontId;
use mpfree_ui::{Color, EventKind, Fonts, Rect, SoftCanvas, Touch, Ui, UiEvent};

struct Rig {
    ui: Ui,
    panel: SynthPanel,
    node: Poly,
}

fn rig(preset_name: &str) -> Rig {
    let node = poly(preset(preset_name), &[]);
    let st = SynthState::from_node(
        &node,
        "Bass",
        Color::rgb(0x3DDC97),
        poly_presets(),
        Some(preset(preset_name)),
    );
    let mut ui = app_ui(Fonts::embedded(), "");
    let panel = SynthPanel::add(&mut ui, PAGE, st).unwrap();
    ui.show(PAGE).unwrap();
    Rig { ui, panel, node }
}

fn sample(name: &str) -> sample::Sample {
    sample::samples(Fonts::embedded)
        .unwrap()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap()
}

fn view(ui: &Ui) -> &SectionView {
    ui.get::<SectionView>(TILES).unwrap()
}

/// The page's drawings, section by section.
fn cards(ui: &Ui) -> Vec<Card> {
    view(ui)
        .sections
        .iter()
        .flat_map(|s| s.cards.iter().cloned())
        .collect()
}

fn titles(ui: &Ui) -> Vec<String> {
    cards(ui).into_iter().map(|c| c.title).collect()
}

/// Section `key` and where it lies.
fn section(ui: &Ui, key: &str) -> (SectionBox, Geom) {
    let r = ui.rect_of(TILES).unwrap();
    let (s, g) = view(ui).section(r, key).expect("on this page");
    (s.clone(), g)
}

fn page_name(panel: &SynthPanel) -> &str {
    &panel.pages()[panel.page()].name
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// A finger down and up at (x, y), straight to the panel.
fn tap_panel(ui: &mut Ui, panel: &mut SynthPanel, (x, y): (i32, i32)) -> Vec<SynthAction> {
    let mut out = Vec::new();
    for phase in [Touch::Down, Touch::Up] {
        if let Some(ev) = ui.touch(1, phase, x, y) {
            out.extend(panel.event(ui, &ev));
        }
    }
    out
}

impl Rig {
    /// A finger down, moved, and lifted; every action is applied to the node.
    fn gesture(&mut self, path: &[(i32, i32)]) -> Vec<SynthAction> {
        let mut out = Vec::new();
        for (k, &(x, y)) in path.iter().enumerate() {
            let phase = if k == 0 {
                Touch::Down
            } else if k == path.len() - 1 {
                Touch::Up
            } else {
                Touch::Move
            };
            if let Some(ev) = self.ui.touch(1, phase, x, y) {
                if let Some(a) = self.panel.event(&mut self.ui, &ev) {
                    self.apply(&a);
                    out.push(a);
                }
            }
        }
        out
    }

    fn tap(&mut self, (x, y): (i32, i32)) -> Vec<SynthAction> {
        self.gesture(&[(x, y), (x, y)])
    }

    fn apply(&mut self, a: &SynthAction) {
        match a {
            SynthAction::Set { id, value } => self.node.set_param(*id, *value),
            SynthAction::Preset(i) => self.node.load_preset(&Poly::presets()[*i]),
            // The app opens the browser's preset picker, the list of
            // synths or the options; nothing changes here.
            SynthAction::OpenPresets | SynthAction::OpenSynths | SynthAction::Options => {}
        }
    }

    fn page(&mut self, name: &str) {
        let i = self
            .panel
            .pages()
            .iter()
            .position(|p| p.name == name)
            .unwrap();
        let r = self.ui.rect_of(TITLE).unwrap();
        let tab = self.ui.get::<TitleBar>(TITLE).unwrap().tab_rect(r, i);
        self.tap(centre(tab));
        assert_eq!(self.panel.page(), i);
    }

    /// The control showing parameter `id` on the current page, and where
    /// it lies.
    fn ctl(&self, id: u16) -> (Kind, CtlGeom) {
        let i = self.panel.state.index(id).unwrap();
        let r = self.ui.rect_of(TILES).unwrap();
        let (c, g) = view(&self.ui).control_geom(r, i).expect("on this page");
        (c.kind.clone(), g)
    }

    /// Selects parameter `id`'s control with a tap away from its buttons.
    fn select(&mut self, id: u16) {
        let (_, g) = self.ctl(id);
        let free = |r: Rect| {
            let (x, y) = centre(r);
            (r.w > 0 && g.cell.contains(x, y) && !g.parts.iter().any(|p| p.contains(x, y)))
                .then_some((x, y))
        };
        let at = free(g.label)
            .or_else(|| free(g.value))
            .or_else(|| free(g.cell))
            .unwrap_or((g.cell.x + 2, g.cell.y + 2));
        self.tap(at);
        assert_eq!(self.panel.focus(), Some(id));
    }
}

#[test]
fn dragging_a_knob_up_turns_it_up_on_the_node() {
    let mut r = rig("Grit Bass");
    r.page("Filter");
    let before = r.node.param(pp::CUTOFF);
    let (kind, g) = r.ctl(pp::CUTOFF);
    assert!(matches!(kind, Kind::Knob { .. }));
    let (x, y) = centre(g.cell);
    let acts = r.gesture(&[(x, y), (x, y - 40), (x, y - 80), (x, y - 80)]);
    assert!(!acts.is_empty());
    let after = r.node.param(pp::CUTOFF);
    assert!(after > before * 1.5, "{before} -> {after}");
    assert_eq!(r.panel.focus(), Some(pp::CUTOFF));
    assert_eq!(
        r.panel.state.value(pp::CUTOFF),
        after,
        "the panel's copy follows"
    );
    assert!(r.panel.state.edited);
    // Dragging down past the bottom stops at the minimum.
    r.gesture(&[(x, y), (x, y + 2000), (x, y + 2000)]);
    assert_eq!(r.node.param(pp::CUTOFF), 20.0);
}

#[test]
fn a_fader_drags_over_its_own_track() {
    let mut r = rig("Init");
    r.page("Env");
    let (kind, g) = r.ctl(pp::AMP_S);
    assert!(matches!(kind, Kind::Fader { .. }));
    let p = *pp::info(pp::AMP_S).unwrap();
    let start = p.to_unit(r.node.param(pp::AMP_S));
    let (x, y) = (g.track.x + g.track.w / 2, g.track.y + g.track.h / 2);
    // A quarter of the track down is a quarter of the range down.
    let dy = g.track.h / 4;
    r.gesture(&[(x, y), (x, y + dy), (x, y + dy)]);
    let now = p.to_unit(r.node.param(pp::AMP_S));
    assert!((start - now - 0.25).abs() < 0.02, "{start} -> {now}");
}

#[test]
fn choices_and_switches_change_on_a_tap() {
    let mut r = rig("Grit Bass");
    // Voice mode's three choices fit as buttons: a tap picks one.
    r.page("Voice");
    let (_, g) = r.ctl(pp::VOICE_MODE);
    assert!(!g.chip);
    assert_eq!(g.parts.len(), 3);
    assert_eq!(
        r.tap(centre(g.parts[0])),
        vec![SynthAction::Set {
            id: pp::VOICE_MODE,
            value: 0.0
        }]
    );
    // Tapping the chosen one again changes nothing.
    assert!(r.tap(centre(g.parts[0])).is_empty());
    // The filter type's four fit across its section too.
    r.page("Filter");
    let (_, g) = r.ctl(pp::FILTER_MODE);
    assert_eq!(g.parts.len(), 4);
    assert_eq!(
        r.tap(centre(g.parts[1])),
        vec![SynthAction::Set {
            id: pp::FILTER_MODE,
            value: 1.0
        }]
    );

    r.page("LFO");
    let (kind, g) = r.ctl(pp::LFO_RETRIG);
    assert_eq!(kind, Kind::Toggle(false));
    assert_eq!(
        r.tap(centre(g.cell)),
        vec![SynthAction::Set {
            id: pp::LFO_RETRIG,
            value: 1.0
        }]
    );
    assert_eq!(
        r.tap(centre(g.cell)),
        vec![SynthAction::Set {
            id: pp::LFO_RETRIG,
            value: 0.0
        }]
    );
    // Sync has eleven choices, so it is a chip with arrows.
    let (_, g) = r.ctl(pp::LFO_SYNC);
    assert!(g.chip);
    assert_eq!(
        r.tap(centre(g.parts[1])),
        vec![SynthAction::Set {
            id: pp::LFO_SYNC,
            value: 1.0
        }]
    );
    assert_eq!(
        r.tap(centre(g.parts[0])),
        vec![SynthAction::Set {
            id: pp::LFO_SYNC,
            value: 0.0
        }]
    );
    assert!(
        r.tap(centre(g.parts[0])).is_empty(),
        "already at the first choice"
    );
}

/// A tap on the preset's name above the page asks for the engine's
/// presets (layout spec 4g); the page tabs beside it still change page.
#[test]
fn a_tap_on_the_presets_name_opens_the_engines_presets() {
    let mut r = rig("Pluck");
    let rect = r.ui.rect_of(TITLE).unwrap();
    let bar = r.ui.get::<TitleBar>(TITLE).unwrap();
    assert!(bar.opens, "an engine's name opens its presets");
    assert_eq!(bar.title, "Pluck");
    let name = bar.name_rect(rect);
    assert!(name.right() <= bar.tab_rect(rect, 0).x, "clear of the tabs");
    // The gear at the right end (Jay's device notes, 20 Nov 2026): the
    // options, clear of the tabs.
    let gear = bar.gear_rect(rect).expect("the gear");
    let last = bar.tab_rect(rect, bar.tabs.len() - 1);
    assert!(
        last.right() < gear.x && gear.right() <= rect.right(),
        "{last:?} {gear:?}"
    );
    assert_eq!(r.tap(centre(name)), vec![SynthAction::OpenPresets]);
    assert_eq!(r.tap(centre(gear)), vec![SynthAction::Options]);
    // The instrument card: the list of synths.
    let card = r.ui.rect_of(mpfree_synth_ui::view::INST).unwrap();
    assert_eq!(r.tap(centre(card)), vec![SynthAction::OpenSynths]);
    r.page("Filter");
}

#[test]
fn a_stepper_steps_by_one_and_stops_at_its_ends() {
    let mut r = rig("Init");
    r.page("Voice");
    let (kind, g) = r.ctl(pp::POLYPHONY);
    assert_eq!(kind, Kind::Stepper);
    let start = r.node.param(pp::POLYPHONY);
    assert_eq!(
        r.tap(centre(g.parts[1])),
        vec![SynthAction::Set {
            id: pp::POLYPHONY,
            value: start + 1.0
        }]
    );
    let p = pp::info(pp::POLYPHONY).unwrap();
    for _ in 0..64 {
        r.tap(centre(g.parts[0]));
    }
    assert_eq!(r.node.param(pp::POLYPHONY), p.min);
    assert!(r.tap(centre(g.parts[0])).is_empty());
}

#[test]
fn a_finger_that_slides_off_a_choice_does_not_change_it() {
    let mut r = rig("Init");
    r.page("Voice");
    let (_, g) = r.ctl(pp::VOICE_MODE);
    let from = centre(g.parts[2]);
    let far = centre(r.ctl(pp::GLIDE).1.cell);
    assert!(r.gesture(&[from, far, far]).is_empty());
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
    let SynthAction::Set { id, value } = a else {
        panic!()
    };
    assert_eq!(id, pp::CUTOFF);
    assert!((p.to_unit(value) - p.to_unit(start) - 0.1).abs() < 1e-4);
    // The wheel turns the selected control, here Drive after a tap.
    r.select(pp::DRIVE);
    let a = r.panel.wheel(&mut r.ui, 5).unwrap();
    assert!(matches!(a, SynthAction::Set { id: pp::DRIVE, .. }));
    // A stepped parameter moves one step a detent.
    r.page("Voice");
    r.select(pp::POLYPHONY);
    assert_eq!(
        r.panel.wheel(&mut r.ui, 1),
        Some(SynthAction::Set {
            id: pp::POLYPHONY,
            value: 9.0
        })
    );
    assert_eq!(
        r.panel.wheel(&mut r.ui, -3),
        Some(SynthAction::Set {
            id: pp::POLYPHONY,
            value: 6.0
        })
    );
    // Past the end, nothing more.
    assert!(r.panel.wheel(&mut r.ui, 100).is_some());
    assert_eq!(r.panel.wheel(&mut r.ui, 1), None);
    // A Q-Link the page doesn't use does nothing.
    assert_eq!(r.panel.qlink(&mut r.ui, 7, 1), None);
}

#[test]
fn the_qlink_column_shows_what_the_qlinks_turn() {
    let mut r = rig("Grit Bass");
    r.page("Osc");
    let cards = r.panel.qlink_cards();
    let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Fine", "Osc 2", "Sub", "Noise"]);
    // Each card names the section its control sits in.
    for c in &cards {
        assert!(
            view(&r.ui).sections.iter().any(|s| s.title == c.scope),
            "{}: {}",
            c.name,
            c.scope
        );
    }
    // The four controls carry their Q-Link's badge, Q1 to Q4.
    let mut marked: Vec<usize> = view(&r.ui)
        .sections
        .iter()
        .flat_map(|s| s.controls.iter().filter_map(|c| c.qlink))
        .collect();
    marked.sort();
    assert_eq!(marked, [0, 1, 2, 3]);
    // The app passes a drag on card 3 to the panel: it turns Sub.
    let col = r.ui.rect_of(QLINK_COLUMN).unwrap();
    assert_eq!(col.w, mpfree_ui::qlink::COLUMN_WIDTH);
    let a = r.panel.qlink(&mut r.ui, 2, -4).unwrap();
    assert!(matches!(
        a,
        SynthAction::Set {
            id: pp::SUB_LEVEL,
            ..
        }
    ));
    assert_eq!(r.panel.focus(), Some(pp::SUB_LEVEL));
}

#[test]
fn presets_load_through_the_app() {
    let mut r = rig("Init");
    let list = r.ui.rect_of(PRESETS).unwrap();
    assert_eq!(list.w, 220);
    // Rows are 46 px under the 34 px title, inside 14 px of padding.
    let row = |k: i32| (list.x + 60, list.y + 14 + 34 + k * 46 + 20);
    assert_eq!(r.tap(row(2)), vec![SynthAction::Preset(2)]);
    assert_eq!(
        r.node.param(pp::VOICE_MODE),
        Poly::presets()[2]
            .values
            .iter()
            .find(|v| v.0 == pp::VOICE_MODE)
            .unwrap()
            .1
    );
    // The app reloads the panel from the node after a preset.
    let st = SynthState::from_node(
        &r.node,
        "Bass",
        Color::rgb(0x3DDC97),
        poly_presets(),
        Some(2),
    );
    r.panel.load(&mut r.ui, st).unwrap();
    assert!(r
        .ui
        .get::<TitleBar>(TITLE)
        .unwrap()
        .title
        .ends_with(Poly::presets()[2].name));
}

#[test]
fn every_poly_parameter_can_be_reached_and_set() {
    let mut r = rig("Init");
    for (i, p) in pp::PARAMS.iter().enumerate() {
        let page = r
            .panel
            .pages()
            .iter()
            .position(|pg| pg.params.contains(&i))
            .expect("on a page");
        r.panel.show_page(&mut r.ui, page);
        let (kind, g) = r.ctl(p.id);
        assert!(g.cell.w > 0 && g.cell.h > 0, "{} has no room", p.name);
        if p.readonly {
            assert_eq!(kind, Kind::Meter, "{} is a meter", p.name);
            assert!(!r.panel.order().contains(&i), "the wheel skips meters");
            continue;
        }
        r.select(p.id);
        let turned = r
            .panel
            .wheel(&mut r.ui, 1)
            .or_else(|| r.panel.wheel(&mut r.ui, -1));
        let Some(SynthAction::Set { id, value }) = turned else {
            panic!("{} did not turn", p.name)
        };
        assert_eq!(id, p.id);
        assert_eq!(value, p.clamp(value));
        r.apply(&SynthAction::Set { id, value });
        assert_eq!(r.node.param(id), value, "{}", p.name);
    }
}

#[test]
fn the_wheel_picks_controls_in_section_order() {
    let mut r = rig("Init");
    r.page("Filter");
    let order = r.panel.order();
    assert!(order.len() > 4);
    r.panel.pick_tile(&mut r.ui, 1);
    assert_eq!(view(&r.ui).selected, Some(order[0]));
    r.panel.pick_tile(&mut r.ui, 3);
    assert_eq!(view(&r.ui).selected, Some(order[3]));
    r.panel.pick_tile(&mut r.ui, 100);
    assert_eq!(view(&r.ui).selected, order.last().copied());
    r.panel.set_editing(&mut r.ui, true);
    assert!(view(&r.ui).editing);
}

#[test]
fn a_section_opens_the_page_it_links_to() {
    // The filter envelope on Analog's Filter page opens Envelopes.
    let mut s = sample("synth-analog-filter");
    assert_eq!(page_name(&s.panel), "Filter");
    let (fenv, g) = section(&s.ui, "fenv");
    assert!(fenv.link.is_some());
    assert!(tap_panel(&mut s.ui, &mut s.panel, centre(g.head)).is_empty());
    assert_eq!(page_name(&s.panel), "Envelopes");
    // There the same section has no link: it is the page.
    assert!(section(&s.ui, "fenv").0.link.is_none());
    // SHIFT + wheel steps through the pages and wraps.
    let mut r = rig("Grit Bass");
    r.page("LFO 2");
    r.panel.step_page(&mut r.ui, 1);
    assert_eq!(page_name(&r.panel), "Voice");
    r.panel.step_page(&mut r.ui, 1);
    assert_eq!(page_name(&r.panel), "Routes");
    r.panel.step_page(&mut r.ui, 1);
    assert_eq!(page_name(&r.panel), "Pressure");
    r.panel.step_page(&mut r.ui, 1);
    assert_eq!(page_name(&r.panel), "Osc");
    r.panel.step_page(&mut r.ui, -1);
    assert_eq!(page_name(&r.panel), "Pressure");
}

/// Taps the title bar's tab called `name`.
fn tap_tab(ui: &mut Ui, panel: &mut SynthPanel, name: &str) {
    let bar = ui.get::<TitleBar>(TITLE).unwrap();
    let i = bar
        .tabs
        .iter()
        .position(|t| t == name)
        .unwrap_or_else(|| panic!("no tab {name} in {:?}", bar.tabs));
    let r = ui.rect_of(TITLE).unwrap();
    let at = centre(bar.tab_rect(r, i));
    tap_panel(ui, panel, at);
}

fn active_tab(ui: &Ui) -> String {
    let bar = ui.get::<TitleBar>(TITLE).unwrap();
    bar.tabs[bar.active].clone()
}

#[test]
fn drums_show_one_effects_tab_for_the_pad_shown_last() {
    use mpfree_synth::drum_fx::{field as x, fx_id};
    let mut s = sample("synth-drums-effects");
    assert_eq!(page_name(&s.panel), "Pad 2 effects");
    let tabs = s.ui.get::<TitleBar>(TITLE).unwrap().tabs.clone();
    let mut want: Vec<String> = (1..=16).map(|n| n.to_string()).collect();
    want.extend(["Kit", "Effects", "Sends"].map(String::from));
    assert_eq!(tabs, want, "sixteen effects pages share one tab");
    assert_eq!(active_tab(&s.ui), "Effects");
    // The kit's picture rings the pad whose effects show and names it.
    let (kit, g) = section(&s.ui, "kit");
    let card = kit.cards[0].clone();
    assert!(card.caption.starts_with("Pad 2 · "), "{}", card.caption);
    let ringed: Vec<usize> = (0..16).filter(|&p| card.nodes[p].ring.is_some()).collect();
    assert_eq!(ringed, [1]);
    // A tap on pad 5's box opens pad 5's effects.
    let d = g.drawing.unwrap();
    let plot = plot_rect(wells(d, 1)[0]);
    let at = centre(node_rect(plot, &card.nodes[4]));
    tap_panel(&mut s.ui, &mut s.panel, at);
    assert_eq!(page_name(&s.panel), "Pad 5 effects");
    assert_eq!(active_tab(&s.ui), "Effects");
    let i = s.panel.state.index(fx_id(4, x::PUNCH)).unwrap();
    assert!(s.panel.pages()[s.panel.page()].params.contains(&i));
    // Pad 9's own page, then the Effects tab: pad 9's effects.
    tap_tab(&mut s.ui, &mut s.panel, "9");
    assert_eq!(page_name(&s.panel), "Pad 9");
    tap_tab(&mut s.ui, &mut s.panel, "Effects");
    assert_eq!(page_name(&s.panel), "Pad 9 effects");
    // The Kit page leaves the pad as it was.
    tap_tab(&mut s.ui, &mut s.panel, "Kit");
    tap_tab(&mut s.ui, &mut s.panel, "Effects");
    assert_eq!(page_name(&s.panel), "Pad 9 effects");
    // SHIFT + wheel steps through the tabs, not the sixteen pages.
    s.panel.step_page(&mut s.ui, 1);
    assert_eq!(page_name(&s.panel), "Sends");
    s.panel.step_page(&mut s.ui, 1);
    assert_eq!(page_name(&s.panel), "Pad 1");
    s.panel.step_page(&mut s.ui, -1);
    assert_eq!(page_name(&s.panel), "Sends");
    s.panel.step_page(&mut s.ui, -1);
    assert_eq!(page_name(&s.panel), "Pad 1 effects");
    s.panel.step_page(&mut s.ui, -1);
    assert_eq!(page_name(&s.panel), "Kit");
    // Faint where it does nothing: the compressor's times at 1:1, the
    // distortion's tone with no distortion, the sends with nothing sent.
    let faint = |s: &sample::Sample, id: u16| {
        mpfree_synth_ui::view::inactive(&s.panel.state, s.panel.state.index(id).unwrap())
    };
    assert_eq!(faint(&s, fx_id(0, x::COMP_ATTACK)), Some("ratio is 1:1"));
    assert_eq!(faint(&s, fx_id(0, x::DIST_TONE)), Some("distortion is off"));
    assert_eq!(faint(&s, fx_id(0, x::MID_FREQ)), Some("mid is flat"));
    assert_eq!(
        faint(&s, fx_id(1, x::COMP_ATTACK)),
        None,
        "pad 2 compresses"
    );
    assert_eq!(faint(&s, fx_id(1, x::DIST_TONE)), None);
    use mpfree_synth::drum_fx::kit as k;
    assert_eq!(
        faint(&s, k::REVERB_DECAY),
        None,
        "pad 2 sends to the reverb"
    );
    assert_eq!(faint(&s, k::DELAY_FEEDBACK), Some("no drum sends to it"));
}

#[test]
fn the_pressure_page_draws_the_curve_and_live_bars() {
    let mut r = rig("Brass");
    r.page("Pressure");
    assert_eq!(titles(&r.ui), ["Pressure", "Voices"]);
    // The app passes in each sounding voice's pressure.
    sample::press_chord(&mut r.node);
    let mut out = [0.0; 16];
    let n = r.node.pressures(&mut out);
    assert_eq!(n, 4);
    r.panel.set_pressures(&mut r.ui, &out[..n]);
    let c = cards(&r.ui);
    assert_eq!(c[1].caption, "4 sounding");
    assert_eq!(section(&r.ui, "voices").0.note, "4 sounding");
    assert!(c[1].bars[0] > 0.8 && c[1].bars[2] == 0.0, "{:?}", c[1].bars);
    // The curve is flat to the threshold, then rises to 1 at full.
    let curve = &c[0].traces[0].pts;
    assert!(curve.iter().filter(|p| p.0 < 0.08).all(|p| p.1 == 0.0));
    assert!(curve
        .iter()
        .filter(|p| p.0 >= 0.85)
        .all(|p| (p.1 - 1.0).abs() < 1e-6));
    // Slot depths read in their targets' units.
    let (slots, _) = section(&r.ui, "slots");
    let depth = |label: &str| {
        slots
            .controls
            .iter()
            .find(|c| c.label == label)
            .map(|c| format!("{} {}", c.value, c.unit))
            .unwrap()
    };
    assert_eq!(depth("Depth 1"), "9 dB swell");
    assert_eq!(depth("Depth 2"), "+1.5 oct");
    assert_eq!(depth("Depth 3"), "— ");
    // Turning the threshold redraws the curve's flat start.
    r.select(pp::PRESS_THRESHOLD);
    // 20 detents of half a percent each: 8 % to 18 %.
    for _ in 0..20 {
        if let Some(a) = r.panel.wheel(&mut r.ui, 1) {
            r.apply(&a);
        }
    }
    assert!((r.node.param(pp::PRESS_THRESHOLD) - 18.0).abs() < 1e-3);
    assert!(cards(&r.ui)[0].traces[0]
        .pts
        .iter()
        .filter(|p| p.0 < 0.18)
        .all(|p| p.1 == 0.0));
    // Pressure is the last page: Osc is a step on.
    r.panel.step_page(&mut r.ui, 1);
    assert_eq!(page_name(&r.panel), "Osc");
}

#[test]
fn fm_draws_its_algorithm_and_a_tap_on_a_box_opens_that_operator() {
    let mut s = sample("synth-fm-algo");
    assert_eq!(s.panel.layout(), LAYOUT);
    assert_eq!(titles(&s.ui), ["Algorithm", "Levels"]);
    // Brass Section: algorithm 22, feedback 7 on operator 6. Carriers 1, 3,
    // 4 and 5 sit on the bottom row; 6 sits over 4.
    let (algo, g) = section(&s.ui, "algo");
    let card = algo.cards[0].clone();
    let nodes = &card.nodes;
    assert_eq!(nodes.len(), 6);
    assert_eq!(card.caption, "22 · feedback 7 on 6");
    assert_eq!(algo.note, card.caption, "the caption sits by the name");
    assert_eq!(nodes[5].feedback, Some(5));
    assert!(nodes.iter().all(|n| n.ring.is_none()));
    let bottom = nodes.iter().map(|n| n.y).fold(1.0, f32::min);
    let on_bottom: Vec<&str> = nodes
        .iter()
        .filter(|n| (n.y - bottom).abs() < 1e-4)
        .map(|n| n.label.as_str())
        .collect();
    assert_eq!(on_bottom, ["1", "3", "4", "5"]);
    assert!((nodes[5].x - nodes[3].x).abs() < 1e-4);
    let levels = &cards(&s.ui)[1];
    assert_eq!(levels.bars.len(), 6);
    assert_eq!(levels.bar_labels, ["1", "2", "3", "4", "5", "6"]);
    // A tap on box 3 opens Op 3, which rings box 3 beside its envelope.
    let d = g.drawing.unwrap();
    let plot = plot_rect(wells(d, 1)[0]);
    let at = centre(node_rect(plot, &nodes[2]));
    let ev =
        s.ui.touch(1, Touch::Down, at.0, at.1)
            .expect("the section takes the touch");
    assert!(matches!(ev.kind, EventKind::Tab(_)), "{:?}", ev.kind);
    assert!(s.panel.event(&mut s.ui, &ev).is_none());
    s.ui.touch(1, Touch::Up, at.0, at.1);
    assert_eq!(page_name(&s.panel), "Op 3");
    assert_eq!(titles(&s.ui), ["Op 3 env", "Algorithm"]);
    let (algo, g) = section(&s.ui, "algo");
    assert_eq!(
        algo.cards[0]
            .nodes
            .iter()
            .filter(|n| n.ring.is_some())
            .map(|n| n.label.as_str())
            .collect::<Vec<_>>(),
        ["3"]
    );
    let env = &cards(&s.ui)[0];
    assert!(
        env.caption.starts_with("carrier · output "),
        "{}",
        env.caption
    );
    // A tap on the drawing away from the boxes goes back to the Algo page.
    let d = g.drawing.unwrap();
    tap_panel(&mut s.ui, &mut s.panel, (d.x + 6, d.y + 6));
    assert_eq!(page_name(&s.panel), "Algo");
    // Every FM page draws something.
    for k in 0..s.panel.pages().len() {
        s.panel.show_page(&mut s.ui, k);
        assert_eq!(s.panel.layout(), LAYOUT);
        assert!(!cards(&s.ui).is_empty(), "{}", page_name(&s.panel));
    }
    s.panel.show_page(&mut s.ui, 1);
    assert_eq!(titles(&s.ui), ["LFO", "Pitch env"]);
    assert!(s.panel.show_page_named(&mut s.ui, "Kbd 3-4"));
    assert_eq!(titles(&s.ui), ["Scaling"]);
    assert_eq!(cards(&s.ui)[0].traces.len(), 2);
}

#[test]
fn fm_pressure_page_shows_live_bars() {
    let s = sample("synth-fm-pressure");
    assert_eq!(s.panel.layout(), LAYOUT);
    let c = cards(&s.ui);
    assert_eq!(c.len(), 2);
    assert!(
        c[1].bars.iter().filter(|&&b| b > 0.0).count() == 3,
        "{:?}",
        c[1].bars
    );
}

#[test]
fn a_node_without_a_plan_gets_a_section_for_each_group() {
    let node = Example::new();
    let st = SynthState::from_node(&node, "", Color::rgb(0x6EA8FE), Vec::new(), None);
    let mut ui = app_ui(Fonts::embedded(), "");
    let panel = SynthPanel::add(&mut ui, PAGE, st).unwrap();
    ui.show(PAGE).unwrap();
    assert_eq!(panel.layout(), LAYOUT);
    let names: Vec<&str> = panel.pages().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Delay", "Output"]);
    let v = view(&ui);
    assert!(!v.sections.is_empty());
    assert!(
        cards(&ui).is_empty(),
        "no drawings for a node we don't know"
    );
    // Every parameter of the page is on it, once.
    let page = &panel.pages()[0];
    let mut shown: Vec<usize> = v
        .sections
        .iter()
        .flat_map(|s| s.controls.iter().map(|c| c.index))
        .collect();
    shown.sort();
    let mut want = page.params.clone();
    want.sort();
    assert_eq!(shown, want);
    let mut c = SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
}

/// The heading of section `s` fits its name, "Open ›" and its note.
fn heading_fits(fonts: &mut Fonts, s: &SectionBox, g: &Geom) -> Result<(), String> {
    if g.head.w <= 0 {
        return Ok(());
    }
    let title = fonts.measure(FontId::Bold, 13.0, &s.title.to_uppercase());
    let mut room = g.head.w - title - 16;
    if s.link.is_some() {
        room -= fonts.measure(FontId::Bold, 13.0, "Open ›") + 12;
    }
    if room < 0 {
        return Err(format!("{}: the name doesn't fit", s.title));
    }
    let note = fonts.measure(FontId::Sans, 13.0, &s.note);
    if !s.note.is_empty() && note > room {
        return Err(format!(
            "{}: \"{}\" is {note} px in {room}",
            s.title, s.note
        ));
    }
    Ok(())
}

#[test]
fn every_sample_lays_out_inside_the_screen_and_paints() {
    let screen = Rect::new(0, 0, sample::WIDTH, sample::HEIGHT);
    let mut fonts = Fonts::embedded();
    let mut misfits = Vec::new();
    for mut s in sample::samples(Fonts::embedded).unwrap() {
        assert_eq!(s.panel.layout(), LAYOUT);
        let tiles_r = s.ui.rect_of(TILES).unwrap();
        assert_eq!(tiles_r.intersect(screen), tiles_r);
        assert_eq!(tiles_r.w, 884, "{}", s.name);
        let v = view(&s.ui);
        let geo = v.geometry(tiles_r);
        for (k, (sec, g)) in v.sections.iter().zip(&geo).enumerate() {
            let name = format!("{}: {}", s.name, sec.title);
            assert!(g.rect.w > 0 && g.rect.h > 0, "{name} has no room");
            assert_eq!(g.rect.intersect(tiles_r), g.rect, "{name} spills out");
            for o in &geo[k + 1..] {
                assert!(g.rect.intersect(o.rect).is_empty(), "{name} overlaps");
            }
            for (c, cg) in sec.controls.iter().zip(&g.controls) {
                assert!(cg.cell.w > 0 && cg.cell.h > 0, "{name}: {} hidden", c.label);
                assert_eq!(
                    cg.cell.intersect(g.rect),
                    cg.cell,
                    "{name}: {} spills out",
                    c.label
                );
                // Buttons and arrows are big enough for a finger.
                for p in &cg.parts {
                    assert!(p.h >= 40 && p.w >= 40, "{name}: {} {p:?}", c.label);
                }
            }
            for (a, ca) in g.controls.iter().enumerate() {
                for cb in &g.controls[a + 1..] {
                    assert!(
                        ca.cell.intersect(cb.cell).is_empty(),
                        "{name}: controls overlap"
                    );
                }
            }
            if let Err(e) = heading_fits(&mut fonts, sec, g) {
                misfits.push(format!("{}: {e}", s.name));
            }
        }
        let mut c = SoftCanvas::new(sample::WIDTH, sample::HEIGHT);
        assert_eq!(s.ui.paint(&mut c), vec![screen]);
        assert!(!s.ui.needs_paint(), "{}: nothing animates", s.name);
        // A turn redraws only the panel's widgets, not the app's header.
        if let Some(a) = s.panel.wheel(&mut s.ui, 1) {
            let _ = a;
            let damage = s.ui.paint(&mut c);
            let header = s.ui.rect_of("header").unwrap();
            assert!(
                damage.iter().all(|d| d.intersect(header).is_empty()),
                "{}",
                s.name
            );
        }
    }
    assert!(
        misfits.is_empty(),
        "headings that don't fit:\n{}",
        misfits.join("\n")
    );
}

#[test]
fn stepped_and_choice_values_are_whole_numbers_after_a_drag() {
    let mut r = rig("Init");
    r.page("Voice");
    r.select(pp::POLYPHONY);
    // A drag reports positions along the range; the panel rounds a stepped
    // value to a whole one.
    for pos in [0.13, 0.37, 0.71] {
        let ev = UiEvent {
            widget: TILES.to_string(),
            kind: EventKind::Value(pos),
        };
        if let Some(SynthAction::Set { id, value }) = r.panel.event(&mut r.ui, &ev) {
            let p = pp::info(id).unwrap();
            assert!(matches!(p.curve, Curve::Stepped));
            assert_eq!(value, value.round());
        }
    }
}

#[test]
fn every_sample_takes_its_colours_from_the_theme() {
    use mpfree_synth_ui::widgets::{QLINK_COLORS, TEXT, TEXT_DIM, TEXT_FAINT};
    use mpfree_ui::Theme;
    // The default theme's text colours, drawn solid anywhere in the panel
    // under another theme, would be a colour that missed the theme; so
    // would its Q-Link colours anywhere but on the Q-Link badges and cards
    // (the knobs keep their colours under every theme). Edges of shapes
    // blend, so only exact matches count.
    let d = Theme::dark();
    let text: Vec<Color> = [TEXT, TEXT_DIM, TEXT_FAINT].map(|p| p.on(&d)).to_vec();
    let qlinks: Vec<Color> = QLINK_COLORS.map(|p| p.on(&d)).to_vec();
    let found = |c: &SoftCanvas, r: Rect, fixed: &[Color]| {
        let mut n = 0;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let p = c.pixel(x, y);
                if fixed.iter().any(|f| (f.r, f.g, f.b) == (p.r, p.g, p.b)) {
                    n += 1;
                }
            }
        }
        n
    };
    let all: Vec<Color> = qlinks.iter().chain(&text).copied().collect();
    for name in ["console", "synthwave", "spaceship"] {
        for mut s in sample::samples(Fonts::embedded).unwrap() {
            s.ui.set_theme(Theme::builtin(name).unwrap());
            let mut c = SoftCanvas::new(sample::WIDTH, sample::HEIGHT);
            s.ui.paint(&mut c);
            for widget in [TITLE, INST, TILES, QLINK_COLUMN] {
                let r = s.ui.rect_of(widget).unwrap();
                let fixed: &[Color] = if widget == TITLE || widget == INST {
                    &all
                } else {
                    &text
                };
                let n = found(&c, r, fixed);
                assert_eq!(n, 0, "{name} {}: {widget} has default colours", s.name);
            }
            // The drawings shade their curves in their section's colour.
            let r = s.ui.rect_of(TILES).unwrap();
            for g in view(&s.ui).geometry(r) {
                if let Some(d) = g.drawing {
                    let n = found(&c, d, &qlinks);
                    assert_eq!(n, 0, "{name} {}: a drawing has Q-Link colours", s.name);
                }
            }
        }
    }
}

#[test]
fn the_instrument_engines_draw_inside_their_sections() {
    // Each page's drawings, and the sections that open another page.
    type Pages<'a> = &'a [(&'a str, &'a [&'a str])];
    type Links<'a> = &'a [(&'a str, &'a str, &'a str)];
    let engines: [(&str, Pages, Links); 9] = [
        (
            "synth-keys-strike",
            &[
                ("Strike", &["Hammer", "Decay"]),
                ("Tone", &["Pickup"]),
                ("Clav", &[]),
                ("Amp", &["Tremolo"]),
                ("Voice", &[]),
            ],
            &[],
        ),
        (
            "synth-keys-clav",
            &[
                ("Strike", &["Hammer", "Decay"]),
                ("Tone", &["Pickup"]),
                ("Clav", &["Pickups", "Mute"]),
                ("Amp", &["Tremolo"]),
                ("Voice", &[]),
            ],
            &[],
        ),
        (
            "synth-drawbar",
            &[
                ("Drawbars", &["Wave", "Percussion"]),
                ("Percussion", &["Percussion"]),
                ("Rotary", &["Rotor"]),
                ("Voice", &[]),
            ],
            &[
                ("Drawbars", "perc", "Percussion"),
                ("Drawbars", "rotary", "Rotary"),
            ],
        ),
        (
            "synth-drawbar-combo",
            &[
                ("Drawbars", &["Wave", "Percussion"]),
                ("Percussion", &["Percussion"]),
                ("Rotary", &["Rotor"]),
                ("Voice", &[]),
            ],
            &[
                ("Drawbars", "perc", "Percussion"),
                ("Drawbars", "rotary", "Rotary"),
            ],
        ),
        (
            "synth-squelch",
            &[
                ("Synth", &["Filter", "Wave"]),
                ("Play", &["Sweep"]),
                ("Voice", &[]),
            ],
            &[("Synth", "playing", "Play")],
        ),
        (
            "synth-analog-filter",
            &[
                ("Osc", &["Osc"]),
                ("Filter", &["Filter", "Env"]),
                ("Envelopes", &["Env", "Env"]),
                ("Mod", &["LFO"]),
                ("Voice", &["Voices"]),
            ],
            &[("Filter", "fenv", "Envelopes")],
        ),
        (
            "synth-bass",
            &[
                ("Source", &["Wave"]),
                ("Filter", &["Filter"]),
                ("Shape", &["Drop"]),
                ("Play", &[]),
                ("Output", &["Drive"]),
            ],
            &[],
        ),
        (
            "synth-wind",
            &[
                ("Instrument", &["Mouthpiece", "Tone"]),
                ("Breath", &["Level", "Breath"]),
                ("Play", &[]),
                ("Pressure", &["Pressure", "Voices"]),
            ],
            &[],
        ),
        (
            "synth-choir",
            &[
                ("Choir", &["Vowel"]),
                ("Shape", &["Swell", "Singers"]),
                ("Ensemble", &["Ensemble"]),
                ("Play", &[]),
            ],
            &[],
        ),
    ];
    for (name, pages, links) in engines {
        let mut s = sample(name);
        let names: Vec<&str> = s.panel.pages().iter().map(|p| p.name.as_str()).collect();
        let want: Vec<&str> = pages.iter().map(|p| p.0).collect();
        assert_eq!(names, want, "{name}");
        for &(page, drawn) in pages {
            assert!(s.panel.show_page_named(&mut s.ui, page));
            assert_eq!(s.panel.layout(), LAYOUT, "{name}");
            assert_eq!(titles(&s.ui), drawn, "{name}: {page}");
            // Only the sections listed link elsewhere.
            for sec in &view(&s.ui).sections {
                let listed = links.iter().any(|l| l.0 == page && l.1 == sec.key);
                assert_eq!(sec.link.is_some(), listed, "{name}: {page} {}", sec.key);
            }
        }
        for &(page, key, to) in links {
            assert!(s.panel.show_page_named(&mut s.ui, page));
            let (_, g) = section(&s.ui, key);
            tap_panel(&mut s.ui, &mut s.panel, centre(g.head));
            assert_eq!(page_name(&s.panel), to, "{name}: {key}");
        }
    }
    // Analog's two envelopes each draw their own curve of the pair, in
    // their section's colour.
    let mut s = sample("synth-analog-filter");
    assert!(s.panel.show_page_named(&mut s.ui, "Envelopes"));
    let c = cards(&s.ui);
    assert_eq!(c[0].traces.len(), 1);
    assert_eq!(c[1].traces.len(), 1);
    assert_ne!(c[0].traces[0].pts, c[1].traces[0].pts);
}

#[test]
fn the_instrument_engines_headings_fit() {
    use mpfree_synth::{Analog, Bass, Choir, Drawbar, Keys, Preset, Squelch, Strings, Wind};
    /// Every factory preset, then every parameter on its own at its
    /// minimum and maximum (each choice at each of its values).
    fn each<N: Node>(
        mut make: impl FnMut() -> N,
        presets: &[Preset],
        load: fn(&mut N, &Preset),
        check: &mut dyn FnMut(&dyn Node, &str),
    ) {
        for p in presets {
            let mut n = make();
            load(&mut n, p);
            check(&n, p.name);
        }
        let params = make().params().to_vec();
        for p in &params {
            let values: Vec<f32> = match p.curve {
                Curve::Choice(names) => (0..names.len()).map(|i| i as f32).collect(),
                _ => vec![p.min, p.max],
            };
            for v in values {
                let mut n = make();
                n.set_param(p.id, v);
                check(&n, &format!("{} at {v}", p.name));
            }
        }
    }
    let mut ui = app_ui(Fonts::embedded(), "");
    let mut fonts = Fonts::embedded();
    let mut misfits: Vec<(String, String)> = Vec::new();
    let node = Keys::new(48_000);
    let st = SynthState::from_node(&node, "Keys", Color::rgb(0x3DDC97), Vec::new(), None);
    let mut panel = SynthPanel::add(&mut ui, PAGE, st).unwrap();
    ui.show(PAGE).unwrap();
    let mut check = |n: &dyn Node, preset: &str| {
        let st = SynthState::from_node(n, "Keys", Color::rgb(0x3DDC97), Vec::new(), None);
        panel.load(&mut ui, st).unwrap();
        for k in 0..panel.pages().len() {
            panel.show_page(&mut ui, k);
            let r = ui.rect_of(TILES).unwrap();
            let v = view(&ui);
            for (s, g) in v.sections.iter().zip(v.geometry(r)) {
                if let Err(e) = heading_fits(&mut fonts, s, &g) {
                    let what = format!("{}, {}: {e}", n.info().name, page_name(&panel));
                    if !misfits.iter().any(|m: &(String, String)| m.0 == what) {
                        misfits.push((what, preset.to_string()));
                    }
                }
            }
        }
    };
    each(
        || Keys::new(48_000),
        Keys::presets(),
        Keys::load_preset,
        &mut check,
    );
    each(
        || Drawbar::new(48_000),
        Drawbar::presets(),
        Drawbar::load_preset,
        &mut check,
    );
    each(
        || Squelch::new(48_000),
        Squelch::presets(),
        Squelch::load_preset,
        &mut check,
    );
    each(
        || Analog::new(48_000),
        Analog::presets(),
        Analog::load_preset,
        &mut check,
    );
    each(
        || Strings::new(48_000),
        Strings::presets(),
        Strings::load_preset,
        &mut check,
    );
    each(
        || Bass::new(48_000),
        Bass::presets(),
        Bass::load_preset,
        &mut check,
    );
    each(
        || Wind::new(48_000),
        Wind::presets(),
        Wind::load_preset,
        &mut check,
    );
    each(
        || Choir::new(48_000),
        Choir::presets(),
        Choir::load_preset,
        &mut check,
    );
    // The longest captions each drawing can have.
    use mpfree_synth::{analog_params as an, drawbar_params as db, keys_params as kp};
    let mut keys = Keys::new(48_000);
    for (id, v) in [
        (kp::TREMOLO, 50.0),
        (kp::TREM_RATE, 11.9),
        (kp::HARDNESS, 0.0),
    ] {
        keys.set_param(id, v);
    }
    check(&keys, "longest");
    let mut organ = Drawbar::new(48_000);
    for (id, v) in [
        (db::PERC, 1.0),
        (db::PERC_HARM, 0.0),
        (db::PERC_DECAY, 1.0),
        (db::PERC_VOL, 1.0),
        (db::ROTARY, 1.0),
        (db::SPEED, 2.0),
    ] {
        organ.set_param(id, v);
    }
    check(&organ, "longest");
    let mut poly = Analog::new(48_000);
    for (id, v) in [
        (an::OSC1_WAVE, 2.0),
        (an::OSC2_WAVE, 1.0),
        (an::OSC1_LEVEL, 100.0),
        (an::OSC2_LEVEL, 100.0),
        (an::SYNC, 0.0),
        (an::LFO_WAVE, 1.0),
        (an::LFO_RATE, 0.15),
        (an::AGE, 100.0),
    ] {
        poly.set_param(id, v);
    }
    check(&poly, "longest");
    let mut strings = Strings::new(48_000);
    for (id, v) in mpfree_synth::strings_params::REGISTERS {
        let _ = v;
        strings.set_param(id, 100.0);
    }
    strings.set_param(mpfree_synth::strings_params::LINES, 1.0);
    strings.set_param(mpfree_synth::strings_params::SWELL, 1.0);
    check(&strings, "longest");
    let mut bass = Bass::new(48_000);
    {
        use mpfree_synth::bass_params as bp;
        for (id, v) in [
            (bp::BODY, bp::REESE),
            (bp::WAVE, bp::TRIANGLE),
            (bp::SUB, 100.0),
            (bp::DROP, 24.0),
            (bp::DROP_TIME, 1_000.0),
            (bp::PUNCH, 100.0),
            (bp::DRIVE, 100.0),
            (bp::CUTOFF, 12_000.0),
        ] {
            bass.set_param(id, v);
        }
    }
    check(&bass, "longest");
    let mut wind = Wind::new(48_000);
    {
        use mpfree_synth::wind_params as wn;
        for (id, v) in [
            (wn::MODEL, wn::FLUTE),
            (wn::EDGE, 100.0),
            (wn::BREATH_FROM, wn::FROM_FOOT),
            (wn::TONGUE, 100.0),
            (wn::BREATH, 100.0),
            (wn::TONE, 100.0),
        ] {
            wind.set_param(id, v);
        }
    }
    check(&wind, "longest");
    let mut choir = Choir::new(48_000);
    {
        use mpfree_synth::choir_params as chp;
        for (id, v) in [
            (chp::SINGERS, 8.0),
            (chp::DETUNE, 50.0),
            (chp::VOWEL, 37.0),
            (chp::LINES, 1.0),
            (chp::SLOW_RATE, 2.99),
            (chp::FAST_RATE, 11.99),
            (chp::ATTACK, 7_999.0),
            (chp::RELEASE, 11_999.0),
        ] {
            choir.set_param(id, v);
        }
    }
    check(&choir, "longest");
    let list: Vec<String> = misfits
        .iter()
        .map(|(what, preset)| format!("{what} ({preset})"))
        .collect();
    assert!(
        list.is_empty(),
        "headings that don't fit:\n{}",
        list.join("\n")
    );
}

#[test]
fn the_routes_page_reads_each_depth_in_its_targets_unit() {
    let mut r = rig("Grit Bass");
    r.page("Routes");
    let titles: Vec<String> = view(&r.ui)
        .sections
        .iter()
        .map(|s| s.title.clone())
        .collect();
    assert_eq!(titles, ["Route 1", "Route 2", "Route 3", "Route 4"]);
    // Route 1 sends the wheel to vibrato: 50 % is half a semitone.
    let (s1, _) = section(&r.ui, "route-1");
    let depth = s1.controls.iter().find(|c| c.label == "Depth 1").unwrap();
    assert_eq!((depth.value.as_str(), depth.unit.as_str()), ("+0.5", "st"));
    assert!(!depth.dim);
    // Route 2 is off: its target and depth do nothing, and say why.
    let (s2, _) = section(&r.ui, "route-2");
    assert!(s2.controls.iter().filter(|c| c.dim).count() == 2);
    assert_eq!(s2.note, "To 2 · no source");
    // Every label on the page is its own.
    let mut labels: Vec<String> = view(&r.ui)
        .sections
        .iter()
        .flat_map(|s| s.controls.iter().map(|c| c.label.clone()))
        .collect();
    let n = labels.len();
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), n);
}

#[test]
fn the_lfo_2_page_draws_lfo_2() {
    let mut r = rig("Grit Bass");
    r.page("LFO 2");
    assert_eq!(titles(&r.ui), ["LFO 2"]);
    let (lfo, _) = section(&r.ui, "lfo-2");
    assert_eq!(lfo.title, "LFO 2");
}
