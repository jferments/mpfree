//! The pad editor's Wave tab: where a pad's sound starts and ends, where it
//! loops, how it fades, and how loud, tuned and turned round it plays.
//!
//! - The **overview** shows the whole source with a box round what the
//!   waveform shows; a touch moves the box there.
//! - The **waveform** shows the region bright and the rest dimmed, the loop
//!   shaded, the fades as curves, bar and beat lines when the source has a
//!   tempo, and the Start, End and loop markers as tabs to drag. Dragging
//!   the wave scrolls it, pinching zooms, and a tap plays the pad.
//! - The **Region** list holds the settings: play mode, loop, crossfade,
//!   gain, tune, warp (to the project tempo by stretching or repitching,
//!   from the region's own tempo), reverse, channels, fades, and how
//!   markers snap.
//! - **Q-Links 1 to 4** move Start, End, Loop start and Loop end: one pixel
//!   of the current zoom per detent (so zooming in makes them finer), snapped
//!   in the direction of the turn; with SHIFT, one frame per detent, never
//!   snapped. The **wheel** zooms round the marker last moved or touched.
//!
//! The tab does not own the project. The app makes a [`WaveState`] for the
//! pad and layer on show ([`WaveState::from_project`]), calls
//! [`WaveTab::update`] when the project changes, and applies the
//! [`WaveAction`]s it gets back ([`WaveAction::apply`] for a region). A drag
//! or a turn is one undo step: the app calls [`WaveTab::lift`] when a finger
//! lifts or a Q-Link is let go, and the next edit starts a new step.
//!
//! The mockup is `mpfree-claude/sampler/mockups` (Wave). The header and the
//! hint line are the app's; the tab adds its own widgets and its layout.

use std::sync::Arc;

use mpfree_analysis::{nearest_zero_crossing, previous_zero_crossing};
use mpfree_sampler::model::{
    ChannelPick, Fade, Loop, LoopMode, ModelError, PlayMode, Project, Region, SourceId, Warp,
    WarpMethod, WARP_BPM,
};
use mpfree_ui::layout;
use mpfree_ui::wave::{Beats, Marker, MarkerKind, WaveOverview, WaveView, Waveform};
use mpfree_ui::{Color, EventKind, Rect, Ui, UiEvent};

use crate::source::SourceView;
use crate::widgets::{InfoCard, KnobCard, PadBar, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-bar";
pub const OVERVIEW: &str = "smp-overview";
pub const WAVE: &str = "smp-wave";
pub const REGION: &str = "smp-region";
pub const QLINKS: [&str; 4] = ["smp-q1", "smp-q2", "smp-q3", "smp-q4"];
pub const WHEEL: &str = "smp-wheel";

/// The tab's page: the app's header, the pad bar, the overview, the
/// waveform beside the Region list, the Q-Link cards and the wheel card,
/// and the app's hint line.
pub const LAYOUT: &str =
    "col(header:60, smp-bar:60, smp-overview:56, row(smp-wave:*, smp-region:360):*, \
                          row(smp-q1, smp-q2, smp-q3, smp-q4, smp-wheel:210):88, hint:26)";

/// The pad editor's tabs: Pads (the kit's pads, to pick one and set it
/// up) first, then the editors for the pad picked.
pub const TABS: [&str; 7] = ["Pads", "Wave", "Chop", "Play", "Grain", "FX", "Rec"];
/// The tabs built so far; the others are drawn faint.
pub const BUILT: [&str; 6] = ["Pads", "Wave", "Chop", "Play", "Grain", "Rec"];
/// The Wave tab's place in [`TABS`].
pub const TAB: usize = 1;

/// The hint line the app shows under the tab.
pub const HINT: &str = "Drag markers by their tabs · drag to scroll, pinch or turn the wheel to zoom · tap the wave to \
                        hear the pad · SHIFT + Q-Link: one frame at a time";

/// Shortest region and shortest loop, in frames.
pub const MIN_FRAMES: u64 = 32;
/// How far a marker looks for a zero crossing: this many frames, or eight
/// pixels of the current zoom if that is more.
const ZERO_RADIUS: u64 = 256;
/// How far a Q-Link turn looks ahead for the next zero crossing.
const ZERO_AHEAD: u64 = 8192;
/// Each wheel detent zooms by this much.
const WHEEL_ZOOM: f64 = 1.25;
/// Gesture ids the tab hands out start here, clear of other screens'.
const GESTURE_BASE: u64 = 0x5741_5645_0000_0000;

/// The markers, in the order the Q-Links move them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Start,
    End,
    LoopStart,
    LoopEnd,
}

impl Mark {
    pub const ALL: [Mark; 4] = [Mark::Start, Mark::End, Mark::LoopStart, Mark::LoopEnd];

