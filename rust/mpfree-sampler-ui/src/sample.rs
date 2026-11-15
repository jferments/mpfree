//! Sample screens with a real project behind them, for the workstation
//! preview and the tests. The sound is a made-up two-bar drum loop at
//! 92 BPM (no Akai content).

use std::sync::Arc;

use mpfree_sampler::engine::{Engine, EngineView, Event, GrainView};
use mpfree_sampler::model::{
    Ahdsr, ChopMode, ChopSet, Fade, FadeCurve, Filter, FilterKind, GrainParams, GrainWindow, Layer,
    LayerSelect, Loop, LoopMode, Pad, PlayMode, PressureRoute, PressureTarget, Project, Region,
    Source, Warp,
};
use mpfree_sampler::record::{self, Feed, Input, Recording};
use mpfree_ui::widgets::{Chip, Header, Label};
use mpfree_ui::{Align, Fonts, Theme, Tone, Ui};

use crate::auto_tab::{self, AutoSettings, AutoState, AutoTab, Onto, Running};
use crate::chop_tab::{self, ChopState, ChopTab};
use crate::grain_tab::{self, GrainState, GrainTab};
use crate::pads_tab::{self, PadsState, PadsTab};
use crate::play_tab::{self, PlayState, PlayTab};
use crate::rec_tab::{self, EndAt, RecState, RecTab, Skip, StartAt};
use crate::source::SourceView;
use crate::warp_tab::{self, WarpTab};
use crate::wave_tab::{pad_label, WaveState, WaveTab, HINT};

/// The MPC One's screen, turned.
pub const SCREEN: (i32, i32) = (1280, 800);
/// The app's header tabs, the one list mpfree-ui shares (the pad player's
/// layout spec, approved by Jay on 2026-11-11).
pub use mpfree_ui::widgets::APP_TABS;
pub const EDITOR_TAB: usize = 5;
/// The page names the samples add the Wave, Chop and Play tabs under.
pub const PAGE: &str = "Sample · Wave";
pub const CHOP_PAGE: &str = "Sample · Chop";
pub const PLAY_PAGE: &str = "Sample · Play";
pub const PADS_PAGE: &str = "Pads · Pad";
pub const GRAIN_PAGE: &str = "Sample · Grain";
pub const REC_PAGE: &str = "Sample · Rec";
pub const WARP_PAGE: &str = "Sample · Warp";
pub const AUTO_PAGE: &str = "Sample · Auto-sample";
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
                Some(hz) => {
                    (t * hz * std::f32::consts::TAU * (1.0 + 2.0 * (-t * 30.0).exp())).sin()
                }
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

/// The drum loop played loosely: the same two bars at [`BPM`], each hit
/// up to 25 ms early or late, as a drummer might.
pub fn loose_loop() -> Vec<f32> {
    let rate = f64::from(RATE);
    let beat = rate * 60.0 / BPM;
    let frames = (beat * 8.0).round() as usize;
    let tight = drum_loop();
    let mut out = vec![0.0f32; frames * 2];
    // Each eighth's slice of the tight loop, moved by its own amount.
    let mut seed = 0x9E37_79B9u32;
    let eighth = (beat / 2.0).round() as usize;
    for k in 0..16 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let off_ms = if k == 0 {
            0.0
        } else {
            (seed as f64 / f64::from(u32::MAX) * 2.0 - 1.0) * 25.0
        };
        let shift = (off_ms / 1000.0 * rate).round() as i64;
        for i in 0..eighth {
            let from = k * eighth + i;
            let to = from as i64 + shift;
            if from >= frames || to < 0 || to as usize >= frames {
                continue;
            }
            for c in 0..2 {
                out[to as usize * 2 + c] += tight[from * 2 + c];
            }
        }
    }
    out
}

/// A project with the loosely played loop on pad A5, whole and warped to
/// stretch at its tempo.
pub fn loose_project() -> (Project, Arc<SourceView>) {
    let mut p = Project::new();
    let source = Source::new("Loose break 92", RATE, 2, loose_loop()).expect("valid");
    let view = Arc::new(SourceView::analyse(&source));
    let mut region = Region::whole(&source);
    region.warp = Some(Warp::new(BPM as f32));
    p.add_source(source);
    p.set_pad(PAD, Some(Pad::new("Loose break", Layer::new(region))), None)
        .expect("the sample pad is valid");
    (p, view)
}

