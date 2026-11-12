//! The Chop tab: how each method cuts the pad's region, what the hand edits
//! do, and what Chop to pads does to a real project.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{ChopMode, Keys, PadsPlay, PlayMode, SLICE_KEYS_FROM};
use mpfree_sampler_ui::chop_tab::{self, Button, ChopAction, ChopState, Method, Row};
use mpfree_sampler_ui::sample::{self, chop_sample, ChopSample, BPM, PAD, RATE};
use mpfree_sampler_ui::wave_tab;
use mpfree_sampler_ui::widgets::{ButtonRow, PadBar, SettingList};
use mpfree_sampler_ui::SourceView;
use mpfree_ui::wave::Waveform;
use mpfree_ui::{EventKind, Fonts, SoftCanvas, Touch, UiEvent};
use std::sync::Arc;

fn chop() -> ChopSample {
    let mut s = chop_sample(Fonts::embedded()).unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

fn beat() -> f64 {
    f64::from(RATE) * 60.0 / BPM
}

fn step(s: &mut ChopSample, row: Row, delta: i32) {
    let index = Row::ALL.iter().position(|r| *r == row).unwrap();
    let ev = UiEvent {
        widget: chop_tab::LIST.into(),
        kind: EventKind::Step { index, delta },
    };
    let st = s.state.clone();
    assert_eq!(
        s.tab.action(&mut s.ui, &ev, &st),
        None,
        "the list changes only the chop"
    );
}

fn method(s: &mut ChopSample, m: Method) {
    while s.tab.method() != m {
        step(s, Row::Method, 1);
    }
}

fn qlink(s: &mut ChopSample, knob: usize, delta: i32, fine: bool) {
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, knob, delta, fine);
}

fn press(s: &mut ChopSample, b: Button) -> Option<ChopAction> {
    let st = s.state.clone();
    s.tab.press(&mut s.ui, &st, b)
}

/// Applies an action and shows the new state, as the app does.
fn apply(s: &mut ChopSample, a: &ChopAction) -> std::ops::Range<usize> {
    let pads = a.apply(&mut s.project).unwrap();
    s.state = ChopState::from_project(&s.project, PAD, 0, s.state.source.clone()).unwrap();
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    pads
}

fn buttons(s: &ChopSample) -> (Vec<String>, Vec<bool>) {
    let b = s.ui.get::<ButtonRow>(chop_tab::BUTTONS).unwrap();
    (b.labels.clone(), b.enabled.clone())
}