    pub fn label(self) -> &'static str {
        match self {
            Mark::Start => "Start",
            Mark::End => "End",
            Mark::LoopStart => "Loop start",
            Mark::LoopEnd => "Loop end",
        }
    }

    fn tab(self) -> &'static str {
        match self {
            Mark::Start => "START",
            Mark::End => "END",
            Mark::LoopStart => "LOOP",
            Mark::LoopEnd => "LOOP END",
        }
    }

    fn is_loop(self) -> bool {
        matches!(self, Mark::LoopStart | Mark::LoopEnd)
    }

    /// Where it is in `r`, if it is there.
    pub fn frame(self, r: &Region) -> Option<u64> {
        match self {
            Mark::Start => Some(r.start),
            Mark::End => Some(r.end),
            Mark::LoopStart => r.sustain_loop.map(|l| l.start),
            Mark::LoopEnd => r.sustain_loop.map(|l| l.end),
        }
    }
}

/// Where markers land when they are moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Snap {
    Off,
    /// On a zero crossing, so the sound starts and stops without a click.
    #[default]
    ZeroCrossing,
    /// On sixteenth notes of the source's tempo (only when it has one).
    Sixteenths,
}

impl Snap {
    fn label(self) -> &'static str {
        match self {
            Snap::Off => "Off",
            Snap::ZeroCrossing => "Zero crossing",
            Snap::Sixteenths => "16ths",
        }
    }
}

/// Everything the tab shows: one layer of one pad, and its source.
#[derive(Clone)]
pub struct WaveState {
    pub pad: usize,
    pub layer: usize,
    pub pad_name: String,
    pub layers: usize,
    pub region: Region,
    pub source_name: String,
    pub rate: u32,
    pub channels: u8,
    /// The source's length.
    pub frames: u64,
    /// How many pads use the source.
    pub used_by: usize,
    pub source: Arc<SourceView>,
}

impl WaveState {
    /// Layer `layer` of pad `pad`, if there is one and `source` is its
    /// source.
    pub fn from_project(
        p: &Project,
        pad: usize,
        layer: usize,
        source: Arc<SourceView>,
    ) -> Option<WaveState> {
        let pd = p.pad(pad)?;
        let region = pd.layers.get(layer)?.region.clone();
        let s = p.source(region.source)?;
        (s.id() == source.id).then(|| WaveState {
            pad,
            layer,
            pad_name: pd.name.clone(),
            layers: pd.layers.len(),
            region,
            source_name: s.name.clone(),
            rate: s.rate(),
            channels: s.channels(),
            frames: s.frames(),
            used_by: p.users_of(s.id()).len(),
            source,
        })
    }

    fn frames_per_beat(&self) -> Option<f64> {
        self.source
            .tempo
            .filter(|b| b.is_finite() && *b > 0.0)
            .map(|bpm| f64::from(self.rate) * 60.0 / bpm)
    }

    /// Bar and beat lines, in 4/4 from the source's first frame.
    pub fn beats(&self) -> Option<Beats> {
        self.frames_per_beat().map(|fpb| Beats {
            first: 0.0,
            frames_per_beat: fpb,
            per_bar: 4,
        })
    }
}

/// What a touch or control on the tab asks for. The app applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum WaveAction {
    /// The layer's region, changed. `gesture` is the same for every change
    /// of one drag or turn, so [`Project::edit_pad`] merges them into one
    /// undo step.
    Region {
        pad: usize,
        layer: usize,
        region: Region,
        gesture: u64,
    },
    /// Play the pad, as a hit at full velocity would.
    Audition { pad: usize },
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
}

impl WaveAction {
    /// Applies a region change to the project (one undo step per gesture).
    /// Other actions change nothing here.
    pub fn apply(&self, p: &mut Project) -> Result<(), ModelError> {
        match self {
            WaveAction::Region {
                pad,
                layer,
                region,
                gesture,
            } => {
                let (layer, region) = (*layer, region.clone());
                if p.pad(*pad).is_none_or(|pd| layer >= pd.layers.len()) {
                    return Err(ModelError::BadPad);
                }
                p.edit_pad(*pad, Some(*gesture), move |pd| {
                    pd.layers[layer].region = region
                })
            }
            _ => Ok(()),
        }
    }
}

/// The Region list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Mode,
    Loop,
    Crossfade,
    Gain,
    Tune,
    Warp,
    WarpTempo,
    Reverse,
    Channels,
    FadeIn,
    FadeOut,
    Snap,
}

impl Row {
    pub const ALL: [Row; 12] = [
        Row::Mode,
        Row::Loop,
        Row::Crossfade,
        Row::Gain,
        Row::Tune,
        Row::Warp,
        Row::WarpTempo,
        Row::Reverse,
        Row::Channels,
        Row::FadeIn,
        Row::FadeOut,
        Row::Snap,
    ];

