//! The XY FX page: the pad opens the effect where the finger is and closes
//! it on the lift unless Hold is on, the effects grid switches what X and
//! Y play, and every move reaches the node as a parameter change.

use mpfree_fx::ids::xyfx as xy;
use mpfree_fx::mpfree_node::Node;
use mpfree_fx::xyfx::axes;
use mpfree_fx_ui::perform::XyView;
use mpfree_fx_ui::sample::{app_ui, BPM, RATE};
use mpfree_fx_ui::xyfx::{self, axis_text, EffectGrid};
use mpfree_fx_ui::{make, FxAction, FxState, Locks, XyFxPanel};
use mpfree_synth_ui::widgets::{QLinkStrip, TileGrid};
use mpfree_ui::{Color, EventKind, Fonts, Touch, Ui, UiEvent};

fn node(values: &[(u16, f32)]) -> Box<dyn Node> {
    let mut n = make("mpfree.fx.xy").unwrap();
    n.prepare(RATE, 512);
    for &(k, v) in values {
        n.set_param(k, v);
    }
    n
}

fn page(values: &[(u16, f32)]) -> (Ui, XyFxPanel) {
    let n = node(values);
    let st = FxState::from_node(
        n.as_ref(),
        "Master · insert 2",
        Color::rgb(0x6EA8FE),
        BPM,
        RATE,
    );
    let mut ui = app_ui(Fonts::embedded(), "");
    let p = XyFxPanel::add(&mut ui, "xy", st).unwrap();
    ui.show("xy").unwrap();
    (ui, p)
}

/// The pad's playing area.
fn area(ui: &Ui) -> mpfree_ui::Rect {
    XyView::area(ui.rect_of(xyfx::PAD).unwrap())
}

/// The screen point at pad place (x, y), y up.
fn at(ui: &Ui, x: f32, y: f32) -> (i32, i32) {
    let a = area(ui);
    (
        a.x + (x * a.w as f32).round() as i32,
        a.bottom() - (y * a.h as f32).round() as i32,
    )
}

fn touch(
    ui: &mut Ui,
    p: &mut XyFxPanel,
    finger: u64,
    phase: Touch,
    x: i32,
    y: i32,
) -> Vec<FxAction> {
    match ui.touch(finger, phase, x, y) {
        Some(e) => p.event(ui, &e),
        None => Vec::new(),
    }
}

fn tap_hold(ui: &mut Ui, p: &mut XyFxPanel) -> Vec<FxAction> {
    let h = XyView::hold_rect(ui.rect_of(xyfx::PAD).unwrap());
    let out = touch(ui, p, 9, Touch::Down, h.x + 6, h.y + 6);
    ui.touch(9, Touch::Up, h.x + 6, h.y + 6);
    out
}

fn sent(out: &[FxAction], want: u16) -> Option<f32> {
    out.iter().find_map(|a| match a {
        FxAction::Set { id, value } if *id == want => Some(*value),
        _ => None,
    })
}

#[test]
fn only_xy_fx_gets_this_page() {
    let n = make("mpfree.fx.delay").unwrap();
    let st = FxState::from_node(n.as_ref(), "", Color::rgb(0), BPM, RATE);
    let mut ui = app_ui(Fonts::embedded(), "");
    assert!(XyFxPanel::add(&mut ui, "xy", st).is_err());
}

