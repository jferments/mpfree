//! The Pads tab: the kit's pads on the map, picking and hearing a pad,
//! copy, paste and clear, the Pad list, and putting the kit's sounds on
//! pads, each change one undo step in a real project.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::PlayMode;
use mpfree_sampler_ui::pads_tab::{self, Button, PadsAction, PadsState, Row};
use mpfree_sampler_ui::sample::{self, pads_sample, pads_samples, PadsSample, PAD, RATE};
use mpfree_sampler_ui::widgets::{ButtonRow, PadBar, PadMap, SettingList};
use mpfree_ui::widgets::ListView;
use mpfree_ui::{EventKind, Fonts, SoftCanvas, Touch, UiEvent};

/// The Pads tab on the sample kit with pad `pad` picked, drawn once.
fn pads(pad: usize) -> PadsSample {
    let mut s = pads_sample(Fonts::embedded(), pad).unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

/// Picks pad `pad`, as the app does for [`PadsAction::Pick`].
fn pick(s: &mut PadsSample, pad: usize) {
    s.state = PadsState::from_project(&s.project, pad);
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
}

/// Applies an action and shows the new state, as the app does.
fn apply(s: &mut PadsSample, a: &PadsAction) {
    a.apply(&mut s.project).unwrap();
    let pad = s.state.pad;
    pick(s, pad);
}

fn act(s: &mut PadsSample, widget: &str, kind: EventKind) -> Option<PadsAction> {
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &UiEvent { widget: widget.into(), kind }, &st)
}

fn row_value(s: &PadsSample, row: Row) -> String {
    s.ui.get::<SettingList>(pads_tab::LIST).unwrap().rows[Row::ALL.iter().position(|r| *r == row).unwrap()].value.clone()
}

fn step(s: &mut PadsSample, row: Row, delta: i32) -> Option<PadsAction> {
    let index = Row::ALL.iter().position(|r| *r == row).unwrap();
    act(s, pads_tab::LIST, EventKind::Step { index, delta })
}

fn press(s: &mut PadsSample, b: Button) -> Option<PadsAction> {
    let st = s.state.clone();
    s.tab.press(&mut s.ui, &st, b)
}

#[test]
fn the_map_shows_the_banks_pads_and_the_list_the_kits_sounds() {
    let s = pads(PAD);
    let bar = s.ui.get::<PadBar>(pads_tab::BAR).unwrap();
    assert_eq!(bar.title, "Pad A5 · Break 92");
    assert_eq!(bar.subtitle, "6 of 16 pads with a sound · 3 sounds in the kit");
    assert_eq!((bar.active, bar.tabs[0].as_str()), (pads_tab::TAB, "Pads"));
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(map.title, "Kit 1 · Breaks");
    let names: Vec<&str> = map.cells.iter().take(9).map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["Kick 58", "", "", "", "Break 92", "Break 92 1", "Break 92 2", "Break 92 3", "Break 92 4"]);
    assert_eq!((map.cells[4].label.as_str(), map.cells[4].detail.as_str(), map.selected), ("A5", "Loop", 4));
    assert!(map.cells.iter().all(|c| c.clip.is_none()), "no pad launches a clip yet");
    // Pad 1 is bottom left, as on the MPC.
    let r = s.ui.rect_of(pads_tab::MAP).unwrap();
    let (a1, a13) = (map.cell_rect(r, 0), map.cell_rect(r, 12));
    assert!(a1.y > a13.y && a1.x == a13.x);
    for (row, value) in [(Row::Sound, "Break 92"), (Row::Mode, "Loop"), (Row::Polyphony, "8 voices"), (Row::MuteGroup, "Off"),
                         (Row::Level, "+0.0 dB"), (Row::Pan, "Center"), (Row::Tune, "+0 st")]
    {
        assert_eq!(row_value(&s, row), value, "{row:?}");
    }
    assert_eq!(s.ui.get::<SettingList>(pads_tab::LIST).unwrap().right, "clip: none");
    let sounds = s.ui.get::<ListView>(pads_tab::SOUNDS).unwrap();
    let items: Vec<(String, String)> = sounds.items.iter().map(|i| (i.text.clone(), i.detail.clone())).collect();
    assert_eq!(items, vec![("Break 92".into(), "5.22 s · 5 pads".into()), ("Kick 58".into(), "400 ms · A1".into()),
                           ("Hat closed".into(), "80 ms · unused".into())]);
    assert_eq!(sounds.selected, 0, "the picked pad's sound");
    let put = s.ui.get::<ButtonRow>(pads_tab::PUT).unwrap();
    assert_eq!((put.labels[0].as_str(), put.enabled[0]), ("Put on A5", true));
    let b = s.ui.get::<ButtonRow>(pads_tab::BUTTONS).unwrap();
    assert_eq!(b.enabled, vec![true, false, true], "nothing copied yet");
}