    fn name(self) -> &'static str {
        match self {
            Row::Mode => "Mode",
            Row::Loop => "Loop",
            Row::Crossfade => "Crossfade",
            Row::Gain => "Gain",
            Row::Tune => "Tune",
            Row::Warp => "Warp",
            Row::WarpTempo => "Warp tempo",
            Row::Reverse => "Reverse",
            Row::Channels => "Channels",
            Row::FadeIn => "Fade in",
            Row::FadeOut => "Fade out",
            Row::Snap => "Snap",
        }
    }
}

const MODES: [PlayMode; 5] = [
    PlayMode::OneShot,
    PlayMode::Gate,
    PlayMode::Toggle,
    PlayMode::Loop,
    PlayMode::Granular,
];
const CHANNELS: [ChannelPick; 4] = [
    ChannelPick::Stereo,
    ChannelPick::MonoSum,
    ChannelPick::Left,
    ChannelPick::Right,
];
/// Loop crossfade lengths the arrows step through, in ms.
const CROSSFADE_MS: [f64; 8] = [0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];
/// Fade lengths the arrows step through, in ms.
const FADE_MS: [f64; 13] = [
    0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0,
];
const GAIN_STEP_DB: f32 = 0.5;
const GAIN_RANGE_DB: (f32, f32) = (-48.0, 24.0);
const TUNE_LIMIT_CENTS: i32 = 4800;

pub fn mode_label(m: PlayMode) -> &'static str {
    match m {
        PlayMode::OneShot => "One shot",
        PlayMode::Gate => "Gate",
        PlayMode::Toggle => "Toggle",
        PlayMode::Loop => "Loop",
        PlayMode::Granular => "Granular",
    }
}

/// A pad's name on the MPC: bank letter and number, such as "A5".
pub fn pad_label(pad: usize) -> String {
    let bank = (b'A' + (pad / 16 % 26) as u8) as char;
    format!("{bank}{}", pad % 16 + 1)
}

/// A length in frames as ms or seconds.
pub fn duration_text(frames: u64, rate: u32) -> String {
    let ms = frames as f64 * 1000.0 / f64::from(rate.max(1));
    if ms < 10.0 && frames > 0 {
        format!("{ms:.1} ms")
    } else if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

/// A marker's position: with a tempo, bar.beat.sixteenth (from 1.1.1) and
/// the ticks past that sixteenth (240 to a sixteenth, 960 to a beat);
/// without one, seconds and frames.
pub fn position_text(frame: u64, rate: u32, tempo: Option<f64>) -> (String, String) {
    match tempo.filter(|b| b.is_finite() && *b > 0.0) {
        Some(bpm) => {
            let fpb = f64::from(rate.max(1)) * 60.0 / bpm;
            // In whole ticks, so a marker on a sixteenth reads +000 however
            // the frames round.
            let all = (frame as f64 / fpb * 960.0).round() as u64;
            let (whole, ticks) = (all / 240, all % 240);
            let (bar, beat, six) = (whole / 16 + 1, whole / 4 % 4 + 1, whole % 4 + 1);
            (format!("{bar}.{beat}.{six}"), format!("+{ticks:03}"))
        }
        None => {
            let s = frame as f64 / f64::from(rate.max(1));
            (format!("{s:.3} s"), format!("{frame}"))
        }
    }
}

/// Steps through `list` (ms) from the entry nearest `now` (frames).
fn step_ms(list: &[f64], now: u64, rate: u32, delta: i32) -> u64 {
    let ms = now as f64 * 1000.0 / f64::from(rate.max(1));
    let i = list
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - ms).abs().total_cmp(&(b.1 - ms).abs()))
        .map_or(0, |(i, _)| i) as i32;
    let j = (i + delta).clamp(0, list.len() as i32 - 1) as usize;
    (list[j] * f64::from(rate) / 1000.0).round() as u64
}

/// `x` within `lo..=hi`; `lo` wins when they cross.
fn within(x: u64, lo: u64, hi: u64) -> u64 {
    x.min(hi).max(lo)
}

/// Shortens the fades to fit the region, fade out first.
fn fit_fades(r: &mut Region) {
    let len = r.frames();
    let fi = u64::from(r.fade_in.frames).min(len);
    let fo = u64::from(r.fade_out.frames).min(len - fi);
    r.fade_in.frames = fi as u32;
    r.fade_out.frames = fo as u32;
}

