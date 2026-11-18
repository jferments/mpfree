//! The Sample page's Warp tool: the wave drawn as a warped layer plays,
//! pins put on hits by a tap, a drag, Pin hits and the Q-Links, each a
//! region change the app applies.

use mpfree_sampler::model::{Pin, WarpMethod};
use mpfree_sampler_ui::sample::{loose_project, warp_sample, Sample, BPM, PAD, RATE};
use mpfree_sampler_ui::warp_tool::{self, Grid, Row};
use mpfree_sampler_ui::wave_tab::{
    self, Tool, WaveAction, WaveState, QLINKS, REGION, TOOLS, WARP_HINT, WAVE,
};
use mpfree_sampler_ui::widgets::{KnobCard, PadBar, SettingList, ToolStrip};
use mpfree_ui::wave::Waveform;
use mpfree_ui::{EventKind, Fonts, Touch, UiEvent};

fn sample() -> Sample {
    warp_sample(Fonts::embedded(), loose_project()).unwrap()
}

fn fpb() -> f64 {
    f64::from(RATE) * 60.0 / BPM
}

fn pins(s: &Sample) -> Vec<Pin> {
    s.state.region.warp.as_ref().unwrap().pins.clone()
}

fn act(s: &mut Sample, widget: &str, kind: EventKind) -> Option<WaveAction> {
    let ev = UiEvent {
        widget: widget.into(),
        kind,
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

fn apply(s: &mut Sample, a: Option<WaveAction>) {
    s.apply(a).unwrap();
}

/// A touch on a widget, and what it asks for applied.
fn go(s: &mut Sample, widget: &str, kind: EventKind) {
    let a = act(s, widget, kind);
    apply(s, a);
}

/// Draws the screen, so the wave knows where its tabs are.
fn paint(s: &mut Sample) {
    let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
    s.ui.paint(&mut c);
}

fn wave(s: &Sample) -> &Waveform {
    s.ui.get::<Waveform>(WAVE).unwrap()
}

/// The hits the tool found, after the start.
fn hits(s: &Sample) -> Vec<u64> {
    let st = s.state.clone();
    let mut w = s.tab.warp().clone();
    w.hits_of(&st).iter().copied().filter(|&h| h > 0).collect()
}

/// Whether Pin hits, Remove pin and Clear pins can be pressed (not faint).
fn buttons_on(s: &Sample) -> [bool; 3] {
    let l = s.ui.get::<SettingList>(REGION).unwrap();
    [Row::PinHits, Row::RemovePin, Row::ClearPins].map(|k| !l.controls[k.index()].faint)
}

/// The pins' markers on the wave (after Trim's).
fn pin_marks(s: &Sample) -> Vec<mpfree_ui::wave::Marker> {
    let n = pins(s).len();
    let m = wave(s).markers();
    m[m.len() - n..].to_vec()
}

#[test]
fn the_warp_tool_shows_the_hits_and_counts_bars_from_the_regions_start() {
    let s = sample();
    assert_eq!(s.tab.tool(), Tool::Warp);
    assert_eq!(s.tab.hint(), WARP_HINT);
    let strip = s.ui.get::<ToolStrip>(TOOLS).unwrap();
    assert_eq!(strip.tools[strip.active], "Warp");
    // The Warp page has gone from the pad bar.
    let bar = s.ui.get::<PadBar>(wave_tab::BAR).unwrap();
    assert!(!bar.tabs.iter().any(|t| t == "Warp"), "{:?}", bar.tabs);
    let w = wave(&s);
    assert!(w.ticks().len() >= 12, "{}", w.ticks().len());
    assert!(w.bend().is_empty(), "no pins: drawn as recorded");
    let b = w.beats().unwrap();
    assert_eq!(b.first, s.state.region.start as f64);
    assert!((b.frames_per_beat - fpb()).abs() < 1e-6);
    let l = s.ui.get::<SettingList>(REGION).unwrap();
    let titles: Vec<&str> = l
        .panel
        .as_ref()
        .unwrap()
        .sections
        .iter()
        .map(|s| s.title.as_str())
        .collect();
    assert_eq!(titles, ["Warp", "Pins"]);
    assert_eq!(buttons_on(&s), [true, false, false]);
}

#[test]
fn a_tap_pins_the_nearest_hit_to_the_grid_and_a_second_picks_it() {
    let mut s = sample();
    let h = hits(&s)[2];
    let off = h as f64 / fpb();
    // A tap a little after the hit.
    let a = act(&mut s, WAVE, EventKind::Frame(h + 300));
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
    assert_eq!(s.tab.warp().selected(), Some(0));
    // The wave is drawn as it plays now: the hit on its sixteenth.
    let drawn = warp_tool::played(wave(&s).bend(), h);
    assert!((drawn - p[0].beat * fpb()).abs() < 1.0, "{drawn}");
    assert_eq!(pin_marks(&s)[0].frame, drawn.round() as u64);
    // A tap on it again picks it, changing nothing.
    s.tab.lift();
    assert_eq!(
        act(&mut s, WAVE, EventKind::Frame(drawn.round() as u64)),
        None
    );
    assert_eq!(s.tab.warp().selected(), Some(0));
}

#[test]
fn dragging_a_pins_tab_moves_it_along_the_grid() {
    let mut s = sample();
    go(&mut s, REGION, EventKind::Tab(Row::PinHits.index()));
    let before = pins(&s);
    assert!(before.len() >= 10);
    // Pin 4's tab, dragged a sixteenth right.
    paint(&mut s);
    let t =
        s.ui.targets()
            .into_iter()
            .find(|t| t.widget == WAVE && t.target.label == "4")
            .unwrap()
            .target;
    let (x, y) = t.centre();
    let px_per_16th = (fpb() / 4.0 / wave(&s).view().frames_per_px).round() as i32;
    assert!(px_per_16th > 12, "{px_per_16th}");
    let down = s.ui.touch(1, Touch::Down, x, y).unwrap();
    let st = s.state.clone();
    assert_eq!(s.tab.action(&mut s.ui, &down, &st), None, "touched: picked");
    assert_eq!(s.tab.warp().selected(), Some(3));
    let mut gesture = None;
    for dx in [px_per_16th / 2 + 2, px_per_16th] {
        let ev = s.ui.touch(1, Touch::Move, x + dx, y).unwrap();
        let EventKind::Marker { index, .. } = ev.kind else {
            panic!("{ev:?}");
        };
        assert_eq!(index, wave(&s).markers().len() - before.len() + 3);
        let st = s.state.clone();
        let a = s.tab.action(&mut s.ui, &ev, &st);
        if let Some(WaveAction::Region { gesture: g, .. }) = &a {
            assert!(gesture.is_none_or(|x| x == *g), "one undo step");
            gesture = Some(*g);
        }
        apply(&mut s, a);
    }
    s.ui.touch(1, Touch::Up, x + px_per_16th, y);
    let after = pins(&s);
    assert_eq!(after[3].frame, before[3].frame, "the same hit");
    assert!((after[3].beat - (before[3].beat + 0.25)).abs() < 1e-9);
    assert_eq!(s.tab.warp().selected(), Some(3));
}

#[test]
fn pin_hits_remove_and_clear() {
    let mut s = sample();
    let found = hits(&s);
    go(&mut s, REGION, EventKind::Tab(Row::PinHits.index()));
    let p = pins(&s);
    assert!(p.len() >= 10, "{}", p.len());
    for pin in &p {
        assert!(found.contains(&pin.frame));
        assert_eq!(pin.beat * 4.0, (pin.beat * 4.0).round(), "on a sixteenth");
    }
    let l = s.ui.get::<SettingList>(REGION).unwrap();
    assert_eq!(
        l.panel.as_ref().unwrap().sections[1].note,
        format!("{} pins", p.len())
    );
    // Pick the second pin by Q-Link 1, then remove it.
    let st = s.state.clone();
    assert!(s.tab.qlink(&mut s.ui, &st, 0, 2, false).is_none());
    assert_eq!(s.tab.warp().selected(), Some(1));
    assert_eq!(pin_marks(&s)[1].color, Some(s.ui.theme.text), "lit");
    go(&mut s, REGION, EventKind::Tab(Row::RemovePin.index()));
    let q = pins(&s);
    assert_eq!(q.len(), p.len() - 1);
    assert!(!q.contains(&p[1]));
    assert_eq!(s.tab.warp().selected(), None);
    go(&mut s, REGION, EventKind::Tab(Row::ClearPins.index()));
    assert!(pins(&s).is_empty());
    assert!(wave(&s).bend().is_empty());
    assert_eq!(buttons_on(&s), [true, false, false]);
}

#[test]
fn q_links_nudge_the_pin_and_tune_the_tempo_in_one_step_a_turn() {
    let mut s = sample();
    go(&mut s, REGION, EventKind::Tab(Row::PinHits.index()));
    s.tab.lift();
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, 0, 5, false);
    assert_eq!(s.tab.warp().selected(), Some(4));
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
    let card = s.ui.get::<KnobCard>(QLINKS[1]).unwrap();
    assert_eq!(card.label, "Q-Link 2 · Pin beat");
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
    // A tap says how to start while it isn't warped.
    assert_eq!(
        act(&mut s, WAVE, EventKind::Frame(30_000)),
        Some(WaveAction::Say(warp_tool::OFF_NOTE.to_string()))
    );
    let step = |row: Row, delta: i32| EventKind::Step {
        index: row.index(),
        delta,
    };
    go(&mut s, REGION, step(Row::Warp, 1));
    let w = s.state.region.warp.clone().unwrap();
    assert_eq!(w.method, WarpMethod::Stretch);
    assert!((f64::from(w.bpm) - BPM).abs() < 0.05, "{}", w.bpm);
    // Grid to eighths, pin the hits, step pin 1 on by an eighth.
    assert_eq!(act(&mut s, REGION, step(Row::Grid, -1)), None);
    assert_eq!(s.tab.warp().grid, Grid::Eighth);
    go(&mut s, REGION, EventKind::Tab(Row::PinHits.index()));
    for p in pins(&s) {
        assert_eq!(p.beat * 2.0, (p.beat * 2.0).round(), "on an eighth");
    }
    assert_eq!(act(&mut s, REGION, step(Row::Pin, 1)), None);
    assert_eq!(s.tab.warp().selected(), Some(0));
    let before = pins(&s)[0].beat;
    let a = act(&mut s, REGION, step(Row::PinBeat, 1));
    apply(&mut s, a);
    let after = pins(&s)[0].beat;
    assert!(
        after > before && after <= before + 0.5 + 1e-9,
        "{before} -> {after}"
    );
    assert_eq!(
        s.tab.warp().value_text(&s.state, Row::Pin),
        format!("1 of {}", pins(&s).len())
    );
    // Repitching, the pins stay but don't count: faint, drawn as recorded,
    // and a touch on one says so.
    go(&mut s, REGION, step(Row::Warp, 1));
    assert!(pin_marks(&s).iter().all(|m| m.faint && m.tab));
    assert!(wave(&s).bend().is_empty());
    assert_eq!(
        act(&mut s, WAVE, EventKind::Frame(30_000)),
        Some(WaveAction::Say(warp_tool::REPITCH_NOTE.to_string()))
    );
}

#[test]
fn the_wheel_zooms_round_the_pin_and_q_link_4_scrolls() {
    let mut s = sample();
    go(&mut s, REGION, EventKind::Tab(Row::PinHits.index()));
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, 0, 7, false);
    let at = pin_marks(&s)[6].frame as f64;
    for _ in 0..5 {
        s.tab.wheel(&mut s.ui, &st, 1);
    }
    let w = wave(&s).view();
    let width = Waveform::wave_rect(s.ui.rect_of(WAVE).unwrap()).w;
    let end = w.end(width);
    assert!(w.start < at && at < end, "{} {at} {end}", w.start);
    let card = s.ui.get::<KnobCard>(QLINKS[3]).unwrap();
    assert!(
        card.enabled && card.sub == "from",
        "{} {}",
        card.value,
        card.sub
    );
    s.tab.qlink(&mut s.ui, &st, 3, 4, false);
    let w2 = wave(&s).view();
    let moved = (w2.start - w.start) / w.frames_per_px;
    assert!((moved - f64::from(width) / 4.0).abs() < 1.0, "{moved}");
    // Zoomed right out again: the whole sound, nothing to scroll.
    for _ in 0..30 {
        s.tab.wheel(&mut s.ui, &st, -1);
    }
    assert!(!s.ui.get::<KnobCard>(QLINKS[3]).unwrap().enabled);
}