/// One sample screen of the Warp tab.
pub struct WarpSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: WarpTab,
    pub project: Project,
    pub state: WaveState,
}

impl WarpSample {
    /// Applies what the tab asked for and shows the project after it.
    pub fn apply(&mut self, a: Option<crate::wave_tab::WaveAction>) -> Result<(), String> {
        if let Some(a) = a {
            a.apply(&mut self.project).map_err(|e| format!("{e:?}"))?;
            self.state = WaveState::from_project(&self.project, PAD, 0, self.state.source.clone())
                .ok_or("no pad")?;
        }
        self.tab.update(&mut self.ui, &self.state);
        Ok(())
    }

    /// A touch on one of the tab's widgets.
    pub fn event(&mut self, widget: &str, kind: mpfree_ui::EventKind) -> Result<(), String> {
        let ev = mpfree_ui::UiEvent {
            widget: widget.into(),
            kind,
        };
        let st = self.state.clone();
        let a = self.tab.action(&mut self.ui, &ev, &st);
        self.apply(a)
    }
}

/// The Warp tab on `project`'s pad A5.
pub fn warp_sample(
    fonts: Fonts,
    (project, view): (Project, Arc<SourceView>),
) -> Result<WarpSample, String> {
    let state = WaveState::from_project(&project, PAD, 0, view).ok_or("no pad")?;
    let mut ui = app_ui_with(fonts, &project, warp_tab::HINT);
    let tab = WarpTab::add(&mut ui, WARP_PAGE, &state)?;
    ui.show(WARP_PAGE)?;
    Ok(WarpSample {
        name: "warp",
        about: "A loop played loosely, warped: its hits off the grid",
        ui,
        tab,
        project,
        state,
    })
}

/// The Warp tab's sample screens.
pub fn warp_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<WarpSample>, String> {
    let mut out = vec![warp_sample(fonts(), loose_project())?];

    // Pin hits on the eighths: every hit on its line, one pin picked.
    let mut s = warp_sample(fonts(), loose_project())?;
    s.name = "warp-pinned";
    s.about = "Pin hits on a 1/8 grid: each hit pinned to its eighth, the fifth pin picked";
    let grid_row = warp_tab::Row::Grid.index();
    s.event(
        warp_tab::LIST,
        mpfree_ui::EventKind::Step {
            index: grid_row,
            delta: -1,
        },
    )?;
    s.event(warp_tab::BUTTONS, mpfree_ui::EventKind::Tab(0))?;
    s.event(warp_tab::VIEW, mpfree_ui::EventKind::Select(4))?;
    s.tab
        .set_playheads(&mut s.ui, &[(f64::from(RATE) * 60.0 / BPM * 3.3) as u64]);
    out.push(s);

    // Zoomed in on that pin, nudged a little late by Q-Link 2.
    let mut s = warp_sample(fonts(), loose_project())?;
    s.name = "warp-zoomed";
    s.about =
        "Zoomed round the fifth pin (wheel), nudged 2/64 of a beat late by Q-Link 2, 1/16 grid";
    s.event(warp_tab::BUTTONS, mpfree_ui::EventKind::Tab(0))?;
    s.event(warp_tab::VIEW, mpfree_ui::EventKind::Select(4))?;
    let st = s.state.clone();
    for _ in 0..6 {
        s.tab.wheel(&mut s.ui, &st, 1);
    }
    let a = s.tab.qlink(&mut s.ui, &st, 1, 2, false);
    s.apply(a)?;
    out.push(s);

    // Not warped: the tab says how to start.
    let (mut project, view) = loose_project();
    project
        .edit_pad(PAD, None, |p| p.layers[0].region.warp = None)
        .map_err(|e| format!("{e:?}"))?;
    let mut s = warp_sample(fonts(), (project, view))?;
    s.name = "warp-off";
    s.about = "A layer that isn't warped: the tab says to turn Warp to Stretch";
    out.push(s);
    Ok(out)
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
    region.sustain_loop = Some(Loop {
        start: at(4.0),
        end: at(6.0),
        crossfade: 529,
        mode: LoopMode::Forward,
    });
    region.fade_in = Fade {
        frames: 441,
        curve: FadeCurve::Log,
    };
    region.fade_out = Fade {
        frames: 4410,
        curve: FadeCurve::Exp,
    };
    region.gain_db = 2.0;
    region.mode = PlayMode::Loop;
    p.add_source(source);
    p.set_pad(PAD, Some(Pad::new("Break 92", Layer::new(region))), None)
        .expect("the sample pad is valid");
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

/// One sample screen of the Grain tab.
pub struct GrainSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: GrainTab,
    pub project: Project,
    pub state: GrainState,
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

/// One sample screen of the Rec tab, with the input behind it.
pub struct RecSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: RecTab,
    pub state: RecState,
    pub feed: Feed,
    pub input: Input,
    pub recording: Option<Recording>,
}