/// `r` with marker `m` moved as near `to` as the other markers allow. Warp
/// pins stay on their hits (from a source at `rate`).
pub fn moved(r: &Region, m: Mark, to: u64, frames: u64, rate: u32) -> Region {
    let old = (r.start, r.end);
    let mut r = r.clone();
    match (m, r.sustain_loop) {
        (Mark::Start, Some(l)) => r.start = within(to, 0, l.start - u64::from(l.crossfade)),
        (Mark::Start, None) => r.start = within(to, 0, r.end.saturating_sub(MIN_FRAMES)),
        (Mark::End, Some(l)) => r.end = within(to, l.end.max(r.start + MIN_FRAMES), frames),
        (Mark::End, None) => r.end = within(to, r.start + MIN_FRAMES, frames),
        (Mark::LoopStart, Some(mut l)) => {
            let span = MIN_FRAMES.max(2 * u64::from(l.crossfade));
            l.start = within(
                to,
                r.start + u64::from(l.crossfade),
                l.end.saturating_sub(span),
            );
            r.sustain_loop = Some(l);
        }
        (Mark::LoopEnd, Some(mut l)) => {
            let span = MIN_FRAMES.max(2 * u64::from(l.crossfade));
            l.end = within(to, l.start + span, r.end);
            r.sustain_loop = Some(l);
        }
        (_, None) => {}
    }
    fit_fades(&mut r);
    if let Some(w) = &r.warp {
        r.warp = Some(w.moved(old, (r.start, r.end), rate));
    }
    r
}

/// The next zero crossing at or after `at`, within `ahead` frames.
pub(crate) fn next_zero_crossing(x: &[f32], at: usize, ahead: usize) -> Option<usize> {
    let end = x.len().min(at.saturating_add(ahead).saturating_add(1));
    (at.max(1)..end)
        .find(|&i| x[i] == 0.0 || (x[i - 1] < 0.0) != (x[i] < 0.0))
        .map(|i| {
            if x[i - 1].abs() < x[i].abs() {
                i - 1
            } else {
                i
            }
        })
}

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Marker(Mark),
    Row(Row),
}

/// The Wave tab's widgets and what their touches mean. It keeps the snap
/// setting, the marker the wheel zooms round and the undo step being made;
/// everything else comes from the [`WaveState`].
pub struct WaveTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, when the app's pads 1-16 stand
    /// for one bank of its own (the pad player's kits are banks A-H).
    /// None names pads by their place in the project, as "A5" for pad 4.
    pub bank: Option<char>,
    snap: Snap,
    focus: Mark,
    gesture: Option<(Key, u64)>,
    next_gesture: u64,
    /// The source the waveform shows, so a new one starts zoomed out.
    shown: Option<SourceId>,
    /// The waveform's width in pixels (its views are measured in it).
    wave_w: i32,
}

