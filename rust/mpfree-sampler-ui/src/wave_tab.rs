//! The pad editor's Wave tab: where a pad's sound starts and ends, where it
//! loops, how it fades, and how loud, tuned and turned round it plays.
//!
//! - The **overview** shows the whole source with a box round what the
//!   waveform shows; a touch moves the box there. Beside it, the wheel's
//!   zoom and the marker it zooms round.
//! - The **waveform** shows the region bright and the rest dimmed, the loop
//!   shaded, the fades as curves, bar and beat lines when the source has a
//!   tempo, and the Start, End and loop markers as tabs to drag. Dragging
//!   the wave scrolls it, pinching zooms, and a tap plays the pad.
//! - The **panel** under the wave holds the settings as an instrument's
//!   controls (Jay, 17 Nov 2026; [`crate::panel`]), in three sections:
//!   **Mode** (how a hit plays, the loop and its crossfade), **Gain and
//!   fades** (the sound's gain, its fades, and **Normalize**, which sets
//!   the gain so the region's loudest frame plays at 0 dBFS:
//!   [`WaveAction::Normalize`], the app reads the audio) and **Options**
//!   (reverse, how markers snap, channels). [`Row::ALL`] is their order.
//! - **Sound** beside them (the one Sample page, Jay 18 Nov 2026;
//!   [`crate::sound_panel`]): how the pad plays its sound, four controls at
//!   a time (Amp, Filter, Pitch, Movement), each change a
//!   [`WaveAction::Play`]. Warping is the Warp tool's
//!   ([`crate::warp_tool`]), and the pad's tune is Sound's (Pitch), so the
//!   page has one Tune.
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

use crate::recorder::{self, Recorder, RecorderAction, RecorderView, INPUT_ROWS, RECORDER};
use std::sync::Arc;

use mpfree_analysis::{nearest_zero_crossing, next_zero_crossing, previous_zero_crossing, Samples};
use mpfree_sampler::model::{
    ChannelPick, ChopMode, Fade, Loop, LoopMode, ModelError, PlayMode, Project, Region, SourceId,
    WARP_BPM,
};
use mpfree_ui::layout;
use mpfree_ui::numpad::NumberAsk;
use mpfree_ui::wave::{
    Beats, GrainSpan, LaneCell, Marker, MarkerKind, Peaks, WaveOverview, WaveView, Waveform,
};
use mpfree_ui::{Color, ColorKey, EventKind, Paint, Rect, Role, Ui, UiEvent};

use crate::grain_tab::{self, CloudNow};
use crate::grain_tool::{self, GrainPart};
use crate::panel::{Control, Item, Look, Panel, Section};
use crate::play_tab::{PlayAction, PlayState};
use crate::slice_tool::{self, ChopAction, ChopState, SliceTool};
use crate::sound_panel::{self, SoundPanel};
use crate::source::SourceView;
use crate::warp_tool::{self, Tapped, WarpTool};
use crate::widgets::{KnobCard, PadBar, Setting, SettingList, ToolStrip};
use mpfree_sampler::engine::GrainView;

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-bar";
pub const OVERVIEW: &str = "smp-overview";
pub const WAVE: &str = "smp-wave";
pub const REGION: &str = "smp-region";
pub const QLINKS: [&str; 4] = ["smp-q1", "smp-q2", "smp-q3", "smp-q4"];
/// The tools strip over the wave ([`Tool`]).
pub const TOOLS: &str = "smp-tools";

/// The tab's page: the app's header, the pad bar, the recorder row
/// ([`crate::recorder`]) over the tools strip, the overview (with the zoom
/// in its corner) and the Keep card ([`crate::recorder::KeepCard`], over
/// the wave's right end since Jay's device notes of 20 Nov 2026, in the
/// zoom card's place), the waveform
/// across the page, the panel of settings under it with the Sound section
/// on its right, the Q-Link cards on the right, and the app's hint line.
pub const LAYOUT: &str = "col(header:60, row(smp-bar:*, smp-send:330):60, \
                          row(col(smp-recorder:64, row(smp-tools:360, smp-overview:*, smp-keep:340):56, smp-wave:*, \
                          row(smp-region:*, smp-sound:340):242):*, \
                          col(smp-q1, smp-q2, smp-q3, smp-q4):128):*, hint:26)";

// The pad editor had tabs over its pages, the editors for the pad picked;
// the one Sample page has none, so its pad bar has no switch. The kit's
// pads (to pick one and set it up) went to the app's own Pads tab on 12
// Nov 2026 ([`crate::pads_tab`]). FX left on 18 Nov 2026 (Jay: a pad's
// effects are in Mix, which the Pads tab's Effects row opens). Grain left
// on 18 Nov 2026 too: it is a tool on this page ([`Tool::Grain`]), and so
// did Warp ([`Tool::Warp`]). So did Rec (the one Sample page's step 2f,
// part 4): its controls are the recorder row's, its Input section's and
// Sample Recall's. So did Chop (step 3): it is the Slice tool
// ([`Tool::Slice`]). And so did Play (step 6b): it is the Pads tab's Edit
// pad ([`crate::play_tab`]), opened from the pad's settings.

/// The hint line the app shows under the tab.
pub const HINT: &str = "Drag markers by their tabs · drag to scroll, pinch or SHIFT + - / + to zoom · tap the wave to \
                        hear the pad · SHIFT + Q-Link: one frame at a time";

/// What a touch on the wave says while a take draws into it.
pub const LIVE_WHY: &str =
    "The wave is the take as it records: REC again or STOP keeps it, then trim it here";

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

/// The one Sample page's tools over the wave (Jay, 18 Nov 2026: one wave,
/// four tools). The tool decides what a finger on the wave moves, which
/// marks are bright, the panel's first section and the Q-Links; while REC
/// is on, the recorder's Input section and its Q-Links win.
///
/// Trim is the page as the Wave page was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Start, End, the loop and the fades.
    #[default]
    Trim,
    /// The cuts and where they land ([`crate::slice_tool`]).
    Slice,
    /// Hits and pins ([`crate::warp_tool`]).
    Warp,
    /// The grain window ([`crate::grain_tool`]).
    Grain,
}

impl Tool {
    /// The strip's order.
    pub const ALL: [Tool; 4] = [Tool::Trim, Tool::Slice, Tool::Warp, Tool::Grain];

    /// Its place on the strip ([`TOOLS`]): what a tap on it carries.
    pub fn index(self) -> usize {
        Tool::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Trim => "Trim",
            Tool::Slice => "Slice",
            Tool::Warp => "Warp",
            Tool::Grain => "Grain",
        }
    }

    /// The hint line the app shows under the page with this tool.
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Grain => GRAIN_HINT,
            Tool::Warp => WARP_HINT,
            Tool::Slice => slice_tool::HINT,
            Tool::Trim => HINT,
        }
    }
}

/// The hint line with the Grain tool.
pub const GRAIN_HINT: &str = "Drag GRAIN to move where grains start, SIZE for their length · tap the wave: \
                              grains start there · hit the pad to hear the cloud · SHIFT + Q-Link 1 / 4: the sprays";

/// The hint line with the Warp tool.
pub const WARP_HINT: &str =
    "Tap a hit to pin it to the grid · drag a pin's tab along the beats · Pin hits pins \
                             them all · Q-Link 1 picks a pin, 2 nudges it · SHIFT + - / + zoom";

/// The Slice tool's rows each Q-Link card lights when picked: the
/// method's setting, the slice, (its start has no row), the pads.
const SLICE_CARD_ROWS: [Option<slice_tool::Row>; 4] = [
    Some(slice_tool::Row::Amount),
    Some(slice_tool::Row::Slice),
    None,
    Some(slice_tool::Row::Pads),
];

/// Why the tools are faint while a take records (the mockup's "tools wait
/// for the take").
pub const TOOLS_WAIT: &str = "The tools wait for the take: stop it first";

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

/// The pad number the page's state carries for a take kept on the page
/// (the one Sample page's step 2b): no pad at all, so an action naming it
/// reaches no pad; the app sends a take's edits to the take.
pub const TAKE: usize = usize::MAX;

/// A take kept on the page, as the bar names it: take `index` of `of`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeShown {
    pub index: usize,
    pub of: usize,
}

