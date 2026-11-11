//! The Play tab: what it shows of a pad, and how its arrows, the wheel, the
//! Q-Links and its drags change the pad in a real project, one undo step
//! each.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use std::sync::Arc;

use mpfree_sampler::model::{
    FilterKind, Keys, Layer, LayerSelect, Levels, Pad, PadsPlay, PressureTarget, Project, Region,
    Source, KEYGROUP_POLYPHONY,
};
use mpfree_sampler_ui::play_tab::{self, describe, PlayAction, PlayState, Row};
use mpfree_sampler_ui::play_widgets::{
    env_level, filter_db, EnvPoint, EnvelopeView, FilterCurve, LayerMap,
};
use mpfree_sampler_ui::sample::{self, layered_project, play_samples, PlaySample, PAD, RATE};
use mpfree_sampler_ui::source::SourceView;
use mpfree_sampler_ui::widgets::{KnobCard, PadBar, SettingList};
use mpfree_ui::{EventKind, Fonts, SoftCanvas, Touch, UiEvent};

/// The Play tab on the layered pad (or the plain one), drawn once so every
/// widget has its place.
fn play(layered: bool) -> PlaySample {
    let samples = play_samples(Fonts::embedded).unwrap();
    let mut s = samples
        .into_iter()
        .nth(if layered { 0 } else { 1 })
        .unwrap();
    if !layered {
        // Back to the pad as the sample project has it.
        let (project, view) = sample::project();
        s.state = PlayState::from_project(&project, PAD, |_| view.clone()).unwrap();
        s.project = project;
        let st = s.state.clone();
        s.tab.update(&mut s.ui, &st);
    }
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

fn row_value(s: &PlaySample, row: Row) -> String {
    let list = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    list.rows[Row::ALL.iter().position(|r| *r == row).unwrap()]
        .value
        .clone()
}

fn step(s: &mut PlaySample, row: Row, delta: i32) -> Option<PlayAction> {
    let index = Row::ALL.iter().position(|r| *r == row).unwrap();
    let ev = UiEvent {
        widget: play_tab::LIST.into(),
        kind: EventKind::Step { index, delta },
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

/// Applies an action and shows the new state, as the app does.
fn apply(s: &mut PlaySample, a: &PlayAction) {
    a.apply(&mut s.project).unwrap();
    let view = s.state.sounds[0].view.clone();
    s.state = PlayState::from_project(&s.project, PAD, |_| view.clone()).unwrap();
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
}

fn step_apply(s: &mut PlaySample, row: Row, delta: i32) {
    let a = step(s, row, delta).unwrap_or_else(|| panic!("{row:?} {delta:+} changed nothing"));
    apply(s, &a);
}

fn gesture(a: &PlayAction) -> u64 {
    match a {
        PlayAction::Pad { gesture, .. } => *gesture,
        _ => panic!("not an edit of the pad"),
    }
}

/// A finger down at (x0, y0) and moved to (x1, y1), each event handed to
/// the tab: what the move asks for.
fn drag(s: &mut PlaySample, (x0, y0): (i32, i32), (x1, y1): (i32, i32)) -> Option<PlayAction> {
    let mut last = None;
    for (phase, x, y) in [(Touch::Down, x0, y0), (Touch::Move, x1, y1)] {
        if let Some(ev) = s.ui.touch(1, phase, x, y) {
            let st = s.state.clone();
            last = s.tab.action(&mut s.ui, &ev, &st);
        }
    }
    last
}

/// The finger lifts: the drag is over, as the app sees it.
fn lift(s: &mut PlaySample) {
    s.ui.touch(1, Touch::Up, 0, 0);
    s.tab.lift();
}

#[test]
fn the_tab_shows_the_pads_layers_envelope_filter_and_settings() {
    let s = play(true);
    let bar = s.ui.get::<PadBar>(play_tab::BAR).unwrap();
    assert_eq!(bar.title, "Pad A5 · Break 92");
    assert_eq!(
        bar.subtitle,
        "3 layers · Loop · stacked · 4 voices · 4.73 s long"
    );
    assert_eq!(bar.active, play_tab::TAB);
    assert_eq!(
        bar.enabled,
        vec![true, true, true, true, true, false],
        "Pads, Wave, Chop, Play and Grain are built"
    );
    let map = s.ui.get::<LayerMap>(play_tab::LAYERS).unwrap();
    let ranges: Vec<(u8, u8)> = map.layers.iter().map(|l| (l.lo, l.hi)).collect();
    assert_eq!(ranges, vec![(1, 63), (64, 109), (110, 127)]);
    assert_eq!(map.layers[1].name, "Layer 2 · Break 92");
    assert_eq!(map.last_velocity, Some(108));
    let env = s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap();
    assert_eq!(
        env.caption,
        "A 2 ms · H 0 ms · D 310 ms · S 60 % · R 900 ms · over layer 1"
    );
    // S sits where the sound ends: the region is 7.25 beats at 92 BPM.
    assert!(
        (env.length_ms() - 7.25 * 60_000.0 / 92.0).abs() < 1.0,
        "{}",
        env.length_ms()
    );
    let filter = s.ui.get::<FilterCurve>(play_tab::FILTER).unwrap();
    assert_eq!(filter.title, "Filter · LP 2-pole");
    assert_eq!(filter.caption, "cutoff 2.40 kHz · res 35 % · env +0.50 oct");
    for (row, value) in [
        (Row::Polyphony, "4 voices"),
        (Row::MuteGroup, "Group 1"),
        (Row::Layers, "Stacked"),
        (Row::Level, "+0.0 dB"),
        (Row::Pan, "Center"),
        (Row::Tune, "+0 st"),
        (Row::Keys, "Off"),
        (Row::RootKey, "–"),
        (Row::PadsPlay, "Their sounds"),
        (Row::PadsFrom, "–"),
        (Row::VelToLevel, "100 %"),
        (Row::VelToStart, "20 %"),
        (Row::Pressure, "Filter cutoff"),
        (Row::Depth, "+60 %"),
        (Row::Release, "900 ms"),
        (Row::Filter, "LP 2-pole"),
        (Row::Cutoff, "2.40 kHz"),
        (Row::FilterEnv, "+0.50 oct"),
    ] {
        assert_eq!(row_value(&s, row), value, "{row:?}");
    }
    let cards: Vec<(String, String, bool)> = play_tab::QLINKS
        .iter()
        .map(|q| {
            let c = s.ui.get::<KnobCard>(q).unwrap();
            (c.value.clone(), c.sub.clone(), c.enabled)
        })
        .collect();
    assert_eq!(
        cards,
        vec![
            ("2".into(), "ms".into(), true),
            ("310".into(), "ms".into(), true),
            ("2.40".into(), "kHz".into(), true),
            ("35".into(), "%".into(), true)
        ]
    );
}

#[test]
fn arrows_change_the_pad_and_a_run_of_taps_on_one_row_is_one_undo_step() {
    let mut s = play(false);
    assert_eq!(row_value(&s, Row::Filter), "Off");
    // Turning the filter on starts a low-pass where it is heard.
    let a = step(&mut s, Row::Filter, 1).unwrap();
    let g = gesture(&a);
    apply(&mut s, &a);
    assert_eq!(s.state.settings.filter.kind, FilterKind::LowPass);
    assert_eq!(s.state.settings.filter.cutoff_hz, 2000.0);
    let a = step(&mut s, Row::Filter, 1).unwrap();
    assert_eq!(gesture(&a), g, "a second tap on the row merges");
    apply(&mut s, &a);
    assert_eq!(row_value(&s, Row::Filter), "LP 4-pole");
    // A lift ends the step; another row is another step anyway.
    s.tab.lift();
    let a = step(&mut s, Row::Cutoff, -3).unwrap();
    assert_ne!(gesture(&a), g);
    apply(&mut s, &a);
    assert_eq!(
        s.state.settings.filter.cutoff_hz, 1000.0,
        "three thirds of an octave down"
    );
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().filter.cutoff_hz, 2000.0);
    assert!(s.project.undo());
    assert_eq!(
        s.project.pad(PAD).unwrap().filter.kind,
        FilterKind::Off,
        "both taps on Filter undo together"
    );

    // Polyphony, mute group, layers, tune and pressure.
    let mut s = play(false);
    for (row, delta, value) in [
        (Row::Polyphony, 1, "12 voices"),
        (Row::Polyphony, -5, "2 voices"),
        (Row::Polyphony, -9, "Mono"),
        (Row::MuteGroup, 2, "Group 2"),
        (Row::MuteGroup, -5, "Off"),
        (Row::Layers, -1, "Random"),
        (Row::Level, -3, "-1.5 dB"),
        (Row::Pan, -5, "L 25"),
        (Row::Tune, 7, "+7 st"),
        (Row::Wow, 3, "30 %"),
        (Row::Wow, -5, "Off"),
        (Row::Sustain, -8, "60 %"),
        (Row::Release, 2, "20 ms"),
        (Row::FilterEnv, 3, "+0.75 oct"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&s, row), value, "{row:?} {delta:+}");
    }
    assert_eq!(s.state.settings.select, LayerSelect::Random);
    // Grain targets show only for a granular pad.
    let targets: Vec<String> = (0..4)
        .map(|_| {
            step_apply(&mut s, Row::Pressure, 1);
            row_value(&s, Row::Pressure)
        })
        .collect();
    assert_eq!(targets, vec!["Level", "Pitch", "Filter cutoff", "Off"]);
    step_apply(&mut s, Row::Pressure, 1);
    assert_eq!(
        (s.state.settings.pressure.target, row_value(&s, Row::Depth)),
        (PressureTarget::Level, "+50 %".into())
    );
    // Stepping past the ends stays there: nothing to undo.
    assert_eq!(step(&mut s, Row::MuteGroup, -1), None);
}

#[test]
fn tune_and_times_between_steps_go_to_the_next_step() {
    let (mut project, view) = sample::project();
    project
        .edit_pad(PAD, None, |p| {
            p.tune_cents = 150;
            p.amp.attack_ms = 7.0;
        })
        .unwrap();
    let state = PlayState::from_project(&project, PAD, |_| view.clone()).unwrap();
    let mut ui = sample::app_ui_with(Fonts::embedded(), &project, play_tab::HINT);
    let mut tab = play_tab::PlayTab::add(&mut ui, sample::PLAY_PAGE, &state).unwrap();
    let tune = Row::ALL.iter().position(|r| *r == Row::Tune).unwrap();
    let attack = Row::ALL.iter().position(|r| *r == Row::Attack).unwrap();
    let after = |a: Option<PlayAction>| match a {
        Some(PlayAction::Pad { after, .. }) => *after,
        a => panic!("{a:?}"),
    };
    let ev = |index, delta| UiEvent {
        widget: play_tab::LIST.into(),
        kind: EventKind::Step { index, delta },
    };
    assert_eq!(
        after(tab.action(&mut ui, &ev(tune, 1), &state)).tune_cents,
        200
    );
    assert_eq!(
        after(tab.action(&mut ui, &ev(tune, -1), &state)).tune_cents,
        100
    );
    assert_eq!(
        after(tab.action(&mut ui, &ev(attack, 1), &state))
            .amp
            .attack_ms,
        10.0
    );
    assert_eq!(
        after(tab.action(&mut ui, &ev(attack, -1), &state))
            .amp
            .attack_ms,
        5.0
    );
    assert_eq!(
        after(tab.action(&mut ui, &ev(attack, -2), &state))
            .amp
            .attack_ms,
        2.0
    );
}

#[test]
fn the_wheel_steps_the_row_picked_and_qlinks_turn_their_four() {
    let mut s = play(false);
    // A tap on Q-Link 2's card picks Decay for the wheel.
    let card = s.ui.rect_of(play_tab::QLINKS[1]).unwrap();
    let ev =
        s.ui.touch(1, Touch::Down, card.x + 10, card.y + 10)
            .unwrap();
    let st = s.state.clone();
    assert_eq!(s.tab.action(&mut s.ui, &ev, &st), None);
    assert_eq!(s.tab.row(&s.ui), Row::Decay);
    let a = s.tab.wheel(&mut s.ui, &st, 4).unwrap();
    apply(&mut s, &a);
    assert_eq!(s.state.settings.amp.decay_ms, 10.0, "0, 1, 2, 5, 10 ms");

    // Attack from 0: a millisecond, then a doubling every 8 detents.
    let turn = |s: &mut PlaySample, knob, delta, fine| {
        let st = s.state.clone();
        let a = s.tab.qlink(&mut s.ui, &st, knob, delta, fine);
        if let Some(a) = &a {
            apply(s, a);
        }
        a
    };
    turn(&mut s, 0, 1, false).unwrap();
    assert_eq!(s.state.settings.amp.attack_ms, 1.0);
    for _ in 0..3 {
        turn(&mut s, 0, 1, false);
    }
    assert_eq!(
        s.state.settings.amp.attack_ms, 4.0,
        "short times move a millisecond a detent"
    );
    turn(&mut s, 0, 8, false);
    assert_eq!(s.state.settings.amp.attack_ms, 8.0);
    turn(&mut s, 0, 3, true);
    assert_eq!(
        s.state.settings.amp.attack_ms, 11.0,
        "SHIFT: a millisecond a detent"
    );
    assert_eq!(s.tab.row(&s.ui), Row::Attack, "a turn picks its row");
    // Cutoff and resonance wait for a filter.
    assert_eq!(turn(&mut s, 2, 1, false), None);
    assert!(!s.ui.get::<KnobCard>(play_tab::QLINKS[2]).unwrap().enabled);
    step_apply(&mut s, Row::Filter, 1);
    turn(&mut s, 2, 12, false);
    assert_eq!(
        s.state.settings.filter.cutoff_hz, 4000.0,
        "an octave up from 2 kHz"
    );
    turn(&mut s, 2, -10, true);
    assert_eq!(
        s.state.settings.filter.cutoff_hz, 3775.0,
        "SHIFT: a tenth of a semitone a detent"
    );
    turn(&mut s, 3, 30, false);
    turn(&mut s, 3, 5, true);
    assert!(
        (s.state.settings.filter.resonance - 0.31).abs() < 1e-4,
        "{}",
        s.state.settings.filter.resonance
    );
    assert_eq!(
        s.ui.get::<KnobCard>(play_tab::QLINKS[3]).unwrap().value,
        "31"
    );
}

#[test]
fn dragging_a_layers_edge_sets_its_velocities() {
    let mut s = play(true);
    let r = s.ui.rect_of(play_tab::LAYERS).unwrap();
    let map = s.ui.get::<LayerMap>(play_tab::LAYERS).unwrap();
    let mid = map.bar_rect(r, 1).unwrap();
    let low = map.bar_rect(r, 0).unwrap();
    // Layer 2's left edge from 64 to about 48: it overlaps layer 1 (both
    // play there); layer 1 is untouched.
    let to_x = low.x + (mid.x - low.x) * 47 / 63;
    let a = drag(
        &mut s,
        (mid.x + 2, mid.y + mid.h / 2),
        (to_x, mid.y + mid.h / 2),
    )
    .unwrap();
    assert_eq!(s.tab.layer(), 1, "the touch picked layer 2");
    apply(&mut s, &a);
    let l = &s.state.settings.layers;
    assert!((46..=50).contains(&l[1].vel_lo), "{}", l[1].vel_lo);
    assert_eq!((l[0].vel_lo, l[0].vel_hi, l[1].vel_hi), (1, 63, 109));
    assert!(s
        .ui
        .get::<EnvelopeView>(play_tab::ENV)
        .unwrap()
        .caption
        .ends_with("over layer 2"));
    // The high edge can't cross the low one.
    lift(&mut s);
    let r = s.ui.rect_of(play_tab::LAYERS).unwrap();
    let hard =
        s.ui.get::<LayerMap>(play_tab::LAYERS)
            .unwrap()
            .bar_rect(r, 2)
            .unwrap();
    let a = drag(
        &mut s,
        (hard.right() - 2, hard.y + hard.h / 2),
        (r.x, hard.y + hard.h / 2),
    )
    .unwrap();
    apply(&mut s, &a);
    assert_eq!(
        (
            s.state.settings.layers[2].vel_lo,
            s.state.settings.layers[2].vel_hi
        ),
        (110, 110)
    );
    lift(&mut s);
}

#[test]
fn dragging_the_envelopes_points_and_the_filters_dot() {
    let mut s = play(true);
    let r = s.ui.rect_of(play_tab::ENV).unwrap();
    let env = s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap();
    let axis = env.axis_ms();
    let (dx, dy) = env.point_xy(r, EnvPoint::Decay);
    let (sx, sy) = env.point_xy(r, EnvPoint::Sustain);
    // Decay out by about half a second.
    let px_per_ms =
        (sx - dx) as f32 / (env.point(EnvPoint::Sustain).0 - env.point(EnvPoint::Decay).0);
    let a = drag(&mut s, (dx, dy), (dx + (500.0 * px_per_ms) as i32, dy)).unwrap();
    apply(&mut s, &a);
    let decay = s.state.settings.amp.decay_ms;
    assert!((decay - 810.0).abs() < 20.0, "{decay}");
    assert_eq!(
        s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap().axis_ms(),
        axis,
        "the axis holds still while dragged"
    );
    lift(&mut s);
    // Sustain down to about a quarter.
    let r = s.ui.rect_of(play_tab::ENV).unwrap();
    let (_, top) =
        s.ui.get::<EnvelopeView>(play_tab::ENV)
            .unwrap()
            .point_xy(r, EnvPoint::Attack);
    let floor = sy + (sy - top) * 60 / 40;
    let a = drag(&mut s, (sx, sy), (sx, floor - (floor - top) / 4)).unwrap();
    apply(&mut s, &a);
    assert!(
        (s.state.settings.amp.sustain - 0.25).abs() < 0.03,
        "{}",
        s.state.settings.amp.sustain
    );
    lift(&mut s);
    // The cutoff dot a decade down.
    let r = s.ui.rect_of(play_tab::FILTER).unwrap();
    let f = s.ui.get::<FilterCurve>(play_tab::FILTER).unwrap();
    let (fx, fy) = f.dot_xy(r);
    let decade = (r.w - 28) / 3;
    let a = drag(&mut s, (fx, fy), (fx - decade, fy)).unwrap();
    apply(&mut s, &a);
    let hz = s.state.settings.filter.cutoff_hz;
    assert!((hz / 240.0 - 1.0).abs() < 0.03, "{hz}");
    // Three drags, three undo steps.
    let before = s.project.pad(PAD).unwrap().clone();
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().filter.cutoff_hz, 2400.0);
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().amp.sustain, 0.6);
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().amp.decay_ms, 310.0);
    assert!(describe(s.project.pad(PAD).unwrap(), &before).starts_with("decay 310 ms -> "));
}