/// A bar of 4/4 at [`BPM`], in frames.
pub fn bar_frames() -> f64 {
    4.0 * 60.0 / BPM * f64::from(RATE)
}

/// What the audio thread does with the line input, for `seconds`: the
/// drum loop (at `level`) into the input, in periods of 128 frames, with a
/// bar line on every bar of [`BPM`] from frame 0 while `rolling`.
pub fn feed_input(feed: &mut Feed, seconds: f64, level: f32, rolling: bool) {
    let drums = drum_loop();
    let loop_frames = (drums.len() / 2) as u64;
    let bar = bar_frames();
    let periods = (seconds * f64::from(RATE) / 128.0).round() as u64;
    let mut period = [0.0f32; 256];
    for _ in 0..periods {
        let base = feed.audio.written();
        if rolling {
            // The bar lines in this period, at whole frames.
            let mut k = (base as f64 / bar).ceil() as u64;
            while ((k as f64 * bar).round() as u64) < base + 128 {
                feed.bars.mark((k as f64 * bar).round() as u64);
                k += 1;
            }
        }
        feed.bars.set_bar(rolling.then_some(bar));
        for (i, x) in period.iter_mut().enumerate() {
            let f = (base + i as u64 / 2) % loop_frames;
            *x = level * drums[2 * f as usize + i % 2];
        }
        feed.audio.push(&period);
    }
}