/// Everything the tab shows: one layer of one pad (or of a take kept on
/// the page), and its source.
#[derive(Clone)]
pub struct WaveState {
    /// The pad, or [`TAKE`] for a take.
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
    /// How many pads (and takes) use the source.
    pub used_by: usize,
    pub source: Arc<SourceView>,
    /// A take kept on the page, not a pad.
    pub take: Option<TakeShown>,
    /// The project's tempo, for a grain size synced to it (120 until the
    /// app says).
    pub bpm: f64,
    /// What the Slice tool chops ([`ChopState`]): the same layer, with
    /// the kit's pads that have a sound and the chop kept for the sound.
    /// The app says where Put on may go (files, a new kit).
    pub chop: Option<Arc<ChopState>>,
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
        let mut st = Self::of_sound(p, pad, p.pad(pad)?, layer, source.clone())?;
        st.chop = ChopState::from_project(p, pad, layer, source).map(Arc::new);
        Some(st)
    }

    /// Layer `layer` of take `index` kept on the page (one-page step 2b),
    /// as [`WaveState::from_project`] shows a pad's; its pad is [`TAKE`].
    pub fn of_take(
        p: &Project,
        index: usize,
        layer: usize,
        source: Arc<SourceView>,
    ) -> Option<WaveState> {
        let mut st = Self::of_sound(p, TAKE, p.take(index)?, layer, source.clone())?;
        st.chop = ChopState::of_take(p, index, layer, source).map(Arc::new);
        st.take = Some(TakeShown {
            index,
            of: p.takes().len(),
        });
        Some(st)
    }

    fn of_sound(
        p: &Project,
        pad: usize,
        pd: &mpfree_sampler::model::Pad,
        layer: usize,
        source: Arc<SourceView>,
    ) -> Option<WaveState> {
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
            used_by: p.users_of(s.id()).len() + p.takes_using(s.id()),
            source,
            take: None,
            bpm: 120.0,
            chop: None,
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
            per_beat: 1,
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
        region: Box<Region>,
        gesture: u64,
    },
    /// Play the pad, as a hit at full velocity would.
    Audition { pad: usize },
    /// Ask for a number to be typed (the Warp tempo's value tapped); the
    /// number comes back through the tab's `typed`.
    Ask(NumberAsk),
    /// Set the layer's gain so its region's loudest frame, on the channels
    /// it plays, lands at 0 dBFS (Normalize, one undo step). The app reads
    /// the audio: the tab has only its picture.
    Normalize { pad: usize, layer: usize },
    /// Make a new sound of the layer's region, `semitones` higher and
    /// stretched from its warp tempo to `bpm` when it is warped, and put
    /// it in the region's place (the Warp tab's Process; one undo step).
    /// The app runs it on a worker.
    Process {
        pad: usize,
        layer: usize,
        semitones: i32,
    },
    /// Stop the Process under way; nothing changes.
    StopProcess,
    /// The Sound section changed the pad ([`crate::sound_panel`]): the
    /// app applies it as the Play page's.
    Play(PlayAction),
    /// The recorder row, or the Input section while REC is on, asked for
    /// a take or a setting ([`crate::recorder`]).
    Rec(RecorderAction),
    /// Say this on the hint line (why a touch did nothing).
    Say(String),
    /// The Slice tool asked to hear a slice, to put the slices on pads,
    /// to export them or to chop live ([`crate::slice_tool`]).
    Chop(ChopAction),
}

/// The key a typed Warp tempo comes back with ([`NumberAsk::key`]).
pub const TYPED_WARP_TEMPO: &str = "warp-tempo";

/// What the Grain tool's note says after "Plays as One shot" while the
/// sound doesn't play as grains (an undo took Granular away).
pub const GRAIN_AGAIN: &str = "tap Grain to play it as grains";

/// "Plays as One shot", "Plays as a loop": the sound's mode, in words.
pub fn plays_as(mode: PlayMode) -> String {
    format!("Plays as {}", mode_label(mode))
}

/// The Warp tempo, to be typed: 20 to 999 BPM, to a hundredth.
pub fn warp_tempo_ask(r: &Region) -> Option<NumberAsk> {
    let w = r.warp.as_ref()?;
    Some(NumberAsk {
        key: TYPED_WARP_TEMPO.to_string(),
        title: "Warp tempo".to_string(),
        unit: "BPM".to_string(),
        value: f64::from(w.bpm),
        min: f64::from(*WARP_BPM.start()),
        max: f64::from(*WARP_BPM.end()),
        decimals: 2,
    })
}

/// `r` with its Warp tempo typed as `bpm` (pins fitted to it), if it is
/// warped.
pub fn with_warp_tempo(st: &WaveState, r: &Region, bpm: f64) -> Option<Region> {
    let mut w = r.warp.clone()?;
    w.bpm = (bpm as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end());
    w.fit(r.start, r.end, st.rate);
    Some(Region {
        warp: Some(w),
        ..r.clone()
    })
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
                let (layer, region) = (*layer, (**region).clone());
                if p.pad(*pad).is_none_or(|pd| layer >= pd.layers.len()) {
                    return Err(ModelError::BadPad);
                }
                p.edit_pad(*pad, Some(*gesture), move |pd| {
                    pd.layers[layer].region = region
                })
            }
            WaveAction::Play(a) => a.apply(p),
            _ => Ok(()),
        }
    }
}

/// The settings, as the list's rows. [`Row::ALL`] is the panel's order,
/// section by section, which the wheel walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Mode,
    Loop,
    Crossfade,
    Gain,
    Normalize,
    Reverse,
    Channels,
    FadeIn,
    FadeOut,
    Snap,
}

impl Row {
    pub const ALL: [Row; 10] = [
        Row::Mode,
        Row::Loop,
        Row::Crossfade,
        Row::Gain,
        Row::FadeIn,
        Row::FadeOut,
        Row::Normalize,
        Row::Reverse,
        Row::Snap,
        Row::Channels,
    ];

    fn name(self) -> &'static str {
        match self {
            Row::Mode => "Mode",
            Row::Loop => "Loop",
            Row::Crossfade => "Crossfade",
            Row::Gain => "Gain",
            Row::Normalize => "Normalize",
            Row::Reverse => "Reverse",
            Row::Channels => "Channels",
            Row::FadeIn => "Fade in",
            Row::FadeOut => "Fade out",
            Row::Snap => "Snap",
        }
    }

    /// Its place in [`Row::ALL`], the list's row.
    pub fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
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
/// What the Normalize row's value says (in a list).
pub const NORMALIZE_TEXT: &str = "To 0 dB";
/// What the Normalize button says on the panel.
pub const NORMALIZE_BUTTON: &str = "Normalize to 0 dB";
const LOOPS: [&str; 3] = ["Off", "Forward", "Ping-pong"];
/// The Gain row's range, in dB (Normalize keeps to it too).
pub const GAIN_RANGE_DB: (f32, f32) = (-48.0, 24.0);

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

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Marker(Mark),
    Row(Row),
    /// A Grain tool row, or the grain window ([`grain_tab::Row::Position`]
    /// and [`grain_tab::Row::Size`] for its tabs).
    Grain(grain_tab::Row),
    /// A Warp tool edit ([`warp_tool::Key`]).
    Warp(warp_tool::Key),
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
    /// The source the waveform shows and its peaks, so a new one starts
    /// zoomed out, and peaks worked out since (the same sound, analysed
    /// off the screen thread) come in where it is zoomed.
    shown: Option<(SourceId, Arc<dyn Peaks>)>,
    /// The waveform's width in pixels (its views are measured in it).
    wave_w: i32,
    /// The Sound section beside the panel.
    sound: SoundPanel,
    /// What the recorder row shows; while it records, the panel is Input.
    rec: RecorderView,
    /// The tool in use.
    tool: Tool,
    /// The Grain tool's part open.
    grain_part: GrainPart,
    /// What the engine last said of the cloud playing, and how many grains.
    cloud: (CloudNow, usize),
    /// Those grains: where each reads, 0 to 1 across the region, and its
    /// window's level.
    grain_marks: Vec<(f32, f32)>,
    /// A take recording, drawn into the wave ([`WaveTab::show_live`]): its
    /// frames so far and the span shown. The wave and the overview are
    /// the take's until [`WaveTab::end_live`].
    live: Option<(u64, u64)>,
    /// The Warp tool's settings and pin picked.
    warp: WarpTool,
    /// The wave as drawn ([`warp_tool::bend`]: as it plays with the Warp
    /// tool, else empty), to put playheads where they are drawn.
    bend: Vec<(f64, f64)>,
    /// The Slice tool's chop: its method, cuts, slice picked and where
    /// the slices go.
    slice: SliceTool,
}

