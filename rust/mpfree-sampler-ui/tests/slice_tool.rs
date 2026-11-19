//! The Sample page's Slice tool (once the Chop page): how each method cuts
//! the pad's region, what the hand edits do, and what Chop to pads does to
//! a real project.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{
    ChopMode, ChopSet, Keys, PadsPlay, PlayMode, KEPT_CHOP, SLICE_KEYS_FROM,
};
use mpfree_sampler_ui::panel::{Look, PadCell};
use mpfree_sampler_ui::sample::{self, slice_sample, Sample, BPM, PAD, RATE};
use mpfree_sampler_ui::slice_tool::{self, Button, ChopAction, ChopState, Method, Row};
use mpfree_sampler_ui::wave_tab::{self, Tool, WaveAction, WaveState, WaveTab};
use mpfree_sampler_ui::widgets::{KnobCard, PadBar, SettingList};
use mpfree_sampler_ui::SourceView;
use mpfree_ui::wave::Waveform;
use mpfree_ui::{EventKind, Fonts, SoftCanvas, Touch, Ui, UiEvent};
use std::sync::Arc;

fn chop() -> Sample {
    let mut s = slice_sample(Fonts::embedded()).unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

fn beat() -> f64 {
    f64::from(RATE) * 60.0 / BPM
}

/// What the Slice tool chops on the page's state.
fn cs(st: &WaveState) -> &ChopState {
    st.chop.as_deref().expect("the page's chop")
}

/// The page's state with its chop changed by `f`, as the app makes it.
fn with_chop(st: &mut WaveState, f: impl FnOnce(&mut ChopState)) {
    f(Arc::make_mut(st.chop.as_mut().expect("the page's chop")));
}

/// The page's state with its sound's view `v` (analysed further, say).
fn set_source(st: &mut WaveState, v: Arc<SourceView>) {
    st.source = v.clone();
    with_chop(st, |c| c.source = v);
}

/// Shows the page's state again, as the app does after a change.
fn shown(s: &mut Sample) {
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
}

/// What the Slice tool asked the app for.
fn chopped(a: Option<WaveAction>) -> Option<ChopAction> {
    match a {
        Some(WaveAction::Chop(a)) => Some(a),
        _ => None,
    }
}

/// A touch on the page; what the Slice tool asked the app for.
fn act(s: &mut Sample, ev: &UiEvent) -> Option<ChopAction> {
    let st = s.state.clone();
    chopped(s.tab.action(&mut s.ui, ev, &st))
}

/// Steps a row of the Slice tool's section; what the page said back.
fn step(s: &mut Sample, row: Row, delta: i32) -> Option<WaveAction> {
    let ev = UiEvent {
        widget: wave_tab::REGION.into(),
        kind: EventKind::Step {
            index: row.index(),
            delta,
        },
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

fn method(s: &mut Sample, m: Method) {
    while s.tab.slice().method() != m {
        step(s, Row::Method, 1);
    }
}

fn qlink(s: &mut Sample, knob: usize, delta: i32, fine: bool) {
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, knob, delta, fine);
}

fn press(s: &mut Sample, b: Button) -> Option<ChopAction> {
    let st = s.state.clone();
    chopped(s.tab.press_slice(&mut s.ui, &st, b))
}

/// Applies an action and shows the new state, as the app does.
fn apply(s: &mut Sample, a: &ChopAction) -> std::ops::Range<usize> {
    let pads = a.apply(&mut s.project).unwrap();
    s.state = WaveState::from_project(&s.project, PAD, 0, s.state.source.clone()).unwrap();
    shown(s);
    pads
}

/// A Sample page made afresh on `st` (the app opened again), with the
/// Slice tool.
fn afresh(s: &Sample, st: &WaveState) -> (Ui, WaveTab) {
    let mut ui = sample::app_ui_with(Fonts::embedded(), &s.project, slice_tool::HINT);
    let mut tab = WaveTab::add(&mut ui, sample::PAGE, st).unwrap();
    tab.set_tool(&mut ui, st, Tool::Slice);
    (ui, tab)
}

/// What Split, Merge, Clear and Chop say on the panel, and whether each
/// can be pressed (not faint).
fn buttons(s: &Sample) -> (Vec<String>, Vec<bool>) {
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    Button::ALL
        .iter()
        .map(|b| {
            let k = b.row().index();
            (l.rows[k].value.clone(), !l.controls[k].faint)
        })
        .unzip()
}

/// The pad map's sixteen pads, and whether it is faint.
fn map(s: &Sample) -> (Vec<PadCell>, bool) {
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    let c = &l.controls[Row::Pads.index()];
    match &c.look {
        Look::Pads(cells) => (cells.clone(), c.faint),
        look => panic!("{look:?}"),
    }
}

/// The slices' pads on the lane under the wave, and which are lit.
fn lane(s: &Sample) -> (Vec<String>, Vec<bool>) {
    s.ui.get::<Waveform>(wave_tab::WAVE)
        .unwrap()
        .lane()
        .iter()
        .map(|c| (c.text.clone(), c.lit))
        .unzip()
}

fn list_right(s: &Sample) -> String {
    s.ui.get::<SettingList>(wave_tab::REGION)
        .unwrap()
        .right
        .clone()
}

#[test]
fn transient_chop_cuts_the_pads_region_at_its_hits() {
    let s = chop();
    let st = &s.state;
    // The pad plays from a sixteenth in to the middle of the last beat; the
    // loop has a hit on every eighth.
    let b = s.tab.slice().bounds(cs(st));
    assert_eq!((b[0], *b.last().unwrap()), (st.region.start, st.region.end));
    assert_eq!(s.tab.slice().slices(), 15, "{b:?}");
    for (k, &cut) in b[1..b.len() - 1].iter().enumerate() {
        let eighth = (k + 1) as f64 * beat() / 2.0;
        assert!(
            (cut as f64 - eighth).abs() < 0.01 * f64::from(RATE),
            "cut {k} at {cut}, eighth at {eighth}"
        );
    }
    assert_eq!(s.tab.slice().method(), Method::Transient);
    // Fifteen slices go on the empty pads after A5, on into bank B: A6
    // to B4, A5 (the pad itself) kept.
    assert_eq!(s.tab.slice().pads(cs(st)), Some(5..20));
    assert!(s.tab.slice().replaced(cs(st)).is_empty());
    assert_eq!(list_right(&s), "no pad replaced");
    assert_eq!(
        buttons(&s),
        (
            vec![
                "Split".to_string(),
                "Merge".into(),
                "Clear".into(),
                "Chop to A6–B4".into()
            ],
            vec![true, true, true, true]
        )
    );
    let bar = s.ui.get::<PadBar>(wave_tab::BAR).unwrap();
    assert_eq!(
        (bar.active, bar.title.as_str()),
        (wave_tab::TAB, "Pad A5 · Break 92")
    );
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    let slice = &l.panel.as_ref().unwrap().sections[0];
    assert_eq!(
        (slice.title.as_str(), slice.note.as_str()),
        ("Slice", "15 slices · by transient")
    );
    // Less sensitive never finds more hits.
    let mut s = chop();
    let mut last = usize::MAX;
    for _ in 0..10 {
        step(&mut s, Row::Amount, -1);
        assert!(s.tab.slice().slices() <= last);
        last = s.tab.slice().slices();
    }
    assert!(
        last < 15,
        "at 0 % only the strongest hits cut ({last} slices)"
    );
}

#[test]
fn beat_grid_and_silence_chop_again_as_their_settings_turn() {
    let mut s = chop();
    let (rs, re) = (s.state.region.start, s.state.region.end);
    // Beat: on the source's own beat grid (the bar lines), every beat by default.
    method(&mut s, Method::Beat);
    let b = s.tab.slice().bounds(cs(&s.state));
    assert_eq!(
        s.tab.slice().slices(),
        8,
        "beats 1 to 7 inside the region: {b:?}"
    );
    for (k, &cut) in b[1..b.len() - 1].iter().enumerate() {
        assert!(
            (cut as f64 - (k + 1) as f64 * beat()).abs() <= 256.0,
            "cut {k} at {cut}"
        );
    }
    step(&mut s, Row::Amount, -1);
    assert_eq!(s.tab.slice().slices(), 15, "every 8th");
    qlink(&mut s, 0, 2, false);
    assert_eq!(
        s.tab.slice().slices(),
        4,
        "every 2 beats: cuts on beats 2, 4 and 6"
    );
    // Grid: equal slices of the region, snapped to zero crossings.
    method(&mut s, Method::Grid);
    assert_eq!(s.tab.slice().slices(), 8);
    let b = s.tab.slice().bounds(cs(&s.state));
    for (k, &cut) in b[1..b.len() - 1].iter().enumerate() {
        let even = rs + (re - rs) * (k as u64 + 1) / 8;
        assert!(
            cut.abs_diff(even) <= 256,
            "cut {k} at {cut}, even at {even}"
        );
    }
    qlink(&mut s, 0, -2, false);
    assert_eq!(s.tab.slice().slices(), 6, "Q-Link 3 turns the setting");
    step(&mut s, Row::Snap, 1);
    let b = s.tab.slice().bounds(cs(&s.state));
    assert_eq!(b[1], rs + (re - rs) / 6, "unsnapped, exactly even");
    // Silence: one slice per sound; a lower threshold joins more of them.
    method(&mut s, Method::Silence);
    let sounds = s.tab.slice().slices();
    assert!((2..=15).contains(&sounds), "{sounds}");
    for _ in 0..8 {
        step(&mut s, Row::Amount, -1);
    }
    assert!(s.tab.slice().slices() <= sounds);
    // Manual has no setting; Q-Link 3 does nothing.
    method(&mut s, Method::Manual);
    let before = s.tab.slice().bounds(cs(&s.state));
    qlink(&mut s, 0, 5, false);
    assert_eq!(s.tab.slice().bounds(cs(&s.state)), before);
}

#[test]
fn split_merge_and_moving_a_cut_make_the_chop_manual() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    assert_eq!(s.tab.slice().slices(), 4);
    let mono = s.state.source.mono.clone();
    // Q-Link 1 picks slice 2; Split cuts it near its middle, on a zero crossing.
    qlink(&mut s, 1, 1, false);
    assert_eq!(s.tab.slice().selected(), 1);
    let b = s.tab.slice().bounds(cs(&s.state));
    assert_eq!(press(&mut s, Button::Split), None);
    assert_eq!(
        (s.tab.slice().slices(), s.tab.slice().method()),
        (5, Method::Manual)
    );
    let mid = s.tab.slice().bounds(cs(&s.state))[2];
    assert!(mid.abs_diff((b[1] + b[2]) / 2) <= 256);
    let z = mid as usize;
    assert!(
        mono[z] == 0.0
            || (mono[z - 1] < 0.0) != (mono[z] < 0.0)
            || (mono[z] < 0.0) != (mono[z + 1] < 0.0)
    );
    // Merge joins the slice picked to the one before.
    qlink(&mut s, 1, 1, false);
    assert_eq!(s.tab.slice().selected(), 2);
    press(&mut s, Button::Merge);
    assert_eq!((s.tab.slice().slices(), s.tab.slice().selected()), (4, 1));
    assert_eq!(
        s.tab.slice().bounds(cs(&s.state)),
        b,
        "split then merged is where it began"
    );
    // Q-Link 2 moves the start of the slice picked: one frame with SHIFT.
    let start = s.tab.slice().bounds(cs(&s.state))[1];
    qlink(&mut s, 2, 3, true);
    assert_eq!(s.tab.slice().bounds(cs(&s.state))[1], start + 3);
    qlink(&mut s, 2, -3, true);
    // Unsnapped by hand too, a turn moves a pixel of the zoom per detent.
    qlink(&mut s, 2, 1, false);
    assert!(s.tab.slice().bounds(cs(&s.state))[1] > start);
    // Dragging a cut's marker moves it, but never past its neighbours.
    let st = s.state.clone();
    let ev = UiEvent {
        widget: wave_tab::WAVE.into(),
        kind: EventKind::Marker { index: 2, frame: 0 },
    };
    assert_eq!(act(&mut s, &ev), None);
    let b = s.tab.slice().bounds(cs(&s.state));
    assert_eq!((b[2], s.tab.slice().selected()), (b[1] + 32, 2));
    // The first marker is the pad's Start and the last its End: they stay.
    for index in [0, 4] {
        let ev = UiEvent {
            widget: wave_tab::WAVE.into(),
            kind: EventKind::Marker { index, frame: 1000 },
        };
        act(&mut s, &ev);
        assert_eq!(s.tab.slice().bounds(cs(&s.state)), b);
        assert_eq!(
            s.ui.get::<Waveform>(wave_tab::WAVE).unwrap().markers()[index].frame,
            b[index],
            "put back"
        );
    }
    // A tap on the wave picks the slice under it and plays it.
    let ev = UiEvent {
        widget: wave_tab::WAVE.into(),
        kind: EventKind::Frame(b[3] + 10),
    };
    assert_eq!(
        act(&mut s, &ev),
        Some(ChopAction::Audition {
            pad: st.pad,
            layer: st.layer,
            start: b[3],
            end: b[4]
        })
    );
    assert_eq!(s.tab.slice().selected(), 3);
    // Outside the region there is no slice to play.
    let ev = UiEvent {
        widget: wave_tab::WAVE.into(),
        kind: EventKind::Frame(b[b.len() - 1] + 10),
    };
    assert_eq!(act(&mut s, &ev), None);
    assert_eq!(s.tab.slice().selected(), 3);
    // A hand-made chop stays when the method's list is left alone, even
    // when the region's end moves (another pad chops again).
    let mut moved = s.state.clone();
    moved.region.end -= 1000;
    s.tab.update(&mut s.ui, &moved);
    assert_eq!(
        (
            s.tab.slice().method(),
            s.tab.slice().bounds(cs(&moved))[1..4].to_vec()
        ),
        (Method::Manual, b[1..4].to_vec())
    );
}

#[test]
fn chop_to_pads_puts_each_slice_on_a_pad_in_one_undo_step() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let b = s.tab.slice().bounds(cs(&s.state));
    assert_eq!(
        s.tab.slice().pads(cs(&s.state)),
        Some(5..9),
        "the pads after A5"
    );
    assert_eq!(list_right(&s), "no pad replaced");
    let a = press(&mut s, Button::ToPads).unwrap();
    let ChopAction::ToPads {
        to_pad,
        mode,
        ref bounds,
        ..
    } = a
    else {
        panic!("{a:?}")
    };
    assert_eq!((to_pad, mode, bounds), (5, ChopMode::Grid, &b));
    assert_eq!(apply(&mut s, &a), 5..9);
    let template = s.project.pad(PAD).unwrap().clone();
    for k in 0..4 {
        let pad = s.project.pad(5 + k).unwrap();
        assert_eq!(pad.name, format!("Break 92 {}", k + 1));
        let r = &pad.layers[0].region;
        assert_eq!((r.start, r.end), (b[k], b[k + 1]));
        assert_eq!(
            (r.mode, r.sustain_loop, r.gain_db),
            (PlayMode::OneShot, None, 2.0),
            "plays once, at the pad's gain"
        );
        assert_eq!(pad.level_db, template.level_db);
    }
    assert_eq!(s.project.chop_sets.len(), 1);
    // The new pads play.
    let view = EngineView::build(&s.project, RATE);
    let mut e = Engine::new(RATE, 32);
    e.set_view(view);
    assert!(e.schedule(
        0,
        Event::NoteOn {
            pad: 7,
            velocity: 127
        }
    ));
    let (mut l, mut r) = (vec![0.0; 4096], vec![0.0; 4096]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    assert!(l.iter().any(|x| x.abs() > 0.01), "pad A8 sounds");
    // Now they are on their pads (step 3b: Chop to waits for a change);
    // cut again, they would be replaced. One undo takes the pads and the
    // chop back.
    assert_eq!(list_right(&s), "on their pads");
    assert!(!buttons(&s).1[3]);
    step(&mut s, Row::Amount, 1);
    assert_eq!(list_right(&s), "replaces 4 pads");
    assert!(s.project.undo());
    assert!(s.project.pad(5).is_none() && s.project.chop_sets.is_empty());
    assert!(s.project.pad(PAD).is_some());
}

#[test]
fn one_pad_puts_every_slice_on_the_chopped_pad_played_by_slice_and_by_note() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let b = s.tab.slice().bounds(cs(&s.state));
    let n = s.tab.slice().slices();
    assert_eq!(n, 4);
    let value = |s: &Sample, row: Row| {
        s.ui.get::<SettingList>(wave_tab::REGION).unwrap().rows[row.index()]
            .value
            .clone()
    };
    step(&mut s, Row::Put, 1);
    assert_eq!(value(&s, Row::Put), "One pad");
    // The chopped pad itself, unless a First pad is picked.
    assert_eq!(value(&s, Row::Pads), "A5");
    assert_eq!(buttons(&s).0[3], "Slices on A5");
    assert_eq!(list_right(&s), "replaces A5");
    let a = press(&mut s, Button::ToPads).unwrap();
    let ChopAction::ToPads {
        one_pad,
        keys_from,
        to_pad,
        ..
    } = a
    else {
        panic!("{a:?}")
    };
    assert!(one_pad);
    assert_eq!((keys_from, to_pad), (None, PAD));
    let others: Vec<bool> = (0..16).map(|i| s.project.pad(i).is_some()).collect();
    assert_eq!(apply(&mut s, &a), PAD..PAD + 1);
    let pad = s.project.pad(PAD).unwrap();
    let r = &pad.layers[0].region;
    assert_eq!((r.start, r.end), (b[0], b[n]));
    assert_eq!(r.slice_count(), n);
    for k in 0..n {
        assert_eq!(r.slice_bounds(k), Some((b[k], b[k + 1])));
    }
    assert_eq!(pad.slice, 0, "a hit plays slice 1");
    assert_eq!(
        pad.keys,
        Some(Keys {
            lo: 36,
            hi: 39,
            root: 36
        }),
        "C2 to D#2, a note a slice"
    );
    // No other pad changes, and the pads still play their sounds.
    let now: Vec<bool> = (0..16).map(|i| s.project.pad(i).is_some()).collect();
    assert_eq!(now, others);
    assert_eq!(s.project.pads_play(), PadsPlay::Pads);
    // A hit plays slice 1; D2 (note 38) plays slice 3.
    let play = |p: &mpfree_sampler::model::Project, ev: Event| {
        let mut e = Engine::new(RATE, 32);
        e.set_view(EngineView::build(p, RATE));
        e.apply(ev);
        let (mut l, mut r) = (vec![0.0; 4096], vec![0.0; 4096]);
        for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            e.render(a, b);
        }
        l
    };
    let hit = play(
        &s.project,
        Event::NoteOn {
            pad: PAD as u8,
            velocity: 127,
        },
    );
    assert!(hit.iter().any(|x| x.abs() > 0.01), "a hit sounds");
    let d2 = play(
        &s.project,
        Event::Key {
            note: 38,
            velocity: 127,
        },
    );
    assert!(d2.iter().any(|x| x.abs() > 0.01), "D2 sounds");
    assert_ne!(hit, d2, "another slice");
    // The Chop tab opens on the sliced pad's own slices (the sound has no
    // chop kept).
    let mut fresh = chop();
    assert_ne!(
        fresh.tab.slice().bounds(cs(&fresh.state)),
        b,
        "a fresh chop differs"
    );
    fresh.project = s.project.clone();
    let view = fresh.state.source.clone();
    fresh.state = WaveState::from_project(&fresh.project, PAD, 0, view).unwrap();
    let st = fresh.state.clone();
    fresh.tab.update(&mut fresh.ui, &st);
    assert_eq!(fresh.tab.slice().method(), Method::Manual);
    assert_eq!(fresh.tab.slice().bounds(cs(&fresh.state)), b);
    // A First pad picked moves the one pad.
    qlink(&mut s, 3, 2, false);
    assert_eq!(value(&s, Row::Pads), "A7");
    // One undo gives the pad back as it was, and takes the chop away.
    assert!(s.project.undo());
    let back = s.project.pad(PAD).unwrap();
    assert_eq!(back.layers[0].region.slice_count(), 1);
    assert_eq!(back.keys, None);
    assert!(s.project.chop_sets.is_empty());
}

