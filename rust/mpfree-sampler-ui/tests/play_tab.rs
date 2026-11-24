//! The Play tab: what it shows of a pad, and how its arrows, the wheel, the
//! Q-Links and its drags change the pad in a real project, one undo step
//! each.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use std::sync::Arc;

use mpfree_sampler::model::{
    FilterKind, Keys, Layer, LayerSelect, Levels, Pad, PadsPlay, PressureTarget, Project, Region,
    SliceSet, Source, KEYGROUP_POLYPHONY,
};
use mpfree_sampler_ui::keys_widgets::KeyZones;
use mpfree_sampler_ui::play_tab::{self, describe, PlayAction, PlayState, Row, Section};
use mpfree_sampler_ui::play_widgets::{
    env_level, filter_db, EnvPoint, EnvelopeView, FilterCurve, LayerMap,
};
use mpfree_sampler_ui::sample::{self, layered_project, play_samples, PlaySample, PAD, RATE};
use mpfree_sampler_ui::source::SourceView;
use mpfree_sampler_ui::widgets::{KnobCard, PadBar, SectionChips, SettingList};
use mpfree_ui::wave::Waveform;
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

/// A tap on the chip of section `section`.
fn open(s: &mut PlaySample, section: Section) {
    let i = Section::ALL.iter().position(|x| *x == section).unwrap();
    let ev = UiEvent {
        widget: play_tab::SECTIONS.into(),
        kind: EventKind::Tab(i),
    };
    let st = s.state.clone();
    assert_eq!(s.tab.action(&mut s.ui, &ev, &st), None);
    assert_eq!(s.tab.section(), section);
}

/// Row `row`'s value as its section's list shows it (the section opened).
fn row_value(s: &mut PlaySample, row: Row) -> String {
    open(s, row.section());
    let list = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    assert_eq!(list.rows[row.place()].name, row.name_on(&s.state.settings));
    list.rows[row.place()].value.clone()
}