#[test]
fn the_other_tools_draw_the_pins_as_faint_lines_and_the_hits_faint() {
    let mut s = sample();
    go(&mut s, REGION, EventKind::Tab(Row::PinHits.index()));
    let n = pins(&s).len();
    let st = s.state.clone();
    s.tab.set_tool(&mut s.ui, &st, Tool::Trim);
    let w = wave(&s);
    assert!(w.bend().is_empty(), "Trim draws the sound as recorded");
    let m = w.markers();
    let lines = &m[m.len() - n..];
    assert!(lines.iter().all(|m| m.faint && !m.tab && m.fixed));
    assert_eq!(lines[0].frame, pins(&s)[0].frame, "on the hit itself");
    assert!(w.ticks().len() >= 12);
    // No pin tabs to take hold of with Trim, but its own.
    paint(&mut s);
    assert!(s
        .ui
        .targets()
        .iter()
        .any(|t| t.widget == WAVE && t.target.label == "START"));
    assert!(!s
        .ui
        .targets()
        .iter()
        .any(|t| t.widget == WAVE && t.target.label == "1"));
}

#[test]
fn moving_the_start_with_trim_keeps_the_pins_on_their_hits() {
    let mut s = sample();
    go(&mut s, REGION, EventKind::Tab(Row::PinHits.index()));
    let before = pins(&s);
    let r = &s.state.region;
    let n = wave_tab::moved(
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
fn process_asks_for_a_new_sound_moved_by_shift_and_stops_while_it_runs() {
    let mut s = sample();
    let value = |s: &Sample, row: Row| {
        s.ui.get::<SettingList>(REGION).unwrap().rows[row.index()]
            .value
            .clone()
    };
    let list = s.ui.get::<SettingList>(REGION).unwrap();
    assert!(
        list.buttons.contains(&Row::Process.index()),
        "Process is a button"
    );
    assert_eq!(value(&s, Row::Shift), "0 st");
    assert_eq!(value(&s, Row::Process), warp_tool::PROCESS_TEXT);
    // Shift steps a semitone at a time, to 24 either way, and changes
    // nothing in the project.
    let shift = |s: &mut Sample, delta: i32| {
        act(
            s,
            REGION,
            EventKind::Step {
                index: Row::Shift.index(),
                delta,
            },
        )
    };
    for _ in 0..3 {
        assert_eq!(shift(&mut s, 1), None);
    }
    assert_eq!(value(&s, Row::Shift), "+3 st");
    for _ in 0..40 {
        shift(&mut s, -1);
    }
    assert_eq!(value(&s, Row::Shift), "-24 st");
    for _ in 0..21 {
        shift(&mut s, 1);
    }
    // Named for the sound it makes, so it can't pass for a live pitch,
    // and the Warp part's note says what a tap makes.
    let list = s.ui.get::<SettingList>(REGION).unwrap();
    assert_eq!(
        (
            list.rows[Row::Shift.index()].name.as_str(),
            list.rows[Row::Process.index()].name.as_str()
        ),
        ("New sound's pitch", "New sound")
    );
    let note = &list.panel.as_ref().unwrap().sections[0].note;
    assert_eq!(note, "new sound: 3 st down, tap Make it");
    let process = |s: &mut Sample| act(s, REGION, EventKind::Tab(Row::Process.index()));
    assert_eq!(
        process(&mut s),
        Some(WaveAction::Process {
            pad: PAD,
            layer: 0,
            semitones: -3
        })
    );
    // While one runs, the button says how far it has got and stops it.
    let st = s.state.clone();
    s.tab.set_processing(&mut s.ui, &st, Some(0.4));
    assert_eq!(value(&s, Row::Process), "Stop · 40 %");
    assert_eq!(process(&mut s), Some(WaveAction::StopProcess));
    s.tab.set_processing(&mut s.ui, &st, None);
    assert_eq!(value(&s, Row::Process), warp_tool::PROCESS_TEXT);
}

#[test]
fn the_warp_section_fits_the_band_beside_sound_and_says_why_a_button_is_faint() {
    use mpfree_sampler_ui::panel::Look;
    use mpfree_ui::FontId;
    let mut s = sample();
    let band = s.ui.rect_of(REGION).unwrap();
    assert_eq!(band.h, 242);
    let l = s.ui.get::<SettingList>(REGION).unwrap();
    let panel = l.panel.as_ref().unwrap();
    let placed = panel.place(band, &l.controls);
    let mut rows: Vec<usize> = placed.iter().map(|p| p.row).collect();
    rows.sort();
    assert_eq!(rows, (0..Row::ALL.len()).collect::<Vec<_>>(), "every row");
    // Inside the band, and every value and name whole.
    let mut fonts = Fonts::embedded();
    for p in &placed {
        assert!(
            p.rect.x >= band.x
                && p.rect.right() <= band.right()
                && p.rect.y >= band.y
                && p.rect.bottom() <= band.bottom(),
            "{:?} out of {band:?}",
            Row::ALL[p.row]
        );
        let name = &l.rows[p.row].name;
        if p.control.y > p.rect.y {
            let w = fonts.measure(FontId::Sans, 13.0, name);
            assert!(w <= p.rect.w, "{name:?} ({w} px) in {} px", p.rect.w);
        }
        if let Look::Button(text) = &l.controls[p.row].look {
            let w = fonts.measure(FontId::Bold, 14.0, text);
            assert!(w <= p.control.w - 16, "{text:?} ({w} px)");
        }
    }
    // A faint button tapped says why, and leaves the wheel where it was.
    let picked = l.selected;
    let t =
        s.ui.targets()
            .into_iter()
            .find(|t| t.widget == REGION && t.target.label == "Remove pin")
            .unwrap()
            .target;
    assert!(!t.enabled);
    let (x, y) = t.centre();
    let ev = s.ui.touch(3, Touch::Down, x, y).unwrap();
    s.ui.touch(3, Touch::Up, x, y);
    assert_eq!(ev.kind, EventKind::Select(Row::RemovePin.index()));
    let l = s.ui.get::<SettingList>(REGION).unwrap();
    assert_eq!(l.selected, picked);
    assert_eq!(l.why_faint(Row::RemovePin.index()), Some("Tap a pin first"));
    // Pin hits pressed on the panel pins them, and the pin buttons wake.
    let t =
        s.ui.targets()
            .into_iter()
            .find(|t| t.widget == REGION && t.target.label == "Pin hits")
            .unwrap()
            .target;
    let (x, y) = t.centre();
    let ev = s.ui.touch(3, Touch::Down, x, y).unwrap();
    s.ui.touch(3, Touch::Up, x, y);
    let st = s.state.clone();
    let a = s.tab.action(&mut s.ui, &ev, &st);
    apply(&mut s, a);
    assert!(pins(&s).len() >= 4);
    assert_eq!(buttons_on(&s), [true, false, true]);
}

#[test]
fn a_typed_warp_tempo_is_one_undo_step() {
    let mut s = sample();
    let a = act(&mut s, REGION, EventKind::Type(Row::WarpTempo.index()));
    let Some(WaveAction::Ask(ask)) = a else {
        panic!("{a:?}")
    };
    assert_eq!(ask.key, wave_tab::TYPED_WARP_TEMPO);
    let st = s.state.clone();
    let a = s.tab.typed(&st, &ask.key, 95.5);
    apply(&mut s, a);
    assert!((s.state.region.warp.as_ref().unwrap().bpm - 95.5).abs() < 1e-4);
}
