//! The effect pages and the Perform FX page: what they draw, and what
//! touches, Q-Links and the wheel send.

use mpfree_fx::ids::{delay, eq, filter, perform as pf};
use mpfree_fx::mpfree_node::{Ctx, Curve, Kind, Node, NodeInfo, ParamInfo, Timed, Unit};
use mpfree_fx_ui::perform::{self, PerformPads, XyView};
use mpfree_fx_ui::sample::{app_ui, samples, BPM, RATE};
use mpfree_fx_ui::view::{self, FxAction, PLAIN_LAYOUT};
use mpfree_fx_ui::visual::has_visuals;
use mpfree_fx_ui::{make, preset_values, FxPanel, FxState, Locks, PerformAction, PerformPanel};
use mpfree_synth_ui::widgets::TitleBar;
use mpfree_synth_ui::widgets::{TileGrid, Visuals};
use mpfree_ui::{Color, EventKind, Fonts, SoftCanvas, Touch, Ui, UiEvent};

fn state(id: &str, values: &[(u16, f32)]) -> FxState {
    let mut n = make(id).unwrap();
    n.prepare(RATE, 512);
    for &(k, v) in values {
        n.set_param(k, v);
    }
    FxState::from_node(
        n.as_ref(),
        "Drums · insert 1",
        Color::rgb(0x3DDC97),
        BPM,
        RATE,
    )
}

fn panel(id: &str, values: &[(u16, f32)]) -> (Ui, FxPanel) {
    let mut ui = app_ui(Fonts::embedded(), "");
    let p = FxPanel::add(&mut ui, "fx", state(id, values)).unwrap();
    ui.show("fx").unwrap();
    (ui, p)
}

fn ev(widget: &str, kind: EventKind) -> UiEvent {
    UiEvent {
        widget: widget.to_string(),
        kind,
    }
}

#[test]
fn every_sample_screen_draws() {
    let list = samples(Fonts::embedded).unwrap();
    assert_eq!(list.len(), 29);
    for mut s in list {
        let (w, h) = s.ui.size();
        let mut c = SoftCanvas::new(w, h);
        s.ui.paint(&mut c);
        // Something other than the background in the middle of the page.
        let bg = c.pixel(2, 400);
        let drawn = (100..1200).step_by(7).any(|x| c.pixel(x, 400) != bg);
        assert!(drawn, "{} is blank", s.name);
    }
}

