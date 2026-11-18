//! The pad editor's Grain tab: how a layer plays as a grain cloud.
//!
//! - The **cloud** shows the layer's region with the grains playing now
//!   over it: a band where grains may start (the position, spread by the
//!   spray as it plays now, its middle stronger when peaked), a line at
//!   the position, the voice's reading place with its path over the last
//!   second and an arrow the way the scan goes, a bar for each grain, and
//!   a "thinned" tag while the cloud wants more grains than a voice keeps
//!   ([`CloudNow`]). A tap or drag on it moves the position.
//! - The **section chips** over the cloud, as the Play tab's, pick what the
//!   panel holds ([`View`]): **Grains**, **Movement**, **Filter and keys**
//!   and **Presets**, each with a line of what is in it now.
//! - The **touch pad** moves the position (across) and the grain size (up)
//!   together, on a log scale from 2 ms to 2 s. It stays beside the panel
//!   on every section.
//! - The **panel** beside the touch pad ([`crate::panel`]):
//!   - **Grains** has two sections: **Cloud** (how grains scatter: scan
//!     speed, spray, size, pitch and level spray, timing, reverse chance,
//!     stereo spread and the cloud's level as knobs, and the spray's shape)
//!     and **Shape** (the play mode, only Granular playing grains; the
//!     grain mode; the grains' Shape knob; pitch quantise; tempo sync). A
//!     grain mode other than Cloud shows only its own rows, and names the
//!     first section after itself.
//!   - **Movement**: an **LFO** and an **Envelope**, each moving one
//!     setting (position, size, density, pitch, spray or the filter's
//!     cutoff) of the grains as they are made, by up to its depth either
//!     way. The LFO runs free in Hz or once every so many beats of the
//!     project tempo; the envelope rises over its attack and falls over its
//!     decay to its sustain. Both start again with each hit.
//!   - **Filter and keys**: a **Filter** on each grain (off, low, band or
//!     high pass; cutoff, resonance, and a spray that gives each grain its
//!     own cutoff), and **Keys and quality**: played from keys (or tuned)
//!     the grains move in pitch (a Tone's note); **Size follows key**
//!     shortens them as much and **Scan follows key** speeds the scan as
//!     much, as a sampler's whole sound would move. **Glide** slides to
//!     each new key: on a mono pad the cloud playing moves there, and with
//!     more voices each new one slides from the last key. **Quality**
//!     colours the cloud on its way out: Clean, Tape (wow, flutter, a soft
//!     saturation) or Lo-fi (11 kHz, 8 bits).
//!   - **Presets**: the eight of [`mpfree_sampler::grain_presets`] as
//!     buttons, the one the layer matches lit. A tap puts its mode,
//!     settings, keys, glide and movement on the layer (the sound,
//!     position, level and quality stay).
//!
//!   A grain mode shows only the rows it uses ([`Row::shown`]). Position,
//!   size, density and pitch are the Q-Links' (and the touch pad's), on
//!   every section. A touch on a control or a Q-Link card picks it; the
//!   **wheel** and - / + step the one picked (Scan, until another is), and
//!   a finger resting on a control lets the wheel turn it.
//! - **Tempo sync** locks the density, the size or both to the project
//!   tempo: Density then steps in grains a beat and Size in beats, and the
//!   engine starts each grain on the beat grid at the tempo playing now
//!   (`Engine::frames_per_beat`).
//! - The layer shown is the one picked on the Play tab's layer map, so
//!   every layer of a pad can be its own cloud.
//! - **Q-Links 1 to 4** turn Position, Size, Density and Pitch. With SHIFT,
//!   Q-Link 1 turns the spray and Q-Link 4 the pitch spray (each the spray
//!   of its knob), and Q-Links 2 and 3 turn finer.
//!
//! Every change is a [`GrainAction::Region`] for the app to apply
//! ([`GrainAction::apply`]); a drag, a turn or a run of taps on one row is
//! one undo step, until the app calls [`GrainTab::lift`]. The grains the
//! engine plays come in through [`GrainTab::set_cloud`]. The region's
//! other settings are the Wave tab's; pressure, which can drive density,
//! size, position or spray, is on the Play tab.

use std::sync::Arc;

use mpfree_analysis::pitch::note_name;
use mpfree_sampler::engine::{CloudState, GrainView, MAX_GRAINS};
use mpfree_sampler::grain_presets;
use mpfree_sampler::model::{
    GrainFilter, GrainFilterKind, GrainMode, GrainParams, GrainQuality, GrainSprayShape,
    GrainWindow, LfoShape, ModelError, MotionTarget, Pad, PlayMode, Project, Region, Source,
    TempoSync, GRAIN_CUTOFF_MAX_HZ, GRAIN_CUTOFF_MIN_HZ, GRAIN_FILTER_SPRAY_OCTAVES,
};
use mpfree_ui::{Color, EventKind, Ui, UiEvent};

use crate::chop_tab::pad_text;
use crate::grain_widgets::{GrainCloud, XyPad, GRAIN_COLOR};
use crate::panel::{Control, Item, Look, Panel, Section};
use crate::play_tab::LayerSound;
use crate::source::SourceView;
use crate::wave_tab::{self, duration_text, mode_label, pad_color, TABS};
use crate::widgets::{KnobCard, PadBar, SectionChip, SectionChips, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-grain-bar";
pub const CLOUD: &str = "smp-grain-cloud";
pub const XY: &str = "smp-grain-xy";
pub const LIST: &str = "smp-grain-list";
pub const SECTIONS: &str = "smp-grain-sections";
pub const QLINKS: [&str; 4] = [
    "smp-grain-q1",
    "smp-grain-q2",
    "smp-grain-q3",
    "smp-grain-q4",
];

/// The tab's page: the section chips, the cloud across the page, and under
/// it the touch pad beside the panel, between the pad bar and the Q-Link
/// cards.
pub const LAYOUT: &str = "col(header:60, row(smp-grain-bar:*, smp-send:330):60, \
                          row(col(smp-grain-sections:56, smp-grain-cloud:*, \
                          row(smp-grain-xy:236, smp-grain-list:*):260):*, \
                          col(smp-grain-q1, smp-grain-q2, smp-grain-q3, smp-grain-q4):128):*, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = 4;

/// The hint line the app shows under the tab.
pub const HINT: &str = "Hit the pad to hear the cloud · tap a section for its controls · the touch pad: position \
                        and size · touch a control, then turn the wheel · SHIFT + Q-Link 1 / 4: the sprays";

/// Grain sizes the arrows step through, in ms.
const SIZES_MS: [f32; 19] = [
    2.0, 5.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 80.0, 100.0, 120.0, 150.0, 200.0, 300.0, 500.0,
    750.0, 1000.0, 1500.0, 2000.0,
];
pub const SIZE_RANGE_MS: (f32, f32) = (2.0, 2000.0);
/// Densities the arrows step through, grains a second.
const DENSITIES: [f32; 18] = [
    1.0, 2.0, 3.0, 5.0, 8.0, 10.0, 15.0, 20.0, 25.0, 30.0, 40.0, 50.0, 60.0, 80.0, 100.0, 120.0,
    150.0, 200.0,
];
pub const DENSITY_RANGE: (f32, f32) = (1.0, 200.0);
pub const PITCH_LIMIT: f32 = 36.0;
pub const PITCH_SPRAY_LIMIT: f32 = 12.0;
/// Scan speeds the arrows step through (1 is the sound's own speed).
const SCANS: [f32; 13] = [
    -2.0, -1.0, -0.5, -0.25, -0.125, 0.0, 0.125, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0,
];
pub const SCAN_LIMIT: f32 = 2.0;
/// Q-Link detents for a doubling of the size or the density.
const DOUBLING_DETENTS: f32 = 8.0;
/// Pitch quantise: the notes grains may take, from the sound's own pitch
/// (bit 0) up (see `mpfree_grain::quantise`).
const SCALES: [(&str, u16); 7] = [
    ("Off", 0),
    ("Major", 0b1010_1011_0101),
    ("Minor", 0b0101_1010_1101),
    ("Pentatonic", 0b0010_1001_0101),
    ("Fifths", 0b0000_1000_0001),
    ("Octaves", 0b1),
    ("Chromatic", 0b1111_1111_1111),
];
/// The play modes the Mode row steps through, as the Wave tab's Mode row.
const MODES: [PlayMode; 5] = [
    PlayMode::OneShot,
    PlayMode::Gate,
    PlayMode::Toggle,
    PlayMode::Loop,
    PlayMode::Granular,
];
/// What Tempo sync locks, in the order its row steps: (label, density,
/// size).
const SYNCS: [(&str, bool, bool); 4] = [
    ("Off", false, false),
    ("Density", true, false),
    ("Size", false, true),
    ("Both", true, true),
];
/// Grains a beat the synced Density row steps through (a quarter is one a
/// bar of 4/4).
const PER_BEAT: [f32; 12] = [
    0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0,
];
/// Grain lengths, in beats, the synced Size row steps through.
const BEATS: [f32; 9] = [
    1.0 / 64.0,
    1.0 / 32.0,
    1.0 / 16.0,
    1.0 / 8.0,
    1.0 / 4.0,
    1.0 / 2.0,
    1.0,
    2.0,
    4.0,
];
/// What a cloud synced on this tab saves as its frames per beat (120 BPM
/// at 44.1 kHz). The engine plays it at the project tempo whatever this
/// says.
pub const SAVED_FRAMES_PER_BEAT: f64 = 22_050.0;
/// The cloud's level, in dB: the Level row's range.
pub const LEVEL_RANGE_DB: (f32, f32) = (-24.0, 12.0);
/// Gesture ids the tab hands out start here, clear of other screens'.
const GESTURE_BASE: u64 = 0x4752_4149_0000_0000;

/// What the engine reports of the layer's cloud beside its grains, for
/// [`GrainTab::set_cloud`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CloudNow {
    /// Where its first voice reads, 0 to 1 across the region, while one
    /// plays.
    pub playhead: Option<f32>,
    /// How it plays now ([`mpfree_sampler::engine::Engine::cloud_of`]):
    /// the spray as moved, and whether it is thinned.
    pub state: Option<CloudState>,
    /// Grains playing on every pad.
    pub live: usize,
}

