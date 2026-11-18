//! The Pads tab: the kit's pads on the map, picking and hearing a pad,
//! copy, paste and clear, the Pad list, and putting the kit's sounds on
//! pads, each change one undo step in a real project.

use mpfree_midi::profile::PadOut;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{LoopMode, PadKind, PlayMode};
use mpfree_sampler_ui::pads_tab::{
    self, Button, OutPort, PadType, PadsAction, PadsState, PadsTab, Row,
};
use mpfree_sampler_ui::sample::{
    self, pad_outs, pads_sample, pads_samples, pads_types, PadsSample, PAD, RATE,
};
use mpfree_sampler_ui::widgets::{
    ButtonRow, KitTrack, KitTracks, PadBar, PadMap, SectionChips, SettingList,
};
use mpfree_ui::widgets::ListView;
use mpfree_ui::{EventKind, Fonts, SoftCanvas, Touch, UiEvent};

/// The Pads tab on the sample kit with pad `pad` picked, drawn once.
fn pads(pad: usize) -> PadsSample {
    let mut s = pads_sample(Fonts::embedded(), pad).unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

/// Picks pad `pad`, as the app does for [`PadsAction::Pick`] (with the
/// project's tempo, the engines and the ports as it gives them).
fn pick(s: &mut PadsSample, pad: usize) {
    let ports = s.state.ports.clone();
    s.state = PadsState::from_project(&s.project, pad);
    pad_outs(&mut s.state);
    s.state.ports = ports;
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
    s.tab.action(
        &mut s.ui,
        &UiEvent {
            widget: widget.into(),
            kind,
        },
        &st,
    )
}

fn row_value(s: &PadsSample, row: Row) -> String {
    s.ui.get::<SettingList>(pads_tab::LIST).unwrap().rows
        [Row::ALL.iter().position(|r| *r == row).unwrap()]
    .value
    .clone()
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
    assert_eq!(bar.subtitle, "6 of 16 pads in use · 3 sounds in the kit");
    // No Browse | Pad switch: a button that opens the browser for A5.
    assert!(bar.tabs.is_empty());
    assert_eq!(bar.button.as_deref(), Some("Browse for A5"));
    assert!(bar
        .button_rect(s.ui.rect_of(pads_tab::BAR).unwrap())
        .is_some());
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(map.title, "Kit 1 · Breaks");
    let names: Vec<&str> = map.cells.iter().take(9).map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Kick 58",
            "",
            "",
            "",
            "Break 92",
            "Break 92 1",
            "Break 92 2",
            "Break 92 3",
            "Break 92 4"
        ]
    );
    assert_eq!(
        (
            map.cells[4].label.as_str(),
            map.cells[4].detail.as_str(),
            map.selected
        ),
        ("A5", "Loop", 4)
    );
    assert!(
        map.cells.iter().all(|c| c.clip.is_none()),
        "no pad launches a clip yet"
    );
    // Pad 1 is bottom left, as on the MPC.
    let r = s.ui.rect_of(pads_tab::MAP).unwrap();
    let (a1, a13) = (map.cell_rect(r, 0), map.cell_rect(r, 12));
    assert!(a1.y > a13.y && a1.x == a13.x);
    for (row, value) in [
        (Row::Sound, "Break 92"),
        (Row::Mode, "Loop"),
        (Row::Polyphony, "8 voices"),
        (Row::MuteGroup, "Off"),
        (Row::Level, "+0.0 dB"),
        (Row::Pan, "Center"),
        (Row::Tune, "+0 st"),
    ] {
        assert_eq!(row_value(&s, row), value, "{row:?}");
    }
    assert_eq!(
        s.ui.get::<SettingList>(pads_tab::LIST).unwrap().right,
        "clip: none"
    );
    let sounds = s.ui.get::<ListView>(pads_tab::SOUNDS).unwrap();
    let items: Vec<(String, String)> = sounds
        .items
        .iter()
        .map(|i| (i.text.clone(), i.detail.clone()))
        .collect();
    assert_eq!(
        items,
        vec![
            ("Break 92".into(), "5.22 s · 5 pads".into()),
            ("Kick 58".into(), "400 ms · A1".into()),
            ("Hat closed".into(), "80 ms · unused".into())
        ]
    );
    assert_eq!(sounds.selected, 0, "the picked pad's sound");
    let put = s.ui.get::<ButtonRow>(pads_tab::PUT).unwrap();
    assert_eq!(
        (put.labels[0].as_str(), put.enabled[0]),
        ("Put on A5", true)
    );
    let b = s.ui.get::<ButtonRow>(pads_tab::BUTTONS).unwrap();
    assert_eq!(
        b.enabled,
        vec![true, false, true, true],
        "nothing copied yet; the bank has sounds to map to keys"
    );
}

