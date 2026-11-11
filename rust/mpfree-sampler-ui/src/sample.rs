//! Sample screens with a real project behind them, for the workstation
//! preview and the tests. The sound is a made-up two-bar drum loop at
//! 92 BPM (no Akai content).

use std::sync::Arc;

use mpfree_sampler::model::{
    Ahdsr, ChopMode, ChopSet, Fade, FadeCurve, Filter, FilterKind, Layer, LayerSelect, Loop, LoopMode, Pad, PlayMode,
    PressureRoute, PressureTarget, Project, Region, Source,
};
use mpfree_ui::widgets::{Chip, Header, Label};
use mpfree_ui::{Align, Fonts, Theme, Tone, Ui};

use crate::chop_tab::{self, ChopState, ChopTab};
use crate::pads_tab::{self, PadsState, PadsTab};
use crate::play_tab::{self, PlayState, PlayTab};
use crate::source::SourceView;
use crate::wave_tab::{pad_label, WaveState, WaveTab, HINT};

/// The MPC One's screen, turned.
pub const SCREEN: (i32, i32) = (1280, 800);
/// The app's header tabs in the samples, as the pad player's layout spec
/// orders them (Jay, 2026-11-11), until mpfree-ui shares one list.
pub const APP_TABS: [&str; 5] = ["Play", "Clips", "Synth", "Looper", "Sample"];
pub const EDITOR_TAB: usize = 4;
/// The page names the samples add the Wave, Chop and Play tabs under.
pub const PAGE: &str = "Sample · Wave";
pub const CHOP_PAGE: &str = "Sample · Chop";
pub const PLAY_PAGE: &str = "Sample · Play";
pub const PADS_PAGE: &str = "Sample · Pads";
pub const RATE: u32 = 44_100;
pub const BPM: f64 = 92.0;
/// The pad the samples edit (A5).
pub const PAD: usize = 4;

/// Two bars of drums at 92 BPM, stereo, interleaved.
pub fn drum_loop() -> Vec<f32> {
    let rate = f64::from(RATE);
    let beat = rate * 60.0 / BPM;
    let frames = (beat * 8.0).round() as usize;
    let mut out = vec![0.0f32; frames * 2];
    let mut seed = 0x2545_F491u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut hit = |at: f64, len_s: f64, level: f32, tone: Option<f32>, out: &mut Vec<f32>| {
        let start = at.round() as usize;
        let len = (len_s * rate) as usize;
        for i in 0..len.min(frames.saturating_sub(start)) {
            let t = i as f32 / RATE as f32;
            let env = (-t / (len_s as f32 / 4.0)).exp();
            let x = match tone {
                Some(hz) => (t * hz * std::f32::consts::TAU * (1.0 + 2.0 * (-t * 30.0).exp())).sin(),
                None => noise(),
            };
            out[(start + i) * 2] += level * env * x;
            out[(start + i) * 2 + 1] += level * env * x * 0.92;
        }
    };
    for k in 0..16 {
        let at = f64::from(k) * beat / 2.0;
        hit(at, 0.05, 0.22, None, &mut out); // hats on the eighths
        if k % 4 == 0 || k == 7 || k == 10 {
            hit(at, 0.35, 0.85, Some(58.0), &mut out); // kicks
        }
        if k % 4 == 2 {
            hit(at, 0.2, 0.6, None, &mut out); // snares
        }
    }
    out
}

/// A project with the drum loop on pad A5: trimmed a little, looping over
/// the second and third beats of bar 2, with short fades.
pub fn project() -> (Project, Arc<SourceView>) {
    let mut p = Project::new();
    let source = Source::new("Break 92", RATE, 2, drum_loop()).expect("the sample sound is valid");
    let view = Arc::new(SourceView::analyse(&source));
    let beat = f64::from(RATE) * 60.0 / BPM;
    let at = |b: f64| (b * beat).round() as u64;
    let mut region = Region::whole(&source);
    region.start = at(0.25);
    region.end = at(7.5);
    region.sustain_loop = Some(Loop { start: at(4.0), end: at(6.0), crossfade: 529, mode: LoopMode::Forward });
    region.fade_in = Fade { frames: 441, curve: FadeCurve::Log };
    region.fade_out = Fade { frames: 4410, curve: FadeCurve::Exp };
    region.gain_db = 2.0;
    region.mode = PlayMode::Loop;
    p.add_source(source);
    p.set_pad(PAD, Some(Pad::new("Break 92", Layer::new(region))), None).expect("the sample pad is valid");
    (p, view)
}

/// One sample screen.
pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: WaveTab,
    pub project: Project,
    pub state: WaveState,
}

/// One sample screen of the Chop tab.
pub struct ChopSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: ChopTab,
    pub project: Project,
    pub state: ChopState,
}