#[test]
fn a_touch_opens_the_effect_where_the_finger_is_and_the_lift_closes_it() {
    let (mut ui, mut p) = page(&[(xy::ACTIVE, 0.0), (xy::EFFECT, 1.0)]);
    assert!(!p.hold(), "closed as it was left: Hold is off");
    // Down at a quarter across, three quarters up: X, Y and Active.
    let (x, y) = at(&ui, 0.25, 0.75);
    let out = touch(&mut ui, &mut p, 1, Touch::Down, x, y);
    assert!((sent(&out, xy::X).unwrap() - 25.0).abs() < 0.5, "{out:?}");
    assert!((sent(&out, xy::Y).unwrap() - 75.0).abs() < 0.5, "{out:?}");
    assert_eq!(sent(&out, xy::ACTIVE), Some(1.0));
    assert!(p.touched());
    // Moving plays X and Y only.
    let (x2, y2) = at(&ui, 0.6, 0.5);
    let out = touch(&mut ui, &mut p, 1, Touch::Move, x2, y2);
    assert!((sent(&out, xy::X).unwrap() - 60.0).abs() < 0.5);
    assert!((sent(&out, xy::Y).unwrap() - 50.0).abs() < 0.5);
    assert_eq!(sent(&out, xy::ACTIVE), None);
    // The Echo's time at 60 % across is the fifth column, 1/8D: the tile,
    // the Q-Link and the pad's readout all say so.
    let tiles = &ui.get::<TileGrid>(xyfx::TILES).unwrap().tiles;
    assert_eq!(
        (tiles[0].label.as_str(), tiles[0].value.as_str()),
        ("Time", "1/8D")
    );
    assert_eq!(tiles[1].label, "Feedback");
    let q = &ui.get::<QLinkStrip>(xyfx::QLINKS).unwrap().cells;
    assert_eq!((q[0].label.as_str(), q[0].value.as_str()), ("Time", "1/8D"));
    let pad = ui.get::<XyView>(xyfx::PAD).unwrap();
    assert!(
        pad.readout.starts_with("Time 1/8D · Feedback"),
        "{}",
        pad.readout
    );
    assert!(pad.touched && !pad.show_rest);
    assert_eq!(pad.columns.len(), 8, "the Echo's eight times");
    // The lift closes it, so the echoes ring out.
    let out = touch(&mut ui, &mut p, 1, Touch::Up, x2, y2);
    assert_eq!(
        out,
        vec![FxAction::Set {
            id: xy::ACTIVE,
            value: 0.0
        }]
    );
    assert!(!p.touched());
    // The node takes the values the page sent as they are.
    let mut n = node(&[(xy::EFFECT, 1.0)]);
    for id in [xy::X, xy::Y, xy::ACTIVE] {
        n.set_param(id, p.state.value(id));
        assert_eq!(n.param(id), p.state.value(id));
    }
    // A second finger can't take the pad from the first.
    let out = touch(&mut ui, &mut p, 2, Touch::Down, x, y);
    assert_eq!(sent(&out, xy::ACTIVE), Some(1.0));
    let out = touch(&mut ui, &mut p, 3, Touch::Down, x2, y2);
    assert!(out.is_empty());
    touch(&mut ui, &mut p, 2, Touch::Up, x, y);
}

#[test]
fn hold_keeps_it_open_and_a_tap_on_hold_opens_or_closes_it_in_place() {
    let (mut ui, mut p) = page(&[(xy::ACTIVE, 0.0)]);
    // Hold with no finger on the pad opens the effect where it is.
    assert_eq!(
        tap_hold(&mut ui, &mut p),
        vec![FxAction::Set {
            id: xy::ACTIVE,
            value: 1.0
        }]
    );
    assert!(p.hold());
    assert!(ui.get::<XyView>(xyfx::PAD).unwrap().hold);
    // A touch moves it and the lift leaves it open.
    let (x, y) = at(&ui, 0.2, 0.4);
    let out = touch(&mut ui, &mut p, 1, Touch::Down, x, y);
    assert_eq!(sent(&out, xy::ACTIVE), None, "already open");
    assert!(touch(&mut ui, &mut p, 1, Touch::Up, x, y).is_empty());
    assert_eq!(p.state.value(xy::ACTIVE), 1.0);
    // Hold off closes it.
    assert_eq!(
        tap_hold(&mut ui, &mut p),
        vec![FxAction::Set {
            id: xy::ACTIVE,
            value: 0.0
        }]
    );
    // Hold turned on while the finger is down: the lift leaves it open.
    let out = touch(&mut ui, &mut p, 2, Touch::Down, x, y);
    assert_eq!(sent(&out, xy::ACTIVE), Some(1.0));
    assert!(
        tap_hold(&mut ui, &mut p).is_empty(),
        "the finger has it open"
    );
    assert!(touch(&mut ui, &mut p, 2, Touch::Up, x, y).is_empty());
    assert_eq!(p.state.value(xy::ACTIVE), 1.0);
    // The Active tile closes it, and Hold follows.
    let k = xyfx::TILE_IDS
        .iter()
        .position(|&i| i == xy::ACTIVE)
        .unwrap();
    let out = p.event(
        &mut ui,
        &UiEvent {
            widget: xyfx::TILES.to_string(),
            kind: EventKind::Step {
                index: k,
                delta: -1,
            },
        },
    );
    assert_eq!(
        out,
        vec![FxAction::Set {
            id: xy::ACTIVE,
            value: 0.0
        }]
    );
    assert!(!p.hold());
}