/// An arrow of row `row` tapped (its section opened first).
fn step(s: &mut PlaySample, row: Row, delta: i32) -> Option<PlayAction> {
    open(s, row.section());
    let index = row.place();
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
    let mut s = play(true);
    let bar = s.ui.get::<PadBar>(play_tab::BAR).unwrap();
    assert_eq!(bar.title, "Pad A5 · Break 92");
    assert_eq!(
        bar.subtitle,
        "3 layers · Loop · stacked · 4 voices · 4.73 s long"
    );
    // Edit pad, a page of the Pads tab: no switch, a way back to the pads.
    assert!(bar.tabs.is_empty(), "no Wave | Play switch: {:?}", bar.tabs);
    assert_eq!(bar.back.as_deref(), Some(play_tab::BACK));
    let map = s.ui.get::<LayerMap>(play_tab::LAYERS).unwrap();
    let ranges: Vec<(u8, u8)> = map.layers.iter().map(|l| (l.lo, l.hi)).collect();
    assert_eq!(ranges, vec![(1, 63), (64, 109), (110, 127)]);
    assert_eq!(map.layers[1].name, "Layer 2 · Break 92");
    assert_eq!(map.layers[0].sub, "velocity 1–63 · -9.0 dB · stacked");
    assert_eq!(map.last_velocity, Some(108));
    // The sections, each with what is in it now; Layers is open.
    let chips = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
    let chips: Vec<(&str, &str)> = chips
        .chips
        .iter()
        .map(|c| (c.name.as_str(), c.sub.as_str()))
        .collect();
    assert_eq!(
        chips,
        vec![
            ("Layers", "3 · stacked"),
            ("Amp", "A2 D310 S60%"),
            ("Filter", "LP 12 dB · 2.40 kHz"),
            ("Pitch", "+0 st"),
            ("Mod", "pressure → filter cutoff"),
            ("Movement", "Off"),
            ("Pad", "4 voices · group 1"),
            ("Slices", "not sliced"),
        ]
    );
    assert_eq!(s.tab.section(), Section::Layers);
    for row in Row::ALL {
        let n = Section::ALL
            .iter()
            .filter(|x| x.rows().contains(&row))
            .count();
        // What moves the filter has the rows no section has (F5).
        let moves = play_tab::MOVES.contains(&row) && n == 0;
        assert!(n == 1 || moves, "{row:?} is in one section");
    }
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
    assert_eq!(filter.title, "Filter · Low-pass");
    assert_eq!(
        filter.caption,
        "cutoff 2.40 kHz · res 35 % · drag across the graph"
    );
    let cards = |s: &PlaySample| -> Vec<(String, String, String, bool)> {
        play_tab::QLINKS
            .iter()
            .map(|q| {
                let c = s.ui.get::<KnobCard>(q).unwrap();
                (c.label.clone(), c.value.clone(), c.sub.clone(), c.enabled)
            })
            .collect()
    };
    let card = |l: &str, v: &str, u: &str| (l.to_string(), v.to_string(), u.to_string(), true);
    assert_eq!(
        cards(&s),
        vec![
            card("Q-Link 1 · Layer level", "-9.0", "dB"),
            card("Q-Link 2 · Layer pan", "Center", ""),
            card("Q-Link 3 · Layer tune", "+0", "st"),
            card("Q-Link 4 · Vel low", "1", ""),
        ]
    );
    for (row, value) in [
        (Row::Sound, "Break 92"),
        (Row::VelLow, "1"),
        (Row::VelHigh, "63"),
        (Row::LayerLevel, "-9.0 dB"),
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
        (Row::Filter, "Low-pass"),
        (Row::Slope, "12 dB"),
        (Row::Cutoff, "2.40 kHz"),
        (Row::FilterEnv, "+0.50 oct"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    // The Q-Links follow the section open.
    open(&mut s, Section::Amp);
    assert_eq!(
        cards(&s),
        vec![
            card("Q-Link 1 · Attack", "2", "ms"),
            card("Q-Link 2 · Decay", "310", "ms"),
            card("Q-Link 3 · Sustain", "60", "%"),
            card("Q-Link 4 · Release", "900", "ms"),
        ]
    );
    open(&mut s, Section::Filter);
    assert_eq!(
        cards(&s),
        vec![
            card("Q-Link 1 · Cutoff", "2.40", "kHz"),
            card("Q-Link 2 · Resonance", "35", "%"),
            card("Q-Link 3 · Env depth", "+0.50", "oct"),
            card("Q-Link 4 · Env decay", "0", "ms"),
        ]
    );
    assert_eq!(
        s.tab.qlinks_text(),
        "cutoff, resonance, env depth, env decay"
    );
    let env = s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap();
    assert_eq!(
        env.title, "Filter envelope",
        "the Filter section shows its envelope"
    );
}

#[test]
fn arrows_change_the_pad_and_a_run_of_taps_on_one_row_is_one_undo_step() {
    let mut s = play(false);
    assert_eq!(row_value(&mut s, Row::Filter), "Off");
    // Turning the filter on starts a low-pass where it is heard.
    let a = step(&mut s, Row::Filter, 1).unwrap();
    let g = gesture(&a);
    apply(&mut s, &a);
    assert_eq!(s.state.settings.filter.kind, FilterKind::LowPass);
    assert_eq!(s.state.settings.filter.cutoff_hz, 2000.0);
    let a = step(&mut s, Row::Filter, 1).unwrap();
    assert_eq!(gesture(&a), g, "a second tap on the row merges");
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Filter), "High-pass");
    assert_eq!(row_value(&mut s, Row::Slope), "12 dB");
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
        (Row::Vintage, 1, "On"),
        (Row::Vintage, -1, "Off"),
        (Row::Vintage, 1, "On"),
        (Row::VelFade, 1, "2"),
        (Row::VelFade, 4, "12"),
        (Row::VelFade, 20, "64"),
        (Row::VelFade, -20, "Off"),
        (Row::Sustain, -8, "60 %"),
        (Row::Release, 2, "20 ms"),
        (Row::FilterEnv, 3, "+0.75 oct"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert_eq!(s.state.settings.select, LayerSelect::Random);
    assert!(s.state.settings.vintage, "Vintage reaches the pad");
    // Grain targets show only for a granular pad.
    let targets: Vec<String> = (0..4)
        .map(|_| {
            step_apply(&mut s, Row::Pressure, 1);
            row_value(&mut s, Row::Pressure)
        })
        .collect();
    assert_eq!(targets, vec!["Level", "Pitch", "Filter cutoff", "Off"]);
    step_apply(&mut s, Row::Pressure, 1);
    assert_eq!(
        (
            s.state.settings.pressure.target,
            row_value(&mut s, Row::Depth)
        ),
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
    let (tune, attack) = (Row::Tune.place(), Row::Attack.place());
    let after = |a: Option<PlayAction>| match a {
        Some(PlayAction::Pad { after, .. }) => *after,
        a => panic!("{a:?}"),
    };
    let ev = |index, delta| UiEvent {
        widget: play_tab::LIST.into(),
        kind: EventKind::Step { index, delta },
    };
    tab.open_section(&mut ui, &state, Section::Pitch);
    assert_eq!(
        after(tab.action(&mut ui, &ev(tune, 1), &state)).tune_cents,
        200
    );
    assert_eq!(
        after(tab.action(&mut ui, &ev(tune, -1), &state)).tune_cents,
        100
    );
    tab.open_section(&mut ui, &state, Section::Amp);
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
    // On Amp, a tap on Q-Link 2's card picks Decay for the wheel.
    open(&mut s, Section::Amp);
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
    // Sustain and release turn smoothly.
    turn(&mut s, 2, -15, false);
    turn(&mut s, 3, 24, false);
    assert_eq!(
        (
            s.state.settings.amp.sustain,
            s.state.settings.amp.release_ms
        ),
        (0.85, 40.0),
        "1 % a detent; 5 ms doubled three times"
    );
    // On Filter, cutoff and resonance wait for a filter.
    open(&mut s, Section::Filter);
    assert_eq!(turn(&mut s, 0, 1, false), None);
    assert!(!s.ui.get::<KnobCard>(play_tab::QLINKS[0]).unwrap().enabled);
    step_apply(&mut s, Row::Filter, 1);
    turn(&mut s, 0, 12, false);
    assert_eq!(
        s.state.settings.filter.cutoff_hz, 4000.0,
        "an octave up from 2 kHz"
    );
    turn(&mut s, 0, -10, true);
    assert_eq!(
        s.state.settings.filter.cutoff_hz, 3775.0,
        "SHIFT: a tenth of a semitone a detent"
    );
    turn(&mut s, 1, 30, false);
    turn(&mut s, 1, 5, true);
    assert!(
        (s.state.settings.filter.resonance - 0.31).abs() < 1e-4,
        "{}",
        s.state.settings.filter.resonance
    );
    assert_eq!(
        s.ui.get::<KnobCard>(play_tab::QLINKS[1]).unwrap().value,
        "31"
    );
    // The filter envelope: its depth and decay.
    turn(&mut s, 2, 40, false);
    turn(&mut s, 3, 1, false);
    turn(&mut s, 3, 16, false);
    let f = &s.state.settings;
    assert_eq!((f.filter.env_octaves, f.filter_env.decay_ms), (2.0, 4.0));
    // A choice on a Q-Link steps as its arrows do: Pad's voices.
    open(&mut s, Section::Pad);
    let voices = s.state.settings.polyphony;
    turn(&mut s, 0, 1, false).unwrap();
    assert!(s.state.settings.polyphony > voices);
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
    // Each section shows its own picture: Amp the envelope, Filter the
    // filter beside its envelope.
    assert_eq!(s.ui.rect_of(play_tab::ENV), None);
    open(&mut s, Section::Amp);
    assert_eq!(s.ui.rect_of(play_tab::LAYERS), None);
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
    assert_eq!(s.ui.rect_of(play_tab::FILTER), None);
    open(&mut s, Section::Filter);
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
        drive_db: 0.0,
    };
    // The Cutoff is the -3 dB point: a Butterworth at Resonance 0.
    assert!(
        (filter_db(&f, 1000.0) - (-3.01)).abs() < 0.1,
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
    assert_eq!(step(&mut s, Row::KeyFade, 1), None);
    assert_eq!(row_value(&mut s, Row::KeyFade), "–");
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
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    // The zone narrows; the low key never passes the high one.
    for (row, delta, value) in [
        (Row::LowKey, 36, "C2"),
        (Row::HighKey, -60, "G4"),
        (Row::RootKey, 3, "C4"),
        (Row::LowKey, 100, "G4"),
        (Row::LowKey, -31, "C2"),
        (Row::KeyFade, 1, "1 note"),
        (Row::KeyFade, 2, "3 notes"),
        (Row::KeyFade, 40, "24 notes"),
        (Row::KeyFade, -21, "3 notes"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    let bar = s.ui.get::<PadBar>(play_tab::BAR).unwrap();
    assert!(
        bar.subtitle
            .ends_with("keys C2–G4, root C4 · tuning: song's"),
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
        (
            row_value(&mut s, Row::PadsPlay),
            row_value(&mut s, Row::PadsFrom)
        ),
        ("Notes".into(), "C3".into())
    );
    let view = EngineView::build(&s.project, RATE);
    assert!(
        (0..16).all(|i| view.has_pad(i)),
        "every pad is a note the keygroup holds"
    );
    for (delta, value) in [(1, "C4"), (-9, "C-1"), (20, "C8")] {
        step_apply(&mut s, Row::PadsFrom, delta);
        assert_eq!(row_value(&mut s, Row::PadsFrom), value, "{delta:+}");
    }
    let bar = s.ui.get::<PadBar>(play_tab::BAR).unwrap();
    assert!(
        bar.subtitle
            .ends_with("pads play notes from C8 · tuning: song's"),
        "{}",
        bar.subtitle
    );
    // Past the top, nothing changes.
    assert_eq!(step(&mut s, Row::PadsFrom, 1), None);
    step_apply(&mut s, Row::PadsPlay, -1);
    assert_eq!(row_value(&mut s, Row::PadsPlay), "Their sounds");
    assert!(s.project.undo());
    assert_eq!(s.project.pads_play(), PadsPlay::Keys { from: 108 });
}

/// Tunings step T6 (layout spec 4e, board 4): the kit's Tuning under The
/// kit's pads names the kit's tuning as the app does. A step asks the app,
/// which keeps it with the project's instruments, so the kit doesn't
/// change; a keygroup's line says which it plays, a plain pad's doesn't
/// (hits keep their pitch).
#[test]
fn the_kits_tuning_asks_the_app_and_a_keygroups_line_says_it() {
    use mpfree_sampler_ui::play_tab::KitTuning;
    let mut s = tone();
    assert_eq!(row_value(&mut s, Row::KitTuning), "Song's: 12-TET");
    let before = s.project.pads_play();
    for d in [1, -2] {
        let a = step(&mut s, Row::KitTuning, d);
        assert_eq!(a, Some(PlayAction::KitTuning(d)));
        assert_eq!(a.unwrap().change(), None, "not the kit's to keep");
    }
    assert_eq!(s.project.pads_play(), before);
    let line = |s: &PlaySample| s.ui.get::<PadBar>(play_tab::BAR).unwrap().subtitle.clone();
    assert!(!line(&s).contains("tuning"), "{}", line(&s));
    // A keygroup's line says it, as the app names it.
    step_apply(&mut s, Row::Keys, 1);
    assert!(line(&s).ends_with("tuning: song's"), "{}", line(&s));
    s.state.kit_tuning = KitTuning {
        value: "Meantone 1/4 comma on C · 415 Hz".into(),
        line: "tuning: own".into(),
    };
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert_eq!(
        row_value(&mut s, Row::KitTuning),
        "Meantone 1/4 comma on C · 415 Hz"
    );
    assert!(line(&s).ends_with("tuning: own"), "{}", line(&s));
    // Under The kit's pads, with what follows it at the section's foot.
    let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    let panel = l.panel.as_ref().expect("the Pad section's panel");
    let kits = panel
        .sections
        .iter()
        .find(|p| p.title == "The kit's pads")
        .expect("The kit's pads");
    assert!(kits
        .lines
        .iter()
        .flatten()
        .any(|it| it.row == Row::KitTuning.place()));
    assert_eq!(kits.foot, play_tab::TUNING_FOOT);
}

#[test]
fn sixteen_levels_spread_the_pad_over_the_kits_pads() {
    let mut s = tone();
    assert_eq!(row_value(&mut s, Row::LevelsBy), "–");
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
        (
            row_value(&mut s, Row::PadsPlay),
            row_value(&mut s, Row::LevelsBy)
        ),
        ("16 Levels".into(), "Velocity".into())
    );
    assert_eq!(row_value(&mut s, Row::PadsFrom), "–");
    let view = EngineView::build(&s.project, RATE);
    assert!((0..16).all(|i| view.has_pad(i)), "every pad plays A5");
    for (delta, by) in [(1, "Tune"), (3, "Start"), (1, "Velocity"), (-2, "Decay")] {
        step_apply(&mut s, Row::LevelsBy, delta);
        assert_eq!(row_value(&mut s, Row::LevelsBy), by, "{delta:+}");
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
    assert_eq!(row_value(&mut s, Row::LevelsBy), "Tune");
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
    assert_eq!(row_value(&mut s, Row::LevelsBy), "–");
}

#[test]
fn the_layer_map_cuts_each_layer_along_the_velocity_fade() {
    use mpfree_sampler_ui::play_widgets::LayerMap;
    let mut s = play(true);
    assert_eq!(s.ui.get::<LayerMap>(play_tab::LAYERS).unwrap().fade, 0);
    step_apply(&mut s, Row::VelFade, 4);
    assert_eq!(s.state.settings.vel_fade, 8);
    assert_eq!(s.ui.get::<LayerMap>(play_tab::LAYERS).unwrap().fade, 8);
    // The middle layer (64-109) is cut at both ends, the lowest only at its top.
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    let r = s.ui.rect_of(play_tab::LAYERS).unwrap();
    let m = s.ui.get::<LayerMap>(play_tab::LAYERS).unwrap();
    let b = m.bar_rect(r, 1).unwrap();
    let mid = c.pixel(b.x + b.w / 2, b.y + 3);
    assert_ne!(c.pixel(b.x + 6, b.y + 3), mid, "cut at its low edge");
    assert_ne!(c.pixel(b.right() - 3, b.y + 3), mid, "cut at its high edge");
    assert_eq!(c.pixel(b.x + 6, b.bottom() - 3), mid, "full at the bottom");
}

/// The tab's keyboard and where it is.
fn keyboard(s: &PlaySample) -> (KeyZones, mpfree_ui::Rect) {
    let z = s.ui.get::<KeyZones>(play_tab::KEYS).unwrap().clone();
    (z, s.ui.rect_of(play_tab::KEYS).unwrap())
}

/// A finger down and up at (x, y): the actions it asks for.
fn tap(s: &mut PlaySample, x: i32, y: i32) -> Vec<PlayAction> {
    let mut out = Vec::new();
    for phase in [Touch::Down, Touch::Up] {
        if let Some(ev) = s.ui.touch(1, phase, x, y) {
            let st = s.state.clone();
            out.extend(s.tab.action(&mut s.ui, &ev, &st));
        }
    }
    s.tab.lift();
    out
}

#[test]
fn a_keygroup_shows_the_kits_keyboard_to_drag_its_zone_pick_a_pad_and_play_notes() {
    let mut s = tone();
    assert!(!s.tab.keys_shown());
    assert_eq!(s.tab.hint(), play_tab::HINT);
    // Keys on: the keyboard spans the top of the tab, left of the list.
    step_apply(&mut s, Row::Keys, 1);
    s.tab.lift();
    assert!(s.tab.keys_shown());
    assert_eq!(s.tab.hint(), play_tab::KEYS_HINT);
    let (_, r) = keyboard(&s);
    assert!(r.w > 700 && r.h >= 190, "{r:?}");
    // Narrow it to C2–G4 with the rows, and add a second keygroup on the
    // next pad, C5 to the top.
    step_apply(&mut s, Row::LowKey, 36);
    s.tab.lift();
    step_apply(&mut s, Row::HighKey, -60);
    s.tab.lift();
    let mut other = s.project.pad(PAD).unwrap().clone();
    other.keys = Some(Keys {
        lo: 72,
        hi: 127,
        root: 72,
    });
    s.project.set_pad(PAD + 1, Some(other), None).unwrap();
    apply(&mut s, &PlayAction::Pads);
    let (z, r) = keyboard(&s);
    assert_eq!(z.zones.len(), 2);
    assert_eq!(z.picked, Some(PAD));
    assert_eq!(z.right, "2 keygroups");
    assert!(z.caption.starts_with("A5: C2–G4, root A3"), "{}", z.caption);
    assert!(z.first <= 36 && z.last >= 79, "{}–{}", z.first, z.last);
    let picked = z.zones.iter().position(|k| k.pad == PAD).unwrap();
    let zr = z.zone_rect(r, picked).unwrap();
    // Its right edge dragged to B4.
    let b4 = z.key_point(r, 71).unwrap();
    let a = drag(
        &mut s,
        (zr.right(), zr.y + zr.h / 2),
        (b4.0, zr.y + zr.h / 2),
    )
    .unwrap();
    let first = gesture(&a);
    apply(&mut s, &a);
    lift(&mut s);
    assert_eq!(
        s.state.settings.keys,
        Some(Keys {
            lo: 36,
            hi: 71,
            root: 57
        })
    );
    // Its root dragged to C4: an undo step of its own.
    let (z, r) = keyboard(&s);
    let zr = z.zone_rect(r, picked).unwrap();
    let rx = z.root_x(r, s.state.settings.keys.unwrap());
    let c4 = z.key_point(r, 60).unwrap();
    let a = drag(&mut s, (rx, zr.y + zr.h / 2), (c4.0, zr.y + zr.h / 2)).unwrap();
    assert_ne!(gesture(&a), first);
    apply(&mut s, &a);
    lift(&mut s);
    assert_eq!(s.state.settings.keys.unwrap().root, 60);
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().keys.unwrap().root, 57);
    // A tap on the other zone picks its pad.
    let (z, r) = keyboard(&s);
    let i = z.zones.iter().position(|k| k.pad == PAD + 1).unwrap();
    let zr = z.zone_rect(r, i).unwrap();
    assert_eq!(
        tap(&mut s, zr.x + zr.w / 2, zr.y + zr.h / 2),
        [PlayAction::Pick(PAD + 1)]
    );
    // A key pressed plays the note, and lets it go.
    let (z, r) = keyboard(&s);
    let e4 = z.key_point(r, 64).unwrap();
    assert_eq!(
        tap(&mut s, e4.0, e4.1),
        [
            PlayAction::Note {
                note: 64,
                down: true
            },
            PlayAction::Note {
                note: 64,
                down: false
            }
        ]
    );
    let f_sharp = z.key_point(r, 66).unwrap();
    assert_eq!(
        tap(&mut s, f_sharp.0, f_sharp.1)[0],
        PlayAction::Note {
            note: 66,
            down: true
        }
    );
    // Keys off, and the pads play their sounds: the keyboard goes, and
    // Pitch shows the sound under its envelope.
    step_apply(&mut s, Row::Keys, -1);
    assert!(!s.tab.keys_shown());
    assert_eq!(s.ui.rect_of(play_tab::KEYS), None);
    assert!(s.ui.rect_of(play_tab::ENV).is_some());
}

#[test]
fn the_keyboard_shows_while_the_pads_play_notes_and_marks_their_notes() {
    let mut s = tone();
    step_apply(&mut s, Row::Keys, 1);
    s.tab.lift();
    let a = step(&mut s, Row::PadsPlay, 1).unwrap();
    apply(&mut s, &a);
    // The pad's keys off: the pads still play notes, so the keyboard stays.
    step_apply(&mut s, Row::Keys, -1);
    assert!(s.tab.keys_shown());
    let (z, _) = keyboard(&s);
    assert_eq!(z.pads_from, Some(48));
    assert_eq!(z.picked, None);
    assert!(z.zones.is_empty());
    assert!(z.caption.contains("is not a keygroup"), "{}", z.caption);
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
}

#[test]
fn key_fade_overlaps_the_kits_zones_and_the_keyboard_draws_their_curves() {
    let mut s = play_samples(Fonts::embedded)
        .unwrap()
        .into_iter()
        .find(|s| s.name == "play-keys")
        .unwrap();
    assert_eq!(row_value(&mut s, Row::KeyFade), "Off");
    // One step for the whole kit: each zone fades over 4 notes and the
    // ones that met overlap by 4.
    let a = step(&mut s, Row::KeyFade, 4).unwrap();
    assert_eq!(a, PlayAction::KeyFade(4));
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::KeyFade), "4 notes");
    let (z, r) = keyboard(&s);
    assert!(z.zones.iter().all(|z| z.fade == 4));
    let (a1, a2) = (z.zones[0].keys, z.zones[1].keys);
    assert_eq!(a1.hi + 1 - a2.lo, 4, "{a1:?} {a2:?}");
    // The bar is cut along the curve where it fades out, full elsewhere.
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    let i = z.zones.iter().position(|q| q.pad == 1).unwrap();
    let zr = z.zone_rect(r, i).unwrap();
    let mid = c.pixel(zr.x + zr.w / 2, zr.y + 2);
    assert_ne!(
        c.pixel(zr.right() - 3, zr.y + 2),
        mid,
        "cut at its top edge"
    );
    // Back to edge to edge, and a kit with no keygroup has nothing to fade.
    let a = step(&mut s, Row::KeyFade, -4).unwrap();
    apply(&mut s, &a);
    let (z, _) = keyboard(&s);
    assert_eq!(z.zones[0].keys.hi + 1, z.zones[1].keys.lo);
    let mut plain = play(false);
    assert_eq!(row_value(&mut plain, Row::KeyFade), "–");
    assert_eq!(step(&mut plain, Row::KeyFade, 1), None);
}

#[test]
fn the_scene_strip_stores_fades_and_returns() {
    use mpfree_sampler::scene::Scene;
    use mpfree_sampler_ui::panel::Look;
    use mpfree_sampler_ui::play_tab::SceneAction;
    let mut s = play(false);
    let ev = |widget: &str, kind: EventKind| UiEvent {
        widget: widget.into(),
        kind,
    };
    let act = |s: &mut PlaySample, e: UiEvent| {
        let st = s.state.clone();
        s.tab.action(&mut s.ui, &e, &st)
    };
    // The scenes' panel: three buttons, then the crossfader, which says
    // where it is (or, faint, what to store first).
    let strip = |s: &PlaySample| {
        let l = s.ui.get::<SettingList>(play_tab::SCENES).unwrap();
        assert_eq!(l.rows.len(), play_tab::SCENE_ROWS);
        let labels: Vec<String> = l.controls[..3]
            .iter()
            .map(|c| match &c.look {
                Look::Button(t) => t.clone(),
                other => panic!("a button, not {other:?}"),
            })
            .collect();
        let enabled: Vec<bool> = l.controls.iter().map(|c| !c.faint).collect();
        let fader = match l.controls[3].look {
            Look::Slider { pos } => pos,
            ref other => panic!("a slider, not {other:?}"),
        };
        assert_eq!(fader, s.tab.fade());
        let text = if l.controls[3].faint {
            l.panel.as_ref().unwrap().sections[0].note.clone()
        } else {
            l.rows[3].value.clone()
        };
        (labels, enabled[..3].to_vec(), text)
    };
    let (labels, enabled, text) = strip(&s);
    assert_eq!(labels, ["Store A", "Store B", "Return to A"]);
    assert_eq!(enabled, [true, true, false], "nothing to return to");
    assert_eq!(text, "store A first");
    // The fader does nothing until A and B are stored.
    assert_eq!(
        act(&mut s, ev(play_tab::SCENES, EventKind::Value(0.5))),
        None
    );
    assert_eq!(s.tab.fade(), 0.0);
    let store = |s: &mut PlaySample, k: usize| {
        assert_eq!(
            act(s, ev(play_tab::SCENES, EventKind::Tab(k))),
            Some(PlayAction::Scene(SceneAction::Store(k)))
        );
        // The app stores it in the kit.
        s.project.scenes[k] = Some(Scene::of(&s.project));
        let view = s.state.sounds[0].view.clone();
        s.state = PlayState::from_project(&s.project, PAD, |_| view.clone()).unwrap();
        let st = s.state.clone();
        s.tab.update(&mut s.ui, &st);
    };
    store(&mut s, 0);
    assert_eq!(strip(&s).2, "store B first");
    store(&mut s, 1);
    let (labels, enabled, text) = strip(&s);
    assert_eq!(labels, ["Store A again", "Store B again", "Return to A"]);
    assert_eq!(enabled, [true, true, true]);
    assert_eq!(text, "A");
    assert_eq!(
        act(&mut s, ev(play_tab::SCENES, EventKind::Value(0.4))),
        Some(PlayAction::Scene(SceneAction::Fade(0.4)))
    );
    assert_eq!(strip(&s).2, "40 % to B");
    assert_eq!(
        act(&mut s, ev(play_tab::SCENES, EventKind::Value(0.4))),
        None,
        "no move"
    );
    act(&mut s, ev(play_tab::SCENES, EventKind::Value(1.0)));
    assert_eq!(strip(&s).2, "B");
    // Return: the fader back at A.
    assert_eq!(
        act(&mut s, ev(play_tab::SCENES, EventKind::Tab(2))),
        Some(PlayAction::Scene(SceneAction::Return))
    );
    assert_eq!((s.tab.fade(), strip(&s).2.as_str()), (0.0, "A"));
    // It changes nothing in the kit by itself.
    let before = s.project.pad(PAD).cloned();
    apply(&mut s, &PlayAction::Scene(SceneAction::Return));
    assert_eq!(s.project.pad(PAD).cloned(), before);
}

#[test]
fn stops_also_plays_and_latch() {
    let mut s = play(false);
    for row in [Row::Chokes, Row::Links, Row::Latch] {
        assert_eq!(row_value(&mut s, row), "Off", "{row:?}");
    }
    // Arrows and the wheel: a step up adds the pad after the highest, never
    // the pad shown (A5); a step down takes the highest out.
    for (delta, value) in [
        (1, "A1"),
        (3, "A1 A2 A3 A4"),
        (1, "A1 A2 A3 A4 A6"),
        (-2, "A1 A2 A3"),
    ] {
        step_apply(&mut s, Row::Links, delta);
        assert_eq!(row_value(&mut s, Row::Links), value, "{delta:+}");
    }
    assert_eq!(s.state.settings.links, [0, 1, 2]);
    step_apply(&mut s, Row::Links, -3);
    assert_eq!(row_value(&mut s, Row::Links), "Off");
    assert_eq!(step(&mut s, Row::Links, -1), None, "nothing to take out");

    // Latch: any step turns it over.
    step_apply(&mut s, Row::Latch, 1);
    assert_eq!(row_value(&mut s, Row::Latch), "On");
    assert!(s.state.settings.latch);
    step_apply(&mut s, Row::Latch, 1);
    assert_eq!(row_value(&mut s, Row::Latch), "Off");

    // With Stops or Also plays picked, a hit adds a pad or takes it out,
    // an undo step each; the pad shown changes nothing.
    let pick = |s: &mut PlaySample, row: Row| {
        s.ui.get_mut::<SettingList>(play_tab::LIST)
            .unwrap()
            .selected = Row::ALL.iter().position(|r| *r == row).unwrap();
    };
    pick(&mut s, Row::Level);
    assert!(!s.tab.takes_pads(&s.ui));
    let st = s.state.clone();
    assert_eq!(
        s.tab.pad_hit(&s.ui, &st, 3),
        None,
        "Level doesn't take pads"
    );
    pick(&mut s, Row::Chokes);
    assert!(s.tab.takes_pads(&s.ui));
    let before = s.state.settings.clone();
    let mut gestures = Vec::new();
    for pad in [3, 9, PAD, 3] {
        let st = s.state.clone();
        let a = s.tab.pad_hit(&s.ui, &st, pad);
        if pad == PAD {
            assert_eq!(a, None, "a pad never stops itself");
            continue;
        }
        let a = a.unwrap();
        gestures.push(gesture(&a));
        apply(&mut s, &a);
    }
    gestures.dedup();
    assert_eq!(gestures.len(), 3, "each hit is its own undo step");
    assert_eq!(row_value(&mut s, Row::Chokes), "A10");
    assert_eq!(describe(&before, &s.state.settings), "stops Off -> A10");
    pick(&mut s, Row::Links);
    let st = s.state.clone();
    let a = s.tab.pad_hit(&s.ui, &st, 0).unwrap();
    apply(&mut s, &a);
    assert_eq!(s.state.settings.links, [0]);
    assert!(s.project.undo());
    assert!(s.project.pad(PAD).unwrap().links.is_empty());
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().chokes, [3, 9]);
}