#[test]
fn applying_keeps_the_projects_regions_and_the_engine_plays_the_layers() {
    let (mut project, view) = layered_project();
    let state = PlayState::from_project(&project, PAD, |_| view.clone()).unwrap();
    let mut after = state.settings.clone();
    after.layers[0].region.start += 100;
    after.layers[0].vel_hi = 40;
    after.polyphony = 2;
    PlayAction::Pad {
        pad: PAD,
        after: Box::new(after),
        gesture: 1,
    }
    .apply(&mut project)
    .unwrap();
    let p = project.pad(PAD).unwrap();
    assert_eq!(
        p.layers[0].region, state.settings.layers[0].region,
        "the Wave tab's region stays"
    );
    assert_eq!((p.layers[0].vel_hi, p.polyphony), (40, 2));
    assert_eq!(
        describe(&state.settings, p),
        "polyphony 4 voices -> 2 voices, layer 1 velocities 1-63 -> 1-40"
    );

    // A soft hit plays layer 1 only, a hard one layer 3 only.
    for (velocity, layer) in [(30u8, 0u8), (120, 2)] {
        let mut e = Engine::new(RATE, 32);
        e.set_view(EngineView::build(&project, RATE));
        assert!(e.schedule(
            0,
            Event::NoteOn {
                pad: PAD as u8,
                velocity
            }
        ));
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        e.render(&mut l, &mut r);
        let playing: Vec<u8> = e.playheads().map(|(_, layer, _)| layer).collect();
        assert_eq!(playing, vec![layer], "velocity {velocity}");
    }
}