#[test]
fn an_effect_that_is_open_as_the_page_opens_shows_hold_lit() {
    // Active is on by default, so the insert plays from its tiles.
    let (mut ui, mut p) = page(&[]);
    assert!(p.hold());
    let (x, y) = at(&ui, 0.5, 0.5);
    touch(&mut ui, &mut p, 1, Touch::Down, x, y);
    assert!(touch(&mut ui, &mut p, 1, Touch::Up, x, y).is_empty());
}

#[test]
fn a_tap_on_an_effect_switches_it_and_the_pad_names_what_x_and_y_play() {
    let (mut ui, mut p) = page(&[]);
    let r = ui.rect_of(xyfx::EFFECTS).unwrap();
    let names = &ui.get::<EffectGrid>(xyfx::EFFECTS).unwrap().names;
    assert_eq!(names.len(), 9);
    let gate = names.iter().position(|n| n == "Gate").unwrap();
    let c = EffectGrid::cell(r, gate);
    let out = touch(&mut ui, &mut p, 1, Touch::Down, c.x + 10, c.y + 10);
    assert_eq!(
        out,
        vec![FxAction::Set {
            id: xy::EFFECT,
            value: gate as f32
        }]
    );
    ui.touch(1, Touch::Up, c.x + 10, c.y + 10);
    assert_eq!(p.effect(), gate);
    let grid = ui.get::<EffectGrid>(xyfx::EFFECTS).unwrap();
    assert_eq!(grid.selected, gate);
    assert_eq!(grid.details[gate], "Rate · Depth");
    // The Gate's rate is a beat division: six named columns, no names at
    // the left and right ends.
    let pad = ui.get::<XyView>(xyfx::PAD).unwrap();
    assert_eq!(pad.columns, ["1/32", "1/16T", "1/16", "1/8T", "1/8", "1/4"]);
    assert!(pad.edges[0].is_empty() && pad.edges[1].is_empty());
    assert_eq!(pad.title, "XY · Gate");
    let tiles = &ui.get::<TileGrid>(xyfx::TILES).unwrap().tiles;
    assert_eq!(tiles[0].label, "Rate");
    assert_eq!(tiles[1].label, "Depth");
    // A smooth X names its ends: the Dub echo's time, 60 ms to 900 ms.
    let dub = 2;
    let c = EffectGrid::cell(r, dub);
    touch(&mut ui, &mut p, 2, Touch::Down, c.x + 10, c.y + 10);
    let pad = ui.get::<XyView>(xyfx::PAD).unwrap();
    assert!(pad.columns.is_empty());
    assert_eq!(pad.edges[0], "60 ms");
    assert_eq!(pad.edges[1], "900 ms");
    // Halfway across in ratio: about 232 ms.
    let (ax, _) = axes(dub);
    assert_eq!(axis_text(ax, 0.5), ("232".to_string(), "ms"));
}