/// What the panel holds: the section chip open over the cloud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// How grains scatter and their shape (the cloud's own settings).
    #[default]
    Grains,
    /// The LFO and the envelope.
    Movement,
    /// The filter on each grain, key tracking and glide.
    FilterKeys,
    /// The eight starting points.
    Presets,
}

impl View {
    /// The chips, left to right.
    pub const ALL: [View; 4] = [
        View::Grains,
        View::Movement,
        View::FilterKeys,
        View::Presets,
    ];

    pub fn label(self) -> &'static str {
        match self {
            View::Grains => "Grains",
            View::Movement => "Movement",
            View::FilterKeys => "Filter and keys",
            View::Presets => "Presets",
        }
    }
}

/// The Grain list's rows: each view's panel's, in its order (which the
/// wheel walks), then the Q-Links'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Mode,
    GrainMode,
    /// The first four presets, as buttons ([`View::Presets`]).
    Preset,
    /// The other four.
    PresetMore,
    Scan,
    Spray,
    SprayShape,
    SizeSpray,
    PitchSpray,
    LevelSpray,
    Timing,
    Shape,
    Reverse,
    Spread,
    Quantise,
    Sync,
    Position,
    Size,
    Density,
    Pitch,
    Level,
    Filter,
    Cutoff,
    Resonance,
    FilterSpray,
    KeySize,
    KeyScan,
    Glide,
    Quality,
    LfoTo,
    LfoShape,
    LfoRate,
    LfoSync,
    LfoDepth,
    EnvTo,
    EnvDepth,
    EnvAttack,
    EnvDecay,
    EnvSustain,
}

/// The movement rows, in the Movement panel's order.
pub const MOTION_ROWS: [Row; 10] = [
    Row::LfoTo,
    Row::LfoShape,
    Row::LfoRate,
    Row::LfoDepth,
    Row::LfoSync,
    Row::EnvTo,
    Row::EnvDepth,
    Row::EnvAttack,
    Row::EnvDecay,
    Row::EnvSustain,
];

impl Row {
    /// The Grains view's rows for a cloud, in the panel's order (which the
    /// wheel walks): Cloud's, then Shape's, then the Q-Links'. A grain
    /// mode shows some of them ([`Row::shown`]), in this order.
    pub const GRAINS: [Row; 19] = [
        Row::Scan,
        Row::Spray,
        Row::SizeSpray,
        Row::PitchSpray,
        Row::LevelSpray,
        Row::Timing,
        Row::Reverse,
        Row::Spread,
        Row::Level,
        Row::SprayShape,
        Row::Mode,
        Row::GrainMode,
        Row::Shape,
        Row::Quantise,
        Row::Sync,
        Row::Position,
        Row::Size,
        Row::Density,
        Row::Pitch,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Mode => "Mode",
            Row::GrainMode => "Grain mode",
            Row::Scan => "Scan speed",
            Row::Spray => "Spray",
            Row::SprayShape => "Spray shape",
            Row::SizeSpray => "Size spray",
            Row::PitchSpray => "Pitch spray",
            Row::LevelSpray => "Level spray",
            Row::Timing => "Timing",
            Row::Shape => "Shape",
            Row::Reverse => "Reverse chance",
            Row::Spread => "Stereo spread",
            Row::Quantise => "Pitch quantise",
            Row::Sync => "Tempo sync",
            Row::Position => "Position",
            Row::Size => "Size",
            Row::Density => "Density",
            Row::Pitch => "Pitch",
            Row::Level => "Level",
            Row::Preset | Row::PresetMore => "Preset",
            Row::Filter => "Filter",
            Row::Cutoff => "Cutoff",
            Row::Resonance => "Resonance",
            Row::FilterSpray => "Filter spray",
            Row::KeySize => "Size follows key",
            Row::KeyScan => "Scan follows key",
            Row::Glide => "Glide",
            Row::Quality => "Quality",
            Row::LfoTo => "LFO to",
            Row::LfoShape => "LFO wave",
            Row::LfoRate => "LFO rate",
            Row::LfoSync => "LFO sync",
            Row::LfoDepth => "LFO depth",
            Row::EnvTo => "Envelope to",
            Row::EnvDepth => "Env depth",
            Row::EnvAttack => "Env attack",
            Row::EnvDecay => "Env decay",
            Row::EnvSustain => "Env sustain",
        }
    }

    /// Its name over its knob on the panel (and in the list's rows): the
    /// label, shorter where a knob is too narrow for it, or where its
    /// section already says whose it is.
    pub fn short(self) -> &'static str {
        match self {
            Row::Scan => "Scan",
            Row::Reverse => "Reverse",
            Row::Spread => "Stereo",
            Row::LfoShape => "Wave",
            Row::LfoRate => "Rate",
            Row::LfoSync => "Sync to tempo",
            Row::EnvAttack => "Attack",
            Row::EnvDecay => "Decay",
            Row::EnvSustain => "Sustain",
            other => other.label(),
        }
    }

    /// The section chip whose panel holds this row (the Q-Links' rows are
    /// on every one; this says Grains for them).
    pub fn view(self) -> View {
        match self {
            Row::Preset | Row::PresetMore => View::Presets,
            Row::Filter
            | Row::Cutoff
            | Row::Resonance
            | Row::FilterSpray
            | Row::KeySize
            | Row::KeyScan
            | Row::Glide
            | Row::Quality => View::FilterKeys,
            r if MOTION_ROWS.contains(&r) => View::Movement,
            _ => View::Grains,
        }
    }

    /// The rows `view` shows for a grain mode, in its panel's order, then
    /// the Q-Links'. On Grains: every row for a cloud; for a stretch, what
    /// moves it (no randomness, so its grains stay in step); for a repeat,
    /// its tempo-locked rate and length. On Filter and keys, a stretch has
    /// no filter spray (nothing random), a repeat no key tracking (its
    /// grains are its beat) and a tone no size following (its size follows
    /// its note already).
    pub fn shown(view: View, mode: GrainMode) -> &'static [Row] {
        match (view, mode) {
            (View::Grains, GrainMode::Cloud) => &Row::GRAINS,
            (View::Grains, GrainMode::Stretch) => &STRETCH_ROWS,
            (View::Grains, GrainMode::Repeat) => &REPEAT_ROWS,
            (View::Grains, GrainMode::Tone) => &TONE_ROWS,
            (View::Movement, _) => &MOVEMENT_ROWS,
            (View::FilterKeys, GrainMode::Cloud) => &FILTER_ROWS,
            (View::FilterKeys, GrainMode::Stretch) => &STRETCH_FILTER_ROWS,
            (View::FilterKeys, GrainMode::Repeat) => &REPEAT_FILTER_ROWS,
            (View::FilterKeys, GrainMode::Tone) => &TONE_FILTER_ROWS,
            (View::Presets, _) => &PRESET_ROWS,
        }
    }

    /// Where this row is in `view`'s list for `mode`, if it shows there.
    pub fn index_in(self, view: View, mode: GrainMode) -> Option<usize> {
        Row::shown(view, mode).iter().position(|r| *r == self)
    }

    /// Whether `mode` shows this row on any section.
    pub fn used_in(self, mode: GrainMode) -> bool {
        View::ALL.iter().any(|v| self.index_in(*v, mode).is_some())
    }
}

