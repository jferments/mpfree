//! The Wave tab: what it shows, and what its touches, Q-Links and wheel do
//! to a real project.

use std::sync::Arc;

use mpfree_sampler::model::{LoopMode, PlayMode, Project, Region, Source, WarpMethod};
use mpfree_sampler_ui::sample::{self, wave_sample, Sample, BPM, PAD, RATE};
use mpfree_sampler_ui::wave_tab::{
    self, moved, position_text, Mark, Row, Snap, WaveAction, WaveState,
};
use mpfree_sampler_ui::widgets::{KnobCard, PadBar, SettingList};
use mpfree_sampler_ui::{chop_tab, grain_tab, play_tab, warp_tab};
use mpfree_sampler_ui::{PyramidPeaks, SourceView};
use mpfree_ui::wave::{Peaks, RawPeaks, Waveform};
use mpfree_ui::{EventKind, Fonts, SoftCanvas, Touch, UiEvent};

fn sample() -> Sample {
    let mut s = wave_sample(Fonts::embedded()).unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

fn beat() -> f64 {
    f64::from(RATE) * 60.0 / BPM
}

/// Applies an action and shows the new state, as the app does.
fn apply(s: &mut Sample, a: &WaveAction) {
    a.apply(&mut s.project).unwrap();
    s.state = WaveState::from_project(&s.project, PAD, 0, s.state.source.clone()).unwrap();
    s.tab.update(&mut s.ui, &s.state);
}

fn region(s: &Sample) -> Region {
    s.project.pad(PAD).unwrap().layers[0].region.clone()
}

fn step(s: &mut Sample, row: Row, delta: i32) -> Option<WaveAction> {
    let index = Row::ALL.iter().position(|r| *r == row).unwrap();
    let ev = UiEvent {
        widget: wave_tab::REGION.into(),
        kind: EventKind::Step { index, delta },
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

fn zero_crossing_at(mono: &[f32], f: u64) -> bool {
    let f = f as usize;
    let near = |i: usize| {
        i > 0 && i < mono.len() && (mono[i] == 0.0 || (mono[i - 1] < 0.0) != (mono[i] < 0.0))
    };
    near(f) || near(f + 1)
}

#[test]
fn positions_names_and_lengths_read_as_on_an_mpc() {
    let fpb = beat();
    assert_eq!(
        position_text(0, RATE, Some(BPM)),
        ("1.1.1".to_string(), "+000".to_string())
    );
    assert_eq!(
        position_text((fpb * 0.25).round() as u64, RATE, Some(BPM)).0,
        "1.1.2"
    );
    assert_eq!(
        position_text((fpb * 7.5).round() as u64, RATE, Some(BPM)),
        ("2.4.3".to_string(), "+000".to_string())
    );
    assert_eq!(
        position_text((fpb * 4.125).round() as u64, RATE, Some(BPM)),
        ("2.1.1".to_string(), "+120".to_string())
    );
    assert_eq!(
        position_text(66_150, RATE, None),
        ("1.500 s".to_string(), "66150".to_string())
    );
    assert_eq!(wave_tab::pad_label(0), "A1");
    assert_eq!(wave_tab::pad_label(4), "A5");
    assert_eq!(wave_tab::pad_label(16 + 15), "B16");
    assert_eq!(wave_tab::duration_text(441, RATE), "10 ms");
    assert_eq!(wave_tab::duration_text(44, RATE), "1.0 ms");
    assert_eq!(wave_tab::duration_text(66_150, RATE), "1.50 s");
}

#[test]
fn the_source_is_analysed_once_for_the_screens() {
    let (p, view) = sample::project();
    let s = &p.sources()[0];
    let tempo = view.tempo.expect("a drum loop has a tempo");
    assert!((tempo - BPM).abs() < 0.05, "tempo {tempo}");
    assert_eq!(view.mono.len() as u64, s.frames());
    // Zoomed in, the pyramid draws the samples themselves; zoomed out, its
    // columns hold the same peaks to within its 16-bit storage.
    let raw = RawPeaks::new(s.data().clone(), 2).unwrap();
    let pyr = PyramidPeaks::new(s.data().clone(), 2).unwrap();
    for (f0, w) in [
        (1000.0, 3.0),
        (5000.0, 40.0),
        (20_000.0, 1024.0),
        (0.0, 30_000.0),
    ] {
        for ch in 0..2 {
            let (a, b) = (raw.column(ch, f0, f0 + w), pyr.column(ch, f0, f0 + w));
            assert!(
                (a.min - b.min).abs() < 1e-3 && (a.max - b.max).abs() < 1e-3,
                "{f0} {w}: {a:?} vs {b:?}"
            );
        }
    }
    assert_eq!(pyr.column(5, 0.0, 10.0), Default::default());
}

#[test]
fn it_shows_the_pad_its_region_and_the_q_links() {
    let s = sample();
    let bar = s.ui.get::<PadBar>(wave_tab::BAR).unwrap();
    assert_eq!(bar.title, "Pad A5 · Break 92");
    assert!(
        bar.subtitle.contains("2 bars")
            && bar.subtitle.contains("92.0 BPM")
            && bar.subtitle.contains("stereo"),
        "{}",
        bar.subtitle
    );
    let w = s.ui.get::<Waveform>(wave_tab::WAVE).unwrap();
    let labels: Vec<&str> = w.markers().iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, ["START", "END", "LOOP", "LOOP END"]);
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    let value = |name: &str| {
        list.rows
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(value("Mode"), "Loop");
    assert_eq!(value("Loop"), "Forward");
    assert_eq!(value("Crossfade"), "12 ms");
    assert_eq!(value("Gain"), "+2.0 dB");
    assert_eq!(value("Fade out"), "100 ms");
    assert_eq!(value("Snap"), "Zero crossing");
    let q = |k: usize| s.ui.get::<KnobCard>(wave_tab::QLINKS[k]).unwrap();
    assert_eq!(
        (q(0).value.as_str(), q(1).value.as_str()),
        ("1.1.2", "2.4.3")
    );
    assert_eq!(
        (q(2).value.as_str(), q(3).value.as_str()),
        ("2.1.1", "2.3.1")
    );
    assert!(q(0).focused && !q(2).focused);
}

#[test]
fn dragging_a_marker_snaps_it_and_one_drag_is_one_undo_step() {
    let mut s = sample();
    let before = region(&s);
    let r = s.ui.rect_of(wave_tab::WAVE).unwrap();
    let wave = Waveform::wave_rect(r);
    let view = s.tab.view(&s.ui).unwrap();
    let x = wave.x + view.x_of(before.sustain_loop.unwrap().start as f64).round() as i32;
    let y = r.y + 15;
    let st = s.state.clone();
    let ev = s.ui.touch(1, Touch::Down, x, y).unwrap();
    assert!(
        matches!(ev.kind, EventKind::Marker { index: 2, .. }),
        "{ev:?}"
    );
    assert_eq!(
        s.tab.action(&mut s.ui, &ev, &st),
        None,
        "touching a tab changes nothing"
    );
    let mut gestures = Vec::new();
    for dx in [40, 80, 120] {
        let ev = s.ui.touch(1, Touch::Move, x + dx, y).unwrap();
        let st = s.state.clone();
        let a = s
            .tab
            .action(&mut s.ui, &ev, &st)
            .expect("the loop start moves");
        let WaveAction::Region {
            gesture,
            ref region,
            ..
        } = a
        else {
            panic!("{a:?}")
        };
        gestures.push(gesture);
        let l = region.sustain_loop.unwrap();
        assert!(
            zero_crossing_at(&st.source.mono, l.start),
            "loop start {} is on a zero crossing",
            l.start
        );
        assert!(l.start > before.sustain_loop.unwrap().start);
        apply(&mut s, &a);
    }
    s.ui.touch(1, Touch::Up, x + 120, y);
    s.tab.lift();
    assert!(
        gestures.windows(2).all(|g| g[0] == g[1]),
        "one drag, one gesture: {gestures:?}"
    );
    assert_eq!(s.tab.focus(), Mark::LoopStart);
    // The tab shows where the marker is.
    let shown = s.ui.get::<Waveform>(wave_tab::WAVE).unwrap().markers()[2].frame;
    assert_eq!(shown, region(&s).sustain_loop.unwrap().start);
    // One undo puts the whole drag back.
    assert!(s.project.undo());
    assert_eq!(region(&s), before);
}

#[test]
fn markers_never_cross_and_the_region_stays_valid() {
    let (mut p, _) = sample::project();
    let frames = p.sources()[0].frames();
    let mut seed = 0x9E37_79B9u32;
    let mut rand = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        u64::from(seed) % n
    };
    for k in 0..2000u64 {
        let r = p.pad(PAD).unwrap().layers[0].region.clone();
        let m = Mark::ALL[rand(4) as usize];
        let to = rand(frames + 2000);
        let n = moved(&r, m, to, frames, RATE);
        if let Some(l) = n.sustain_loop {
            assert!(
                n.start + u64::from(l.crossfade) <= l.start && l.start < l.end && l.end <= n.end,
                "{n:?}"
            );
        }
        assert!(n.start < n.end && n.end <= frames);
        p.edit_pad(PAD, Some(k), |pd| pd.layers[0].region = n)
            .expect("every move gives a valid region");
    }
}

#[test]
fn q_links_move_a_pixel_a_detent_snapped_or_a_frame_with_shift() {
    let mut s = sample();
    let mono = s.state.source.mono.clone();
    let start = region(&s).start;
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 0, 1, false).unwrap();
    let WaveAction::Region {
        region: ref r,
        gesture: g1,
        ..
    } = a
    else {
        panic!()
    };
    let px = s.tab.view(&s.ui).unwrap().frames_per_px;
    assert!(
        r.start as f64 >= start as f64 + px - 1.0,
        "a pixel on: {} -> {}",
        start,
        r.start
    );
    assert!(zero_crossing_at(&mono, r.start));
    apply(&mut s, &a);
    // SHIFT: exactly one frame, unsnapped, still the same turn.
    let at = region(&s).start;
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 0, -1, true).unwrap();
    let WaveAction::Region {
        region: ref r,
        gesture: g2,
        ..
    } = a
    else {
        panic!()
    };
    assert_eq!(r.start, at - 1);
    assert_eq!(g1, g2);
    apply(&mut s, &a);
    // Let go: the next turn is a new undo step.
    s.tab.lift();
    let st = s.state.clone();
    let WaveAction::Region { gesture: g3, .. } = s.tab.qlink(&mut s.ui, &st, 0, 1, true).unwrap()
    else {
        panic!()
    };
    assert_ne!(g1, g3);
    // End can't pass the loop end, however far it is turned.
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 1, -10_000, false).unwrap();
    let WaveAction::Region { region: ref r, .. } = a else {
        panic!()
    };
    assert_eq!(r.end, r.sustain_loop.unwrap().end);
    // With snap on sixteenths, a detent lands on the next sixteenth.
    step(&mut s, Row::Snap, 1);
    assert_eq!(s.tab.snap(), Snap::Sixteenths);
    let st = s.state.clone();
    let a = s.tab.qlink(&mut s.ui, &st, 2, 1, false).unwrap();
    let WaveAction::Region { region: ref r, .. } = a else {
        panic!()
    };
    let six = beat() / 4.0;
    assert_eq!(r.sustain_loop.unwrap().start, (17.0 * six).round() as u64);
}