#[test]
fn phrase_on_one_pad_is_its_slices_in_time() {
    let mut s = chop();
    method(&mut s, Method::Beat);
    step(&mut s, Row::Amount, -1);
    let n = s.tab.slice().slices();
    step(&mut s, Row::Put, 1);
    step(&mut s, Row::Phrase, 1);
    assert_eq!(buttons(&s).0[3], "Phrase on A5");
    let a = press(&mut s, Button::ToPads).unwrap();
    assert!(matches!(
        a,
        ChopAction::ToPads {
            one_pad: true,
            phrase: Some(_),
            ..
        }
    ));
    let p = a.phrase(&s.project).unwrap().unwrap();
    assert_eq!(p.hits.len(), n);
    assert!(p.hits.iter().enumerate().all(|(k, h)| h.slice == k));
    assert_eq!(apply(&mut s, &a), PAD..PAD + 1);
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.slice_count(),
        n
    );
}

#[test]
fn to_keys_puts_each_slice_on_a_note_and_the_pads_play_those_notes() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let b = s.tab.slice().bounds(cs(&s.state));
    step(&mut s, Row::Put, 2);
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert_eq!(list.rows[Row::Put.index()].value, "Keys from C2");
    assert_eq!(buttons(&s).0[3], "To keys C2–D#2");
    let a = press(&mut s, Button::ToPads).unwrap();
    let ChopAction::ToPads { keys_from, .. } = a else {
        panic!("{a:?}")
    };
    assert_eq!(keys_from, Some(SLICE_KEYS_FROM));
    assert_eq!(apply(&mut s, &a), 5..9);
    for k in 0..4 {
        let pad = s.project.pad(5 + k).unwrap();
        assert_eq!(pad.keys, Some(Keys::one(36 + k as u8)), "C2 up");
        let r = &pad.layers[0].region;
        assert_eq!((r.start, r.end), (b[k], b[k + 1]));
    }
    assert_eq!(s.project.pads_play(), PadsPlay::Keys { from: 36 });
    // Pad 1 is now the first slice's note; pad 5 (A5's own) holds no note.
    let view = EngineView::build(&s.project, RATE);
    assert!((0..4).all(|i| view.has_pad(i)) && !view.has_pad(4));
    let mut e = Engine::new(RATE, 32);
    e.set_view(view);
    e.apply(Event::Key {
        note: 37,
        velocity: 127,
    });
    let (mut l, mut r) = (vec![0.0; 4096], vec![0.0; 4096]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    assert!(
        l.iter().any(|x| x.abs() > 0.01),
        "C#2 plays the second slice"
    );
    // One undo takes it all back, the pads playing their sounds again.
    assert!(s.project.undo());
    assert!(s.project.pad(5).is_none() && s.project.chop_sets.is_empty());
    assert_eq!(s.project.pads_play(), PadsPlay::Pads);
}

