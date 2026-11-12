//! The Warp tab: pins put on hits by a tap, a drag, Pin hits and the
//! Q-Links, each a region change the app applies.

use mpfree_sampler::model::{Pin, WarpMethod};
use mpfree_sampler_ui::sample::{loose_project, warp_sample, WarpSample, BPM, PAD, RATE};
use mpfree_sampler_ui::warp_tab::{self, Row, BUTTONS, LIST, VIEW};
use mpfree_sampler_ui::warp_widgets::{WarpView, TICKS};
use mpfree_sampler_ui::wave_tab::{WaveAction, WaveState};
use mpfree_sampler_ui::widgets::{ButtonRow, KnobCard, PadBar};
use mpfree_ui::{EventKind, Fonts, Touch, UiEvent};

fn sample() -> WarpSample {
    warp_sample(Fonts::embedded(), loose_project()).unwrap()
}

fn fpb() -> f64 {
    f64::from(RATE) * 60.0 / BPM
}

fn pins(s: &WarpSample) -> Vec<Pin> {
    s.state.region.warp.as_ref().unwrap().pins.clone()
}

fn act(s: &mut WarpSample, widget: &str, kind: EventKind) -> Option<WaveAction> {
    let ev = UiEvent {
        widget: widget.into(),
        kind,
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

fn apply(s: &mut WarpSample, a: Option<WaveAction>) {
    s.apply(a).unwrap();
}

/// A touch on a widget, and what it asks for applied.
fn go(s: &mut WarpSample, widget: &str, kind: EventKind) {
    let a = act(s, widget, kind);
    apply(s, a);
}

/// The hits the tab found, after the start.
fn hits(s: &mut WarpSample) -> Vec<u64> {
    let st = s.state.clone();
    s.tab
        .hits_of(&st)
        .iter()
        .copied()
        .filter(|&h| h > 0)
        .collect()
}

#[test]
fn the_tab_shows_the_region_in_its_beats() {
    let s = sample();
    let bar = s.ui.get::<PadBar>(warp_tab::BAR).unwrap();
    assert_eq!(bar.active, warp_tab::TAB);
    assert_eq!(bar.tabs[warp_tab::TAB], "Warp");
    assert_eq!(
        bar.subtitle,
        "2 bars at 92.0 BPM · stretched to the project tempo"
    );
    let v = s.ui.get::<WarpView>(VIEW).unwrap();
    assert!((v.total() - 8.0).abs() < 1e-4);
    assert!(v.pins_on);
    assert!(v.hits.len() >= 12, "{}", v.hits.len());
    assert_eq!(v.right, format!("{} hits found", v.hits.len()));
    let b = s.ui.get::<ButtonRow>(BUTTONS).unwrap();
    assert_eq!(b.enabled, vec![true, false, false]);
}

#[test]
fn a_tap_pins_the_nearest_hit_to_the_grid_and_a_second_picks_it() {
    let mut s = sample();
    let h = hits(&mut s)[2];
    let off = h as f64 / fpb();
    // A tap a little after the hit.
    let a = act(&mut s, VIEW, EventKind::Frame(h + 300));
    let Some(WaveAction::Region { pad, layer, .. }) = &a else {
        panic!("{a:?}");
    };
    assert_eq!((*pad, *layer), (PAD, 0));
    apply(&mut s, a);
    let p = pins(&s);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].frame, h);
    assert_eq!(
        p[0].beat,
        (off * 4.0).round() / 4.0,
        "on the nearest sixteenth"
    );
    assert_eq!(s.tab.selected(), Some(0));
    // A tap on it again picks it, changing nothing.
    s.tab.lift();
    let tap = s.ui.get::<WarpView>(VIEW).unwrap().frame_of(p[0].beat) as u64;
    assert_eq!(act(&mut s, VIEW, EventKind::Frame(tap)), None);
    assert_eq!(s.tab.selected(), Some(0));
}

#[test]
fn dragging_a_handle_moves_its_pin_along_the_grid() {
    let mut s = sample();
    go(&mut s, BUTTONS, EventKind::Tab(0));
    let before = pins(&s);
    assert!(before.len() >= 10);
    // Grab pin 3's handle in the ruler and drag it a little right.
    let r = s.ui.rect_of(VIEW).unwrap();
    let (ruler, wave, _) = WarpView::areas(r);
    let v = s.ui.get::<WarpView>(VIEW).unwrap();
    let x = v.x_of(before[3].beat, wave);
    let px_per_16th = v.x_of(0.25, wave) - v.x_of(0.0, wave);
    assert!(s.ui.touch(1, Touch::Down, x, ruler.y + 10).is_none());
    let ev =
        s.ui.touch(1, Touch::Move, x + px_per_16th / 3, ruler.y + 10);
    let Some(ev) = ev else { panic!("no drag") };
    let EventKind::Marker { index, frame } = ev.kind else {
        panic!("{ev:?}");
    };
    assert_eq!(index, 3);
    assert!(frame as f64 / TICKS > before[3].beat);
    let st = s.state.clone();
    let a = s.tab.action(&mut s.ui, &ev, &st);
    let g1 = match &a {
        Some(WaveAction::Region { gesture, .. }) => *gesture,
        // Snapped back onto the line it was on: no change.
        None => 0,
        _ => panic!("{a:?}"),
    };
    apply(&mut s, a);
    // Further: onto the next sixteenth, in the same undo step.
    let ev =
        s.ui.touch(1, Touch::Move, x + px_per_16th, ruler.y + 10)
            .unwrap();
    let st = s.state.clone();
    let a = s.tab.action(&mut s.ui, &ev, &st);
    if let Some(WaveAction::Region { gesture, .. }) = &a {
        if g1 != 0 {
            assert_eq!(*gesture, g1);
        }
    }
    apply(&mut s, a);
    s.ui.touch(1, Touch::Up, x + px_per_16th, ruler.y + 10);
    let after = pins(&s);
    assert_eq!(after[3].frame, before[3].frame, "the same hit");
    assert!((after[3].beat - (before[3].beat + 0.25)).abs() < 1e-9);
    assert_eq!(s.tab.selected(), Some(3));
}