#[test]
fn the_envelope_and_filter_pictures_follow_the_engine() {
    use mpfree_sampler::model::{Ahdsr, Filter};
    let e = Ahdsr {
        attack_ms: 10.0,
        hold_ms: 5.0,
        decay_ms: 100.0,
        sustain: 0.5,
        release_ms: 200.0,
    };
    assert_eq!(env_level(&e, 5.0, 1000.0), 0.5);
    assert_eq!(env_level(&e, 12.0, 1000.0), 1.0);
    assert!(
        (env_level(&e, 115.0, 1000.0) - 0.5).abs() < 0.001,
        "decayed to sustain"
    );
    assert!(env_level(&e, 1200.0, 1000.0) < 0.5 * 0.001, "released");
    let f = Filter {
        kind: FilterKind::LowPass,
        poles: 2,
        cutoff_hz: 1000.0,
        resonance: 0.0,
        env_octaves: 0.0,
    };
    assert!(
        (filter_db(&f, 1000.0) - (-6.02)).abs() < 0.1,
        "{}",
        filter_db(&f, 1000.0)
    );
    assert!(filter_db(&f, 10_000.0) < -39.0);
    let f4 = Filter { poles: 4, ..f };
    assert!((filter_db(&f4, 10_000.0) - 2.0 * filter_db(&f, 10_000.0)).abs() < 0.01);
    let hp = Filter {
        kind: FilterKind::HighPass,
        ..f
    };
    assert!(filter_db(&hp, 100.0) < -39.0 && filter_db(&hp, 10_000.0) > -0.1);
}