impl WaveTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &WaveState) -> Result<WaveTab, String> {
        crate::send::add(ui);
        ui.add(BAR, PadBar::new(&[]));
        ui.add(OVERVIEW, WaveOverview::new());
        ui.add(WAVE, Waveform::new());
        ui.add(REGION, SettingList::new("Region"));
        // The marks' colours: Start and End, Loop start and Loop end.
        let colors = [
            Paint::Key(ColorKey::Accent),
            Paint::Mix(ColorKey::Accent, ColorKey::Bad, 55),
            Paint::Key(ColorKey::Good),
            Paint::Mix(ColorKey::Good, ColorKey::Warn, 50),
        ];
        for (k, m) in Mark::ALL.iter().enumerate() {
            ui.add(
                QLINKS[k],
                KnobCard::new(&format!("Q-Link {} · {}", k + 1, m.label()), colors[k])
                    .on(k, "Wave"),
            );
        }
        ui.add(crate::recorder::KEEP, crate::recorder::KeepCard::new());
        let labels: Vec<&str> = Tool::ALL.iter().map(|t| t.label()).collect();
        ui.add(TOOLS, ToolStrip::new(&labels));
        ui.add(RECORDER, Recorder::new());
        let sound = SoundPanel::add(ui);
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
            sound,
            rec: RecorderView::default(),
            tool: Tool::default(),
            grain_part: GrainPart::default(),
            cloud: (CloudNow::default(), 0),
            grain_marks: Vec::new(),
            live: None,
            warp: WarpTool::default(),
            bend: Vec::new(),
            slice: SliceTool::default(),
        };
        tab.update(ui, st);
        Ok(tab)
    }

    pub fn snap(&self) -> Snap {
        self.snap
    }

    /// The tool in use.
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Picks tool `tool` (the app's way in: SAMPLE EDIT, a device check,
    /// Chop to pads).
    pub fn set_tool(&mut self, ui: &mut Ui, st: &WaveState, tool: Tool) {
        if tool == self.tool {
            return;
        }
        if self.tool == Tool::Slice {
            self.slice.leave();
        }
        self.tool = tool;
        self.gesture = None;
        if let Some(list) = ui.get_mut::<SettingList>(REGION) {
            list.selected = 0;
        }
        self.show_tools(ui);
        self.update(ui, st);
    }

    /// The Grain tool's part open.
    pub fn grain_part(&self) -> GrainPart {
        self.grain_part
    }

    /// The Warp tool's settings and pin picked.
    pub fn warp(&self) -> &WarpTool {
        &self.warp
    }

    /// The Slice tool's chop: its method, cuts and slice picked.
    pub fn slice(&self) -> &SliceTool {
        &self.slice
    }

    /// The chop to keep for the sound once the Slice tool has changed it
    /// ([`SliceTool::take_kept`]); the app keeps it in the project. None
    /// with another tool: the Slice tool only follows the sound while it is
    /// picked, so what it holds then is nothing to keep.
    pub fn take_kept(&mut self, st: &WaveState) -> Option<(ChopMode, Vec<u64>)> {
        if self.tool != Tool::Slice {
            return None;
        }
        let cs = st.chop.as_deref()?;
        self.slice.take_kept(cs)
    }

    /// The slice picked and the pad that plays it, when the Slice tool is
    /// picked and that slice is on a pad already
    /// ([`SliceTool::slice_pad`]): the Sound section follows it there, so
    /// its knobs set what that pad plays (the app gives it that pad's
    /// [`PlayState`], with [`PlayState::slice`]). None: the Sound section
    /// is the page's sound, which each slice's pad will take.
    pub fn sound_follows(&self, st: &WaveState) -> Option<(usize, usize)> {
        if self.tool != Tool::Slice || self.rec.recording {
            return None;
        }
        let cs = st.chop.as_deref()?;
        let k = self.slice.selected();
        Some((k, self.slice.slice_pad(cs, k)?))
    }

    /// Whether the Slice tool waits for the sound's pitch to be followed
    /// ([`SliceTool::wants_pitch`]).
    pub fn wants_pitch(&self, st: &WaveState) -> bool {
        self.tool == Tool::Slice
            && st
                .chop
                .as_deref()
                .is_some_and(|cs| self.slice.wants_pitch(cs))
    }

    /// Live chop stopped here (another pad or region, or a change on the
    /// panel): the [`ChopAction::Live`] that tells the app, once.
    pub fn take_stop(&mut self, st: &WaveState) -> Option<ChopAction> {
        let cs = st.chop.as_deref()?;
        self.slice.take_stop(cs)
    }

    /// Live chop's cuts, where the sound was at each pad played
    /// ([`SliceTool::live_marks`]), shown at once. Returns how many were
    /// kept.
    pub fn live_marks(&mut self, ui: &mut Ui, st: &WaveState, marks: &[u64]) -> usize {
        let Some(cs) = st.chop.clone() else {
            return 0;
        };
        let kept = self.slice.live_marks(&cs, marks);
        if kept > 0 {
            self.show_slice(ui, st);
        }
        kept
    }

    /// The sound live chop played ended: Start again starts a new take.
    pub fn live_ended(&mut self, ui: &mut Ui, st: &WaveState) {
        if self.slice.live_ended() {
            self.show_slice(ui, st);
        }
    }

    /// Presses the Slice tool's button `b` (as a tap on it would): Chop
    /// to pads beside the bar presses its main button.
    pub fn press_slice(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        b: slice_tool::Button,
    ) -> Option<WaveAction> {
        let cs = st.chop.clone()?;
        let px = self.px(ui);
        let a = self.slice.press(&cs, b, px);
        self.slice_done(ui, st, a)
    }

    /// A Process runs and has got this far (0 to 1), or none runs: the
    /// Warp section's Process button says so, and stops it while it runs.
    pub fn set_processing(&mut self, ui: &mut Ui, st: &WaveState, done: Option<f32>) {
        if self.warp.set_processing(done) && self.tool == Tool::Warp && !self.rec.recording {
            self.show_list(ui, st);
        }
    }

    /// A number typed for the field `key` asked for ([`WaveAction::Ask`]):
    /// the change, as an undo step of its own.
    pub fn typed(&mut self, st: &WaveState, key: &str, value: f64) -> Option<WaveAction> {
        let r = WarpTool::typed(st, key, value)?;
        self.gesture = None;
        let out = self.edit(
            st,
            Key::Warp(warp_tool::Key::Row(warp_tool::Row::WarpTempo)),
            r,
        );
        self.gesture = None;
        out
    }

    /// Opens part `part` of the Grain section.
    pub fn open_grain_part(&mut self, ui: &mut Ui, st: &WaveState, part: GrainPart) {
        if part == self.grain_part {
            return;
        }
        self.grain_part = part;
        self.gesture = None;
        if let Some(list) = ui.get_mut::<SettingList>(REGION) {
            list.selected = 0;
        }
        if self.tool == Tool::Grain && !self.rec.recording {
            self.show_list(ui, st);
            self.show_grain_cards(ui, st);
        }
    }

    /// The hint line for the tool in use.
    pub fn hint(&self) -> &'static str {
        self.tool.hint()
    }

    /// The grains the engine plays now for the sound shown, and what else
    /// it says of the cloud: the spray as it moves draws on the wave, and
    /// the Grain section's note says how many play and when it is thinned.
    /// Each grain is drawn on the wave where it reads, as its window opens.
    pub fn set_cloud(&mut self, ui: &mut Ui, st: &WaveState, grains: &[GrainView], now: CloudNow) {
        let marks: Vec<(f32, f32)> = grains.iter().map(|g| (g.position, g.level)).collect();
        if self.cloud == (now, grains.len()) && self.grain_marks == marks {
            return;
        }
        self.cloud = (now, grains.len());
        self.grain_marks = marks;
        if self.tool == Tool::Grain {
            self.show_wave(ui, st);
            if !self.rec.recording {
                self.show_list(ui, st);
            }
        }
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
        self.sound.lift();
    }

    /// Shows pad `st` in the Sound section (the app makes it as for the
    /// Play page; None while it has none).
    pub fn set_sound(&mut self, ui: &mut Ui, st: Option<PlayState>) {
        self.sound.update(ui, st);
    }

    /// The Sound section.
    pub fn sound(&self) -> &SoundPanel {
        &self.sound
    }

    /// Shows `v` on the recorder row. While it records, the panel under
    /// the wave is its Input section; when it stops, the region's settings
    /// again (`st`, the state shown).
    pub fn set_recorder(&mut self, ui: &mut Ui, v: RecorderView, st: &WaveState) {
        if v == self.rec {
            return;
        }
        let was = self.rec.recording;
        if v.keep != self.rec.keep {
            if let Some(w) = ui.get_mut::<crate::recorder::KeepCard>(crate::recorder::KEEP) {
                w.view = v.keep.clone();
            }
        }
        self.rec = v.clone();
        if let Some(w) = ui.get_mut::<Recorder>(RECORDER) {
            w.view = v;
        }
        self.show_tools(ui);
        if was != self.rec.recording || self.rec.recording {
            self.show_list(ui, st);
            self.show_cards(ui, st);
        }
    }

    /// The Keep card's picture of what a tap keeps
    /// ([`crate::recorder::keep_wave`]); redrawn only when it changes.
    pub fn set_keep_wave(&self, ui: &mut Ui, wave: Vec<f32>) {
        use crate::recorder::{KeepCard, KEEP};
        if ui.get::<KeepCard>(KEEP).is_some_and(|w| w.wave != wave) {
            if let Some(w) = ui.get_mut::<KeepCard>(KEEP) {
                w.wave = wave;
            }
        }
    }

    /// What the recorder row shows.
    pub fn recorder(&self) -> &RecorderView {
        &self.rec
    }

    /// The source's peak on each side over the last `dt` seconds, for the
    /// row's meter (redrawn only when it moves a pixel).
    pub fn meter(&self, ui: &mut Ui, peak: [f32; 2], dt: f32) {
        let moved = ui
            .get_mut_quiet::<Recorder>(RECORDER)
            .is_some_and(|w| w.meter(peak, dt));
        if moved {
            ui.get_mut::<Recorder>(RECORDER);
        }
    }

    /// Where the pad's voices are playing, in source frames. Only their
    /// strips are redrawn. While a take records, the head is the take's.
    pub fn set_playheads(&self, ui: &mut Ui, frames: &[u64]) {
        if self.live.is_some() {
            return;
        }
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            if self.bend.is_empty() {
                w.set_playheads(frames);
            } else {
                let drawn: Vec<u64> = frames
                    .iter()
                    .map(|&f| warp_tool::played(&self.bend, f).round().max(0.0) as u64)
                    .collect();
                w.set_playheads(&drawn);
            }
        }
    }

    /// A take recording, drawn into the wave and the overview as it comes
    /// in (the one Sample page's step 2f, part 3): `take` so far across
    /// its span (`first`, the take's planned length or a few bars,
    /// doubled while the take outgrows it), the part recorded bright, the
    /// head where it has got to, `beats` its bar lines; no marks to drag,
    /// and a touch on the wave does nothing until it ends. The pad bar
    /// names it (`title`, `subtitle`).
    pub fn show_live(
        &mut self,
        ui: &mut Ui,
        take: &crate::live_take::LiveTake,
        first: u64,
        beats: Option<Beats>,
        title: &str,
        subtitle: &str,
    ) {
        let span = take.span(first);
        let frames = take.frames();
        let peaks = take.peaks(span);
        let width = self.wave_w;
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            w.set_peaks(Some(peaks.clone()), width);
            // The take is drawn as it came in: no Warp pins, bend or hits.
            w.set_bend(Vec::new());
            w.set_ticks(Vec::new(), false);
            w.set_lane(Vec::new());
            w.set_markers(Vec::new());
            w.set_window(None);
            w.set_grains(Vec::new());
            w.set_region(Some((0, frames)));
            w.set_loop(None);
            w.set_fades(0, 0);
            w.set_beats(beats);
            w.set_playheads(&[frames]);
        }
        if let Some(o) = ui.get_mut::<WaveOverview>(OVERVIEW) {
            o.set_peaks(Some(peaks));
            o.set_region(Some((0, frames)));
        }
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = Paint::from(Role::Record);
            bar.title = title.to_string();
            bar.subtitle = subtitle.to_string();
        }
        // The next sound shown starts zoomed out.
        self.shown = None;
        self.bend.clear();
        self.live = Some((frames, span));
        self.sync_overview(ui);
    }

    /// The take has ended: the wave is the page's sound's again once
    /// [`WaveTab::update`] shows it.
    pub fn end_live(&mut self, ui: &mut Ui) {
        if self.live.take().is_some() {
            if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
                w.set_playheads(&[]);
            }
        }
    }

    /// A take's frames so far and the span the wave shows, while one is
    /// drawn into it.
    pub fn live(&self) -> Option<(u64, u64)> {
        self.live
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &WaveState) {
        self.sound.layer = st.layer;
        let r = &st.region;
        let src = &st.source;
        if st.region.sustain_loop.is_none() && self.focus.is_loop() {
            self.focus = Mark::Start;
        }
        // Sixteenths need a tempo; one still being worked out may come.
        if self.snap == Snap::Sixteenths && src.tempo.is_none() && src.ready {
            self.snap = Snap::ZeroCrossing;
        }
        let pad_color = match st.take {
            Some(_) => Paint::from(Role::Record),
            None => Paint::Content(pad_color(st.pad)),
        };
        let live = self.live.is_some();
        if let Some(bar) = ui.get_mut::<PadBar>(BAR).filter(|_| !live) {
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
            bar.title = match st.take {
                // The mockup's "Take 3 · new": on the page, on no pad yet.
                Some(_) => format!("{} · new", st.pad_name),
                None => format!("Pad {pad} · {}{layer}", st.pad_name),
            };
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
            match st.take {
                Some(t) if t.of > 1 => about.push(format!("take {} of {}", t.index + 1, t.of)),
                Some(_) => {}
                None if st.used_by > 1 => about.push(format!("used by {} pads", st.used_by)),
                None => {}
            }
            bar.subtitle = about.join(" · ");
        }
        let width = self.wave_w;
        if let Some(w) = ui.get_mut::<Waveform>(WAVE).filter(|_| !live) {
            match &self.shown {
                Some((id, p)) if *id == src.id => {
                    if !std::ptr::addr_eq(Arc::as_ptr(p), Arc::as_ptr(&src.peaks)) {
                        let view = w.view();
                        w.set_peaks(Some(src.peaks.clone()), width);
                        w.set_view(view, width);
                    }
                }
                _ => w.set_peaks(Some(src.peaks.clone()), width),
            }
            self.shown = Some((src.id, src.peaks.clone()));
            w.set_region(Some((r.start, r.end)));
            w.set_fades(u64::from(r.fade_in.frames), u64::from(r.fade_out.frames));
        }
        self.warp.follow(st);
        self.slice.bank = self.bank;
        if let (Tool::Slice, Some(cs)) = (self.tool, st.chop.as_deref()) {
            self.slice.follow(cs);
        }
        self.show_wave(ui, st);
        if let Some(o) = ui.get_mut::<WaveOverview>(OVERVIEW).filter(|_| !live) {
            o.set_peaks(Some(src.peaks.clone()));
            o.set_region(Some((r.start, r.end)));
        }
        self.sync_overview(ui);
        self.show_list(ui, st);
        self.show_cards(ui, st);
        self.update_wheel(ui, st);
    }

    /// The marks on the wave: every tool's drawn, but only the tool
    /// picked has tabs in the strip; the other tools' marks are faint
    /// lines with no tab ([`Marker::line_only`]). Trim's are the region's
    /// and the loop's (a sliced sound's cuts are the Slice tool's); the
    /// Grain tool's the grain window and its two tabs; the Warp tool's the
    /// pins, with the wave drawn as it plays and the bar lines from the
    /// region's start at its warp tempo. The hits found are ticks along
    /// the wave's foot with every tool. A sound playing as grains shows its
    /// window with every tool.
    fn show_wave(&mut self, ui: &mut Ui, st: &WaveState) {
        // A take drawing into the wave has no marks.
        if self.live.is_some() {
            return;
        }
        let r = &st.region;
        let warp = self.tool == Tool::Warp;
        self.bend = if warp {
            warp_tool::bend(st)
        } else {
            Vec::new()
        };
        let bend = &self.bend;
        let drawn = |f: u64| warp_tool::played(bend, f).round().max(0.0) as u64;
        let grains = self.tool == Tool::Grain || r.mode == PlayMode::Granular;
        let span = grains
            .then(|| grain_tool::span(r, st.rate, st.bpm, self.cloud.0.state.map(|c| c.spray)))
            .map(|s| GrainSpan {
                at: drawn(s.at),
                ..s
            });
        let mut all = markers(r);
        if self.tool != Tool::Trim {
            all = all
                .into_iter()
                .map(|m| Marker {
                    frame: drawn(m.frame),
                    ..m.faint().line_only()
                })
                .collect();
        }
        if let (Tool::Grain, Some(s)) = (self.tool, span.as_ref()) {
            all.extend(grain_tool::markers(s));
        }
        // The Slice tool's cuts in place of the region's marks: slice 1
        // starts at Start and the last ends at END, and each cut is in its
        // slice's colour, as the lane under the wave is.
        let tint = |k: usize| Paint::Key(ColorKey::Track((k % 8) as u8));
        let chop = st.chop.as_deref().filter(|_| self.tool == Tool::Slice);
        let mut lane = Vec::new();
        if let Some(cs) = chop {
            let marks = self.slice.markers(cs);
            let last = marks.len() - 1;
            all = marks
                .into_iter()
                .enumerate()
                .map(|(k, m)| {
                    if k > 0 && k < last {
                        m.colored(tint(k))
                    } else {
                        m
                    }
                })
                .collect();
            let picked = self.slice.selected();
            lane = self
                .slice
                .lands(cs)
                .into_iter()
                .enumerate()
                .map(|(k, l)| LaneCell {
                    start: l.start,
                    end: l.end,
                    text: l.text,
                    color: tint(k),
                    lit: l.on,
                    picked: k == picked,
                })
                .collect();
        }
        all.extend(self.warp.markers(
            st,
            bend,
            warp,
            Paint::Key(ColorKey::Warn),
            Paint::Key(ColorKey::Text),
        ));
        let hits: Vec<u64> = self.warp.hits_of(st).iter().map(|&h| drawn(h)).collect();
        let beats = if warp {
            Some(warp_tool::beats(st, self.warp.grid))
        } else {
            st.beats()
        };
        // The grains playing, with the Grain tool open.
        let grain_marks: Vec<(u64, f32)> = if self.tool == Tool::Grain {
            let len = r.frames() as f64;
            self.grain_marks
                .iter()
                .map(|&(at, level)| (drawn(r.start + (f64::from(at) * len) as u64), level))
                .collect()
        } else {
            Vec::new()
        };
        // The slice picked is shaded where the loop is with the others.
        let lp = match chop {
            Some(cs) => Some(self.slice.picked(cs)),
            None => r.sustain_loop.map(|l| (drawn(l.start), drawn(l.end))),
        };
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            w.set_bend(self.bend.clone());
            w.set_lane(lane);
            w.set_markers(all);
            w.set_window(span);
            w.set_grains(grain_marks);
            w.set_ticks(hits, warp);
            w.set_beats(beats);
            w.set_loop(lp);
        }
    }

    /// The tools strip: the tool in use, faint while a take records.
    fn show_tools(&self, ui: &mut Ui) {
        let (active, recording) = (
            Tool::ALL.iter().position(|t| *t == self.tool).unwrap_or(0),
            self.rec.recording,
        );
        if let Some(w) = ui.get_mut_quiet::<ToolStrip>(TOOLS) {
            if w.active == active && w.enabled == !recording {
                return;
            }
        }
        if let Some(w) = ui.get_mut::<ToolStrip>(TOOLS) {
            w.active = active;
            w.enabled = !recording;
            w.why = TOOLS_WAIT.to_string();
        }
    }

    /// The Q-Link cards: the markers, or while REC is on the Input's
    /// Gain, Threshold, Pre-roll and Monitor's level
    /// ([`recorder::QLINK_ROWS`]).
    fn show_cards(&self, ui: &mut Ui, st: &WaveState) {
        if self.rec.recording {
            for (k, name) in QLINKS.iter().enumerate() {
                let (Some(card), Some(q)) = (
                    ui.get_mut::<KnobCard>(name),
                    recorder::qlink_card(&self.rec, k),
                ) else {
                    continue;
                };
                card.focused = false;
                card.label = format!("Q-Link {} · {}", k + 1, q.name);
                card.value = q.value;
                card.sub = q.sub;
                card.pos = q.pos;
                card.enabled = q.faint.is_none();
                card.why = q.faint.unwrap_or_default();
            }
            return;
        }
        if self.tool == Tool::Grain {
            self.show_grain_cards(ui, st);
            return;
        }
        if self.tool == Tool::Warp {
            self.show_warp_cards(ui, st);
            return;
        }
        if let (Tool::Slice, Some(cs)) = (self.tool, st.chop.as_deref()) {
            let row = self.slice_row(ui);
            for (k, c) in self.slice.cards(cs).into_iter().enumerate() {
                let Some(card) = ui.get_mut::<KnobCard>(QLINKS[k]) else {
                    continue;
                };
                card.label = c.label;
                card.value = c.value;
                card.sub = c.sub;
                card.pos = c.pos;
                card.enabled = c.enabled;
                card.why = c.why;
                card.focused = row.is_some() && SLICE_CARD_ROWS[k] == row;
            }
            return;
        }
        let (r, src) = (&st.region, &st.source);
        for (k, m) in Mark::ALL.iter().enumerate() {
            if let Some(card) = ui.get_mut::<KnobCard>(QLINKS[k]) {
                card.label = format!("Q-Link {} · {}", k + 1, m.label());
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
                card.why = "No loop: set Mode to Loop first".to_string();
                card.focused = *m == self.focus;
            }
        }
    }

    /// The Q-Link cards with the Grain tool: Position, Size, Density and
    /// Pitch, as the Grain section's rows.
    fn show_grain_cards(&self, ui: &mut Ui, st: &WaveState) {
        let r = &st.region;
        let g = &r.grain;
        let picked = self.grain_row(ui, st);
        for (k, row) in grain_tab::QLINK_ROWS.iter().enumerate() {
            let Some(card) = ui.get_mut::<KnobCard>(QLINKS[k]) else {
                continue;
            };
            card.label = format!("Q-Link {} · {}", k + 1, row.short());
            (card.value, card.sub) = grain_tab::split_unit(&grain_tab::value_text(r, *row));
            card.pos = match row {
                grain_tab::Row::Position => g.position,
                grain_tab::Row::Size => grain_tab::size_y(g),
                grain_tab::Row::Density => {
                    let (lo, hi) = grain_tab::DENSITY_RANGE;
                    (g.density / lo).ln() / (hi / lo).ln()
                }
                _ => (g.pitch + grain_tab::PITCH_LIMIT) / (2.0 * grain_tab::PITCH_LIMIT),
            }
            .clamp(0.0, 1.0);
            card.enabled = true;
            card.why = String::new();
            card.focused = Some(*row) == picked || Some(grain_tab::SHIFT_ROWS[k]) == picked;
        }
    }

    /// The Q-Link cards with the Warp tool: Pin, Pin beat, Warp tempo and
    /// Scroll ([`WarpTool::cards`]).
    fn show_warp_cards(&self, ui: &mut Ui, st: &WaveState) {
        let scroll = self.view(ui).map_or((0.0, false), |v| {
            let whole = st.frames as f64 / f64::from(self.wave_w.max(1));
            (
                warp_tool::beat_drawn(st, v.start),
                v.frames_per_px < whole * 0.999,
            )
        });
        let row = self.warp_row(ui);
        let rows = [
            warp_tool::Row::Pin,
            warp_tool::Row::PinBeat,
            warp_tool::Row::WarpTempo,
        ];
        for (k, c) in self.warp.cards(st, scroll).into_iter().enumerate() {
            let Some(card) = ui.get_mut::<KnobCard>(QLINKS[k]) else {
                continue;
            };
            card.label = c.label;
            card.value = c.value;
            card.sub = c.sub;
            card.pos = c.pos;
            card.enabled = c.enabled;
            card.why = c.why;
            card.focused = rows.get(k).is_some_and(|r| Some(*r) == row);
        }
    }

    /// The Warp tool's row picked on the panel.
    fn warp_row(&self, ui: &Ui) -> Option<warp_tool::Row> {
        let i = ui.get::<SettingList>(REGION)?.selected;
        warp_tool::Row::ALL.get(i).copied()
    }

    /// The Grain tool plays the sound as grains: picked (or tapped again,
    /// after an undo) on a sound in another mode, it makes it Granular,
    /// one undo step (Jay's device notes, 20 Nov 2026: "the grain tab
    /// should automatically be in granular mode"). The Trim tool's Mode
    /// puts it back.
    fn grain_on(&mut self, st: &WaveState) -> Option<WaveAction> {
        if self.tool != Tool::Grain || st.region.mode == PlayMode::Granular {
            return None;
        }
        let mut r = st.region.clone();
        r.mode = PlayMode::Granular;
        self.gesture = None;
        let a = self.edit(st, Key::Row(Row::Mode), r);
        self.gesture = None;
        a
    }

    /// The Grain tool's row picked on the panel, if one is.
    fn grain_row(&self, ui: &Ui, st: &WaveState) -> Option<grain_tab::Row> {
        let i = ui.get::<SettingList>(REGION)?.selected;
        self.grain_part
            .rows(&st.region)
            .get(i.checked_sub(1)?)
            .copied()
    }

    /// What the Grain section's note says: what the cloud is doing, or
    /// the part open's own word ([`grain_tool::part_note`]).
    fn grain_note(&self, st: &WaveState) -> String {
        let r = &st.region;
        if r.mode != PlayMode::Granular {
            return format!("{}: {GRAIN_AGAIN}", plays_as(r.mode));
        }
        if let Some(note) = grain_tool::part_note(r, self.grain_part) {
            return note.to_string();
        }
        let (now, n) = &self.cloud;
        let mut parts = vec![match n {
            0 => "hit the pad to hear it".to_string(),
            1 => "1 grain playing".to_string(),
            n => format!("{n} grains playing"),
        }];
        if now.state.is_some_and(|c| c.thinned) {
            parts.push(format!(
                "thinned to fit {}",
                mpfree_sampler::engine::MAX_GRAINS
            ));
        }
        parts.join(" · ")
    }

    /// The settings' rows, controls and panel.
    fn show_list(&self, ui: &mut Ui, st: &WaveState) {
        if self.rec.recording {
            let (rows, controls, panel) = recorder::input_list(&self.rec);
            if let Some(list) = ui.get_mut::<SettingList>(REGION) {
                list.right = "REC".to_string();
                list.rows = rows;
                list.controls = controls;
                list.panel = Some(panel);
                list.typed = Vec::new();
                list.buttons = Vec::new();
                list.selected = list.selected.min(INPUT_ROWS.len() - 1);
            }
            return;
        }
        if self.tool == Tool::Warp {
            let no_hits = self
                .warp
                .clone()
                .hits_of(st)
                .iter()
                .all(|&h| h <= st.region.start || h >= st.region.end);
            let (rows, controls, panel) = self.warp.section(st, no_hits);
            let n = rows.len();
            if let Some(list) = ui.get_mut::<SettingList>(REGION) {
                list.right = self.warp.grid.label().to_string();
                list.rows = rows;
                list.controls = controls;
                list.panel = Some(panel);
                list.typed = if st.region.warp.is_some() {
                    vec![warp_tool::Row::WarpTempo.index()]
                } else {
                    Vec::new()
                };
                list.buttons = warp_tool::Row::BUTTONS.iter().map(|k| k.index()).collect();
                list.selected = list.selected.min(n - 1);
            }
            return;
        }
        if let (Tool::Slice, Some(cs)) = (self.tool, st.chop.as_deref()) {
            let (rows, controls, panel) = self.slice.section(cs);
            let n = rows.len();
            if let Some(list) = ui.get_mut::<SettingList>(REGION) {
                list.right = self.slice.put_note(cs);
                list.rows = rows;
                list.controls = controls;
                list.panel = Some(panel);
                list.typed = Vec::new();
                list.buttons = slice_tool::Button::ALL
                    .iter()
                    .map(|b| b.row().index())
                    .collect();
                list.selected = list.selected.min(n - 1);
            }
            return;
        }
        if self.tool == Tool::Grain {
            let (rows, controls, panel) =
                grain_tool::section(&st.region, self.grain_part, &self.grain_note(st));
            let n = rows.len();
            if let Some(list) = ui.get_mut::<SettingList>(REGION) {
                list.right = "grains".to_string();
                list.rows = rows;
                list.controls = controls;
                list.panel = Some(panel);
                list.typed = Vec::new();
                list.buttons = Vec::new();
                list.selected = list.selected.min(n - 1);
            }
            return;
        }
        let r = &st.region;
        let rows = self.rows(st);
        let controls = self.controls(st);
        let panel = panel();
        if let Some(list) = ui.get_mut::<SettingList>(REGION) {
            list.right = mode_label(r.mode).to_string();
            list.rows = rows;
            list.controls = controls;
            list.panel = Some(panel);
            list.typed = Vec::new();
            list.buttons = vec![Row::Normalize.index()];
        }
    }

    /// How each row looks on the panel, in [`Row::ALL`]'s order.
    fn controls(&self, st: &WaveState) -> Vec<Control> {
        let r = &st.region;
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let forward = r.sustain_loop.filter(|l| l.mode == LoopMode::Forward);
        let knob = |list: &[f64], frames: u64, bipolar| Look::Knob {
            pos: ms_place(list, frames, st.rate),
            steps: list.len() as u32 - 1,
            bipolar,
        };
        Row::ALL
            .iter()
            .map(|k| match k {
                Row::Mode => Control::new(Look::Choice {
                    options: MODES.iter().map(|m| mode_label(*m).to_string()).collect(),
                    on: MODES.iter().position(|m| *m == r.mode).unwrap_or(0),
                    off: Vec::new(),
                }),
                Row::Loop => Control::new(Look::Choice {
                    options: names(&LOOPS),
                    on: match r.sustain_loop {
                        None => 0,
                        Some(l) if l.mode == LoopMode::Forward => 1,
                        Some(_) => 2,
                    },
                    // Loop mode can't be without a loop.
                    off: if r.mode == PlayMode::Loop {
                        vec![0]
                    } else {
                        Vec::new()
                    },
                }),
                Row::Crossfade => Control::new(knob(
                    &CROSSFADE_MS,
                    forward.map_or(0, |l| u64::from(l.crossfade)),
                    false,
                ))
                .faint_if(forward.is_none()),
                Row::Gain => Control::new(Look::Knob {
                    pos: (r.gain_db - GAIN_RANGE_DB.0) / (GAIN_RANGE_DB.1 - GAIN_RANGE_DB.0),
                    steps: ((GAIN_RANGE_DB.1 - GAIN_RANGE_DB.0) / GAIN_STEP_DB) as u32,
                    bipolar: false,
                }),
                Row::Normalize => Control::new(Look::Button(NORMALIZE_BUTTON.into())),
                Row::Reverse => Control::new(Look::Toggle(r.reverse)),
                Row::Channels => Control::new(Look::Chip).faint_if(st.channels == 1),
                Row::FadeIn => Control::new(knob(&FADE_MS, u64::from(r.fade_in.frames), false)),
                Row::FadeOut => Control::new(knob(&FADE_MS, u64::from(r.fade_out.frames), false)),
                Row::Snap => Control::new(Look::Chip),
            })
            .collect()
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
                    Row::Normalize => NORMALIZE_TEXT.into(),
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
        let round = match (self.tool, self.warp.selected()) {
            (Tool::Warp, Some(i)) => format!("pin {}", i + 1),
            (Tool::Warp, None) => "middle".to_string(),
            (Tool::Slice, _) => format!("slice {}", self.slice.selected() + 1),
            _ => self.focus.label().to_lowercase(),
        };
        let label = format!("{zoom:.1}× · {round}");
        if ui
            .get::<WaveOverview>(OVERVIEW)
            .is_some_and(|o| o.label() != label)
        {
            if let Some(o) = ui.get_mut::<WaveOverview>(OVERVIEW) {
                o.set_label(&label);
            }
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
        if ev.widget == sound_panel::SOUND {
            if let Some((row, delta)) = self.sound.grain_step(ev) {
                // The Grain chip's four are the region's, as the Q-Links'.
                self.sound.lift();
                if delta == 0 || st.region.mode != PlayMode::Granular {
                    return None;
                }
                let r = grain_tab::stepped(&st.region, row, delta, st.bpm);
                return self.edit(st, Key::Grain(row), r);
            }
            // The Sound section's own undo steps; the panel's ends.
            self.gesture = None;
            return self.sound.action(ui, ev).map(WaveAction::Play);
        }
        if ev.widget == RECORDER {
            return recorder::action(&self.rec, &ev.kind).map(WaveAction::Rec);
        }
        if ev.widget == recorder::KEEP {
            return recorder::keep_action(&ev.kind).map(WaveAction::Rec);
        }
        if self.live.is_some() && (ev.widget == WAVE || ev.widget == OVERVIEW) {
            return Some(WaveAction::Say(LIVE_WHY.to_string()));
        }
        if ev.widget == REGION && self.rec.recording {
            // The panel is the Input section while REC is on.
            return match &ev.kind {
                EventKind::Step { index, delta } => {
                    recorder::input_action(*index, *delta).map(WaveAction::Rec)
                }
                _ => None,
            };
        }
        match (ev.widget.as_str(), &ev.kind) {
            (TOOLS, EventKind::Tab(i)) => {
                let tool = *Tool::ALL.get(*i)?;
                self.set_tool(ui, st, tool);
                self.grain_on(st)
            }
            (WAVE, EventKind::Marker { index, frame }) if self.tool == Tool::Slice => {
                let cs = st.chop.clone()?;
                let px = self.px(ui);
                let at = self.slice.dragged(&cs, *index, *frame, px);
                // The widget moved the tab with the finger; put it where
                // the cut really is.
                if let (Some(w), Some(f)) = (ui.get_mut::<Waveform>(WAVE), at) {
                    w.set_marker_frame(*index, f);
                }
                self.slice_done(ui, st, None)
            }
            (WAVE, EventKind::Frame(f)) if self.tool == Tool::Slice => {
                let cs = st.chop.clone()?;
                let a = self.slice.tap(&cs, *f);
                self.slice_done(ui, st, a)
            }
            (REGION, EventKind::Step { index, delta }) if self.tool == Tool::Slice => {
                let cs = st.chop.clone()?;
                let row = *slice_tool::Row::ALL.get(*index)?;
                let stop = self.slice.step(&cs, row, *delta);
                self.slice_done(ui, st, stop)
            }
            // A pad of Put on's map tapped: the slices start there.
            (REGION, EventKind::Cell { col, .. }) if self.tool == Tool::Slice => {
                let cs = st.chop.clone()?;
                self.slice.cell(&cs, *col);
                self.slice_done(ui, st, None)
            }
            (REGION, EventKind::Tab(index)) if self.tool == Tool::Slice => {
                let b = slice_tool::Row::ALL.get(*index)?.button()?;
                self.press_slice(ui, st, b)
            }
            (REGION, EventKind::Select(_)) if self.tool == Tool::Slice => {
                self.show_cards(ui, st);
                None
            }
            (WAVE, EventKind::Marker { index, frame }) if self.tool == Tool::Warp => {
                self.warp_marker(ui, st, *index, *frame)
            }
            (WAVE, EventKind::Frame(f)) if self.tool == Tool::Warp => {
                let at = warp_tool::source(&self.bend, *f);
                match self.warp.tap(st, at) {
                    Tapped::Picked => {
                        self.show_warp(ui, st);
                        None
                    }
                    Tapped::Pinned(r) => self.edit(st, Key::Warp(warp_tool::Key::Tap), *r),
                    Tapped::Say(why) => Some(WaveAction::Say(why.to_string())),
                }
            }
            (REGION, EventKind::Step { index, delta }) if self.tool == Tool::Warp => {
                let row = *warp_tool::Row::ALL.get(*index)?;
                let after = self.warp.step(st, row, *delta);
                self.warp_edit(ui, st, warp_tool::Key::Row(row), after)
            }
            (REGION, EventKind::Type(index))
                if self.tool == Tool::Warp
                    && warp_tool::Row::ALL.get(*index) == Some(&warp_tool::Row::WarpTempo) =>
            {
                warp_tempo_ask(&st.region).map(WaveAction::Ask)
            }
            (REGION, EventKind::Tab(index)) if self.tool == Tool::Warp => {
                let row = *warp_tool::Row::ALL.get(*index)?;
                // A press is a step of its own.
                self.gesture = None;
                if row == warp_tool::Row::Process {
                    return Some(match self.warp.processing() {
                        Some(_) => WaveAction::StopProcess,
                        None => WaveAction::Process {
                            pad: st.pad,
                            layer: st.layer,
                            semitones: self.warp.shift,
                        },
                    });
                }
                let after = self.warp.button(st, row);
                let out = self.warp_edit(ui, st, warp_tool::Key::Button(row), after);
                self.gesture = None;
                out
            }
            (REGION, EventKind::Select(_)) if self.tool == Tool::Warp => {
                self.show_warp_cards(ui, st);
                None
            }
            (WAVE, EventKind::Marker { index, frame }) if self.tool == Tool::Grain => {
                self.grain_marker(ui, st, *index, *frame)
            }
            (WAVE, EventKind::Frame(f)) if self.tool == Tool::Grain => {
                let r = grain_tool::moved_start(&st.region, *f);
                self.edit(st, Key::Grain(grain_tab::Row::Position), r)
            }
            (REGION, EventKind::Step { index, delta }) if self.tool == Tool::Grain => {
                self.grain_step(ui, st, *index, *delta)
            }
            (REGION, EventKind::Select(_)) if self.tool == Tool::Grain => {
                self.show_grain_cards(ui, st);
                None
            }
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
                if self.tool == Tool::Warp {
                    self.show_warp_cards(ui, st);
                }
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
            (REGION, EventKind::Tab(index)) if Row::ALL.get(*index) == Some(&Row::Normalize) => {
                // A press ends the turn before it: Normalize is a step of
                // its own.
                self.gesture = None;
                Some(WaveAction::Normalize {
                    pad: st.pad,
                    layer: st.layer,
                })
            }
            (name, EventKind::Select(_)) if QLINKS.contains(&name) && self.tool == Tool::Slice => {
                let k = QLINKS.iter().position(|q| *q == name)?;
                self.pick_slice_row(ui, st, k);
                None
            }
            (name, EventKind::Select(_)) if QLINKS.contains(&name) && self.tool == Tool::Warp => {
                let k = QLINKS.iter().position(|q| *q == name)?;
                self.pick_warp_row(ui, st, k);
                None
            }
            (name, EventKind::Select(_)) if QLINKS.contains(&name) && self.tool == Tool::Grain => {
                let k = QLINKS.iter().position(|q| *q == name)?;
                self.pick_grain_row(ui, st, grain_tab::QLINK_ROWS[k]);
                None
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
    /// detent, unsnapped. While REC is on, they turn Gain, the Threshold,
    /// the Pre-roll and Monitor's level instead
    /// ([`recorder::QLINK_ROWS`]).
    pub fn qlink(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        knob: usize,
        delta: i32,
        fine: bool,
    ) -> Option<WaveAction> {
        if self.rec.recording {
            return recorder::qlink_action(knob, delta, fine).map(WaveAction::Rec);
        }
        if self.tool == Tool::Slice {
            let cs = st.chop.clone()?;
            self.pick_slice_row(ui, st, knob);
            if delta == 0 {
                return None;
            }
            let px = self.px(ui);
            self.slice.qlink(&cs, knob, delta, fine, px);
            return self.slice_done(ui, st, None);
        }
        if self.tool == Tool::Warp {
            self.pick_warp_row(ui, st, knob);
            if delta == 0 {
                return None;
            }
            use warp_tool::Key as K;
            return match knob {
                0 => {
                    self.warp.pick(delta, st);
                    self.show_warp(ui, st);
                    None
                }
                1 => {
                    let r = self.warp.nudged(st, delta, fine)?;
                    self.edit(st, Key::Warp(K::Nudge), r)
                }
                2 => {
                    let r = WarpTool::tempo_turned(st, delta, fine)?;
                    self.edit(st, Key::Warp(K::Tempo), r)
                }
                3 => {
                    let width = self.wave_w;
                    if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
                        let v = w.view();
                        let start =
                            v.start + f64::from(delta) * v.frames_per_px * f64::from(width) / 16.0;
                        w.set_view(WaveView { start, ..v }, width);
                    }
                    self.sync_overview(ui);
                    self.show_warp_cards(ui, st);
                    None
                }
                _ => None,
            };
        }
        if self.tool == Tool::Grain {
            // Position, Size, Density, Pitch; with
            // SHIFT the spray, finer size and density, the pitch spray.
            let row = if fine {
                *grain_tab::SHIFT_ROWS.get(knob)?
            } else {
                *grain_tab::QLINK_ROWS.get(knob)?
            };
            self.pick_grain_row(ui, st, row);
            if delta == 0 {
                return None;
            }
            let finer = fine && matches!(row, grain_tab::Row::Size | grain_tab::Row::Density);
            let mut r = st.region.clone();
            r.grain = grain_tab::turned(&r.grain, row, delta as f32, finer);
            return self.edit(st, Key::Grain(row), r);
        }
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
        if self.live.is_some() {
            return;
        }
        let width = self.wave_w;
        let at = match self.tool {
            Tool::Slice => st.chop.as_deref().map(|cs| {
                let (a, b) = self.slice.picked(cs);
                a + (b - a) / 2
            }),
            Tool::Grain => Some(grain_tool::span(&st.region, st.rate, st.bpm, None).at),
            Tool::Warp => self
                .warp
                .selected()
                .and_then(|i| st.region.warp.as_ref()?.pins.get(i).copied())
                .map(|p| warp_tool::played(&self.bend, p.frame).round().max(0.0) as u64),
            _ => self.focus.frame(&st.region),
        };
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
        if self.tool == Tool::Warp {
            self.show_warp_cards(ui, st);
        }
    }

    /// Frames a pixel at the zoom shown.
    fn px(&self, ui: &Ui) -> f64 {
        self.view(ui).map_or(1.0, |v| v.frames_per_px)
    }

    /// Shows what the Slice tool's chop changed: the cuts and the slice
    /// picked on the wave, the section, the cards and the wheel's card.
    fn show_slice(&mut self, ui: &mut Ui, st: &WaveState) {
        self.show_wave(ui, st);
        if !self.rec.recording {
            self.show_list(ui, st);
            self.show_cards(ui, st);
        }
        self.update_wheel(ui, st);
    }

    /// After a Slice tool touch: shows it, and hands the app what it asks
    /// for, or else why it changed nothing.
    fn slice_done(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        a: Option<ChopAction>,
    ) -> Option<WaveAction> {
        self.show_slice(ui, st);
        let note = self.slice.take_note();
        match (a, note) {
            (Some(a), _) => Some(WaveAction::Chop(a)),
            (None, Some(why)) => Some(WaveAction::Say(why)),
            (None, None) => None,
        }
    }

    /// The Slice tool's row picked on the panel.
    fn slice_row(&self, ui: &Ui) -> Option<slice_tool::Row> {
        let i = ui.get::<SettingList>(REGION)?.selected;
        slice_tool::Row::ALL.get(i).copied()
    }

    /// Q-Link `knob`'s row picked on the Slice section (the method's
    /// setting, the slice, the slice for its start, the pads), and its
    /// card lit.
    fn pick_slice_row(&self, ui: &mut Ui, st: &WaveState, knob: usize) {
        let Some(row) = SLICE_CARD_ROWS.get(knob).copied().flatten() else {
            return;
        };
        if let Some(list) = ui.get_mut::<SettingList>(REGION) {
            list.selected = row.index();
        }
        self.show_cards(ui, st);
    }

    /// Shows what the Warp tool's own settings change: the pins (the one
    /// picked), the hits, the section, the cards and the wheel's card.
    fn show_warp(&mut self, ui: &mut Ui, st: &WaveState) {
        self.show_wave(ui, st);
        if !self.rec.recording {
            self.show_list(ui, st);
            self.show_cards(ui, st);
        }
        self.update_wheel(ui, st);
    }

    /// A Warp tool edit: the region changed (one undo step per `key`), or
    /// only the tool's own settings, which show at once.
    fn warp_edit(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        key: warp_tool::Key,
        after: Option<Region>,
    ) -> Option<WaveAction> {
        let out = after.and_then(|r| self.edit(st, Key::Warp(key), r));
        if out.is_none() {
            self.show_warp(ui, st);
        }
        out
    }

    /// Q-Link `knob`'s row picked on the Warp section (Pin, Pin beat,
    /// Warp tempo; Scroll has none), and its card lit.
    fn pick_warp_row(&self, ui: &mut Ui, st: &WaveState, knob: usize) {
        let rows = [
            warp_tool::Row::Pin,
            warp_tool::Row::PinBeat,
            warp_tool::Row::WarpTempo,
        ];
        if let Some(row) = rows.get(knob) {
            if let Some(list) = ui.get_mut::<SettingList>(REGION) {
                list.selected = row.index();
            }
        }
        self.show_warp_cards(ui, st);
    }

    /// A pin's tab touched or dragged with the Warp tool (the other tools'
    /// marks have no tab to touch): the pin is picked, and moves to the
    /// grid line nearest the beat under the finger.
    fn warp_marker(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        index: usize,
        frame: u64,
    ) -> Option<WaveAction> {
        let r = &st.region;
        let i = index.checked_sub(markers(r).len())?;
        let w = r.warp.as_ref()?;
        if w.method != mpfree_sampler::model::WarpMethod::Stretch {
            return Some(WaveAction::Say(warp_tool::REPITCH_NOTE.to_string()));
        }
        let p = *w.pins.get(i)?;
        let now = warp_tool::played(&self.bend, p.frame).round().max(0.0) as u64;
        if frame == now {
            // Touched, not moved: picked, where it is (on the grid or not).
            if self.warp.selected() != Some(i) {
                self.warp.select(Some(i));
                self.show_warp(ui, st);
            }
            return None;
        }
        let after = self
            .warp
            .dragged(st, i, warp_tool::beat_drawn(st, frame as f64));
        self.warp_edit(ui, st, warp_tool::Key::Drag(i), after)
    }

    /// A grain window's tab touched or dragged with the Grain tool (the
    /// other tools' marks have no tab to touch).
    fn grain_marker(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        index: usize,
        frame: u64,
    ) -> Option<WaveAction> {
        let r = &st.region;
        let marks = markers(r);
        let (row, after) = match index.checked_sub(marks.len()) {
            Some(0) => (grain_tab::Row::Position, grain_tool::moved_start(r, frame)),
            Some(1) => (
                grain_tab::Row::Size,
                grain_tool::moved_end(r, frame, st.rate, st.bpm),
            ),
            _ => return None,
        };
        // The tab follows the finger; put it where the window really is.
        let s = grain_tool::span(&after, st.rate, st.bpm, None);
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            let to = if row == grain_tab::Row::Position {
                s.at
            } else {
                s.at + s.len
            };
            w.set_marker_frame(index, to);
        }
        self.pick_grain_row(ui, st, row);
        self.edit(st, Key::Grain(row), after)
    }

    /// The Grain section's row `index` stepped `delta` steps: the parts'
    /// switch, a preset's button, a switch flipped, or a row stepped as
    /// its row steps ([`grain_tab::stepped`]).
    fn grain_step(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        index: usize,
        delta: i32,
    ) -> Option<WaveAction> {
        use grain_tab::Row as G;
        if index == grain_tool::PARTS_ROW {
            let n = GrainPart::ALL.len() as i32;
            let to = (self.grain_part.index() as i32 + delta).rem_euclid(n) as usize;
            self.grain_part = GrainPart::ALL[to];
            self.gesture = None;
            self.show_list(ui, st);
            self.show_grain_cards(ui, st);
            return None;
        }
        let row = *self.grain_part.rows(&st.region).get(index - 1)?;
        let r = &st.region;
        let after = match row {
            G::Preset | G::PresetMore => {
                let first = if row == G::Preset {
                    0
                } else {
                    grain_tab::PRESETS_A_LINE
                };
                let i = first as i32 + grain_tab::preset_on(r, row) as i32 + delta;
                let n = mpfree_sampler::grain_presets::presets().len() as i32;
                if !(0..n).contains(&i) {
                    return None;
                }
                mpfree_sampler::grain_presets::apply(r, i as usize)
            }
            _ if grain_tab::is_switch(row) => {
                let d = if grain_tab::switch_on(r, row) { -1 } else { 1 };
                grain_tab::stepped(r, row, d, st.bpm)
            }
            _ => grain_tab::stepped(r, row, delta, st.bpm),
        };
        let key = if matches!(row, G::PresetMore) {
            G::Preset
        } else {
            row
        };
        self.edit(st, Key::Grain(key), after)
    }

    /// Puts the Grain section's highlight on `row` if the part open shows
    /// it, and lights its Q-Link card.
    fn pick_grain_row(&self, ui: &mut Ui, st: &WaveState, row: grain_tab::Row) {
        if let Some(i) = self
            .grain_part
            .rows(&st.region)
            .iter()
            .position(|r| *r == row)
        {
            if let Some(list) = ui.get_mut::<SettingList>(REGION) {
                list.selected = i + 1;
            }
        }
        self.show_grain_cards(ui, st);
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
                let Some(x) = &st.source.mono else {
                    return to;
                };
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
            region: Box::new(r),
            gesture,
        })
    }

    /// A setting stepped `delta` steps: an arrow or - / + (one), a knob
    /// dragged (as many as it passed), or a choice's button tapped (from
    /// the option on to that one). A choice wraps round at its ends.
    fn step(&mut self, ui: &mut Ui, st: &WaveState, row: Row, delta: i32) -> Option<WaveAction> {
        let mut r = st.region.clone();
        let len = r.frames();
        let d = delta;
        if d == 0 {
            return None;
        }
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
                    next = (next as i32 + d.signum()).rem_euclid(3) as usize;
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
            // A button: pressed, not stepped.
            Row::Normalize => return None,
            // An arrow or the switch: each step turns it over.
            Row::Reverse => r.reverse ^= d % 2 != 0,
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
                self.show_list(ui, st);
                return None;
            }
        }
        self.edit(st, Key::Row(row), r)
    }
}