impl WaveTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &WaveState) -> Result<WaveTab, String> {
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        ui.add(OVERVIEW, WaveOverview::new());
        ui.add(WAVE, Waveform::new());
        ui.add(REGION, SettingList::new("Region"));
        let t = ui.theme.clone();
        let colors = [
            t.accent,
            t.accent.mix(t.bad, 0.55),
            t.good,
            t.good.mix(t.warn, 0.5),
        ];
        for (k, m) in Mark::ALL.iter().enumerate() {
            ui.add(
                QLINKS[k],
                KnobCard::new(&format!("Q-Link {} · {}", k + 1, m.label()), colors[k]),
            );
        }
        ui.add(WHEEL, InfoCard::new("Wheel · zoom"));
        ui.add_page(name, LAYOUT)?;
        let wave = slot(ui, WAVE).ok_or("no waveform slot")?;
        let mut tab = WaveTab {
            name: name.to_string(),
            bank: None,
            snap: Snap::default(),
            focus: Mark::Start,
            gesture: None,
            next_gesture: GESTURE_BASE,
            shown: None,
            wave_w: Waveform::wave_rect(wave).w,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    pub fn snap(&self) -> Snap {
        self.snap
    }

    /// The marker the wheel zooms round.
    pub fn focus(&self) -> Mark {
        self.focus
    }

    /// The waveform's view.
    pub fn view(&self, ui: &Ui) -> Option<WaveView> {
        ui.get::<Waveform>(WAVE).map(Waveform::view)
    }

    /// Ends the drag or turn being made: the next edit is a new undo step.
    /// Call it when a finger lifts and when a Q-Link is let go.
    pub fn lift(&mut self) {
        self.gesture = None;
    }

    /// Where the pad's voices are playing, in source frames. Only their
    /// strips are redrawn.
    pub fn set_playheads(&self, ui: &mut Ui, frames: &[u64]) {
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            w.set_playheads(frames);
        }
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &WaveState) {
        let r = &st.region;
        let src = &st.source;
        if st.region.sustain_loop.is_none() && self.focus.is_loop() {
            self.focus = Mark::Start;
        }
        if self.snap == Snap::Sixteenths && src.tempo.is_none() {
            self.snap = Snap::ZeroCrossing;
        }
        let pad_color = pad_color(st.pad);
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color;
            let layer = if st.layers > 1 {
                format!(" · layer {} of {}", st.layer + 1, st.layers)
            } else {
                String::new()
            };
            let pad = match self.bank {
                Some(b) => format!("{b}{}", st.pad % 16 + 1),
                None => pad_label(st.pad),
            };
            bar.title = format!("Pad {pad} · {}{layer}", st.pad_name);
            let mut about = vec![if st.source_name == st.pad_name {
                format!("Source {}", short_id(st.source.id))
            } else {
                format!("Source {} ({})", st.source_name, short_id(st.source.id))
            }];
            if let (Some(bpm), Some(fpb)) = (src.tempo, st.frames_per_beat()) {
                let bars = st.frames as f64 / fpb / 4.0;
                if (bars - bars.round()).abs() < 0.02 && bars.round() >= 1.0 {
                    about.push(format!(
                        "{} bar{}",
                        bars.round(),
                        if bars.round() == 1.0 { "" } else { "s" }
                    ));
                }
                about.push(format!("{bpm:.1} BPM"));
            }
            if let Some(root) = &src.root {
                about.push(format!("root {root}"));
            }
            about.push(if st.channels == 2 {
                "stereo".into()
            } else {
                "mono".into()
            });
            about.push(duration_text(st.frames, st.rate));
            if st.used_by > 1 {
                about.push(format!("used by {} pads", st.used_by));
            }
            bar.subtitle = about.join(" · ");
        }
        let width = self.wave_w;
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            if self.shown != Some(src.id) {
                w.set_peaks(Some(src.peaks.clone()), width);
                self.shown = Some(src.id);
            }
            w.set_markers(markers(r));
            w.set_region(Some((r.start, r.end)));
            w.set_loop(r.sustain_loop.map(|l| (l.start, l.end)));
            w.set_fades(u64::from(r.fade_in.frames), u64::from(r.fade_out.frames));
            w.set_beats(st.beats());
        }
        if let Some(o) = ui.get_mut::<WaveOverview>(OVERVIEW) {
            o.set_peaks(Some(src.peaks.clone()));
            o.set_region(Some((r.start, r.end)));
        }
        self.sync_overview(ui);
        let rows = self.rows(st);
        if let Some(list) = ui.get_mut::<SettingList>(REGION) {
            list.right = mode_label(r.mode).to_string();
            list.rows = rows;
        }
        for (k, m) in Mark::ALL.iter().enumerate() {
            if let Some(card) = ui.get_mut::<KnobCard>(QLINKS[k]) {
                match m.frame(r) {
                    Some(f) => {
                        let (value, sub) = position_text(f, st.rate, src.tempo);
                        card.value = value;
                        card.sub = sub;
                        card.pos = (f as f64 / st.frames.max(1) as f64) as f32;
                        card.enabled = true;
                    }
                    None => card.enabled = false,
                }
                card.focused = *m == self.focus;
            }
        }
        self.update_wheel(ui, st);
    }

    fn rows(&self, st: &WaveState) -> Vec<Setting> {
        let r = &st.region;
        let row = |k: Row, value: String| Setting {
            name: k.name().to_string(),
            value,
        };
        Row::ALL
            .iter()
            .map(|k| {
                let value = match k {
                    Row::Mode => mode_label(r.mode).to_string(),
                    Row::Loop => match r.sustain_loop {
                        None => "Off".into(),
                        Some(l) if l.mode == LoopMode::Forward => "Forward".into(),
                        Some(_) => "Ping-pong".into(),
                    },
                    Row::Crossfade => match r.sustain_loop {
                        Some(l) if l.mode == LoopMode::Forward => {
                            duration_text(u64::from(l.crossfade), st.rate)
                        }
                        _ => "—".into(),
                    },
                    Row::Gain => format!("{:+.1} dB", r.gain_db).replace("+0.0", "0.0"),
                    Row::Tune => tune_text(r.tune_cents),
                    Row::Warp => warp_label(r.warp.as_ref()).into(),
                    Row::WarpTempo => match &r.warp {
                        Some(w) => format!("{:.1} BPM", w.bpm),
                        None => "—".into(),
                    },
                    Row::Reverse => {
                        if r.reverse {
                            "On".into()
                        } else {
                            "Off".into()
                        }
                    }
                    Row::Channels if st.channels == 1 => "Mono".into(),
                    Row::Channels => match r.channels {
                        ChannelPick::Stereo => "Stereo".into(),
                        ChannelPick::MonoSum => "Mono sum".into(),
                        ChannelPick::Left => "Left".into(),
                        ChannelPick::Right => "Right".into(),
                    },
                    Row::FadeIn => duration_text(u64::from(r.fade_in.frames), st.rate),
                    Row::FadeOut => duration_text(u64::from(r.fade_out.frames), st.rate),
                    Row::Snap => self.snap.label().into(),
                };
                row(*k, value)
            })
            .collect()
    }

    fn update_wheel(&self, ui: &mut Ui, st: &WaveState) {
        let Some(view) = self.view(ui) else { return };
        let whole = st.frames.max(1) as f64 / f64::from(self.wave_w.max(1));
        let zoom = whole / view.frames_per_px.max(1e-9);
        if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
            card.text = format!("{zoom:.1}× · {}", self.focus.label().to_lowercase());
        }
    }

    fn sync_overview(&self, ui: &mut Ui) {
        let Some(view) = self.view(ui) else { return };
        let width = self.wave_w;
        if let Some(o) = ui.get_mut::<WaveOverview>(OVERVIEW) {
            o.set_shown(view, width);
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// on other widgets, or that only change what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &WaveState) -> Option<WaveAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(WaveAction::Tab(*i)),
            (WAVE, EventKind::Marker { index, frame }) => {
                let m = *markers_order(&st.region).get(*index)?;
                self.focus = m;
                let to = self.snapped(st, *frame, 0, ui);
                let r = moved(&st.region, m, to, st.frames, st.rate);
                // The widget moved its tab with the finger; put it where
                // the marker really is.
                if let (Some(w), Some(f)) = (ui.get_mut::<Waveform>(WAVE), m.frame(&r)) {
                    w.set_marker_frame(*index, f);
                }
                self.focus_cards(ui);
                self.update_wheel(ui, st);
                self.edit(st, Key::Marker(m), r)
            }
            (WAVE, EventKind::View { .. }) => {
                self.sync_overview(ui);
                self.update_wheel(ui, st);
                None
            }
            (WAVE, EventKind::Frame(_)) => Some(WaveAction::Audition { pad: st.pad }),
            (OVERVIEW, EventKind::Frame(f)) => {
                let width = self.wave_w;
                if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
                    let v = w.view();
                    let start = *f as f64 - f64::from(width) / 2.0 * v.frames_per_px;
                    w.set_view(WaveView { start, ..v }, width);
                }
                self.sync_overview(ui);
                None
            }
            (REGION, EventKind::Step { index, delta }) => {
                self.step(ui, st, *Row::ALL.get(*index)?, *delta)
            }
            (name, EventKind::Select(_)) if QLINKS.contains(&name) => {
                let k = QLINKS.iter().position(|q| *q == name)?;
                self.focus = Mark::ALL[k];
                self.focus_cards(ui);
                self.update_wheel(ui, st);
                None
            }
            _ => None,
        }
    }

    /// A Q-Link turned: `knob` 0 to 3 moves Start, End, Loop start or Loop
    /// end by `delta` detents; `fine` (SHIFT held) moves one frame per
    /// detent, unsnapped.
    pub fn qlink(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        knob: usize,
        delta: i32,
        fine: bool,
    ) -> Option<WaveAction> {
        let m = *Mark::ALL.get(knob)?;
        let now = m.frame(&st.region)?;
        self.focus = m;
        self.focus_cards(ui);
        if delta == 0 {
            return None;
        }
        let to = if fine {
            now.saturating_add_signed(i64::from(delta))
        } else {
            let px = self.view(ui).map_or(1.0, |v| v.frames_per_px).max(1.0);
            let raw = (now as f64 + f64::from(delta) * px).round().max(0.0) as u64;
            self.snapped(st, raw, delta.signum(), ui)
        };
        let r = moved(&st.region, m, to, st.frames, st.rate);
        self.update_wheel(ui, st);
        self.edit(st, Key::Marker(m), r)
    }

    /// The data wheel: zooms in (turned up) or out round the focused marker,
    /// or round the middle if it is out of view.
    pub fn wheel(&mut self, ui: &mut Ui, st: &WaveState, delta: i32) {
        let width = self.wave_w;
        let at = self.focus.frame(&st.region);
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            let v = w.view();
            let mid = f64::from(width) / 2.0;
            let (frame, x) = match at.map(|f| (f as f64, v.x_of(f as f64))) {
                Some((f, x)) if (0.0..=f64::from(width)).contains(&x) => (f, x),
                _ => (v.frame_at(mid), mid),
            };
            let nv = v.zoomed(WHEEL_ZOOM.powi(delta), frame, x, st.frames, width);
            w.set_view(nv, width);
        }
        self.sync_overview(ui);
        self.update_wheel(ui, st);
    }

    fn focus_cards(&self, ui: &mut Ui) {
        for (k, m) in Mark::ALL.iter().enumerate() {
            if let Some(card) = ui.get_mut::<KnobCard>(QLINKS[k]) {
                card.focused = *m == self.focus;
            }
        }
    }

    /// `to` snapped as the tab's snap says: to the nearest place (`dir` 0)
    /// or the next one in the direction of `dir`.
    fn snapped(&self, st: &WaveState, to: u64, dir: i32, ui: &Ui) -> u64 {
        let to = to.min(st.frames);
        match self.snap {
            Snap::Off => to,
            Snap::Sixteenths => {
                let Some(fpb) = st.frames_per_beat() else {
                    return to;
                };
                let grid = fpb / 4.0;
                let k = to as f64 / grid;
                let k = match dir {
                    0 => k.round(),
                    d if d > 0 => k.ceil(),
                    _ => k.floor(),
                };
                ((k * grid).round() as u64).min(st.frames)
            }
            Snap::ZeroCrossing => {
                let x = &st.source.mono;
                if x.len() < 2 || to >= x.len() as u64 {
                    return to;
                }
                let at = to as usize;
                let found = match dir {
                    0 => {
                        let px = self.view(ui).map_or(1.0, |v| v.frames_per_px);
                        let radius = ZERO_RADIUS.max((px * 8.0) as u64) as usize;
                        nearest_zero_crossing(x, at, radius)
                    }
                    d if d > 0 => next_zero_crossing(x, at, ZERO_AHEAD as usize),
                    _ => previous_zero_crossing(x, at, ZERO_AHEAD as usize),
                };
                found.map_or(to, |f| f as u64)
            }
        }
    }

    /// The action for a changed region (None if it didn't change), in the
    /// undo step of the edit being made, or a new one.
    fn edit(&mut self, st: &WaveState, key: Key, r: Region) -> Option<WaveAction> {
        if r == st.region {
            return None;
        }
        let gesture = match self.gesture {
            Some((k, g)) if k == key => g,
            _ => {
                let g = self.next_gesture;
                self.next_gesture += 1;
                self.gesture = Some((key, g));
                g
            }
        };
        Some(WaveAction::Region {
            pad: st.pad,
            layer: st.layer,
            region: r,
            gesture,
        })
    }

    /// An arrow on a Region row.
    fn step(&mut self, ui: &mut Ui, st: &WaveState, row: Row, delta: i32) -> Option<WaveAction> {
        let mut r = st.region.clone();
        let len = r.frames();
        let d = delta.signum();
        match row {
            Row::Mode => {
                let i = MODES.iter().position(|m| *m == r.mode).unwrap_or(0) as i32;
                r.mode = MODES[(i + d).rem_euclid(MODES.len() as i32) as usize];
                if r.mode == PlayMode::Loop && r.sustain_loop.is_none() {
                    r.sustain_loop = Some(default_loop(&r));
                }
            }
            Row::Loop => {
                // Off, Forward, Ping-pong; Loop mode can't be without one.
                let states = [None, Some(LoopMode::Forward), Some(LoopMode::PingPong)];
                let now = states
                    .iter()
                    .position(|s| *s == r.sustain_loop.map(|l| l.mode))
                    .unwrap_or(0) as i32;
                let mut next = (now + d).rem_euclid(3) as usize;
                if states[next].is_none() && r.mode == PlayMode::Loop {
                    next = (next as i32 + d).rem_euclid(3) as usize;
                }
                r.sustain_loop = states[next].map(|mode| match r.sustain_loop {
                    Some(l) => Loop {
                        mode,
                        crossfade: if mode == LoopMode::Forward {
                            l.crossfade
                        } else {
                            0
                        },
                        ..l
                    },
                    None => Loop {
                        mode,
                        ..default_loop(&r)
                    },
                });
            }
            Row::Crossfade => {
                let mut l = r.sustain_loop.filter(|l| l.mode == LoopMode::Forward)?;
                let max = ((l.end - l.start) / 2).min(l.start - r.start);
                l.crossfade =
                    step_ms(&CROSSFADE_MS, u64::from(l.crossfade), st.rate, d).min(max) as u32;
                r.sustain_loop = Some(l);
            }
            Row::Gain => {
                let g = ((r.gain_db / GAIN_STEP_DB).round() + d as f32) * GAIN_STEP_DB;
                r.gain_db = g.clamp(GAIN_RANGE_DB.0, GAIN_RANGE_DB.1);
            }
            Row::Tune => {
                r.tune_cents = (r.tune_cents + 100 * d).clamp(-TUNE_LIMIT_CENTS, TUNE_LIMIT_CENTS)
            }
            Row::Warp => {
                // Off, Stretch, Repitch.
                let states = [None, Some(WarpMethod::Stretch), Some(WarpMethod::Repitch)];
                let now = states
                    .iter()
                    .position(|m| *m == r.warp.as_ref().map(|w| w.method))
                    .unwrap_or(0) as i32;
                let next = states[(now + d).rem_euclid(3) as usize];
                let was = r
                    .warp
                    .clone()
                    .unwrap_or_else(|| Warp::new(region_tempo(st, &r)));
                r.warp = next.map(|method| Warp { method, ..was });
            }
            Row::WarpTempo => {
                let mut w = r.warp.clone()?;
                // A BPM at a time, keeping the tenths a detected tempo has;
                // pins the new tempo leaves no room for go.
                w.bpm = (w.bpm + d as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end());
                w.fit(r.start, r.end, st.rate);
                r.warp = Some(w);
            }
            Row::Reverse => r.reverse = !r.reverse,
            Row::Channels => {
                if st.channels == 1 {
                    return None;
                }
                let i = CHANNELS.iter().position(|c| *c == r.channels).unwrap_or(0) as i32;
                r.channels = CHANNELS[(i + d).rem_euclid(CHANNELS.len() as i32) as usize];
            }
            Row::FadeIn => {
                let max = len - u64::from(r.fade_out.frames);
                r.fade_in = Fade {
                    frames: step_ms(&FADE_MS, u64::from(r.fade_in.frames), st.rate, d).min(max)
                        as u32,
                    ..r.fade_in
                };
            }
            Row::FadeOut => {
                let max = len - u64::from(r.fade_in.frames);
                r.fade_out = Fade {
                    frames: step_ms(&FADE_MS, u64::from(r.fade_out.frames), st.rate, d).min(max)
                        as u32,
                    ..r.fade_out
                };
            }
            Row::Snap => {
                let all: &[Snap] = if st.source.tempo.is_some() {
                    &[Snap::Off, Snap::ZeroCrossing, Snap::Sixteenths]
                } else {
                    &[Snap::Off, Snap::ZeroCrossing]
                };
                let i = all.iter().position(|s| *s == self.snap).unwrap_or(0) as i32;
                self.snap = all[(i + d).rem_euclid(all.len() as i32) as usize];
                let rows = self.rows(st);
                if let Some(list) = ui.get_mut::<SettingList>(REGION) {
                    list.rows = rows;
                }
                return None;
            }
        }
        self.edit(st, Key::Row(row), r)
    }
}