#[test]
fn every_play_sample_screen_draws() {
    for mut s in play_samples(Fonts::embedded).unwrap() {
        let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
        s.ui.paint(&mut c);
        assert!(!s.about.is_empty(), "{}", s.name);
    }
}

/// The tab on a pad holding a second of A3 (220 Hz).
fn tone() -> PlaySample {
    let mut s = play(false);
    let data = (0..RATE as usize)
        .map(|i| (0.5 * (std::f64::consts::TAU * 220.0 * i as f64 / f64::from(RATE)).sin()) as f32)
        .collect();
    let source = Source::new("Tone", RATE, 1, data).unwrap();
    let mut project = Project::new();
    project.add_source(source.clone());
    project
        .set_pad(
            PAD,
            Some(Pad::new("Tone", Layer::new(Region::whole(&source)))),
            None,
        )
        .unwrap();
    let view = Arc::new(SourceView::analyse(&source));
    assert_eq!(view.root_note, Some(57));
    s.state = PlayState::from_project(&project, PAD, |_| view.clone()).unwrap();
    s.project = project;
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    s
}

#[test]
fn keys_make_the_pad_a_keygroup_rooted_at_the_note_it_is_heard_at() {
    let mut s = tone();
    let before = s.state.settings.clone();
    // Nothing to step while Keys are off.
    assert_eq!(step(&mut s, Row::LowKey, 1), None);
    step_apply(&mut s, Row::Keys, 1);
    assert_eq!(s.state.settings.keys, Some(Keys::all(57)), "A3, as heard");
    assert_eq!(
        s.state.settings.polyphony, KEYGROUP_POLYPHONY,
        "voices for chords"
    );
    for (row, value) in [
        (Row::Keys, "On"),
        (Row::LowKey, "C-1"),
        (Row::HighKey, "G9"),
        (Row::RootKey, "A3"),
    ] {
        assert_eq!(row_value(&s, row), value, "{row:?}");
    }
    // The zone narrows; the low key never passes the high one.
    for (row, delta, value) in [
        (Row::LowKey, 36, "C2"),
        (Row::HighKey, -60, "G4"),
        (Row::RootKey, 3, "C4"),
        (Row::LowKey, 100, "G4"),
        (Row::LowKey, -31, "C2"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&s, row), value, "{row:?} {delta:+}");
    }
    let bar = s.ui.get::<PadBar>(play_tab::BAR).unwrap();
    assert!(
        bar.subtitle.ends_with("keys C2–G4, root C4"),
        "{}",
        bar.subtitle
    );
    let what = describe(&before, &s.state.settings);
    assert!(
        what.contains("keys Off -> On, low key – -> C2, high key – -> G4, root key – -> C4"),
        "{what}"
    );
    // Off again.
    step_apply(&mut s, Row::Keys, -1);
    assert_eq!(s.state.settings.keys, None);
}