#[test]
fn browse_for_asks_for_the_browser_and_follows_the_pad_picked() {
    let mut s = pads(PAD);
    // A touch on the button gives Tab(0), which asks for the browser.
    let r = s.ui.rect_of(pads_tab::BAR).unwrap();
    let b =
        s.ui.get::<PadBar>(pads_tab::BAR)
            .unwrap()
            .button_rect(r)
            .unwrap();
    let bar = s.ui.get_mut::<PadBar>(pads_tab::BAR).unwrap();
    assert_eq!(
        mpfree_ui::Widget::touch(bar, 1, Touch::Down, b.x + 10, b.y + 10, r),
        Some(EventKind::Tab(0))
    );
    assert_eq!(
        act(&mut s, pads_tab::BAR, EventKind::Tab(0)),
        Some(PadsAction::Browse)
    );
    // Another pad picked: the button names it.
    pick(&mut s, 1);
    assert_eq!(
        s.ui.get::<PadBar>(pads_tab::BAR).unwrap().button.as_deref(),
        Some("Browse for A2")
    );
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
    assert_eq!(
        s.ui.get::<PadBar>(pads_tab::BAR).unwrap().title,
        "Pad A1 · Kick 58"
    );
    assert_eq!(
        s.ui.get::<ListView>(pads_tab::SOUNDS).unwrap().selected,
        1,
        "the kick is picked in the sounds"
    );
    for (phase, down) in [(Touch::Down, true), (Touch::Up, false)] {
        let ev = s.ui.touch(2, phase, a1.x + 10, a1.y + 10).unwrap();
        let st = s.state.clone();
        assert_eq!(
            s.tab.action(&mut s.ui, &ev, &st),
            Some(PadsAction::Audition { pad: 0, down })
        );
    }
    // A pad held on the hardware lights up.
    let st = s.state.clone();
    s.tab.set_lit(&mut s.ui, &st, 6, true);
    assert!(s.ui.get::<PadMap>(pads_tab::MAP).unwrap().lit[6]);
}

/// The strip over the pads (sampler R3): the kit's banks A to H, a dot
/// for each pad with a sound, the bank shown outlined. A tap on a bank,
/// even in the gap before it, asks the app to put it on the pads; a pad
/// picked on bank B shows bank B.
#[test]
fn the_bank_strip_shows_each_banks_sounds_and_a_tap_turns_the_bank() {
    let mut s = pads(PAD);
    let r = s.ui.rect_of(pads_tab::MAP).unwrap();
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(map.banks.len(), 8);
    assert_eq!(map.banks[0].count_ones(), 6, "bank A's six sounds");
    assert!(map.banks[1..].iter().all(|&b| b == 0));
    assert_eq!((map.bank, map.right.as_str()), (0, "bank A"));
    let (cell, answer) = map.bank_rect(r, 2);
    assert!(answer.x < cell.x && answer.contains(cell.x - 1, cell.y + 5));
    assert!(
        map.cell_rect(r, 12).y >= cell.bottom(),
        "the pads sit under the strip"
    );
    let ev = s.ui.touch(1, Touch::Down, cell.x - 1, cell.y + 5).unwrap();
    let st = s.state.clone();
    assert_eq!(s.tab.action(&mut s.ui, &ev, &st), Some(PadsAction::Bank(2)));
    s.ui.touch(1, Touch::Up, cell.x - 1, cell.y + 5);

    // A sound on B5: bank B gets its dot, and picking B5 shows bank B.
    let a5 = s.project.pad(PAD).cloned();
    s.project.set_pad(16 + PAD, a5, None).unwrap();
    pick(&mut s, 16 + PAD);
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(map.banks[1], 1 << PAD);
    assert_eq!((map.bank, map.right.as_str()), (1, "bank B"));
    assert_eq!((map.cells[PAD].label.as_str(), map.selected), ("B5", PAD));
    let title = &s.ui.get::<PadBar>(pads_tab::BAR).unwrap().title;
    assert!(title.starts_with("Pad B5 · "), "{title}");
}