/// A tap on Add layer (0) or Remove layer (1), on the Layers section's
/// panel.
fn layer_button(s: &mut PlaySample, i: usize) -> Option<PlayAction> {
    if s.tab.section() != Section::Layers {
        open(s, Section::Layers);
    }
    let row = [Row::AddLayer, Row::RemoveLayer][i];
    let ev = UiEvent {
        widget: play_tab::LIST.into(),
        kind: EventKind::Tab(row.place()),
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

fn ranges(s: &PlaySample) -> Vec<(u8, u8)> {
    s.state
        .settings
        .layers
        .iter()
        .map(|l| (l.vel_lo, l.vel_hi))
        .collect()
}

#[test]
fn layers_are_added_and_removed_and_each_has_its_velocities_level_pan_and_tune() {
    let mut s = play(false);
    open(&mut s, Section::Layers);
    let buttons = |s: &PlaySample| {
        let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
        [Row::AddLayer, Row::RemoveLayer]
            .map(|r| !l.controls[r.place()].faint)
            .to_vec()
    };
    assert_eq!(buttons(&s), vec![true, false], "a pad keeps one layer");
    // Add layer: a copy of the layer picked, taking the upper half of its
    // velocities, picked once it is there.
    let a = layer_button(&mut s, 0).unwrap();
    assert_eq!(a, PlayAction::AddLayer { pad: PAD, after: 0 });
    apply(&mut s, &a);
    assert_eq!(ranges(&s), vec![(1, 63), (64, 127)]);
    assert_eq!(s.tab.layer(), 1);
    assert_eq!(buttons(&s), vec![true, true]);
    let l = &s.state.settings.layers;
    assert_eq!(l[1].region, l[0].region, "the same sound");
    // Its own level, pan and tune, on the Layers section's rows.
    let before = s.state.settings.clone();
    step_apply(&mut s, Row::LayerLevel, -4);
    step_apply(&mut s, Row::LayerPan, -2);
    step_apply(&mut s, Row::LayerTune, 1);
    let l = &s.state.settings.layers;
    assert_eq!(
        (l[1].level_db, l[1].pan, l[1].tune_cents),
        (-2.0, -0.1, 100)
    );
    assert_eq!((l[0].level_db, l[0].pan, l[0].tune_cents), (0.0, 0.0, 0));
    assert_eq!(row_value(&mut s, Row::LayerPan), "L 10");
    let list = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    assert_eq!(list.title, "Layer 2 · Break 92");
    assert_eq!(
        s.ui.get::<LayerMap>(play_tab::LAYERS).unwrap().layers[1].sub,
        "velocity 64–127 · -2.0 dB · L 10 · +1 st · stacked"
    );
    assert_eq!(
        describe(&before, &s.state.settings),
        "layer 2 level +0.0 dB -> -2.0 dB, layer 2 pan Center -> L 10, layer 2 tune +0 st -> +1 st"
    );
    // Its velocities, low edge up to its high one and no further.
    step_apply(&mut s, Row::VelLow, 8);
    assert_eq!(ranges(&s), vec![(1, 63), (72, 127)]);
    let a = step(&mut s, Row::VelHigh, -100).unwrap();
    apply(&mut s, &a);
    assert_eq!(ranges(&s), vec![(1, 63), (72, 72)]);
    // A second Add on a layer of one velocity: both play it.
    let a = layer_button(&mut s, 0).unwrap();
    apply(&mut s, &a);
    assert_eq!(ranges(&s), vec![(1, 63), (72, 72), (72, 72)]);
    assert_eq!(s.tab.layer(), 2);
    // Remove layer takes the one picked out and picks the one below.
    let a = layer_button(&mut s, 1).unwrap();
    assert_eq!(a, PlayAction::RemoveLayer { pad: PAD, layer: 2 });
    apply(&mut s, &a);
    assert_eq!(ranges(&s), vec![(1, 63), (72, 72)]);
    assert_eq!(s.tab.layer(), 1);
    // Each add and remove is one undo step.
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().layers.len(), 3);
}

#[test]
fn a_layers_sound_steps_through_the_kits_sounds() {
    let mut s = play(false);
    // With one sound in the kit there is nothing to step to.
    let a = layer_button(&mut s, 0).unwrap();
    apply(&mut s, &a);
    assert_eq!(step(&mut s, Row::Sound, 1), None);
    // A second sound: the layer picked plays it whole, keeping its mode.
    let data = vec![0.25f32; RATE as usize / 2];
    let tone = Source::new("Tone", RATE, 1, data).unwrap();
    let id = s.project.add_source(tone);
    apply(&mut s, &PlayAction::Pads);
    let a = step(&mut s, Row::Sound, 1).unwrap();
    assert_eq!(
        a,
        PlayAction::LayerSound {
            pad: PAD,
            layer: 1,
            source: id
        }
    );
    a.apply(&mut s.project).unwrap();
    let pad = s.project.pad(PAD).unwrap();
    let (first, second) = (&pad.layers[0].region, &pad.layers[1].region);
    assert_eq!(
        (second.source, second.start, second.end),
        (id, 0, RATE as u64 / 2)
    );
    assert_eq!(second.mode, first.mode);
    assert_ne!(first.source, id, "the other layer keeps its sound");
    assert_eq!(pad.name, "Break 92", "the pad keeps its name");
}

#[test]
fn the_filter_section_steps_and_drags_the_filter_envelope() {
    let mut s = play(true);
    open(&mut s, Section::Filter);
    for (row, delta) in [
        (Row::EnvAttack, 4),
        (Row::EnvDecay, 9),
        (Row::EnvSustain, -10),
        (Row::EnvRelease, 7),
    ] {
        step_apply(&mut s, row, delta);
        s.tab.lift();
    }
    let e = s.state.settings.filter_env;
    assert_eq!(
        (e.attack_ms, e.decay_ms, e.sustain, e.release_ms),
        (10.0, 500.0, 0.5, 1000.0)
    );
    assert_eq!(s.state.settings.amp.attack_ms, 2.0, "the amp's untouched");
    assert_eq!(row_value(&mut s, Row::EnvDecay), "500 ms");
    // The picture is the filter envelope's: dragging its points sets it.
    let env = s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap();
    assert_eq!(env.env, e);
    assert_eq!(
        env.caption,
        "A 10 ms · H 0 ms · D 500 ms · S 50 % · R 1.00 s · over layer 1"
    );
    let r = s.ui.rect_of(play_tab::ENV).unwrap();
    let (x, y) = env.point_xy(r, EnvPoint::Sustain);
    let a = drag(&mut s, (x, y), (x, y - 40)).unwrap();
    lift(&mut s);
    apply(&mut s, &a);
    assert!(s.state.settings.filter_env.sustain > 0.5);
    assert_eq!(s.state.settings.amp.sustain, 0.6, "the amp's untouched");
    // Back on Amp the picture is the amp's again.
    open(&mut s, Section::Amp);
    assert_eq!(
        s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap().env,
        s.state.settings.amp
    );
}

#[test]
fn each_section_shows_its_picture_across_the_page_and_its_controls_on_the_panel() {
    use mpfree_sampler_ui::panel::Look;
    let mut s = play(true);
    let scenes = s.ui.rect_of(play_tab::SCENES).unwrap();
    for (section, view) in [
        (Section::Layers, play_tab::LAYERS),
        (Section::Amp, play_tab::ENV),
        (Section::Filter, play_tab::FILTER),
        (Section::Pitch, play_tab::ENV),
        (Section::Mod, play_tab::LAYERS),
        (Section::Movement, play_tab::MOTION),
        (Section::Pad, play_tab::LAYERS),
    ] {
        if s.tab.section() != section {
            open(&mut s, section);
        }
        let v = s.ui.rect_of(view).expect("its picture");
        let r = s.ui.rect_of(play_tab::LIST).unwrap();
        // The picture spans the panel and the scenes, which sit side by
        // side under it.
        assert_eq!(v.x, r.x, "{section:?}");
        if section != Section::Filter {
            assert_eq!(v.right(), scenes.right(), "{section:?}");
        }
        assert!(v.bottom() < r.y);
        assert_eq!((r.y, r.h), (scenes.y, scenes.h));
        assert!(r.right() < scenes.x);
        let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
        assert_eq!(l.rows.len(), section.rows().len());
        assert_eq!(l.controls.len(), l.rows.len());
        let placed: Vec<_> = (0..l.rows.len())
            .filter_map(|k| {
                let row = section.rows()[k];
                // The type's own knobs show only on that type: Drive on the
                // Ladder (F5).
                let Some(c) = l.control_rect(r, k) else {
                    assert_eq!(row, Row::Drive, "{row:?} on the panel");
                    return None;
                };
                assert!(
                    c.x >= r.x && c.right() <= r.right() && c.y >= r.y && c.bottom() <= r.bottom(),
                    "{row:?} inside the panel"
                );
                Some(c)
            })
            .collect();
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                let apart =
                    a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
                assert!(apart, "{section:?}: {a:?} and {b:?} overlap");
            }
        }
    }
    assert_eq!(s.ui.rect_of(play_tab::ENV), None, "Pad shows the layers");
    // Amp: Level and Pan, then the envelope's five knobs.
    open(&mut s, Section::Amp);
    let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    let p = l.panel.as_ref().unwrap();
    let titles: Vec<&str> = p.sections.iter().map(|x| x.title.as_str()).collect();
    assert_eq!(titles, ["Level", "Envelope"]);
    assert!(l
        .controls
        .iter()
        .all(|c| matches!(c.look, Look::Knob { .. })));
    // A drag up on Attack's knob steps it up, an undo step.
    let r = s.ui.rect_of(play_tab::LIST).unwrap();
    let k = l.control_rect(r, Row::Attack.place()).unwrap();
    let (x, y) = (k.x + k.w / 2, k.y + k.h / 2);
    let a = drag(&mut s, (x, y), (x, y - 40)).unwrap();
    apply(&mut s, &a);
    lift(&mut s);
    assert!(s.state.settings.amp.attack_ms > 2.0);
    assert_eq!(s.tab.row(&s.ui), Row::Attack, "the wheel's now");
    // Layers: a tap on Cycle picks it.
    open(&mut s, Section::Layers);
    let r = s.ui.rect_of(play_tab::LIST).unwrap();
    let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    let placed = l
        .panel
        .as_ref()
        .unwrap()
        .place(r, &l.controls)
        .into_iter()
        .find(|p| p.row == Row::Layers.place())
        .unwrap();
    let cycle = placed.options[1];
    let a = drag(
        &mut s,
        (cycle.x + 4, cycle.y + 4),
        (cycle.x + 4, cycle.y + 4),
    )
    .unwrap();
    apply(&mut s, &a);
    lift(&mut s);
    assert_eq!(row_value(&mut s, Row::Layers), "Cycle");
    // Scenes: the crossfader is faint until A and B are stored, and says
    // why when touched.
    let r = s.ui.rect_of(play_tab::SCENES).unwrap();
    let l = s.ui.get::<SettingList>(play_tab::SCENES).unwrap();
    let fader = l.control_rect(r, 3).unwrap();
    assert!(l.controls[3].faint);
    let ev =
        s.ui.touch(1, Touch::Down, fader.x + fader.w / 2, fader.y + fader.h / 2)
            .unwrap();
    s.ui.touch(1, Touch::Up, 0, 0);
    assert_eq!(ev.kind, EventKind::Select(3));
    assert_eq!(
        s.ui.get::<SettingList>(play_tab::SCENES)
            .unwrap()
            .why_faint(3),
        Some("Store A first")
    );
}

