//! The Sample page's Grain tool: the grain window it draws on the wave,
//! the Grain section's parts, the Q-Links and the Sound section's Grain
//! chip, and how their touches and turns change the region in a real
//! project, one undo step each.

use mpfree_sampler::engine::{CloudState, Engine, EngineView, Event};
use mpfree_sampler::model::{
    GrainFilter, GrainFilterKind, GrainMode, GrainQuality, GrainWindow, LfoShape, MotionTarget,
    PlayMode, Project,
};
use mpfree_sampler_ui::grain_tab::{self, describe, size_ms_at, CloudNow, Row, View};
use mpfree_sampler_ui::grain_tool::{GrainPart, PARTS_ROW};
use mpfree_sampler_ui::panel::{self, Look};
use mpfree_sampler_ui::play_tab::PlayAction;
use mpfree_sampler_ui::sample::{
    self, grain_project, grain_samples, grains_after, Sample, PAD, RATE,
};
use mpfree_sampler_ui::sound_panel::{self, Part};
use mpfree_sampler_ui::wave_tab::{self, Tool, WaveAction, WaveState};
use mpfree_sampler_ui::widgets::{KnobCard, PadBar, SettingList, ToolStrip};
use mpfree_ui::wave::{GrainSpan, Waveform};
use mpfree_ui::{EventKind, FontId, Fonts, SoftCanvas, UiEvent};

/// The Sample page with the Grain tool on the granular pad (or on the
/// loop, not granular yet), drawn once so every widget has its place.
fn grain(granular: bool) -> Sample {
    let mut s = grain_samples(Fonts::embedded)
        .unwrap()
        .into_iter()
        .nth(if granular { 0 } else { 1 })
        .unwrap();
    let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
    s.ui.paint(&mut c);
    s
}

/// How many undo steps the project holds (undone on a copy).
fn undo_steps(p: &Project) -> usize {
    let mut p = p.clone();
    let mut n = 0;
    while p.undo() {
        n += 1;
    }
    n
}

/// The rows the part open shows for the layer's grain mode.
fn shown(s: &Sample) -> Vec<Row> {
    s.tab.grain_part().rows(&s.state.region)
}

/// Where `row` is in the Grain section's list (the parts' switch first).
fn index(s: &Sample, row: Row) -> usize {
    1 + shown(s)
        .iter()
        .position(|r| *r == row)
        .unwrap_or_else(|| panic!("{row:?} isn't shown"))
}

/// The Q-Links' four: on their cards and the Sound section's Grain chip,
/// not on the Grain section.
fn on_qlinks(row: Row) -> bool {
    grain_tab::QLINK_ROWS.contains(&row)
}

fn region_ev(kind: EventKind) -> UiEvent {
    UiEvent {
        widget: wave_tab::REGION.into(),
        kind,
    }
}

/// Taps part `part` on the Grain section's switch, as a finger would.
fn open(s: &mut Sample, part: GrainPart) {
    let delta = part.index() as i32 - s.tab.grain_part().index() as i32;
    if delta != 0 {
        let ev = region_ev(EventKind::Step {
            index: PARTS_ROW,
            delta,
        });
        let st = s.state.clone();
        assert!(s.tab.action(&mut s.ui, &ev, &st).is_none());
    }
    assert_eq!(s.tab.grain_part(), part);
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert!(matches!(
        &list.controls[PARTS_ROW].look,
        Look::Choice { on, .. } if *on == part.index()
    ));
}

/// Opens the part that holds `row`, unless the one open shows it.
fn open_for(s: &mut Sample, row: Row) {
    if !shown(s).contains(&row) {
        let part = GrainPart::of(row).unwrap_or_else(|| panic!("{row:?} is on no part"));
        open(s, part);
    }
}

/// What the page shows for `row`: the Grain section's row (its part
/// opened first), or for the Q-Links' four what their cards say.
fn row_value(s: &mut Sample, row: Row) -> String {
    if let Some(k) = grain_tab::QLINK_ROWS.iter().position(|r| *r == row) {
        let card = s.ui.get::<KnobCard>(wave_tab::QLINKS[k]).unwrap();
        let text = grain_tab::value_text(&s.state.region, row);
        let (value, sub) = grain_tab::split_unit(&text);
        assert_eq!((&card.value, &card.sub), (&value, &sub), "{row:?}'s card");
        return text;
    }
    open_for(s, row);
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    list.rows[index(s, row)].value.clone()
}

/// Opens part `part` of the Sound section.
fn open_sound(s: &mut Sample, part: Part) {
    let now = s.tab.sound().part();
    let i = |p: Part| Part::ALL.iter().position(|x| *x == p).unwrap() as i32;
    if now != part {
        let ev = UiEvent {
            widget: sound_panel::SOUND.into(),
            kind: EventKind::Step {
                index: 0,
                delta: i(part) - i(now),
            },
        };
        let st = s.state.clone();
        assert!(s.tab.action(&mut s.ui, &ev, &st).is_none());
    }
    assert_eq!(s.tab.sound().part(), part);
}