/// One sample screen of the Play tab.
pub struct PlaySample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: PlayTab,
    pub project: Project,
    pub state: PlayState,
}

/// One sample screen of the Pads tab.
pub struct PadsSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: PadsTab,
    pub project: Project,
    pub state: PadsState,
}

/// The app's part of the screen: the header and the hint line.
pub fn app_ui(fonts: Fonts, p: &Project) -> Ui {
    app_ui_with(fonts, p, HINT)
}

pub fn app_ui_with(fonts: Fonts, p: &Project, hint: &str) -> Ui {
    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::new(&APP_TABS);
    header.active = EDITOR_TAB;
    let undo = if p.can_undo() { "Undo" } else { "Nothing to undo" };
    // Chips are laid out from the right edge.
    header.chips = vec![Chip::new(undo, Tone::Good), Chip::new(format!("{BPM:.1} BPM"), Tone::Normal),
                        Chip::new(format!("Pad {}", pad_label(PAD)), Tone::Accent)];
    ui.add("header", header);
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Center));
    ui
}

/// The Wave tab on the sample project, shown.
pub fn wave_sample(fonts: Fonts) -> Result<Sample, String> {
    let (project, view) = project();
    let state = WaveState::from_project(&project, PAD, 0, view).ok_or("no pad")?;
    let mut ui = app_ui(fonts, &project);
    let tab = WaveTab::add(&mut ui, PAGE, &state)?;
    ui.show(PAGE)?;
    Ok(Sample { name: "wave", about: "The whole loop: Start, End, the loop and the fades", ui, tab, project, state })
}

/// The Chop tab on the sample project's pad, chopped by transients.
pub fn chop_sample(fonts: Fonts) -> Result<ChopSample, String> {
    let (project, view) = project();
    let state = ChopState::from_project(&project, PAD, 0, view).ok_or("no pad")?;
    let mut ui = app_ui_with(fonts, &project, chop_tab::HINT);
    let tab = ChopTab::add(&mut ui, CHOP_PAGE, &state)?;
    ui.show(CHOP_PAGE)?;
    Ok(ChopSample { name: "chop", about: "The pad's region chopped at its hits (Transient, 50 %)", ui, tab, project, state })
}

/// The sample project with pad A5 played as three velocity layers (the
/// loop quieter, as it is, and louder: soft, mid and hard hits), shaped by
/// an envelope and a resonant low-pass that pressure opens.
pub fn layered_project() -> (Project, Arc<SourceView>) {
    let (mut p, view) = project();
    let region = p.pad(PAD).expect("the sample pad").layers[0].region.clone();
    let layer = |lo: u8, hi: u8, level_db: f32| Layer { vel_lo: lo, vel_hi: hi, level_db, ..Layer::new(region.clone()) };
    p.set_pad(PAD, Some(Pad {
        layers: vec![layer(1, 63, -9.0), layer(64, 109, 0.0), layer(110, 127, 3.0)],
        select: LayerSelect::All,
        polyphony: 4,
        mute_group: Some(1),
        amp: Ahdsr { attack_ms: 2.0, hold_ms: 0.0, decay_ms: 310.0, sustain: 0.6, release_ms: 900.0 },
        filter: Filter { kind: FilterKind::LowPass, poles: 2, cutoff_hz: 2400.0, resonance: 0.35, env_octaves: 0.5 },
        vel_to_start: 0.2,
        pressure: PressureRoute { target: PressureTarget::Filter, depth: 0.6 },
        ..Pad::new("Break 92", layer(1, 127, 0.0))
    }), None).expect("the layered pad is valid");
    (p, view)
}

/// The Play tab on `project`'s pad A5, shown.
fn play_on(fonts: Fonts, project: Project, view: Arc<SourceView>) -> Result<PlaySample, String> {
    let state = PlayState::from_project(&project, PAD, |_| view.clone()).ok_or("no pad")?;
    let mut ui = app_ui_with(fonts, &project, play_tab::HINT);
    let tab = PlayTab::add(&mut ui, PLAY_PAGE, &state)?;
    ui.show(PLAY_PAGE)?;
    Ok(PlaySample { name: "play", about: "", ui, tab, project, state })
}