#[test]
fn phrase_on_also_gives_the_slices_in_time_for_a_clip() {
    let mut s = chop();
    // Every eighth on the loop's own beat grid: the region starts a
    // sixteenth in, so the first slice is a sixteenth long.
    method(&mut s, Method::Beat);
    step(&mut s, Row::Amount, -1);
    assert_eq!(s.tab.slice().slices(), 15);
    let value = |s: &Sample, row: Row| {
        s.ui.get::<SettingList>(wave_tab::REGION).unwrap().rows[row.index()]
            .value
            .clone()
    };
    let list_value = |s: &Sample| value(s, Row::Put);
    let faint = |s: &Sample, row: Row| {
        s.ui.get::<SettingList>(wave_tab::REGION).unwrap().controls[row.index()].faint
    };
    // The loop has a tempo: Phrase can be turned on, on the pads.
    assert_eq!(value(&s, Row::Phrase), "Off");
    assert!(!faint(&s, Row::Phrase));
    step(&mut s, Row::Phrase, 1);
    assert_eq!(value(&s, Row::Phrase), "On");
    assert_eq!(list_value(&s), "Pads");
    assert_eq!(buttons(&s).0[3], "Phrase on A6–B4");
    // Keys can't have one (a phrase on keys needs a track that plays
    // notes): Off and faint, and a tap says why.
    step(&mut s, Row::Put, 2);
    assert_eq!(list_value(&s), "Keys from C2");
    assert_eq!(value(&s, Row::Phrase), "Off");
    assert!(faint(&s, Row::Phrase));
    assert!(matches!(
        step(&mut s, Row::Phrase, 1),
        Some(WaveAction::Say(n)) if n.contains("Pads or One pad")
    ));
    // Back on the pads it is on again.
    step(&mut s, Row::Put, -2);
    assert_eq!(list_value(&s), "Pads");
    assert_eq!(value(&s, Row::Phrase), "On");
    let a = press(&mut s, Button::ToPads).unwrap();
    let ChopAction::ToPads {
        phrase, keys_from, ..
    } = &a
    else {
        panic!("{a:?}")
    };
    // At the loop's detected tempo, a hair off the 92 BPM it was made at.
    assert!(
        phrase.is_some_and(|f| (f - beat()).abs() < 0.5),
        "{phrase:?}"
    );
    assert_eq!(*keys_from, None);
    let p = a.phrase(&s.project).unwrap().unwrap();
    assert_eq!(p.hits.len(), 15);
    assert_eq!(p.hits[0].beat, 0.0);
    assert!((p.hits[0].beats - 0.25).abs() < 0.01, "{:?}", p.hits[0]);
    for (k, h) in p.hits.iter().enumerate().skip(1) {
        assert_eq!(h.slice, k);
        assert!((h.beat - (k as f64 * 0.5 - 0.25)).abs() < 0.01, "{h:?}");
    }
    assert_eq!(p.beats, 8, "7.25 beats of slices round up to 8");
    // The pads are filled as by Chop to pads; the phrase is still there.
    assert_eq!(apply(&mut s, &a), 5..20);
    assert!(s.project.pads_play() == PadsPlay::Pads);
    assert_eq!(a.phrase(&s.project).unwrap(), Some(p));
    // Without a tempo there is no phrase to make: Phrase reads Off.
    let v = &s.state.source;
    let plain = SourceView {
        id: v.id,
        peaks: v.peaks.clone(),
        pyramid: v.pyramid.clone(),
        mono: v.mono.clone(),
        tempo: None,
        root: None,
        root_note: None,
        onsets: v.onsets.clone(),
        pitch: None,
        ready: true,
    };
    set_source(&mut s.state, Arc::new(plain));
    shown(&mut s);
    assert_eq!(list_value(&s), "Pads");
    assert_eq!(value(&s, Row::Phrase), "Off");
    assert!(faint(&s, Row::Phrase));
    assert!(matches!(
        step(&mut s, Row::Phrase, 1),
        Some(WaveAction::Say(n)) if n.contains("no tempo")
    ));
    // Every slice is on its pad and no phrase is asked for: Chop to has
    // nothing to do (it would only undo edits to those pads), and says so.
    assert!(!buttons(&s).1[3]);
    let st = s.state.clone();
    assert_eq!(
        s.tab.press_slice(&mut s.ui, &st, Button::ToPads),
        Some(WaveAction::Say(
            "The slices are on A6–B4 already: move a cut to chop again".into()
        ))
    );
    // A pad on, it chops again, with no phrase.
    qlink(&mut s, 3, 1, false);
    let a = press(&mut s, Button::ToPads).unwrap();
    assert!(matches!(a, ChopAction::ToPads { phrase: None, .. }));
    assert_eq!(a.phrase(&s.project), Ok(None));
    step(&mut s, Row::Put, -1);
    assert_eq!(list_value(&s), "Keys from C2", "Pads, One pad, Keys");
    step(&mut s, Row::Put, 1);
    assert_eq!(list_value(&s), "Pads");
}