/// The sources the sample screens offer, as the app names them.
pub fn rec_sources() -> Vec<String> {
    let mut v: Vec<String> = ["Line in", "Master", "Drums", "Synth", "Loop", "Return A"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.extend((1..=16).map(|p| format!("Pad {p}")));
    v
}

/// The Rec tab's sample screens.
pub fn rec_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<RecSample>, String> {
    let make = |name: &'static str, about: &'static str, live: bool| -> Result<RecSample, String> {
        let (project, _) = kit_project();
        let (feed, input) = record::input(120, RATE);
        let state = RecState {
            pad: PAD,
            pad_name: project.pad(PAD).map(|p| p.name.clone()),
            target: rec_tab::target(PAD, rec_tab::Onto::NextEmpty, |p| project.pad(p).is_some()),
            sources: rec_sources(),
            live,
            last: String::new(),
            can_flatten: project.pad(PAD).is_some(),
            flattening: None,
        };
        let mut ui = app_ui_with(fonts(), &project, rec_tab::HINT);
        let tab = RecTab::add(&mut ui, REC_PAGE, &state, RATE)?;
        ui.show(REC_PAGE)?;
        Ok(RecSample {
            name,
            about,
            ui,
            tab,
            state,
            feed,
            input,
            recording: None,
        })
    };
    let mut out = Vec::new();

    let mut s = make(
        "rec",
        "The drums coming in on the line input, the transport playing: Skip back would keep the last 10 s",
        true,
    )?;
    feed_input(&mut s.feed, 40.0, 0.4, true);
    s.tab.feed(&mut s.ui, &s.input, None, true, 0.05);
    out.push(s);

    let mut s = make(
        "rec-take",
        "A take of 4 bars that started on the next bar line, two bars in; Skip back set to 2 bars",
        true,
    )?;
    s.tab.settings.start = StartAt::NextBar;
    s.tab.settings.end = EndAt::Length;
    s.tab.settings.skip = Skip::Bars(2);
    feed_input(&mut s.feed, 20.0, 0.4, true);
    let mut rec = s.tab.settings.recording(&s.input, bar_frames());
    feed_input(
        &mut s.feed,
        bar_frames() * 2.4 / f64::from(RATE) + 0.5,
        0.4,
        true,
    );
    rec.poll(&s.input).map_err(|e| e.to_string())?;
    s.tab.feed(&mut s.ui, &s.input, Some(&rec), true, 0.05);
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    s.recording = Some(rec);
    out.push(s);

    let mut s = make(
        "rec-master",
        "Resampling the master: Source turned to Master, the last 4 bars ready for Skip back",
        true,
    )?;
    let st = s.state.clone();
    s.tab.settings.source = 1;
    s.tab.settings.skip = Skip::Bars(4);
    s.tab.update(&mut s.ui, &st);
    feed_input(&mut s.feed, 25.0, 0.7, true);
    s.tab.feed(&mut s.ui, &s.input, None, true, 0.05);
    out.push(s);

    let mut s = make(
        "rec-flatten",
        "Flatten: the pad picked resampled from its own channel as it plays, held for 1 s, onto the next empty pad",
        true,
    )?;
    let mut st = s.state.clone();
    st.flattening = Some(format!(
        "Flattening {} onto {}",
        chop_tab::pad_text(None, PAD),
        st.target
            .map_or_else(String::new, |p| chop_tab::pad_text(None, p))
    ));
    s.tab.update(&mut s.ui, &st);
    s.state = st;
    feed_input(&mut s.feed, 3.0, 0.0, false);
    let rec = s.tab.settings.flatten_recording(&s.input);
    feed_input(&mut s.feed, 1.4, 0.6, false);
    let mut rec = rec;
    rec.poll(&s.input).map_err(|e| e.to_string())?;
    s.tab.feed(&mut s.ui, &s.input, Some(&rec), true, 0.05);
    s.recording = Some(rec);
    out.push(s);

    let mut s = make(
        "rec-off",
        "The line input is off: the tab says where to turn it on",
        false,
    )?;
    feed_input(&mut s.feed, 5.0, 0.0, false);
    s.tab.feed(&mut s.ui, &s.input, None, false, 0.05);
    out.push(s);
    Ok(out)
}

/// One sample screen of the Rec tab's Auto-sample mode.
pub struct AutoSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub tab: AutoTab,
    pub project: Project,
    pub state: AutoState,
}

/// The synth patch the Auto-sample screens sample, as the app names it.
pub const AUTO_SYNTH: &str = "Poly Pluck";

/// The MIDI out the samples play external gear through.
pub const GEAR_OUT: &str = "MIDI Out (DIN)";

/// What the Auto-sample mode shows for `project` with `settings`. There is
/// a MIDI out for gear ([`GEAR_OUT`]) when the settings pick gear.
pub fn auto_state(
    project: &Project,
    settings: &AutoSettings,
    running: Option<Running>,
) -> AutoState {
    let has = |p: usize| project.pad(p).is_some_and(|pd| !pd.layers.is_empty());
    AutoState {
        pad: PAD,
        synth: Some(AUTO_SYNTH.to_string()),
        pads: 16,
        target: auto_tab::target(settings.onto, settings.notes(), 16, has),
        running,
        gear: settings.gear.map(|_| GEAR_OUT.to_string()),
        last: String::new(),
        rate: RATE,
    }
}