/// The Movement view's rows: the LFO's, the envelope's, the Q-Links'.
const MOVEMENT_ROWS: [Row; 14] = [
    Row::LfoTo,
    Row::LfoShape,
    Row::LfoRate,
    Row::LfoDepth,
    Row::LfoSync,
    Row::EnvTo,
    Row::EnvDepth,
    Row::EnvAttack,
    Row::EnvDecay,
    Row::EnvSustain,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// Filter and keys for a cloud: the filter's, the keys' and the
/// quality, the Q-Links'.
const FILTER_ROWS: [Row; 12] = [
    Row::Filter,
    Row::Cutoff,
    Row::Resonance,
    Row::FilterSpray,
    Row::KeySize,
    Row::KeyScan,
    Row::Glide,
    Row::Quality,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// A stretch's: one cutoff for every grain.
const STRETCH_FILTER_ROWS: [Row; 11] = [
    Row::Filter,
    Row::Cutoff,
    Row::Resonance,
    Row::KeySize,
    Row::KeyScan,
    Row::Glide,
    Row::Quality,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// A repeat's: its grains are its beat, so keys move only its pitch.
const REPEAT_FILTER_ROWS: [Row; 10] = [
    Row::Filter,
    Row::Cutoff,
    Row::Resonance,
    Row::FilterSpray,
    Row::Glide,
    Row::Quality,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// A tone's: its size follows its note already.
const TONE_FILTER_ROWS: [Row; 11] = [
    Row::Filter,
    Row::Cutoff,
    Row::Resonance,
    Row::FilterSpray,
    Row::KeyScan,
    Row::Glide,
    Row::Quality,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// The Presets view's rows: two lines of four, the Q-Links'.
const PRESET_ROWS: [Row; 6] = [
    Row::Preset,
    Row::PresetMore,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// Presets a line of the Presets view holds.
const PRESETS_A_LINE: usize = 4;

/// The order [`describe`] says the changes in: the modes and the preset
/// first, the level last (the log's, which the panels' order doesn't
/// change), then the filter, keys and movement.
const LOG_ORDER: [Row; 38] = [
    Row::Mode,
    Row::GrainMode,
    Row::Preset,
    Row::Scan,
    Row::Spray,
    Row::SprayShape,
    Row::SizeSpray,
    Row::PitchSpray,
    Row::LevelSpray,
    Row::Timing,
    Row::Shape,
    Row::Reverse,
    Row::Spread,
    Row::Quantise,
    Row::Sync,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
    Row::Level,
    Row::Filter,
    Row::Cutoff,
    Row::Resonance,
    Row::FilterSpray,
    Row::KeySize,
    Row::KeyScan,
    Row::Glide,
    Row::Quality,
    Row::LfoTo,
    Row::LfoShape,
    Row::LfoRate,
    Row::LfoSync,
    Row::LfoDepth,
    Row::EnvTo,
    Row::EnvDepth,
    Row::EnvAttack,
    Row::EnvDecay,
    Row::EnvSustain,
];

/// A stretch's rows: how fast it moves through the sound (Scan speed),
/// where, its grains' size, overlap and shape, its pitch and its width.
const STRETCH_ROWS: [Row; 10] = [
    Row::Scan,
    Row::Spread,
    Row::Level,
    Row::Mode,
    Row::GrainMode,
    Row::Shape,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// A repeat's rows: where, how often (a beat division) and how long, the
/// edges, pitch, a chance of playing backwards and a little spray.
const REPEAT_ROWS: [Row; 10] = [
    Row::Spray,
    Row::Reverse,
    Row::Level,
    Row::Mode,
    Row::GrainMode,
    Row::Shape,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// A tone's rows: its note (the density), how many cycles a grain holds
/// (the size), where in the sound and at what pitch it reads (its
/// colour), and what moves or widens it.
const TONE_ROWS: [Row; 12] = [
    Row::Scan,
    Row::Spray,
    Row::PitchSpray,
    Row::Spread,
    Row::Level,
    Row::Mode,
    Row::GrainMode,
    Row::Shape,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
];
/// The Grain mode row's words, in the order it steps.
const GRAIN_MODES: [(&str, GrainMode); 4] = [
    ("Cloud", GrainMode::Cloud),
    ("Stretch", GrainMode::Stretch),
    ("Repeat", GrainMode::Repeat),
    ("Tone", GrainMode::Tone),
];
/// What the LFO and the envelope can move, as the rows step (a tone's
/// density is its note).
const MOTION_TARGETS: [MotionTarget; 7] = [
    MotionTarget::Off,
    MotionTarget::Position,
    MotionTarget::Size,
    MotionTarget::Density,
    MotionTarget::Pitch,
    MotionTarget::Spray,
    MotionTarget::Cutoff,
];
/// The grain filter's passes, as the Filter row steps.
const FILTERS: [(&str, GrainFilterKind); 4] = [
    ("Off", GrainFilterKind::Off),
    ("Low pass", GrainFilterKind::Low),
    ("Band pass", GrainFilterKind::Band),
    ("High pass", GrainFilterKind::High),
];
/// Octaves the Cutoff row moves a step.
const CUTOFF_STEP_OCT: f32 = 1.0 / 6.0;
/// The LFO's waves, as the LFO shape row steps.
const LFO_SHAPES: [(&str, LfoShape); 7] = [
    ("Sine", LfoShape::Sine),
    ("Triangle", LfoShape::Triangle),
    ("Saw up", LfoShape::SawUp),
    ("Saw down", LfoShape::SawDown),
    ("Square", LfoShape::Square),
    ("Hold", LfoShape::Hold),
    ("Drift", LfoShape::Drift),
];
/// Free LFO rates the row steps through, Hz.
const LFO_RATES: [f32; 20] = [
    0.05, 0.1, 0.2, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0,
    20.0, 30.0, 40.0,
];
/// A synced LFO's cycle, in beats, as the row steps.
const LFO_BEATS: [f32; 10] = [0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 8.0, 16.0, 32.0, 64.0];
/// Glide times the row steps through, ms (0 is off).
const GLIDES_MS: [f32; 13] = [
    0.0, 10.0, 20.0, 50.0, 100.0, 150.0, 200.0, 300.0, 500.0, 750.0, 1000.0, 2000.0, 5000.0,
];
/// The envelope's attack and decay times the rows step through, ms.
const ENV_TIMES_MS: [f32; 20] = [
    0.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 300.0, 500.0, 750.0, 1000.0, 1500.0, 2000.0, 3000.0,
    5000.0, 7500.0, 10_000.0, 15_000.0, 20_000.0, 30_000.0,
];

/// A tone's notes: A0 to B5 (the grain voice plays up to 1,000 grains a
/// second), as MIDI notes.
const TONE_NOTES: (f32, f32) = (21.0, 83.0);
/// How many cycles of its note a tone's grain may hold.
const TONE_CYCLES: (f32, f32) = (1.0, 16.0);

/// The rows the Q-Links turn, in knob order.
pub const QLINK_ROWS: [Row; 4] = [Row::Position, Row::Size, Row::Density, Row::Pitch];
/// What the Q-Links turn with SHIFT held: Q-Links 1 and 4 their spray,
/// Q-Links 2 and 3 their own row, finer.
pub const SHIFT_ROWS: [Row; 4] = [Row::Spray, Row::Size, Row::Density, Row::PitchSpray];

/// Everything the tab shows: one layer of one pad, its region and its
/// sound.
#[derive(Clone)]
pub struct GrainState {
    pub pad: usize,
    pub layer: usize,
    pub settings: Pad,
    /// The layer's region: its grain settings and play mode.
    pub region: Region,
    pub sound: LayerSound,
    /// The project tempo, for what a synced size or density comes to (120
    /// until the app says).
    pub bpm: f64,
}

impl GrainState {
    /// Layer `layer` of pad `pad`, if it has a sound. `view` gives what the
    /// screens know of a source (the app keeps them, so each is worked out
    /// once).
    pub fn from_project(
        p: &Project,
        pad: usize,
        layer: usize,
        mut view: impl FnMut(&Source) -> Arc<SourceView>,
    ) -> Option<GrainState> {
        let settings = p.pad(pad)?.clone();
        let region = settings.layers.get(layer)?.region.clone();
        let s = p.source(region.source)?;
        let sound = LayerSound {
            name: s.name.clone(),
            rate: s.rate(),
            view: view(s),
        };
        Some(GrainState {
            pad,
            layer,
            settings,
            region,
            sound,
            bpm: 120.0,
        })
    }

    pub fn granular(&self) -> bool {
        self.region.mode == PlayMode::Granular
    }

    /// The grain size heard, in ms: the size row's, or its beats at the
    /// tempo when synced.
    pub fn size_ms(&self) -> f32 {
        size_ms_at(&self.region.grain, self.bpm)
    }
}

/// What a touch or control on the tab asks for. The app applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum GrainAction {
    /// Layer `layer`'s region with new grain settings (or play mode).
    /// `gesture` is the same for every change of one drag, turn or run of
    /// taps on a row, so they merge into one undo step.
    Region {
        pad: usize,
        layer: usize,
        region: Box<Region>,
        gesture: u64,
    },
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
}

impl GrainAction {
    /// Applies a change to the project (one undo step per gesture). Other
    /// actions change nothing here.
    pub fn apply(&self, p: &mut Project) -> Result<(), ModelError> {
        let GrainAction::Region {
            pad,
            layer,
            region,
            gesture,
        } = self
        else {
            return Ok(());
        };
        let (layer, region) = (*layer, (**region).clone());
        if p.pad(*pad).is_none_or(|pd| layer >= pd.layers.len()) {
            return Err(ModelError::BadPad);
        }
        p.edit_pad(*pad, Some(*gesture), move |pd| {
            pd.layers[layer].region = region
        })
    }
}

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Row(Row),
    /// The touch pad, or a drag on the cloud.
    Touch,
}

/// The Grain tab's widgets and what their touches mean. It keeps the undo
/// step being made; the settings come from the [`GrainState`].
pub struct GrainTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`wave_tab::WaveTab::bank`].
    pub bank: Option<char>,
    shown: Option<(usize, usize)>,
    /// The section chip open: what the panel holds.
    view: View,
    /// The grain mode whose rows the list shows.
    mode: GrainMode,
    gesture: Option<(Key, u64)>,
    next_gesture: u64,
}

impl GrainTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &GrainState) -> Result<GrainTab, String> {
        crate::send::add(ui);
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        let labels: Vec<&str> = View::ALL.iter().map(|v| v.label()).collect();
        ui.add(SECTIONS, SectionChips::new(&labels));
        let mut cloud = GrainCloud::new("Grains over the sound");
        cloud.right = "drag for the position".into();
        ui.add(CLOUD, cloud);
        ui.add(XY, XyPad::new("Touch", "position", "size"));
        let mut list = SettingList::new("Grain");
        list.right = "this layer".into();
        list.selected = Row::Scan
            .index_in(View::Grains, GrainMode::Cloud)
            .unwrap_or(0);
        ui.add(LIST, list);
        let t = ui.theme.clone();
        let colors = [Color::rgb(GRAIN_COLOR), t.accent, t.good, t.warn];
        for (k, row) in QLINK_ROWS.iter().enumerate() {
            ui.add(
                QLINKS[k],
                KnobCard::new(&format!("Q-Link {} · {}", k + 1, row.label()), colors[k])
                    .on(k, "Grain"),
            );
        }
        ui.add_page(name, LAYOUT)?;
        let mut tab = GrainTab {
            name: name.to_string(),
            bank: None,
            shown: None,
            view: View::Grains,
            mode: GrainMode::Cloud,
            gesture: None,
            next_gesture: GESTURE_BASE,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The row picked: a control on the panel, or a Q-Link card.
    pub fn row(&self, ui: &Ui) -> Row {
        ui.get::<SettingList>(LIST)
            .and_then(|l| Row::shown(self.view, self.mode).get(l.selected).copied())
            .unwrap_or(Row::Scan)
    }

    /// The section chip open.
    pub fn view(&self) -> View {
        self.view
    }

    /// Opens section `view`: the panel shows its controls, and the row
    /// picked stays if it shows there (a Q-Link's), else its first.
    pub fn open(&mut self, ui: &mut Ui, st: &GrainState, view: View) {
        if view == self.view {
            return;
        }
        let row = self.row(ui);
        self.view = view;
        self.lift();
        let i = row.index_in(view, self.mode).unwrap_or(0);
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.selected = i;
        }
        self.show(ui, st);
    }

    /// Ends the drag, turn or run of taps being made: the next edit is a
    /// new undo step. Call it when a finger lifts and when a Q-Link or the
    /// wheel rests.
    pub fn lift(&mut self) {
        self.gesture = None;
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &GrainState) {
        if self.shown != Some((st.pad, st.layer)) {
            self.shown = Some((st.pad, st.layer));
            self.set_cloud(ui, st, &[], CloudNow::default());
        }
        let mode = st.region.grain_mode;
        if mode != self.mode {
            // The same row stays picked if the new mode shows it, else the
            // Grain mode row, which changed it (or, on another section,
            // its first row).
            let row = self.row(ui);
            self.mode = mode;
            let i = row
                .index_in(self.view, mode)
                .or(Row::GrainMode.index_in(self.view, mode))
                .unwrap_or(0);
            if let Some(list) = ui.get_mut::<SettingList>(LIST) {
                list.selected = i;
            }
        }
        self.show(ui, st);
    }

    /// The grains the engine plays now for the layer shown (positions 0 to
    /// 1 across its region), and what else it says of that cloud
    /// ([`CloudNow`]): call it each time the engine reports, so the cloud
    /// can draw the path the reading place took.
    pub fn set_cloud(&self, ui: &mut Ui, st: &GrainState, grains: &[GrainView], now: CloudNow) {
        let g = &st.region.grain;
        let Some(c) = ui.get_mut::<GrainCloud>(CLOUD) else {
            return;
        };
        let playing = !grains.is_empty();
        c.grains.clear();
        c.grains.extend_from_slice(grains);
        c.follow(now.playhead);
        c.spray_now = now.state.map(|s| s.spray);
        c.thinned = now.state.is_some_and(|s| s.thinned);
        let spray = c.spray_now.unwrap_or(g.spray);
        let mut caption = vec![
            format!("position {} %", percent(g.position)),
            format!("spray ±{} %", percent(spray * 0.5)),
        ];
        if st.granular() {
            caption.push(match (playing, grains.len()) {
                (false, _) => "hit the pad to hear it".to_string(),
                (true, 1) => "1 grain playing".to_string(),
                (true, n) => format!("{n} grains playing"),
            });
        }
        if c.thinned {
            caption.push(format!("thinned to fit {MAX_GRAINS}"));
        }
        caption.push(format!("scan {}", scan_text(g.scan)));
        c.caption = caption.join(" · ");
        c.right = if now.live > 0 {
            format!("{} grains on all pads", now.live)
        } else {
            "drag for the position".into()
        };
    }

    fn show(&self, ui: &mut Ui, st: &GrainState) {
        let g = st.region.grain;
        let n = st.settings.layers.len();
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.pad);
            bar.active = TAB;
            let layer = if n > 1 {
                format!(" · layer {}", st.layer + 1)
            } else {
                String::new()
            };
            bar.title = format!(
                "Pad {} · {}{layer}",
                pad_text(self.bank, st.pad),
                st.settings.name
            );
            let mut about = vec![
                mode_label(st.region.mode).to_string(),
                st.sound.name.clone(),
            ];
            about.push(format!(
                "{} long",
                duration_text(st.region.frames(), st.sound.rate)
            ));
            if let Some(root) = &st.sound.view.root {
                about.push(format!("root {root}"));
            }
            bar.subtitle = about.join(" · ");
        }
        let rate = st.sound.rate.max(1) as f32;
        if let Some(c) = ui.get_mut::<GrainCloud>(CLOUD) {
            c.set_sound(
                Some(st.sound.view.peaks.clone()),
                st.region.start,
                st.region.end,
            );
            c.position = g.position;
            c.spray = g.spray;
            c.peaked = g.spray_shape == GrainSprayShape::Peaked;
            c.scan = g.scan;
            c.grain_len = st.size_ms() * 0.001 * rate / st.region.frames().max(1) as f32;
            c.granular = st.granular();
            c.note = format!(
                "Mode is {}: tap to play this pad as grains",
                mode_label(st.region.mode)
            );
            if !c.granular {
                c.grains.clear();
                c.playhead = None;
            }
        }
        if let Some(xy) = ui.get_mut::<XyPad>(XY) {
            xy.x = g.position;
            xy.y = size_y(&g);
            xy.right = format!(
                "{} % · {}",
                percent(g.position),
                value_text(&st.region, Row::Size)
            );
        }
        let rows = Row::shown(self.view, self.mode)
            .iter()
            .map(|r| Setting {
                name: r.short().into(),
                value: panel_text(&st.region, *r),
            })
            .collect();
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
            list.controls = controls(&st.region, self.view, self.mode);
            list.panel = Some(panel(&st.region, self.view, self.mode));
        }
        if let Some(c) = ui.get_mut::<SectionChips>(SECTIONS) {
            c.selected = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
            c.chips = View::ALL
                .iter()
                .map(|v| SectionChip {
                    name: v.label().into(),
                    sub: section_text(&st.region, *v),
                })
                .collect();
        }
        self.show_cards(ui, st);
    }

    fn show_cards(&self, ui: &mut Ui, st: &GrainState) {
        let g = &st.region.grain;
        let row = self.row(ui);
        for (k, r) in QLINK_ROWS.iter().enumerate() {
            let Some(c) = ui.get_mut::<KnobCard>(QLINKS[k]) else {
                continue;
            };
            c.focused = *r == row || SHIFT_ROWS[k] == row;
            (c.value, c.sub) = split_unit(&value_text(&st.region, *r));
            c.pos = match (r, synced(g)) {
                (Row::Position, _) => g.position,
                (Row::Size, _) => size_y(g),
                (Row::Density, (true, _)) => {
                    list_place(&PER_BEAT, g.sync.and_then(|t| t.grains_per_beat))
                }
                (Row::Density, _) => {
                    (g.density / DENSITY_RANGE.0).ln() / (DENSITY_RANGE.1 / DENSITY_RANGE.0).ln()
                }
                _ => (g.pitch + PITCH_LIMIT) / (2.0 * PITCH_LIMIT),
            }
            .clamp(0.0, 1.0);
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &GrainState) -> Option<GrainAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(GrainAction::Tab(*i)),
            (CLOUD, EventKind::Select(crate::grain_widgets::PLAY_AS_GRAINS)) => {
                let mut r = st.region.clone();
                r.mode = PlayMode::Granular;
                self.edit(st, Key::Row(Row::Mode), r)
            }
            (CLOUD, EventKind::Value(x)) => {
                let mut r = st.region.clone();
                r.grain.position = round_to(*x, 0.001).clamp(0.0, 1.0);
                self.edit(st, Key::Touch, r)
            }
            (XY, EventKind::Marker { .. }) => {
                let (x, y) = ui.get::<XyPad>(XY).map(|p| (p.x, p.y))?;
                let mut r = st.region.clone();
                r.grain.position = round_to(x, 0.001).clamp(0.0, 1.0);
                match r.grain.sync.as_mut() {
                    Some(t) if t.size_beats.is_some() => {
                        let i = (y.clamp(0.0, 1.0) * (BEATS.len() - 1) as f32).round();
                        t.size_beats = Some(BEATS[i as usize]);
                    }
                    _ => r.grain.size_ms = size_at(y),
                }
                self.edit(st, Key::Touch, r)
            }
            (SECTIONS, EventKind::Tab(i)) => {
                let view = *View::ALL.get(*i)?;
                self.open(ui, st, view);
                None
            }
            // A tap on a preset's button: that one (the line's lit one
            // is `delta` from it, or none is lit and the line's end is).
            (LIST, EventKind::Step { index, delta })
                if matches!(
                    Row::shown(self.view, self.mode).get(*index),
                    Some(Row::Preset | Row::PresetMore)
                ) =>
            {
                let row = Row::shown(self.view, self.mode)[*index];
                let first = if row == Row::Preset {
                    0
                } else {
                    PRESETS_A_LINE
                };
                let on = preset_on(&st.region, row);
                let i = first as i32 + on as i32 + delta;
                let n = grain_presets::presets().len() as i32;
                if !(0..n).contains(&i) {
                    return None;
                }
                let r = grain_presets::apply(&st.region, i as usize);
                let a = self.edit(st, Key::Row(Row::Preset), r);
                self.show_cards(ui, st);
                a
            }
            // A tap on a switch flips it.
            (LIST, EventKind::Step { index, .. })
                if Row::shown(self.view, self.mode)
                    .get(*index)
                    .is_some_and(|r| is_switch(*r)) =>
            {
                let row = Row::shown(self.view, self.mode)[*index];
                let delta = if switch_on(&st.region, row) { -1 } else { 1 };
                let a = self.step(st, row, delta);
                self.show_cards(ui, st);
                a
            }
            (LIST, EventKind::Step { index, delta }) => {
                let row = *Row::shown(self.view, self.mode).get(*index)?;
                let a = self.step(st, row, *delta);
                self.show_cards(ui, st);
                a
            }
            (LIST, EventKind::Select(_)) => {
                self.show_cards(ui, st);
                None
            }
            (name, EventKind::Select(_)) if QLINKS.contains(&name) => {
                let k = QLINKS.iter().position(|q| *q == name)?;
                self.pick_row(ui, QLINK_ROWS[k]);
                self.show_cards(ui, st);
                None
            }
            _ => None,
        }
    }

    fn pick_row(&self, ui: &mut Ui, row: Row) {
        if let (Some(list), Some(i)) = (
            ui.get_mut::<SettingList>(LIST),
            row.index_in(self.view, self.mode),
        ) {
            list.selected = i;
        }
    }

    /// A Q-Link turned by `delta` detents: `knob` 0 to 3 turns Position,
    /// Size, Density or Pitch; with `shift` held, Q-Link 1 turns the spray
    /// and Q-Link 4 the pitch spray, and Q-Links 2 and 3 turn finer.
    pub fn qlink(
        &mut self,
        ui: &mut Ui,
        st: &GrainState,
        knob: usize,
        delta: i32,
        shift: bool,
    ) -> Option<GrainAction> {
        let row = if shift {
            *SHIFT_ROWS.get(knob)?
        } else {
            *QLINK_ROWS.get(knob)?
        };
        self.pick_row(ui, row);
        self.show_cards(ui, st);
        if delta == 0 {
            return None;
        }
        let fine = shift && matches!(row, Row::Size | Row::Density);
        let mut r = st.region.clone();
        r.grain = turned(&r.grain, row, delta as f32, fine);
        self.edit(st, Key::Row(row), r)
    }

    /// The data wheel: steps the row picked by `delta` detents.
    pub fn wheel(&mut self, ui: &mut Ui, st: &GrainState, delta: i32) -> Option<GrainAction> {
        let row = self.row(ui);
        let a = self.step(st, row, delta);
        self.show_cards(ui, st);
        a
    }

    /// The action for a changed region (None if nothing changed), in the
    /// undo step of the edit being made, or a new one.
    fn edit(&mut self, st: &GrainState, key: Key, after: Region) -> Option<GrainAction> {
        if after == st.region {
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
        Some(GrainAction::Region {
            pad: st.pad,
            layer: st.layer,
            region: Box::new(after),
            gesture,
        })
    }

    fn step(&mut self, st: &GrainState, row: Row, delta: i32) -> Option<GrainAction> {
        if delta == 0 {
            return None;
        }
        let r = stepped(&st.region, row, delta, st.bpm);
        self.edit(st, Key::Row(row), r)
    }
}

