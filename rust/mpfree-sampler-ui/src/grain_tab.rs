//! The grain settings of a layer: its rows, their values and how they step.
//! The Sample page's Grain tool ([`crate::grain_tool`], the one Sample page's
//! step 5, Jay 18 Nov 2026) lays them out on the panel and turns them; the
//! Grain page they were on went with it.
//!
//! - **Cloud** (how grains scatter: scan speed, spray, size, pitch and level
//!   spray, timing, reverse chance, stereo spread, the cloud's level, the
//!   spray's shape) and **Shape** (the play mode, only Granular playing
//!   grains; the grain mode; the grains' Shape; pitch quantise; tempo sync).
//!   A grain mode other than Cloud uses only its own rows ([`Row::shown`]).
//! - **Movement**: an **LFO** and an **Envelope**, each moving one setting
//!   (position, size, density, pitch, spray or the filter's cutoff) of the
//!   grains as they are made, by up to its depth either way. The LFO runs
//!   free in Hz or once every so many beats of the project tempo; the
//!   envelope rises over its attack and falls over its decay to its
//!   sustain. Both start again with each hit.
//! - **Filter and keys**: a **Filter** on each grain (off, low, band or high
//!   pass; cutoff, resonance, and a spray that gives each grain its own
//!   cutoff); played from keys (or tuned) the grains move in pitch (a
//!   Tone's note); **Size follows key** shortens them as much and **Scan
//!   follows key** speeds the scan as much. **Glide** slides to each new
//!   key. **Quality** colours the cloud on its way out: Clean, Tape (wow,
//!   flutter, a soft saturation) or Lo-fi (11 kHz, 8 bits).
//! - **Presets**: the eight of [`mpfree_sampler::grain_presets`]. One puts
//!   its mode, settings, keys, glide and movement on the layer (the sound,
//!   position, level and quality stay).
//! - **Tempo sync** locks the density, the size or both to the project
//!   tempo: Density then steps in grains a beat and Size in beats, and the
//!   engine starts each grain on the beat grid at the tempo playing now
//!   (`Engine::frames_per_beat`).
//! - **Q-Links 1 to 4** turn Position, Size, Density and Pitch
//!   ([`QLINK_ROWS`]). With SHIFT, Q-Link 1 turns the spray and Q-Link 4
//!   the pitch spray, and Q-Links 2 and 3 turn finer ([`SHIFT_ROWS`],
//!   [`turned`]).

use mpfree_analysis::pitch::note_name;
use mpfree_sampler::engine::CloudState;
use mpfree_sampler::grain_field::GrainField;
use mpfree_sampler::grain_presets;
use mpfree_sampler::model::{
    GrainFilter, GrainFilterKind, GrainMode, GrainParams, GrainQuality, GrainSprayShape,
    GrainWindow, LfoShape, MotionTarget, PlayMode, Region, RouteFrom, TempoSync,
    GRAIN_CUTOFF_MAX_HZ, GRAIN_CUTOFF_MIN_HZ, GRAIN_FILTER_SPRAY_OCTAVES, GRAIN_ROUTES,
};