#[test]
fn the_filter_reads_as_low_pass_off_and_high_pass() {
    let (ax, ay) = axes(0);
    assert_eq!(axis_text(ax, 0.25), ("LP 50".to_string(), "%"));
    assert_eq!(axis_text(ax, 0.5), ("off".to_string(), ""));
    assert_eq!(axis_text(ax, 1.0), ("HP 100".to_string(), "%"));
    assert_eq!(axis_text(ay, 1.0), ("90".to_string(), "%"));
    // The Crush's bits read as whole numbers.
    let (_, bits) = axes(4);
    assert_eq!(axis_text(bits, 0.0), ("16".to_string(), ""));
}

#[test]
fn the_wheel_picks_the_effect_and_q_links_turn_x_y_mix_and_release() {
    let (mut ui, mut p) = page(&[]);
    assert_eq!(
        p.wheel(&mut ui, 1),
        Some(FxAction::Set {
            id: xy::EFFECT,
            value: 1.0
        })
    );
    assert_eq!(p.effect(), 1);
    match p.qlink(&mut ui, 0, 10) {
        Some(FxAction::Set { id, value }) => {
            assert_eq!(id, xy::X);
            assert!(value > 50.0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(p.focus(), Some(xy::X));
    // With X selected the wheel turns it.
    let before = p.state.value(xy::X);
    p.wheel(&mut ui, -5);
    assert!(p.state.value(xy::X) < before);
    let ids: Vec<u16> = (0..4)
        .filter_map(|n| match p.qlink(&mut ui, n, 1) {
            Some(FxAction::Set { id, .. }) => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        [xy::X, xy::Y, xy::RELEASE],
        "Mix is at its top already"
    );
}

#[test]
fn a_held_step_locks_the_pad_and_leaves_the_effect_alone() {
    let (mut ui, mut p) = page(&[(xy::ACTIVE, 0.0)]);
    p.set_locks(&mut ui, Some(Locks::new("Step 3")));
    let (x, y) = at(&ui, 0.75, 0.25);
    let out = touch(&mut ui, &mut p, 1, Touch::Down, x, y);
    assert_eq!(out.len(), 2, "{out:?}");
    for a in &out {
        match a {
            FxAction::Lock { id, value } if *id == xy::X => assert!((value - 75.0).abs() < 0.5),
            FxAction::Lock { id, value } if *id == xy::Y => assert!((value - 25.0).abs() < 0.5),
            other => panic!("{other:?}"),
        }
    }
    assert!(touch(&mut ui, &mut p, 1, Touch::Up, x, y).is_empty());
    assert!(tap_hold(&mut ui, &mut p).is_empty());
    assert_eq!(p.state.value(xy::X), 50.0, "the effect's own X stays");
    assert_eq!(p.state.value(xy::ACTIVE), 0.0);
    // The page shows the step's place, and unlock takes both off.
    assert!((ui.get::<XyView>(xyfx::PAD).unwrap().pos.0 - 0.75).abs() < 0.01);
    let all = p.unlock_all(&mut ui);
    assert_eq!(all.len(), 2);
    assert!(p.locks().unwrap().values.is_empty());
}

#[test]
fn the_trail_runs_out_after_the_lift() {
    let (mut ui, mut p) = page(&[]);
    for (k, x) in [0.2, 0.3, 0.4, 0.5].iter().enumerate() {
        let (px, py) = at(&ui, *x, 0.5);
        let phase = if k == 0 { Touch::Down } else { Touch::Move };
        touch(&mut ui, &mut p, 1, phase, px, py);
    }
    assert_eq!(ui.get::<XyView>(xyfx::PAD).unwrap().trail.len(), 4);
    p.tick(&mut ui, 16.0);
    assert_eq!(
        ui.get::<XyView>(xyfx::PAD).unwrap().trail.len(),
        4,
        "held: it stays"
    );
    let (px, py) = at(&ui, 0.5, 0.5);
    touch(&mut ui, &mut p, 1, Touch::Up, px, py);
    for _ in 0..10 {
        p.tick(&mut ui, 16.0);
    }
    assert!(ui.get::<XyView>(xyfx::PAD).unwrap().trail.is_empty());
}