/// A sliced pad (R4): the Pad section's Slice picks the slice a hit
/// plays, from 1 to the last, one undo step a turn; on a pad that isn't
/// sliced it is faint and says why.
#[test]
fn slice_picks_the_slice_a_sliced_pad_plays() {
    let mut s = play(false);
    assert_eq!(row_value(&mut s, Row::Slice), "–");
    let list = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    assert!(list.controls[Row::Slice.place()].faint);
    assert!(list.controls[Row::Slice.place()].why.contains("1 pad"));
    // Slice the pad in 8, as Chop to one pad would.
    s.project
        .edit_pad(PAD, None, |p| {
            let r = &mut p.layers[0].region;
            let (a, b) = (r.start, r.end);
            r.slices = (1..8).map(|k| a + (b - a) * k / 8).collect();
        })
        .unwrap();
    let view = s.state.sounds[0].view.clone();
    s.state = PlayState::from_project(&s.project, PAD, |_| view.clone()).unwrap();
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert_eq!(row_value(&mut s, Row::Slice), "1 of 8");
    step_apply(&mut s, Row::Slice, 3);
    assert_eq!(row_value(&mut s, Row::Slice), "4 of 8");
    assert_eq!(s.project.pad(PAD).unwrap().slice, 3);
    step_apply(&mut s, Row::Slice, 20);
    assert_eq!(
        row_value(&mut s, Row::Slice),
        "8 of 8",
        "no further than the last"
    );
    let chips = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
    let pad = Section::ALL
        .iter()
        .position(|x| *x == Section::Pad)
        .unwrap();
    assert!(
        chips.chips[pad].sub.ends_with("· 8 slices"),
        "{}",
        chips.chips[pad].sub
    );
    // The steps on one row are one turn: one undo takes them back.
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD).unwrap().slice, 0);
}