#[test]
fn the_first_pad_moves_across_the_kits_banks_and_slices_run_on_past_a16() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    qlink(&mut s, 3, -10, false);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(0..4));
    assert_eq!(list_right(&s), "no pad replaced");
    qlink(&mut s, 3, 2, false);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(2..6));
    assert_eq!(list_right(&s), "replaces A5");
    // Past A16 into bank B, and on to the kit's last pads, H13 to H16.
    qlink(&mut s, 3, 12, false);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(14..18));
    assert_eq!(buttons(&s).0[3], "Chop to A15–B2");
    qlink(&mut s, 3, 200, false);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(124..128));
    step(&mut s, Row::Pads, -1);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(123..127));
    let card = |s: &Sample| {
        let c = s.ui.get::<KnobCard>(wave_tab::QLINKS[3]).unwrap();
        (c.value.clone(), c.sub.clone())
    };
    assert_eq!(card(&s), ("H12".to_string(), "to H15".to_string()));
    // 17 slices, more than a bank, go on from the First pad (held back
    // to fit the kit) and Chop to pads can be pressed.
    for _ in 0..13 {
        step(&mut s, Row::Amount, 1);
    }
    assert_eq!(s.tab.slice().slices(), 17);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(111..128));
    let (labels, enabled) = buttons(&s);
    assert_eq!((labels[3].as_str(), enabled[3]), ("Chop to G16–H16", true));
    qlink(&mut s, 3, -200, false);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(0..17));
    assert_eq!(list_right(&s), "replaces A5");
    let a = press(&mut s, Button::ToPads).unwrap();
    assert_eq!(apply(&mut s, &a), 0..17);
    assert!(s.project.pad(16).is_some(), "B1 holds the 17th slice");
    // Another pad of the kit starts again from the empty pads after it.
    let mut b1 = s.state.clone();
    with_chop(&mut b1, |c| c.pad = 16);
    let n = s.tab.slice().slices();
    assert_eq!(s.tab.slice().pads(cs(&b1)), Some(17..17 + n), "from B2");
}

#[test]
fn every_slice_sample_screen_draws() {
    for mut s in sample::slice_samples(Fonts::embedded).unwrap() {
        let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
        assert!(!s.ui.paint(&mut c).is_empty(), "{}", s.name);
    }
}