/// A step of `row`: on the Grain section (its part opened first) a knob's
/// drag, a selector's arrow, a choice's button `delta` from the lit one, a
/// switch's tap (which flips it whatever `delta` is); the Q-Links' four on
/// the Sound section's Grain chip.
fn step(s: &mut Sample, row: Row, delta: i32) -> Option<WaveAction> {
    let ev = if on_qlinks(row) {
        open_sound(s, Part::Grain);
        let k = sound_panel::GRAIN_ROWS
            .iter()
            .position(|r| *r == row)
            .unwrap();
        UiEvent {
            widget: sound_panel::SOUND.into(),
            kind: EventKind::Step {
                index: k + 1,
                delta,
            },
        }
    } else {
        open_for(s, row);
        region_ev(EventKind::Step {
            index: index(s, row),
            delta,
        })
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

/// Applies an action and shows the new state, as the app does (the tempo
/// kept).
fn apply(s: &mut Sample, a: &WaveAction) {
    a.apply(&mut s.project).unwrap();
    let bpm = s.state.bpm;
    s.state = WaveState::from_project(&s.project, PAD, 0, s.state.source.clone()).unwrap();
    s.state.bpm = bpm;
    s.tab.update(&mut s.ui, &s.state);
    let sound = sample::sound_state(&s.project, &s.state.source);
    s.tab.set_sound(&mut s.ui, sound);
}

fn step_apply(s: &mut Sample, row: Row, delta: i32) {
    let a = step(s, row, delta).unwrap_or_else(|| panic!("{row:?} {delta:+} changed nothing"));
    apply(s, &a);
}

fn qlink(s: &mut Sample, knob: usize, delta: i32, shift: bool) -> Option<WaveAction> {
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, knob, delta, shift)
}

fn gesture(a: &WaveAction) -> u64 {
    match a {
        WaveAction::Region { gesture, .. } => *gesture,
        WaveAction::Play(PlayAction::Pad { gesture, .. }) => *gesture,
        other => panic!("not an edit: {other:?}"),
    }
}

/// Hands the tab a touch on the wave.
fn on_wave(s: &mut Sample, kind: EventKind) -> Option<WaveAction> {
    let ev = UiEvent {
        widget: wave_tab::WAVE.into(),
        kind,
    };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

/// The marks on the wave: their labels, and which are faint.
fn marks(s: &Sample) -> Vec<(String, bool)> {
    let w = s.ui.get::<Waveform>(wave_tab::WAVE).unwrap();
    w.markers()
        .iter()
        .map(|m| (m.label.clone(), m.faint))
        .collect()
}

/// The grain window's tab named `label` on the wave: its index and frame.
fn tab(s: &Sample, label: &str) -> (usize, u64) {
    let w = s.ui.get::<Waveform>(wave_tab::WAVE).unwrap();
    w.markers()
        .iter()
        .enumerate()
        .find(|(_, m)| m.label == label)
        .map(|(i, m)| (i, m.frame))
        .unwrap_or_else(|| panic!("no {label} tab"))
}

fn window(s: &Sample) -> GrainSpan {
    let w = s.ui.get::<Waveform>(wave_tab::WAVE).unwrap();
    w.window().expect("the grain window is drawn")
}

fn note(s: &Sample) -> String {
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    list.panel.as_ref().unwrap().sections[0].note.clone()
}

/// The rows the Grain section's panel places, in its order (the parts'
/// switch left out).
fn placed(s: &Sample) -> Vec<Row> {
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    let rows = shown(s);
    list.panel
        .as_ref()
        .unwrap()
        .sections
        .iter()
        .flat_map(|sec| sec.lines.iter().flatten().map(|i| i.row))
        .filter(|k| *k != PARTS_ROW)
        .map(|k| rows[k - 1])
        .collect()
}

/// Every control of the Grain section inside the panel's band, none on
/// another.
fn fits(s: &Sample, what: &str) {
    let r = s.ui.rect_of(wave_tab::REGION).unwrap();
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert_eq!(l.controls.len(), l.rows.len(), "{what}");
    let placed: Vec<_> = (0..l.rows.len())
        .map(|k| {
            let c = l.control_rect(r, k).expect("on the panel");
            assert!(
                c.x >= r.x && c.right() <= r.right() && c.y >= r.y && c.bottom() <= r.bottom(),
                "{what}: {} inside the panel: {c:?} in {r:?}",
                l.rows[k].name
            );
            c
        })
        .collect();
    for (i, a) in placed.iter().enumerate() {
        for b in &placed[i + 1..] {
            let apart =
                a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
            assert!(apart, "{what}: {a:?} and {b:?} overlap");
        }
    }
    names_whole(s, what);
}

/// Every name on the Grain section reads whole, none cut with "…": a
/// knob's over it (on two lines where it has them), a selector's or a
/// choice's over it, a switch's beside it.
fn names_whole(s: &Sample, what: &str) {
    let r = s.ui.rect_of(wave_tab::REGION).unwrap();
    let l = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    let mut fonts = Fonts::embedded();
    for p in l.panel.as_ref().unwrap().place(r, &l.controls) {
        let name = &l.rows[p.row].name;
        match l.controls[p.row].look {
            Look::Knob { .. } => {
                let lines = panel::knob_name_lines(&mut fonts, name, p.rect.w, p.tall);
                assert!(
                    lines.iter().all(|x| !x.ends_with('…')),
                    "{what}: {name:?} cut to {lines:?} in {} px",
                    p.rect.w
                );
            }
            // Bold 14 px, left of the 46 px switch with 8 px between.
            Look::Toggle(_) => {
                let w = fonts.measure(FontId::Bold, 14.0, name);
                assert!(
                    w <= p.rect.w - 54,
                    "{what}: {name:?} ({w} px) beside its switch"
                );
            }
            // A choice's or selector's name is over it, when it has one.
            _ if p.control.y > p.rect.y => {
                let w = fonts.measure(FontId::Sans, 13.0, name);
                assert!(
                    w <= p.rect.w,
                    "{what}: {name:?} ({w} px) in {} px",
                    p.rect.w
                );
            }
            _ => {}
        }
    }
}

#[test]
fn the_grain_tool_shows_the_layers_grain_settings_and_its_window_on_the_wave() {
    let mut s = grain(true);
    assert_eq!(s.tab.tool(), Tool::Grain);
    assert_eq!(s.tab.hint(), wave_tab::GRAIN_HINT);
    let strip = s.ui.get::<ToolStrip>(wave_tab::TOOLS).unwrap();
    assert_eq!(strip.tools[strip.active], "Grain");
    let bar = s.ui.get::<PadBar>(wave_tab::BAR).unwrap();
    assert_eq!(bar.title, "Pad A5 · Break 92");
    assert!(
        bar.tabs.is_empty(),
        "no Grain sub-tab: it is a tool on the page, which has no sub-tabs"
    );
    for (row, value) in [
        (Row::Mode, "Granular"),
        (Row::GrainMode, "Cloud"),
        (Row::Quality, "Clean"),
        (Row::Scan, "0.25×"),
        (Row::Spray, "16 %"),
        (Row::SprayShape, "Even"),
        (Row::SizeSpray, "0 %"),
        (Row::PitchSpray, "±0.5 st"),
        (Row::LevelSpray, "0 %"),
        (Row::Shape, "Tukey 0.5"),
        (Row::Reverse, "15 %"),
        (Row::Spread, "70 %"),
        (Row::Quantise, "Minor"),
        (Row::Sync, "Off"),
        (Row::Position, "56.0 %"),
        (Row::Size, "150 ms"),
        (Row::Density, "120 /s"),
        (Row::Pitch, "-5 st"),
        (Row::Level, "+0.0 dB"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    // The Q-Links: Position, Size, Density, Pitch, as on the Grain page.
    let labels: Vec<String> = wave_tab::QLINKS
        .iter()
        .map(|q| s.ui.get::<KnobCard>(q).unwrap().label.clone())
        .collect();
    assert_eq!(
        labels,
        [
            "Q-Link 1 · Position",
            "Q-Link 2 · Size",
            "Q-Link 3 · Density",
            "Q-Link 4 · Grain pitch"
        ]
    );
    // The Grain section: the parts' switch over Grains' ten knobs.
    open(&mut s, GrainPart::Grains);
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert_eq!(list.right, "grains");
    assert_eq!(list.rows[PARTS_ROW].name, "Grain");
    let Look::Choice { options, on, .. } = &list.controls[PARTS_ROW].look else {
        panic!("the parts are a switch")
    };
    assert_eq!(
        options,
        &[
            "Grains",
            "Shape",
            "Grain filter",
            "Keys",
            "Grain LFO",
            "Grain env",
            "Presets"
        ]
    );
    assert_eq!(*on, 0);
    let panel = list.panel.as_ref().unwrap();
    let titles: Vec<&str> = panel.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Grain"]);
    assert_eq!(
        placed(&s),
        [
            Row::Scan,
            Row::Spray,
            Row::SizeSpray,
            Row::PitchSpray,
            Row::LevelSpray,
            Row::Timing,
            Row::Reverse,
            Row::Spread,
            Row::Level,
            Row::SprayShape
        ]
    );
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert!(matches!(
        list.controls[index(&s, Row::Scan)].look,
        Look::Knob { bipolar: true, .. }
    ));
    fits(&s, "Grains");
    // The note: the engine's own grains, playing 1.5 s after a hit.
    let (grains, head, _) = grains_after(&s.project, 1.5);
    assert!(
        (10..=30).contains(&grains.len()),
        "120/s of 150 ms grains: about 18, got {}",
        grains.len()
    );
    assert!(head.is_some_and(|h| h > 0.56), "{head:?}");
    assert_eq!(note(&s), format!("{} grains playing", grains.len()));
    // The window on the wave: from 56 % of the region, one 150 ms grain
    // long, the spray's 16 % round it.
    let r = s.state.region.clone();
    let len = r.end - r.start;
    let w = window(&s);
    assert_eq!(
        w.at,
        r.start + (f64::from(0.56f32) * len as f64).round() as u64
    );
    assert_eq!(w.len, u64::from(RATE) * 150 / 1000);
    assert_eq!(w.spray, (f64::from(0.16f32) * len as f64).round() as u64);
    assert_eq!(w.within, (r.start, r.end));
    // Every mark is drawn: Trim's faint, the window's two tabs bright.
    assert_eq!(
        marks(&s),
        [
            ("START".to_string(), true),
            ("END".into(), true),
            ("LOOP".into(), true),
            ("LOOP END".into(), true),
            ("GRAIN".into(), false),
            ("SIZE".into(), false)
        ]
    );
    assert_eq!(tab(&s, "GRAIN").1, w.at);
    assert_eq!(tab(&s, "SIZE").1, w.at + w.len);
}

#[test]
fn the_window_draws_the_spray_as_it_plays_and_the_note_says_when_it_is_thinned() {
    let mut s = grain(true);
    let (grains, now) = sample::cloud_after(&s.project, 1.5);
    let state = now.state.expect("it plays");
    let len = s.state.region.end - s.state.region.start;
    assert_eq!(
        window(&s).spray,
        (f64::from(state.spray) * len as f64).round() as u64
    );
    // A report of a moved spray and a thinned cloud.
    let st = s.state.clone();
    let moved = CloudNow {
        state: Some(CloudState {
            spray: 0.5,
            thinned: true,
        }),
        ..now
    };
    s.tab.set_cloud(&mut s.ui, &st, grains.len(), moved);
    assert_eq!(window(&s).spray, (0.5 * len as f64).round() as u64);
    assert_eq!(
        note(&s),
        format!("{} grains playing · thinned to fit 64", grains.len())
    );
    // The voice stops: the spray as set again, and the note says how to
    // hear it.
    s.tab.set_cloud(&mut s.ui, &st, 0, CloudNow::default());
    assert_eq!(
        window(&s).spray,
        (f64::from(0.16f32) * len as f64).round() as u64
    );
    assert_eq!(note(&s), "hit the pad to hear it");
    // Every sample screen fits its controls.
    for s in grain_samples(Fonts::embedded).unwrap() {
        fits(&s, s.name);
    }
}

#[test]
fn the_parts_switch_opens_each_part_and_every_part_fits_the_panel() {
    let mut s = grain(true);
    for (k, part) in [
        GrainPart::Shape,
        GrainPart::Filter,
        GrainPart::Keys,
        GrainPart::Lfo,
        GrainPart::Envelope,
        GrainPart::Presets,
        GrainPart::Grains,
    ]
    .into_iter()
    .enumerate()
    {
        open(&mut s, part);
        assert_eq!(placed(&s), part.rows(&s.state.region), "{part:?}");
        fits(&s, part.label());
        // The window stays on the wave; the Q-Links turn their four.
        let d = if k % 2 == 0 { 1 } else { -1 };
        let a = qlink(&mut s, 2, d, false).expect("density moves");
        apply(&mut s, &a);
        assert_eq!(s.tab.grain_part(), part);
        assert!(s
            .ui
            .get::<Waveform>(wave_tab::WAVE)
            .unwrap()
            .window()
            .is_some());
        s.tab.lift();
    }
    // In every grain mode, every row the Grain page showed is on one part
    // (the Q-Links' four on the cards), and each part fits.
    for mode in ["Cloud", "Stretch", "Repeat", "Tone"] {
        if mode != "Cloud" {
            step_apply(&mut s, Row::GrainMode, 1);
        }
        assert_eq!(row_value(&mut s, Row::GrainMode), mode);
        let m = s.state.region.grain_mode;
        let mut want: Vec<Row> = View::ALL
            .iter()
            .flat_map(|v| Row::shown(*v, m).iter().copied())
            .filter(|r| !on_qlinks(*r))
            .collect();
        want.sort_by_key(|r| format!("{r:?}"));
        want.dedup();
        let mut got = Vec::new();
        for part in GrainPart::ALL {
            open(&mut s, part);
            fits(&s, &format!("{mode} {part:?}"));
            got.extend(placed(&s));
        }
        let n = got.len();
        got.sort_by_key(|r| format!("{r:?}"));
        got.dedup();
        assert_eq!(got.len(), n, "{mode}: a row on two parts");
        assert_eq!(got, want, "{mode}");
    }
}

#[test]
fn a_layer_that_doesnt_play_grains_says_so_and_shapes_mode_turns_it_on() {
    let mut s = grain(false);
    assert_eq!(note(&s), "Mode is Loop: pick Granular in Shape");
    // The window still shows where grains would play.
    assert!(s
        .ui
        .get::<Waveform>(wave_tab::WAVE)
        .unwrap()
        .window()
        .is_some());
    // The Sound section's Grain chip waits, saying why.
    open_sound(&mut s, Part::Grain);
    let l = s.ui.get::<SettingList>(sound_panel::SOUND).unwrap();
    for k in 1..=4 {
        assert_eq!(l.why_faint(k), Some(sound_panel::NOT_GRAINS));
    }
    assert!(step(&mut s, Row::Size, 1).is_none());
    assert_eq!(row_value(&mut s, Row::Mode), "Loop");
    assert_eq!(s.tab.grain_part(), GrainPart::Shape);
    step_apply(&mut s, Row::Mode, 1);
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.mode,
        PlayMode::Granular
    );
    assert_eq!(note(&s), "hit the pad to hear it");
    let l = s.ui.get::<SettingList>(sound_panel::SOUND).unwrap();
    assert!(!l.controls[1].faint, "the Grain chip turns now");
    // And the engine now plays it as grains.
    let mut e = Engine::new(RATE, 4);
    e.set_view(EngineView::build(&s.project, RATE));
    e.apply(Event::NoteOn {
        pad: PAD as u8,
        velocity: 100,
    });
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    for _ in 0..40 {
        e.render(&mut l, &mut r);
    }
    assert!(e.live_grains() > 0);
    assert!(
        step(&mut s, Row::Mode, 1).is_none(),
        "Granular is the last mode"
    );
}

/// Live test 3: from a one-shot pad with no loop the Mode row stopped at
/// Toggle, because Loop without loop points is refused. Now it gives Loop
/// a loop, as Trim does, and goes on to Granular.
#[test]
fn the_mode_row_reaches_granular_from_a_pad_with_no_loop() {
    let mut s = grain(false);
    {
        let pad = s.project.pad(PAD).unwrap();
        let mut r = pad.layers[0].region.clone();
        r.mode = PlayMode::OneShot;
        r.sustain_loop = None;
        let a = WaveAction::Region {
            pad: PAD,
            layer: 0,
            region: Box::new(r),
            gesture: 1,
        };
        apply(&mut s, &a);
    }
    assert_eq!(row_value(&mut s, Row::Mode), "One shot");
    for want in [
        PlayMode::Gate,
        PlayMode::Toggle,
        PlayMode::Loop,
        PlayMode::Granular,
    ] {
        step_apply(&mut s, Row::Mode, 1);
        let r = &s.project.pad(PAD).unwrap().layers[0].region;
        assert_eq!(r.mode, want);
        if want == PlayMode::Loop {
            assert!(r.sustain_loop.is_some(), "Loop got a loop");
        }
    }
}

#[test]
fn arrows_step_each_row_and_a_run_of_taps_on_one_row_is_one_undo_step() {
    let mut s = grain(true);
    let steps = undo_steps(&s.project);
    let a = step(&mut s, Row::Spray, 1).unwrap();
    let g = gesture(&a);
    apply(&mut s, &a);
    let b = step(&mut s, Row::Spray, 1).unwrap();
    assert_eq!(gesture(&b), g, "the same row again is the same undo step");
    apply(&mut s, &b);
    assert_eq!(row_value(&mut s, Row::Spray), "26 %");
    assert_eq!(undo_steps(&s.project), steps + 1);
    let c = step(&mut s, Row::Reverse, -1).unwrap();
    assert_ne!(gesture(&c), g, "another row is a new step");
    apply(&mut s, &c);
    assert_eq!(undo_steps(&s.project), steps + 2);
    // The Sound section's Grain chip: a run on one knob is a step too.
    let d = step(&mut s, Row::Size, 1).unwrap();
    apply(&mut s, &d);
    let e = step(&mut s, Row::Size, 1).unwrap();
    assert_eq!(gesture(&e), gesture(&d));
    apply(&mut s, &e);
    assert_eq!(undo_steps(&s.project), steps + 3);
    for _ in 0..3 {
        s.project.undo();
    }
    let g = s.project.pad(PAD).unwrap().layers[0].region.grain;
    assert_eq!((g.spray, g.size_ms), (0.16, 150.0));

    let mut s = grain(true);
    for (row, delta, value) in [
        (Row::Scan, 1, "0.5×"),
        (Row::Scan, -3, "Frozen"),
        (Row::PitchSpray, 2, "±1.5 st"),
        (Row::SprayShape, 1, "Peaked"),
        (Row::SprayShape, -1, "Even"),
        (Row::SizeSpray, 3, "15 %"),
        (Row::SizeSpray, 30, "100 %"),
        (Row::LevelSpray, 10, "50 %"),
        (Row::LevelSpray, -11, "0 %"),
        // From the old Tukey window (as a trapezoid, 20 %) onto the knob.
        (Row::Shape, 1, "25 % · Trapezoid"),
        (Row::Shape, -9, "0 % · Square"),
        (Row::Shape, 12, "60 % · Hann"),
        (Row::Shape, 4, "80 % · Swell"),
        (Row::Shape, 9, "100 % · Pluck"),
        (Row::Spread, 7, "100 %"),
        (Row::Quantise, 1, "Pentatonic"),
        (Row::Quantise, -9, "Off"),
        (Row::Position, -1, "55.0 %"),
        (Row::Size, 1, "200 ms"),
        (Row::Size, -2, "120 ms"),
        (Row::Density, 1, "150 /s"),
        (Row::Density, 9, "200 /s"),
        (Row::Pitch, 3, "-2 st"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert!(
        step(&mut s, Row::Density, 1).is_none(),
        "200 grains a second is the most"
    );
    assert!(
        step(&mut s, Row::SprayShape, -1).is_none(),
        "Even is the first"
    );
    assert_eq!(
        s.project.pad(PAD).unwrap().layers[0].region.grain.window,
        GrainWindow::Shape { shape: 1.0 }
    );
    step_apply(&mut s, Row::Mode, -1);
    assert_eq!(row_value(&mut s, Row::Mode), "Loop");
}

#[test]
fn a_grain_mode_starts_from_its_own_settings_and_shows_its_rows() {
    let mut s = grain(true);
    let steps = undo_steps(&s.project);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Cloud");
    let (position, gain) = {
        let g = &s.state.region.grain;
        (g.position, g.gain)
    };
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Stretch");
    // Each part holds only the mode's rows.
    open(&mut s, GrainPart::Grains);
    assert_eq!(placed(&s), [Row::Scan, Row::Spread, Row::Level]);
    open(&mut s, GrainPart::Shape);
    assert_eq!(
        placed(&s),
        [Row::Mode, Row::GrainMode, Row::Quality, Row::Shape]
    );
    for (row, value) in [
        (Row::Scan, "1×"),
        (Row::Size, "90 ms"),
        (Row::Density, "45 /s"),
        (Row::Pitch, "+0 st"),
        (Row::Shape, "Hann"),
        (Row::Spread, "0 %"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    let g = s.state.region.grain;
    assert_eq!((g.position, g.gain), (position, gain), "kept");
    assert_eq!(
        (g.spray, g.pitch_spray, g.reverse, g.timing, g.scale),
        (0.0, 0.0, 0.0, 0.0, 0),
        "nothing random, so the grains stay in step"
    );
    // A stretch has no spray: the window is one grain wide (once the
    // engine plays it so).
    let st = s.state.clone();
    s.tab.set_cloud(&mut s.ui, &st, 0, CloudNow::default());
    assert_eq!(window(&s).spray, 0);
    // A sound with a tempo (from the Warp page) stretches in time.
    let mut r = s.state.region.clone();
    r.warp = Some(mpfree_sampler::model::Warp::new(92.0));
    assert_eq!(grain_tab::value_text(&r, Row::Scan), "1× · in time");

    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Repeat");
    assert_eq!(row_value(&mut s, Row::Density), "4 /beat");
    assert_eq!(row_value(&mut s, Row::Size), "1/4 beat");
    assert_eq!(row_value(&mut s, Row::Shape), "10 % · Trapezoid");
    let g = s.state.region.grain;
    assert_eq!((g.scan, g.timing), (0.0, 0.0), "the same spot, on the grid");
    // Its rate steps through beat divisions.
    step_apply(&mut s, Row::Density, 1);
    assert_eq!(row_value(&mut s, Row::Density), "6 /beat");

    // Back to a cloud: every row again, scattered a little.
    step_apply(&mut s, Row::GrainMode, -2);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Cloud");
    open(&mut s, GrainPart::Grains);
    assert_eq!(placed(&s).len(), 10);
    let g = s.state.region.grain;
    assert!(g.sync.is_none() && g.spray > 0.0 && g.spread > 0.0 && g.timing > 0.0);
    // Each run of taps on a row is one undo step (opening another part
    // ends it): Stretch, Repeat, the Density step, Cloud. Undo brings the
    // settings back.
    assert_eq!(undo_steps(&s.project), steps + 4);
    for _ in 0..4 {
        s.project.undo();
    }
    let r = &s.project.pad(PAD).unwrap().layers[0].region;
    assert_eq!(r.grain_mode, GrainMode::Cloud);
    assert_eq!(r.grain.spray, 0.16);
}

#[test]
fn the_keys_part_sets_key_tracking_and_glide() {
    let mut s = grain(true);
    for (row, delta, value) in [
        (Row::KeySize, 0, "Off"),
        (Row::KeyScan, 0, "Off"),
        (Row::Glide, 0, "Off"),
        (Row::KeySize, 1, "On"),
        (Row::KeyScan, 1, "On"),
        // A tap on a switch flips it.
        (Row::KeyScan, 1, "Off"),
        (Row::Glide, 4, "100 ms"),
        (Row::Glide, 20, "5.00 s"),
        (Row::Glide, -7, "150 ms"),
    ] {
        if delta != 0 {
            step_apply(&mut s, row, delta);
        }
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert_eq!(s.tab.grain_part(), GrainPart::Keys);
    let r = &s.state.region;
    assert!(r.grain_keys.size && !r.grain_keys.scan);
    assert_eq!(r.grain_glide_ms, 150.0);
    // - and + flip a switch too (as a tap does).
    step_apply(&mut s, Row::KeySize, -1);
    assert_eq!(row_value(&mut s, Row::KeySize), "Off");
    step_apply(&mut s, Row::KeySize, 1);
    // A tone's size follows its note anyway: no row for it, and the
    // settings stay across modes.
    step_apply(&mut s, Row::GrainMode, 3);
    assert!(!Row::KeySize.used_in(GrainMode::Tone));
    assert_eq!(row_value(&mut s, Row::Glide), "150 ms");
    assert!(!placed(&s).contains(&Row::KeySize));
    assert!(s.state.region.grain_keys.size);
}

#[test]
fn quality_is_three_buttons_clean_tape_and_lofi_in_every_mode() {
    let mut s = grain(true);
    assert_eq!(row_value(&mut s, Row::Quality), "Clean");
    assert_eq!(s.tab.grain_part(), GrainPart::Shape);
    let buttons = |s: &Sample| {
        let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
        match &list.controls[index(s, Row::Quality)].look {
            Look::Choice { options, on, .. } => (options.clone(), *on),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        buttons(&s),
        (vec!["Clean".into(), "Tape".into(), "Lo-fi".into()], 0)
    );
    // A tap on a button: a step from the lit one to it.
    for (delta, value, lit) in [
        (1, "Tape", 1),
        (1, "Lo-fi", 2),
        (-2, "Clean", 0),
        (2, "Lo-fi", 2),
    ] {
        step_apply(&mut s, Row::Quality, delta);
        assert_eq!(row_value(&mut s, Row::Quality), value, "{delta:+}");
        assert_eq!(buttons(&s).1, lit);
    }
    // Lo-fi is the last: it stops there.
    assert!(step(&mut s, Row::Quality, 1).is_none());
    assert_eq!(s.state.region.grain.quality, GrainQuality::LoFi);
    // Every grain mode has it, and keeps it.
    for (k, mode) in [GrainMode::Stretch, GrainMode::Repeat, GrainMode::Tone]
        .into_iter()
        .enumerate()
    {
        assert!(Row::Quality.used_in(mode), "{mode:?}");
        step_apply(&mut s, Row::GrainMode, 1);
        assert_eq!(s.state.region.grain_mode, mode, "step {k}");
        assert_eq!(row_value(&mut s, Row::Quality), "Lo-fi", "{mode:?}");
        assert!(placed(&s).contains(&Row::Quality), "{mode:?}");
    }
    // A preset keeps it.
    let a = tap_preset(&mut s, 0).expect("a preset");
    apply(&mut s, &a);
    assert_eq!(s.state.region.grain.quality, GrainQuality::LoFi);
}

#[test]
fn the_lfo_and_envelope_parts_set_the_movement() {
    let mut s = grain(true);
    for (row, value) in [
        (Row::LfoTo, "Off"),
        (Row::LfoShape, "Sine"),
        (Row::LfoRate, "1.0 Hz"),
        (Row::LfoSync, "Off"),
        (Row::LfoDepth, "+50 %"),
        (Row::EnvTo, "Off"),
        (Row::EnvDepth, "+50 %"),
        (Row::EnvAttack, "1.00 s"),
        (Row::EnvDecay, "1.00 s"),
        (Row::EnvSustain, "100 %"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    assert_eq!(note(&s), "off: pick what it moves", "the envelope's");
    open(&mut s, GrainPart::Lfo);
    assert_eq!(note(&s), "off: pick what it moves", "the LFO's");
    for (row, delta, value) in [
        (Row::LfoTo, 1, "Position"),
        (Row::LfoTo, 3, "Pitch"),
        (Row::LfoTo, 9, "Cutoff"),
        (Row::LfoTo, -1, "Spray"),
        (Row::LfoTo, -2, "Density"),
        (Row::LfoShape, 5, "Hold"),
        (Row::LfoShape, 9, "Drift"),
        (Row::LfoRate, 2, "2.0 Hz"),
        (Row::LfoRate, -6, "0.20 Hz"),
        (Row::LfoRate, 8, "4.0 Hz"),
        // Synced at 120 BPM: the cycle nearest 4 Hz, then a step faster.
        (Row::LfoSync, 1, "On"),
        (Row::LfoRate, 0, "1/2 beat"),
        (Row::LfoRate, -2, "2 beats"),
        (Row::LfoRate, 1, "1 beat"),
        (Row::LfoSync, 1, "Off"),
        (Row::LfoRate, 0, "2.0 Hz"),
        (Row::LfoDepth, -60, "-10 %"),
        (Row::LfoDepth, -200, "-100 %"),
        (Row::EnvTo, 2, "Size"),
        (Row::EnvDepth, 25, "+75 %"),
        (Row::EnvAttack, -1, "750 ms"),
        (Row::EnvAttack, -30, "0 ms"),
        (Row::EnvDecay, 2, "2.00 s"),
        (Row::EnvSustain, -10, "50 %"),
    ] {
        if delta != 0 {
            step_apply(&mut s, row, delta);
        }
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert_eq!(note(&s), "once, from each hit");
    open(&mut s, GrainPart::Lfo);
    assert_eq!(note(&s), "over and over, from each hit");
    let g = s.state.region.grain;
    assert_eq!(
        (
            g.lfo.target,
            g.lfo.shape,
            g.lfo.rate_hz,
            g.lfo.beats,
            g.lfo.depth
        ),
        (MotionTarget::Density, LfoShape::Drift, 2.0, None, -1.0)
    );
    assert_eq!(
        (
            g.env.target,
            g.env.depth,
            g.env.attack_ms,
            g.env.decay_ms,
            g.env.sustain
        ),
        (MotionTarget::Size, 0.75, 0.0, 2000.0, 0.5)
    );
    // Every grain mode shows them, and keeps them: a tone's density is its
    // note.
    step_apply(&mut s, Row::GrainMode, 3);
    for part in [GrainPart::Lfo, GrainPart::Envelope] {
        open(&mut s, part);
        assert_eq!(placed(&s).len(), 5, "{part:?}");
    }
    assert_eq!(s.state.region.grain.lfo, g.lfo);
    assert_eq!(s.state.region.grain.env, g.env);
    assert_eq!(row_value(&mut s, Row::LfoTo), "Note");
}

/// Taps preset `k`'s button (0 to 7) on the Presets part, as the panel
/// reports it: a step from its line's lit button (or the line's end) to it.
fn tap_preset(s: &mut Sample, k: usize) -> Option<WaveAction> {
    let row = if k < 4 { Row::Preset } else { Row::PresetMore };
    open_for(s, row);
    let index = index(s, row);
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    let Look::Choice { on, .. } = list.controls[index].look else {
        panic!("a preset's line is a choice")
    };
    let ev = region_ev(EventKind::Step {
        index,
        delta: (k % 4) as i32 - on as i32,
    });
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st)
}

/// The preset buttons' two lines: their names and the one lit (4: none).
fn preset_lines(s: &Sample) -> Vec<(Vec<String>, usize)> {
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    [Row::Preset, Row::PresetMore]
        .iter()
        .map(|r| match &list.controls[index(s, *r)].look {
            Look::Choice { options, on, .. } => (options.clone(), *on),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn the_presets_part_puts_a_starting_point_on_the_pad() {
    let mut s = grain(true);
    let steps = undo_steps(&s.project);
    let (position, gain) = {
        let g = &s.state.region.grain;
        (g.position, g.gain)
    };
    open(&mut s, GrainPart::Presets);
    assert_eq!(
        preset_lines(&s),
        vec![
            (
                vec![
                    "Glass pad".to_string(),
                    "Frozen choir".into(),
                    "Tape stretch".into(),
                    "Stutter 1/16".into()
                ],
                4
            ),
            (
                vec![
                    "Shimmer fifths".to_string(),
                    "Grit cloud".into(),
                    "Swarm".into(),
                    "Sub drone".into()
                ],
                4
            )
        ],
        "none lit: the pad's settings are its own"
    );
    assert_eq!(
        note(&s),
        "a starting point: the sound, position, level and quality stay"
    );
    assert_eq!(row_value(&mut s, Row::Preset), "—");
    let a = tap_preset(&mut s, 0).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Preset), "Glass pad");
    assert_eq!(row_value(&mut s, Row::Pitch), "+12 st");
    assert_eq!(preset_lines(&s)[0].1, 0);
    let presets = mpfree_sampler::grain_presets::presets();
    assert_eq!(note(&s), presets[0].about);
    // Each tap puts on that one, in its mode; the part stays open.
    for (k, name, mode) in [
        (2, "Tape stretch", "Stretch"),
        (3, "Stutter 1/16", "Repeat"),
        (7, "Sub drone", "Tone"),
    ] {
        let a = tap_preset(&mut s, k).unwrap();
        apply(&mut s, &a);
        assert_eq!(row_value(&mut s, Row::Preset), name);
        assert_eq!(grain_tab::value_text(&s.state.region, Row::GrainMode), mode);
        assert_eq!(s.tab.grain_part(), GrainPart::Presets);
        let lit = preset_lines(&s);
        assert_eq!(
            (lit[0].1, lit[1].1),
            if k < 4 { (k, 4) } else { (4, k - 4) }
        );
    }
    assert_eq!(
        grain_tab::value_text(&s.state.region, Row::Density),
        "A1 · 55.0 Hz"
    );
    assert_eq!(grain_tab::value_text(&s.state.region, Row::Glide), "150 ms");
    let g = s.state.region.grain;
    assert_eq!((g.position, g.gain), (position, gain), "kept");
    // - / + step from one to the next; past the last, nothing.
    assert!(
        step(&mut s, Row::PresetMore, 1).is_none(),
        "Sub drone is the last"
    );
    step_apply(&mut s, Row::PresetMore, -1);
    assert_eq!(row_value(&mut s, Row::Preset), "Swarm");
    // A turn of any control leaves the preset as a starting point.
    step_apply(&mut s, Row::Spray, 1);
    assert_eq!(row_value(&mut s, Row::Preset), "—");
    // The run of taps was one undo step, the turn another.
    assert_eq!(undo_steps(&s.project), steps + 2);
    s.project.undo();
    s.project.undo();
    let r = &s.project.pad(PAD).unwrap().layers[0].region;
    assert_eq!(r.grain_mode, GrainMode::Cloud);
    assert_eq!(grain_tab::value_text(r, Row::Preset), "—");
    // From none, a step down picks the last.
    let r = grain_tab::stepped(r, Row::Preset, -1, 120.0);
    assert_eq!(grain_tab::value_text(&r, Row::Preset), "Sub drone");
}

#[test]
fn the_filter_part_colours_each_grain_and_a_new_mode_keeps_it() {
    let mut s = grain(true);
    for (row, value) in [
        (Row::Filter, "Off"),
        (Row::Cutoff, "2.00 kHz"),
        (Row::Resonance, "0 %"),
        (Row::FilterSpray, "Off"),
    ] {
        assert_eq!(row_value(&mut s, row), value, "{row:?}");
    }
    assert_eq!(note(&s), "off: pick a pass");
    for (row, delta, value) in [
        (Row::Filter, 1, "Low pass"),
        (Row::Filter, 9, "High pass"),
        (Row::Filter, -1, "Band pass"),
        // A sixth of an octave a step.
        (Row::Cutoff, 6, "4.00 kHz"),
        (Row::Cutoff, -12, "1.00 kHz"),
        (Row::Cutoff, -200, "20.0 Hz"),
        (Row::Cutoff, 200, "20.00 kHz"),
        (Row::Cutoff, -18, "2.50 kHz"),
        (Row::Resonance, 9, "45 %"),
        (Row::FilterSpray, 10, "±1.5 oct"),
        (Row::FilterSpray, 20, "±3 oct"),
    ] {
        step_apply(&mut s, row, delta);
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
    assert_eq!(note(&s), "set as each grain starts");
    let f = s.state.region.grain.filter;
    assert_eq!(
        f,
        GrainFilter {
            kind: GrainFilterKind::Band,
            cutoff_hz: 2_500.0,
            resonance: 0.45,
            spray: 1.0,
        }
    );
    // The choice lights the pass.
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert!(matches!(
        &list.controls[index(&s, Row::Filter)].look,
        Look::Choice { on: 2, .. }
    ));
    // The LFO can sweep it.
    step_apply(&mut s, Row::LfoTo, 6);
    assert_eq!(row_value(&mut s, Row::LfoTo), "Cutoff");
    // A stretch keeps the filter but not its spray (nothing random).
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(s.state.region.grain.filter, GrainFilter { spray: 0.0, ..f });
    assert_eq!(s.state.region.grain.lfo.target, MotionTarget::Cutoff);
    assert!(!Row::FilterSpray.used_in(GrainMode::Stretch));
    // A repeat and a tone keep all of it.
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::FilterSpray), "Off");
    step_apply(&mut s, Row::FilterSpray, 2);
    step_apply(&mut s, Row::GrainMode, 1);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Tone");
    assert_eq!(row_value(&mut s, Row::FilterSpray), "±0.3 oct");
}

#[test]
fn a_tone_plays_a_note_that_steps_by_semitones_and_keeps_its_cycles() {
    let mut s = grain(true);
    step_apply(&mut s, Row::GrainMode, 3);
    assert_eq!(row_value(&mut s, Row::GrainMode), "Tone");
    assert!(
        step(&mut s, Row::GrainMode, 1).is_none(),
        "Tone is the last mode"
    );
    assert_eq!(row_value(&mut s, Row::Density), "A2 · 110 Hz");
    assert_eq!(row_value(&mut s, Row::Size), "36 ms · 4.0 cycles");
    let g = s.state.region.grain;
    assert_eq!(
        (g.timing, g.scan, g.spray),
        (0.0, 0.0, 0.0),
        "regular and still"
    );
    for (row, delta, value) in [
        (Row::Density, 3, "C3 · 131 Hz"),
        (Row::Size, 0, "31 ms · 4.0 cycles"),
        (Row::Size, 2, "38 ms · 5.0 cycles"),
        (Row::Density, 12, "C4 · 262 Hz"),
        (Row::Size, 0, "19 ms · 5.0 cycles"),
        (Row::Density, -60, "A0 · 27.5 Hz"),
        (Row::Density, 99, "B5 · 988 Hz"),
        (Row::Size, -20, "1 ms · 1.0 cycles"),
    ] {
        if delta != 0 {
            step_apply(&mut s, row, delta);
        }
        assert_eq!(row_value(&mut s, row), value, "{row:?} {delta:+}");
    }
}

#[test]
fn tempo_sync_locks_density_and_size_to_beats_and_level_sets_the_cloud() {
    let mut s = grain(true);
    let before = undo_steps(&s.project);
    // 120 grains a second of 150 ms at 120 BPM: 60 a beat, 0.3 of a beat.
    step_apply(&mut s, Row::Sync, 1);
    assert_eq!(row_value(&mut s, Row::Sync), "Density");
    assert_eq!(
        row_value(&mut s, Row::Density),
        "32 /beat",
        "the most a beat"
    );
    assert_eq!(row_value(&mut s, Row::Size), "150 ms");
    step_apply(&mut s, Row::Sync, 1);
    assert_eq!(row_value(&mut s, Row::Sync), "Size");
    assert_eq!(row_value(&mut s, Row::Density), "120 /s", "free again");
    assert_eq!(
        row_value(&mut s, Row::Size),
        "1/4 beat",
        "nearest 0.3 of a beat"
    );
    step_apply(&mut s, Row::Sync, 1);
    assert_eq!(row_value(&mut s, Row::Sync), "Both");
    step_apply(&mut s, Row::Density, -3);
    assert_eq!(row_value(&mut s, Row::Density), "12 /beat");
    step_apply(&mut s, Row::Size, 1);
    assert_eq!(row_value(&mut s, Row::Size), "1/2 beat");
    let g = s.project.pad(PAD).unwrap().layers[0].region.grain;
    let t = g.sync.unwrap();
    assert_eq!((t.grains_per_beat, t.size_beats), (Some(12.0), Some(0.5)));
    assert_eq!(t.frames_per_beat, grain_tab::SAVED_FRAMES_PER_BEAT);
    assert_eq!((g.density, g.size_ms), (120.0, 150.0), "kept for later");
    // A synced size: the window is the tempo's length, and Q-Link 2 and
    // the SIZE tab step its beats.
    assert_eq!(size_ms_at(&g, s.state.bpm), 250.0);
    assert_eq!(window(&s).len, u64::from(RATE) / 4);
    s.state.bpm = 60.0;
    assert_eq!(size_ms_at(&g, s.state.bpm), 500.0);
    let a = qlink(&mut s, 1, 2, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Size), "2 beats");
    assert_eq!(row_value(&mut s, Row::Density), "12 /beat");
    assert_eq!(window(&s).len, u64::from(RATE) * 2, "2 beats at 60 BPM");
    s.tab.lift();
    let (size, at) = tab(&s, "SIZE");
    assert_eq!(at, window(&s).at + window(&s).len);
    let frame = window(&s).at + u64::from(RATE) * 9 / 10;
    let a = on_wave(&mut s, EventKind::Marker { index: size, frame }).unwrap();
    apply(&mut s, &a);
    s.tab.lift();
    assert_eq!(row_value(&mut s, Row::Size), "1 beat", "the nearest beats");
    // Level: the cloud's gain, half a dB a step.
    step_apply(&mut s, Row::Level, -4);
    assert_eq!(row_value(&mut s, Row::Level), "-2.0 dB");
    let gain = s.project.pad(PAD).unwrap().layers[0].region.grain.gain;
    assert!((gain - 0.794).abs() < 1e-3, "{gain}");
    step_apply(&mut s, Row::Level, 99);
    assert_eq!(row_value(&mut s, Row::Level), "+12.0 dB");
    // Off frees both, back to what they were.
    step_apply(&mut s, Row::Sync, -3);
    assert_eq!(row_value(&mut s, Row::Sync), "Off");
    assert_eq!(row_value(&mut s, Row::Size), "150 ms");
    assert_eq!(row_value(&mut s, Row::Density), "120 /s");
    assert!(s.project.pad(PAD).unwrap().layers[0]
        .region
        .grain
        .sync
        .is_none());
    assert!(step(&mut s, Row::Sync, -1).is_none(), "Off is the first");
    assert!(undo_steps(&s.project) > before);
    let a = grain_project().0.pad(PAD).unwrap().layers[0].region.clone();
    let mut b = a.clone();
    b.grain.gain = s.project.pad(PAD).unwrap().layers[0].region.grain.gain;
    b.grain.sync = Some(mpfree_sampler::model::TempoSync {
        frames_per_beat: 22_050.0,
        grains_per_beat: Some(4.0),
        size_beats: None,
    });
    assert_eq!(
        describe(&a, &b),
        "tempo sync Off -> Density, density 120 /s -> 4 /beat, level +0.0 dB -> +12.0 dB"
    );
}

#[test]
fn qlinks_turn_their_four_and_shift_turns_the_sprays() {
    let mut s = grain(true);
    let a = qlink(&mut s, 0, 4, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Position),
        "58.0 %",
        "half a per cent a detent"
    );
    let a = qlink(&mut s, 1, 8, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Size),
        "300 ms",
        "eight detents double it"
    );
    let a = qlink(&mut s, 1, -3, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Size),
        "297 ms",
        "SHIFT: a millisecond a detent"
    );
    let a = qlink(&mut s, 2, -8, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Density), "60 /s");
    let a = qlink(&mut s, 3, -40, false).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Pitch),
        "-36 st",
        "pitch stops at three octaves"
    );
    let a = qlink(&mut s, 0, 5, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::Spray),
        "21 %",
        "SHIFT + Q-Link 1: spray"
    );
    // The spray's row is picked on the panel, and its card lit.
    let list = s.ui.get::<SettingList>(wave_tab::REGION).unwrap();
    assert_eq!(list.selected, index(&s, Row::Spray));
    assert!(s.ui.get::<KnobCard>(wave_tab::QLINKS[0]).unwrap().focused);
    let a = qlink(&mut s, 3, 15, true).unwrap();
    apply(&mut s, &a);
    assert_eq!(
        row_value(&mut s, Row::PitchSpray),
        "±2.0 st",
        "SHIFT + Q-Link 4: pitch spray"
    );
    // A turn is one undo step until it rests.
    let g1 = gesture(&qlink(&mut s, 2, 1, false).unwrap());
    let g2 = gesture(&qlink(&mut s, 2, 1, false).unwrap());
    assert_eq!(g1, g2);
    s.tab.lift();
    assert_ne!(gesture(&qlink(&mut s, 2, 1, false).unwrap()), g1);
    assert!(
        qlink(&mut s, 2, 0, false).is_none(),
        "a touch on a knob changes nothing"
    );
}

#[test]
fn the_wheel_zooms_round_where_grains_start() {
    let mut s = grain(true);
    let at = window(&s).at as f64;
    let view = |s: &Sample| s.tab.view(&s.ui).unwrap();
    let x0 = view(&s).x_of(at);
    let fpp = view(&s).frames_per_px;
    let st = s.state.clone();
    s.tab.wheel(&mut s.ui, &st, 3);
    let v = view(&s);
    assert!(v.frames_per_px < fpp * 0.9, "zoomed in");
    assert!((v.x_of(at) - x0).abs() < 1.0, "GRAIN stays put");
}

#[test]
fn dragging_the_window_sets_position_and_size_and_a_tap_moves_the_start() {
    let mut s = grain(true);
    let (r0, r1) = (s.state.region.start, s.state.region.end);
    let len = r1 - r0;
    let steps = undo_steps(&s.project);
    // GRAIN drags where grains start; one drag is one undo step.
    let (grain_tab_at, _) = tab(&s, "GRAIN");
    let a = on_wave(
        &mut s,
        EventKind::Marker {
            index: grain_tab_at,
            frame: r0 + len / 4,
        },
    )
    .unwrap();
    let g = gesture(&a);
    apply(&mut s, &a);
    let b = on_wave(
        &mut s,
        EventKind::Marker {
            index: grain_tab_at,
            frame: r0 + len / 5,
        },
    )
    .unwrap();
    assert_eq!(gesture(&b), g, "one drag, one undo step");
    apply(&mut s, &b);
    s.tab.lift();
    assert_eq!(row_value(&mut s, Row::Position), "20.0 %");
    assert_eq!(tab(&s, "GRAIN").1, window(&s).at);
    // SIZE drags its end: the grains 20 ms long.
    let (size, _) = tab(&s, "SIZE");
    let to = window(&s).at + u64::from(RATE) / 50;
    let a = on_wave(
        &mut s,
        EventKind::Marker {
            index: size,
            frame: to,
        },
    )
    .unwrap();
    assert_ne!(gesture(&a), g);
    apply(&mut s, &a);
    s.tab.lift();
    assert_eq!(row_value(&mut s, Row::Size), "20 ms");
    assert_eq!(tab(&s, "SIZE").1, to);
    assert_eq!(undo_steps(&s.project), steps + 2);
    // A tap on the wave: grains start there (no audition with this tool).
    let a = on_wave(&mut s, EventKind::Frame(r0 + len * 3 / 4)).unwrap();
    assert!(matches!(a, WaveAction::Region { .. }));
    apply(&mut s, &a);
    s.tab.lift();
    assert_eq!(row_value(&mut s, Row::Position), "75.0 %");
    assert_eq!(row_value(&mut s, Row::Size), "20 ms", "the size stays");
    // Trim's marks are faint lines here, with no tab: the strip holds
    // only GRAIN and SIZE.
    let w = s.ui.get::<Waveform>(wave_tab::WAVE).unwrap();
    let tabs: Vec<&str> = w
        .markers()
        .iter()
        .filter(|m| m.tab)
        .map(|m| m.label.as_str())
        .collect();
    assert_eq!(tabs, ["GRAIN", "SIZE"]);
    assert!(w
        .markers()
        .iter()
        .filter(|m| !m.tab)
        .all(|m| m.faint && m.fixed));
    assert_eq!(s.state.region.start, r0);
}

#[test]
fn the_sound_sections_grain_chip_turns_the_four() {
    let mut s = grain(true);
    open_sound(&mut s, Part::Grain);
    let l = s.ui.get::<SettingList>(sound_panel::SOUND).unwrap();
    let rows: Vec<(&str, &str)> = l.rows[1..]
        .iter()
        .map(|r| (r.name.as_str(), r.value.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("Position", "56.0 %"),
            ("Size", "150 ms"),
            ("Density", "120 /s"),
            ("Grain pitch", "-5 st")
        ]
    );
    assert!(l.controls[1..]
        .iter()
        .all(|c| matches!(c.look, Look::Knob { .. }) && !c.faint));
    // A step there is a region edit, on the layer shown.
    let a = step(&mut s, Row::Pitch, 2).unwrap();
    assert!(
        matches!(
            a,
            WaveAction::Region {
                pad: PAD,
                layer: 0,
                ..
            }
        ),
        "the layer's region, as the Q-Links change it"
    );
    apply(&mut s, &a);
    assert_eq!(row_value(&mut s, Row::Pitch), "-3 st");
    let l = s.ui.get::<SettingList>(sound_panel::SOUND).unwrap();
    assert_eq!(l.rows[4].value, "-3 st");
    // The rest of the pad stays.
    let before = grain_project().0.pad(PAD).unwrap().clone();
    let after = s.project.pad(PAD).unwrap();
    assert_eq!(after.amp, before.amp);
    assert_eq!(
        describe(&before.layers[0].region, &after.layers[0].region),
        "pitch -5 st -> -3 st"
    );
}

#[test]
fn applying_keeps_the_rest_of_the_pad_and_says_what_changed() {
    let mut s = grain(true);
    let before = s.project.pad(PAD).unwrap().clone();
    let a = step(&mut s, Row::Spray, 1).unwrap();
    apply(&mut s, &a);
    let after = s.project.pad(PAD).unwrap();
    assert_eq!(after.amp, before.amp);
    assert_eq!(after.layers[0].region.start, before.layers[0].region.start);
    assert_eq!(
        describe(&before.layers[0].region, &after.layers[0].region),
        "spray 16 % -> 21 %"
    );
    let bad = WaveAction::Region {
        pad: PAD,
        layer: 3,
        region: Box::new(after.layers[0].region.clone()),
        gesture: 1,
    };
    assert!(bad.apply(&mut s.project).is_err(), "no layer 4");
}

#[test]
fn every_grain_sample_screen_draws() {
    let names: Vec<&str> = grain_samples(Fonts::embedded)
        .unwrap()
        .into_iter()
        .map(|mut s| {
            let mut c = SoftCanvas::new(sample::SCREEN.0, sample::SCREEN.1);
            s.ui.paint(&mut c);
            assert!(!s.about.is_empty(), "{}", s.name);
            assert_eq!(s.tab.tool(), Tool::Grain, "{}", s.name);
            s.name
        })
        .collect();
    assert_eq!(
        names,
        [
            "grain",
            "grain-off",
            "grain-thinned",
            "grain-movement",
            "grain-filter",
            "grain-shape",
            "grain-presets"
        ]
    );
    let (p, _) = grain_project();
    assert!(!grains_after(&p, 0.5).0.is_empty());
}