#[test]
fn a_loop_q_link_without_a_loop_does_nothing() {
    let mut s = sample();
    s.project
        .edit_pad(PAD, None, |p| {
            let r = &mut p.layers[0].region;
            r.mode = PlayMode::OneShot;
            r.sustain_loop = None;
        })
        .unwrap();
    s.state = WaveState::from_project(&s.project, PAD, 0, s.state.source.clone()).unwrap();
    s.tab.update(&mut s.ui, &s.state);
    let st = s.state.clone();
    assert_eq!(s.tab.qlink(&mut s.ui, &st, 2, 5, false), None);
    assert!(!s.ui.get::<KnobCard>(wave_tab::QLINKS[3]).unwrap().enabled);
    assert_eq!(
        s.ui.get::<Waveform>(wave_tab::WAVE)
            .unwrap()
            .markers()
            .len(),
        2
    );
}

#[test]
fn the_wheel_zooms_round_the_focused_marker() {
    let mut s = sample();
    let wave = Waveform::wave_rect(s.ui.rect_of(wave_tab::WAVE).unwrap());
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, 2, 0, false);
    assert_eq!(s.tab.focus(), Mark::LoopStart);
    let ls = region(&s).sustain_loop.unwrap().start as f64;
    let v0 = s.tab.view(&s.ui).unwrap();
    s.tab.wheel(&mut s.ui, &st, 3);
    let v1 = s.tab.view(&s.ui).unwrap();
    assert!((v0.frames_per_px / v1.frames_per_px - 1.25f64.powi(3)).abs() < 1e-6);
    assert!(
        (v0.x_of(ls) - v1.x_of(ls)).abs() < 1.0,
        "the loop start stays put"
    );
    s.tab.wheel(&mut s.ui, &st, -3);
    let v2 = s.tab.view(&s.ui).unwrap();
    assert!((v2.frames_per_px - v0.frames_per_px).abs() < 1e-6);
    // A tap on the overview centres the view there.
    s.tab.wheel(&mut s.ui, &st, 4);
    let ev = UiEvent {
        widget: wave_tab::OVERVIEW.into(),
        kind: EventKind::Frame(100_000),
    };
    assert_eq!(s.tab.action(&mut s.ui, &ev, &st), None);
    let v = s.tab.view(&s.ui).unwrap();
    assert!((v.frame_at(f64::from(wave.w) / 2.0) - 100_000.0).abs() < v.frames_per_px * 2.0);
}