/// Renders `frames` frames of `e` after scheduling `ev`.
fn sound(e: &mut Engine, ev: Event, frames: usize) -> Vec<f32> {
    assert!(e.schedule(0, ev));
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l.into_iter().zip(r).flat_map(|(a, b)| [a, b]).collect()
}

#[test]
fn a_tapped_slice_sounds_as_the_pad_made_from_it_will() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let b = s.tab.slice().bounds(cs(&s.state));
    let ev = UiEvent {
        widget: wave_tab::WAVE.into(),
        kind: EventKind::Frame(b[2] + 5),
    };
    let tap = act(&mut s, &ev).unwrap();
    let ChopAction::Audition {
        pad,
        layer,
        start,
        end,
    } = tap
    else {
        panic!("{tap:?}")
    };
    assert_eq!((pad, layer, start, end), (PAD, 0, b[2], b[3]));
    // The tap, before anything goes on pads...
    let frames = 4096;
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&s.project, RATE));
    let heard = sound(
        &mut e,
        Event::Audition {
            pad: pad as u8,
            layer: layer as u8,
            start: start as u32,
            end: end as u32,
            velocity: 100,
        },
        frames,
    );
    // ...sounds as the slice's pad, once it is made.
    let a = press(&mut s, Button::ToPads).unwrap();
    let pads = apply(&mut s, &a);
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&s.project, RATE));
    let made = sound(
        &mut e,
        Event::NoteOn {
            pad: (pads.start + 2) as u8,
            velocity: 100,
        },
        frames,
    );
    assert!(heard.iter().any(|x| x.abs() > 0.01), "the tap plays");
    for (k, (h, m)) in heard.iter().zip(&made).enumerate() {
        assert!((h - m).abs() < 1e-6, "sample {k}: {h} vs {m}");
    }
}

#[test]
fn put_on_files_exports_the_pad_and_its_slices_where_the_app_can_write() {
    let mut s = chop();
    let value = |s: &Sample, row: Row| {
        let i = Row::ALL.iter().position(|r| *r == row).unwrap();
        s.ui.get::<SettingList>(wave_tab::REGION).unwrap().rows[i]
            .value
            .clone()
    };
    // Without files: Pads, One pad, Keys and round again.
    for _ in 0..3 {
        step(&mut s, Row::Put, 1);
        assert_ne!(value(&s, Row::Put), "Files");
    }
    assert_eq!(value(&s, Row::Put), "Pads");
    // With them, Files comes after Keys.
    with_chop(&mut s.state, |c| c.files = true);
    shown(&mut s);
    let st = s.state.clone();
    step(&mut s, Row::Put, -1);
    assert_eq!(value(&s, Row::Put), "Files");
    assert_eq!(value(&s, Row::Pads), "None");
    let n = s.tab.slice().slices();
    assert!(n > 1);
    let (labels, enabled) = buttons(&s);
    assert_eq!(labels[3], format!("Export {n} slices"));
    assert!(enabled[3]);
    let cuts = s.tab.slice().bounds(cs(&st))[1..n].to_vec();
    assert_eq!(
        press(&mut s, Button::ToPads),
        Some(ChopAction::Export {
            pad: PAD,
            layer: 0,
            cuts
        })
    );
    // One slice is the pad itself.
    method(&mut s, Method::Grid);
    for _ in 0..8 {
        step(&mut s, Row::Amount, -1);
    }
    while s.tab.slice().slices() > 1 {
        press(&mut s, Button::Merge);
    }
    assert_eq!(buttons(&s).0[3], "Export the pad");
    // The files gone again, Put on is back on Pads.
    with_chop(&mut s.state, |c| c.files = false);
    shown(&mut s);
    assert_eq!(value(&s, Row::Put), "Pads");
    assert!(matches!(
        press(&mut s, Button::ToPads),
        Some(ChopAction::ToPads { .. })
    ));
}

#[test]
fn live_chop_drops_cuts_where_pads_are_played_snapped_to_hits() {
    let mut s = chop();
    let st = s.state.clone();
    let transients = s.tab.slice().bounds(cs(&st));
    // Transient steps back round to Live, which keeps the cuts.
    step(&mut s, Row::Method, -1);
    assert_eq!(s.tab.slice().method(), Method::Live);
    assert_eq!(
        s.tab.slice().bounds(cs(&st)),
        transients,
        "picking Live keeps the cuts"
    );
    let (labels, enabled) = buttons(&s);
    assert_eq!(labels[0], "Start");
    assert!(enabled[0]);
    // Start: the region plays once and the old cuts go.
    let a = press(&mut s, Button::Split).unwrap();
    assert_eq!(
        a,
        ChopAction::Live {
            pad: PAD,
            layer: 0,
            start: st.region.start,
            end: st.region.end,
            on: true
        }
    );
    assert!(s.tab.slice().live());
    assert_eq!(s.tab.slice().slices(), 1);
    assert_eq!(buttons(&s).0[0], "Stop");
    // Each mark moves onto the nearest hit within 40 ms; a mark far from
    // any hit stays where it is (on a zero crossing); one outside the
    // region or too near a cut is dropped.
    let hits = st.source.onsets.as_ref().unwrap().pick(0.6);
    let inside: Vec<u64> = hits
        .iter()
        .copied()
        .filter(|&h| h > st.region.start + 2000 && h + 2000 < st.region.end)
        .collect();
    let (h1, h2) = (inside[0], inside[2]);
    let ms = u64::from(RATE) / 1000;
    let lone = (h1 + inside[1]) / 2;
    let kept = s.tab.live_marks(
        &mut s.ui,
        &st,
        &[
            h1 + 25 * ms,
            h2 - 10 * ms,
            h1 + 26 * ms,
            st.region.end + 100,
        ],
    );
    assert_eq!(kept, 2, "the third is the first again, the fourth past End");
    assert_eq!(s.tab.slice().bounds(cs(&st))[1..3], [h1, h2]);
    assert_eq!(
        s.tab.slice().selected(),
        2,
        "the slice the newest cut starts"
    );
    // Pull to hits Off and zero-crossing Snap Off: a cut lands where the
    // sound was.
    step(&mut s, Row::Amount, -4);
    step(&mut s, Row::Snap, 1);
    assert_eq!(s.tab.live_marks(&mut s.ui, &st, &[lone]), 1);
    assert!(s.tab.slice().bounds(cs(&st)).contains(&lone));
    assert_eq!(s.tab.slice().method(), Method::Live);
    // Merge takes one out and stays Live; Stop ends the take.
    press(&mut s, Button::Merge);
    assert_eq!(s.tab.slice().method(), Method::Live);
    assert_eq!(s.tab.slice().slices(), 3);
    let a = press(&mut s, Button::Split).unwrap();
    assert!(matches!(a, ChopAction::Live { on: false, .. }));
    assert!(!s.tab.slice().live());
    assert_eq!(
        s.tab.live_marks(&mut s.ui, &st, &[lone + 5000]),
        0,
        "not live"
    );
    assert_eq!(buttons(&s).0[0], "Start");
}

#[test]
fn live_chop_ends_with_its_sound_or_another_method() {
    let mut s = chop();
    let st = s.state.clone();
    method(&mut s, Method::Live);
    press(&mut s, Button::Split);
    s.tab.live_ended(&mut s.ui, &st);
    assert!(!s.tab.slice().live());
    assert_eq!(buttons(&s).0[0], "Start");
    press(&mut s, Button::Split);
    // Another method while live: the step tells the app to stop.
    let index = Row::ALL.iter().position(|r| *r == Row::Method).unwrap();
    let ev = UiEvent {
        widget: wave_tab::REGION.into(),
        kind: EventKind::Step { index, delta: 1 },
    };
    let a = act(&mut s, &ev);
    assert!(
        matches!(a, Some(ChopAction::Live { on: false, .. })),
        "{a:?}"
    );
    assert!(!s.tab.slice().live());
    assert_eq!(s.tab.slice().method(), Method::Transient);
    assert_eq!(buttons(&s).0[0], "Split");
}

