//! The Rec tab: what it shows of the line input, and how Record, Stop and
//! Skip back turn what came in into a sound on a pad, with the input fed
//! period by period as the audio thread feeds it on the MPC.

use mpfree_sampler::model::Project;
use mpfree_sampler::record::{Pick, Progress};
use mpfree_sampler_ui::rec_tab::{self, EndAt, Onto, RecAction, Row, Skip, StartAt};
use mpfree_sampler_ui::rec_widgets::InputView;
use mpfree_sampler_ui::sample::{
    self, bar_frames, feed_input, kit_project, rec_samples, RecSample, RATE,
};
use mpfree_sampler_ui::widgets::{ButtonRow, InfoCard, PadBar, SettingList};
use mpfree_ui::{EventKind, Fonts, SoftCanvas, UiEvent};

fn screen(name: &str) -> RecSample {
    let mut s = rec_samples(Fonts::embedded)
        .unwrap()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

fn buttons(s: &RecSample) -> (Vec<String>, Vec<bool>) {
    let b = s.ui.get::<ButtonRow>(rec_tab::BUTTONS).unwrap();
    (b.labels.clone(), b.enabled.clone())
}

fn press(s: &mut RecSample, button: usize) -> Option<RecAction> {
    let ev = UiEvent {
        widget: rec_tab::BUTTONS.into(),
        kind: EventKind::Tab(button),
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

fn step(s: &mut RecSample, row: Row, delta: i32) {
    let index = Row::ALL.iter().position(|r| *r == row).unwrap();
    let ev = UiEvent {
        widget: rec_tab::LIST.into(),
        kind: EventKind::Step { index, delta },
    };
    let st = s.state.clone();
    assert_eq!(
        s.tab.action(&mut s.ui, &ev, &st),
        None,
        "settings change no project"
    );
}

/// The audio thread's next `seconds`, then the screen's next frame.
fn run(s: &mut RecSample, seconds: f64) {
    feed_input(&mut s.feed, seconds, 0.4, true);
    if let Some(r) = s.recording.as_mut() {
        r.poll(&s.input).unwrap();
    }
    s.tab
        .feed(&mut s.ui, &s.input, s.recording.as_ref(), true, 0.033);
}

/// What the app does with a finished take: onto the target pad, one undo step.
fn keep(s: &mut RecSample, project: &mut Project, pick: Pick) -> usize {
    let r = s.recording.take().unwrap();
    let src = r.finish("Take 1", pick).unwrap().unwrap();
    let pad = s.state.target.unwrap();
    project.put_source(pad, src).unwrap();
    pad
}

#[test]
fn the_tab_shows_the_input_the_bar_lines_and_what_skip_back_keeps() {
    let s = screen("rec");
    let bar = s.ui.get::<PadBar>(rec_tab::BAR).unwrap();
    assert_eq!(bar.title, "Record onto pad A10", "A5 to A9 have sounds");
    assert_eq!(bar.active, rec_tab::TAB);
    assert_eq!(bar.tabs[rec_tab::TAB], "Rec");
    let v = s.ui.get::<InputView>(rec_tab::VIEW).unwrap();
    assert!(
        v.meter_db.iter().all(|d| (-12.0..0.0).contains(d)),
        "{:?}",
        v.meter_db
    );
    // 40 s fed: the strip shows the last 30, with every bar line in them.
    assert_eq!(v.now, s.input.now());
    let in_view = v
        .bars
        .iter()
        .filter(|b| **b + v.strip.span() > v.now)
        .count();
    assert!(
        (11..=12).contains(&in_view),
        "{in_view} bar lines in 30 s at 92 BPM"
    );
    let skip = v.skip.as_ref().unwrap();
    assert_eq!(skip.end - skip.start, 10 * u64::from(RATE));
    assert_eq!(skip.label, "Skip back 10 s");
    assert!(v.note.is_empty() && v.threshold.is_none());
    assert_eq!(v.caption, "40.00 s held");
    let (labels, enabled) = buttons(&s);
    assert_eq!(labels, ["Record", "Skip back 10 s"]);
    assert_eq!(enabled, [true, true]);
    let list = s.ui.get::<SettingList>(rec_tab::LIST).unwrap();
    let rows: Vec<(&str, &str)> = list
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.value.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("Source", "Line in"),
            ("Channels", "Left + right"),
            ("Start", "Now"),
            ("Threshold", "-30 dB"),
            ("Pre-roll", "20 ms"),
            ("End", "Stop"),
            ("Length", "4 bars"),
            ("Silence for", "1 s"),
            ("Skip back", "10 s"),
            ("Onto", "Next empty"),
        ]
    );
}

#[test]
fn a_take_on_the_next_bar_is_drawn_as_it_grows_and_the_button_stops_it() {
    let s = screen("rec-take");
    let v = s.ui.get::<InputView>(rec_tab::VIEW).unwrap();
    let take = v.take.as_ref().unwrap();
    assert!(v.bars.contains(&take.start), "it started on a bar line");
    assert!(take.label.starts_with("● "));
    let (labels, enabled) = buttons(&s);
    assert_eq!(labels, ["Stop", "Skip back 2 bars"]);
    assert_eq!(enabled, [true, false]);
    // Two whole bars back from the latest bar line.
    let skip = v.skip.as_ref().unwrap();
    assert_eq!(
        (skip.start, skip.end),
        (s.input.bars.back(2).unwrap(), s.input.bars.back(0).unwrap())
    );
}

#[test]
fn record_then_stop_puts_the_take_onto_the_next_empty_pad() {
    let mut s = screen("rec");
    let (mut project, _) = kit_project();
    assert_eq!(press(&mut s, 0), Some(RecAction::Record));
    // The app makes the take from the tab's settings and polls it.
    s.recording = Some(s.tab.settings.recording(&s.input, bar_frames()));
    let from = s.input.now();
    run(&mut s, 1.5);
    assert!(s.tab.recording());
    assert_eq!(buttons(&s).0[0], "Stop");
    assert_eq!(press(&mut s, 0), Some(RecAction::Stop));
    s.recording.as_mut().unwrap().stop(&s.input);
    let to = s.input.now();
    run(&mut s, 0.1);
    assert_eq!(
        s.recording.as_ref().unwrap().progress(),
        Progress::Done { frames: to - from }
    );
    assert!(!s.tab.recording());
    assert_eq!(buttons(&s).0[0], "Record");
    let pad = keep(&mut s, &mut project, Pick::Stereo);
    assert_eq!(pad, 9, "A10");
    let p = project.pad(pad).unwrap();
    assert_eq!(p.name, "Take 1");
    let src = project.source(p.layers[0].region.source).unwrap();
    assert_eq!((src.channels(), src.frames()), (2, to - from));
    assert!(project.undo());
    assert!(project.pad(pad).is_none(), "one undo step takes it off");
}

#[test]
fn a_threshold_take_from_the_wheel_waits_for_the_input_and_ends_after_its_bars() {
    let mut s = screen("rec");
    // Start is the row picked: the wheel turns it to Threshold.
    let st = s.state.clone();
    s.tab.wheel(&mut s.ui, &st, 1);
    assert_eq!(s.tab.settings.start, StartAt::Threshold);
    assert_eq!(
        s.ui.get::<InfoCard>(rec_tab::WHEEL).unwrap().text,
        "Start: Threshold"
    );
    step(&mut s, Row::End, 2);
    step(&mut s, Row::Length, -2);
    step(&mut s, Row::Input, 1);
    assert_eq!(
        (s.tab.settings.end, s.tab.settings.bars, s.tab.settings.pick),
        (EndAt::Length, 1, Pick::Left)
    );
    // Silence first: it waits, and the picture says for what.
    feed_input(&mut s.feed, 2.0, 0.0, true);
    s.recording = Some(s.tab.settings.recording(&s.input, bar_frames()));
    run_quiet(&mut s, 1.0);
    let v = s.ui.get::<InputView>(rec_tab::VIEW).unwrap();
    assert_eq!(v.note, "Waiting for the input to go over -30 dB");
    assert!(v.threshold.is_some());
    assert_eq!(buttons(&s).0[0], "Stop");
    // Then the drums: it starts at the first hit, less the pre-roll, and
    // lasts one bar at 92 BPM.
    run(&mut s, 4.0);
    let r = s.recording.as_ref().unwrap();
    assert_eq!(
        r.progress(),
        Progress::Done {
            frames: bar_frames().round() as u64
        }
    );
    let (mut project, _) = kit_project();
    let pad = keep(&mut s, &mut project, Pick::Left);
    let src = project
        .source(project.pad(pad).unwrap().layers[0].region.source)
        .unwrap();
    assert_eq!(src.channels(), 1, "the left jack only");
}

fn run_quiet(s: &mut RecSample, seconds: f64) {
    feed_input(&mut s.feed, seconds, 0.0, true);
    s.recording.as_mut().unwrap().poll(&s.input).unwrap();
    s.tab
        .feed(&mut s.ui, &s.input, s.recording.as_ref(), true, 0.033);
}

#[test]
fn skip_back_by_bars_and_onto_this_pad() {
    let mut s = screen("rec");
    step(&mut s, Row::SkipBack, 6);
    assert_eq!(s.tab.settings.skip, Skip::Bars(4));
    step(&mut s, Row::Onto, 1);
    assert_eq!(s.tab.settings.onto, Onto::ThisPad);
    assert_eq!(buttons(&s).0[1], "Skip back 4 bars");
    assert_eq!(press(&mut s, 1), Some(RecAction::SkipBack));
    // The app cuts the last four bars on the bar lines.
    let data = s.input.last_bars(4).unwrap();
    assert_eq!(
        data.len() as u64 / 2,
        s.input.bars.back(0).unwrap() - s.input.bars.back(4).unwrap()
    );
    let bars4 = 4.0 * bar_frames();
    assert!(((data.len() / 2) as f64 - bars4).abs() <= 1.0);
}

#[test]
fn with_the_line_input_off_the_tab_says_where_to_turn_it_on() {
    let s = screen("rec-off");
    let v = s.ui.get::<InputView>(rec_tab::VIEW).unwrap();
    assert_eq!(v.note, rec_tab::OFF_NOTE);
    assert!(v
        .note
        .starts_with("The line input is off: Looper tab, MENU, Line input"));
    assert!(v.skip.is_none());
    assert_eq!(buttons(&s).1, [false, false]);
}

#[test]
fn the_source_row_resamples_the_mix_and_waits_while_a_take_is_made() {
    let mut s = screen("rec");
    let index = Row::ALL.iter().position(|r| *r == Row::Source).unwrap();
    let turn = |s: &mut RecSample, delta: i32| {
        let ev = UiEvent {
            widget: rec_tab::LIST.into(),
            kind: EventKind::Step { index, delta },
        };
        let st = s.state.clone();
        s.tab.action(&mut s.ui, &ev, &st)
    };
    // Line in, then Master: the app is told, and the tab says what it hears.
    assert_eq!(turn(&mut s, 1), Some(RecAction::Source(1)));
    let v = s.ui.get::<InputView>(rec_tab::VIEW).unwrap();
    assert_eq!(v.title, "Master");
    let bar = s.ui.get::<PadBar>(rec_tab::BAR).unwrap();
    assert_eq!(bar.subtitle, "Left + right · resampling Master");
    // Past the last source it stays there; back to the line input.
    assert_eq!(turn(&mut s, 40), Some(RecAction::Source(21)));
    assert_eq!(
        s.ui.get::<SettingList>(rec_tab::LIST).unwrap().rows[0].value,
        "Pad 16"
    );
    assert_eq!(turn(&mut s, 5), None, "already the last");
    assert_eq!(turn(&mut s, -30), Some(RecAction::Source(0)));
    assert_eq!(
        s.ui.get::<InputView>(rec_tab::VIEW).unwrap().title,
        "Line input"
    );
    // While a take is being made, the source stays.
    assert_eq!(press(&mut s, 0), Some(RecAction::Record));
    s.recording = Some(s.tab.settings.recording(&s.input, bar_frames()));
    run(&mut s, 0.5);
    assert!(s.tab.recording());
    assert_eq!(turn(&mut s, 1), None);
    assert_eq!(s.tab.settings.source, 0);
    assert_eq!(
        s.ui.get::<InfoCard>(rec_tab::WHEEL).unwrap().text,
        "Source: stop the take first"
    );
}

#[test]
fn the_master_screen_shows_what_it_resamples() {
    let s = screen("rec-master");
    let v = s.ui.get::<InputView>(rec_tab::VIEW).unwrap();
    assert_eq!(v.title, "Master");
    let skip = v.skip.as_ref().unwrap();
    assert_eq!(skip.label, "Skip back 4 bars");
    assert!(((skip.end - skip.start) as f64 - 4.0 * bar_frames()).abs() <= 2.0);
    assert_eq!(buttons(&s).0, ["Record", "Skip back 4 bars"]);
}