/// The Auto-sample mode's sample screens.
pub fn auto_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<AutoSample>, String> {
    let make = |name: &'static str,
                about: &'static str,
                project: Project,
                settings: AutoSettings,
                running: Option<Running>|
     -> Result<AutoSample, String> {
        let state = auto_state(&project, &settings, running);
        let mut ui = app_ui_with(fonts(), &project, auto_tab::HINT);
        let mut tab = AutoTab::add(&mut ui, AUTO_PAGE, &state)?;
        tab.bank = Some('A');
        tab.settings = settings;
        tab.update(&mut ui, &state);
        ui.show(AUTO_PAGE)?;
        Ok(AutoSample {
            name,
            about,
            ui,
            tab,
            project,
            state,
        })
    };
    // Every 6 semitones from C2 to C5 (7 notes), 3 layers of 2 round
    // robins: onto the kit's last 7 pads, which are empty.
    let seven = AutoSettings {
        hi: 72,
        every: 6,
        round_robins: 2,
        ..AutoSettings::default()
    };
    Ok(vec![
        make(
            "rec-auto",
            "Auto-sample on a new kit: the Synth tab's Poly Pluck from C2 to A5 every 3 semitones, 3 velocity \
             layers, onto all 16 pads",
            Project::new(),
            AutoSettings::default(),
            None,
        )?,
        make(
            "rec-auto-running",
            "Auto-sampling 7 notes × 3 layers × 2 round robins onto the kit's empty pads A10-A16, 20 takes of \
             42 done",
            kit_project().0,
            seven,
            Some(Running {
                done: 20,
                total: 42,
                note: Some(54),
            }),
        )?,
        make(
            "rec-auto-gear",
            "Auto-sampling external gear: its notes go out on MIDI channel 3 and its sound comes back on the \
             line input, C2 to C5 every 6 semitones onto the kit's empty pads",
            kit_project().0,
            AutoSettings {
                gear: Some(2),
                ..seven
            },
            None,
        )?,
        make(
            "rec-auto-full",
            "The kit has no 16 empty pads in a row: the bar says so and Start waits (Onto: Whole kit, or fewer notes)",
            kit_project().0,
            AutoSettings {
                onto: Onto::EmptyPads,
                ..AutoSettings::default()
            },
            None,
        )?,
    ])
}

/// The app's part of the screen: the header and the hint line.
pub fn app_ui(fonts: Fonts, p: &Project) -> Ui {
    app_ui_with(fonts, p, HINT)
}

pub fn app_ui_with(fonts: Fonts, p: &Project, hint: &str) -> Ui {
    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::app(APP_TABS[EDITOR_TAB]);
    let undo = if p.can_undo() {
        "Undo"
    } else {
        "Nothing to undo"
    };
    // Chips are laid out from the right edge.
    header.chips = vec![
        Chip::new(undo, Tone::Good),
        Chip::new(format!("{BPM:.1} BPM"), Tone::Normal),
        Chip::new(format!("Pad {}", pad_label(PAD)), Tone::Accent),
    ];
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
    Ok(Sample {
        name: "wave",
        about: "The whole loop: Start, End, the loop and the fades",
        ui,
        tab,
        project,
        state,
    })
}

/// The Chop tab on the sample project's pad, chopped by transients.
pub fn chop_sample(fonts: Fonts) -> Result<ChopSample, String> {
    let (project, view) = project();
    let state = ChopState::from_project(&project, PAD, 0, view).ok_or("no pad")?;
    let mut ui = app_ui_with(fonts, &project, chop_tab::HINT);
    let tab = ChopTab::add(&mut ui, CHOP_PAGE, &state)?;
    ui.show(CHOP_PAGE)?;
    Ok(ChopSample {
        name: "chop",
        about: "The pad's region chopped at its hits (Transient, 50 %)",
        ui,
        tab,
        project,
        state,
    })
}