fn warp_label(w: Option<&Warp>) -> &'static str {
    match w.map(|w| w.method) {
        None => "Off",
        Some(WarpMethod::Stretch) => "Stretch",
        Some(WarpMethod::Repitch) => "Repitch",
    }
}

/// The tempo a region turned to warp starts at: the source's detected
/// tempo, made exact when the region is whole beats of it (within 2 %);
/// with no tempo detected, whole beats near 120 BPM.
pub fn region_tempo(st: &WaveState, r: &Region) -> f32 {
    let hint = st.source.tempo.map(|t| t as f32);
    let exact = mpfree_analysis::loop_tempo(r.frames(), st.rate, hint);
    let bpm = match (hint, exact) {
        (Some(h), Some(e)) if (e / h - 1.0).abs() < 0.02 => e,
        (Some(h), _) => h,
        (None, e) => e.unwrap_or(120.0),
    };
    (bpm * 10.0).round() / 10.0
}

/// A loop over the middle half of `r`, for a region turned to Loop mode.
pub fn default_loop(r: &Region) -> Loop {
    let q = r.frames() / 4;
    Loop {
        start: r.start + q,
        end: r.end - q,
        crossfade: 0,
        mode: LoopMode::Forward,
    }
}

fn tune_text(cents: i32) -> String {
    let (st, ct) = (cents / 100, cents % 100);
    match (st, ct) {
        (0, 0) => "0 st".into(),
        (s, 0) => format!("{s:+} st"),
        (0, c) => format!("{c:+} ct"),
        (s, c) => format!("{s:+} st {c:+} ct"),
    }
}

