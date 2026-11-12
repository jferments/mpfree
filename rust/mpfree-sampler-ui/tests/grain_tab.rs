//! The Grain tab: what it shows of a layer's grain settings and the cloud
//! the engine plays, and how its arrows, the wheel, the Q-Links and its
//! drags change the region in a real project, one undo step each.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{GrainWindow, PlayMode, Project};
use mpfree_sampler_ui::grain_tab::{
    self, describe, size_at, size_place, GrainAction, GrainState, Row,
};
use mpfree_sampler_ui::grain_widgets::{GrainCloud, XyPad};
use mpfree_sampler_ui::sample::{
    self, grain_project, grain_samples, grains_after, GrainSample, PAD, RATE,
};
use mpfree_sampler_ui::widgets::{InfoCard, KnobCard, PadBar, SettingList};
use mpfree_ui::{EventKind, Fonts, Rect, SoftCanvas, Touch, UiEvent};

/// The Grain tab on the granular pad (or on the loop, not granular yet),
/// drawn once so every widget has its place.
fn grain(granular: bool) -> GrainSample {
    let mut s = grain_samples(Fonts::embedded)
        .unwrap()
        .into_iter()
        .nth(if granular { 0 } else { 1 })
        .unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

/// How many undo steps the project holds (undone on a copy).
fn undo_steps(p: &Project) -> usize {
    let mut p = p.clone();
    let mut n = 0;
    while p.undo() {
        n += 1;
    }
    n
}

fn row_value(s: &GrainSample, row: Row) -> String {
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    list.rows[Row::ALL.iter().position(|r| *r == row).unwrap()]
        .value
        .clone()
}

fn step(s: &mut GrainSample, row: Row, delta: i32) -> Option<GrainAction> {
    let index = Row::ALL.iter().position(|r| *r == row).unwrap();
    let ev = UiEvent {
        widget: grain_tab::LIST.into(),
        kind: EventKind::Step { index, delta },
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

/// Applies an action and shows the new state, as the app does.
fn apply(s: &mut GrainSample, a: &GrainAction) {
    a.apply(&mut s.project).unwrap();
    let view = s.state.sound.view.clone();
    s.state = GrainState::from_project(&s.project, PAD, 0, |_| view.clone()).unwrap();
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
}

fn step_apply(s: &mut GrainSample, row: Row, delta: i32) {
    let a = step(s, row, delta).unwrap_or_else(|| panic!("{row:?} {delta:+} changed nothing"));
    apply(s, &a);
}

fn qlink(s: &mut GrainSample, knob: usize, delta: i32, shift: bool) -> Option<GrainAction> {
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, knob, delta, shift)
}

fn gesture(a: &GrainAction) -> u64 {
    match a {
        GrainAction::Region { gesture, .. } => *gesture,
        GrainAction::Tab(_) => panic!("not an edit"),
    }
}

/// A finger down at (x0, y0) and moved to (x1, y1), each event handed to
/// the tab: every action it asks for.
fn drag(s: &mut GrainSample, (x0, y0): (i32, i32), (x1, y1): (i32, i32)) -> Vec<GrainAction> {
    let mut out = Vec::new();
    for (phase, x, y) in [(Touch::Down, x0, y0), (Touch::Move, x1, y1)] {
        if let Some(ev) = s.ui.touch(1, phase, x, y) {
            let st = s.state.clone();
            out.extend(s.tab.action(&mut s.ui, &ev, &st));
        }
    }
    out
}

fn lift(s: &mut GrainSample) {
    s.ui.touch(1, Touch::Up, 0, 0);
    s.tab.lift();
}

fn rect(s: &GrainSample, name: &str) -> Rect {
    s.ui.rect_of(name)
        .unwrap_or_else(|| panic!("{name} has no place"))
}

#[test]
fn the_tab_shows_the_layers_grain_settings_and_the_engines_cloud() {
    let s = grain(true);
    let bar = s.ui.get::<PadBar>(grain_tab::BAR).unwrap();
    assert_eq!(bar.title, "Pad A5 · Break 92");
    assert!(
        bar.subtitle
            .starts_with("Granular · Break 92 · 4.73 s long"),
        "{}",
        bar.subtitle
    );
    assert_eq!(bar.active, grain_tab::TAB);
    assert_eq!(
        bar.enabled,
        vec![true, true, true, true, true, false, true],
        "every tab but FX is built"
    );
    for (row, value) in [
        (Row::Mode, "Granular"),
        (Row::Scan, "0.25×"),
        (Row::Spray, "16 %"),
        (Row::PitchSpray, "±0.5 st"),
        (Row::Window, "Tukey 0.5"),
        (Row::Reverse, "15 %"),
        (Row::Spread, "70 %"),
        (Row::Quantise, "Minor"),
        (Row::Position, "56.0 %"),
        (Row::Size, "150 ms"),
        (Row::Density, "120 /s"),
        (Row::Pitch, "-5 st"),
    ] {
        assert_eq!(row_value(&s, row), value, "{row:?}");
    }
    let cards: Vec<(String, String)> = grain_tab::QLINKS
        .iter()
        .map(|q| {
            let c = s.ui.get::<KnobCard>(q).unwrap();
            (c.value.clone(), c.sub.clone())
        })
        .collect();
    assert_eq!(
        cards,
        vec![
            ("56.0".into(), "%".into()),
            ("150".into(), "ms".into()),
            ("120".into(), "/s".into()),
            ("-5".into(), "st".into())
        ]
    );
    assert_eq!(
        s.tab.row(&s.ui),
        Row::Scan,
        "the wheel starts on the scan speed"
    );
    assert_eq!(
        s.ui.get::<InfoCard>(grain_tab::WHEEL).unwrap().text,
        "Scan speed: 0.25×"
    );
    let xy = s.ui.get::<XyPad>(grain_tab::XY).unwrap();
    assert_eq!((xy.x, xy.y), (0.56, size_place(150.0)));
    // The cloud: the engine's own grains, all inside the region.
    let cloud = s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap();
    assert!(cloud.granular);
    let (grains, head, live) = grains_after(&s.project, 1.5);
    assert_eq!(cloud.grains.len(), grains.len());
    assert!(
        (10..=30).contains(&grains.len()),
        "120/s of 150 ms grains: about 18, got {}",
        grains.len()
    );
    assert!(
        head.is_some_and(|h| h > 0.56),
        "a quarter speed scan has moved the reading place on: {head:?}"
    );
    assert!(
        cloud
            .caption
            .contains(&format!("{} grains playing", grains.len())),
        "{}",
        cloud.caption
    );
    assert_eq!(cloud.right, format!("{live} grains on all pads"));
}

#[test]
fn a_layer_that_doesnt_play_grains_says_how_and_mode_turns_it_on() {
    let mut s = grain(false);
    let cloud = s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap();
    assert!(!cloud.granular);
    assert_eq!(cloud.note, "Loop: set Mode to Granular to play grains");
    assert_eq!(row_value(&s, Row::Mode), "Loop");
    step_apply(&mut s, Row::Mode, 1);
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.mode,
        PlayMode::Granular
    );
    assert!(s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap().granular);
    // And the engine now plays it as grains.
    let mut e = Engine::new(RATE, 4);
    e.set_view(EngineView::build(&s.project, RATE));
    e.apply(Event::NoteOn {
        pad: PAD as u8,
        velocity: 100,
    });
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    for _ in 0..40 {
        e.render(&mut l, &mut r);
    }
    assert!(e.live_grains() > 0);
    assert!(
        step(&mut s, Row::Mode, 1).is_none(),
        "Granular is the last mode"
    );
}

#[test]
fn arrows_step_each_row_and_a_run_of_taps_on_one_row_is_one_undo_step() {
    let mut s = grain(true);
    let steps = undo_steps(&s.project);
    let a = step(&mut s, Row::Spray, 1).unwrap();
    let g = gesture(&a);
    apply(&mut s, &a);
    let b = step(&mut s, Row::Spray, 1).unwrap();
    assert_eq!(gesture(&b), g, "the same row again is the same undo step");
    apply(&mut s, &b);
    assert_eq!(row_value(&s, Row::Spray), "26 %");
    assert_eq!(undo_steps(&s.project), steps + 1);
    let c = step(&mut s, Row::Reverse, -1).unwrap();
    assert_ne!(gesture(&c), g, "another row is a new step");
    apply(&mut s, &c);
    assert_eq!(undo_steps(&s.project), steps + 2);
    s.project.undo();
    s.project.undo();
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.grain.spray,
        0.16
    );

    let mut s = grain(true);
    for (row, delta, value) in [
        (Row::Scan, 1, "0.5×"),
        (Row::Scan, -3, "Frozen"),
        (Row::PitchSpray, 2, "±1.5 st"),
        (Row::Window, 1, "Tukey 0.2"),
        (Row::Window, -9, "Hann"),
        (Row::Spread, 7, "100 %"),
        (Row::Quantise, 1, "Pentatonic"),
        (Row::Quantise, -9, "Off"),
        (Row::Position, -1, "55.0 %"),
        (Row::Size, 1, "200 ms"),
        (Row::Size, -2, "120 ms"),
        (Row::Density, 1, "150 /s"),
        (Row::Density, 9, "200 /s"),
        (Row::Pitch, 3, "-2 st"),
        (Row::Mode, -1, "Loop"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&s, row), value, "{row:?} {delta:+}");
    }
    assert!(
        step(&mut s, Row::Density, 1).is_none(),
        "200 grains a second is the most"
    );
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.grain.window,
        GrainWindow::Hann
    );
}