#[test]
fn region_settings_step_and_stay_valid() {
    let mut s = sample();
    // Gain in half decibels, one undo step per tap.
    let a = step(&mut s, Row::Gain, 1).unwrap();
    apply(&mut s, &a);
    s.tab.lift();
    assert_eq!(region(&s).gain_db, 2.5);
    // Loop mode can't drop its loop: back from Forward, Off is skipped.
    let a = step(&mut s, Row::Loop, -1).unwrap();
    apply(&mut s, &a);
    s.tab.lift();
    let l = region(&s).sustain_loop.unwrap();
    assert_eq!(
        (l.mode, l.crossfade),
        (LoopMode::PingPong, 0),
        "ping-pong loops have no crossfade"
    );
    assert_eq!(
        step(&mut s, Row::Crossfade, 1),
        None,
        "nothing to cross-fade"
    );
    // A one-shot can drop its loop; going back to Loop mode makes one.
    let a = step(&mut s, Row::Mode, -1).unwrap();
    apply(&mut s, &a);
    s.tab.lift();
    assert_eq!(region(&s).mode, PlayMode::Toggle);
    let a = step(&mut s, Row::Loop, 1).unwrap();
    apply(&mut s, &a);
    s.tab.lift();
    assert_eq!(region(&s).sustain_loop, None, "Ping-pong, then Off");
    let a = step(&mut s, Row::Mode, 1).unwrap();
    apply(&mut s, &a);
    s.tab.lift();
    let r = region(&s);
    assert_eq!(r.mode, PlayMode::Loop);
    let l = r.sustain_loop.unwrap();
    assert_eq!(
        (l.start, l.end),
        (r.start + r.frames() / 4, r.end - r.frames() / 4)
    );
    // Fades can't add up to more than the region.
    for _ in 0..20 {
        if let Some(a) = step(&mut s, Row::FadeIn, 1) {
            apply(&mut s, &a);
        }
    }
    let r = region(&s);
    assert!(u64::from(r.fade_in.frames) + u64::from(r.fade_out.frames) <= r.frames());
    // Snap goes Off, Zero crossing, 16ths and round, changing nothing in
    // the project.
    let rev = s.project.revision();
    assert_eq!(step(&mut s, Row::Snap, 1), None);
    assert_eq!(s.tab.snap(), Snap::Sixteenths);
    assert_eq!(step(&mut s, Row::Snap, 1), None);
    assert_eq!(s.tab.snap(), Snap::Off);
    assert_eq!(s.project.revision(), rev);
}