/// The sample project with pad A5 played as three velocity layers (the
/// loop quieter, as it is, and louder: soft, mid and hard hits), shaped by
/// an envelope and a resonant low-pass that pressure opens.
pub fn layered_project() -> (Project, Arc<SourceView>) {
    let (mut p, view) = project();
    let region = p.pad(PAD).expect("the sample pad").layers[0].region.clone();
    let layer = |lo: u8, hi: u8, level_db: f32| Layer {
        vel_lo: lo,
        vel_hi: hi,
        level_db,
        ..Layer::new(region.clone())
    };
    p.set_pad(
        PAD,
        Some(Pad {
            layers: vec![
                layer(1, 63, -9.0),
                layer(64, 109, 0.0),
                layer(110, 127, 3.0),
            ],
            select: LayerSelect::All,
            polyphony: 4,
            mute_group: Some(1),
            amp: Ahdsr {
                attack_ms: 2.0,
                hold_ms: 0.0,
                decay_ms: 310.0,
                sustain: 0.6,
                release_ms: 900.0,
            },
            filter: Filter {
                kind: FilterKind::LowPass,
                poles: 2,
                cutoff_hz: 2400.0,
                resonance: 0.35,
                env_octaves: 0.5,
            },
            vel_to_start: 0.2,
            pressure: PressureRoute {
                target: PressureTarget::Filter,
                depth: 0.6,
            },
            ..Pad::new("Break 92", layer(1, 127, 0.0))
        }),
        None,
    )
    .expect("the layered pad is valid");
    (p, view)
}

/// The Play tab on `project`'s pad A5, shown.
fn play_on(fonts: Fonts, project: Project, view: Arc<SourceView>) -> Result<PlaySample, String> {
    let state = PlayState::from_project(&project, PAD, |_| view.clone()).ok_or("no pad")?;
    let mut ui = app_ui_with(fonts, &project, play_tab::HINT);
    let tab = PlayTab::add(&mut ui, PLAY_PAGE, &state)?;
    if let Some(h) = ui.get_mut::<Label>("hint") {
        h.text = tab.hint().into();
    }
    ui.show(PLAY_PAGE)?;
    Ok(PlaySample {
        name: "play",
        about: "",
        ui,
        tab,
        project,
        state,
    })
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
    (s.name, s.about) = (
        "play-plain",
        "One layer, the open envelope and no filter; Release picked and turned up by the wheel",
    );
    let index = play_tab::Row::ALL
        .iter()
        .position(|r| *r == play_tab::Row::Release)
        .unwrap_or(0);
    let ev = mpfree_ui::UiEvent {
        widget: play_tab::LIST.into(),
        kind: mpfree_ui::EventKind::Select(index),
    };
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

    // A kit mapped to keys: eight pads, each a sound named by its note,
    // spread halfway to each other; pad A5's zone picked, E4 held.
    let (mut p, view) = project();
    let pad = p.pad(PAD).ok_or("no pad")?.clone();
    let notes = [36u8, 43, 48, 55, 60, 67, 72, 79];
    for (k, &note) in notes.iter().enumerate() {
        let mut q = pad.clone();
        q.name = format!("Keys {}", mpfree_analysis::note_name(note));
        p.set_pad(k, Some(q), None).map_err(|e| format!("{e:?}"))?;
    }
    let roots: Vec<(usize, u8)> = notes.iter().copied().enumerate().collect();
    p.map_to_keys(&roots).map_err(|e| format!("{e:?}"))?;
    let mut s = play_on(fonts(), p, view)?;
    (s.name, s.about) = (
        "play-keys",
        "A kit mapped to keys: eight keygroups over the keyboard, pad A5's zone picked with its handles, E4 held",
    );
    if let Some(z) =
        s.ui.get_mut::<crate::keys_widgets::KeyZones>(play_tab::KEYS)
    {
        z.held = vec![64];
    }
    out.push(s);

    // The same layers overlapping by 12 velocities, crossfaded over them.
    let (mut p, view) = layered_project();
    if let Some(mut pad) = p.pad(PAD).cloned() {
        for (l, (lo, hi)) in pad.layers.iter_mut().zip([(1, 69), (58, 115), (104, 127)]) {
            (l.vel_lo, l.vel_hi) = (lo, hi);
        }
        pad.vel_fade = 12;
        p.set_pad(PAD, Some(pad), None)
            .map_err(|e| format!("{e:?}"))?;
    }
    let mut s = play_on(fonts(), p, view)?;
    (s.name, s.about) = (
        "play-fades",
        "Three velocity layers overlapping by 12, crossfaded over the overlaps; the last hit at 64",
    );
    s.tab.set_velocity(&mut s.ui, Some(64));
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
    let marks = [
        region.start,
        region.start + quarter,
        region.start + 2 * quarter,
        region.start + 3 * quarter,
        region.end,
    ];
    let c = ChopSet::from_markers("Break 92", &source, ChopMode::Grid, marks).expect("the chop");
    let template = p.pad(PAD).cloned();
    p.chop_to_pads(c, 1, 4, PAD + 1, template.as_ref())
        .expect("the slices");
    let kick = Source::new("Kick 58", RATE, 2, hit_sound(0.4, Some(58.0))).expect("the kick");
    let hat = Source::new("Hat closed", RATE, 2, hit_sound(0.08, None)).expect("the hat");
    let kick_region = Region::whole(&kick);
    p.add_source(kick);
    p.add_source(hat);
    p.set_pad(
        0,
        Some(Pad {
            mute_group: Some(1),
            ..Pad::new("Kick 58", Layer::new(kick_region))
        }),
        None,
    )
    .expect("the kick pad");
    (p, view)
}