#[test]
fn every_effect_gets_a_page_and_its_drawings_stay_in_their_cards() {
    for fx in mpfree_fx::EFFECTS {
        let (id, name) = (&fx.id, &fx.name);
        let (ui, p) = panel(id, &[]);
        assert!(!p.pages().is_empty(), "{name} has no page");
        let placed: usize = p.pages().iter().map(|pg| pg.params.len()).sum();
        if *id != "mpfree.fx.perform" {
            assert_eq!(
                placed,
                p.state.params.len(),
                "{name}: every parameter on a page"
            );
        }
        for pg in p.pages() {
            assert!(
                pg.params.len() <= 16,
                "{name}: {} has too many tiles",
                pg.name
            );
        }
        let cards = &ui.get::<Visuals>(view::VISUALS).unwrap().cards;
        assert_eq!(!cards.is_empty(), has_visuals(id), "{name}");
        for c in cards {
            for t in &c.traces {
                for &(x, y) in &t.pts {
                    assert!(
                        (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                        "{name}: {} has a point at ({x}, {y})",
                        c.title
                    );
                }
            }
            for b in &c.bars {
                assert!((0.0..=1.0).contains(b), "{name}: bar {b}");
            }
        }
    }
}

#[test]
fn qlinks_turn_what_each_page_picks() {
    let (mut ui, mut p) = panel("mpfree.fx.eq", &[]);
    assert_eq!(p.qlink_ids(), (0..4).map(eq::gain).collect::<Vec<_>>());
    match p.qlink(&mut ui, 1, 5) {
        Some(FxAction::Set { id, value }) => {
            assert_eq!(id, eq::gain(1));
            assert!(
                (value - 1.8).abs() < 1e-4,
                "5 of 100 detents of 36 dB: {value}"
            );
        }
        a => panic!("{a:?}"),
    }
    // The delay's Q-Link 1 turns Sync while synced, Time while free.
    let (mut ui, mut p) = panel("mpfree.fx.delay", &[(delay::SYNC, 7.0)]);
    assert_eq!(p.qlink_ids()[0], delay::SYNC);
    assert_eq!(
        p.qlink(&mut ui, 0, -7),
        Some(FxAction::Set {
            id: delay::SYNC,
            value: 0.0
        })
    );
    assert_eq!(p.qlink_ids()[0], delay::TIME);
}

#[test]
fn a_turn_moves_the_drawing_at_once() {
    let (mut ui, mut p) = panel("mpfree.fx.filter", &[(filter::CUTOFF, 1000.0)]);
    let dot = |ui: &Ui| {
        ui.get::<Visuals>(view::VISUALS).unwrap().cards[0]
            .dot
            .unwrap()
    };
    let before = dot(&ui);
    let k = p.pages()[0]
        .params
        .iter()
        .position(|&i| p.state.params[i].id == filter::CUTOFF)
        .unwrap();
    p.event(&mut ui, &ev(view::TILES, EventKind::Select(k)));
    let a = p.wheel(&mut ui, 10).unwrap();
    let FxAction::Set { id, value } = a else {
        panic!()
    };
    assert_eq!(id, filter::CUTOFF);
    assert!(value > 1500.0, "{value}");
    assert!(dot(&ui) > before + 0.05, "the dot follows the cutoff");
}

#[test]
fn a_preset_shows_at_once_and_keeps_the_mix() {
    let (mut ui, mut p) = panel("mpfree.fx.cloud", &[(mpfree_fx::ids::cloud::MIX, 100.0)]);
    assert_eq!(p.layout(), view::LAYOUT, "the reverb has presets");
    let a = p.event(&mut ui, &ev(view::PRESETS, EventKind::Select(2)));
    assert_eq!(a, Some(FxAction::Preset(2)));
    let want = preset_values(&p.state, 2);
    for (id, v) in want {
        assert_eq!(p.state.value(id), v, "param {id}");
    }
    assert_eq!(p.state.value(mpfree_fx::ids::cloud::MIX), 100.0, "Mix kept");
    assert_eq!(p.state.preset, Some(2));
    assert!(!p.state.edited);
}

#[test]
fn meters_update_without_sending_anything() {
    let (mut ui, mut p) = panel("mpfree.fx.compressor", &[]);
    use mpfree_fx::ids::compressor as comp;
    p.set_meters(&mut ui, &[(comp::REDUCTION, -6.0), (comp::THRESHOLD, -3.0)]);
    assert_eq!(p.state.value(comp::REDUCTION), -6.0);
    assert_eq!(
        p.state.value(comp::THRESHOLD),
        -18.0,
        "only meters come in this way"
    );
    let cards = &ui.get::<Visuals>(view::VISUALS).unwrap().cards;
    assert!(
        cards[0].caption.contains("now -6.0 dB"),
        "{}",
        cards[0].caption
    );
}

#[test]
fn a_tap_on_an_eq_band_selects_its_gain() {
    let (mut ui, mut p) = panel("mpfree.fx.eq", &[]);
    p.show_page(&mut ui, 1);
    p.event(&mut ui, &ev(view::VISUALS, EventKind::Select(2)));
    assert_eq!(p.pages()[p.page()].name, "Bands");
    assert_eq!(p.focus(), Some(eq::gain(2)));
    let grid = ui.get::<TileGrid>(view::TILES).unwrap();
    let sel = grid.selected.unwrap();
    assert_eq!(grid.tiles[sel].label, "Band 3 · Gain");
}

/// A node from elsewhere (a hosted plugin): pages from its groups, no
/// drawing.
struct Plugin {
    v: [f32; 3],
}

static PLUGIN: NodeInfo = NodeInfo {
    id: "com.example.wobble",
    name: "Wobble",
    kind: Kind::Effect,
};

static PLUGIN_PARAMS: [ParamInfo; 3] = [
    ParamInfo {
        id: 0,
        name: "Rate",
        group: "Wobble",
        min: 0.1,
        max: 20.0,
        default: 2.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Depth",
        group: "Wobble",
        min: 0.0,
        max: 100.0,
        default: 50.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
];

impl Node for Plugin {
    fn info(&self) -> &'static NodeInfo {
        &PLUGIN
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PLUGIN_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        if let Some(v) = self.v.get_mut(id as usize) {
            *v = value;
        }
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _: &Ctx, _: &[Timed], _: &mut [f32], _: &mut [f32]) {}
}

#[test]
fn a_plugin_gets_pages_from_its_groups_and_no_drawing() {
    let node = Plugin {
        v: [2.0, 50.0, 100.0],
    };
    let st = FxState::from_node(&node, "Return B", Color::rgb(0x6EA8FE), BPM, RATE);
    let mut ui = app_ui(Fonts::embedded(), "");
    let mut p = FxPanel::add(&mut ui, "fx", st).unwrap();
    assert_eq!(p.layout(), PLAIN_LAYOUT);
    assert_eq!(p.pages()[0].params, vec![0, 1, 2]);
    assert_eq!(p.qlink_ids(), vec![0, 1, 2]);
    assert!(matches!(
        p.qlink(&mut ui, 2, -10),
        Some(FxAction::Set { id: 2, .. })
    ));
}

// ---------------------------------------------------------------- Perform

fn perform_page() -> (Ui, PerformPanel) {
    let mut ui = app_ui(Fonts::embedded(), "");
    let st = state("mpfree.fx.perform", &[]);
    let p = PerformPanel::add(&mut ui, "perform", st).unwrap();
    ui.show("perform").unwrap();
    (ui, p)
}

#[test]
fn a_screen_pad_plays_while_held_and_selects_itself() {
    let (mut ui, mut p) = perform_page();
    let r = ui.rect_of(perform::PADS).unwrap();
    let c = PerformPads::cell(r, 5);
    let down = ui.touch(1, Touch::Down, c.x + 5, c.y + 5).unwrap();
    assert_eq!(p.event(&mut ui, &down), vec![PerformAction::Press(5)]);
    assert_eq!(p.selected(), 5);
    // Pad 1 is bottom left, as on the MPC.
    let c0 = PerformPads::cell(r, 0);
    let c12 = PerformPads::cell(r, 12);
    assert!(c0.y > c12.y && c0.x == c12.x);
    let up = ui.touch(1, Touch::Up, c.x + 5, c.y + 5).unwrap();
    assert_eq!(p.event(&mut ui, &up), vec![PerformAction::Release(5)]);
    // The lights follow the node's states.
    p.set_meters(&mut ui, &[(pf::slot(5) + pf::STATE, 1.0)]);
    assert_eq!(
        ui.get::<PerformPads>(perform::PADS).unwrap().pads[5].state,
        1
    );
}

#[test]
fn the_xy_pad_plays_the_filter_and_echo_and_glides_back() {
    let (mut ui, mut p) = perform_page();
    let a = XyView::area(ui.rect_of(perform::XY).unwrap());
    // A finger at the left quarter, three quarters up.
    let (x, y) = (a.x + a.w / 4, a.bottom() - a.h * 3 / 4);
    let e = ui.touch(1, Touch::Down, x, y).unwrap();
    let sent = p.event(&mut ui, &e);
    let get = |sent: &[PerformAction], want: u16| {
        sent.iter().find_map(|s| match s {
            PerformAction::Set { id, value } if *id == want => Some(*value),
            _ => None,
        })
    };
    let dj = get(&sent, pf::DJ_FILTER).unwrap();
    let echo = get(&sent, pf::ECHO).unwrap();
    assert!((dj + 50.0).abs() < 1.0, "a low-pass halfway: {dj}");
    assert!((echo - 75.0).abs() < 1.0, "{echo}");
    let e = ui.touch(1, Touch::Up, x, y).unwrap();
    assert!(p.event(&mut ui, &e).is_empty());
    // Lifted: it glides home within a second, and says so as it goes.
    let mut moved = 0;
    for _ in 0..60 {
        if !p.tick(&mut ui, 16.0).is_empty() {
            moved += 1;
        }
    }
    assert!(moved > 5);
    assert_eq!(p.state.value(pf::DJ_FILTER), 0.0);
    assert_eq!(p.state.value(pf::ECHO), 0.0);
    // With Hold on it stays where the finger let go.
    let h = XyView::hold_rect(ui.rect_of(perform::XY).unwrap());
    let e = ui.touch(2, Touch::Down, h.x + 4, h.y + 4).unwrap();
    p.event(&mut ui, &e);
    ui.touch(2, Touch::Up, h.x + 4, h.y + 4);
    assert!(p.xy().hold);
    let e = ui.touch(3, Touch::Down, x, y).unwrap();
    p.event(&mut ui, &e);
    let e = ui.touch(3, Touch::Up, x, y).unwrap();
    p.event(&mut ui, &e);
    for _ in 0..60 {
        assert!(p.tick(&mut ui, 16.0).is_empty());
    }
    assert!(p.state.value(pf::ECHO) > 70.0);
}

#[test]
fn the_dj_knob_stops_in_the_middle_on_its_way_through() {
    let (mut ui, mut p) = perform_page();
    assert_eq!(p.qlink_ids()[0], pf::DJ_FILTER);
    p.qlink(&mut ui, 0, -3);
    let v = p.state.value(pf::DJ_FILTER);
    assert!(v < 0.0);
    assert_eq!(
        p.qlink(&mut ui, 0, 10),
        Some(PerformAction::Set {
            id: pf::DJ_FILTER,
            value: 0.0
        }),
        "stops at off"
    );
    let Some(PerformAction::Set { value, .. }) = p.qlink(&mut ui, 0, 10) else {
        panic!()
    };
    assert!(value > 0.0, "then goes on into the high-pass");
    // Q-Link 4 follows the selected pad's Amount.
    p.select(&mut ui, 7);
    assert_eq!(p.qlink_ids()[3], pf::slot(7) + pf::AMOUNT);
}

#[test]
fn a_held_step_locks_turns_and_leaves_the_effect_alone() {
    let (mut ui, mut p) = panel("mpfree.fx.filter", &[(filter::CUTOFF, 1000.0)]);
    let k = p.pages()[0]
        .params
        .iter()
        .position(|&i| p.state.params[i].id == filter::CUTOFF)
        .unwrap();
    let tile = |ui: &Ui| ui.get::<TileGrid>(view::TILES).unwrap().tiles[k].clone();
    let dot = |ui: &Ui| {
        ui.get::<Visuals>(view::VISUALS).unwrap().cards[0]
            .dot
            .unwrap()
    };
    let (base_tile, base_dot) = (tile(&ui), dot(&ui));
    // Step 5 is held; it already locks the resonance.
    let mut held = Locks::new("Step 5");
    held.values.push((filter::RESONANCE, 50.0));
    p.set_locks(&mut ui, Some(held));
    assert!(ui
        .get::<TitleBar>(view::TITLE)
        .unwrap()
        .subtitle
        .starts_with("Locking Step 5 · 1 lock"));
    p.event(&mut ui, &ev(view::TILES, EventKind::Select(k)));
    let a = p.wheel(&mut ui, 10).unwrap();
    let FxAction::Lock { id, value } = a else {
        panic!("{a:?}")
    };
    assert_eq!(id, filter::CUTOFF);
    assert!(value > 1500.0, "{value}");
    assert_eq!(
        p.state.value(filter::CUTOFF),
        1000.0,
        "the effect's own value stays"
    );
    assert_eq!(p.locks().unwrap().get(filter::CUTOFF), Some(value));
    let t = tile(&ui);
    assert_ne!(t.value, base_tile.value, "the tile shows the lock");
    assert_eq!(t.color, ui.theme.warn, "locked tiles are marked");
    assert!(
        dot(&ui) > base_dot + 0.05,
        "the drawing shows the locked cutoff"
    );
    // A second turn starts from the lock.
    let FxAction::Lock { value: v2, .. } = p.wheel(&mut ui, 1).unwrap() else {
        panic!()
    };
    assert!(v2 > value);
    // Taking the selected lock off; then every lock.
    assert_eq!(
        p.unlock(&mut ui),
        Some(FxAction::Unlock { id: filter::CUTOFF })
    );
    assert_eq!(p.unlock(&mut ui), None, "nothing left on that tile");
    assert_eq!(tile(&ui).value, base_tile.value);
    assert_eq!(
        p.unlock_all(&mut ui),
        vec![FxAction::Unlock {
            id: filter::RESONANCE
        }]
    );
    // Let go of the step: turns set the effect again.
    p.set_locks(&mut ui, None);
    assert!(matches!(p.wheel(&mut ui, 1), Some(FxAction::Set { .. })));
}

#[test]
fn presets_wait_while_a_step_is_held() {
    let (mut ui, mut p) = panel("mpfree.fx.cloud", &[]);
    p.set_locks(&mut ui, Some(Locks::new("Step 1")));
    assert_eq!(
        p.event(&mut ui, &ev(view::PRESETS, EventKind::Select(2))),
        None
    );
    assert_eq!(p.state.preset, None);
}

#[test]
fn a_held_step_locks_perform_pads_turns_and_the_xy_pad() {
    let (mut ui, mut p) = perform_page();
    p.set_locks(&mut ui, Some(Locks::new("Step 3")));
    let r = ui.rect_of(perform::PADS).unwrap();
    let c = PerformPads::cell(r, 4);
    let tap = |ui: &mut Ui, p: &mut PerformPanel| {
        let d = ui.touch(1, Touch::Down, c.x + 5, c.y + 5).unwrap();
        let mut out = p.event(ui, &d);
        let u = ui.touch(1, Touch::Up, c.x + 5, c.y + 5).unwrap();
        out.extend(p.event(ui, &u));
        out
    };
    // A tap locks the pad on for the step: it lights, and plays nothing now.
    assert_eq!(
        tap(&mut ui, &mut p),
        vec![PerformAction::Lock {
            id: pf::hold(4),
            value: 1.0
        }]
    );
    let pads = ui.get::<PerformPads>(perform::PADS).unwrap();
    assert_eq!(pads.pads[4].state, 2);
    assert_eq!(pads.locking.as_deref(), Some("Step 3"));
    // Again: the lock comes off.
    assert_eq!(
        tap(&mut ui, &mut p),
        vec![PerformAction::Unlock { id: pf::hold(4) }]
    );
    assert_eq!(
        ui.get::<PerformPads>(perform::PADS).unwrap().pads[4].state,
        0
    );
    // A hardware pad does the same.
    assert_eq!(
        p.pad_lock(&mut ui, 9),
        Some(PerformAction::Lock {
            id: pf::hold(9),
            value: 1.0
        })
    );
    // The DJ knob locks; the XY pad locks both its parameters.
    let a = p.qlink(&mut ui, 0, -20).unwrap();
    assert!(
        matches!(
            a,
            PerformAction::Lock {
                id: pf::DJ_FILTER,
                ..
            }
        ),
        "{a:?}"
    );
    let xa = XyView::area(ui.rect_of(perform::XY).unwrap());
    let (x, y) = (xa.x + xa.w * 3 / 4, xa.bottom() - xa.h / 2);
    let e = ui.touch(2, Touch::Down, x, y).unwrap();
    let sent = p.event(&mut ui, &e);
    assert!(sent.contains(&PerformAction::Lock {
        id: pf::ECHO,
        value: p.locks().unwrap().get(pf::ECHO).unwrap()
    }));
    assert!(
        p.locks().unwrap().get(pf::DJ_FILTER).unwrap() > 0.0,
        "a high-pass"
    );
    let e = ui.touch(2, Touch::Up, x, y).unwrap();
    assert!(p.event(&mut ui, &e).is_empty(), "no glide home for a lock");
    assert_eq!(p.state.value(pf::DJ_FILTER), 0.0, "the live values stay");
    assert_eq!(p.state.value(pf::ECHO), 0.0);
    // Let go of the step: the pads play again.
    p.set_locks(&mut ui, None);
    let d = ui.touch(3, Touch::Down, c.x + 5, c.y + 5).unwrap();
    assert_eq!(p.event(&mut ui, &d), vec![PerformAction::Press(4)]);
}

#[test]
fn a_watched_effect_gets_a_levels_card() {
    use mpfree_fx::ids::compressor as comp;
    let theme = mpfree_ui::Theme::dark();
    let mut st = state("mpfree.fx.compressor", &[]);
    let r = st.index(comp::REDUCTION).unwrap();
    st.values[r] = -6.0;
    let cards = |st: &FxState| {
        let scratch = mpfree_fx_ui::Scratch::new(st);
        mpfree_fx_ui::visual::strip(st, "Compressor", None, &scratch, &theme).cards
    };
    let plain = cards(&st).len();
    assert!(cards(&st).iter().all(|c| c.title != "Levels"));
    st.levels = Some((-12.0, -18.0));
    let with = cards(&st);
    assert_eq!(with.len(), plain + 1);
    let c = with.last().unwrap();
    assert_eq!(c.title, "Levels");
    assert_eq!(c.bar_labels, ["In", "Out", "Reduction"]);
    // -60 to 0 dBFS across the bar; the reduction against its 60 dB.
    let want = [48.0 / 60.0, 42.0 / 60.0, 6.0 / 60.0];
    for (b, w) in c.bars.iter().zip(want) {
        assert!((b - w).abs() < 1e-4, "{:?}", c.bars);
    }
    assert!(
        c.caption.contains("in -12.0 · out -18.0 dB"),
        "{}",
        c.caption
    );
    // An effect with no gain meter of its own: just in and out.
    let mut st = state("mpfree.fx.delay", &[]);
    st.levels = Some((-130.0, 3.0));
    let c = cards(&st).pop().unwrap();
    assert_eq!(c.bars, [0.0, 1.0]);
    assert!(
        c.caption.starts_with("in -inf · out 3.0 dB"),
        "{}",
        c.caption
    );
}

#[test]
fn the_new_effects_draw_what_they_play() {
    use mpfree_fx::ids::{ducker, pitch, transient};
    let theme = mpfree_ui::Theme::dark();
    let card = |st: &FxState, page: &str| {
        let scratch = mpfree_fx_ui::Scratch::new(st);
        mpfree_fx_ui::visual::strip(st, page, None, &scratch, &theme)
            .cards
            .remove(0)
    };
    let low = |c: &mpfree_synth_ui::widgets::Card| {
        let pts = &c.traces.last().unwrap().pts;
        pts.iter().map(|p| p.1).fold(f32::MAX, f32::min)
    };
    // The ducker's pump goes deeper with more depth, on Tempo and on
    // Sidechain.
    for source in [0.0, 1.0] {
        let a = card(
            &state(
                "mpfree.fx.ducker",
                &[(ducker::SOURCE, source), (ducker::DEPTH, 6.0)],
            ),
            "Ducker",
        );
        let b = card(
            &state(
                "mpfree.fx.ducker",
                &[(ducker::SOURCE, source), (ducker::DEPTH, 24.0)],
            ),
            "Ducker",
        );
        assert_eq!(a.title, "Pump");
        assert!(
            low(&b) < low(&a) - 0.1,
            "source {source}: {} {}",
            low(&a),
            low(&b)
        );
    }
    // More attack lifts the front of the hit.
    let top = |c: &mpfree_synth_ui::widgets::Card| {
        c.traces
            .last()
            .unwrap()
            .pts
            .iter()
            .map(|p| p.1)
            .fold(0.0, f32::max)
    };
    let flat = card(
        &state("mpfree.fx.transient", &[(transient::ATTACK, 0.0)]),
        "Transient shaper",
    );
    let punch = card(
        &state("mpfree.fx.transient", &[(transient::ATTACK, 100.0)]),
        "Transient shaper",
    );
    assert!(
        top(&punch) > top(&flat) + 0.03,
        "{} {}",
        top(&flat),
        top(&punch)
    );
    // The shift sits where the pitch says, and feedback adds stairs.
    let dry = card(
        &state(
            "mpfree.fx.pitch",
            &[(pitch::PITCH, 7.0), (pitch::FEEDBACK, 0.0)],
        ),
        "Pitch shifter",
    );
    let fb = card(
        &state(
            "mpfree.fx.pitch",
            &[(pitch::PITCH, 7.0), (pitch::FEEDBACK, 60.0)],
        ),
        "Pitch shifter",
    );
    let y7 = 0.5 + 7.0 / 52.0;
    assert!(dry
        .traces
        .iter()
        .any(|t| t.pts.iter().all(|p| (p.1 - y7).abs() < 1e-5)));
    assert!(fb.traces.len() >= dry.traces.len() + 2);
}

#[test]
fn the_vocal_effects_draw_their_key() {
    use mpfree_fx::ids::{harmonizer as harm, pitch_correct as tune};
    let theme = mpfree_ui::Theme::dark();
    let card = |st: &FxState, page: &str| {
        let scratch = mpfree_fx_ui::Scratch::new(st);
        mpfree_fx_ui::visual::strip(st, page, None, &scratch, &theme)
            .cards
            .remove(0)
    };
    // Pitch correct in C major at full Amount: stairs, one flat step per
    // note of the scale; the grid names its seven notes.
    let full = card(
        &state(
            "mpfree.fx.pitch_correct",
            &[(tune::SCALE, 1.0), (tune::AMOUNT, 100.0)],
        ),
        "Pitch correct",
    );
    assert_eq!(full.title, "Tune");
    let mut levels: Vec<f32> = full.traces[0].pts.iter().map(|p| p.1).collect();
    levels.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    assert_eq!(levels.len(), 8, "C to C: {levels:?}");
    let names: Vec<&str> = full.grid.iter().map(|g| g.1.as_str()).collect();
    assert_eq!(names, ["C", "D", "E", "F", "G", "A", "B"]);
    // At zero Amount the line is the dashed one: nothing changes.
    let none = card(
        &state("mpfree.fx.pitch_correct", &[(tune::AMOUNT, 0.0)]),
        "Pitch correct",
    );
    let (a, b) = (&none.traces[0].pts, &none.traces[1].pts);
    assert!((a[0].1 - b[0].1).abs() < 1e-4 && (a[a.len() - 1].1 - b[1].1).abs() < 1e-4);

    // The Harmonizer, a 3rd up in C major: four semitones on C, F and G,
    // three on D, E, A and B.
    let h = card(
        &state(
            "mpfree.fx.harmonizer",
            &[(harm::SCALE, 1.0), (harm::VOICE1, 7.0), (harm::VOICE2, 0.0)],
        ),
        "Harmonizer",
    );
    assert_eq!(h.title, "Voices");
    let step = |n: usize| ((h.traces[0].pts[2 * n].1 - 0.5) * 26.0).round() as i32;
    let thirds: Vec<i32> = [0, 2, 4, 5, 7, 9, 11].iter().map(|&n| step(n)).collect();
    assert_eq!(thirds, [4, 3, 3, 4, 4, 3, 3]);
    assert!(h.caption.contains("3rd up"), "{}", h.caption);
}