/// The pad sliced in 8 (as Chop to one pad would), shown again.
fn slice_in_8(s: &mut PlaySample) {
    s.project
        .edit_pad(PAD, None, |p| {
            let r = &mut p.layers[0].region;
            let (a, b) = (r.start, r.end);
            r.slices = (1..8).map(|k| a + (b - a) * k / 8).collect();
        })
        .unwrap();
    let view = s.state.sounds[0].view.clone();
    s.state = PlayState::from_project(&s.project, PAD, |_| view.clone()).unwrap();
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
}

/// R4 part 5: the Slices section shows a sliced pad's slices on its
/// sound, picks one by a tap or the Slice row, and sets that slice's own
/// pitch, level, reverse, filter, fade out and where it plays to, one
/// undo step a row; on a pad that isn't sliced it is faint and says how.
#[test]
fn the_slices_section_sets_each_slices_own_settings() {
    let mut s = play(false);
    assert_eq!(row_value(&mut s, Row::SliceTune), "–");
    let list = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    assert!(list.controls[Row::SliceTune.place()].faint);
    assert!(list.controls[Row::SliceTune.place()].why.contains("1 pad"));
    assert_eq!(step(&mut s, Row::SliceTune, 1), None, "nothing to set");
    slice_in_8(&mut s);
    let sliced = s.project.pad(PAD).unwrap().layers[0].region.clone();
    open(&mut s, Section::Slices);
    let chips = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
    let at = Section::ALL
        .iter()
        .position(|x| *x == Section::Slices)
        .unwrap();
    assert_eq!(chips.chips[at].sub, "8 slices");
    // The picture: a fixed line numbered at each slice's start.
    let w = s.ui.get::<Waveform>(play_tab::SLICES).unwrap();
    let marks: Vec<&str> = w.markers().iter().map(|m| m.label.as_str()).collect();
    assert_eq!(marks, ["1", "2", "3", "4", "5", "6", "7", "8"]);
    assert!(w.markers().iter().all(|m| m.fixed));
    // The Slice row picks slice 3; a tap on slice 6's line picks it.
    assert_eq!(row_value(&mut s, Row::SlicePick), "1 of 8");
    assert_eq!(step(&mut s, Row::SlicePick, 2), None, "only what it shows");
    assert_eq!(s.tab.slice(), 2);
    assert_eq!(row_value(&mut s, Row::SlicePick), "3 of 8");
    let st = s.state.clone();
    let tap = UiEvent {
        widget: play_tab::SLICES.into(),
        kind: EventKind::Marker { index: 5, frame: 0 },
    };
    assert_eq!(s.tab.action(&mut s.ui, &tap, &st), None);
    assert_eq!(s.tab.slice(), 5);
    // Slice 6 up 5 semitones, down 3 dB, reversed, filtered, faded, on
    // to the sound's end.
    step_apply(&mut s, Row::SliceTune, 5);
    step_apply(&mut s, Row::SliceLevel, -6);
    step_apply(&mut s, Row::SliceReverse, 1);
    step_apply(&mut s, Row::SliceCutoff, -1);
    step_apply(&mut s, Row::SliceFade, 6);
    step_apply(&mut s, Row::SliceTo, 1);
    assert_eq!(row_value(&mut s, Row::SliceTune), "+5 st");
    assert_eq!(row_value(&mut s, Row::SliceLevel), "-3.0 dB");
    assert_eq!(row_value(&mut s, Row::SliceReverse), "On");
    assert_eq!(row_value(&mut s, Row::SliceFade), "20 ms");
    assert_eq!(row_value(&mut s, Row::SliceTo), "Sound end");
    let r = &s.project.pad(PAD).unwrap().layers[0].region;
    let six = r.slice_set(5);
    assert_eq!(six.tune_st, 5);
    assert_eq!(six.level_db, -3.0);
    assert!(six.reverse && six.to_end);
    assert_eq!(six.fade_out_ms, Some(20.0));
    assert!(six.cutoff_hz.is_some_and(|hz| hz < 20_000.0), "{six:?}");
    assert_eq!(r.slice_set(4), SliceSet::DEFAULT, "slice 5 as it was");
    // Of the region, only its slices' settings changed.
    let mut plain = r.clone();
    plain.slice_sets.clear();
    assert_eq!(plain, sliced);
    let chips = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
    assert_eq!(chips.chips[at].sub, "8 · 1 set");
    // Up from 20 kHz is Off again.
    step_apply(&mut s, Row::SliceCutoff, 1);
    assert_eq!(row_value(&mut s, Row::SliceCutoff), "Off");
    // Back on slice 1, its own settings show.
    let st = s.state.clone();
    let tap = UiEvent {
        widget: play_tab::SLICES.into(),
        kind: EventKind::Marker { index: 0, frame: 0 },
    };
    s.tab.action(&mut s.ui, &tap, &st);
    assert_eq!(row_value(&mut s, Row::SliceTune), "+0 st");
    // Q-Link 3 turns the slice's level, half a dB a detent.
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 2, 2, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::SliceLevel), "+1.0 dB");
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0]
            .region
            .slice_set(0)
            .level_db,
        1.0
    );
    // Each row was its own undo step: the last one goes back alone.
    assert!(s.project.undo());
    let r = &s.project.pad(PAD).unwrap().layers[0].region;
    assert_eq!(r.slice_set(0), SliceSet::DEFAULT);
    assert_eq!(r.slice_set(5).tune_st, 5);
}