#[test]
fn a_sound_still_being_analysed_chops_by_its_hits_once_they_are_found() {
    let mut s = chop();
    let full = s.state.source.clone();
    let source = s.project.source(full.id).unwrap().clone();
    // Its hits not found yet: no cuts, and Transient stays the method.
    let mut st = s.state.clone();
    set_source(&mut st, Arc::new(SourceView::quick(&source)));
    s.tab.update(&mut s.ui, &st);
    assert_eq!(
        (s.tab.slice().slices(), s.tab.slice().method()),
        (1, Method::Transient)
    );
    // They come in: the tool chops again at them.
    set_source(&mut st, full);
    s.tab.update(&mut s.ui, &st);
    assert_eq!(s.tab.slice().slices(), 15);
}

#[test]
fn a_changed_chop_is_kept_for_its_sound_and_comes_back() {
    let mut s = chop();
    let st = s.state.clone();
    assert!(cs(&st).kept.is_none());
    assert_eq!(
        s.tab.take_kept(&st),
        None,
        "the tab's own first chop is nothing to keep"
    );
    // Grid, then a slice split by hand: Manual.
    method(&mut s, Method::Grid);
    assert_eq!(press(&mut s, Button::Split), None);
    assert_eq!(s.tab.slice().method(), Method::Manual);
    let bounds = s.tab.slice().bounds(cs(&st));
    let (mode, cuts) = s.tab.take_kept(&st).expect("a changed chop");
    assert_eq!(mode, ChopMode::Manual);
    assert_eq!(cuts, bounds[1..bounds.len() - 1]);
    assert_eq!(s.tab.take_kept(&st), None, "told once");
    // The app keeps it for the sound.
    let src = s.project.source(st.source.id).unwrap().clone();
    let kept = ChopSet::from_markers(KEPT_CHOP, &src, mode, cuts.clone()).unwrap();
    s.project.keep_chop(kept).unwrap();
    let st = WaveState::from_project(&s.project, PAD, 0, st.source.clone()).unwrap();
    assert_eq!(cs(&st).kept.as_ref().map(|k| k.markers()), Some(&cuts[..]));
    // A page made afresh (the app opened again) shows the kept slices, not
    // a fresh Transient chop, and has nothing new to keep.
    let (mut ui, mut again) = afresh(&s, &st);
    assert_eq!(again.slice().method(), Method::Manual);
    assert_eq!(again.slice().bounds(cs(&st)), bounds);
    assert_eq!(again.take_kept(&st), None);
    // Start moved later with Trim, and back: the kept slices inside the
    // region show each time, none lost.
    let mut moved = st.clone();
    // Three quarters of a beat later: clear of every cut by far more than
    // the shortest slice.
    let later = (beat() * 0.75) as u64;
    moved.region.start += later;
    with_chop(&mut moved, |c| c.region.start += later);
    again.update(&mut ui, &moved);
    assert_eq!(again.slice().method(), Method::Manual);
    let inside: Vec<u64> = cuts
        .iter()
        .copied()
        .filter(|&c| c > moved.region.start)
        .collect();
    assert!(inside.len() < cuts.len());
    let b = again.slice().bounds(cs(&moved));
    assert_eq!(&b[1..b.len() - 1], &inside[..]);
    again.update(&mut ui, &st);
    assert_eq!(again.slice().bounds(cs(&st)), bounds);
}

/// A finger's tap on the panel's control labelled `label`; what the tab
/// makes of it.
fn tap_panel(s: &mut Sample, label: &str) -> Option<ChopAction> {
    chopped(tap_says(s, label))
}