#[test]
fn snapping_to_sixteenths_needs_a_tempo() {
    // A sine has no beat, so the snap choices are Off and Zero crossing.
    let data: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let mut p = Project::new();
    let src = Source::new("Tone", RATE, 1, data).unwrap();
    let view = Arc::new(SourceView::analyse(&src));
    assert_eq!(view.tempo, None);
    let id = p.add_source(src);
    let region = Region::whole(p.source(id).unwrap());
    p.set_pad(
        0,
        Some(mpfree_sampler::model::Pad::new(
            "Tone",
            mpfree_sampler::model::Layer::new(region),
        )),
        None,
    )
    .unwrap();
    let st = WaveState::from_project(&p, 0, 0, view).unwrap();
    let mut ui = sample::app_ui(Fonts::embedded(), &p);
    let mut tab = wave_tab::WaveTab::add(&mut ui, "wave", &st).unwrap();
    let index = Row::ALL.iter().position(|r| *r == Row::Snap).unwrap();
    for want in [Snap::Off, Snap::ZeroCrossing, Snap::Off] {
        let ev = UiEvent {
            widget: wave_tab::REGION.into(),
            kind: EventKind::Step { index, delta: 1 },
        };
        tab.action(&mut ui, &ev, &st);
        assert_eq!(tab.snap(), want);
    }
    let list = ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert_eq!(
        list.rows
            .iter()
            .find(|r| r.name == "Channels")
            .unwrap()
            .value,
        "Mono"
    );
    assert!(!ui
        .get::<PadBar>(wave_tab::BAR)
        .unwrap()
        .subtitle
        .contains("BPM"));
}