#[test]
fn qlinks_turn_their_four_and_shift_turns_the_sprays() {
    let mut s = grain(true);
    let a = qlink(&mut s, 0, 4, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&s, Row::Position),
        "58.0 %",
        "half a per cent a detent"
    );
    let a = qlink(&mut s, 1, 8, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&s, Row::Size),
        "300 ms",
        "eight detents double it"
    );
    let a = qlink(&mut s, 1, -3, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&s, Row::Size),
        "297 ms",
        "SHIFT: a millisecond a detent"
    );
    let a = qlink(&mut s, 2, -8, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&s, Row::Density), "60 /s");
    let a = qlink(&mut s, 3, -40, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&s, Row::Pitch),
        "-36 st",
        "pitch stops at three octaves"
    );
    let a = qlink(&mut s, 0, 5, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        (row_value(&s, Row::Spray), s.tab.row(&s.ui)),
        ("21 %".into(), Row::Spray),
        "SHIFT + Q-Link 1: spray"
    );
    let a = qlink(&mut s, 3, 15, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&s, Row::PitchSpray),
        "±2.0 st",
        "SHIFT + Q-Link 4: pitch spray"
    );
    // A turn is one undo step until it rests.
    let g1 = gesture(&qlink(&mut s, 2, 1, false).unwrap());
    let g2 = gesture(&qlink(&mut s, 2, 1, false).unwrap());
    assert_eq!(g1, g2);
    s.tab.lift();
    assert_ne!(gesture(&qlink(&mut s, 2, 1, false).unwrap()), g1);
    assert!(
        qlink(&mut s, 2, 0, false).is_none(),
        "a touch on a knob changes nothing"
    );
}