#[test]
fn the_kit_of_row_shows_the_kit_tracks_and_answers_taps() {
    let mut s = pads(PAD);
    let r = s.ui.rect_of(pads_tab::KITS).unwrap();
    let bar = s.ui.rect_of(pads_tab::BAR).unwrap();
    let map = s.ui.rect_of(pads_tab::MAP).unwrap();
    assert!(
        r.y >= bar.bottom() && map.y >= r.bottom(),
        "under the bar, over the pads"
    );
    let track = |t: usize, kit: &str, armed: bool, enabled: bool| KitTrack {
        track: t,
        name: format!("Track {}", t + 1),
        kit: kit.to_string(),
        color: mpfree_ui::Color::rgb(0xFF8A3D),
        armed,
        enabled,
    };
    let w = s.ui.get_mut::<KitTracks>(pads_tab::KITS).unwrap();
    w.tracks = vec![
        track(0, "Breaks", false, true),
        track(2, "Kit 18", true, true),
        track(3, "synth, no kit", false, false),
    ];
    let w = s.ui.get::<KitTracks>(pads_tab::KITS).unwrap();
    let (shown, width) = w.shown(r);
    assert_eq!(shown, vec![0, 1, 2]);
    assert!(width >= 150, "{width}");
    let chip = |s: &PadsSample, i: usize| {
        s.ui.get::<KitTracks>(pads_tab::KITS)
            .unwrap()
            .chip_rect(r, i)
            .unwrap()
    };
    let add = w.add_rect(r);
    assert!(chip(&s, 2).right() < add.x, "+ Kit track on the right");
    // A track with a kit arms it; the armed one and a synth track don't
    // answer; + Kit track does.
    let tap = |s: &mut PadsSample, x: i32, y: i32| {
        let ev = s.ui.touch(1, Touch::Down, x, y).map(|e| e.kind);
        s.ui.touch(1, Touch::Up, x, y);
        ev
    };
    let c0 = chip(&s, 0);
    assert_eq!(
        tap(&mut s, c0.x + 10, c0.y + 10),
        Some(EventKind::Select(0))
    );
    let c1 = chip(&s, 1);
    assert_eq!(tap(&mut s, c1.x + 10, c1.y + 10), None, "armed already");
    let c2 = chip(&s, 2);
    assert_eq!(tap(&mut s, c2.x + 10, c2.y + 10), None, "no kit to show");
    assert_eq!(tap(&mut s, add.x + 10, add.y + 10), Some(EventKind::Tab(0)));
    // More tracks than fit: the armed one keeps the last place.
    let w = s.ui.get_mut::<KitTracks>(pads_tab::KITS).unwrap();
    w.tracks = (0..12).map(|t| track(t, "Breaks", t == 10, true)).collect();
    let (shown, _) = w.shown(r);
    assert!(shown.len() < 12);
    assert_eq!(shown.last(), Some(&10));
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
}