/// How each row `view` shows for `mode` looks on the panel, in
/// [`Row::shown`]'s order (the Q-Links' rows aren't on it, and are chips).
fn controls(r: &Region, view: View, mode: GrainMode) -> Vec<Control> {
    let g = &r.grain;
    let knob = |pos: f32, steps: u32, bipolar: bool| {
        Control::new(Look::Knob {
            pos: pos.clamp(0.0, 1.0),
            steps,
            bipolar,
        })
    };
    let (lo, hi) = LEVEL_RANGE_DB;
    let names = grain_presets::presets().map(|p| p.name.to_string());
    let presets = |row: Row| {
        let first = if row == Row::Preset {
            0
        } else {
            PRESETS_A_LINE
        };
        Control::new(Look::Choice {
            options: names[first..first + PRESETS_A_LINE].to_vec(),
            on: preset_on(r, row),
            off: Vec::new(),
        })
    };
    let cutoff_octaves = (GRAIN_CUTOFF_MAX_HZ / GRAIN_CUTOFF_MIN_HZ).log2();
    Row::shown(view, mode)
        .iter()
        .map(|row| match row {
            Row::Scan => knob(
                (g.scan + SCAN_LIMIT) / (2.0 * SCAN_LIMIT),
                SCANS.len() as u32 - 1,
                true,
            ),
            Row::Spray => knob(g.spray, 20, false),
            Row::SizeSpray => knob(g.size_spray, 20, false),
            Row::PitchSpray => knob(g.pitch_spray / PITCH_SPRAY_LIMIT, 24, false),
            Row::LevelSpray => knob(g.level_spray, 20, false),
            Row::Timing => knob(g.timing, 20, false),
            Row::Reverse => knob(g.reverse, 20, false),
            Row::Spread => knob(g.spread, 20, false),
            Row::Level => knob(
                (level_db(g.gain) - lo) / (hi - lo),
                ((hi - lo) * 2.0) as u32,
                false,
            ),
            Row::Shape => knob(g.window.shape_value(), 20, false),
            Row::Preset | Row::PresetMore => presets(*row),
            Row::Filter => Control::new(Look::Choice {
                options: FILTERS.iter().map(|f| f.0.to_string()).collect(),
                on: FILTERS
                    .iter()
                    .position(|f| f.1 == g.filter.kind)
                    .unwrap_or(0),
                off: Vec::new(),
            }),
            Row::Cutoff => knob(
                (g.filter.cutoff_hz / GRAIN_CUTOFF_MIN_HZ).log2() / cutoff_octaves,
                (cutoff_octaves / CUTOFF_STEP_OCT).round() as u32,
                false,
            ),
            Row::Resonance => knob(g.filter.resonance, 20, false),
            Row::FilterSpray => knob(g.filter.spray, 20, false),
            Row::KeySize => Control::new(Look::Toggle(r.grain_keys.size)),
            Row::KeyScan => Control::new(Look::Toggle(r.grain_keys.scan)),
            Row::Glide => knob(
                list_place(&GLIDES_MS, Some(r.grain_glide_ms)),
                GLIDES_MS.len() as u32 - 1,
                false,
            ),
            // A step up is faster: a shorter cycle when synced.
            Row::LfoRate => match g.lfo.beats {
                Some(b) => knob(
                    1.0 - list_place(&LFO_BEATS, Some(b)),
                    LFO_BEATS.len() as u32 - 1,
                    false,
                ),
                None => knob(
                    list_place(&LFO_RATES, Some(g.lfo.rate_hz)),
                    LFO_RATES.len() as u32 - 1,
                    false,
                ),
            },
            Row::LfoDepth => knob((g.lfo.depth + 1.0) / 2.0, 200, true),
            Row::LfoSync => Control::new(Look::Toggle(g.lfo.beats.is_some())),
            Row::EnvDepth => knob((g.env.depth + 1.0) / 2.0, 200, true),
            Row::EnvAttack => knob(
                list_place(&ENV_TIMES_MS, Some(g.env.attack_ms)),
                ENV_TIMES_MS.len() as u32 - 1,
                false,
            ),
            Row::EnvDecay => knob(
                list_place(&ENV_TIMES_MS, Some(g.env.decay_ms)),
                ENV_TIMES_MS.len() as u32 - 1,
                false,
            ),
            Row::EnvSustain => knob(g.env.sustain, 20, false),
            Row::Mode
            | Row::GrainMode
            | Row::SprayShape
            | Row::Quantise
            | Row::Sync
            | Row::Quality
            | Row::LfoTo
            | Row::LfoShape
            | Row::EnvTo
            | Row::Position
            | Row::Size
            | Row::Density
            | Row::Pitch => Control::new(Look::Chip),
        })
        .collect()
}