/// As [`tap_panel`], all the tab says back (a note, say).
fn tap_says(s: &mut Sample, label: &str) -> Option<WaveAction> {
    let t =
        s.ui.targets()
            .into_iter()
            .find(|t| t.widget == wave_tab::REGION && t.target.label == label)
            .unwrap_or_else(|| panic!("no \"{label}\""))
            .target;
    let (x, y) = t.centre();
    let ev = s.ui.touch(5, Touch::Down, x, y)?;
    s.ui.touch(5, Touch::Up, x, y);
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

#[test]
fn the_slice_tool_has_the_slice_and_put_on_sections() {
    let mut s = chop();
    let wave = s.ui.rect_of(wave_tab::WAVE).unwrap();
    let band = s.ui.rect_of(wave_tab::REGION).unwrap();
    // The band under the wave, up to the Sound section.
    assert_eq!((band.x, band.h), (wave.x, 242));
    assert!(band.w < wave.w);
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    let panel = l.panel.as_ref().unwrap();
    let titles: Vec<&str> = panel.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Slice", "Put on"]);
    let placed: Vec<usize> = panel
        .place(band, &l.controls)
        .iter()
        .map(|p| p.row)
        .collect();
    assert_eq!(
        placed,
        (0..Row::ALL.len()).collect::<Vec<_>>(),
        "the wheel's order"
    );
    // The method's arrows chop with the next one at once: Beat, then Grid.
    assert_eq!(tap_panel(&mut s, "Method ›"), None);
    assert_eq!(s.tab.slice().method(), Method::Beat);
    assert_eq!(tap_panel(&mut s, "Method ›"), None);
    assert_eq!(s.tab.slice().method(), Method::Grid);
    assert_eq!(s.tab.slice().slices(), 8);
    // Split on the panel adds a slice by hand.
    assert_eq!(tap_panel(&mut s, "Split"), None);
    assert_eq!(
        (s.tab.slice().method(), s.tab.slice().slices()),
        (Method::Manual, 9)
    );
    // Manual has no setting: its knob is faint.
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert!(l.controls[Row::Amount.index()].faint);
    // The main button names the pads, and puts the slices there.
    let (labels, on) = buttons(&s);
    assert_eq!((labels[3].as_str(), on[3]), ("Chop to A6–A14", true));
    let a = tap_panel(&mut s, "Chop to A6–A14");
    assert!(matches!(a, Some(ChopAction::ToPads { .. })), "{a:?}");
}

#[test]
fn pitch_chops_a_sung_line_where_its_notes_change() {
    let mut s = chop();
    let (project, followed) = sample::melody_project();
    let (_, starts) = sample::melody();
    s.project = project;
    // Its view as it first comes, the pitch not followed.
    let bare = Arc::new(SourceView {
        pitch: None,
        ..(*followed).clone()
    });
    s.state = WaveState::from_project(&s.project, PAD, 0, bare).unwrap();
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert!(!s.tab.wants_pitch(&st), "only for Pitch");
    method(&mut s, Method::Pitch);
    // Pitch waits for the sound's pitch: no cuts, and nothing to keep.
    assert!(s.tab.wants_pitch(&st));
    assert_eq!(s.tab.slice().slices(), 1);
    assert_eq!(s.tab.take_kept(&st), None);
    // It comes in (the app follows it off the screen thread): the chop.
    set_source(&mut s.state, followed);
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert!(!s.tab.wants_pitch(&st));
    let value = |s: &Sample| {
        s.ui.get::<SettingList>(wave_tab::REGION).unwrap().rows[Row::Amount.index()]
            .value
            .clone()
    };
    // Every change of a semitone or more, and the G after the rest: each
    // cut within 5 ms of its note's start, on a zero crossing.
    let near = |s: &Sample, want: &[usize]| {
        let b = s.tab.slice().bounds(cs(&s.state));
        let cuts = &b[1..b.len() - 1];
        assert_eq!(cuts.len(), want.len(), "cuts {cuts:?}, notes {starts:?}");
        for (&c, &k) in cuts.iter().zip(want) {
            assert!(
                c.abs_diff(starts[k]) <= u64::from(RATE) / 200,
                "{c} for note {k}"
            );
        }
    };
    assert_eq!(value(&s), "1 semi");
    near(&s, &[1, 2, 4, 5]);
    let mono = &s.state.source.mono;
    let b = s.tab.slice().bounds(cs(&s.state));
    for &c in &b[1..b.len() - 1] {
        let (x, y) = (mono[c as usize - 1], mono[c as usize]);
        assert!(
            x * y <= 0.0 || x.abs() < 1e-3,
            "{c} isn't on a zero crossing"
        );
    }
    // Four semitones or more: E to G (three) is no cut.
    qlink(&mut s, 0, 3, false);
    assert_eq!(value(&s), "4 semis");
    near(&s, &[1, 4, 5]);
    // An octave: only the note after the rest.
    for _ in 0..10 {
        step(&mut s, Row::Amount, 1);
    }
    assert_eq!(value(&s), "Octave");
    near(&s, &[4]);
    // Kept as a Pitch chop, and a tab made afresh opens on it.
    let (mode, cuts) = s.tab.take_kept(&s.state).expect("a changed chop");
    assert_eq!(mode, ChopMode::Pitch);
    let src = s.project.source(s.state.source.id).unwrap().clone();
    let kept = ChopSet::from_markers(KEPT_CHOP, &src, mode, cuts.clone()).unwrap();
    s.project.keep_chop(kept).unwrap();
    let st = WaveState::from_project(&s.project, PAD, 0, s.state.source.clone()).unwrap();
    let (_ui, again) = afresh(&s, &st);
    assert_eq!(again.slice().method(), Method::Pitch);
    let b = again.slice().bounds(cs(&st));
    assert_eq!(&b[1..b.len() - 1], &cuts[..]);
    // The drum loop has few notes to follow: few cuts.
    let mut d = chop();
    let v = d.state.source.clone();
    set_source(
        &mut d.state,
        Arc::new(v.with_pitch(v.follow_pitch(RATE).unwrap())),
    );
    method(&mut d, Method::Pitch);
    assert!(
        d.tab.slice().slices() <= 3,
        "{:?}",
        d.tab.slice().bounds(cs(&d.state))
    );
}

#[test]
fn put_on_new_kit_chops_onto_an_empty_kit_for_a_new_track_in_one_undo_step() {
    use mpfree_sampler::model::Project;
    let mut s = chop();
    let value = |s: &Sample, row: Row| {
        s.ui.get::<SettingList>(wave_tab::REGION).unwrap().rows[row.index()]
            .value
            .clone()
    };
    // Until the app offers it, Put on has no New kit: Pads, One pad,
    // Keys and round again.
    for _ in 0..3 {
        step(&mut s, Row::Put, 1);
        assert_ne!(value(&s, Row::Put), "New kit");
    }
    // Offered, it comes after Keys.
    with_chop(&mut s.state, |c| c.new_kit = Some(Ok(())));
    shown(&mut s);
    assert_eq!(value(&s, Row::Put), "Pads");
    for _ in 0..3 {
        step(&mut s, Row::Put, 1);
    }
    assert_eq!(value(&s, Row::Put), "New kit");
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let n = s.tab.slice().slices();
    let b = s.tab.slice().bounds(cs(&s.state));
    assert_eq!(
        s.tab.slice().pads(cs(&s.state)),
        Some(0..n),
        "the new kit's pads from A1"
    );
    // The pad map shows them there, faint (it picks nothing), none with a
    // sound yet; the lane names them.
    let (cells, faint) = map(&s);
    assert!(faint);
    assert!(cells.iter().all(|c| !c.used));
    assert_eq!(
        cells.iter().map(|c| c.tint).collect::<Vec<_>>(),
        (0..16).map(|k| (k < n).then_some(k)).collect::<Vec<_>>()
    );
    assert_eq!(lane(&s).0.len(), n);
    // Q-Link 4 (First pad) has nothing to move: they start on A1.
    qlink(&mut s, 3, 2, false);
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(0..n));
    let q4 = s.ui.get::<KnobCard>(wave_tab::QLINKS[3]).unwrap();
    assert!(!q4.enabled && q4.why == "On a new kit the slices start on A1");
    assert!(s.tab.slice().replaced(cs(&s.state)).is_empty());
    assert_eq!(list_right(&s), "on a new track");
    let (labels, enabled) = buttons(&s);
    assert_eq!((labels[3].as_str(), enabled[3]), ("Chop to new kit", true));
    // With a phrase, the button says so.
    step(&mut s, Row::Phrase, 1);
    assert_eq!(buttons(&s).0[3], "Phrase to new kit");
    let a = press(&mut s, Button::ToPads).unwrap();
    let ChopAction::ToPads {
        to_pad,
        new_kit,
        one_pad,
        keys_from,
        phrase,
        ..
    } = a
    else {
        panic!("{a:?}")
    };
    assert_eq!(
        (to_pad, new_kit, one_pad, keys_from),
        (0, true, false, None)
    );
    assert!(phrase.is_some());
    // The slices go on the empty kit with the sound, and the chopped pad's
    // kit stays as it was.
    let kit = |p: &Project| {
        let pads: Vec<_> = (0..128).map(|i| p.pad(i).cloned()).collect();
        (pads, p.chop_sets.len(), p.history())
    };
    let before = kit(&s.project);
    let mut to = Project::new();
    assert_eq!(a.apply_to(&s.project, &mut to), Ok(0..n));
    assert!(
        kit(&s.project) == before,
        "the chopped pad's kit doesn't change"
    );
    let template = s.project.pad(PAD).unwrap().clone();
    let id = template.layers[0].region.source;
    assert!(to.source(id).is_some(), "the sound came with the slices");
    for k in 0..n {
        let pad = to.pad(k).unwrap();
        assert_eq!(pad.name, format!("Break 92 {}", k + 1));
        let r = &pad.layers[0].region;
        assert_eq!((r.source, r.start, r.end), (id, b[k], b[k + 1]));
        assert_eq!(pad.level_db, template.level_db);
    }
    assert!(to.pad(n).is_none());
    // They play.
    let mut e = Engine::new(RATE, 32);
    e.set_view(EngineView::build(&to, RATE));
    assert!(e.schedule(
        0,
        Event::NoteOn {
            pad: 1,
            velocity: 127
        }
    ));
    let (mut l, mut r) = (vec![0.0; 4096], vec![0.0; 4096]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    assert!(
        l.iter().any(|x| x.abs() > 0.01),
        "pad A2 of the new kit sounds"
    );
    // One undo takes the pads, the chop and the sound back out.
    assert!(to.undo());
    assert!(to.pad(0).is_none() && to.chop_sets.is_empty() && to.source(id).is_none());
    // Not now: the button is faint and says why; the note says so too.
    let why = "No empty kit is left for a new track: chop to this kit's pads";
    with_chop(&mut s.state, |c| c.new_kit = Some(Err(why.to_string())));
    shown(&mut s);
    assert_eq!(value(&s, Row::Put), "New kit");
    assert!(!buttons(&s).1[3]);
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert_eq!(l.controls[Row::Chop.index()].why, why);
    assert_eq!(list_right(&s), "not now");
    assert_eq!(press(&mut s, Button::ToPads), None);
    // A phrase goes with a new kit, not keys.
    step(&mut s, Row::Put, -1);
    assert!(value(&s, Row::Put).starts_with("Keys"));
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert_eq!(
        l.controls[Row::Phrase.index()].why,
        "A phrase goes with Pads, One pad or New kit, not keys or files"
    );
    // No longer offered, Put on is back on Pads.
    step(&mut s, Row::Put, 1);
    with_chop(&mut s.state, |c| c.new_kit = None);
    shown(&mut s);
    assert_eq!(value(&s, Row::Put), "Pads");
}

/// One-page step 3b: the lane under the wave names the pad each slice
/// lands on, lit once that pad plays it; the pad map shows them on the
/// bank in their colours, a tap on a pad starts them there and its arrows
/// turn the bank. Once they are on, the Sound section follows the slice
/// picked to its pad, until it is cut again.
#[test]
fn the_lane_and_the_pad_map_show_where_each_slice_lands() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let b = s.tab.slice().bounds(cs(&s.state));
    let lands = s.tab.slice().lands(cs(&s.state));
    let spans: Vec<(u64, u64)> = lands.iter().map(|l| (l.start, l.end)).collect();
    assert_eq!(
        spans,
        b.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()
    );
    assert_eq!(
        lane(&s),
        (
            vec!["A6".into(), "A7".into(), "A8".into(), "A9".into()],
            vec![false; 4]
        ),
        "the pads after A5, none playing its slice yet"
    );
    let (cells, faint) = map(&s);
    assert!(!faint);
    assert_eq!(cells.len(), 16);
    for (pad, c) in cells.iter().enumerate() {
        let k = (5..9).contains(&pad).then(|| pad - 5);
        assert_eq!(c.tint, k, "pad {pad}");
        assert_eq!(c.text, k.map_or(String::new(), |k| (k + 1).to_string()));
        assert_eq!(c.used, pad == PAD, "pad {pad}");
        assert_eq!(c.picked, k == Some(s.tab.slice().selected()));
    }
    assert_eq!(s.tab.sound_follows(&s.state), None, "not on pads yet");
    // A tap on pad 13 starts them there.
    tap_panel(&mut s, "Pads: pad 13");
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(12..16));
    assert_eq!(lane(&s).0, ["A13", "A14", "A15", "A16"]);
    // Tapped again, there is nothing to move: it says so.
    assert_eq!(
        tap_says(&mut s, "Pads: pad 13"),
        Some(WaveAction::Say("Slice 1 lands on A13 already".into()))
    );
    // The arrows go round the banks, as Method's do.
    tap_panel(&mut s, "Map ‹");
    assert_eq!(s.tab.slice().bank_shown(cs(&s.state)), 7, "bank H");
    tap_panel(&mut s, "Map ›");
    // The arrows turn the map (not the pads' bank) to B1-B16, which have
    // none; a tap on its pad 1 starts them on B1.
    let value = |s: &Sample| {
        s.ui.get::<SettingList>(wave_tab::REGION).unwrap().rows[Row::Bank.index()]
            .value
            .clone()
    };
    assert_eq!(value(&s), "A1–A16");
    tap_panel(&mut s, "Map ›");
    assert_eq!(value(&s), "B1–B16");
    assert!(map(&s).0.iter().all(|c| c.tint.is_none() && !c.used));
    tap_panel(&mut s, "Pads: pad 1");
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(16..20));
    assert_eq!(lane(&s).0, ["B1", "B2", "B3", "B4"]);
    assert_eq!(map(&s).0[0].tint, Some(0));
    // Chopped there, each slice's pad plays it: the lane lights and the
    // map shows them raised.
    let a = press(&mut s, Button::ToPads).unwrap();
    assert_eq!(apply(&mut s, &a), 16..20);
    assert_eq!(lane(&s), (lane(&s).0, vec![true; 4]));
    assert_eq!(lane(&s).0, ["B1", "B2", "B3", "B4"], "the same pads again");
    assert!(map(&s).0[..4].iter().all(|c| c.used));
    for k in 0..4 {
        assert_eq!(s.tab.slice().slice_pad(cs(&s.state), k), Some(16 + k));
    }
    // The Sound section follows the slice picked to its pad.
    let k = s.tab.slice().selected();
    assert_eq!(s.tab.sound_follows(&s.state), Some((k, 16 + k)));
    qlink(&mut s, 1, 1, false);
    let k2 = s.tab.slice().selected();
    assert_ne!(k, k2);
    assert_eq!(s.tab.sound_follows(&s.state), Some((k2, 16 + k2)));
    // Cut again, the slices from the one cut on land on pads that play
    // something else: unlit, and the Sound section stays on the pad.
    assert_eq!(press(&mut s, Button::Split), None);
    let on = lane(&s).1;
    assert_eq!(on.len(), 5);
    assert!(on[..k2].iter().all(|&o| o), "{on:?}");
    assert!(on[k2..].iter().all(|&o| !o), "{on:?}");
    assert_eq!(s.tab.sound_follows(&s.state), None);
}