#[test]
fn copy_paste_and_clear_are_one_undo_step_each() {
    let mut s = pads(PAD);
    assert_eq!(press(&mut s, Button::Copy), None);
    pick(&mut s, 10);
    let b = s.ui.get::<ButtonRow>(pads_tab::BUTTONS).unwrap();
    assert_eq!(
        (b.labels[1].as_str(), b.enabled.clone()),
        ("Paste A5", vec![false, true, false, true])
    );
    let a = press(&mut s, Button::Paste).unwrap();
    apply(&mut s, &a);
    let loop_pad = s.project.pad(PAD).unwrap().clone();
    assert_eq!(s.project.pad(10), Some(&loop_pad), "A11 is a copy of A5");
    assert_eq!(
        press(&mut s, Button::Paste),
        None,
        "pasting the same again changes nothing"
    );
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
    assert_eq!(
        (
            hat.name.as_str(),
            hat.layers.len(),
            hat.layers[0].region.mode
        ),
        ("Hat closed", 1, PlayMode::OneShot)
    );
    assert_eq!(
        hat.layers[0].region.frames(),
        (0.08 * RATE as f32) as u64,
        "the whole sound"
    );
    assert_eq!(
        s.ui.get::<ListView>(pads_tab::SOUNDS).unwrap().items[2].detail,
        "80 ms · A11"
    );
    // The new pad plays.
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&s.project, RATE));
    assert!(e.schedule(
        0,
        Event::NoteOn {
            pad: 10,
            velocity: 127
        }
    ));
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
    assert_eq!(
        (p.name.as_str(), r.mode, r.start, r.gain_db),
        (
            "Kick 58",
            PlayMode::Loop,
            0,
            before.layers[0].region.gain_db
        )
    );
    assert!(r
        .sustain_loop
        .is_some_and(|l| l.start > r.start && l.end < r.end));
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
    assert!(s.project.pad(0).unwrap().layers[0]
        .region
        .sustain_loop
        .is_some());
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
    assert!(
        step(&mut s, Row::Sound, -1).is_some(),
        "the last sound goes on"
    );
}

#[test]
fn every_pads_sample_screen_draws() {
    for mut s in pads_samples(Fonts::embedded).unwrap() {
        let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
        s.ui.paint(&mut c);
        assert!(!s.about.is_empty(), "{}", s.name);
    }
}

#[test]
fn map_to_keys_names_the_banks_pads_with_a_sound() {
    let mut s = pads(PAD);
    let want: Vec<usize> = (0..16).filter(|&k| s.state.pads[k].is_some()).collect();
    assert!(want.len() > 1 && want.len() < 16, "{want:?}");
    assert_eq!(
        press(&mut s, Button::MapToKeys),
        Some(PadsAction::MapToKeys { pads: want })
    );
    // The action itself changes nothing: the app finds the notes.
    let rev = s.project.revision();
    apply(&mut s, &PadsAction::MapToKeys { pads: vec![0] });
    assert_eq!(s.project.revision(), rev);
}

#[test]
fn the_effects_row_names_the_pads_effects_and_opens_its_chain() {
    let mut s = pads(PAD);
    assert_eq!(row_value(&s, Row::Effects), "Compressor");
    let index = Row::ALL.iter().position(|r| *r == Row::Effects).unwrap();
    let list = s.ui.get::<SettingList>(pads_tab::LIST).unwrap();
    assert_eq!(list.buttons, [index], "a button, not a setting to step");
    assert_eq!(
        act(&mut s, pads_tab::LIST, EventKind::Tab(index)),
        Some(PadsAction::Effects(PAD))
    );
    assert_eq!(step(&mut s, Row::Effects, 1), None, "nothing to step");
    // A long chain: the first name and how many more.
    s.state.effects = Some(vec!["Compressor".into(), "EQ".into()]);
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert_eq!(row_value(&s, Row::Effects), "Compressor, EQ");
    s.state.effects = Some(vec!["Compressor".into(), "Delay".into(), "Reverb".into()]);
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert_eq!(row_value(&s, Row::Effects), "Compressor +2");
    // No effects yet: + FX.
    let mut e = pads(10);
    assert_eq!(row_value(&e, Row::Effects), "+ FX");
    // Before the mixer says (the app leaves it None): a dash, no button.
    e.state.effects = None;
    let st = e.state.clone();
    e.tab.update(&mut e.ui, &st);
    assert_eq!(row_value(&e, Row::Effects), "—");
    assert!(e
        .ui
        .get::<SettingList>(pads_tab::LIST)
        .unwrap()
        .buttons
        .is_empty());
    assert_eq!(act(&mut e, pads_tab::LIST, EventKind::Tab(index)), None);
}