#[test]
fn tabs_taps_and_the_sample_screens() {
    let mut s = sample();
    let bar = s.ui.rect_of(wave_tab::BAR).unwrap();
    let pb = s.ui.get::<PadBar>(wave_tab::BAR).unwrap();
    let (warp_tab_r, chop_tab_r, play_tab_r, grain_tab_r, fx_tab_r) = (
        pb.tab_rect(bar, warp_tab::TAB),
        pb.tab_rect(bar, chop_tab::TAB),
        pb.tab_rect(bar, play_tab::TAB),
        pb.tab_rect(bar, grain_tab::TAB),
        pb.tab_rect(bar, 5),
    );
    assert_eq!(pb.active, wave_tab::TAB);
    assert_eq!(pb.tabs[0], "Wave", "the pads are the app's Pads tab now");
    let st = s.state.clone();
    for (id, r, tab, what) in [
        (7, warp_tab_r, warp_tab::TAB, "Warp opens"),
        (2, chop_tab_r, chop_tab::TAB, "Chop opens"),
        (4, play_tab_r, play_tab::TAB, "Play opens"),
        (5, grain_tab_r, grain_tab::TAB, "Grain opens"),
    ] {
        let ev = s.ui.touch(id, Touch::Down, r.x + 5, r.y + 5).unwrap();
        assert_eq!(
            s.tab.action(&mut s.ui, &ev, &st),
            Some(WaveAction::Tab(tab)),
            "{what}"
        );
        s.ui.touch(id, Touch::Up, 0, 0);
    }
    assert_eq!(
        s.ui.touch(6, Touch::Down, fx_tab_r.x + 5, fx_tab_r.y + 5),
        None,
        "FX isn't built yet"
    );
    s.ui.touch(6, Touch::Up, 0, 0);
    // A tap on the wave plays the pad.
    let w = Waveform::wave_rect(s.ui.rect_of(wave_tab::WAVE).unwrap());
    s.ui.touch(3, Touch::Down, w.x + 300, w.y + 100);
    let ev = s.ui.touch(3, Touch::Up, w.x + 300, w.y + 100).unwrap();
    assert_eq!(
        s.tab.action(&mut s.ui, &ev, &st),
        Some(WaveAction::Audition { pad: PAD })
    );
    // Every sample screen draws.
    for mut s in sample::samples(Fonts::embedded).unwrap() {
        let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
        assert!(!s.ui.paint(&mut c).is_empty(), "{}", s.name);
    }
}

#[test]
fn warp_turns_on_at_the_regions_own_tempo_and_steps_through_its_methods() {
    let mut s = sample();
    let st = s.state.clone();
    assert_eq!(row_value(&s, Row::Warp), "Off");
    assert_eq!(row_value(&s, Row::WarpTempo), "—");
    assert!(
        step(&mut s, Row::WarpTempo, 1).is_none(),
        "no tempo to set while off"
    );
    let a = step(&mut s, Row::Warp, 1).unwrap();
    apply(&mut s, &a);
    let w = region(&s).warp.unwrap();
    assert_eq!(w.method, WarpMethod::Stretch);
    // The test loop is whole bars at its tempo, so warp starts there.
    let want = wave_tab::region_tempo(&st, &region(&s));
    assert_eq!(w.bpm, want);
    assert!((f64::from(w.bpm) - BPM).abs() < 0.5, "{} vs {BPM}", w.bpm);
    assert_eq!(row_value(&s, Row::Warp), "Stretch");
    assert_eq!(row_value(&s, Row::WarpTempo), format!("{:.1} BPM", w.bpm));
    let a = step(&mut s, Row::WarpTempo, 1).unwrap();
    apply(&mut s, &a);
    assert_eq!(region(&s).warp.unwrap().bpm, w.bpm + 1.0);
    let a = step(&mut s, Row::Warp, 1).unwrap();
    apply(&mut s, &a);
    assert_eq!(region(&s).warp.unwrap().method, WarpMethod::Repitch);
    assert_eq!(region(&s).warp.unwrap().bpm, w.bpm + 1.0, "the tempo stays");
    let a = step(&mut s, Row::Warp, 1).unwrap();
    apply(&mut s, &a);
    assert_eq!(region(&s).warp, None);
}

fn row_value(s: &Sample, row: Row) -> String {
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    list.rows[Row::ALL.iter().position(|r| *r == row).unwrap()]
        .value
        .clone()
}