/// One-page step 3b: a take kept on the page is chopped onto pads straight
/// from the page (from A1, as it has no pad), each slice made from the
/// take; the take leaves the page with them, all in one undo step.
#[test]
fn a_takes_slices_go_on_pads_from_the_page_in_one_undo_step() {
    let mut s = chop();
    let mut take = s.project.pad(PAD).unwrap().clone();
    take.name = "Take 1".into();
    take.level_db = -3.0;
    assert_eq!(s.project.add_take(take.clone()), Ok(0));
    s.state = WaveState::of_take(&s.project, 0, 0, s.state.source.clone()).unwrap();
    shown(&mut s);
    assert_eq!(cs(&s.state).pad, wave_tab::TAKE);
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let b = s.tab.slice().bounds(cs(&s.state));
    assert_eq!(s.tab.slice().pads(cs(&s.state)), Some(0..4), "from A1");
    assert_eq!(lane(&s).0, ["A1", "A2", "A3", "A4"]);
    let (labels, on) = buttons(&s);
    assert_eq!((labels[3].as_str(), on[3]), ("Chop to A1–A4", true));
    let a = press(&mut s, Button::ToPads).unwrap();
    // The take isn't a pad: apply has nothing to make them from.
    assert!(a.apply(&mut s.project).is_err());
    assert_eq!(a.apply_take(&mut s.project, 0), Ok(0..4));
    assert!(s.project.takes().is_empty(), "the take left the page");
    for k in 0..4 {
        let pad = s.project.pad(k).unwrap();
        assert_eq!(pad.name, format!("Take 1 {}", k + 1));
        let r = &pad.layers[0].region;
        assert_eq!((r.start, r.end), (b[k], b[k + 1]));
        assert_eq!(pad.level_db, -3.0, "made from the take");
    }
    // One undo: the take back, the pads empty.
    assert!(s.project.undo());
    assert_eq!(s.project.takes().len(), 1);
    assert_eq!(s.project.takes()[0].name, "Take 1");
    assert!((0..4).all(|k| s.project.pad(k).is_none()));
    assert!(s.project.pad(PAD).is_some());
    // On a new kit: the slices made from the take there, with its sound;
    // this kit as it was (the app lets the take go).
    let mut to = mpfree_sampler::model::Project::new();
    assert_eq!(a.apply_take_to(&s.project, 0, &mut to), Ok(0..4));
    let id = take.layers[0].region.source;
    assert!(to.source(id).is_some(), "the sound came with the slices");
    for k in 0..4 {
        let pad = to.pad(k).unwrap();
        assert_eq!(pad.name, format!("Take 1 {}", k + 1));
        assert_eq!(pad.level_db, -3.0);
    }
    assert_eq!(s.project.takes().len(), 1);
}