/// A sliced pad (R4) says how many slices it holds on the map.
#[test]
fn a_sliced_pad_says_its_slices_on_the_map() {
    let mut s = pads(PAD);
    s.project
        .edit_pad(PAD, None, |p| {
            let r = &mut p.layers[0].region;
            let (a, b) = (r.start, r.end);
            r.slices = (1..16).map(|k| a + (b - a) * k / 16).collect();
        })
        .unwrap();
    pick(&mut s, PAD);
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(map.cells[PAD].detail, "16 slices · Loop");
}

/// The Pad list's rows as shown: names and values.
fn rows(s: &PadsSample) -> Vec<(String, String)> {
    s.ui.get::<SettingList>(pads_tab::LIST)
        .unwrap()
        .rows
        .iter()
        .map(|r| (r.name.clone(), r.value.clone()))
        .collect()
}

/// Steps row `row` of the pad picked's type.
fn step_typed(s: &mut PadsSample, row: Row, delta: i32) -> Option<PadsAction> {
    let index = s
        .state
        .picked_type()
        .rows()
        .iter()
        .position(|r| *r == row)?;
    act(s, pads_tab::LIST, EventKind::Step { index, delta })
}

fn switch(s: &mut PadsSample, t: PadType) -> Option<PadsAction> {
    let i = PadType::ALL.iter().position(|x| *x == t).unwrap();
    act(s, pads_tab::TYPE, EventKind::Tab(i))
}

fn footer(s: &PadsSample) -> Option<(String, bool)> {
    s.ui.get::<SettingList>(pads_tab::LIST)
        .unwrap()
        .footer
        .clone()
}

#[test]
fn the_type_switch_makes_a_sample_a_loop_and_back_one_undo_step_each() {
    let mut s = pads(PAD);
    pick(&mut s, PAD);
    let chips = s.ui.get::<SectionChips>(pads_tab::TYPE).unwrap();
    let names: Vec<&str> = chips.chips.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Sample", "Loop", "Instrument", "MIDI out"]);
    assert_eq!(chips.selected, 0);
    let before = s.project.pad(PAD).cloned().unwrap();
    // Loop: latched, a forward loop over the whole sound, warped, on the
    // launch line; still a Sample pad underneath.
    let a = switch(&mut s, PadType::Loop).unwrap();
    assert!(matches!(a, PadsAction::Set { gesture: None, .. }));
    apply(&mut s, &a);
    let p = s.project.pad(PAD).unwrap();
    assert_eq!(PadType::of(p), PadType::Loop);
    assert!(p.latch && p.on_launch && p.kind == PadKind::Sample);
    let r = &p.layers[0].region;
    assert_eq!(r.mode, PlayMode::Loop);
    let l = r.sustain_loop.unwrap();
    assert_eq!(
        (l.start, l.end, l.mode),
        (r.start, r.end, LoopMode::Forward)
    );
    assert!(r.warp.is_some());
    assert_eq!(
        s.ui.get::<SectionChips>(pads_tab::TYPE).unwrap().selected,
        1
    );
    let shown: Vec<String> = rows(&s).into_iter().map(|r| r.0).collect();
    assert_eq!(
        shown,
        [
            "Sound",
            "Tempo",
            "Mute group",
            "Level",
            "Pan",
            "Tune",
            "Effects"
        ]
    );
    assert!(rows(&s)[1].1.ends_with(" BPM") || rows(&s)[1].1.contains(" BPM · "));
    assert!(footer(&s).unwrap().0.starts_with("Loops in time"));
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(map.cells[PAD].badge.as_deref(), Some("LOOP"));
    // The tempo row: a tenth of a BPM a detent; Q-Link 4 turns it.
    let bpm = |s: &PadsSample| {
        s.project.pad(PAD).unwrap().layers[0]
            .region
            .warp
            .as_ref()
            .unwrap()
            .bpm
    };
    let was = bpm(&s);
    let looped = s.project.pad(PAD).cloned().unwrap();
    assert!(
        pads_tab::describe_type(&before, &looped).starts_with("type Sample -> Loop"),
        "the log names the switch"
    );
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 3, 5).unwrap();
    apply(&mut s, &a);
    assert!((bpm(&s) - was - 0.5).abs() < 1e-3, "{} {was}", bpm(&s));
    assert_eq!(
        pads_tab::describe_type(&looped, s.project.pad(PAD).unwrap()),
        format!("tempo {was:.1} -> {:.1} BPM", bpm(&s))
    );
    s.tab.lift();
    // Back to Sample: not on the launch line any more, the rest kept.
    let a = switch(&mut s, PadType::Sample).unwrap();
    apply(&mut s, &a);
    let p = s.project.pad(PAD).unwrap();
    assert_eq!(PadType::of(p), PadType::Sample);
    assert!(!p.on_launch);
    assert_eq!(rows(&s).len(), Row::ALL.len());
    // Each is one undo step: three undos give the pad back as it was.
    for _ in 0..3 {
        assert!(s.project.undo());
    }
    assert_eq!(s.project.pad(PAD), Some(&before));
    // An empty pad has no sound to loop: it says so and does nothing.
    pick(&mut s, 10);
    assert_eq!(switch(&mut s, PadType::Loop), None);
    assert_eq!(
        s.tab.take_note().as_deref(),
        Some("A11 has no sound to loop: Browse for one")
    );
}