fn list_right(s: &ChopSample) -> String {
    s.ui.get::<SettingList>(chop_tab::LIST)
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
    let b = s.tab.bounds(st);
    assert_eq!((b[0], *b.last().unwrap()), (st.region.start, st.region.end));
    assert_eq!(s.tab.slices(), 15, "{b:?}");
    for (k, &cut) in b[1..b.len() - 1].iter().enumerate() {
        let eighth = (k + 1) as f64 * beat() / 2.0;
        assert!(
            (cut as f64 - eighth).abs() < 0.01 * f64::from(RATE),
            "cut {k} at {cut}, eighth at {eighth}"
        );
    }
    assert_eq!(s.tab.method(), Method::Transient);
    // Fifteen slices fill A2-A16, A5 (the pad itself) among them.
    assert_eq!(s.tab.pads(st), Some(1..16));
    assert_eq!(s.tab.replaced(st), vec![PAD]);
    assert_eq!(list_right(&s), "replaces A5");
    assert_eq!(
        buttons(&s),
        (
            vec!["Split".to_string(), "Merge".into(), "Chop to A2–A16".into()],
            vec![true, true, true]
        )
    );
    let bar = s.ui.get::<PadBar>(chop_tab::BAR).unwrap();
    assert_eq!(
        (bar.active, bar.title.as_str()),
        (chop_tab::TAB, "Pad A5 · Break 92")
    );
    assert!(
        bar.subtitle.starts_with("15 slices · Transient · 92.0 BPM"),
        "{}",
        bar.subtitle
    );
    // Less sensitive never finds more hits.
    let mut s = chop();
    let mut last = usize::MAX;
    for _ in 0..10 {
        step(&mut s, Row::Amount, -1);
        assert!(s.tab.slices() <= last);
        last = s.tab.slices();
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
    let b = s.tab.bounds(&s.state);
    assert_eq!(s.tab.slices(), 8, "beats 1 to 7 inside the region: {b:?}");
    for (k, &cut) in b[1..b.len() - 1].iter().enumerate() {
        assert!(
            (cut as f64 - (k + 1) as f64 * beat()).abs() <= 256.0,
            "cut {k} at {cut}"
        );
    }
    step(&mut s, Row::Amount, -1);
    assert_eq!(s.tab.slices(), 15, "every 8th");
    qlink(&mut s, 2, 2, false);
    assert_eq!(s.tab.slices(), 4, "every 2 beats: cuts on beats 2, 4 and 6");
    // Grid: equal slices of the region, snapped to zero crossings.
    method(&mut s, Method::Grid);
    assert_eq!(s.tab.slices(), 8);
    let b = s.tab.bounds(&s.state);
    for (k, &cut) in b[1..b.len() - 1].iter().enumerate() {
        let even = rs + (re - rs) * (k as u64 + 1) / 8;
        assert!(
            cut.abs_diff(even) <= 256,
            "cut {k} at {cut}, even at {even}"
        );
    }
    qlink(&mut s, 2, -2, false);
    assert_eq!(s.tab.slices(), 6, "Q-Link 3 turns the setting");
    step(&mut s, Row::Snap, 1);
    let b = s.tab.bounds(&s.state);
    assert_eq!(b[1], rs + (re - rs) / 6, "unsnapped, exactly even");
    // Silence: one slice per sound; a lower threshold joins more of them.
    method(&mut s, Method::Silence);
    let sounds = s.tab.slices();
    assert!((2..=15).contains(&sounds), "{sounds}");
    for _ in 0..8 {
        step(&mut s, Row::Amount, -1);
    }
    assert!(s.tab.slices() <= sounds);
    // Manual has no setting; Q-Link 3 does nothing.
    method(&mut s, Method::Manual);
    let before = s.tab.bounds(&s.state);
    qlink(&mut s, 2, 5, false);
    assert_eq!(s.tab.bounds(&s.state), before);
}

#[test]
fn split_merge_and_moving_a_cut_make_the_chop_manual() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    assert_eq!(s.tab.slices(), 4);
    let mono = s.state.source.mono.clone();
    // Q-Link 1 picks slice 2; Split cuts it near its middle, on a zero crossing.
    qlink(&mut s, 0, 1, false);
    assert_eq!(s.tab.selected(), 1);
    let b = s.tab.bounds(&s.state);
    assert_eq!(press(&mut s, Button::Split), None);
    assert_eq!((s.tab.slices(), s.tab.method()), (5, Method::Manual));
    let mid = s.tab.bounds(&s.state)[2];
    assert!(mid.abs_diff((b[1] + b[2]) / 2) <= 256);
    let z = mid as usize;
    assert!(
        mono[z] == 0.0
            || (mono[z - 1] < 0.0) != (mono[z] < 0.0)
            || (mono[z] < 0.0) != (mono[z + 1] < 0.0)
    );
    // Merge joins the slice picked to the one before.
    qlink(&mut s, 0, 1, false);
    assert_eq!(s.tab.selected(), 2);
    press(&mut s, Button::Merge);
    assert_eq!((s.tab.slices(), s.tab.selected()), (4, 1));
    assert_eq!(
        s.tab.bounds(&s.state),
        b,
        "split then merged is where it began"
    );
    // Q-Link 2 moves the start of the slice picked: one frame with SHIFT.
    let start = s.tab.bounds(&s.state)[1];
    qlink(&mut s, 1, 3, true);
    assert_eq!(s.tab.bounds(&s.state)[1], start + 3);
    qlink(&mut s, 1, -3, true);
    // Unsnapped by hand too, a turn moves a pixel of the zoom per detent.
    qlink(&mut s, 1, 1, false);
    assert!(s.tab.bounds(&s.state)[1] > start);
    // Dragging a cut's marker moves it, but never past its neighbours.
    let st = s.state.clone();
    let ev = UiEvent {
        widget: chop_tab::WAVE.into(),
        kind: EventKind::Marker { index: 2, frame: 0 },
    };
    assert_eq!(s.tab.action(&mut s.ui, &ev, &st), None);
    let b = s.tab.bounds(&s.state);
    assert_eq!((b[2], s.tab.selected()), (b[1] + 32, 2));
    // The first marker is the pad's Start and the last its End: they stay.
    for index in [0, 4] {
        let ev = UiEvent {
            widget: chop_tab::WAVE.into(),
            kind: EventKind::Marker { index, frame: 1000 },
        };
        s.tab.action(&mut s.ui, &ev, &st);
        assert_eq!(s.tab.bounds(&s.state), b);
        assert_eq!(
            s.ui.get::<Waveform>(chop_tab::WAVE).unwrap().markers()[index].frame,
            b[index],
            "put back"
        );
    }
    // A tap on the wave picks the slice under it and plays it.
    let ev = UiEvent {
        widget: chop_tab::WAVE.into(),
        kind: EventKind::Frame(b[3] + 10),
    };
    assert_eq!(
        s.tab.action(&mut s.ui, &ev, &st),
        Some(ChopAction::Audition {
            pad: st.pad,
            layer: st.layer,
            start: b[3],
            end: b[4]
        })
    );
    assert_eq!(s.tab.selected(), 3);
    // Outside the region there is no slice to play.
    let ev = UiEvent {
        widget: chop_tab::WAVE.into(),
        kind: EventKind::Frame(b[b.len() - 1] + 10),
    };
    assert_eq!(s.tab.action(&mut s.ui, &ev, &st), None);
    assert_eq!(s.tab.selected(), 3);
    // A hand-made chop stays when the method's list is left alone, even
    // when the region's end moves (another pad chops again).
    let mut moved = s.state.clone();
    moved.region.end -= 1000;
    s.tab.update(&mut s.ui, &moved);
    assert_eq!(
        (s.tab.method(), s.tab.bounds(&moved)[1..4].to_vec()),
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
    let b = s.tab.bounds(&s.state);
    assert_eq!(s.tab.pads(&s.state), Some(5..9), "the pads after A5");
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
    // Now they would be replaced; one undo takes the pads and the chop back.
    assert_eq!(list_right(&s), "replaces 4 pads");
    assert!(s.project.undo());
    assert!(s.project.pad(5).is_none() && s.project.chop_sets.is_empty());
    assert!(s.project.pad(PAD).is_some());
}

#[test]
fn to_keys_puts_each_slice_on_a_note_and_the_pads_play_those_notes() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let b = s.tab.bounds(&s.state);
    step(&mut s, Row::Put, 1);
    let list = s.ui.get::<SettingList>(chop_tab::LIST).unwrap();
    assert_eq!(list.rows[5].value, "Keys from C2");
    assert_eq!(buttons(&s).0[2], "To keys C2–D#2");
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
fn pads_and_phrase_also_gives_the_slices_in_time_for_a_clip() {
    let mut s = chop();
    // Every eighth on the loop's own beat grid: the region starts a
    // sixteenth in, so the first slice is a sixteenth long.
    method(&mut s, Method::Beat);
    step(&mut s, Row::Amount, -1);
    assert_eq!(s.tab.slices(), 15);
    let list_value = |s: &ChopSample| {
        s.ui.get::<SettingList>(chop_tab::LIST).unwrap().rows[5]
            .value
            .clone()
    };
    // Pads, Keys, then Pads + phrase (the loop has a tempo), round again.
    step(&mut s, Row::Put, 1);
    step(&mut s, Row::Put, 1);
    assert_eq!(list_value(&s), "Pads + phrase");
    assert_eq!(buttons(&s).0[2], "Phrase on A2–A16");
    step(&mut s, Row::Put, 1);
    assert_eq!(list_value(&s), "Pads");
    step(&mut s, Row::Put, -1);
    assert_eq!(list_value(&s), "Pads + phrase");
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
    assert_eq!(apply(&mut s, &a), 1..16);
    assert!(s.project.pads_play() == PadsPlay::Pads);
    assert_eq!(a.phrase(&s.project).unwrap(), Some(p));
    // Without a tempo there is no phrase to make: the row reads Pads.
    let v = &s.state.source;
    let plain = SourceView {
        id: v.id,
        peaks: v.peaks.clone(),
        mono: v.mono.clone(),
        tempo: None,
        root: None,
        root_note: None,
        onsets: v.onsets.clone(),
    };
    s.state.source = Arc::new(plain);
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    assert_eq!(list_value(&s), "Pads");
    let a = press(&mut s, Button::ToPads).unwrap();
    assert!(matches!(a, ChopAction::ToPads { phrase: None, .. }));
    assert_eq!(a.phrase(&s.project), Ok(None));
    step(&mut s, Row::Put, -1);
    assert_eq!(list_value(&s), "Keys from C2", "Pads, Keys only");
    step(&mut s, Row::Put, 1);
    assert_eq!(list_value(&s), "Pads");
}

#[test]
fn the_first_pad_moves_within_the_bank_and_too_many_slices_cant_go_on_pads() {
    let mut s = chop();
    method(&mut s, Method::Grid);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    qlink(&mut s, 3, -10, false);
    assert_eq!(s.tab.pads(&s.state), Some(0..4));
    assert_eq!(list_right(&s), "no pad replaced");
    qlink(&mut s, 3, 2, false);
    assert_eq!(s.tab.pads(&s.state), Some(2..6));
    assert_eq!(list_right(&s), "replaces A5");
    qlink(&mut s, 3, 40, false);
    assert_eq!(s.tab.pads(&s.state), Some(12..16));
    step(&mut s, Row::Pads, -1);
    assert_eq!(s.tab.pads(&s.state), Some(11..15));
    // 17 slices don't fit in a bank of 16.
    for _ in 0..13 {
        step(&mut s, Row::Amount, 1);
    }
    assert_eq!(s.tab.slices(), 17);
    assert_eq!(s.tab.pads(&s.state), None);
    assert_eq!(press(&mut s, Button::ToPads), None);
    let (labels, enabled) = buttons(&s);
    assert_eq!((labels[2].as_str(), enabled[2]), ("Max 16 slices", false));
    assert!(list_right(&s).contains("too many"));
    // The bar's Wave tab goes back to Wave.
    let bar = s.ui.rect_of(chop_tab::BAR).unwrap();
    let wave =
        s.ui.get::<PadBar>(chop_tab::BAR)
            .unwrap()
            .tab_rect(bar, wave_tab::TAB);
    let ev = s.ui.touch(1, Touch::Down, wave.x + 5, wave.y + 5).unwrap();
    let st = s.state.clone();
    assert_eq!(
        s.tab.action(&mut s.ui, &ev, &st),
        Some(ChopAction::Tab(wave_tab::TAB))
    );
}

#[test]
fn every_chop_sample_screen_draws() {
    for mut s in sample::chop_samples(Fonts::embedded).unwrap() {
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
    let b = s.tab.bounds(&s.state);
    let ev = UiEvent {
        widget: chop_tab::WAVE.into(),
        kind: EventKind::Frame(b[2] + 5),
    };
    let st = s.state.clone();
    let tap = s.tab.action(&mut s.ui, &ev, &st).unwrap();
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
fn live_chop_drops_cuts_where_pads_are_played_snapped_to_hits() {
    let mut s = chop();
    let st = s.state.clone();
    let transients = s.tab.bounds(&st);
    // Transient steps back round to Live, which keeps the cuts.
    step(&mut s, Row::Method, -1);
    assert_eq!(s.tab.method(), Method::Live);
    assert_eq!(s.tab.bounds(&st), transients, "picking Live keeps the cuts");
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
    assert!(s.tab.live());
    assert_eq!(s.tab.slices(), 1);
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
    assert_eq!(s.tab.bounds(&st)[1..3], [h1, h2]);
    assert_eq!(s.tab.selected(), 2, "the slice the newest cut starts");
    // Pull to hits Off and zero-crossing Snap Off: a cut lands where the
    // sound was.
    step(&mut s, Row::Amount, -4);
    step(&mut s, Row::Snap, 1);
    assert_eq!(s.tab.live_marks(&mut s.ui, &st, &[lone]), 1);
    assert!(s.tab.bounds(&st).contains(&lone));
    assert_eq!(s.tab.method(), Method::Live);
    // Merge takes one out and stays Live; Stop ends the take.
    press(&mut s, Button::Merge);
    assert_eq!(s.tab.method(), Method::Live);
    assert_eq!(s.tab.slices(), 3);
    let a = press(&mut s, Button::Split).unwrap();
    assert!(matches!(a, ChopAction::Live { on: false, .. }));
    assert!(!s.tab.live());
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
    assert!(!s.tab.live());
    assert_eq!(buttons(&s).0[0], "Start");
    press(&mut s, Button::Split);
    // Another method while live: the step tells the app to stop.
    let index = Row::ALL.iter().position(|r| *r == Row::Method).unwrap();
    let ev = UiEvent {
        widget: chop_tab::LIST.into(),
        kind: EventKind::Step { index, delta: 1 },
    };
    let a = s.tab.action(&mut s.ui, &ev, &st);
    assert!(
        matches!(a, Some(ChopAction::Live { on: false, .. })),
        "{a:?}"
    );
    assert!(!s.tab.live());
    assert_eq!(s.tab.method(), Method::Transient);
    assert_eq!(buttons(&s).0[0], "Split");
}
