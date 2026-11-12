//! The Rec tab's Auto-sample mode: what the picture and the list show of
//! the plan, where the instrument would go, and what Start, Cancel and Rec
//! ask the app for. The app's run of it (a copy of the Synth tab's patch on
//! a worker thread) is tested in mpfree-app.

use mpfree_sampler::model::Keys;
use mpfree_sampler_ui::auto_tab::{self, AutoAction, Onto, Row};
use mpfree_sampler_ui::auto_widgets::{KeyMapView, LayerBand};
use mpfree_sampler_ui::sample::{self, auto_samples, auto_state, AutoSample, AUTO_SYNTH, RATE};
use mpfree_sampler_ui::widgets::{ButtonRow, InfoCard, PadBar, SettingList};
use mpfree_ui::{EventKind, Fonts, SoftCanvas, UiEvent};

fn screen(name: &str) -> AutoSample {
    let mut s = auto_samples(Fonts::embedded)
        .unwrap()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    paint(&mut s);
    s
}

fn paint(s: &mut AutoSample) {
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
}

fn bar(s: &AutoSample) -> (String, String) {
    let b = s.ui.get::<PadBar>(auto_tab::BAR).unwrap();
    (b.title.clone(), b.subtitle.clone())
}

fn view(s: &AutoSample) -> &KeyMapView {
    s.ui.get::<KeyMapView>(auto_tab::VIEW).unwrap()
}

fn buttons(s: &AutoSample) -> (Vec<String>, Vec<bool>) {
    let b = s.ui.get::<ButtonRow>(auto_tab::BUTTONS).unwrap();
    (b.labels.clone(), b.enabled.clone())
}