#[test]
fn the_movement_section_runs_the_lfo_and_the_pitch_envelope() {
    use mpfree_sampler::model::{LfoShape, LfoTarget};
    use mpfree_sampler_ui::play_widgets::MotionView;
    let mut s = play(false);
    let before = s.state.settings.clone();
    open(&mut s, Section::Movement);
    assert!(s.ui.rect_of(play_tab::MOTION).is_some(), "its picture");
    let why = |s: &PlaySample, row: Row| {
        let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
        l.controls[row.place()].why.clone()
    };
    let chip = |s: &PlaySample| {
        let c = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
        let i = Section::ALL.iter().position(|x| *x == Section::Movement);
        c.chips[i.unwrap()].sub.clone()
    };
    let caption = |s: &PlaySample| {
        let m = s.ui.get::<MotionView>(play_tab::MOTION).unwrap();
        (m.lfo_on, m.env_on, m.caption.clone())
    };
    // Off until the LFO is given something to move: its other rows and
    // its Q-Links wait.
    assert_eq!(chip(&s), "Off");
    assert_eq!(
        why(&s, Row::LfoRate),
        "The LFO is off: pick what it moves in LFO to"
    );
    assert_eq!(
        why(&s, Row::PitchDecay),
        "The pitch envelope is off: give Pitch env an amount"
    );
    let turn = |s: &mut PlaySample, knob, delta, fine| {
        let st = s.state.clone();
        let a = s.tab.qlink(&mut s.ui, &st, knob, delta, fine);
        if let Some(a) = &a {
            apply(s, a);
        }
        a
    };
    assert_eq!(turn(&mut s, 0, 1, false), None);
    // On Pitch: a vibrato of a tenth, at 5 Hz from each hit.
    step_apply(&mut s, Row::LfoTo, 1);
    assert_eq!(s.state.settings.lfo.target, LfoTarget::Pitch);
    assert_eq!(row_value(&mut s, Row::LfoTo), "Pitch");
    assert_eq!(row_value(&mut s, Row::LfoRate), "5.0 Hz");
    assert_eq!(row_value(&mut s, Row::LfoDepth), "+10 %");
    assert_eq!(why(&s, Row::LfoRate), "");
    assert_eq!(
        caption(&s),
        (
            true,
            false,
            "LFO: sine on pitch, ±1.2 st at 5.0 Hz · pitch envelope off".into()
        )
    );
    assert_eq!(chip(&s), "pitch LFO · 5.0 Hz");
    // The rate: a step up the list, synced to the tempo (120 BPM: 6 Hz
    // is an eighth triplet), a step faster, and free again.
    step_apply(&mut s, Row::LfoRate, 1);
    assert_eq!(s.state.settings.lfo.rate_hz, 6.0);
    assert_eq!(s.state.bpm, 120.0);
    step_apply(&mut s, Row::LfoSync, 1);
    assert_eq!(row_value(&mut s, Row::LfoRate), "1/8 T");
    step_apply(&mut s, Row::LfoRate, 1);
    assert_eq!(row_value(&mut s, Row::LfoRate), "1/16");
    assert_eq!(s.state.settings.lfo.beats, Some(0.25));
    step_apply(&mut s, Row::LfoSync, 1);
    assert_eq!(
        (s.state.settings.lfo.beats, s.state.settings.lfo.rate_hz),
        (None, 8.0),
        "a sixteenth at 120 BPM is 8 Hz"
    );
    // Q-Link 1 turns a free rate an octave in six detents.
    turn(&mut s, 0, 6, false).unwrap();
    assert_eq!(s.state.settings.lfo.rate_hz, 16.0);
    assert_eq!(
        s.ui.get::<KnobCard>(play_tab::QLINKS[0]).unwrap().value,
        "16.0"
    );
    // The wave, the depth (below 0 the other way) and retrigger.
    step_apply(&mut s, Row::LfoWave, 1);
    assert_eq!(s.state.settings.lfo.shape, LfoShape::Triangle);
    step_apply(&mut s, Row::LfoDepth, -3);
    assert_eq!(row_value(&mut s, Row::LfoDepth), "-5 %");
    step_apply(&mut s, Row::LfoRetrig, 1);
    assert!(!s.state.settings.lfo.retrig);
    assert_eq!(row_value(&mut s, Row::LfoRetrig), "Off");
    // The pitch envelope: up an octave, back over 200 ms.
    turn(&mut s, 2, 12, false).unwrap();
    turn(&mut s, 3, 8, false).unwrap();
    let e = s.state.settings.pitch_env;
    assert_eq!((e.semitones, e.decay_ms), (12.0, 200.0));
    assert_eq!(row_value(&mut s, Row::PitchEnv), "+12 st");
    assert_eq!(row_value(&mut s, Row::PitchDecay), "200 ms");
    assert_eq!(
        caption(&s),
        (
            true,
            true,
            "LFO: triangle on pitch, ±0.6 st at 16.0 Hz · pitch from +12 st over 200 ms".into()
        )
    );
    assert_eq!(chip(&s), "pitch LFO + env");
    // SHIFT: a tenth of a semitone a detent.
    turn(&mut s, 2, -5, true).unwrap();
    assert_eq!(row_value(&mut s, Row::PitchEnv), "+11.5 st");
    // Mod: velocity on the cutoff waits for a filter; on the pitch it
    // goes a semitone a step.
    open(&mut s, Section::Mod);
    assert_eq!(
        why(&s, Row::VelToCutoff),
        "The filter is off: pick one in the Filter section"
    );
    step_apply(&mut s, Row::VelToPitch, 2);
    assert_eq!(row_value(&mut s, Row::VelToPitch), "+2 st");
    // It is all the pad's, saved with it, and the log says what moved.
    let p = s.project.pad(PAD).unwrap();
    assert_eq!(p.lfo, s.state.settings.lfo);
    assert_eq!(p.vel_to_pitch, 2.0);
    let log = describe(&before, p);
    for part in [
        "lfo to Off -> Pitch",
        "lfo wave Sine -> Triangle",
        "retrigger On -> Off",
        "pitch env Off -> +11.5 st",
        "vel → pitch Off -> +2 st",
    ] {
        assert!(log.contains(part), "{part} in {log}");
    }
}

/// Mono, Legato and Glide (sampler R5.5) in the Pitch section: for a pad
/// with keys, saying why while they do nothing.
#[test]
fn the_pitch_section_plays_a_keyed_pad_one_note_at_a_time() {
    let mut s = play(false);
    open(&mut s, Section::Pitch);
    let why = |s: &PlaySample, row: Row| {
        let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
        l.controls[row.place()].why.clone()
    };
    let chip = |s: &PlaySample| {
        let c = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
        let i = Section::ALL.iter().position(|x| *x == Section::Pitch);
        c.chips[i.unwrap()].sub.clone()
    };
    assert_eq!(
        why(&s, Row::Mono),
        "Mono plays the pad's keys: turn Keys on"
    );
    step_apply(&mut s, Row::Keys, 1);
    assert!(s.state.settings.keys.is_some());
    assert_eq!(why(&s, Row::Mono), "");
    assert_eq!(
        why(&s, Row::Legato),
        "These play one note at a time: turn Mono on"
    );
    step_apply(&mut s, Row::Mono, 1);
    assert!(s.state.settings.mono.on);
    assert_eq!(row_value(&mut s, Row::Mono), "On");
    assert!(chip(&s).ends_with(" · mono"), "{}", chip(&s));
    assert_eq!(row_value(&mut s, Row::Glide), "Off");
    assert_eq!(why(&s, Row::GlideOn), "Glide is off: give it a time");
    // Glide steps through its list: 10 ms, then up to 200 ms.
    step_apply(&mut s, Row::Glide, 1);
    assert_eq!(row_value(&mut s, Row::Glide), "10 ms");
    step_apply(&mut s, Row::Glide, 4);
    assert_eq!(s.state.settings.mono.glide_ms, 200.0);
    assert_eq!(row_value(&mut s, Row::GlideOn), "Every note");
    assert_eq!(why(&s, Row::GlideOn), "");
    step_apply(&mut s, Row::GlideOn, 1);
    assert!(s.state.settings.mono.glide_held);
    assert_eq!(row_value(&mut s, Row::GlideOn), "Held keys");
    step_apply(&mut s, Row::Legato, 1);
    assert!(s.state.settings.mono.legato);
    assert!(chip(&s).ends_with(" · legato"), "{}", chip(&s));
    // Down past the shortest is off; up past the longest stays at 2 s.
    step_apply(&mut s, Row::Glide, -9);
    assert_eq!(row_value(&mut s, Row::Glide), "Off");
    step_apply(&mut s, Row::Glide, 20);
    assert_eq!(row_value(&mut s, Row::Glide), "2.00 s");
}

#[test]
fn the_filter_section_offers_a_notch_a_driven_ladder_a_vowel_and_a_comb() {
    let mut s = play(false);
    open(&mut s, Section::Filter);
    let why = |s: &PlaySample, row: Row| {
        let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
        l.controls[row.place()].why.clone()
    };
    let chip = |s: &PlaySample| {
        let c = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
        let i = Section::ALL.iter().position(|x| *x == Section::Filter);
        c.chips[i.unwrap()].sub.clone()
    };
    // Off, Low-pass, High-pass, Band-pass, then the Notch (F5: the Type
    // row steps the types, the Slope row their slopes).
    step_apply(&mut s, Row::Filter, 4);
    assert_eq!(s.state.settings.filter.kind, FilterKind::Notch);
    assert_eq!(row_value(&mut s, Row::Filter), "Notch");
    assert_eq!(row_value(&mut s, Row::Slope), "12 dB");
    assert!(chip(&s).starts_with("Notch 12 dB · "), "{}", chip(&s));
    assert_eq!(
        why(&s, Row::Drive),
        "Drive is the Ladder's: pick Ladder in the Type row"
    );
    // Past the Peak.
    step_apply(&mut s, Row::Filter, 2);
    assert_eq!(s.state.settings.filter.kind, FilterKind::Ladder);
    assert_eq!(row_value(&mut s, Row::Filter), "Ladder");
    assert_eq!(row_value(&mut s, Row::Slope), "24 dB, fixed");
    assert_eq!(why(&s, Row::Slope), "The Ladder is 24 dB, fixed");
    assert_eq!(why(&s, Row::Drive), "");
    assert_eq!(row_value(&mut s, Row::Drive), "Off");
    // The arrows go 3 dB a step, up to 24 dB.
    step_apply(&mut s, Row::Drive, 2);
    assert_eq!(s.state.settings.filter.drive_db, 6.0);
    assert_eq!(row_value(&mut s, Row::Drive), "6.0 dB");
    step_apply(&mut s, Row::Drive, 20);
    assert_eq!(row_value(&mut s, Row::Drive), "24.0 dB");
    // The vowel: its cutoff names the vowel it makes, and its knobs take
    // its names.
    step_apply(&mut s, Row::Filter, 1);
    assert_eq!(s.state.settings.filter.kind, FilterKind::Vowel);
    assert_eq!(Row::Cutoff.name_on(&s.state.settings), "Vowel");
    assert_eq!(Row::Resonance.name_on(&s.state.settings), "Sharpness");
    let cutoff = row_value(&mut s, Row::Cutoff);
    assert!(
        ["oo", "oh", "ah", "eh", "ee"]
            .iter()
            .any(|v| cutoff.ends_with(&format!(" · {v}"))),
        "{cutoff}"
    );
    assert!(chip(&s).starts_with("Vowel · "), "{}", chip(&s));
    assert_eq!(row_value(&mut s, Row::Slope), "–");
    // The comb: its cutoff is the note it rings at, its Pitch.
    step_apply(&mut s, Row::Filter, 1);
    assert_eq!(s.state.settings.filter.kind, FilterKind::Comb);
    assert_eq!(row_value(&mut s, Row::Filter), "Comb");
    let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    assert_eq!(l.rows[Row::Cutoff.place()].name, "Pitch");
    assert_eq!(l.rows[Row::Resonance.place()].name, "Feedback");
    let cutoff = row_value(&mut s, Row::Cutoff);
    assert!(
        cutoff.contains(" Hz · ") || cutoff.contains(" kHz · "),
        "{cutoff}"
    );
    assert!(chip(&s).starts_with("Comb · "), "{}", chip(&s));
    // The last: the arrow stays; back down to Off.
    assert!(step(&mut s, Row::Filter, 1).is_none());
    step_apply(&mut s, Row::Filter, -8);
    assert_eq!(s.state.settings.filter.kind, FilterKind::Off);
    assert_eq!(
        why(&s, Row::Drive),
        "The filter is off: pick one in the Type row"
    );
}