#[test]
fn pin_hits_remove_and_clear() {
    let mut s = sample();
    let found = hits(&mut s);
    go(&mut s, BUTTONS, EventKind::Tab(0));
    let p = pins(&s);
    assert!(p.len() >= 10, "{}", p.len());
    for pin in &p {
        assert!(found.contains(&pin.frame));
        assert_eq!(pin.beat * 4.0, (pin.beat * 4.0).round(), "on a sixteenth");
    }
    let v = s.ui.get::<WarpView>(VIEW).unwrap();
    assert_eq!(v.right, format!("{} pins", p.len()));
    // Pick the second pin by Q-Link 1, then remove it.
    let st = s.state.clone();
    assert!(s.tab.qlink(&mut s.ui, &st, 0, 2, false).is_none());
    assert_eq!(s.tab.selected(), Some(1));
    go(&mut s, BUTTONS, EventKind::Tab(1));
    let q = pins(&s);
    assert_eq!(q.len(), p.len() - 1);
    assert!(!q.contains(&p[1]));
    assert_eq!(s.tab.selected(), None);
    go(&mut s, BUTTONS, EventKind::Tab(2));
    assert!(pins(&s).is_empty());
    let b = s.ui.get::<ButtonRow>(BUTTONS).unwrap();
    assert_eq!(b.enabled, vec![true, false, false]);
}

#[test]
fn q_links_nudge_the_pin_and_tune_the_tempo_in_one_step_a_turn() {
    let mut s = sample();
    go(&mut s, BUTTONS, EventKind::Tab(0));
    s.tab.lift();
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, 0, 5, false);
    assert_eq!(s.tab.selected(), Some(4));
    let before = pins(&s)[4].beat;
    let mut gesture = None;
    for _ in 0..2 {
        let st = s.state.clone();
        let a = s.tab.qlink(&mut s.ui, &st, 1, 1, false);
        let Some(WaveAction::Region { gesture: g, .. }) = &a else {
            panic!("{a:?}")
        };
        assert!(gesture.is_none_or(|x| x == *g), "one undo step");
        gesture = Some(*g);
        apply(&mut s, a);
    }
    assert!((pins(&s)[4].beat - (before + 2.0 / 64.0)).abs() < 1e-9);
    let card = s.ui.get::<KnobCard>(warp_tab::QLINKS[1]).unwrap();
    assert!(card.value.ends_with("+13"), "{}", card.value);
    // SHIFT: a 960th of a beat.
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 1, -1, true);
    apply(&mut s, a);
    assert!((pins(&s)[4].beat - (before + 2.0 / 64.0 - 1.0 / 960.0)).abs() < 1e-9);
    // Q-Link 3: a tenth of a BPM.
    s.tab.lift();
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 2, 3, false);
    apply(&mut s, a);
    let w = s.state.region.warp.as_ref().unwrap();
    assert!((w.bpm - 92.3).abs() < 1e-4, "{}", w.bpm);
}