fn act(s: &mut AutoSample, widget: &str, kind: EventKind) -> Option<AutoAction> {
    let ev = UiEvent {
        widget: widget.into(),
        kind,
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

fn press(s: &mut AutoSample, button: usize) -> Option<AutoAction> {
    act(s, auto_tab::BUTTONS, EventKind::Tab(button))
}

/// Steps a row, then shows the state the app works out again (the pads
/// the instrument would go onto follow the settings).
fn step(s: &mut AutoSample, row: Row, delta: i32) {
    let kind = EventKind::Step {
        index: row.index(),
        delta,
    };
    assert_eq!(act(s, auto_tab::LIST, kind), None);
    if s.state.running.is_none() {
        s.state = auto_state(&s.project, &s.tab.settings, None);
        let st = s.state.clone();
        s.tab.update(&mut s.ui, &st);
    }
    paint(s);
}

fn row(s: &AutoSample, r: Row) -> (String, String) {
    let list = s.ui.get::<SettingList>(auto_tab::LIST).unwrap();
    let x = &list.rows[r.index()];
    (x.name.clone(), x.value.clone())
}

#[test]
fn the_default_plan_fills_a_new_kit_and_start_asks_for_it() {
    let mut s = screen("rec-auto");
    assert_eq!(
        bar(&s),
        (
            "Auto-sample Poly Pluck onto pads A1–A16".to_string(),
            "16 notes × 3 layers = 48 takes · 3:22 long · up to 102 MB".to_string()
        )
    );
    let list = s.ui.get::<SettingList>(auto_tab::LIST).unwrap();
    assert_eq!(list.right, "A1–A16");
    let rows: Vec<(String, String)> = Row::ALL.iter().map(|r| row(&s, *r)).collect();
    let want = [
        ("Source", "Poly Pluck"),
        ("Lowest note", "C2"),
        ("Highest note", "A5"),
        ("Every", "3 semitones"),
        ("Layers", "3 · 40 90 127"),
        ("Layer spread", "Even"),
        ("Round robins", "1"),
        ("Hold", "2 s"),
        ("Tail", "up to 4 s"),
        ("Loop", "Off"),
        ("Channels", "Stereo"),
        ("Normalize", "On"),
        ("Onto", "Empty pads"),
    ];
    assert_eq!(
        rows,
        want.map(|(a, b)| (a.to_string(), b.to_string())).to_vec()
    );
    // The picture: each note's zone reaches halfway to its neighbours and
    // the ends stretch to 0 and 127; three velocity bands, each played at
    // its top.
    let v = view(&s);
    assert_eq!(v.title, AUTO_SYNTH);
    assert_eq!(v.right, "16 notes, 3 semitones");
    assert_eq!(v.zones.len(), 16);
    assert_eq!(
        (v.zones[0], v.zones[1], v.zones[15]),
        (
            Keys {
                lo: 0,
                hi: 37,
                root: 36
            },
            Keys {
                lo: 38,
                hi: 40,
                root: 39
            },
            Keys {
                lo: 80,
                hi: 127,
                root: 81
            }
        )
    );
    assert_eq!(
        v.layers,
        [
            LayerBand {
                lo: 1,
                hi: 40,
                played: 40
            },
            LayerBand {
                lo: 41,
                hi: 90,
                played: 90
            },
            LayerBand {
                lo: 91,
                hi: 127,
                played: 127
            },
        ]
    );
    assert_eq!((v.first, v.last), (24, 95), "whole octaves round the notes");
    assert!(v.note.is_empty() && v.progress.is_none());
    assert_eq!(
        buttons(&s),
        (vec!["Start".into(), "Rec".into()], vec![true, true])
    );
    // Start hands the app the plan, named after the patch.
    let Some(AutoAction::Start(plan)) = press(&mut s, 0) else {
        panic!("Start asks for the plan");
    };
    assert_eq!(
        (plan.name.as_str(), plan.notes().len(), plan.rate),
        (AUTO_SYNTH, 16, RATE)
    );
    assert_eq!(plan, s.tab.settings.plan(AUTO_SYNTH, RATE));
    assert_eq!(press(&mut s, 1), Some(AutoAction::Back));
    assert_eq!(
        act(&mut s, auto_tab::BAR, EventKind::Tab(1)),
        Some(AutoAction::Tab(1))
    );
}

#[test]
fn the_wheel_and_the_rows_change_the_plan_and_the_picture_follows() {
    let mut s = screen("rec-auto");
    // A tap picks a row; the wheel card names it.
    act(
        &mut s,
        auto_tab::LIST,
        EventKind::Select(Row::Every.index()),
    );
    let list = s.ui.get_mut::<SettingList>(auto_tab::LIST).unwrap();
    list.selected = Row::Every.index();
    s.tab.wheel(&mut s.ui, &s.state.clone(), 9);
    assert_eq!(s.tab.settings.every, 12);
    assert_eq!(
        s.ui.get::<InfoCard>(auto_tab::WHEEL).unwrap().text,
        "Every: 12 semitones (octave)"
    );
    s.state = auto_state(&s.project, &s.tab.settings, None);
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert_eq!(bar(&s).0, "Auto-sample Poly Pluck onto pads A1–A4");
    assert_eq!(view(&s).zones.len(), 4);
    // Two round robins, five layers spread to the soft end.
    step(&mut s, Row::RoundRobins, 1);
    step(&mut s, Row::Layers, 2);
    step(&mut s, Row::Curve, 1);
    assert_eq!(row(&s, Row::Layers).1, "5 · 5 20 46 81 127");
    let v = view(&s);
    assert_eq!(v.layers.len(), 5);
    assert_eq!(v.round_robins, 2);
    assert!(bar(&s)
        .1
        .starts_with("4 notes × 5 layers × 2 round robins = 40 takes"));
    // Loops, mono, no normalize.
    step(&mut s, Row::Loop, 2);
    step(&mut s, Row::Channels, 1);
    step(&mut s, Row::Normalize, 1);
    assert_eq!(
        [Row::Loop, Row::Channels, Row::Normalize].map(|r| row(&s, r).1),
        ["Ping-pong", "Mono", "Off"]
    );
    let Some(AutoAction::Start(plan)) = press(&mut s, 0) else {
        panic!("Start");
    };
    assert_eq!(
        (plan.channels, plan.normalize, plan.round_robins),
        (1, false, 2)
    );
}

#[test]
fn while_it_runs_the_picture_follows_and_only_cancel_answers() {
    let mut s = screen("rec-auto-running");
    assert_eq!(bar(&s).0, "Auto-sample Poly Pluck onto pads A10–A16");
    let v = view(&s);
    assert_eq!(v.progress, Some((20, 42)));
    assert_eq!(v.current, Some(54));
    // Six takes a note: three notes are done.
    assert_eq!(v.done, [true, true, true, false, false, false, false]);
    assert_eq!(v.caption, "take 21 of 42 · F#3");
    assert_eq!(
        buttons(&s),
        (vec!["Cancel".into(), "Rec".into()], vec![true, false])
    );
    assert_eq!(press(&mut s, 0), Some(AutoAction::Cancel));
    assert_eq!(press(&mut s, 1), None);
    // The plan waits while it runs.
    let before = s.tab.settings;
    step(&mut s, Row::Every, 1);
    assert_eq!(s.tab.settings, before);
    assert_eq!(
        s.ui.get::<InfoCard>(auto_tab::WHEEL).unwrap().text,
        "Cancel first to change the plan"
    );
}

#[test]
fn with_no_room_start_waits_and_the_picture_says_why() {
    let mut s = screen("rec-auto-full");
    assert_eq!(bar(&s).0, "No 16 empty pads in a row on this kit");
    assert_eq!(
        view(&s).note,
        "Onto: Whole kit replaces the kit's pads (UNDO puts them back), or sample fewer notes"
    );
    assert_eq!(buttons(&s).1, [false, true]);
    assert_eq!(press(&mut s, 0), None);
    // Whole kit: from A1, replacing.
    step(&mut s, Row::Onto, 1);
    assert_eq!(s.tab.settings.onto, Onto::WholeKit);
    assert_eq!(row(&s, Row::Onto).1, "Whole kit (replacing)");
    assert_eq!(bar(&s).0, "Auto-sample Poly Pluck onto pads A1–A16");
    assert!(view(&s).note.is_empty());
    assert!(matches!(press(&mut s, 0), Some(AutoAction::Start(_))));
    // Back to empty pads with an octave apart: the 7 empty pads at the end
    // take its 4 notes.
    step(&mut s, Row::Onto, -1);
    step(&mut s, Row::Every, 9);
    assert_eq!(bar(&s).0, "Auto-sample Poly Pluck onto pads A10–A13");
    // More notes than a kit has pads.
    step(&mut s, Row::Every, -11);
    step(&mut s, Row::Highest, 3);
    assert_eq!(bar(&s).0, "49 notes need 49 pads; a kit has 16");
    // 147 takes are past the memory budget too, which the picture says
    // first.
    assert!(view(&s).note.contains("MB"), "{}", view(&s).note);
}

#[test]
fn a_plan_past_the_memory_budget_says_so_and_start_waits() {
    let mut s = screen("rec-auto");
    step(&mut s, Row::RoundRobins, 1);
    step(&mut s, Row::Layers, 5);
    step(&mut s, Row::Hold, 4);
    assert_eq!((s.tab.settings.layers, s.tab.settings.round_robins), (8, 2));
    let note = view(&s).note.clone();
    assert!(note.contains("MB"), "{note}");
    assert_eq!(buttons(&s).1, [false, true]);
    assert_eq!(press(&mut s, 0), None);
}

#[test]
fn external_gear_is_offered_with_a_midi_out_and_starts_on_its_channel() {
    // Without a MIDI out the Source row keeps the synth, and says why.
    let mut s = screen("rec-auto");
    assert_eq!(s.state.gear, None);
    let source = EventKind::Step {
        index: Row::Source.index(),
        delta: 1,
    };
    assert_eq!(act(&mut s, auto_tab::LIST, source), None);
    assert_eq!(s.tab.settings.gear, None);
    let card = &s.ui.get::<InfoCard>(auto_tab::WHEEL).unwrap().text;
    assert!(card.contains("needs a MIDI out"), "{card}");
    // With one, the steps after the synth are the gear on channels 1 to 16.
    s.state.gear = Some(sample::GEAR_OUT.to_string());
    for _ in 0..3 {
        step(&mut s, Row::Source, 1);
    }
    assert_eq!(s.tab.settings.gear, Some(2));
    assert_eq!(
        row(&s, Row::Source),
        ("Source".to_string(), "Gear · MIDI ch 3".to_string())
    );
    assert_eq!(bar(&s).0, "Auto-sample the gear onto pads A1–A16");
    assert_eq!(view(&s).title, "Gear · MIDI channel 3");
    assert!(
        view(&s)
            .note
            .starts_with("Plays MIDI channel 3 on MIDI Out (DIN)"),
        "{}",
        view(&s).note
    );
    match press(&mut s, 0) {
        Some(AutoAction::StartGear { plan, channel: 2 }) => {
            assert_eq!(plan.name, auto_tab::GEAR_NAME);
            assert_eq!(plan, s.tab.settings.plan("Gear", RATE));
        }
        a => panic!("{a:?}"),
    }
    // Channel 16 is the last; back past channel 1 is the synth again.
    step(&mut s, Row::Source, 20);
    assert_eq!(s.tab.settings.gear, Some(15));
    step(&mut s, Row::Source, -17);
    assert_eq!(s.tab.settings.gear, None);
    assert_eq!(row(&s, Row::Source).1, AUTO_SYNTH);
    assert!(matches!(press(&mut s, 0), Some(AutoAction::Start(_))));
    // Gear picked but the MIDI out gone: Start waits and the page says so.
    let mut g = screen("rec-auto-gear");
    assert_eq!(buttons(&g).1, [true, true]);
    g.state.gear = None;
    let st = g.state.clone();
    g.tab.update(&mut g.ui, &st);
    assert_eq!(bar(&g).0, "Auto-sample: no MIDI out for the gear");
    assert_eq!(buttons(&g).1, [false, true]);
    assert_eq!(press(&mut g, 0), None);
}