#[test]
fn the_kits_pads_play_notes_from_the_keygroups_octave_an_octave_a_step() {
    let mut s = tone();
    step_apply(&mut s, Row::Keys, 1);
    s.tab.lift();
    // The kit's rows are their own action, an undo step each.
    let a = step(&mut s, Row::PadsPlay, 1).unwrap();
    assert_eq!(
        a,
        PlayAction::PadsPlay(PadsPlay::Keys { from: 48 }),
        "C3: A3's octave"
    );
    apply(&mut s, &a);
    assert_eq!(s.project.pads_play(), PadsPlay::Keys { from: 48 });
    assert_eq!(
        (row_value(&s, Row::PadsPlay), row_value(&s, Row::PadsFrom)),
        ("Notes".into(), "C3".into())
    );
    let view = EngineView::build(&s.project, RATE);
    assert!(
        (0..16).all(|i| view.has_pad(i)),
        "every pad is a note the keygroup holds"
    );
    for (delta, value) in [(1, "C4"), (-9, "C-1"), (20, "C8")] {
        step_apply(&mut s, Row::PadsFrom, delta);
        assert_eq!(row_value(&s, Row::PadsFrom), value, "{delta:+}");
    }
    let bar = s.ui.get::<PadBar>(play_tab::BAR).unwrap();
    assert!(
        bar.subtitle.ends_with("pads play notes from C8"),
        "{}",
        bar.subtitle
    );
    // Past the top, nothing changes.
    assert_eq!(step(&mut s, Row::PadsFrom, 1), None);
    step_apply(&mut s, Row::PadsPlay, -1);
    assert_eq!(row_value(&s, Row::PadsPlay), "Their sounds");
    assert!(s.project.undo());
    assert_eq!(s.project.pads_play(), PadsPlay::Keys { from: 108 });
}