/// The source id as the bar shows it: first and last two bytes.
fn short_id(id: SourceId) -> String {
    let s = id.to_string();
    format!("{}…{}", &s[..4], &s[s.len() - 2..])
}

/// The markers in the waveform's order: Start and End, then the loop's.
fn markers_order(r: &Region) -> Vec<Mark> {
    Mark::ALL
        .iter()
        .copied()
        .filter(|m| m.frame(r).is_some())
        .collect()
}

fn markers(r: &Region) -> Vec<Marker> {
    markers_order(r)
        .into_iter()
        .filter_map(|m| {
            let kind = if m.is_loop() {
                MarkerKind::Loop
            } else {
                MarkerKind::Edge
            };
            m.frame(r).map(|f| Marker::new(f, m.tab(), kind))
        })
        .collect()
}

/// A pad's colour on the screen, by its place in the bank (the pads' own
/// colours are set elsewhere; this only tells pads apart).
pub fn pad_color(pad: usize) -> Color {
    const COLORS: [u32; 8] = [
        0xFF8A3D, 0x6EA8FE, 0x3DDC97, 0xFFC145, 0xC77DFF, 0xFF5C8A, 0x4DD4E8, 0xA3E048,
    ];
    Color::rgb(COLORS[pad % COLORS.len()])
}

/// Where `name` sits on a page laid out by [`LAYOUT`], whichever page shows.
fn slot(ui: &Ui, name: &str) -> Option<Rect> {
    let node = layout::parse(LAYOUT).ok()?;
    let (w, h) = ui.size();
    let g = ui.theme.gap;
    layout::solve(&node, Rect::new(0, 0, w, h).inset(g), g)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| r)
}
