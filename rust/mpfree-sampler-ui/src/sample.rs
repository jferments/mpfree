//! Sample screens with a real project behind them, for the workstation
//! preview and the tests. The sound is a made-up two-bar drum loop at
//! 92 BPM (no Akai content).

use std::sync::Arc;

use mpfree_sampler::model::{Fade, FadeCurve, Layer, Loop, LoopMode, Pad, PlayMode, Project, Region, Source};
use mpfree_ui::widgets::{Chip, Header, Label};
use mpfree_ui::{Align, Fonts, Theme, Tone, Ui};

use crate::source::SourceView;
use crate::wave_tab::{pad_label, WaveState, WaveTab, HINT};

/// The MPC One's screen, turned.
pub const SCREEN: (i32, i32) = (1280, 800);
/// The app's header tabs in the samples. The editor's tab name is a
/// placeholder until it is settled.
pub const APP_TABS: [&str; 4] = ["Play", "Sample", "Debug", "Prefs"];
pub const EDITOR_TAB: usize = 1;
/// The page name the samples add the Wave tab under.
pub const PAGE: &str = "Sample · Wave";
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

/// The app's part of the screen: the header and the hint line.
pub fn app_ui(fonts: Fonts, p: &Project) -> Ui {
    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::new(&APP_TABS);
    header.active = EDITOR_TAB;
    let undo = if p.can_undo() { "Undo" } else { "Nothing to undo" };
    // Chips are laid out from the right edge.
    header.chips = vec![Chip::new(undo, Tone::Good), Chip::new(format!("{BPM:.1} BPM"), Tone::Normal),
                        Chip::new(format!("Pad {}", pad_label(PAD)), Tone::Accent)];
    ui.add("header", header);
    ui.add("hint", Label::new(HINT, Tone::Dim, Align::Center));
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