#[test]
fn the_type_and_slope_rows_step_through_every_type_and_slope() {
    let mut s = play(false);
    open(&mut s, Section::Filter);
    let chip = |s: &PlaySample| {
        let c = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
        let i = Section::ALL.iter().position(|x| *x == Section::Filter);
        c.chips[i.unwrap()].sub.clone()
    };
    // From Off, a low-pass at the pad's slope (12 dB); down to 6 dB, then
    // up through every slope it has.
    step_apply(&mut s, Row::Filter, 1);
    assert_eq!(row_value(&mut s, Row::Slope), "12 dB");
    step_apply(&mut s, Row::Slope, -5);
    assert_eq!(row_value(&mut s, Row::Slope), "6 dB");
    assert!(chip(&s).starts_with("LP 6 dB · "), "{}", chip(&s));
    for db in [12, 18, 24, 36, 48] {
        step_apply(&mut s, Row::Slope, 1);
        assert_eq!(row_value(&mut s, Row::Slope), format!("{db} dB"));
        assert!(
            chip(&s).starts_with(&format!("LP {db} dB · ")),
            "{}",
            chip(&s)
        );
    }
    assert!(step(&mut s, Row::Slope, 1).is_none(), "48 dB is the last");
    // A type change keeps the slope where the new type has it, else the
    // nearest; the fixed types and those without one leave it be.
    for (kind, slope) in [
        ("High-pass", "48 dB"),
        ("Band-pass", "48 dB"),
        ("Notch", "24 dB"),
        ("Peak", "12 dB, fixed"),
        ("Ladder", "24 dB, fixed"),
        ("Vowel", "–"),
        ("Comb", "–"),
    ] {
        step_apply(&mut s, Row::Filter, 1);
        assert_eq!(row_value(&mut s, Row::Filter), kind);
        assert_eq!(row_value(&mut s, Row::Slope), slope, "{kind}");
    }
    assert!(step(&mut s, Row::Slope, 1).is_none(), "the Comb has none");
    step_apply(&mut s, Row::Filter, -7);
    assert_eq!(row_value(&mut s, Row::Filter), "Low-pass");
    assert_eq!(row_value(&mut s, Row::Slope), "24 dB", "the notch's, kept");
    // A pad saved with a slope its type doesn't have shows the one it
    // plays: a 5-pole low-pass is 24 dB, and steps on from there.
    let mut p = s.state.settings.clone();
    p.filter.poles = 5;
    assert_eq!(play_tab::value_text(&p, Row::Slope), "24 dB");
    let p = play_tab::stepped(&p, Row::Slope, 1, false);
    assert_eq!((p.filter.kind, p.filter.poles), (FilterKind::LowPass, 6));
    // The Peak's Resonance is its boost, from nothing at 0 to 35 dB.
    step_apply(&mut s, Row::Filter, 4);
    assert_eq!(s.state.settings.filter.kind, FilterKind::Peak);
    let res = row_value(&mut s, Row::Resonance);
    assert!(res.contains(" % · +") && res.ends_with(" dB"), "{res}");
    let mut p = s.state.settings.clone();
    p.filter.resonance = 0.0;
    assert_eq!(play_tab::value_text(&p, Row::Resonance), "0 % · +0.0 dB");
    p.filter.resonance = 1.0;
    let full = play_tab::value_text(&p, Row::Resonance);
    assert!(full.starts_with("100 % · +3"), "{full}");
    let f = s.ui.get::<FilterCurve>(play_tab::FILTER).unwrap();
    assert!(f.caption.contains(" · boost +"), "{}", f.caption);
}

#[test]
fn a_comb_names_the_note_it_rings_at() {
    use mpfree_sampler_ui::play_tab::note_of;
    assert_eq!(note_of(440.0), "A4");
    assert_eq!(note_of(220.0), "A3");
    assert_eq!(note_of(261.63), "C4");
    assert_eq!(note_of(277.18), "C#4");
    assert_eq!(note_of(20.0), "D#0");
}

// ------------------------------------------------ the pad filter screen (F5)

/// Event `kind` on widget `widget`: what the tab asks for, and what it
/// says on the hint line.
fn on(s: &mut PlaySample, widget: &str, kind: EventKind) -> (Option<PlayAction>, Option<String>) {
    let ev = UiEvent {
        widget: widget.into(),
        kind,
    };
    let st = s.state.clone();
    let a = s.tab.action(&mut s.ui, &ev, &st);
    (a, s.tab.take_note())
}

/// The pad shown again as the project has it (after an undo).
fn resync(s: &mut PlaySample) {
    let view = s.state.sounds[0].view.clone();
    s.state = PlayState::from_project(&s.project, PAD, |_| view.clone()).unwrap();
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
}

/// The pad's settings changed as `shape` says, as one edit.
fn shape(s: &mut PlaySample, shape: impl FnOnce(&mut Pad)) {
    let mut p = s.state.settings.clone();
    shape(&mut p);
    let a = PlayAction::Pad {
        pad: PAD,
        after: Box::new(p),
        gesture: 0xF5,
    };
    apply(s, &a);
}

fn filter_chip(s: &PlaySample) -> String {
    let c = s.ui.get::<SectionChips>(play_tab::SECTIONS).unwrap();
    let i = Section::ALL.iter().position(|x| *x == Section::Filter);
    c.chips[i.unwrap()].sub.clone()
}

/// Over the graph: On bypasses (its own undo step, every setting kept),
/// Compare plays the filter as it was when the section opened until it is
/// tapped again, an edit is made or the section is left (keeping the
/// changes), and Copy, Paste and Reset take the whole filter, each saying
/// what it did on the hint line.
#[test]
fn the_filter_header_bypasses_compares_copies_pastes_and_resets() {
    let mut s = play(false);
    open(&mut s, Section::Filter);
    // Nothing copied, nothing changed: Paste and Compare say why.
    let (a, note) = on(&mut s, play_tab::FILTER, EventKind::Select(3));
    assert_eq!(a, None);
    assert_eq!(
        note.as_deref(),
        Some("Paste: nothing copied yet. Copy a pad's filter first")
    );
    let f = s.ui.get::<FilterCurve>(play_tab::FILTER).unwrap();
    let texts: Vec<&str> = f.buttons.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, ["● On", "Compare", "Copy", "Paste", "Reset"]);
    assert!(f.buttons[0].faint, "the filter is off: nothing to bypass");
    assert!(f.buttons[1].faint && f.buttons[3].faint);
    step_apply(&mut s, Row::Filter, 1);
    s.tab.lift();
    assert_eq!(filter_chip(&s), "LP 12 dB · 2.00 kHz");

    // On: bypassed, one undo step, the settings kept.
    let (a, note) = on(&mut s, play_tab::FILTER, EventKind::Tab(0));
    apply(&mut s, &a.unwrap());
    assert!(s.state.settings.filter_bypass);
    assert_eq!(s.state.settings.filter.kind, FilterKind::LowPass);
    assert_eq!(
        note.as_deref(),
        Some("Bypassed: the pad plays unfiltered · Low-pass 12 dB at 2.00 kHz waits")
    );
    assert_eq!(filter_chip(&s), "Bypassed · LP 12");
    let f = s.ui.get::<FilterCurve>(play_tab::FILTER).unwrap();
    assert!(f.bypassed);
    assert_eq!(f.buttons[0].text, "○ Off");
    assert_eq!(f.title, "Filter · bypassed");
    assert!(s.project.undo());
    resync(&mut s);
    assert!(!s.state.settings.filter_bypass, "UNDO puts it back on");
    assert_eq!(s.state.settings.filter.kind, FilterKind::LowPass);

    // Compare: the filter as it was (Off), the changes dashed; again: back.
    let (a, note) = on(&mut s, play_tab::FILTER, EventKind::Tab(1));
    apply(&mut s, &a.unwrap());
    assert_eq!(s.state.settings.filter.kind, FilterKind::Off);
    assert!(s.tab.comparing());
    assert!(note.unwrap().starts_with("Comparing: "));
    assert_eq!(filter_chip(&s), "Comparing");
    let f = s.ui.get::<FilterCurve>(play_tab::FILTER).unwrap();
    assert_eq!(f.title, "Filter · as it was");
    assert_eq!(f.buttons[1].text, "Comparing");
    assert_eq!(f.ghost.map(|g| g.kind), Some(FilterKind::LowPass));
    let (a, note) = on(&mut s, play_tab::FILTER, EventKind::Tab(1));
    apply(&mut s, &a.unwrap());
    assert_eq!(s.state.settings.filter.kind, FilterKind::LowPass);
    assert!(!s.tab.comparing());
    assert_eq!(note.as_deref(), Some("Back to your changes"));
    // An edit while comparing is made on the earlier version.
    let (a, _) = on(&mut s, play_tab::FILTER, EventKind::Tab(1));
    apply(&mut s, &a.unwrap());
    s.tab.lift();
    step_apply(&mut s, Row::Filter, 2);
    assert!(!s.tab.comparing());
    assert_eq!(s.state.settings.filter.kind, FilterKind::HighPass);
    s.tab.lift();
    // Leaving the section while comparing keeps the changes.
    let (a, _) = on(&mut s, play_tab::FILTER, EventKind::Tab(1));
    apply(&mut s, &a.unwrap());
    assert_eq!(s.state.settings.filter.kind, FilterKind::Off);
    let amp = Section::ALL
        .iter()
        .position(|x| *x == Section::Amp)
        .unwrap();
    let (a, _) = on(&mut s, play_tab::SECTIONS, EventKind::Tab(amp));
    apply(&mut s, &a.expect("the changes go back"));
    assert_eq!(s.tab.section(), Section::Amp);
    assert_eq!(s.state.settings.filter.kind, FilterKind::HighPass);
    assert!(!s.tab.comparing());
    open(&mut s, Section::Filter);

    // Copy, Reset, Paste.
    let (a, note) = on(&mut s, play_tab::FILTER, EventKind::Tab(2));
    assert_eq!(a, None, "Copy changes nothing");
    assert_eq!(
        note.as_deref(),
        Some("Copied A5's filter: High-pass 12 dB, 200 Hz and its envelope")
    );
    let (a, note) = on(&mut s, play_tab::FILTER, EventKind::Tab(4));
    apply(&mut s, &a.unwrap());
    assert_eq!(s.state.settings.filter.kind, FilterKind::Off);
    assert_eq!(
        note.as_deref(),
        Some("Reset A5's filter to a new pad's: Off · UNDO puts it back")
    );
    let (a, note) = on(&mut s, play_tab::FILTER, EventKind::Tab(3));
    apply(&mut s, &a.unwrap());
    assert_eq!(s.state.settings.filter.kind, FilterKind::HighPass);
    assert_eq!(
        note.as_deref(),
        Some("Pasted A5's filter back · UNDO puts it as it was")
    );
    assert!(s.project.undo());
    assert_eq!(
        s.project.pad(PAD).unwrap().filter.kind,
        FilterKind::Off,
        "Paste is one undo step"
    );
    let mut bypassed = s.state.settings.clone();
    bypassed.filter_bypass = true;
    let d = describe(&s.state.settings, &bypassed);
    assert_eq!(d, "filter bypass Off -> On");
}