#[test]
fn a_tap_picks_a_pad_and_a_tap_on_the_pad_picked_plays_it() {
    let mut s = pads(PAD);
    let r = s.ui.rect_of(pads_tab::MAP).unwrap();
    let a1 = s.ui.get::<PadMap>(pads_tab::MAP).unwrap().cell_rect(r, 0);
    let ev = s.ui.touch(1, Touch::Down, a1.x + 10, a1.y + 10).unwrap();
    let st = s.state.clone();
    assert_eq!(s.tab.action(&mut s.ui, &ev, &st), Some(PadsAction::Pick(0)));
    assert_eq!(s.ui.touch(1, Touch::Up, a1.x + 10, a1.y + 10), None);
    pick(&mut s, 0);
    assert_eq!(s.ui.get::<PadBar>(pads_tab::BAR).unwrap().title, "Pad A1 · Kick 58");
    assert_eq!(s.ui.get::<ListView>(pads_tab::SOUNDS).unwrap().selected, 1, "the kick is picked in the sounds");
    for (phase, down) in [(Touch::Down, true), (Touch::Up, false)] {
        let ev = s.ui.touch(2, phase, a1.x + 10, a1.y + 10).unwrap();
        let st = s.state.clone();
        assert_eq!(s.tab.action(&mut s.ui, &ev, &st), Some(PadsAction::Audition { pad: 0, down }));
    }
    // A pad held on the hardware lights up.
    let st = s.state.clone();
    s.tab.set_lit(&mut s.ui, &st, 6, true);
    assert!(s.ui.get::<PadMap>(pads_tab::MAP).unwrap().lit[6]);
}

#[test]
fn copy_paste_and_clear_are_one_undo_step_each() {
    let mut s = pads(PAD);
    assert_eq!(press(&mut s, Button::Copy), None);
    pick(&mut s, 10);
    let b = s.ui.get::<ButtonRow>(pads_tab::BUTTONS).unwrap();
    assert_eq!((b.labels[1].as_str(), b.enabled.clone()), ("Paste A5", vec![false, true, false]));
    let a = press(&mut s, Button::Paste).unwrap();
    apply(&mut s, &a);
    let loop_pad = s.project.pad(PAD).unwrap().clone();
    assert_eq!(s.project.pad(10), Some(&loop_pad), "A11 is a copy of A5");
    assert_eq!(press(&mut s, Button::Paste), None, "pasting the same again changes nothing");
    let a = press(&mut s, Button::Clear).unwrap();
    apply(&mut s, &a);
    assert!(s.project.pad(10).is_none());
    assert!(s.project.undo());
    assert_eq!(s.project.pad(10), Some(&loop_pad));
    assert!(s.project.undo());
    assert!(s.project.pad(10).is_none());
    assert_eq!(s.project.pad(PAD), Some(&loop_pad), "A5 untouched");
}