/// The Pads tab's Pad view on the kit, pad `pad` picked.
pub fn pads_sample(fonts: Fonts, pad: usize) -> Result<PadsSample, String> {
    let (project, _) = kit_project();
    let mut state = PadsState::from_project(&project, pad);
    // The pad's mixer channel, as the app gives it: a compressor on the
    // loop's pad, none on the others.
    state.effects = Some(if pad == PAD {
        vec!["Compressor".into()]
    } else {
        Vec::new()
    });
    let mut ui = app_ui_with(fonts, &project, pads_tab::HINT);
    if let Some(h) = ui.get_mut::<Header>("header") {
        h.active = Header::app("Pads").active;
    }
    let mut tab = PadsTab::add(&mut ui, PADS_PAGE, &state)?;
    tab.kit = "Kit 1 · Breaks".into();
    tab.view = pads_tab::PAD_VIEW;
    tab.update(&mut ui, &state);
    ui.show(PADS_PAGE)?;
    Ok(PadsSample {
        name: "pads",
        about: "The kit's pads: the loop on A5 picked, its four slices on A6-A9, a kick on A1",
        ui,
        tab,
        project,
        state,
    })
}

/// The Pads tab's sample screens.
pub fn pads_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<PadsSample>, String> {
    let mut out = vec![pads_sample(fonts(), PAD)?];
    // An empty pad picked, the unused hat picked to put on it, A6 held.
    let mut s = pads_sample(fonts(), 10)?;
    (s.name, s.about) = (
        "pads-empty",
        "Pad A11 (no sound) picked, the hat picked to put on it, A6 held on the hardware",
    );
    let hat = s
        .state
        .sounds
        .iter()
        .position(|x| x.name == "Hat closed")
        .unwrap_or(0);
    let ev = mpfree_ui::UiEvent {
        widget: pads_tab::SOUNDS.into(),
        kind: mpfree_ui::EventKind::Select(hat),
    };
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
        let ev = mpfree_ui::UiEvent {
            widget: chop_tab::LIST.into(),
            kind: mpfree_ui::EventKind::Step { index, delta },
        };
        let st = s.state.clone();
        s.tab.action(&mut s.ui, &ev, &st);
    };
    let mut out = vec![chop_sample(fonts())?];

    // Beat, a slice every 8th, the third slice picked and zoomed round.
    let mut s = chop_sample(fonts())?;
    (s.name, s.about) = (
        "chop-beat",
        "Beat: a slice every 8th note, slice 3 picked (Q-Link 1), zoomed round it (wheel)",
    );
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
    (s.name, s.about) = (
        "chop-manual",
        "Grid of 4 put on pads A6-A9, then slice 2 split by hand (the method turns to Manual)",
    );
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
    let snap_row = crate::wave_tab::Row::ALL
        .iter()
        .position(|r| *r == crate::wave_tab::Row::Snap)
        .unwrap_or(0);
    let ev = mpfree_ui::UiEvent {
        widget: crate::wave_tab::REGION.into(),
        kind: mpfree_ui::EventKind::Step {
            index: snap_row,
            delta: 1,
        },
    };
    s.tab.action(&mut s.ui, &ev, &s.state);
    s.tab.qlink(&mut s.ui, &s.state.clone(), 2, 0, false);
    for _ in 0..8 {
        s.tab.wheel(&mut s.ui, &s.state.clone(), 1);
    }
    if let Some(a) = s.tab.qlink(&mut s.ui, &s.state.clone(), 2, 1, false) {
        a.apply(&mut s.project).map_err(|e| format!("{e:?}"))?;
        s.state =
            WaveState::from_project(&s.project, PAD, 0, s.state.source.clone()).ok_or("no pad")?;
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
    out.push(Sample {
        name: "wave-one-shot",
        about: "A one-shot without a loop, with a voice playing",
        ui,
        tab,
        project,
        state,
    });
    Ok(out)
}