/// The panel: the settings in three sections across the band under the
/// wave, left of the Sound section (the one Sample page, 18 Nov; the
/// widths of the mockup Jay approved on 17 Nov, the Warp section gone to
/// the Warp page and Options shared out).
fn panel() -> Panel {
    let it = |k: Row| Item::new(k.index());
    Panel {
        sections: vec![
            Section::new(
                "Mode",
                340,
                vec![
                    vec![it(Row::Mode)],
                    vec![it(Row::Loop).labelled(), it(Row::Crossfade).wide(78)],
                ],
            )
            .note("how a hit plays"),
            Section::new(
                "Gain and fades",
                234,
                vec![
                    vec![it(Row::Gain), it(Row::FadeIn), it(Row::FadeOut)],
                    vec![it(Row::Normalize)],
                ],
            ),
            Section::new(
                "Options",
                174,
                vec![
                    vec![it(Row::Reverse)],
                    vec![it(Row::Snap).labelled()],
                    vec![it(Row::Channels).labelled()],
                ],
            )
            .from_top(),
        ],
    }
}

/// Where `now` (frames) sits in `list` (ms), 0 to 1: its nearest entry's
/// place, as a knob shows it.
fn ms_place(list: &[f64], now: u64, rate: u32) -> f32 {
    let ms = now as f64 * 1000.0 / f64::from(rate.max(1));
    let i = list
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - ms).abs().total_cmp(&(b.1 - ms).abs()))
        .map_or(0, |(i, _)| i);
    i as f32 / (list.len().max(2) - 1) as f32
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