#[test]
fn an_instrument_pad_picks_its_engine_and_note() {
    let mut s = pads(PAD);
    pick(&mut s, 10);
    let a = switch(&mut s, PadType::Instrument).unwrap();
    apply(&mut s, &a);
    let p = s.project.pad(10).unwrap();
    assert_eq!(p.name, "Poly", "an empty pad takes the engine's name");
    assert!(p.layers.is_empty());
    assert_eq!(
        p.kind,
        PadKind::Instrument {
            engine: "poly".into(),
            note: 60
        }
    );
    assert_eq!(
        rows(&s),
        [
            ("Engine".to_string(), "Poly".to_string()),
            ("Note".to_string(), "C4".to_string())
        ]
    );
    // The engine steps round the Synth tab's engines; the note by
    // semitones (Q-Link 1).
    let a = step_typed(&mut s, Row::Engine, 1).unwrap();
    apply(&mut s, &a);
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 0, -24).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        s.project.pad(10).unwrap().kind,
        PadKind::Instrument {
            engine: "fm".into(),
            note: 36
        }
    );
    assert_eq!(rows(&s)[0].1, "FM");
    assert_eq!(
        footer(&s),
        Some((
            "A hit plays the Synth tab's FM sound at C2, through the Synth strip on Mix".into(),
            false
        ))
    );
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(
        (
            map.cells[10].detail.as_str(),
            map.cells[10].badge.as_deref()
        ),
        ("FM C2", Some("SYNTH"))
    );
    assert_eq!(PadsTab::qlinks_text(&s.state), "note, engine");
    // Back to Sample with no sound: the pad is empty again.
    s.tab.lift();
    let a = switch(&mut s, PadType::Sample).unwrap();
    assert_eq!(
        a,
        PadsAction::Set {
            pad: 10,
            after: None,
            gesture: None
        }
    );
    // A pad with a sound keeps it as an instrument, and gets it back.
    pick(&mut s, 0);
    let a = switch(&mut s, PadType::Instrument).unwrap();
    apply(&mut s, &a);
    let p = s.project.pad(0).unwrap();
    assert_eq!((p.name.as_str(), p.layers.len()), ("Kick 58", 1));
    let a = switch(&mut s, PadType::Sample).unwrap();
    apply(&mut s, &a);
    assert_eq!(s.project.pad(0).unwrap().kind, PadKind::Sample);
}