#[test]
fn the_wheel_steps_the_row_picked() {
    let mut s = grain(true);
    let st = s.state.clone();
    let a = s.tab.wheel(&mut s.ui, &st, -1).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&s, Row::Scan), "0.125×");
    // A tap on a Q-Link's card picks its row for the wheel.
    let ev = UiEvent {
        widget: grain_tab::QLINKS[2].into(),
        kind: EventKind::Select(0),
    };
    let st = s.state.clone();
    assert!(s.tab.action(&mut s.ui, &ev, &st).is_none());
    assert_eq!(s.tab.row(&s.ui), Row::Density);
    let st = s.state.clone();
    let a = s.tab.wheel(&mut s.ui, &st, 1).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&s, Row::Density), "150 /s");
    assert_eq!(
        s.ui.get::<InfoCard>(grain_tab::WHEEL).unwrap().text,
        "Density: 150 /s"
    );
}

#[test]
fn dragging_the_touch_pad_sets_position_and_size_and_the_sound_sets_position() {
    let mut s = grain(true);
    let r = rect(&s, grain_tab::XY);
    let (x0, y0) = XyPad::point(r, 0.56, size_place(150.0));
    let (x1, y1) = XyPad::point(r, 0.2, size_place(20.0));
    let acts = drag(&mut s, (x0, y0), (x1, y1));
    assert_eq!(acts.len(), 2, "down and move");
    assert_eq!(
        gesture(&acts[0]),
        gesture(&acts[1]),
        "one drag is one undo step"
    );
    apply(&mut s, acts.last().unwrap());
    lift(&mut s);
    let g = s.project.pad(PAD).unwrap().layers[0].region.grain;
    assert!((g.position - 0.2).abs() < 0.01, "{}", g.position);
    assert!((g.size_ms - 20.0).abs() <= 1.0, "{}", g.size_ms);
    assert_eq!(size_at(size_place(20.0)), 20.0);
    // A drag on the sound moves only the position.
    let c = GrainCloud::plot(rect(&s, grain_tab::CLOUD));
    let acts = drag(
        &mut s,
        (c.x + c.w / 2, c.y + 20),
        (c.x + c.w * 3 / 4, c.y + 30),
    );
    apply(&mut s, acts.last().unwrap());
    lift(&mut s);
    let after = s.project.pad(PAD).unwrap().layers[0].region.grain;
    assert!((after.position - 0.75).abs() < 0.01, "{}", after.position);
    assert_eq!(after.size_ms, g.size_ms);
    assert!(
        drag(&mut s, (c.x - 40, c.y + 20), (c.x - 30, c.y + 20)).is_empty(),
        "outside the sound: nothing"
    );
}

#[test]
fn applying_keeps_the_rest_of_the_pad_and_says_what_changed() {
    let mut s = grain(true);
    let before = s.project.pad(PAD).unwrap().clone();
    let a = step(&mut s, Row::Size, 1).unwrap();
    apply(&mut s, &a);
    let after = s.project.pad(PAD).unwrap();
    assert_eq!(after.amp, before.amp);
    assert_eq!(after.layers[0].region.start, before.layers[0].region.start);
    assert_eq!(
        describe(&before.layers[0].region, &after.layers[0].region),
        "size 150 ms -> 200 ms"
    );
    let bad = GrainAction::Region {
        pad: PAD,
        layer: 3,
        region: Box::new(after.layers[0].region.clone()),
        gesture: 1,
    };
    assert!(bad.apply(&mut s.project).is_err(), "no layer 4");
    assert!(GrainAction::Tab(2).apply(&mut s.project).is_ok());
}

#[test]
fn every_grain_sample_screen_draws() {
    for mut s in grain_samples(Fonts::embedded).unwrap() {
        let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
        s.ui.paint(&mut c);
        assert!(!s.about.is_empty(), "{}", s.name);
    }
    let (p, _) = grain_project();
    assert!(!grains_after(&p, 0.5).0.is_empty());
}