/// A tap on the Type chip's name opens the types over the pictures: a tap
/// on a tile puts it on at once, the slope chips set the slope (those the
/// type lacks faint, saying why), the wheel steps the types, and Done or
/// the Filter chip close them.
#[test]
fn the_types_open_over_the_pictures_and_put_a_type_on_at_once() {
    use mpfree_sampler_ui::play_widgets::{FilterTypes, DONE, SLOPE_BASE};
    let mut s = play(false);
    open(&mut s, Section::Filter);
    let (a, _) = on(
        &mut s,
        play_tab::LIST,
        EventKind::Select(Row::Filter.place()),
    );
    assert_eq!(a, None);
    assert_eq!(s.tab.view(), play_tab::View::Types);
    assert!(s.ui.rect_of(play_tab::TYPES_VIEW).is_some());
    assert_eq!(s.ui.rect_of(play_tab::FILTER), None, "over the pictures");
    assert!(s.ui.rect_of(play_tab::LIST).is_some(), "the panel stays");
    let t = s.ui.get::<FilterTypes>(play_tab::TYPES_VIEW).unwrap();
    let names: Vec<&str> = t.tiles.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Off",
            "Low-pass",
            "High-pass",
            "Band-pass",
            "Notch",
            "Peak",
            "Ladder",
            "Vowel",
            "Comb"
        ]
    );
    let (a, _) = on(&mut s, play_tab::TYPES_VIEW, EventKind::Tab(3));
    apply(&mut s, &a.unwrap());
    assert_eq!(s.state.settings.filter.kind, FilterKind::BandPass);
    let t = s.ui.get::<FilterTypes>(play_tab::TYPES_VIEW).unwrap();
    assert_eq!(t.picked, 3);
    let chips: Vec<(&str, bool, bool)> = t
        .slopes
        .iter()
        .map(|c| (c.text.as_str(), c.lit, c.faint))
        .collect();
    assert_eq!(
        chips,
        [
            ("6", false, true),
            ("12", true, false),
            ("18", false, true),
            ("24", false, false),
            ("36", false, false),
            ("48", false, false)
        ]
    );
    let (a, note) = on(&mut s, play_tab::TYPES_VIEW, EventKind::Select(SLOPE_BASE));
    assert_eq!(a, None);
    assert_eq!(note.as_deref(), Some("The band-pass has no 6 dB slope"));
    let (a, _) = on(&mut s, play_tab::TYPES_VIEW, EventKind::Tab(SLOPE_BASE + 5));
    apply(&mut s, &a.unwrap());
    assert_eq!(s.state.settings.filter.poles, 8);
    // The wheel steps the types.
    let st = s.state.clone();
    let a = s.tab.wheel(&mut s.ui, &st, 1).unwrap();
    apply(&mut s, &a);
    assert_eq!(s.state.settings.filter.kind, FilterKind::Notch);
    let (_, _) = on(&mut s, play_tab::TYPES_VIEW, EventKind::Tab(DONE));
    assert_eq!(s.tab.view(), play_tab::View::Main);
    assert!(s.ui.rect_of(play_tab::FILTER).is_some());
    // The Filter chip closes them too.
    on(
        &mut s,
        play_tab::LIST,
        EventKind::Select(Row::Filter.place()),
    );
    assert_eq!(s.tab.view(), play_tab::View::Types);
    let filter = Section::ALL
        .iter()
        .position(|x| *x == Section::Filter)
        .unwrap();
    on(&mut s, play_tab::SECTIONS, EventKind::Tab(filter));
    assert_eq!(s.tab.view(), play_tab::View::Main);
    assert_eq!(s.tab.section(), Section::Filter);
}

/// The Moves button opens what moves the filter in place of the pictures
/// and panels: the cutoff's moves with their reach, the resonance's and
/// the envelope's, where a hit takes it, Q-Links on key track, velocity,
/// random and env depth, and jumps to the LFO's and pressure's sections.
#[test]
fn what_moves_the_filter_shows_every_move_and_turns_them() {
    use mpfree_sampler_ui::play_widgets::{FilterMoves, DONE};
    let mut s = play(false);
    open(&mut s, Section::Filter);
    shape(&mut s, |p| {
        p.filter.kind = FilterKind::LowPass;
        p.filter.poles = 4;
        p.filter.cutoff_hz = 1000.0;
        p.filter.env_octaves = 2.0;
        p.vel_to_cutoff = 0.5;
        p.vel_to_env = 0.6;
        p.pressure.target = PressureTarget::Off;
    });
    s.tab.lift();
    let l = s.ui.get::<SettingList>(play_tab::LIST).unwrap();
    assert_eq!(
        l.controls[Row::Moves.place()].look,
        mpfree_sampler_ui::panel::Look::Button("3 moves ›".into())
    );
    let (a, _) = on(&mut s, play_tab::LIST, EventKind::Tab(Row::Moves.place()));
    assert_eq!(a, None);
    assert_eq!(s.tab.view(), play_tab::View::Moves);
    assert!(s.ui.rect_of(play_tab::MOVES_VIEW).is_some());
    assert_eq!(s.ui.rect_of(play_tab::LIST), None, "in place of the panels");
    assert_eq!(s.tab.rows(), &play_tab::MOVES);
    assert_eq!(
        s.tab.qlinks_text(),
        "key track, vel → cutoff, random, env depth"
    );
    let m = s.ui.get::<FilterMoves>(play_tab::MOVES_VIEW).unwrap();
    let names: Vec<&str> = m.cutoff.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["Envelope", "Velocity", "Key", "Random", "LFO", "Pressure"]
    );
    assert_eq!(m.cutoff[0].span, Some((0.0, 2.0)));
    assert_eq!(m.cutoff[1].span, Some((-2.0, 0.0)), "soft hits darker");
    assert!(m.cutoff[4].faint && m.cutoff[5].faint, "set elsewhere, off");
    assert_eq!(
        m.hits,
        "A hit now: the hardest opens to 4.00 kHz, the softest starts at 250 Hz and \
         opens to 435 Hz (key at the root, before the random)."
    );
    // Q-Link 1 turns key track, a knob's drag the row it is on.
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 0, 10, false).unwrap();
    apply(&mut s, &a);
    assert!((s.state.settings.key_track - 0.1).abs() < 1e-6);
    s.tab.lift();
    let (a, _) = on(
        &mut s,
        play_tab::MOVES_VIEW,
        EventKind::Step { index: 3, delta: 5 },
    );
    apply(&mut s, &a.unwrap());
    assert!((s.state.settings.cutoff_random - 0.25).abs() < 1e-6);
    let m = s.ui.get::<FilterMoves>(play_tab::MOVES_VIEW).unwrap();
    assert_eq!(m.cutoff[3].value, "25 %");
    assert_eq!(m.cutoff[3].aside, "± 0.5 oct a hit");
    assert!(m.cutoff[3].picked);
    // Done: back to the pictures, the Moves button picked.
    on(&mut s, play_tab::MOVES_VIEW, EventKind::Tab(DONE));
    assert_eq!(s.tab.view(), play_tab::View::Main);
    assert_eq!(s.tab.row(&s.ui), Row::Moves);
    // The LFO's jump opens Movement.
    on(&mut s, play_tab::LIST, EventKind::Tab(Row::Moves.place()));
    on(&mut s, play_tab::MOVES_VIEW, EventKind::Tab(1));
    assert_eq!(s.tab.section(), Section::Movement);
    assert_eq!(s.tab.view(), play_tab::View::Main);
}

/// The Filter section's Q-Links follow the type (the Ladder's Drive in
/// place of Env decay) and take its names; the envelope is labelled with
/// the cutoff it takes the filter to, and the graph shows its reach.
#[test]
fn the_filter_q_links_follow_the_type_and_the_envelope_shows_its_hz() {
    let mut s = play(false);
    open(&mut s, Section::Filter);
    shape(&mut s, |p| {
        p.filter.kind = FilterKind::Ladder;
        p.filter.cutoff_hz = 1000.0;
        p.filter.env_octaves = 2.0;
        p.filter_env.sustain = 0.5;
    });
    let card = |s: &PlaySample, k: usize| {
        s.ui.get::<KnobCard>(play_tab::QLINKS[k])
            .unwrap()
            .label
            .clone()
    };
    assert_eq!(card(&s, 3), "Q-Link 4 · Drive");
    assert_eq!(s.tab.qlinks_text(), "cutoff, resonance, env depth, drive");
    let env = s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap();
    let marks: Vec<(f32, &str)> = env.marks.iter().map(|(l, t)| (*l, t.as_str())).collect();
    assert_eq!(
        marks,
        [(1.0, "4.00 kHz"), (0.5, "2.00 kHz"), (0.0, "1.00 kHz")]
    );
    let f = s.ui.get::<FilterCurve>(play_tab::FILTER).unwrap();
    assert_eq!(f.env_target(), Some(4000.0));
    shape(&mut s, |p| p.filter.kind = FilterKind::Comb);
    assert_eq!(card(&s, 0), "Q-Link 1 · Pitch");
    assert_eq!(card(&s, 1), "Q-Link 2 · Feedback");
    assert_eq!(card(&s, 3), "Q-Link 4 · Env decay");
    // Bypassed, the envelope has nothing to label.
    shape(&mut s, |p| p.filter_bypass = true);
    let env = s.ui.get::<EnvelopeView>(play_tab::ENV).unwrap();
    assert!(env.marks.is_empty());
}