/// Which of a line of preset buttons is lit: the preset the layer
/// matches, if it is on `row`'s line; else one past the line's end (none).
fn preset_on(r: &Region, row: Row) -> usize {
    let first = if row == Row::Preset {
        0
    } else {
        PRESETS_A_LINE
    };
    match grain_presets::matching(r) {
        Some(i) if (first..first + PRESETS_A_LINE).contains(&i) => i - first,
        _ => PRESETS_A_LINE,
    }
}

/// Lines of a section's rows for `view` and `mode`: (row, labelled,
/// width; 0 shares the line). Rows the mode doesn't show are left out,
/// and so are lines left empty.
fn lines(view: View, mode: GrainMode, lines: &[&[(Row, bool, i32)]]) -> Vec<Vec<Item>> {
    lines
        .iter()
        .map(|line| {
            line.iter()
                .filter_map(|&(row, labelled, width)| {
                    let mut it = Item::new(row.index_in(view, mode)?);
                    if labelled {
                        it = it.labelled();
                    }
                    Some(if width > 0 { it.wide(width) } else { it })
                })
                .collect::<Vec<_>>()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

/// The panel for section `view` and grain mode `mode`, beside the touch
/// pad. Grains: the mode's own section and Shape, as the mockup Jay
/// approved (17 Nov). Movement: the LFO and the envelope. Filter and
/// keys: the filter on each grain, and what keys do. Presets: two lines
/// of four buttons.
fn panel(r: &Region, view: View, mode: GrainMode) -> Panel {
    let g = &r.grain;
    match view {
        View::Grains => grains_panel(mode),
        View::Movement => {
            let lfo = if g.lfo.target == MotionTarget::Off {
                "off: pick what it moves"
            } else {
                "over and over, from each hit"
            };
            let env = if g.env.target == MotionTarget::Off {
                "off: pick what it moves"
            } else {
                "once, from each hit"
            };
            Panel {
                sections: vec![
                    Section::new(
                        "LFO",
                        430,
                        lines(
                            view,
                            mode,
                            &[
                                &[(Row::LfoTo, true, 0), (Row::LfoShape, true, 0)],
                                &[
                                    (Row::LfoRate, false, 0),
                                    (Row::LfoDepth, false, 0),
                                    (Row::LfoSync, false, 180),
                                ],
                            ],
                        ),
                    )
                    .note(lfo),
                    Section::new(
                        "Envelope",
                        430,
                        lines(
                            view,
                            mode,
                            &[
                                &[(Row::EnvTo, true, 240)],
                                &[
                                    (Row::EnvDepth, false, 0),
                                    (Row::EnvAttack, false, 0),
                                    (Row::EnvDecay, false, 0),
                                    (Row::EnvSustain, false, 0),
                                ],
                            ],
                        ),
                    )
                    .note(env),
                ],
            }
        }
        View::FilterKeys => {
            let filter = if g.filter.kind == GrainFilterKind::Off {
                "off: pick a pass"
            } else {
                "set as each grain starts"
            };
            Panel {
                sections: vec![
                    Section::new(
                        "Filter",
                        480,
                        lines(
                            view,
                            mode,
                            &[
                                &[(Row::Filter, false, 0)],
                                &[
                                    (Row::Cutoff, false, 0),
                                    (Row::Resonance, false, 0),
                                    (Row::FilterSpray, false, 0),
                                ],
                            ],
                        ),
                    )
                    .note(filter),
                    Section::new(
                        "Keys and quality",
                        380,
                        lines(
                            view,
                            mode,
                            &[
                                &[(Row::KeySize, false, 0), (Row::KeyScan, false, 0)],
                                &[(Row::Glide, false, 0), (Row::Quality, true, 0)],
                            ],
                        ),
                    )
                    .note("how it plays"),
                ],
            }
        }
        // The note says what the lit one sounds like.
        View::Presets => Panel {
            sections: vec![Section::new(
                "Presets",
                860,
                lines(
                    view,
                    mode,
                    &[&[(Row::Preset, false, 0)], &[(Row::PresetMore, false, 0)]],
                ),
            )
            .note(grain_presets::matching(r).map_or(
                "a starting point: the sound, position, level and quality stay",
                |i| grain_presets::presets()[i].about,
            ))],
        },
    }
}

/// The line under section chip `view`: what is in it now.
fn section_text(r: &Region, view: View) -> String {
    let g = &r.grain;
    match view {
        View::Grains if r.mode != PlayMode::Granular => {
            format!("Mode {}: no grains", mode_label(r.mode))
        }
        View::Grains => format!(
            "{} · scan {} · spray {} %",
            value_text(r, Row::GrainMode),
            scan_text(g.scan),
            percent(g.spray)
        ),
        View::Movement => {
            let mut parts = Vec::new();
            if g.lfo.target != MotionTarget::Off {
                parts.push(format!(
                    "LFO to {}",
                    target_text(g.lfo.target, r.grain_mode)
                ));
            }
            if g.env.target != MotionTarget::Off {
                parts.push(format!(
                    "envelope to {}",
                    target_text(g.env.target, r.grain_mode)
                ));
            }
            if parts.is_empty() {
                "nothing moves".into()
            } else {
                parts.join(" · ")
            }
        }
        View::FilterKeys => {
            let mut parts = vec![if g.filter.kind == GrainFilterKind::Off {
                "no filter".to_string()
            } else {
                format!(
                    "{} {}",
                    value_text(r, Row::Filter),
                    hz_text(g.filter.cutoff_hz)
                )
            }];
            // Then one more: a quality mode other than Clean, else one
            // thing keys do (the glide, else what follows them).
            let keys = &r.grain_keys;
            if g.quality != GrainQuality::Clean {
                parts.push(g.quality.name().into());
            } else if r.grain_glide_ms > 0.0 {
                parts.push(format!("glide {}", ms_text(r.grain_glide_ms)));
            } else if keys.size || keys.scan {
                parts.push(
                    match (keys.size, keys.scan) {
                        (true, true) => "size, scan follow key",
                        (true, false) => "size follows key",
                        _ => "scan follows key",
                    }
                    .into(),
                );
            }
            parts.join(" · ")
        }
        View::Presets => match grain_presets::matching(r) {
            Some(i) => grain_presets::presets()[i].name.into(),
            None => "your own settings".into(),
        },
    }
}

/// The Grains section's panel for a grain mode: the mode's own section
/// (Cloud: how grains scatter) and Shape, with the play mode in Shape. A
/// mode's rows that it doesn't show are left out ([`Row::shown`]).
fn grains_panel(mode: GrainMode) -> Panel {
    let lines = |l: &[&[(Row, bool, i32)]]| lines(View::Grains, mode, l);
    let (title, note) = match mode {
        GrainMode::Cloud => ("Cloud", "how grains scatter"),
        GrainMode::Stretch => ("Stretch", "how it moves"),
        GrainMode::Repeat => ("Repeat", "how it varies"),
        GrainMode::Tone => ("Tone", "how it moves"),
    };
    Panel {
        sections: vec![
            Section::new(
                title,
                450,
                lines(&[
                    &[
                        (Row::Scan, false, 0),
                        (Row::Spray, false, 0),
                        (Row::SizeSpray, false, 0),
                        (Row::PitchSpray, false, 0),
                        (Row::LevelSpray, false, 0),
                    ],
                    &[
                        (Row::Timing, false, 0),
                        (Row::Reverse, false, 0),
                        (Row::Spread, false, 0),
                        (Row::Level, false, 0),
                        (Row::SprayShape, true, 104),
                    ],
                ]),
            )
            .note(note),
            Section::new(
                "Shape",
                410,
                lines(&[
                    &[(Row::Mode, true, 0), (Row::GrainMode, true, 0)],
                    &[
                        (Row::Shape, false, 76),
                        (Row::Quantise, true, 0),
                        (Row::Sync, true, 0),
                    ],
                ]),
            )
            .note("this layer"),
        ],
    }
}

/// `r` with row `row` stepped by `delta` steps (an arrow, or a detent of
/// the wheel), within its limits. `bpm`, the project tempo, is where
/// turning Tempo sync on starts: the beats nearest the size and density
/// heard.
pub fn stepped(r: &Region, row: Row, delta: i32, bpm: f64) -> Region {
    if matches!(row, Row::Preset | Row::PresetMore) {
        // From a preset to the next; from none, to the first (or the last).
        let n = grain_presets::presets().len();
        let i = match grain_presets::matching(r) {
            Some(i) => shift(i, delta, n),
            None if delta > 0 => 0,
            None => n - 1,
        };
        return grain_presets::apply(r, i);
    }
    let mut r = r.clone();
    let g = &mut r.grain;
    let d = delta as f32;
    let (dsync, ssync) = synced(g);
    match row {
        Row::Sync => {
            let i = SYNCS
                .iter()
                .position(|x| (x.1, x.2) == (dsync, ssync))
                .unwrap_or(0);
            let (_, density, size) = SYNCS[shift(i, delta, SYNCS.len())];
            set_sync(g, density, size, bpm);
        }
        Row::Density if dsync => step_sync(g, &PER_BEAT, delta, true),
        Row::Size if ssync => step_sync(g, &BEATS, delta, false),
        // A tone's note, a semitone a step; its grains keep their cycles.
        Row::Density if r.grain_mode == GrainMode::Tone => {
            let cycles = tone_cycles(g);
            let note = (tone_note(g.density).round() + d).clamp(TONE_NOTES.0, TONE_NOTES.1);
            g.density = note_hz(note);
            g.size_ms = cycles * 1000.0 / g.density;
        }
        Row::Size if r.grain_mode == GrainMode::Tone => {
            let cycles =
                (round_to(tone_cycles(g), 0.5) + d * 0.5).clamp(TONE_CYCLES.0, TONE_CYCLES.1);
            g.size_ms = cycles * 1000.0 / g.density;
        }
        Row::Level => g.gain = level_gain(level_db(g.gain) + d * 0.5),
        Row::Filter => {
            let i = FILTERS
                .iter()
                .position(|f| f.1 == g.filter.kind)
                .unwrap_or(0);
            g.filter.kind = FILTERS[shift(i, delta, FILTERS.len())].1;
        }
        Row::Cutoff => {
            g.filter.cutoff_hz = (g.filter.cutoff_hz * (d * CUTOFF_STEP_OCT).exp2())
                .round()
                .clamp(GRAIN_CUTOFF_MIN_HZ, GRAIN_CUTOFF_MAX_HZ)
        }
        Row::Resonance => {
            g.filter.resonance = round_to(g.filter.resonance + d * 0.05, 0.01).clamp(0.0, 1.0)
        }
        Row::FilterSpray => {
            g.filter.spray = round_to(g.filter.spray + d * 0.05, 0.01).clamp(0.0, 1.0)
        }
        Row::Mode => {
            let i = MODES.iter().position(|m| *m == r.mode).unwrap_or(0);
            r.mode = MODES[shift(i, delta, MODES.len())];
            // Loop mode needs a loop, or the sampler refuses the edit and
            // the row stops there (live test 3); as the Wave tab does.
            if r.mode == PlayMode::Loop && r.sustain_loop.is_none() {
                r.sustain_loop = Some(crate::wave_tab::default_loop(&r));
            }
        }
        Row::GrainMode => {
            let i = GRAIN_MODES
                .iter()
                .position(|m| m.1 == r.grain_mode)
                .unwrap_or(0);
            let mode = GRAIN_MODES[shift(i, delta, GRAIN_MODES.len())].1;
            if mode != r.grain_mode {
                r.grain_mode = mode;
                r.grain = mode_settings(&r.grain, mode, bpm);
            }
        }
        Row::Scan => g.scan = step_list(&SCANS, g.scan, delta),
        Row::Spray => g.spray = round_to(g.spray + d * 0.05, 0.01).clamp(0.0, 1.0),
        Row::SprayShape => {
            let i = SPRAY_SHAPES
                .iter()
                .position(|s| s.1 == g.spray_shape)
                .unwrap_or(0);
            g.spray_shape = SPRAY_SHAPES[shift(i, delta, SPRAY_SHAPES.len())].1;
        }
        Row::Quality => {
            let all = GrainQuality::ALL;
            g.quality = all[shift(g.quality.index(), delta, all.len())];
        }
        Row::SizeSpray => g.size_spray = round_to(g.size_spray + d * 0.05, 0.01).clamp(0.0, 1.0),
        Row::PitchSpray => {
            g.pitch_spray = round_to(g.pitch_spray + d * 0.5, 0.1).clamp(0.0, PITCH_SPRAY_LIMIT)
        }
        Row::LevelSpray => g.level_spray = round_to(g.level_spray + d * 0.05, 0.01).clamp(0.0, 1.0),
        // From any window (a project's older ones too), 5 % a step.
        Row::Shape => {
            g.window = GrainWindow::Shape {
                shape: round_to(g.window.shape_value() + d * 0.05, 0.01).clamp(0.0, 1.0),
            }
        }
        Row::Timing => g.timing = round_to(g.timing + d * 0.05, 0.01).clamp(0.0, 1.0),
        Row::Reverse => g.reverse = round_to(g.reverse + d * 0.05, 0.01).clamp(0.0, 1.0),
        Row::Spread => g.spread = round_to(g.spread + d * 0.05, 0.01).clamp(0.0, 1.0),
        Row::Quantise => {
            let i = SCALES
                .iter()
                .position(|s| s.1 == g.scale & 0xFFF)
                .unwrap_or(0);
            g.scale = SCALES[shift(i, delta, SCALES.len())].1;
        }
        Row::Position => g.position = round_to(g.position + d * 0.01, 0.001).clamp(0.0, 1.0),
        Row::Size => g.size_ms = step_list(&SIZES_MS, g.size_ms, delta),
        Row::Density => g.density = step_list(&DENSITIES, g.density, delta),
        Row::Pitch => g.pitch = round_to(g.pitch + d, 1.0).clamp(-PITCH_LIMIT, PITCH_LIMIT),
        // Put on above.
        Row::Preset | Row::PresetMore => {}
        Row::KeySize => r.grain_keys.size = delta > 0,
        Row::KeyScan => r.grain_keys.scan = delta > 0,
        Row::Glide => r.grain_glide_ms = step_list(&GLIDES_MS, r.grain_glide_ms, delta),
        Row::LfoTo => g.lfo.target = step_target(g.lfo.target, delta),
        Row::LfoShape => {
            let i = LFO_SHAPES
                .iter()
                .position(|s| s.1 == g.lfo.shape)
                .unwrap_or(0);
            g.lfo.shape = LFO_SHAPES[shift(i, delta, LFO_SHAPES.len())].1;
        }
        Row::LfoRate => match g.lfo.beats {
            // A step up is faster, as for the free rate: a shorter cycle.
            Some(b) => g.lfo.beats = Some(step_list(&LFO_BEATS, b, -delta)),
            None => g.lfo.rate_hz = step_list(&LFO_RATES, g.lfo.rate_hz, delta),
        },
        // On, the cycle nearest the rate heard at `bpm`; off, the rate
        // nearest the cycle's.
        Row::LfoSync => match (g.lfo.beats, delta > 0) {
            (None, true) => {
                let beats = bpm as f32 / 60.0 / g.lfo.rate_hz.max(0.01);
                g.lfo.beats = Some(nearest(&LFO_BEATS, beats));
            }
            (Some(b), false) => {
                g.lfo.rate_hz = nearest(&LFO_RATES, bpm as f32 / 60.0 / b.max(0.01));
                g.lfo.beats = None;
            }
            _ => {}
        },
        Row::LfoDepth => g.lfo.depth = round_to(g.lfo.depth + d * 0.01, 0.01).clamp(-1.0, 1.0),
        Row::EnvTo => g.env.target = step_target(g.env.target, delta),
        Row::EnvDepth => g.env.depth = round_to(g.env.depth + d * 0.01, 0.01).clamp(-1.0, 1.0),
        Row::EnvAttack => g.env.attack_ms = step_list(&ENV_TIMES_MS, g.env.attack_ms, delta),
        Row::EnvDecay => g.env.decay_ms = step_list(&ENV_TIMES_MS, g.env.decay_ms, delta),
        Row::EnvSustain => g.env.sustain = round_to(g.env.sustain + d * 0.05, 0.01).clamp(0.0, 1.0),
    }
    r
}

/// Whether `row` is a switch on the panel: a tap flips it.
fn is_switch(row: Row) -> bool {
    matches!(row, Row::KeySize | Row::KeyScan | Row::LfoSync)
}

/// Whether switch `row` is on in `r`.
fn switch_on(r: &Region, row: Row) -> bool {
    match row {
        Row::KeySize => r.grain_keys.size,
        Row::KeyScan => r.grain_keys.scan,
        Row::LfoSync => r.grain.lfo.beats.is_some(),
        _ => false,
    }
}

/// What the LFO or the envelope moves, `delta` steps on.
fn step_target(t: MotionTarget, delta: i32) -> MotionTarget {
    let i = MOTION_TARGETS.iter().position(|x| *x == t).unwrap_or(0);
    MOTION_TARGETS[shift(i, delta, MOTION_TARGETS.len())]
}

/// `g` with a Q-Link's row turned by `d` detents (`fine`: Size a
/// millisecond and Density a tenth a detent). Synced, Size and Density
/// step their beats a detent at a time.
pub fn turned(g: &GrainParams, row: Row, d: f32, fine: bool) -> GrainParams {
    let mut g = *g;
    let (smin, smax) = SIZE_RANGE_MS;
    let (dmin, dmax) = DENSITY_RANGE;
    let (dsync, ssync) = synced(&g);
    match row {
        Row::Size if ssync => step_sync(&mut g, &BEATS, d as i32, false),
        Row::Density if dsync => step_sync(&mut g, &PER_BEAT, d as i32, true),
        Row::Position => g.position = round_to(g.position + d * 0.005, 0.001).clamp(0.0, 1.0),
        Row::Spray => g.spray = round_to(g.spray + d * 0.01, 0.01).clamp(0.0, 1.0),
        Row::Size if fine => g.size_ms = (g.size_ms.round() + d).clamp(smin, smax),
        Row::Size => g.size_ms = doubled(g.size_ms, d, 1.0).clamp(smin, smax),
        Row::Density if fine => g.density = round_to(g.density + d * 0.1, 0.1).clamp(dmin, dmax),
        Row::Density => g.density = doubled(g.density, d, 0.1).clamp(dmin, dmax),
        Row::Pitch => g.pitch = round_to(g.pitch + d, 1.0).clamp(-PITCH_LIMIT, PITCH_LIMIT),
        Row::PitchSpray => {
            g.pitch_spray = round_to(g.pitch_spray + d * 0.1, 0.1).clamp(0.0, PITCH_SPRAY_LIMIT)
        }
        _ => {}
    }
    g
}

/// `x` doubled every [`DOUBLING_DETENTS`] detents of `d`, rounded to
/// `unit`, moving at least one unit a detent.
fn doubled(x: f32, d: f32, unit: f32) -> f32 {
    let to = round_to(x * (d / DOUBLING_DETENTS).exp2(), unit);
    if to == x {
        x + d.signum() * unit
    } else {
        to
    }
}

/// The entry of `list` after (`delta` > 0) or before `now`, `delta` steps
/// on; a value between entries goes to the next one in that direction.
fn step_list(list: &[f32], now: f32, delta: i32) -> f32 {
    let i = if delta > 0 {
        list.iter().position(|&v| v > now).map_or(list.len(), |i| i) as i32 + delta - 1
    } else {
        list.iter().rposition(|&v| v < now).map_or(-1, |i| i as i32) + delta + 1
    };
    list[i.clamp(0, list.len() as i32 - 1) as usize]
}

/// Whether the density and the size are locked to the tempo.
pub fn synced(g: &GrainParams) -> (bool, bool) {
    g.sync.map_or((false, false), |t| {
        (t.grains_per_beat.is_some(), t.size_beats.is_some())
    })
}

/// Locks the density and the size to the tempo or frees them. A newly
/// locked one starts at the beats nearest what plays at `bpm`; a freed one
/// keeps what it was set to before.
fn set_sync(g: &mut GrainParams, density: bool, size: bool, bpm: f64) {
    let bpm = if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    };
    let beat_s = (60.0 / bpm) as f32;
    let old = g.sync;
    let per_beat = old
        .and_then(|t| t.grains_per_beat)
        .unwrap_or_else(|| nearest(&PER_BEAT, g.density * beat_s));
    let beats = old
        .and_then(|t| t.size_beats)
        .unwrap_or_else(|| nearest(&BEATS, g.size_ms * 0.001 / beat_s));
    g.sync = (density || size).then(|| TempoSync {
        frames_per_beat: old.map_or(SAVED_FRAMES_PER_BEAT, |t| t.frames_per_beat),
        grains_per_beat: density.then_some(per_beat),
        size_beats: size.then_some(beats),
    });
}

/// Steps the synced density (`density`) or size through `list`.
fn step_sync(g: &mut GrainParams, list: &[f32], delta: i32, density: bool) {
    let Some(t) = g.sync.as_mut() else { return };
    let slot = if density {
        &mut t.grains_per_beat
    } else {
        &mut t.size_beats
    };
    if let Some(v) = slot.as_mut() {
        let i = list
            .iter()
            .position(|x| *x == nearest(list, *v))
            .unwrap_or(0);
        *v = list[shift(i, delta, list.len())];
    }
}

/// The entry of `list` nearest `x`, on a log scale.
fn nearest(list: &[f32], x: f32) -> f32 {
    let x = x.max(1e-6);
    list.iter()
        .copied()
        .min_by(|a, b| {
            let (da, db) = ((a / x).ln().abs(), (b / x).ln().abs());
            da.total_cmp(&db)
        })
        .unwrap_or(1.0)
}

/// Where `v` sits in `list`, 0 to 1, for a knob.
fn list_place(list: &[f32], v: Option<f32>) -> f32 {
    let Some(v) = v else { return 0.0 };
    let i = list
        .iter()
        .position(|x| *x == nearest(list, v))
        .unwrap_or(0);
    i as f32 / (list.len() - 1).max(1) as f32
}

/// The grain size heard at `bpm`, in ms.
pub fn size_ms_at(g: &GrainParams, bpm: f64) -> f32 {
    match g.sync.and_then(|t| t.size_beats) {
        Some(b) if bpm.is_finite() && bpm > 0.0 => b * (60_000.0 / bpm) as f32,
        _ => g.size_ms,
    }
}

/// Where the size sits on the touch pad's height and its knob, 0 to 1.
fn size_y(g: &GrainParams) -> f32 {
    match g.sync.and_then(|t| t.size_beats) {
        Some(b) => list_place(&BEATS, Some(b)),
        None => size_place(g.size_ms),
    }
}

/// A linear gain in dB (−∞ as the range's floor).
fn level_db(gain: f32) -> f32 {
    if gain > 0.0 {
        20.0 * gain.log10()
    } else {
        LEVEL_RANGE_DB.0
    }
}

/// The gain for `db`, rounded to a tenth of a dB and within the range.
fn level_gain(db: f32) -> f32 {
    let db = round_to(db, 0.1).clamp(LEVEL_RANGE_DB.0, LEVEL_RANGE_DB.1);
    10f32.powf(db / 20.0)
}

/// "1/16 beat", "1 beat", "2 beats".
fn beats_text(b: f32) -> String {
    if b >= 1.0 {
        let n = b.round() as i32;
        if n == 1 {
            "1 beat".into()
        } else {
            format!("{n} beats")
        }
    } else {
        format!("1/{} beat", (1.0 / b).round() as i32)
    }
}

/// Grains a beat: "4 /beat", "0.25 /beat" (one a bar of 4/4).
fn per_beat_text(g: f32) -> String {
    format!("{} /beat", trim(g))
}

/// Where grain mode `mode` starts from `g`: its own settings, keeping the
/// position and the level. A cloud scatters a little (spray, spread,
/// random timing); a stretch reads at the sound's speed with regular
/// grains overlapping four times, nothing random, so its grains stay in
/// step; a repeat replays the spot under the position every sixteenth,
/// a sixteenth long, with short fades; a tone plays A2 (110 Hz, the
/// density) with grains four cycles long, regular and frozen, so it holds
/// one spot's colour at that note.
pub fn mode_settings(g: &GrainParams, mode: GrainMode, bpm: f64) -> GrainParams {
    // The movement, the filter and the quality stay: they are how the
    // sound moves and its colour, whatever plays it.
    let kept = GrainParams {
        position: g.position,
        gain: g.gain,
        lfo: g.lfo,
        env: g.env,
        filter: g.filter,
        quality: g.quality,
        ..GrainParams::default()
    };
    match mode {
        GrainMode::Cloud => GrainParams {
            spray: 0.1,
            spread: 0.5,
            size_ms: 100.0,
            ..kept
        },
        // No randomness, its filter's spray included.
        GrainMode::Stretch => GrainParams {
            scan: 1.0,
            size_ms: 90.0,
            density: 45.0,
            timing: 0.0,
            filter: GrainFilter {
                spray: 0.0,
                ..g.filter
            },
            ..kept
        },
        GrainMode::Tone => GrainParams {
            density: note_hz(45.0),
            size_ms: 4.0 * 1000.0 / note_hz(45.0),
            timing: 0.0,
            ..kept
        },
        GrainMode::Repeat => {
            let mut r = GrainParams {
                timing: 0.0,
                window: GrainWindow::Shape { shape: 0.1 },
                ..kept
            };
            set_sync(&mut r, true, true, bpm);
            if let Some(t) = r.sync.as_mut() {
                (t.grains_per_beat, t.size_beats) = (Some(4.0), Some(0.25));
            }
            r
        }
    }
}

/// How Spray spreads grains: the row's words.
const SPRAY_SHAPES: [(&str, GrainSprayShape); 2] = [
    ("Even", GrainSprayShape::Even),
    ("Peaked", GrainSprayShape::Peaked),
];

fn shift(i: usize, delta: i32, len: usize) -> usize {
    (i as i32 + delta).clamp(0, len as i32 - 1) as usize
}

fn round_to(x: f32, step: f32) -> f32 {
    (x / step).round() * step
}

/// Where a grain size sits on the touch pad's height, 0 to 1 (log).
pub fn size_place(ms: f32) -> f32 {
    let (lo, hi) = SIZE_RANGE_MS;
    ((ms.clamp(lo, hi) / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
}

/// The grain size at height `y` (0 to 1) on the touch pad, in whole ms.
pub fn size_at(y: f32) -> f32 {
    let (lo, hi) = SIZE_RANGE_MS;
    (lo * (hi / lo).powf(y.clamp(0.0, 1.0)))
        .round()
        .clamp(lo, hi)
}

// ---------------------------------------------------------------- text

/// How random the gaps between grains are: Regular, a share random, or
/// Random.
fn timing_text(t: f32) -> String {
    match percent(t) {
        0 => "Regular".into(),
        100 => "Random".into(),
        p => format!("{p} % random"),
    }
}

/// A row's value under its knob on the panel: [`value_text`], but Timing
/// in short ("30 %"), as its knob is narrow.
fn panel_text(r: &Region, row: Row) -> String {
    match row {
        Row::Timing if (1..100).contains(&percent(r.grain.timing)) => {
            format!("{} %", percent(r.grain.timing))
        }
        _ => value_text(r, row),
    }
}

/// A row's value as the list shows it.
pub fn value_text(r: &Region, row: Row) -> String {
    let g = &r.grain;
    match row {
        Row::Mode => mode_label(r.mode).into(),
        Row::GrainMode => GRAIN_MODES
            .iter()
            .find(|m| m.1 == r.grain_mode)
            .map_or("Cloud", |m| m.0)
            .into(),
        Row::Preset | Row::PresetMore => grain_presets::matching(r)
            .map_or("—", |i| grain_presets::presets()[i].name)
            .into(),
        Row::Scan if r.grain_mode == GrainMode::Stretch && r.warp.is_some() && g.scan != 0.0 => {
            format!("{} · in time", scan_text(g.scan))
        }
        Row::Scan => scan_text(g.scan),
        Row::Spray => format!("{} %", percent(g.spray)),
        Row::SprayShape => SPRAY_SHAPES
            .iter()
            .find(|s| s.1 == g.spray_shape)
            .map_or("Even", |s| s.0)
            .into(),
        Row::SizeSpray => format!("{} %", percent(g.size_spray)),
        Row::PitchSpray => format!("±{:.1} st", g.pitch_spray),
        Row::LevelSpray => format!("{} %", percent(g.level_spray)),
        Row::Timing => timing_text(g.timing),
        Row::Shape => window_label(g.window),
        Row::Reverse => format!("{} %", percent(g.reverse)),
        Row::Spread => format!("{} %", percent(g.spread)),
        Row::Quantise => quantise_label(g.scale).into(),
        Row::Sync => {
            let on = synced(g);
            SYNCS
                .iter()
                .find(|x| (x.1, x.2) == on)
                .map_or("Off", |x| x.0)
                .into()
        }
        Row::Position => format!("{:.1} %", g.position * 100.0),
        Row::Size if r.grain_mode == GrainMode::Tone => {
            format!("{} · {:.1} cycles", ms_text(g.size_ms), tone_cycles(g))
        }
        Row::Density if r.grain_mode == GrainMode::Tone => {
            let note = tone_note(g.density).round().clamp(0.0, 127.0) as u8;
            format!("{} · {}", note_name(note), hz_text(g.density))
        }
        Row::Size => match g.sync.and_then(|t| t.size_beats) {
            Some(b) => beats_text(b),
            None => ms_text(g.size_ms),
        },
        Row::Density => match g.sync.and_then(|t| t.grains_per_beat) {
            Some(n) => per_beat_text(n),
            None => density_text(g.density),
        },
        Row::Pitch => format!("{:+} st", g.pitch.round() as i32),
        Row::Level => format!("{:+.1} dB", level_db(g.gain)),
        Row::Filter => FILTERS
            .iter()
            .find(|f| f.1 == g.filter.kind)
            .map_or("Off", |f| f.0)
            .into(),
        Row::Cutoff => hz_text(g.filter.cutoff_hz),
        Row::Resonance => format!("{} %", percent(g.filter.resonance)),
        Row::FilterSpray if g.filter.spray <= 0.0 => "Off".into(),
        Row::FilterSpray => format!(
            "±{} oct",
            trim(round_to(g.filter.spray * GRAIN_FILTER_SPRAY_OCTAVES, 0.01))
        ),
        Row::KeySize => on_off(r.grain_keys.size).into(),
        Row::KeyScan => on_off(r.grain_keys.scan).into(),
        Row::Quality => g.quality.name().into(),
        Row::Glide if r.grain_glide_ms <= 0.0 => "Off".into(),
        Row::Glide => ms_text(r.grain_glide_ms),
        Row::LfoTo => target_text(g.lfo.target, r.grain_mode).into(),
        Row::LfoShape => LFO_SHAPES
            .iter()
            .find(|s| s.1 == g.lfo.shape)
            .map_or("Sine", |s| s.0)
            .into(),
        Row::LfoRate => match g.lfo.beats {
            Some(b) => beats_text(b),
            None if g.lfo.rate_hz < 1.0 => format!("{:.2} Hz", g.lfo.rate_hz),
            None => format!("{:.1} Hz", g.lfo.rate_hz),
        },
        Row::LfoSync => on_off(g.lfo.beats.is_some()).into(),
        Row::LfoDepth => format!("{:+} %", percent(g.lfo.depth)),
        Row::EnvTo => target_text(g.env.target, r.grain_mode).into(),
        Row::EnvDepth => format!("{:+} %", percent(g.env.depth)),
        Row::EnvAttack => ms_text(g.env.attack_ms),
        Row::EnvDecay => ms_text(g.env.decay_ms),
        Row::EnvSustain => format!("{} %", percent(g.env.sustain)),
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

/// What a movement row moves, in words (a tone's density is its note).
fn target_text(t: MotionTarget, mode: GrainMode) -> &'static str {
    match t {
        MotionTarget::Off => "Off",
        MotionTarget::Position => "Position",
        MotionTarget::Size => "Size",
        MotionTarget::Density if mode == GrainMode::Tone => "Note",
        MotionTarget::Density => "Density",
        MotionTarget::Pitch => "Pitch",
        MotionTarget::Spray => "Spray",
        MotionTarget::Cutoff => "Cutoff",
    }
}

fn percent(x: f32) -> i32 {
    (x * 100.0).round() as i32
}

pub fn scan_text(s: f32) -> String {
    if s == 0.0 {
        "Frozen".into()
    } else {
        format!("{}×", trim(s))
    }
}

/// A number without trailing zeros: 0.25, 1, -0.125.
fn trim(x: f32) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

pub fn ms_text(ms: f32) -> String {
    if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

/// A frequency: 27.5 Hz, 110 Hz, 1.05 kHz.
fn hz_text(hz: f32) -> String {
    if hz < 100.0 {
        format!("{hz:.1} Hz")
    } else if hz < 1000.0 {
        format!("{hz:.0} Hz")
    } else {
        format!("{:.2} kHz", hz / 1000.0)
    }
}

/// The MIDI note (fractional) a frequency is.
fn tone_note(hz: f32) -> f32 {
    69.0 + 12.0 * (hz.max(1.0) / 440.0).log2()
}

/// MIDI note `n`'s frequency.
fn note_hz(n: f32) -> f32 {
    440.0 * ((n - 69.0) / 12.0).exp2()
}

/// How many cycles of a tone's note each grain holds.
fn tone_cycles(g: &GrainParams) -> f32 {
    g.size_ms * g.density / 1000.0
}

pub fn density_text(d: f32) -> String {
    if d < 10.0 && d.fract() != 0.0 {
        format!("{d:.1} /s")
    } else {
        format!("{d:.0} /s")
    }
}

/// The grains' shape: a Shape value as a per cent and what it is near
/// (Square, Trapezoid, Triangle, Hann, Swell, Pluck); a window from before
/// the Shape knob by its old name.
pub fn window_label(w: GrainWindow) -> String {
    match w {
        GrainWindow::Hann => "Hann".into(),
        GrainWindow::Tukey { taper } => format!("Tukey {}", trim(taper)),
        GrainWindow::Trapezoid { ramp } => format!("Trapezoid {}", trim(ramp)),
        GrainWindow::Exponential { decay } => format!("Percussive {}", trim(decay)),
        GrainWindow::Shape { .. } => {
            let s = w.shape_value();
            let name = match s {
                s if s < 0.1 => "Square",
                s if s < 0.3 => "Trapezoid",
                s if s < 0.5 => "Triangle",
                s if s < 0.7 => "Hann",
                s if s < 0.9 => "Swell",
                _ => "Pluck",
            };
            format!("{} % · {name}", percent(s))
        }
    }
}

pub fn quantise_label(mask: u16) -> &'static str {
    SCALES
        .iter()
        .find(|s| s.1 == mask & 0xFFF)
        .map_or("Custom", |s| s.0)
}

/// "84 ms" as ("84", "ms"), for a Q-Link card.
fn split_unit(text: &str) -> (String, String) {
    match text.rsplit_once(' ') {
        Some((v, u)) => (v.to_string(), u.to_string()),
        None => (text.to_string(), String::new()),
    }
}

/// What changed between two versions of a layer's region on this tab, for
/// the log: "size 80 ms -> 120 ms, scan 0.25× -> Frozen". Empty if nothing
/// did.
pub fn describe(a: &Region, b: &Region) -> String {
    LOG_ORDER
        .iter()
        .filter_map(|&row| {
            let (x, y) = (value_text(a, row), value_text(b, row));
            (x != y).then(|| format!("{} {x} -> {y}", row.label().to_lowercase()))
        })
        .collect::<Vec<_>>()
        .join(", ")
}