#[test]
fn sixteen_levels_spread_the_pad_over_the_kits_pads() {
    let mut s = tone();
    assert_eq!(row_value(&s, Row::LevelsBy), "–");
    // Pads play goes round: their sounds, notes, 16 Levels of this pad.
    step_apply(&mut s, Row::PadsPlay, -1);
    assert_eq!(
        s.project.pads_play(),
        PadsPlay::Levels {
            pad: PAD as u8,
            by: Levels::Velocity
        }
    );
    assert_eq!(
        (row_value(&s, Row::PadsPlay), row_value(&s, Row::LevelsBy)),
        ("16 Levels".into(), "Velocity".into())
    );
    assert_eq!(row_value(&s, Row::PadsFrom), "–");
    let view = EngineView::build(&s.project, RATE);
    assert!((0..16).all(|i| view.has_pad(i)), "every pad plays A5");
    for (delta, by) in [(1, "Tune"), (3, "Start"), (1, "Velocity"), (-2, "Decay")] {
        step_apply(&mut s, Row::LevelsBy, delta);
        assert_eq!(row_value(&s, Row::LevelsBy), by, "{delta:+}");
    }
    let bar = s.ui.get::<PadBar>(play_tab::BAR).unwrap();
    assert!(
        bar.subtitle.ends_with("pads play 16 Levels of A5 by decay"),
        "{}",
        bar.subtitle
    );
    // Tune at pad 16: seven semitones up from A3 is E4.
    let mut e = Engine::new(RATE, 8);
    step_apply(&mut s, Row::LevelsBy, -2);
    assert_eq!(row_value(&s, Row::LevelsBy), "Tune");
    e.set_view(EngineView::build(&s.project, RATE));
    e.apply(Event::NoteOn {
        pad: 15,
        velocity: 127,
    });
    let (mut l, mut r) = (vec![0.0; 12_000], vec![0.0; 12_000]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    let hz = mpfree_analysis::yin(&l[1000..9192], RATE, 4096, 40.0, 3000.0, 0.15)
        .unwrap()
        .0;
    assert!((hz - 329.63).abs() < 1.0, "{hz}");
    // Once more round: back to their sounds.
    step_apply(&mut s, Row::PadsPlay, 1);
    assert_eq!(s.project.pads_play(), PadsPlay::Pads);
    assert_eq!(row_value(&s, Row::LevelsBy), "–");
}
