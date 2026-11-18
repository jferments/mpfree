//! The Grain tab: what it shows of a layer's grain settings and the cloud
//! the engine plays, and how its arrows, the wheel, the Q-Links and its
//! drags change the region in a real project, one undo step each.

use mpfree_sampler::engine::{CloudState, Engine, EngineView, Event};
use mpfree_sampler::model::{
    GrainFilter, GrainFilterKind, GrainMode, GrainQuality, GrainWindow, LfoShape, MotionTarget,
    PlayMode, Project,
};
use mpfree_sampler_ui::grain_tab::{
    self, describe, size_at, size_place, CloudNow, GrainAction, GrainState, Row, View,
};
use mpfree_sampler_ui::grain_widgets::{GrainCloud, XyPad, TRAIL};
use mpfree_sampler_ui::panel::Look;
use mpfree_sampler_ui::sample::{
    self, grain_project, grain_samples, grains_after, GrainSample, PAD, RATE,
};
use mpfree_sampler_ui::widgets::{KnobCard, PadBar, SectionChips, SettingList};
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

/// Where `row` is in the list the section open shows for the layer's
/// grain mode.
fn index(s: &GrainSample, row: Row) -> usize {
    row.index_in(s.tab.view(), s.state.region.grain_mode)
        .unwrap_or_else(|| panic!("{row:?} isn't shown"))
}

/// Taps the section chip `view`, as a finger would.
fn open(s: &mut GrainSample, view: View) {
    let i = View::ALL.iter().position(|v| *v == view).unwrap();
    let ev = UiEvent {
        widget: grain_tab::SECTIONS.into(),
        kind: EventKind::Tab(i),
    };
    let st = s.state.clone();
    assert!(s.tab.action(&mut s.ui, &ev, &st).is_none());
    assert_eq!(s.tab.view(), view);
}

/// Opens the section whose panel holds `row`, unless the one open shows it
/// (the Q-Links' rows are on every one).
fn open_for(s: &mut GrainSample, row: Row) {
    if row
        .index_in(s.tab.view(), s.state.region.grain_mode)
        .is_none()
    {
        open(s, row.view());
    }
}

/// What the list shows for `row` (its section opened first).
fn row_value(s: &mut GrainSample, row: Row) -> String {
    open_for(s, row);
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    list.rows[index(s, row)].value.clone()
}