#[test]
fn put_on_pad_gives_an_empty_pad_a_sound_and_a_pad_a_new_sound() {
    let mut s = pads(10);
    assert_eq!(act(&mut s, pads_tab::SOUNDS, EventKind::Select(2)), None);
    let a = act(&mut s, pads_tab::PUT, EventKind::Tab(0)).unwrap();
    apply(&mut s, &a);
    let hat = s.project.pad(10).unwrap();
    assert_eq!((hat.name.as_str(), hat.layers.len(), hat.layers[0].region.mode), ("Hat closed", 1, PlayMode::OneShot));
    assert_eq!(hat.layers[0].region.frames(), (0.08 * RATE as f32) as u64, "the whole sound");
    assert_eq!(s.ui.get::<ListView>(pads_tab::SOUNDS).unwrap().items[2].detail, "80 ms · A11");
    // The new pad plays.
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&s.project, RATE));
    assert!(e.schedule(0, Event::NoteOn { pad: 10, velocity: 127 }));
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    e.render(&mut l, &mut r);
    assert!(l.iter().any(|x| x.abs() > 0.01), "A11 sounds");

    // The kick on the loop's pad: still a loop (over the kick), its other
    // settings kept, named after the kick.
    pick(&mut s, PAD);
    act(&mut s, pads_tab::SOUNDS, EventKind::Select(1));
    let a = act(&mut s, pads_tab::PUT, EventKind::Tab(0)).unwrap();
    let before = s.project.pad(PAD).unwrap().clone();
    apply(&mut s, &a);
    let p = s.project.pad(PAD).unwrap();
    let r = &p.layers[0].region;
    assert_eq!((p.name.as_str(), r.mode, r.start, r.gain_db), ("Kick 58", PlayMode::Loop, 0, before.layers[0].region.gain_db));
    assert!(r.sustain_loop.is_some_and(|l| l.start > r.start && l.end < r.end));
    assert_eq!((p.amp, p.polyphony), (before.amp, before.polyphony));
    assert!(s.project.undo());
    assert_eq!(s.project.pad(PAD), Some(&before));
}

#[test]
fn the_list_qlinks_and_wheel_change_the_pad_picked() {
    let mut s = pads(0);
    // Sound steps through the kit's sounds: the kick, then the hat.
    let a = step(&mut s, Row::Sound, 1).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&s, Row::Sound), "Hat closed");
    // One shot to Loop adds a loop, so the pad stays playable.
    s.tab.lift();
    let a = step(&mut s, Row::Mode, 3).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&s, Row::Mode), "Loop");
    assert!(s.project.pad(0).unwrap().layers[0].region.sustain_loop.is_some());
    // Q-Link 1 turns the level; a turn is one undo step.
    s.tab.lift();
    let mut gestures = Vec::new();
    for _ in 0..3 {
        let st = s.state.clone();
        let a = s.tab.qlink(&mut s.ui, &st, 0, -2).unwrap();
        if let PadsAction::Set { gesture, .. } = &a {
            gestures.push(*gesture);
        }
        apply(&mut s, &a);
    }
    assert_eq!(row_value(&s, Row::Level), "-3.0 dB");
    assert!(gestures.windows(2).all(|w| w[0] == w[1] && w[0].is_some()));
    assert_eq!(s.tab.row(&s.ui), Row::Level, "the turn picks its row");
    assert!(s.project.undo());
    assert_eq!(s.project.pad(0).unwrap().level_db, 0.0);
    // The wheel steps the row picked: Q-Link 4 picks voices.
    s.tab.lift();
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 3, -1).unwrap();
    apply(&mut s, &a);
    let st = s.state.clone();
    let a = s.tab.wheel(&mut s.ui, &st, -10).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&s, Row::Polyphony), "Mono");
    // An empty pad has nothing to step but its sound.
    pick(&mut s, 11);
    assert_eq!(step(&mut s, Row::Level, 1), None);
    assert_eq!(row_value(&s, Row::Level), "—");
    assert!(step(&mut s, Row::Sound, -1).is_some(), "the last sound goes on");
}

#[test]
fn every_pads_sample_screen_draws() {
    for mut s in pads_samples(Fonts::embedded).unwrap() {
        let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
        s.ui.paint(&mut c);
        assert!(!s.about.is_empty(), "{}", s.name);
    }
}