use crate::panel::{Control, Look};
use crate::wave_tab::mode_label;

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
    /// The Grain tool's Mod part (sampler refactor step 17c): what moves
    /// the route outlined, and what it moves ([`crate::grain_tool`]).
    RouteFrom,
    RouteTo,
    /// Routes 1 to 4's depths.
    Route1,
    Route2,
    Route3,
    Route4,
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
    /// The grain setting the row turns as a parameter, for MIDI Learn
    /// (sampler refactor step 17): the rows with a range. None for a
    /// choice, a switch or a button.
    pub fn field(self) -> Option<GrainField> {
        Some(match self {
            Row::Position => GrainField::Position,
            Row::Spray => GrainField::Spray,
            Row::Size => GrainField::Size,
            Row::Density => GrainField::Density,
            Row::Pitch => GrainField::Pitch,
            Row::PitchSpray => GrainField::PitchSpray,
            Row::Scan => GrainField::Scan,
            Row::Reverse => GrainField::Reverse,
            Row::Spread => GrainField::Spread,
            Row::Level => GrainField::Level,
            Row::Timing => GrainField::Timing,
            Row::SizeSpray => GrainField::SizeSpray,
            Row::LevelSpray => GrainField::LevelSpray,
            Row::Cutoff => GrainField::Cutoff,
            Row::Resonance => GrainField::Resonance,
            Row::FilterSpray => GrainField::FilterSpray,
            Row::LfoRate => GrainField::LfoRate,
            Row::LfoDepth => GrainField::LfoDepth,
            Row::EnvDepth => GrainField::EnvDepth,
            Row::EnvAttack => GrainField::EnvAttack,
            Row::EnvDecay => GrainField::EnvDecay,
            Row::EnvSustain => GrainField::EnvSustain,
            Row::Mode
            | Row::GrainMode
            | Row::Preset
            | Row::PresetMore
            | Row::SprayShape
            | Row::Shape
            | Row::Quantise
            | Row::Sync
            | Row::Filter
            | Row::KeySize
            | Row::KeyScan
            | Row::Glide
            | Row::Quality
            | Row::LfoTo
            | Row::LfoShape
            | Row::LfoSync
            | Row::EnvTo
            | Row::RouteFrom
            | Row::RouteTo
            | Row::Route1
            | Row::Route2
            | Row::Route3
            | Row::Route4 => return None,
        })
    }

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
            Row::RouteFrom => "Route from",
            Row::RouteTo => "Route to",
            Row::Route1 => "Route 1",
            Row::Route2 => "Route 2",
            Row::Route3 => "Route 3",
            Row::Route4 => "Route 4",
        }
    }

    /// The Mod part's depth knobs, routes 1 to 4.
    pub const ROUTES: [Row; GRAIN_ROUTES] = [Row::Route1, Row::Route2, Row::Route3, Row::Route4];

    /// The route whose depth the row turns (from 0).
    pub fn route(self) -> Option<usize> {
        Row::ROUTES.iter().position(|r| *r == self)
    }

    /// Its name on the screen (over its knob, on its Q-Link card, in the
    /// Sound section): the label, shorter where a knob is too narrow for
    /// it, and saying it is the grains' where the pad's own setting of
    /// that name is on the screen too (the Sound section's filter,
    /// envelope, LFO and Pitch: Jay, 12 Nov, never two look-alike settings
    /// on one screen that do different things). The log keeps
    /// [`Row::label`].
    pub fn short(self) -> &'static str {
        match self {
            Row::Scan => "Scan",
            Row::Reverse => "Reverse",
            Row::Spread => "Stereo",
            Row::Pitch => "Grain pitch",
            Row::Filter => "Pass",
            Row::Cutoff => "Grain cutoff",
            Row::Resonance => "Grain res",
            Row::LfoTo => "Grain LFO to",
            Row::LfoShape => "Wave",
            Row::LfoRate => "Rate",
            Row::LfoDepth | Row::EnvDepth => "Depth",
            Row::LfoSync => "Sync",
            Row::EnvTo => "Grain env to",
            Row::EnvAttack => "Grain attack",
            Row::EnvDecay => "Grain decay",
            Row::EnvSustain => "Grain sustain",
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
pub const PRESETS_A_LINE: usize = 4;

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

/// How row `row` of `r` looks on a panel: a knob, a switch, the presets'
/// buttons, the filter's passes, or ‹ value › (the Sample page's Grain
/// tool lays the rows out its own way).
pub fn control_of(r: &Region, row: Row) -> Control {
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
    match &row {
        Row::RouteFrom | Row::RouteTo => Control::new(Look::Chip),
        Row::Route1 | Row::Route2 | Row::Route3 | Row::Route4 => {
            let route = r.grain_routes.0[row.route().unwrap_or(0)];
            knob((route.depth + 1.0) / 2.0, ROUTE_STEPS, true)
                .faint_because(route.from == RouteFrom::Off, ROUTE_OFF_WHY)
        }
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
        Row::Preset | Row::PresetMore => presets(row),
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
    }
}

/// Which of a line of preset buttons is lit: the preset the layer
/// matches, if it is on `row`'s line; else one past the line's end (none).
pub fn preset_on(r: &Region, row: Row) -> usize {
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
    if matches!(row, Row::RouteFrom | Row::RouteTo) || row.route().is_some() {
        return route_stepped(r, row, 0, delta);
    }
    let mut r = r.clone();
    let g = &mut r.grain;
    let d = delta as f32;
    let (dsync, ssync) = synced(g);
    match row {
        // Stepped above.
        Row::RouteFrom | Row::RouteTo | Row::Route1 | Row::Route2 | Row::Route3 | Row::Route4 => {}
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
pub fn is_switch(row: Row) -> bool {
    matches!(row, Row::KeySize | Row::KeyScan | Row::LfoSync)
}

/// Whether switch `row` is on in `r`.
pub fn switch_on(r: &Region, row: Row) -> bool {
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
pub fn list_place(list: &[f32], v: Option<f32>) -> f32 {
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

/// Where the size sits on its knob, 0 to 1.
pub fn size_y(g: &GrainParams) -> f32 {
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

// ------------------------------------------------------------ routes (17c)

/// A route's depth knob: 5 % a step, from -100 % to +100 %.
const ROUTE_STEPS: u32 = 40;
/// What a tap on a route's knob says while its From is Off.
pub const ROUTE_OFF_WHY: &str = "Pick what moves it first (Route from)";

/// What moves a route, as its picker names it.
pub fn route_from_name(f: RouteFrom) -> &'static str {
    match f {
        RouteFrom::Off => "Off",
        RouteFrom::Velocity => "Velocity",
        RouteFrom::Pressure => "Pressure",
        RouteFrom::Key => "Key",
    }
}

/// A grain setting as a route's To picker names it: the Grain tool's
/// own words, without "Grain" (every one here is the grains').
pub fn route_to_name(f: GrainField) -> &'static str {
    match f {
        GrainField::Position => "Position",
        GrainField::Spray => "Spray",
        GrainField::Size => "Size",
        GrainField::Density => "Density",
        GrainField::Pitch => "Pitch",
        GrainField::PitchSpray => "Pitch spray",
        GrainField::Scan => "Scan",
        GrainField::Reverse => "Reverse",
        GrainField::Spread => "Stereo",
        GrainField::Level => "Level",
        GrainField::Timing => "Timing",
        GrainField::SizeSpray => "Size spray",
        GrainField::LevelSpray => "Level spray",
        GrainField::Cutoff => "Cutoff",
        GrainField::Resonance => "Resonance",
        GrainField::FilterSpray => "Filter spray",
        GrainField::LfoRate => "LFO rate",
        GrainField::LfoDepth => "LFO depth",
        GrainField::EnvDepth => "Env depth",
        GrainField::EnvAttack => "Env attack",
        GrainField::EnvDecay => "Env decay",
        GrainField::EnvSustain => "Env sustain",
    }
}

/// Route `k`'s knob's name: by what it routes ("Vel → size", as the Play
/// page names its own), or "Route 4" while it is off.
pub fn route_name(r: &Region, k: usize) -> String {
    let route = r.grain_routes.0[k];
    let from = match route.from {
        RouteFrom::Off => return format!("Route {}", k + 1),
        RouteFrom::Velocity => "Vel",
        RouteFrom::Pressure => "Press",
        RouteFrom::Key => "Key",
    };
    let to = route_to_name(route.to);
    // Lower case after the arrow, but for a word in capitals (LFO).
    let to = if to.starts_with("LFO") {
        to.to_string()
    } else {
        to.to_lowercase()
    };
    format!("{from} → {to}")
}

/// A Mod part row's value, with route `outlined` (from 0) the one the
/// pickers edit: its From and To, or a route's depth ("+40 %"; the key's
/// a semitone's worth an octave, "+25 %/oct"; "Off" while it is off).
pub fn route_text(r: &Region, row: Row, outlined: usize) -> String {
    let outlined = outlined.min(GRAIN_ROUTES - 1);
    match row {
        Row::RouteFrom => route_from_name(r.grain_routes.0[outlined].from).into(),
        Row::RouteTo => route_to_name(r.grain_routes.0[outlined].to).into(),
        _ => {
            let Some(k) = row.route() else {
                return String::new();
            };
            let route = r.grain_routes.0[k];
            let pct = percent(route.depth);
            let sign = if pct > 0 { "+" } else { "" };
            match route.from {
                RouteFrom::Off => "Off".into(),
                RouteFrom::Key => format!("{sign}{pct} %/oct"),
                _ => format!("{sign}{pct} %"),
            }
        }
    }
}

/// `r` with a Mod part row stepped `delta` steps, route `outlined` the
/// one the pickers edit: its From through Off, Velocity, Pressure and
/// Key, its To through the 22 grain settings (position first), or
/// a route's depth 5 % a step (nothing while its From is Off).
pub fn route_stepped(r: &Region, row: Row, outlined: usize, delta: i32) -> Region {
    let mut r = r.clone();
    let outlined = outlined.min(GRAIN_ROUTES - 1);
    match row {
        Row::RouteFrom => {
            let route = &mut r.grain_routes.0[outlined];
            let i = RouteFrom::ALL
                .iter()
                .position(|f| *f == route.from)
                .unwrap_or(0);
            route.from = RouteFrom::ALL[shift(i, delta, RouteFrom::ALL.len())];
        }
        Row::RouteTo => {
            let route = &mut r.grain_routes.0[outlined];
            let i = route.to as usize;
            route.to = GrainField::ALL[shift(i, delta, GrainField::COUNT)];
        }
        _ => {
            if let Some(k) = row.route() {
                let route = &mut r.grain_routes.0[k];
                if route.from != RouteFrom::Off {
                    let steps = (route.depth * 20.0).round() as i32 + delta;
                    route.depth = (steps.clamp(-20, 20) as f32) / 20.0;
                }
            }
        }
    }
    r
}

fn shift(i: usize, delta: i32, len: usize) -> usize {
    (i as i32 + delta).clamp(0, len as i32 - 1) as usize
}

fn round_to(x: f32, step: f32) -> f32 {
    (x / step).round() * step
}

/// Where a grain size sits on its knob's turn, 0 to 1 (log).
pub fn size_place(ms: f32) -> f32 {
    let (lo, hi) = SIZE_RANGE_MS;
    ((ms.clamp(lo, hi) / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
}

/// `r` with its grains `frames` long (of a sound at `rate`; the grain
/// window's edge dragged on the Sample page's wave), within 2 ms to 2 s,
/// to a millisecond; synced to the tempo `bpm`, the nearest beats.
pub fn with_size_frames(r: &Region, frames: u64, rate: u32, bpm: f64) -> Region {
    let mut r = r.clone();
    let ms = frames as f32 * 1000.0 / rate.max(1) as f32;
    match r.grain.sync.as_mut() {
        Some(t) if t.size_beats.is_some() => {
            let beats = ms / 1000.0 * (bpm.max(1.0) / 60.0) as f32;
            let near = BEATS
                .iter()
                .copied()
                .min_by(|a, b| {
                    (a.ln() - beats.max(1e-6).ln())
                        .abs()
                        .total_cmp(&(b.ln() - beats.max(1e-6).ln()).abs())
                })
                .unwrap_or(1.0);
            t.size_beats = Some(near);
        }
        _ => {
            let (lo, hi) = SIZE_RANGE_MS;
            r.grain.size_ms = ms.round().clamp(lo, hi);
        }
    }
    r
}

/// `r` with its grains starting at `p`, 0 to 1 across the region, to a
/// tenth of a percent.
pub fn with_position(r: &Region, p: f32) -> Region {
    let mut r = r.clone();
    r.grain.position = round_to(p, 0.001).clamp(0.0, 1.0);
    r
}

/// The grain size at `y` (0 to 1) of [`size_place`]'s scale, in whole ms.
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
pub fn panel_text(r: &Region, row: Row) -> String {
    match row {
        Row::Timing if (1..100).contains(&percent(r.grain.timing)) => {
            format!("{} %", percent(r.grain.timing))
        }
        _ => value_text(r, row),
    }
}

/// A row's value as the list shows it.
pub fn value_text(r: &Region, row: Row) -> String {
    if matches!(row, Row::RouteFrom | Row::RouteTo) || row.route().is_some() {
        return route_text(r, row, 0);
    }
    let g = &r.grain;
    match row {
        // Written above.
        Row::RouteFrom | Row::RouteTo | Row::Route1 | Row::Route2 | Row::Route3 | Row::Route4 => {
            String::new()
        }
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

pub fn percent(x: f32) -> i32 {
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
pub fn split_unit(text: &str) -> (String, String) {
    match text.rsplit_once(' ') {
        Some((v, u)) => (v.to_string(), u.to_string()),
        None => (text.to_string(), String::new()),
    }
}

/// What changed between two versions of a layer's region on this tab, for
/// the log: "size 80 ms -> 120 ms, scan 0.25× -> Frozen". Empty if nothing
/// did.
pub fn describe(a: &Region, b: &Region) -> String {
    // A route as the log says it: "off", or "velocity size +40 %".
    let route = |r: &Region, k: usize| {
        let x = r.grain_routes.0[k];
        if x.from == RouteFrom::Off {
            return "off".to_string();
        }
        let from = route_from_name(x.from).to_lowercase();
        let to = route_to_name(x.to).to_lowercase();
        format!("{from} {to} {}", route_text(r, Row::ROUTES[k], k))
    };
    LOG_ORDER
        .iter()
        .filter_map(|&row| {
            let (x, y) = (value_text(a, row), value_text(b, row));
            (x != y).then(|| format!("{} {x} -> {y}", row.label().to_lowercase()))
        })
        .chain((0..GRAIN_ROUTES).filter_map(|k| {
            let (x, y) = (route(a, k), route(b, k));
            (a.grain_routes.0[k] != b.grain_routes.0[k])
                .then(|| format!("route {} {x} -> {y}", k + 1))
        }))
        .collect::<Vec<_>>()
        .join(", ")
}