#[test]
fn a_midi_out_pad_sends_to_its_own_port_and_says_when_it_would_send_nothing() {
    let mut s = pads(PAD);
    s.state.ports.push(OutPort {
        name: "USB MIDI 2".into(),
        shown: "Synth rack".into(),
        why: Some("sends nothing: Synth rack has no output".into()),
        pads: Vec::new(),
    });
    pick(&mut s, 12);
    let a = switch(&mut s, PadType::MidiOut).unwrap();
    apply(&mut s, &a);
    let p = s.project.pad(12).unwrap();
    // A13's own note, as a kit on a port sends it (the MPC's pad 13).
    assert_eq!(p.name, "Volca Kick", "an empty pad takes the port's name");
    assert_eq!(
        p.kind,
        PadKind::MidiOut {
            port: "Volca Kick".into(),
            channel: 10,
            note: 49
        }
    );
    assert_eq!(
        rows(&s),
        [
            ("Port".to_string(), "Volca Kick".to_string()),
            ("Channel".to_string(), "10".to_string()),
            ("Note".to_string(), "49 · C#3".to_string())
        ]
    );
    assert_eq!(
        footer(&s),
        Some((
            "A hit sends note 49 on channel 10 to Volca Kick".into(),
            false
        ))
    );
    // Channel stops at 16; the port steps to the next, which says why it
    // would send nothing, in place.
    let a = step_typed(&mut s, Row::Channel, 20).unwrap();
    apply(&mut s, &a);
    assert_eq!(rows(&s)[1].1, "16");
    let a = step_typed(&mut s, Row::Port, 1).unwrap();
    apply(&mut s, &a);
    assert_eq!(rows(&s)[0].1, "Synth rack");
    assert_eq!(
        footer(&s),
        Some(("sends nothing: Synth rack has no output".into(), true))
    );
    // A port that isn't plugged in now keeps its name and says so.
    s.state.ports.clear();
    pick(&mut s, 12);
    assert_eq!(rows(&s)[0].1, "USB MIDI 2");
    assert_eq!(
        footer(&s),
        Some(("sends nothing: USB MIDI 2 is not plugged in".into(), true))
    );
    assert_eq!(step_typed(&mut s, Row::Port, 1), None);
    assert_eq!(
        s.tab.take_note().as_deref(),
        Some("No MIDI output to pick: plug one in")
    );
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    assert_eq!(
        (
            map.cells[12].detail.as_str(),
            map.cells[12].badge.as_deref()
        ),
        ("ch 16 · 49", Some("MIDI"))
    );
    // A port whose profile gives the pad a note and channel: the new pad
    // starts on them, as a kit on that port would play it.
    s.state.ports.push(OutPort {
        name: "Volca Kick".into(),
        shown: "Volca Kick".into(),
        why: None,
        pads: vec![(
            13,
            PadOut {
                note: 36,
                channel: Some(1),
            },
        )],
    });
    pick(&mut s, 13);
    let a = switch(&mut s, PadType::MidiOut).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        s.project.pad(13).unwrap().kind,
        PadKind::MidiOut {
            port: "Volca Kick".into(),
            channel: 2,
            note: 36
        }
    );
    pick(&mut s, 12);
    // A sound put on it makes it a Sample pad again.
    let st = s.state.clone();
    s.tab.action(
        &mut s.ui,
        &UiEvent {
            widget: pads_tab::SOUNDS.into(),
            kind: EventKind::Select(2),
        },
        &st,
    );
    let a = act(&mut s, pads_tab::PUT, EventKind::Tab(0)).unwrap();
    apply(&mut s, &a);
    let p = s.project.pad(12).unwrap();
    assert_eq!((p.kind.clone(), p.layers.len()), (PadKind::Sample, 1));
}

#[test]
fn the_types_sample_screen_shows_a_pad_of_each_type() {
    let s = pads_types(Fonts::embedded()).unwrap();
    let map = s.ui.get::<PadMap>(pads_tab::MAP).unwrap();
    let badges: Vec<Option<&str>> = [PAD, 10, 12]
        .iter()
        .map(|&i| map.cells[i].badge.as_deref())
        .collect();
    assert_eq!(badges, [Some("LOOP"), Some("SYNTH"), Some("MIDI")]);
    assert_eq!(map.cells[0].badge, None, "a plain sample has none");
    assert_eq!(
        s.ui.get::<SectionChips>(pads_tab::TYPE).unwrap().selected,
        3
    );
}