/// The source id as the bar shows it: first and last two bytes.
fn short_id(id: SourceId) -> String {
    let s = id.to_string();
    format!("{}…{}", &s[..4], &s[s.len() - 2..])
}

/// The markers in the waveform's order: Start and End, then the loop's.
/// The Q-Link (0 to 3) that moves the waveform's marker `index`, as the
/// widget numbers its markers: a finger held on that marker's tab turns it
/// as that Q-Link does (Jay, 17 Nov: hold a control and turn the wheel).
pub fn marker_knob(st: &WaveState, index: usize) -> Option<usize> {
    let m = *markers_order(&st.region).get(index)?;
    Mark::ALL.iter().position(|k| *k == m)
}

fn markers_order(r: &Region) -> Vec<Mark> {
    Mark::ALL
        .iter()
        .copied()
        .filter(|m| m.frame(r).is_some())
        .collect()
}

/// The region's markers ([`markers_order`]), then a sliced region's cuts
/// (R4), each at the start of its slice, numbered from 2: the Slice
/// tool's, so faint lines with no tab (they move with the Slice tool).
fn markers(r: &Region) -> Vec<Marker> {
    let mut all: Vec<Marker> = markers_order(r)
        .into_iter()
        .filter_map(|m| {
            let kind = if m.is_loop() {
                MarkerKind::Loop
            } else {
                MarkerKind::Edge
            };
            m.frame(r).map(|f| Marker::new(f, m.tab(), kind))
        })
        .collect();
    all.extend(r.cuts().enumerate().map(|(k, f)| {
        Marker::new(f, &(k + 2).to_string(), MarkerKind::Slice)
            .faint()
            .line_only()
    }));
    all
}

/// A pad's colour on the screen, by its place in the bank (the pads' own
/// colours are set elsewhere; this only tells pads apart).
pub fn pad_color(pad: usize) -> Color {
    crate::widgets::PAD_COLORS[pad % crate::widgets::PAD_COLORS.len()]
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