#[test]
fn the_rows_turn_warp_on_and_step_the_pin() {
    let (mut project, view) = loose_project();
    project
        .edit_pad(PAD, None, |p| p.layers[0].region.warp = None)
        .unwrap();
    let mut s = warp_sample(Fonts::embedded(), (project, view)).unwrap();
    let v = s.ui.get::<WarpView>(VIEW).unwrap();
    assert_eq!(v.note, warp_tab::OFF_NOTE);
    assert!(!v.pins_on);
    // A tap does nothing while it isn't warped.
    assert_eq!(act(&mut s, VIEW, EventKind::Frame(30_000)), None);
    let step = |row: Row, delta: i32| EventKind::Step {
        index: row.index(),
        delta,
    };
    go(&mut s, LIST, step(Row::Warp, 1));
    let w = s.state.region.warp.clone().unwrap();
    assert_eq!(w.method, WarpMethod::Stretch);
    assert!((f64::from(w.bpm) - BPM).abs() < 0.05, "{}", w.bpm);
    assert!(s.ui.get::<WarpView>(VIEW).unwrap().note.is_empty());
    // Grid to eighths, pin the hits, step pin 1 on by an eighth.
    assert_eq!(act(&mut s, LIST, step(Row::Grid, -1)), None);
    assert_eq!(s.tab.grid, warp_tab::Grid::Eighth);
    go(&mut s, BUTTONS, EventKind::Tab(0));
    for p in pins(&s) {
        assert_eq!(p.beat * 2.0, (p.beat * 2.0).round(), "on an eighth");
    }
    assert_eq!(act(&mut s, LIST, step(Row::Pin, 1)), None);
    assert_eq!(s.tab.selected(), Some(0));
    let before = pins(&s)[0].beat;
    let a = act(&mut s, LIST, step(Row::PinBeat, 1));
    apply(&mut s, a);
    let after = pins(&s)[0].beat;
    assert!(
        after > before && after <= before + 0.5 + 1e-9,
        "{before} -> {after}"
    );
    assert_eq!(
        s.tab.value_text(&s.state, Row::Pin),
        format!("1 of {}", pins(&s).len())
    );
    // Repitching, the pins stay but don't count.
    go(&mut s, LIST, step(Row::Warp, 1));
    let v = s.ui.get::<WarpView>(VIEW).unwrap();
    assert!(!v.pins_on);
    assert_eq!(v.right, "pins count when stretching");
}

#[test]
fn the_wheel_zooms_round_the_pin_and_q_link_4_scrolls() {
    let mut s = sample();
    go(&mut s, BUTTONS, EventKind::Tab(0));
    act(&mut s, VIEW, EventKind::Select(6));
    let beat = pins(&s)[6].beat;
    let st = s.state.clone();
    for _ in 0..5 {
        s.tab.wheel(&mut s.ui, &st, 1);
    }
    let v = s.ui.get::<WarpView>(VIEW).unwrap();
    let (first, span) = v.shown();
    assert!(span < 3.0 && first < beat && beat < first + span);
    let card = s.ui.get::<KnobCard>(warp_tab::QLINKS[3]).unwrap();
    assert!(card.enabled && card.value.starts_with("from "));
    s.tab.qlink(&mut s.ui, &st, 3, 4, false);
    let (first2, _) = s.ui.get::<WarpView>(VIEW).unwrap().shown();
    assert!((first2 - (first + span / 4.0)).abs() < 1e-9);
    // Zoomed right out again: the whole region.
    for _ in 0..20 {
        s.tab.wheel(&mut s.ui, &st, -1);
    }
    assert_eq!(s.ui.get::<WarpView>(VIEW).unwrap().span, 0.0);
    assert!(!s.ui.get::<KnobCard>(warp_tab::QLINKS[3]).unwrap().enabled);
}

#[test]
fn moving_the_start_on_the_wave_tab_keeps_the_pins_on_their_hits() {
    let mut s = sample();
    go(&mut s, BUTTONS, EventKind::Tab(0));
    let before = pins(&s);
    let r = &s.state.region;
    // The start moved to the first pin's hit (Wave tab's Start marker).
    let n = mpfree_sampler_ui::wave_tab::moved(
        r,
        mpfree_sampler_ui::Mark::Start,
        before[0].frame,
        s.state.frames,
        RATE,
    );
    let w = n.warp.as_ref().unwrap();
    assert_eq!(n.start, before[0].frame);
    assert_eq!(w.pins.len(), before.len() - 1);
    for (a, b) in w.pins.iter().zip(&before[1..]) {
        assert_eq!(a.frame, b.frame);
        assert!((a.beat - (b.beat - before[0].beat)).abs() < 1e-9);
    }
    let st = WaveState {
        region: n,
        ..s.state.clone()
    };
    s.tab.update(&mut s.ui, &st);
}

#[test]
fn a_moving_playhead_redraws_only_its_strips() {
    use mpfree_ui::{SoftCanvas, Widget};
    let mut s = sample();
    let r = s.ui.rect_of(VIEW).unwrap();
    let mut c = SoftCanvas::new(1280, 800);
    s.ui.paint(&mut c);
    let st = s.state.clone();
    s.tab.set_playheads(&mut s.ui, &[40_000]);
    let parts =
        s.ui.get::<WarpView>(VIEW)
            .unwrap()
            .changed_parts(r)
            .unwrap();
    assert_eq!(parts.len(), 1);
    assert!(parts[0].w <= 4);
    s.ui.paint(&mut c);
    s.tab.set_playheads(&mut s.ui, &[60_000]);
    let parts =
        s.ui.get::<WarpView>(VIEW)
            .unwrap()
            .changed_parts(r)
            .unwrap();
    assert_eq!(parts.len(), 2, "the strip it left and the one it entered");
    // Anything else changing draws it all.
    s.tab.update(&mut s.ui, &st);
    assert!(s
        .ui
        .get::<WarpView>(VIEW)
        .unwrap()
        .changed_parts(r)
        .is_none());
}