/// The Play tab's sample screens.
pub fn play_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<PlaySample>, String> {
    let mut out = Vec::new();
    let (p, view) = layered_project();
    let mut s = play_on(fonts(), p, view)?;
    (s.name, s.about) = ("play", "Three velocity layers, the last hit at 108, an envelope over the loop, a resonant low-pass");
    s.tab.set_velocity(&mut s.ui, Some(108));
    out.push(s);

    // The pad as the Wave tab leaves it: one layer, open envelope, no
    // filter; the list picked down to Release, turned up by the wheel.
    let (p, view) = project();
    let mut s = play_on(fonts(), p, view)?;
    (s.name, s.about) = ("play-plain", "One layer, the open envelope and no filter; Release picked and turned up by the wheel");
    let index = play_tab::Row::ALL.iter().position(|r| *r == play_tab::Row::Release).unwrap_or(0);
    let ev = mpfree_ui::UiEvent { widget: play_tab::LIST.into(), kind: mpfree_ui::EventKind::Select(index) };
    if let Some(list) = s.ui.get_mut::<crate::widgets::SettingList>(play_tab::LIST) {
        list.selected = index;
    }
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st);
    let view = st.sounds[0].view.clone();
    for _ in 0..3 {
        let st = s.state.clone();
        if let Some(a) = s.tab.wheel(&mut s.ui, &st, 1) {
            a.apply(&mut s.project).map_err(|e| format!("{e:?}"))?;
        }
        s.state = PlayState::from_project(&s.project, PAD, |_| view.clone()).ok_or("no pad")?;
        let st = s.state.clone();
        s.tab.update(&mut s.ui, &st);
    }
    out.push(s);
    Ok(out)
}

/// A made-up drum hit, stereo, interleaved: a pitched thump (`tone` Hz)
/// or noise, `len_s` long.
fn hit_sound(len_s: f32, tone: Option<f32>) -> Vec<f32> {
    let n = (len_s * RATE as f32) as usize;
    let mut seed = 0x1234_5678u32;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let x = match tone {
            Some(hz) => (t * hz * std::f32::consts::TAU * (1.0 + 2.0 * (-t * 30.0).exp())).sin(),
            None => seed as f32 / u32::MAX as f32 * 2.0 - 1.0,
        };
        let v = 0.8 * (-t / (len_s / 4.0)).exp() * x;
        out.extend_from_slice(&[v, v * 0.95]);
    }
    out
}

/// A kit: the drum loop on A5 and chopped into four on A6-A9, a kick on
/// A1, and a hat in the kit that no pad plays yet.
pub fn kit_project() -> (Project, Arc<SourceView>) {
    let (mut p, view) = project();
    let source = p.source(view.id).expect("the loop").clone();
    let region = p.pad(PAD).expect("the loop's pad").layers[0].region.clone();
    let quarter = (region.end - region.start) / 4;
    let marks = [region.start, region.start + quarter, region.start + 2 * quarter, region.start + 3 * quarter, region.end];
    let c = ChopSet::from_markers("Break 92", &source, ChopMode::Grid, marks).expect("the chop");
    let template = p.pad(PAD).cloned();
    p.chop_to_pads(c, 1, 4, PAD + 1, template.as_ref()).expect("the slices");
    let kick = Source::new("Kick 58", RATE, 2, hit_sound(0.4, Some(58.0))).expect("the kick");
    let hat = Source::new("Hat closed", RATE, 2, hit_sound(0.08, None)).expect("the hat");
    let kick_region = Region::whole(&kick);
    p.add_source(kick);
    p.add_source(hat);
    p.set_pad(0, Some(Pad { mute_group: Some(1), ..Pad::new("Kick 58", Layer::new(kick_region)) }), None)
        .expect("the kick pad");
    (p, view)
}

/// The Pads tab on the kit, pad `pad` picked.
pub fn pads_sample(fonts: Fonts, pad: usize) -> Result<PadsSample, String> {
    let (project, _) = kit_project();
    let state = PadsState::from_project(&project, pad);
    let mut ui = app_ui_with(fonts, &project, pads_tab::HINT);
    let mut tab = PadsTab::add(&mut ui, PADS_PAGE, &state)?;
    tab.kit = "Kit 1 · Breaks".into();
    tab.update(&mut ui, &state);
    ui.show(PADS_PAGE)?;
    Ok(PadsSample { name: "pads", about: "The kit's pads: the loop on A5 picked, its four slices on A6-A9, a kick on A1",
                    ui, tab, project, state })
}

/// The Pads tab's sample screens.
pub fn pads_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<PadsSample>, String> {
    let mut out = vec![pads_sample(fonts(), PAD)?];
    // An empty pad picked, the unused hat picked to put on it, A6 held.
    let mut s = pads_sample(fonts(), 10)?;
    (s.name, s.about) = ("pads-empty", "Pad A11 (no sound) picked, the hat picked to put on it, A6 held on the hardware");
    let hat = s.state.sounds.iter().position(|x| x.name == "Hat closed").unwrap_or(0);
    let ev = mpfree_ui::UiEvent { widget: pads_tab::SOUNDS.into(), kind: mpfree_ui::EventKind::Select(hat) };
    let st = s.state.clone();
    s.tab.action(&mut s.ui, &ev, &st);
    s.tab.update(&mut s.ui, &st);
    s.tab.set_lit(&mut s.ui, &st, PAD + 1, true);
    out.push(s);
    Ok(out)
}