/// A step of `row` from the panel (its section opened first): a knob's
/// drag, a selector's arrow, a choice's button `delta` from the lit one, a
/// switch's tap (which flips it whatever `delta` is).
fn step(s: &mut GrainSample, row: Row, delta: i32) -> Option<GrainAction> {
    open_for(s, row);
    let index = index(s, row);
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
    let mut s = grain(true);
    let bar = s.ui.get::<PadBar>(grain_tab::BAR).unwrap();
    assert_eq!(bar.title, "Pad A5 · Break 92");
    assert!(
        bar.subtitle
            .starts_with("Granular · Break 92 · 4.73 s long"),
        "{}",
        bar.subtitle
    );
    assert_eq!(bar.active, grain_tab::TAB);
    assert_eq!(bar.enabled, vec![true; 7], "every tab is built");
    for (row, value) in [
        (Row::Mode, "Granular"),
        (Row::Scan, "0.25×"),
        (Row::Spray, "16 %"),
        (Row::SprayShape, "Even"),
        (Row::SizeSpray, "0 %"),
        (Row::PitchSpray, "±0.5 st"),
        (Row::LevelSpray, "0 %"),
        (Row::Shape, "Tukey 0.5"),
        (Row::Reverse, "15 %"),
        (Row::Spread, "70 %"),
        (Row::Quantise, "Minor"),
        (Row::Sync, "Off"),
        (Row::Position, "56.0 %"),
        (Row::Size, "150 ms"),
        (Row::Density, "120 /s"),
        (Row::Pitch, "-5 st"),
        (Row::Level, "+0.0 dB"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
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
    assert_eq!(row_value(&mut s, Row::Scan), "0.25×");
    // The panel: Cloud's knobs and Shape's selectors; the Q-Links' rows
    // are on their cards and the touch pad.
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    let panel = list.panel.as_ref().unwrap();
    let titles: Vec<&str> = panel.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Cloud", "Shape"]);
    let placed: Vec<Row> = panel
        .sections
        .iter()
        .flat_map(|s| s.lines.iter().flatten().map(|i| Row::GRAINS[i.row]))
        .collect();
    assert_eq!(
        placed,
        [
            Row::Scan,
            Row::Spray,
            Row::SizeSpray,
            Row::PitchSpray,
            Row::LevelSpray,
            Row::Timing,
            Row::Reverse,
            Row::Spread,
            Row::Level,
            Row::SprayShape,
            Row::Mode,
            Row::GrainMode,
            Row::Shape,
            Row::Quantise,
            Row::Sync
        ]
    );
    assert!(grain_tab::QLINK_ROWS.iter().all(|r| !placed.contains(r)));
    let cloud = GrainMode::Cloud;
    assert!(matches!(
        list.controls[Row::Scan.index_in(View::Grains, cloud).unwrap()].look,
        Look::Knob { bipolar: true, .. }
    ));
    assert!(matches!(
        list.controls[Row::Shape.index_in(View::Grains, cloud).unwrap()].look,
        Look::Knob { .. }
    ));
    // The section chips: Grains open, each saying what is in it.
    let chips = s.ui.get::<SectionChips>(grain_tab::SECTIONS).unwrap();
    assert_eq!(chips.selected, 0);
    let chips: Vec<(&str, &str)> = chips
        .chips
        .iter()
        .map(|c| (c.name.as_str(), c.sub.as_str()))
        .collect();
    assert_eq!(
        chips,
        [
            ("Grains", "Cloud · scan 0.25× · spray 16 %"),
            ("Movement", "nothing moves"),
            ("Filter and keys", "no filter"),
            ("Presets", "your own settings")
        ]
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
fn the_cloud_shows_the_scans_path_the_spray_as_it_plays_and_when_it_is_thinned() {
    let mut s = grain(true);
    // The sample screen: a second of reports, 25 of them, behind the
    // reading place, a quarter speed scan moving it on.
    let (_, now, path) = sample::cloud_reports(&s.project, 1.5);
    let cloud = s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap();
    assert_eq!(cloud.trail(), &path[..]);
    assert_eq!(path.len(), TRAIL);
    assert!(path.windows(2).all(|w| w[1] > w[0]), "{path:?}");
    assert!(now.playhead.is_some_and(|h| h > path[TRAIL - 1]));
    assert!(cloud.scan > 0.0 && !cloud.peaked && !cloud.thinned);
    let state = now.state.expect("it plays");
    assert_eq!(cloud.spray_now, Some(state.spray));
    // A report of a moved spray and a thinned cloud.
    let st = s.state.clone();
    let grains = cloud.grains.clone();
    let moved = CloudNow {
        state: Some(CloudState {
            spray: 0.5,
            thinned: true,
        }),
        ..now
    };
    s.tab.set_cloud(&mut s.ui, &st, &grains, moved);
    let cloud = s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap();
    assert_eq!(cloud.spray_now, Some(0.5));
    assert!(cloud.thinned);
    assert!(cloud.caption.contains("spray ±25 %"), "{}", cloud.caption);
    assert!(
        cloud.caption.contains("thinned to fit 64"),
        "{}",
        cloud.caption
    );
    // The voice stops: the path and the spray as played go.
    s.tab.set_cloud(&mut s.ui, &st, &[], CloudNow::default());
    let cloud = s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap();
    assert!(cloud.trail().is_empty() && cloud.playhead.is_none());
    assert_eq!((cloud.spray_now, cloud.thinned), (None, false));
    // A peaked spray draws its middle stronger.
    step_apply(&mut s, Row::SprayShape, 1);
    assert!(s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap().peaked);
}

/// The rows each section's panel places, in its order.
fn placed(s: &GrainSample) -> Vec<Row> {
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    let shown = Row::shown(s.tab.view(), s.state.region.grain_mode);
    list.panel
        .as_ref()
        .unwrap()
        .sections
        .iter()
        .flat_map(|sec| sec.lines.iter().flatten().map(|i| shown[i.row]))
        .collect()
}

#[test]
fn the_section_chips_open_each_panel_and_the_touch_pad_and_qlinks_stay() {
    let mut s = grain(true);
    let xy = rect(&s, grain_tab::XY);
    for (k, view, titles) in [
        (1, View::Movement, &["LFO", "Envelope"][..]),
        (2, View::FilterKeys, &["Filter", "Keys and quality"][..]),
        (3, View::Presets, &["Presets"][..]),
        (0, View::Grains, &["Cloud", "Shape"][..]),
    ] {
        open(&mut s, view);
        assert_eq!(
            s.ui.get::<SectionChips>(grain_tab::SECTIONS)
                .unwrap()
                .selected,
            k
        );
        let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
        let got: Vec<&str> = list
            .panel
            .as_ref()
            .unwrap()
            .sections
            .iter()
            .map(|sec| sec.title.as_str())
            .collect();
        assert_eq!(got, titles, "{view:?}");
        // The touch pad keeps its place; the Q-Links turn their four.
        let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
        s.ui.paint(&mut c);
        assert_eq!(rect(&s, grain_tab::XY), xy, "{view:?}");
        let a = qlink(&mut s, 2, 1, false).expect("density moves");
        apply(&mut s, &a);
        assert_eq!(s.tab.row(&s.ui), Row::Density, "{view:?}");
        assert_eq!(s.tab.view(), view);
        s.tab.lift();
    }
    // In every grain mode each section places every row it shows but the
    // Q-Links', once.
    for mode in ["Cloud", "Stretch", "Repeat", "Tone"] {
        if mode != "Cloud" {
            step_apply(&mut s, Row::GrainMode, 1);
        }
        assert_eq!(row_value(&mut s, Row::GrainMode), mode);
        for view in View::ALL {
            open_for(&mut s, Row::Position);
            if s.tab.view() != view {
                open(&mut s, view);
            }
            let m = s.state.region.grain_mode;
            let want: Vec<Row> = Row::shown(view, m)
                .iter()
                .copied()
                .filter(|r| !grain_tab::QLINK_ROWS.contains(r))
                .collect();
            let mut got = placed(&s);
            assert_eq!(got.len(), want.len(), "{mode} {view:?}: {got:?}");
            got.sort_by_key(|r| want.iter().position(|w| w == r));
            assert_eq!(got, want, "{mode} {view:?}");
            for r in Row::shown(view, m) {
                assert!(
                    r.view() == view || grain_tab::QLINK_ROWS.contains(r),
                    "{r:?}"
                );
            }
        }
    }
    // Every sample screen fits its controls: none is drawn past its panel.
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
}

#[test]
fn a_layer_that_doesnt_play_grains_says_how_and_mode_turns_it_on() {
    let mut s = grain(false);
    let cloud = s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap();
    assert!(!cloud.granular);
    assert_eq!(cloud.note, "Mode is Loop: tap to play this pad as grains");
    assert_eq!(row_value(&mut s, Row::Mode), "Loop");
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

/// Live test 3: from a one-shot pad with no loop the Mode row stopped at
/// Toggle, because Loop without loop points is refused. Now it gives Loop
/// a loop, as the Wave tab does, and goes on to Granular.
#[test]
fn the_mode_row_reaches_granular_from_a_pad_with_no_loop() {
    let mut s = grain(false);
    {
        let pad = s.project.pad(PAD).unwrap();
        let mut r = pad.layers[0].region.clone();
        r.mode = PlayMode::OneShot;
        r.sustain_loop = None;
        let a = GrainAction::Region {
            pad: PAD,
            layer: 0,
            region: Box::new(r),
            gesture: 1,
        };
        apply(&mut s, &a);
    }
    assert_eq!(row_value(&mut s, Row::Mode), "One shot");
    for want in [
        PlayMode::Gate,
        PlayMode::Toggle,
        PlayMode::Loop,
        PlayMode::Granular,
    ] {
        step_apply(&mut s, Row::Mode, 1);
        let r = &s.project.pad(PAD).unwrap().layers[0].region;
        assert_eq!(r.mode, want);
        if want == PlayMode::Loop {
            assert!(r.sustain_loop.is_some(), "Loop got a loop");
        }
    }
}

/// The note over a layer that doesn't play grains is a button that does.
#[test]
fn the_play_as_grains_button_sets_granular() {
    let mut s = grain(false);
    let b = GrainCloud::note_button(rect(&s, grain_tab::CLOUD));
    assert!(b.h >= 64, "a finger-sized button");
    let acts = drag(
        &mut s,
        (b.x + b.w / 2, b.y + b.h / 2),
        (b.x + b.w / 2, b.y + b.h / 2),
    );
    lift(&mut s);
    let a = acts.first().expect("the tap asks for an edit");
    apply(&mut s, a);
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.mode,
        PlayMode::Granular
    );
    assert!(s.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap().granular);
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
    assert_eq!(row_value(&mut s, Row::Spray), "26 %");
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
        (Row::SprayShape, 1, "Peaked"),
        (Row::SprayShape, -1, "Even"),
        (Row::SizeSpray, 3, "15 %"),
        (Row::SizeSpray, 30, "100 %"),
        (Row::LevelSpray, 10, "50 %"),
        (Row::LevelSpray, -11, "0 %"),
        // From the old Tukey window (as a trapezoid, 20 %) onto the knob.
        (Row::Shape, 1, "25 % · Trapezoid"),
        (Row::Shape, -9, "0 % · Square"),
        (Row::Shape, 12, "60 % · Hann"),
        (Row::Shape, 4, "80 % · Swell"),
        (Row::Shape, 9, "100 % · Pluck"),
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
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert!(
        step(&mut s, Row::Density, 1).is_none(),
        "200 grains a second is the most"
    );
    assert!(
        step(&mut s, Row::SprayShape, -1).is_none(),
        "Even is the first"
    );
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.grain.window,
        GrainWindow::Shape { shape: 1.0 }
    );
}

#[test]
fn a_grain_mode_starts_from_its_own_settings_and_shows_its_rows() {
    let mut s = grain(true);
    let steps = undo_steps(&s.project);
    let names = |s: &GrainSample| -> Vec<String> {
        let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
        list.rows.iter().map(|r| r.name.clone()).collect()
    };
    assert_eq!(row_value(&mut s, Row::GrainMode), "Cloud");
    assert_eq!(names(&s).len(), Row::GRAINS.len());
    let (position, gain) = {
        let g = &s.state.region.grain;
        (g.position, g.gain)
    };
    // Pitch picked: it stays picked in a mode that shows it.
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, 3, 0, false);
    assert_eq!(s.tab.row(&s.ui), Row::Pitch);

    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Stretch");
    assert_eq!(
        names(&s),
        [
            "Scan",
            "Stereo",
            "Level",
            "Mode",
            "Grain mode",
            "Shape",
            "Position",
            "Size",
            "Density",
            "Pitch"
        ],
        "the panel's order"
    );
    // The panel names its first section after the mode and holds only the
    // mode's rows.
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    let panel = list.panel.as_ref().unwrap();
    let titles: Vec<&str> = panel.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Stretch", "Shape"]);
    let on_panel = panel
        .sections
        .iter()
        .flat_map(|s| s.lines.iter().flatten())
        .count();
    assert_eq!(on_panel, 6, "Scan, Stereo, Level; Mode, Grain mode, Shape");
    assert_eq!(list.controls.len(), list.rows.len());
    for (row, value) in [
        (Row::Scan, "1×"),
        (Row::Size, "90 ms"),
        (Row::Density, "45 /s"),
        (Row::Pitch, "+0 st"),
        (Row::Shape, "Hann"),
        (Row::Spread, "0 %"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    let g = s.state.region.grain;
    assert_eq!((g.position, g.gain), (position, gain), "kept");
    assert_eq!(
        (g.spray, g.pitch_spray, g.reverse, g.timing, g.scale),
        (0.0, 0.0, 0.0, 0.0, 0),
        "nothing random, so the grains stay in step"
    );
    assert_eq!(s.tab.row(&s.ui), Row::Pitch);
    // A sound with a tempo (from the Warp page) stretches in time.
    let mut r = s.state.region.clone();
    r.warp = Some(mpfree_sampler::model::Warp::new(92.0));
    assert_eq!(grain_tab::value_text(&r, Row::Scan), "1× · in time");

    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Repeat");
    assert_eq!(names(&s).len(), 10);
    assert_eq!(row_value(&mut s, Row::Density), "4 /beat");
    assert_eq!(row_value(&mut s, Row::Size), "1/4 beat");
    assert_eq!(row_value(&mut s, Row::Shape), "10 % · Trapezoid");
    let g = s.state.region.grain;
    assert_eq!((g.scan, g.timing), (0.0, 0.0), "the same spot, on the grid");
    // Its rate steps through beat divisions.
    step_apply(&mut s, Row::Density, 1);
    assert_eq!(row_value(&mut s, Row::Density), "6 /beat");

    // Back to a cloud: every row again, scattered a little.
    step_apply(&mut s, Row::GrainMode, -2);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Cloud");
    assert_eq!(names(&s).len(), Row::GRAINS.len());
    let g = s.state.region.grain;
    assert!(g.sync.is_none() && g.spray > 0.0 && g.spread > 0.0 && g.timing > 0.0);
    assert_eq!(s.tab.row(&s.ui), Row::Pitch);
    // A row the next mode hides gives way to the Grain mode row.
    let timing = index(&s, Row::Timing);
    s.ui.get_mut::<SettingList>(grain_tab::LIST)
        .unwrap()
        .selected = timing;
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(s.tab.row(&s.ui), Row::GrainMode);
    // A run of switches is one undo step (as for any row), and undo
    // brings the settings back.
    assert_eq!(undo_steps(&s.project), steps + 3);
    for _ in 0..3 {
        s.project.undo();
    }
    let r = &s.project.pad(PAD).unwrap().layers[0].region;
    assert_eq!(r.grain_mode, mpfree_sampler::model::GrainMode::Cloud);
    assert_eq!(r.grain.spray, 0.16);
}

#[test]
fn the_key_rows_set_key_tracking_and_glide() {
    let mut s = grain(true);
    for (row, delta, value) in [
        (Row::KeySize, 0, "Off"),
        (Row::KeyScan, 0, "Off"),
        (Row::Glide, 0, "Off"),
        (Row::KeySize, 1, "On"),
        (Row::KeyScan, 1, "On"),
        // A tap on a switch flips it.
        (Row::KeyScan, 1, "Off"),
        (Row::Glide, 4, "100 ms"),
        (Row::Glide, 20, "5.00 s"),
        (Row::Glide, -7, "150 ms"),
    ] {
        if delta != 0 {
            step_apply(&mut s, row, delta);
        }
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert_eq!(s.tab.view(), View::FilterKeys);
    let r = &s.state.region;
    assert!(r.grain_keys.size && !r.grain_keys.scan);
    assert_eq!(r.grain_glide_ms, 150.0);
    // The wheel turns a switch on and off, not round.
    let i = index(&s, Row::KeySize);
    s.ui.get_mut::<SettingList>(grain_tab::LIST)
        .unwrap()
        .selected = i;
    let st = s.state.clone();
    assert!(s.tab.wheel(&mut s.ui, &st, 1).is_none(), "already on");
    let a = s.tab.wheel(&mut s.ui, &st, -1).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::KeySize), "Off");
    step_apply(&mut s, Row::KeySize, 1);
    // A tone's size follows its note anyway: no row for it, and the
    // settings stay across modes.
    step_apply(&mut s, Row::GrainMode, 3);
    assert!(!Row::KeySize.used_in(GrainMode::Tone));
    assert_eq!(row_value(&mut s, Row::Glide), "150 ms");
    assert!(!placed(&s).contains(&Row::KeySize));
    assert!(s.state.region.grain_keys.size);
    // The chip says what keys do.
    let chips = s.ui.get::<SectionChips>(grain_tab::SECTIONS).unwrap();
    assert_eq!(chips.chips[2].sub, "no filter · glide 150 ms");
}

#[test]
fn the_quality_row_steps_clean_tape_and_lofi_in_every_mode() {
    let mut s = grain(true);
    assert_eq!(row_value(&mut s, Row::Quality), "Clean");
    for (delta, value) in [(1, "Tape"), (1, "Lo-fi"), (-2, "Clean"), (2, "Lo-fi")] {
        step_apply(&mut s, Row::Quality, delta);
        assert_eq!(row_value(&mut s, Row::Quality), value, "{delta:+}");
    }
    // Lo-fi is the last: it stops there.
    assert!(step(&mut s, Row::Quality, 1).is_none());
    assert_eq!(s.tab.view(), View::FilterKeys);
    assert_eq!(s.state.region.grain.quality, GrainQuality::LoFi);
    // The chip says it before what keys do.
    step_apply(&mut s, Row::Glide, 4);
    let chips = s.ui.get::<SectionChips>(grain_tab::SECTIONS).unwrap();
    assert_eq!(chips.chips[2].sub, "no filter · Lo-fi");
    // Every grain mode has it, and keeps it.
    for (k, mode) in [GrainMode::Stretch, GrainMode::Repeat, GrainMode::Tone]
        .into_iter()
        .enumerate()
    {
        assert!(Row::Quality.used_in(mode), "{mode:?}");
        step_apply(&mut s, Row::GrainMode, 1);
        assert_eq!(s.state.region.grain_mode, mode, "step {k}");
        assert_eq!(row_value(&mut s, Row::Quality), "Lo-fi", "{mode:?}");
        open(&mut s, View::FilterKeys);
        assert!(placed(&s).contains(&Row::Quality), "{mode:?}");
    }
    // A preset keeps it.
    open(&mut s, View::Presets);
    let a = tap_preset(&mut s, 0).expect("a preset");
    apply(&mut s, &a);
    assert_eq!(s.state.region.grain.quality, GrainQuality::LoFi);
}

#[test]
fn the_movement_rows_set_the_lfo_and_the_envelope() {
    let mut s = grain(true);
    for (row, value) in [
        (Row::LfoTo, "Off"),
        (Row::LfoShape, "Sine"),
        (Row::LfoRate, "1.0 Hz"),
        (Row::LfoSync, "Off"),
        (Row::LfoDepth, "+50 %"),
        (Row::EnvTo, "Off"),
        (Row::EnvDepth, "+50 %"),
        (Row::EnvAttack, "1.00 s"),
        (Row::EnvDecay, "1.00 s"),
        (Row::EnvSustain, "100 %"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    let note = |s: &GrainSample, k: usize| {
        let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
        list.panel.as_ref().unwrap().sections[k].note.clone()
    };
    assert_eq!(note(&s, 0), "off: pick what it moves");
    for (row, delta, value) in [
        (Row::LfoTo, 1, "Position"),
        (Row::LfoTo, 3, "Pitch"),
        (Row::LfoTo, 9, "Cutoff"),
        (Row::LfoTo, -1, "Spray"),
        (Row::LfoTo, -2, "Density"),
        (Row::LfoShape, 5, "Hold"),
        (Row::LfoShape, 9, "Drift"),
        (Row::LfoRate, 2, "2.0 Hz"),
        (Row::LfoRate, -6, "0.20 Hz"),
        (Row::LfoRate, 8, "4.0 Hz"),
        // Synced at 120 BPM: the cycle nearest 4 Hz, then a step faster.
        (Row::LfoSync, 1, "On"),
        (Row::LfoRate, 0, "1/2 beat"),
        (Row::LfoRate, -2, "2 beats"),
        (Row::LfoRate, 1, "1 beat"),
        (Row::LfoSync, 1, "Off"),
        (Row::LfoRate, 0, "2.0 Hz"),
        (Row::LfoDepth, -60, "-10 %"),
        (Row::LfoDepth, -200, "-100 %"),
        (Row::EnvTo, 2, "Size"),
        (Row::EnvDepth, 25, "+75 %"),
        (Row::EnvAttack, -1, "750 ms"),
        (Row::EnvAttack, -30, "0 ms"),
        (Row::EnvDecay, 2, "2.00 s"),
        (Row::EnvSustain, -10, "50 %"),
    ] {
        if delta != 0 {
            step_apply(&mut s, row, delta);
        }
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert_eq!(note(&s, 0), "over and over, from each hit");
    let chips = s.ui.get::<SectionChips>(grain_tab::SECTIONS).unwrap();
    assert_eq!(chips.chips[1].sub, "LFO to Density · envelope to Size");
    let g = s.state.region.grain;
    assert_eq!(
        (
            g.lfo.target,
            g.lfo.shape,
            g.lfo.rate_hz,
            g.lfo.beats,
            g.lfo.depth
        ),
        (MotionTarget::Density, LfoShape::Drift, 2.0, None, -1.0)
    );
    assert_eq!(
        (
            g.env.target,
            g.env.depth,
            g.env.attack_ms,
            g.env.decay_ms,
            g.env.sustain
        ),
        (MotionTarget::Size, 0.75, 0.0, 2000.0, 0.5)
    );
    // Every grain mode shows them, and keeps them: a tone's density is its
    // note.
    step_apply(&mut s, Row::GrainMode, 3);
    open(&mut s, View::Movement);
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    let names: Vec<&str> = list.rows.iter().map(|r| r.name.as_str()).collect();
    let motion: Vec<&str> = grain_tab::MOTION_ROWS.iter().map(|r| r.short()).collect();
    assert!(names.starts_with(&motion), "{names:?}");
    assert_eq!(s.state.region.grain.lfo, g.lfo);
    assert_eq!(s.state.region.grain.env, g.env);
    assert_eq!(row_value(&mut s, Row::LfoTo), "Note");
}

/// Taps preset `k`'s button (0 to 7) on the Presets section, as the panel
/// reports it: a step from its line's lit button (or the line's end) to it.
fn tap_preset(s: &mut GrainSample, k: usize) -> Option<GrainAction> {
    let row = if k < 4 { Row::Preset } else { Row::PresetMore };
    open_for(s, row);
    let index = index(s, row);
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    let Look::Choice { on, .. } = list.controls[index].look else {
        panic!("a preset's line is a choice")
    };
    let ev = UiEvent {
        widget: grain_tab::LIST.into(),
        kind: EventKind::Step {
            index,
            delta: (k % 4) as i32 - on as i32,
        },
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

/// The preset buttons' two lines: their names and the one lit (4: none).
fn preset_lines(s: &GrainSample) -> Vec<(Vec<String>, usize)> {
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    [Row::Preset, Row::PresetMore]
        .iter()
        .map(|r| match &list.controls[index(s, *r)].look {
            Look::Choice { options, on, .. } => (options.clone(), *on),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn the_presets_section_puts_a_starting_point_on_the_pad() {
    let mut s = grain(true);
    let steps = undo_steps(&s.project);
    let (position, gain) = {
        let g = &s.state.region.grain;
        (g.position, g.gain)
    };
    open(&mut s, View::Presets);
    assert_eq!(
        preset_lines(&s),
        vec![
            (
                vec![
                    "Glass pad".to_string(),
                    "Frozen choir".into(),
                    "Tape stretch".into(),
                    "Stutter 1/16".into()
                ],
                4
            ),
            (
                vec![
                    "Shimmer fifths".to_string(),
                    "Grit cloud".into(),
                    "Swarm".into(),
                    "Sub drone".into()
                ],
                4
            )
        ],
        "none lit: the pad's settings are its own"
    );
    assert_eq!(row_value(&mut s, Row::Preset), "—");
    let a = tap_preset(&mut s, 0).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Preset), "Glass pad");
    assert_eq!(row_value(&mut s, Row::Pitch), "+12 st");
    assert_eq!(preset_lines(&s)[0].1, 0);
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    let presets = mpfree_sampler::grain_presets::presets();
    assert_eq!(
        list.panel.as_ref().unwrap().sections[0].note,
        presets[0].about
    );
    // Each tap puts on that one, in its mode; the section stays open.
    for (k, name, mode) in [
        (2, "Tape stretch", "Stretch"),
        (3, "Stutter 1/16", "Repeat"),
        (7, "Sub drone", "Tone"),
    ] {
        let a = tap_preset(&mut s, k).unwrap();
        apply(&mut s, &a);
        assert_eq!(row_value(&mut s, Row::Preset), name);
        assert_eq!(grain_tab::value_text(&s.state.region, Row::GrainMode), mode);
        assert_eq!(s.tab.view(), View::Presets);
        let lit = preset_lines(&s);
        assert_eq!(
            (lit[0].1, lit[1].1),
            if k < 4 { (k, 4) } else { (4, k - 4) }
        );
    }
    let chips = s.ui.get::<SectionChips>(grain_tab::SECTIONS).unwrap();
    assert_eq!(chips.chips[3].sub, "Sub drone");
    assert_eq!(
        grain_tab::value_text(&s.state.region, Row::Density),
        "A1 · 55.0 Hz"
    );
    assert_eq!(grain_tab::value_text(&s.state.region, Row::Glide), "150 ms");
    let g = s.state.region.grain;
    assert_eq!((g.position, g.gain), (position, gain), "kept");
    // The wheel steps from one to the next; past the last, nothing.
    let st = s.state.clone();
    assert!(
        s.tab.wheel(&mut s.ui, &st, 1).is_none(),
        "Sub drone is the last"
    );
    let a = s.tab.wheel(&mut s.ui, &st, -1).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Preset), "Swarm");
    // A turn of any control leaves the preset as a starting point.
    step_apply(&mut s, Row::Density, 1);
    assert_eq!(row_value(&mut s, Row::Preset), "—");
    // The run of taps and turns was one undo step.
    assert_eq!(undo_steps(&s.project), steps + 2);
    s.project.undo();
    s.project.undo();
    let r = &s.project.pad(PAD).unwrap().layers[0].region;
    assert_eq!(r.grain_mode, GrainMode::Cloud);
    assert_eq!(grain_tab::value_text(r, Row::Preset), "—");
    // From none, a step down picks the last.
    let r = grain_tab::stepped(r, Row::Preset, -1, 120.0);
    assert_eq!(grain_tab::value_text(&r, Row::Preset), "Sub drone");
}

#[test]
fn the_filter_rows_colour_each_grain_and_a_new_mode_keeps_them() {
    let mut s = grain(true);
    for (row, value) in [
        (Row::Filter, "Off"),
        (Row::Cutoff, "2.00 kHz"),
        (Row::Resonance, "0 %"),
        (Row::FilterSpray, "Off"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    for (row, delta, value) in [
        (Row::Filter, 1, "Low pass"),
        (Row::Filter, 9, "High pass"),
        (Row::Filter, -1, "Band pass"),
        // A sixth of an octave a step.
        (Row::Cutoff, 6, "4.00 kHz"),
        (Row::Cutoff, -12, "1.00 kHz"),
        (Row::Cutoff, -200, "20.0 Hz"),
        (Row::Cutoff, 200, "20.00 kHz"),
        (Row::Cutoff, -18, "2.50 kHz"),
        (Row::Resonance, 9, "45 %"),
        (Row::FilterSpray, 10, "±1.5 oct"),
        (Row::FilterSpray, 20, "±3 oct"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    let f = s.state.region.grain.filter;
    assert_eq!(
        f,
        GrainFilter {
            kind: GrainFilterKind::Band,
            cutoff_hz: 2_500.0,
            resonance: 0.45,
            spray: 1.0,
        }
    );
    // The choice lights the pass, the chip says it.
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    assert!(matches!(
        &list.controls[index(&s, Row::Filter)].look,
        Look::Choice { on: 2, .. }
    ));
    let chips = s.ui.get::<SectionChips>(grain_tab::SECTIONS).unwrap();
    assert_eq!(chips.chips[2].sub, "Band pass 2.50 kHz");
    // The LFO can sweep it.
    step_apply(&mut s, Row::LfoTo, 6);
    assert_eq!(row_value(&mut s, Row::LfoTo), "Cutoff");
    // A stretch keeps the filter but not its spray (nothing random).
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(s.state.region.grain.filter, GrainFilter { spray: 0.0, ..f });
    assert_eq!(s.state.region.grain.lfo.target, MotionTarget::Cutoff);
    assert!(!Row::FilterSpray.used_in(GrainMode::Stretch));
    // A repeat and a tone keep all of it.
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::FilterSpray), "Off");
    step_apply(&mut s, Row::FilterSpray, 2);
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Tone");
    assert_eq!(row_value(&mut s, Row::FilterSpray), "±0.3 oct");
}

#[test]
fn a_tone_plays_a_note_that_steps_by_semitones_and_keeps_its_cycles() {
    let mut s = grain(true);
    step_apply(&mut s, Row::GrainMode, 3);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Tone");
    assert!(
        step(&mut s, Row::GrainMode, 1).is_none(),
        "Tone is the last mode"
    );
    let list = s.ui.get::<SettingList>(grain_tab::LIST).unwrap();
    assert_eq!(list.rows.len(), 12);
    assert_eq!(
        list.rows[Row::Density
            .index_in(View::Grains, GrainMode::Tone)
            .unwrap()]
        .name,
        "Density"
    );
    assert_eq!(row_value(&mut s, Row::Density), "A2 · 110 Hz");
    assert_eq!(row_value(&mut s, Row::Size), "36 ms · 4.0 cycles");
    let g = s.state.region.grain;
    assert_eq!(
        (g.timing, g.scan, g.spray),
        (0.0, 0.0, 0.0),
        "regular and still"
    );
    for (row, delta, value) in [
        (Row::Density, 3, "C3 · 131 Hz"),
        (Row::Size, 0, "31 ms · 4.0 cycles"),
        (Row::Size, 2, "38 ms · 5.0 cycles"),
        (Row::Density, 12, "C4 · 262 Hz"),
        (Row::Size, 0, "19 ms · 5.0 cycles"),
        (Row::Density, -60, "A0 · 27.5 Hz"),
        (Row::Density, 99, "B5 · 988 Hz"),
        (Row::Size, -20, "1 ms · 1.0 cycles"),
    ] {
        if delta != 0 {
            step_apply(&mut s, row, delta);
        }
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
}

#[test]
fn tempo_sync_locks_density_and_size_to_beats_and_level_sets_the_cloud() {
    let mut s = grain(true);
    let before = undo_steps(&s.project);
    // 120 grains a second of 150 ms at 120 BPM: 60 a beat, 0.3 of a beat.
    step_apply(&mut s, Row::Sync, 1);
    assert_eq!(row_value(&mut s, Row::Sync), "Density");
    assert_eq!(
        row_value(&mut s, Row::Density),
        "32 /beat",
        "the most a beat"
    );
    assert_eq!(row_value(&mut s, Row::Size), "150 ms");
    step_apply(&mut s, Row::Sync, 1);
    assert_eq!(row_value(&mut s, Row::Sync), "Size");
    assert_eq!(row_value(&mut s, Row::Density), "120 /s", "free again");
    assert_eq!(
        row_value(&mut s, Row::Size),
        "1/4 beat",
        "nearest 0.3 of a beat"
    );
    step_apply(&mut s, Row::Sync, 1);
    assert_eq!(row_value(&mut s, Row::Sync), "Both");
    step_apply(&mut s, Row::Density, -3);
    assert_eq!(row_value(&mut s, Row::Density), "12 /beat");
    step_apply(&mut s, Row::Size, 1);
    assert_eq!(row_value(&mut s, Row::Size), "1/2 beat");
    let g = s.project.pad(PAD).unwrap().layers[0].region.grain;
    let t = g.sync.unwrap();
    assert_eq!((t.grains_per_beat, t.size_beats), (Some(12.0), Some(0.5)));
    assert_eq!(t.frames_per_beat, grain_tab::SAVED_FRAMES_PER_BEAT);
    assert_eq!((g.density, g.size_ms), (120.0, 150.0), "kept for later");
    // A synced size: the cloud draws its grains at the tempo's length,
    // the touch pad and Q-Link 2 step its beats.
    assert_eq!(s.state.size_ms(), 250.0);
    s.state.bpm = 60.0;
    assert_eq!(s.state.size_ms(), 500.0);
    let a = qlink(&mut s, 1, 2, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Size), "2 beats");
    let card = s.ui.get::<KnobCard>(grain_tab::QLINKS[1]).unwrap();
    assert_eq!((card.value.as_str(), card.sub.as_str()), ("2", "beats"));
    let card = s.ui.get::<KnobCard>(grain_tab::QLINKS[2]).unwrap();
    assert_eq!((card.value.as_str(), card.sub.as_str()), ("12", "/beat"));
    lift(&mut s);
    let r = rect(&s, grain_tab::XY);
    let acts = drag(
        &mut s,
        XyPad::point(r, 0.56, 0.75),
        XyPad::point(r, 0.56, 1.0),
    );
    apply(&mut s, acts.last().unwrap());
    lift(&mut s);
    assert_eq!(
        row_value(&mut s, Row::Size),
        "4 beats",
        "the top of the pad"
    );
    // Level: the cloud's gain, half a dB a step.
    step_apply(&mut s, Row::Level, -4);
    assert_eq!(row_value(&mut s, Row::Level), "-2.0 dB");
    let gain = s.project.pad(PAD).unwrap().layers[0].region.grain.gain;
    assert!((gain - 0.794).abs() < 1e-3, "{gain}");
    step_apply(&mut s, Row::Level, 99);
    assert_eq!(row_value(&mut s, Row::Level), "+12.0 dB");
    // Off frees both, back to what they were.
    step_apply(&mut s, Row::Sync, -3);
    assert_eq!(row_value(&mut s, Row::Sync), "Off");
    assert_eq!(row_value(&mut s, Row::Size), "150 ms");
    assert_eq!(row_value(&mut s, Row::Density), "120 /s");
    assert!(s.project.pad(PAD).unwrap().layers[0]
        .region
        .grain
        .sync
        .is_none());
    assert!(step(&mut s, Row::Sync, -1).is_none(), "Off is the first");
    assert!(undo_steps(&s.project) > before);
    let a = grain_project().0.pad(PAD).unwrap().layers[0].region.clone();
    let mut b = a.clone();
    b.grain.gain = s.project.pad(PAD).unwrap().layers[0].region.grain.gain;
    b.grain.sync = Some(mpfree_sampler::model::TempoSync {
        frames_per_beat: 22_050.0,
        grains_per_beat: Some(4.0),
        size_beats: None,
    });
    assert_eq!(
        describe(&a, &b),
        "tempo sync Off -> Density, density 120 /s -> 4 /beat, level +0.0 dB -> +12.0 dB"
    );
}

#[test]
fn qlinks_turn_their_four_and_shift_turns_the_sprays() {
    let mut s = grain(true);
    let a = qlink(&mut s, 0, 4, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Position),
        "58.0 %",
        "half a per cent a detent"
    );
    let a = qlink(&mut s, 1, 8, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Size),
        "300 ms",
        "eight detents double it"
    );
    let a = qlink(&mut s, 1, -3, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Size),
        "297 ms",
        "SHIFT: a millisecond a detent"
    );
    let a = qlink(&mut s, 2, -8, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Density), "60 /s");
    let a = qlink(&mut s, 3, -40, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Pitch),
        "-36 st",
        "pitch stops at three octaves"
    );
    let a = qlink(&mut s, 0, 5, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        (row_value(&mut s, Row::Spray), s.tab.row(&s.ui)),
        ("21 %".into(), Row::Spray),
        "SHIFT + Q-Link 1: spray"
    );
    let a = qlink(&mut s, 3, 15, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::PitchSpray),
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
    assert_eq!(row_value(&mut s, Row::Scan), "0.125×");
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
    assert_eq!(row_value(&mut s, Row::Density), "150 /s");
    assert!(
        s.ui.get::<KnobCard>(grain_tab::QLINKS[2]).unwrap().focused,
        "its card shows the wheel is on it"
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