/// The sample project with pad A5 played as a grain cloud: a dense cloud
/// of short grains from the middle of the loop, sprayed a little, read at
/// a quarter speed, pitched down, a few backwards, spread wide and kept to
/// a minor scale.
pub fn grain_project() -> (Project, Arc<SourceView>) {
    let (mut p, view) = project();
    p.edit_pad(PAD, None, |pd| {
        let r = &mut pd.layers[0].region;
        r.mode = PlayMode::Granular;
        r.grain = GrainParams {
            position: 0.56,
            spray: 0.16,
            size_ms: 150.0,
            density: 120.0,
            pitch: -5.0,
            pitch_spray: 0.5,
            scan: 0.25,
            window: GrainWindow::Tukey { taper: 0.5 },
            reverse: 0.15,
            spread: 0.7,
            scale: 0b0101_1010_1101,
            ..GrainParams::default()
        };
    })
    .expect("the grain pad is valid");
    (p, view)
}

/// The Grain tab on `project`'s pad A5, shown.
pub fn grain_on(
    fonts: Fonts,
    project: Project,
    view: Arc<SourceView>,
) -> Result<GrainSample, String> {
    let state = GrainState::from_project(&project, PAD, 0, |_| view.clone()).ok_or("no pad")?;
    let mut ui = app_ui_with(fonts, &project, grain_tab::HINT);
    let tab = GrainTab::add(&mut ui, GRAIN_PAGE, &state)?;
    ui.show(GRAIN_PAGE)?;
    Ok(GrainSample {
        name: "grain",
        about: "",
        ui,
        tab,
        project,
        state,
    })
}

/// What the engine plays of pad A5 `seconds` after a hit: its grains, its
/// reading place and how many grains play in all.
pub fn grains_after(project: &Project, seconds: f32) -> (Vec<GrainView>, Option<f32>, usize) {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(project, RATE));
    e.apply(Event::NoteOn {
        pad: PAD as u8,
        velocity: 100,
    });
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    for _ in 0..(seconds * RATE as f32 / 128.0) as usize {
        e.render(&mut l, &mut r);
    }
    let mut views = [GrainView::default(); 64];
    let n = e.grains_of(PAD as u8, 0, &mut views);
    let region = &project.pad(PAD).expect("pad A5").layers[0].region;
    let head = e
        .playheads()
        .find(|h| h.0 == PAD as u8 && h.1 == 0)
        .map(|h| ((h.2 as f64 - region.start as f64) / region.frames().max(1) as f64) as f32);
    (views[..n].to_vec(), head, e.live_grains())
}

/// The Grain tab's sample screens.
pub fn grain_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<GrainSample>, String> {
    let mut out = Vec::new();
    let (p, view) = grain_project();
    let mut s = grain_on(fonts(), p, view)?;
    (s.name, s.about) = (
        "grain",
        "A cloud from the middle of the loop, 1.5 s after a hit: the engine's own grains",
    );
    let (grains, head, live) = grains_after(&s.project, 1.5);
    let st = s.state.clone();
    s.tab.set_cloud(&mut s.ui, &st, &grains, head, live);
    out.push(s);

    // The loop as the Wave tab leaves it: it doesn't play grains yet.
    let (p, view) = project();
    let mut s = grain_on(fonts(), p, view)?;
    (s.name, s.about) = (
        "grain-off",
        "The pad still plays as a loop: the tab says how to make it play grains",
    );
    let st = s.state.clone();
    s.tab.update(&mut s.ui, &st);
    out.push(s);
    Ok(out)
}