/// The Chop tab's sample screens.
pub fn chop_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<ChopSample>, String> {
    use crate::chop_tab::{Button, Row};
    let step = |s: &mut ChopSample, row: Row, delta: i32| {
        let index = Row::ALL.iter().position(|r| *r == row).unwrap_or(0);
        let ev = mpfree_ui::UiEvent { widget: chop_tab::LIST.into(), kind: mpfree_ui::EventKind::Step { index, delta } };
        let st = s.state.clone();
        s.tab.action(&mut s.ui, &ev, &st);
    };
    let mut out = vec![chop_sample(fonts())?];

    // Beat, a slice every 8th, the third slice picked and zoomed round.
    let mut s = chop_sample(fonts())?;
    (s.name, s.about) = ("chop-beat", "Beat: a slice every 8th note, slice 3 picked (Q-Link 1), zoomed round it (wheel)");
    step(&mut s, Row::Method, 1);
    step(&mut s, Row::Amount, -1);
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, 0, 2, false);
    for _ in 0..4 {
        s.tab.wheel(&mut s.ui, &st, 1);
    }
    out.push(s);

    // Grid of 4 put on pads, then slice 2 split by hand: Manual.
    let mut s = chop_sample(fonts())?;
    (s.name, s.about) = ("chop-manual", "Grid of 4 put on pads A6-A9, then slice 2 split by hand (the method turns to Manual)");
    step(&mut s, Row::Method, 1);
    step(&mut s, Row::Method, 1);
    for _ in 0..4 {
        step(&mut s, Row::Amount, -1);
    }
    let st = s.state.clone();
    if let Some(a) = s.tab.press(&mut s.ui, &st, Button::ToPads) {
        a.apply(&mut s.project).map_err(|e| format!("{e:?}"))?;
        s.state = ChopState::from_project(&s.project, PAD, 0, st.source.clone()).ok_or("no pad")?;
        let st = s.state.clone();
        s.tab.update(&mut s.ui, &st);
    }
    let st = s.state.clone();
    s.tab.qlink(&mut s.ui, &st, 0, 1, false);
    s.tab.press(&mut s.ui, &st, Button::Split);
    out.push(s);
    Ok(out)
}

/// The sample screens the preview draws.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let mut out = Vec::new();
    out.push(wave_sample(fonts())?);

    // Zoomed in round the loop start, snapping to sixteenths, after a
    // Q-Link turn.
    let mut s = wave_sample(fonts())?;
    s.name = "wave-zoomed";
    s.about = "Zoomed round the loop start (wheel), snapping to 16ths, loop start moved a sixteenth on by Q-Link 3";
    let snap_row = crate::wave_tab::Row::ALL.iter().position(|r| *r == crate::wave_tab::Row::Snap).unwrap_or(0);
    let ev = mpfree_ui::UiEvent { widget: crate::wave_tab::REGION.into(), kind: mpfree_ui::EventKind::Step { index: snap_row, delta: 1 } };
    s.tab.action(&mut s.ui, &ev, &s.state);
    s.tab.qlink(&mut s.ui, &s.state.clone(), 2, 0, false);
    for _ in 0..8 {
        s.tab.wheel(&mut s.ui, &s.state.clone(), 1);
    }
    if let Some(a) = s.tab.qlink(&mut s.ui, &s.state.clone(), 2, 1, false) {
        a.apply(&mut s.project).map_err(|e| format!("{e:?}"))?;
        s.state = WaveState::from_project(&s.project, PAD, 0, s.state.source.clone()).ok_or("no pad")?;
        s.tab.update(&mut s.ui, &s.state);
    }
    out.push(s);

    // A one-shot with no loop: the loop's Q-Links have nothing to move.
    let (mut project, view) = project();
    project
        .edit_pad(PAD, None, |p| {
            let r = &mut p.layers[0].region;
            r.mode = PlayMode::OneShot;
            r.sustain_loop = None;
            r.gain_db = 0.0;
        })
        .map_err(|e| format!("{e:?}"))?;
    let state = WaveState::from_project(&project, PAD, 0, view).ok_or("no pad")?;
    let mut ui = app_ui(fonts(), &project);
    let tab = WaveTab::add(&mut ui, PAGE, &state)?;
    ui.show(PAGE)?;
    tab.set_playheads(&mut ui, &[(f64::from(RATE) * 60.0 / BPM * 2.6) as u64]);
    out.push(Sample { name: "wave-one-shot", about: "A one-shot without a loop, with a voice playing", ui, tab, project, state });
    Ok(out)
}
