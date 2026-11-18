//! The pad editor's Play tab: how a pad plays its sound.
//!
//! - **Section chips** (Layers, Amp, Filter, Pitch, Mod, Pad, [`Section`])
//!   split the pad's rows: a tap on one shows its picture across the page
//!   ([`Section::view`]: the layer map, the envelope, the filter beside its
//!   envelope, or the keyboard), its rows as a panel of controls under it
//!   ([`crate::panel`]), and gives Q-Links 1 to 4 its four rows.
//! - The **layer map** shows the pad's layers on a velocity axis, with a
//!   line at the last hit's velocity. Dragging a layer's left or right edge
//!   sets the velocities it plays; a tap picks the layer that the Layers
//!   section's rows edit and the envelope is drawn over. **Add layer**
//!   copies the picked layer onto the upper half of its velocities,
//!   **Remove layer** takes it out ([`PlayAction::AddLayer`],
//!   [`PlayAction::RemoveLayer`]): buttons on the Layers section's panel.
//! - The **amp envelope** is drawn over the sound it shapes: Attack, Hold,
//!   Decay, Sustain and Release as points to drag. Sustain sits where the
//!   sound ends; Release follows it. In the Filter section the same view
//!   shows the filter envelope.
//! - The **filter** shows its response on a log frequency axis, with a dot
//!   at the cutoff to drag sideways.
//! - The **panel** (the list, drawn as knobs, buttons, selectors, steppers
//!   and switches in groups) holds the open section's rows: the picked
//!   layer's sound, velocities, level, pan and tune; the pad's level, pan
//!   and envelope; the filter, its envelope's depth and times; tune, wow
//!   and flutter and the keys it plays; what velocity and pressure do;
//!   voices, mute group, chokes, links and latch. A touch on a control
//!   picks its row and changes it; the **wheel** steps the row picked.
//! - **Keys** make the pad a keygroup: it plays the notes from Low key to
//!   High key, a semitone higher for each note above Root key (the sound's
//!   own pitch, as heard, when Keys turn on). While the pad is a keygroup or
//!   the pads play notes, the Pitch and Pad sections show a **keyboard**
//!   across the page with every
//!   keygroup of the kit over the keys it plays: the pad picked has handles
//!   to drag its lowest and highest key and its root, a tap on another zone
//!   picks that pad ([`PlayAction::Pick`]), and keys pressed on it play the
//!   kit as notes ([`PlayAction::Note`]); the 16 pads' notes are marked
//!   under the keys. **Pads play** is the kit's:
//!   each pad its own sound; notes, pad 1 the note in **Pads from** and
//!   each pad a semitone up, played by the keygroups that hold them; or
//!   **16 Levels** of the pad, each pad a step of what **Levels by** says
//!   (velocity, tune, cutoff, decay or start).
//! - The **scenes**, beside the panel on every section, hold the kit's scenes
//!   (`mpfree_sampler::scene`): **Store A** and **Store B** keep the pads
//!   as they are, the **crossfader** morphs every pad's level, pan, tune,
//!   filter and envelope times from A to B while it plays (the kit itself
//!   doesn't change), and **Return to A** puts the pads back as A stored
//!   them, as one undo step, with the crossfader at A
//!   ([`PlayAction::Scene`]).
//! - **Q-Links 1 to 4** turn the open section's four rows
//!   ([`Section::qlinks`]; with SHIFT, finer). A tap on a Q-Link's card
//!   picks its row.
//!
//! Every change is a [`PlayAction::Pad`] (or, for the kit's rows,
//! [`PlayAction::PadsPlay`]) for the app to apply ([`PlayAction::apply`]);
//! a drag, a turn or a run of taps on one row is one undo step, until the
//! app calls [`PlayTab::lift`]. The tab changes a layer's region only when
//! its Sound row picks another sound ([`PlayAction::LayerSound`], the whole
//! sound with the old region's gain and tune); the rest is the Wave tab's.

use std::sync::Arc;

use mpfree_analysis::note_name;
use mpfree_sampler::model::{
    Ahdsr, Filter, FilterKind, Keys, LayerSelect, Levels, ModelError, Pad, PadsPlay, PlayMode,
    PressureTarget, Project, Source, SourceId, KEYGROUP_POLYPHONY, MAX_KEY_FADE, MAX_LAYERS,
    MAX_VEL_FADE, PADS,
};
use mpfree_ui::{Color, EventKind, Ui, UiEvent};

use crate::chop_tab::pad_text;
use crate::keys_widgets::{KeyZones, Zone};
use crate::pads_tab::{sound_region, Sound};
use crate::panel::{self, Control, Item, Look, Panel};
use crate::play_widgets::{EnvelopeView, FilterCurve, LayerBar, LayerMap};
use crate::source::SourceView;
use crate::wave_tab::{self, duration_text, mode_label, pad_color, TABS};
use crate::widgets::{KnobCard, PadBar, SectionChip, SectionChips, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-play-bar";
pub const LAYERS: &str = "smp-play-layers";
pub const ENV: &str = "smp-play-env";
pub const FILTER: &str = "smp-play-filter";
pub const LIST: &str = "smp-play-list";
pub const KEYS: &str = "smp-play-keys";
pub const QLINKS: [&str; 4] = ["smp-play-q1", "smp-play-q2", "smp-play-q3", "smp-play-q4"];
/// The scenes, beside the section's controls: Store A, Store B and
/// Return to A, and the crossfader (a panel of its own, [`SCENE_ROWS`]).
pub const SCENES: &str = "smp-play-scenes";
/// The section chips over the page: Layers, Amp, Filter, Pitch, Mod, Pad.
pub const SECTIONS: &str = "smp-play-sections";
/// The scenes' rows: Store A, Store B, Return to A (buttons, whose taps
/// are [`EventKind::Tab`] with the row) and the crossfader
/// ([`EventKind::Value`]).
pub const SCENE_ROWS: usize = 4;

/// The tab's page with the Layers section open: the section chips, the
/// open section's picture across the page, and under it the section's
/// controls beside the scenes, between the pad bar and the Q-Link cards.
/// Each section has its own picture ([`layout`]).
pub const LAYOUT: &str = "col(header:60, row(smp-play-bar:*, smp-send:330):60, \
                          row(col(smp-play-sections:56, smp-play-layers:*, \
                          row(smp-play-list:*, smp-play-scenes:356):242):*, \
                          col(smp-play-q1, smp-play-q2, smp-play-q3, smp-play-q4):128):*, hint:26)";

/// The page with section `s` open, its picture [`Section::view`] (the
/// keyboard on Pitch and Pad when `keys`). A layout file can set it under
/// the page's name with the section's after it (`.amp`, `.filter`), and
/// `.keys` after that for the keyboard (`.pitch.keys`).
pub fn layout(s: Section, keys: bool) -> String {
    LAYOUT.replace("smp-play-layers:*", &format!("{}:*", s.view(keys)))
}

/// The velocity a key pressed on the keyboard plays at.
pub const KEY_VELOCITY: u8 = 100;

/// The tab's place in [`TABS`].
pub const TAB: usize = 3;

/// The hint line the app shows under the tab.
pub const HINT: &str = "Tap a section for its controls · drag a layer's edges for its velocities · drag the envelope's points and the \
                        filter's dot · the wheel picks a control, - / + change it · SHIFT + Q-Link: finer";

/// The hint line while the keyboard shows.
pub const KEYS_HINT: &str =
    "Drag the pad's zone edges and root dot · tap a zone to pick its pad · press keys to hear \
                             them · the wheel picks a control, - / + change it";

/// The highest note pad 1 may play, so that pad 16 is a MIDI note.
const MAX_PADS_FROM: u8 = 127 - 15;
/// Voices a pad may hold, as the arrows step them.
const POLYPHONY: [u8; 10] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32];
/// The Vel fade row's steps, up to [`MAX_VEL_FADE`].
const VEL_FADES: [u8; 11] = [0, 2, 4, 6, 8, 12, 16, 24, 32, 48, 64];
const _: () = assert!(VEL_FADES[VEL_FADES.len() - 1] == MAX_VEL_FADE);
/// Envelope times the arrows step through, in ms.
const TIMES_MS: [f32; 15] = [
    0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0,
    20_000.0,
];
/// The longest envelope time.
pub const MAX_MS: f32 = 20_000.0;
/// A release this short at least, so a voice never stops with a click.
const MIN_RELEASE_MS: f32 = 1.0;
const LEVEL_RANGE_DB: (f32, f32) = (-48.0, 12.0);
const LEVEL_STEP_DB: f32 = 0.5;
const TUNE_LIMIT_CENTS: i32 = 4800;
const CUTOFF_RANGE_HZ: (f32, f32) = (20.0, 20_000.0);
const ENV_OCTAVES: f32 = 4.0;
/// Q-Link detents for a doubling of an envelope time.
const TIME_DETENTS: f32 = 8.0;
/// Gesture ids the tab hands out start here, clear of other screens'.
const GESTURE_BASE: u64 = 0x504C_4159_0000_0000;
/// Layer colours on the map, in layer order.
const LAYER_COLORS: [u32; 8] = [
    0x6EA8FE, 0x3DDC97, 0xFF8A3D, 0xFFC145, 0xC77DFF, 0xFF5C8A, 0x4DD4E8, 0xA3E048,
];

/// The tab's rows. Each sits in one [`Section`], which lists them in
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Polyphony,
    MuteGroup,
    /// The pads a hit on this one stops (one-way chokes).
    Chokes,
    /// The pads a hit on this one plays too.
    Links,
    /// Whether a hit starts the pad and the next lets it go.
    Latch,
    /// How layers whose velocities match a hit are picked.
    Layers,
    Level,
    Pan,
    Tune,
    /// How much the pitch wanders, as on worn tape.
    Wow,
    /// Whether the pad plays pitched up as old samplers did, aliasing and
    /// all, instead of clean.
    Vintage,
    /// Whether the pad is a keygroup.
    Keys,
    LowKey,
    HighKey,
    /// The note the pad plays its sound as recorded.
    RootKey,
    /// The kit's: the notes over which its keygroups fade, the zones that
    /// meet overlapping by as much, so they crossfade.
    KeyFade,
    /// The kit's: what the pads play, their sounds or notes.
    PadsPlay,
    /// The kit's: the note pad 1 plays, when the pads play notes.
    PadsFrom,
    /// The kit's: what 16 Levels steps through.
    LevelsBy,
    VelToLevel,
    VelToStart,
    /// The velocities over which each layer fades in and out at its edges.
    VelFade,
    Pressure,
    Depth,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Filter,
    Cutoff,
    Resonance,
    /// How far the filter envelope moves the cutoff, in octaves.
    FilterEnv,
    /// The layer picked's: its sound, one of the kit's.
    Sound,
    /// The layer picked's: the velocities it plays.
    VelLow,
    VelHigh,
    /// The layer picked's own level, pan and tune, on top of the pad's.
    LayerLevel,
    LayerPan,
    LayerTune,
    /// The filter envelope's times and sustain (its hold is dragged on the
    /// picture).
    EnvAttack,
    EnvDecay,
    EnvSustain,
    EnvRelease,
    /// The layer picked copied onto the upper half of its velocities
    /// ([`PlayAction::AddLayer`]): a button.
    AddLayer,
    /// The layer picked taken out ([`PlayAction::RemoveLayer`]): a button.
    RemoveLayer,
}

impl Row {
    pub const ALL: [Row; 45] = [
        Row::Polyphony,
        Row::MuteGroup,
        Row::Chokes,
        Row::Links,
        Row::Latch,
        Row::Layers,
        Row::Level,
        Row::Pan,
        Row::Tune,
        Row::Wow,
        Row::Vintage,
        Row::Keys,
        Row::LowKey,
        Row::HighKey,
        Row::RootKey,
        Row::KeyFade,
        Row::PadsPlay,
        Row::PadsFrom,
        Row::LevelsBy,
        Row::VelToLevel,
        Row::VelToStart,
        Row::VelFade,
        Row::Pressure,
        Row::Depth,
        Row::Attack,
        Row::Hold,
        Row::Decay,
        Row::Sustain,
        Row::Release,
        Row::Filter,
        Row::Cutoff,
        Row::Resonance,
        Row::FilterEnv,
        Row::Sound,
        Row::VelLow,
        Row::VelHigh,
        Row::LayerLevel,
        Row::LayerPan,
        Row::LayerTune,
        Row::EnvAttack,
        Row::EnvDecay,
        Row::EnvSustain,
        Row::EnvRelease,
        Row::AddLayer,
        Row::RemoveLayer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Polyphony => "Polyphony",
            Row::MuteGroup => "Mute group",
            Row::Chokes => "Stops",
            Row::Links => "Also plays",
            Row::Latch => "Latch",
            Row::Layers => "Layers",
            Row::Level => "Level",
            Row::Pan => "Pan",
            Row::Tune => "Tune",
            Row::Wow => "Wow & flutter",
            Row::Vintage => "Vintage",
            Row::Keys => "Keys",
            Row::LowKey => "Low key",
            Row::HighKey => "High key",
            Row::RootKey => "Root key",
            Row::KeyFade => "Key fade",
            Row::PadsPlay => "Pads play",
            Row::PadsFrom => "Pads from",
            Row::LevelsBy => "Levels by",
            Row::VelToLevel => "Vel → level",
            Row::VelToStart => "Vel → start",
            Row::VelFade => "Vel fade",
            Row::Pressure => "Pressure",
            Row::Depth => "Depth",
            Row::Attack => "Attack",
            Row::Hold => "Hold",
            Row::Decay => "Decay",
            Row::Sustain => "Sustain",
            Row::Release => "Release",
            Row::Filter => "Filter",
            Row::Cutoff => "Cutoff",
            Row::Resonance => "Resonance",
            Row::FilterEnv => "Env depth",
            Row::Sound => "Sound",
            Row::VelLow => "Vel low",
            Row::VelHigh => "Vel high",
            Row::LayerLevel => "Level",
            Row::LayerPan => "Pan",
            Row::LayerTune => "Tune",
            Row::EnvAttack => "Env attack",
            Row::EnvDecay => "Env decay",
            Row::EnvSustain => "Env sustain",
            Row::EnvRelease => "Env release",
            Row::AddLayer => "Add layer",
            Row::RemoveLayer => "Remove layer",
        }
    }

    /// The section the row is in.
    pub fn section(self) -> Section {
        Section::ALL
            .into_iter()
            .find(|s| s.rows().contains(&self))
            .unwrap_or(Section::Pad)
    }

    /// The row's place in its section's list.
    pub fn place(self) -> usize {
        self.section()
            .rows()
            .iter()
            .position(|r| *r == self)
            .unwrap_or(0)
    }

    /// Whether the row is the layer picked's, not the whole pad's.
    pub fn of_layer(self) -> bool {
        matches!(
            self,
            Row::Sound
                | Row::VelLow
                | Row::VelHigh
                | Row::LayerLevel
                | Row::LayerPan
                | Row::LayerTune
        )
    }

    /// Whether the row is the kit's, not the pad's.
    pub fn of_kit(self) -> bool {
        matches!(
            self,
            Row::PadsPlay | Row::PadsFrom | Row::LevelsBy | Row::KeyFade
        )
    }
}

/// The tab's sections: a chip each over the page, each with its own rows
/// (eight at most, so its list never scrolls) and its own four Q-Links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// How a hit picks layers, and the layer picked: its sound, velocities,
    /// level, pan and tune.
    Layers,
    /// The pad's level and pan, and the amp envelope.
    Amp,
    /// The filter and its envelope.
    Filter,
    /// Tune, wow and flutter, Vintage, and the keys a keygroup plays.
    Pitch,
    /// What velocity and pressure change.
    Mod,
    /// Voices, mute group, stops, also plays, latch, and what the kit's
    /// pads play.
    Pad,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Layers,
        Section::Amp,
        Section::Filter,
        Section::Pitch,
        Section::Mod,
        Section::Pad,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Layers => "Layers",
            Section::Amp => "Amp",
            Section::Filter => "Filter",
            Section::Pitch => "Pitch",
            Section::Mod => "Mod",
            Section::Pad => "Pad",
        }
    }

    /// Its rows, in the list's order.
    pub fn rows(self) -> &'static [Row] {
        match self {
            Section::Layers => &[
                Row::Layers,
                Row::VelFade,
                Row::Sound,
                Row::VelLow,
                Row::VelHigh,
                Row::LayerLevel,
                Row::LayerPan,
                Row::LayerTune,
                Row::AddLayer,
                Row::RemoveLayer,
            ],
            Section::Amp => &[
                Row::Level,
                Row::Pan,
                Row::Attack,
                Row::Hold,
                Row::Decay,
                Row::Sustain,
                Row::Release,
            ],
            Section::Filter => &[
                Row::Filter,
                Row::Cutoff,
                Row::Resonance,
                Row::FilterEnv,
                Row::EnvAttack,
                Row::EnvDecay,
                Row::EnvSustain,
                Row::EnvRelease,
            ],
            Section::Pitch => &[
                Row::Tune,
                Row::Wow,
                Row::Vintage,
                Row::Keys,
                Row::LowKey,
                Row::HighKey,
                Row::RootKey,
                Row::KeyFade,
            ],
            Section::Mod => &[Row::VelToLevel, Row::VelToStart, Row::Pressure, Row::Depth],
            Section::Pad => &[
                Row::Polyphony,
                Row::MuteGroup,
                Row::Chokes,
                Row::Links,
                Row::Latch,
                Row::PadsPlay,
                Row::PadsFrom,
                Row::LevelsBy,
            ],
        }
    }

    /// The rows Q-Links 1 to 4 turn while the section is open.
    pub fn qlinks(self) -> [Row; 4] {
        match self {
            Section::Layers => [Row::LayerLevel, Row::LayerPan, Row::LayerTune, Row::VelLow],
            Section::Amp => [Row::Attack, Row::Decay, Row::Sustain, Row::Release],
            Section::Filter => [Row::Cutoff, Row::Resonance, Row::FilterEnv, Row::EnvDecay],
            Section::Pitch => [Row::Tune, Row::Wow, Row::LowKey, Row::HighKey],
            Section::Mod => [Row::VelToLevel, Row::VelToStart, Row::Pressure, Row::Depth],
            Section::Pad => [Row::Polyphony, Row::MuteGroup, Row::PadsPlay, Row::PadsFrom],
        }
    }

    /// The picture the section shows across the page: the layer map
    /// (Layers, Mod, Pad), the envelope over the sound (Amp, Pitch), the
    /// filter beside its envelope (Filter), or the keyboard on Pitch and
    /// Pad while `keys` (the pad is a keygroup or the pads play notes).
    pub fn view(self, keys: bool) -> &'static str {
        match self {
            Section::Pitch | Section::Pad if keys => KEYS,
            Section::Layers | Section::Mod | Section::Pad => LAYERS,
            Section::Amp | Section::Pitch => ENV,
            Section::Filter => "row(smp-play-filter:*, smp-play-env:*)",
        }
    }
}

/// The ways a hit picks layers, as the Layers row steps through them.
const SELECTS: [LayerSelect; 3] = [LayerSelect::All, LayerSelect::Cycle, LayerSelect::Random];

/// What the kit's pads play, as the Pads play row steps through them.
const PADS_PLAY: [&str; 3] = ["Their sounds", "Notes", "16 Levels"];

/// The filter types the Filter row steps through.
const FILTERS: [(FilterKind, u8); 7] = [
    (FilterKind::Off, 2),
    (FilterKind::LowPass, 2),
    (FilterKind::LowPass, 4),
    (FilterKind::HighPass, 2),
    (FilterKind::HighPass, 4),
    (FilterKind::BandPass, 2),
    (FilterKind::BandPass, 4),
];

const PRESSURE: [PressureTarget; 8] = [
    PressureTarget::Off,
    PressureTarget::Level,
    PressureTarget::Pitch,
    PressureTarget::Filter,
    PressureTarget::GrainDensity,
    PressureTarget::GrainSize,
    PressureTarget::GrainPosition,
    PressureTarget::GrainSpray,
];

fn is_grain(t: PressureTarget) -> bool {
    matches!(
        t,
        PressureTarget::GrainDensity
            | PressureTarget::GrainSize
            | PressureTarget::GrainPosition
            | PressureTarget::GrainSpray
    )
}

/// One layer's sound.
#[derive(Clone)]
pub struct LayerSound {
    pub name: String,
    pub rate: u32,
    pub view: Arc<SourceView>,
}

/// One keygroup of the kit, for the keyboard.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyZone {
    pub pad: usize,
    pub keys: Keys,
    /// Its key fade, in notes.
    pub fade: u8,
}

/// Everything the tab shows: one pad, all its settings, each layer's
/// sound, what the kit's pads play and the kit's keygroups.
#[derive(Clone)]
pub struct PlayState {
    pub pad: usize,
    pub settings: Pad,
    /// Layer k's sound, in layer order.
    pub sounds: Vec<LayerSound>,
    pub pads_play: PadsPlay,
    /// Every pad of the kit that is a keygroup, in pad order.
    pub zones: Vec<KeyZone>,
    /// The key fade the kit's keygroups share (None when they differ).
    pub key_fade: Option<u8>,
    /// Whether the kit has scenes A and B stored.
    pub scenes: [bool; 2],
    /// The kit's sounds, for a layer's Sound row.
    pub kit_sounds: Vec<Sound>,
}

impl PlayState {
    /// Pad `pad`, if it has a sound. `view` gives what the screens know of a
    /// source (the app keeps them, so each is worked out once).
    pub fn from_project(
        p: &Project,
        pad: usize,
        mut view: impl FnMut(&Source) -> Arc<SourceView>,
    ) -> Option<PlayState> {
        let settings = p.pad(pad)?.clone();
        if settings.layers.is_empty() {
            return None;
        }
        let sounds = settings
            .layers
            .iter()
            .map(|l| {
                let s = p.source(l.region.source)?;
                Some(LayerSound {
                    name: s.name.clone(),
                    rate: s.rate(),
                    view: view(s),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let zones = (0..mpfree_sampler::model::PADS)
            .filter_map(|pad| {
                let q = p.pad(pad)?;
                Some(KeyZone {
                    pad,
                    keys: q.keys?,
                    fade: q.key_fade,
                })
            })
            .collect();
        Some(PlayState {
            pad,
            settings,
            sounds,
            pads_play: p.pads_play(),
            zones,
            key_fade: p.key_fade(),
            scenes: [p.scenes[0].is_some(), p.scenes[1].is_some()],
            kit_sounds: p
                .sources()
                .iter()
                .map(|s| Sound {
                    id: s.id(),
                    name: s.name.clone(),
                    frames: s.frames(),
                    rate: s.rate(),
                    pads: p.users_of(s.id()),
                })
                .collect(),
        })
    }

    /// Whether the tab shows the keyboard: the pad is a keygroup, or the
    /// kit's pads play notes.
    pub fn shows_keys(&self) -> bool {
        self.settings.keys.is_some() || matches!(self.pads_play, PadsPlay::Keys { .. })
    }

    /// The first layer's sound's own note, as heard, if it has a clear
    /// pitch.
    pub fn heard_root(&self) -> Option<u8> {
        self.sounds.first().and_then(|s| s.view.root_note)
    }

    /// A row's value as the list shows it, the kit's rows too; a layer's
    /// rows show the first layer's (see [`PlayState::layer_text`]).
    pub fn value_text(&self, row: Row) -> String {
        if row.of_layer() {
            return self.layer_text(0, row);
        }
        match (row, self.pads_play) {
            (Row::PadsPlay, PadsPlay::Pads) => "Their sounds".into(),
            (Row::PadsPlay, PadsPlay::Keys { .. }) => "Notes".into(),
            (Row::PadsPlay, PadsPlay::Levels { .. }) => "16 Levels".into(),
            (Row::PadsFrom, PadsPlay::Keys { from }) => note_name(from),
            (Row::LevelsBy, PadsPlay::Levels { by, .. }) => by.label().into(),
            (Row::PadsFrom | Row::LevelsBy, _) => "–".into(),
            (Row::KeyFade, _) => match (!self.zones.is_empty(), self.key_fade) {
                (false, _) => "–".into(),
                (true, None) => "Mixed".into(),
                (true, Some(0)) => "Off".into(),
                (true, Some(1)) => "1 note".into(),
                (true, Some(n)) => format!("{n} notes"),
            },
            _ => value_text(&self.settings, row),
        }
    }

    /// A row's value as the list shows it with layer `layer` picked: its
    /// sound, velocities, level, pan and tune; other rows as
    /// [`PlayState::value_text`].
    pub fn layer_text(&self, layer: usize, row: Row) -> String {
        if !row.of_layer() {
            return self.value_text(row);
        }
        let Some(l) = self.settings.layers.get(layer) else {
            return "–".into();
        };
        match row {
            Row::Sound => self
                .sounds
                .get(layer)
                .map_or_else(|| "–".into(), |s| s.name.clone()),
            Row::VelLow => l.vel_lo.to_string(),
            Row::VelHigh => l.vel_hi.to_string(),
            Row::LayerLevel => format!("{:+.1} dB", l.level_db),
            Row::LayerPan => pan_text(l.pan),
            _ => tune_text(l.tune_cents),
        }
    }

    /// What the pads play after the kit's row `row` is stepped by
    /// `delta`: Pads play goes round their sounds, notes (from the octave
    /// of the pad's root, or C3) and 16 Levels of this pad (by velocity);
    /// Pads from moves an octave a step; Levels by goes round what 16
    /// Levels can step through.
    pub fn pads_play_stepped(&self, row: Row, delta: i32) -> PadsPlay {
        match (row, self.pads_play) {
            (_, now) if delta == 0 => now,
            (Row::PadsPlay, now) => {
                let at = match now {
                    PadsPlay::Pads => 0,
                    PadsPlay::Keys { .. } => 1,
                    PadsPlay::Levels { .. } => 2,
                };
                match (at + delta).rem_euclid(3) {
                    0 => PadsPlay::Pads,
                    1 => {
                        let root = self.settings.keys.map_or(48, |k| k.root);
                        PadsPlay::Keys {
                            from: (root - root % 12).min(MAX_PADS_FROM - MAX_PADS_FROM % 12),
                        }
                    }
                    _ => PadsPlay::Levels {
                        pad: self.pad.min(usize::from(u8::MAX)) as u8,
                        by: Levels::default(),
                    },
                }
            }
            (Row::LevelsBy, PadsPlay::Levels { pad, by }) => {
                let n = Levels::ALL.len() as i32;
                let i = Levels::ALL.iter().position(|l| *l == by).unwrap_or(0) as i32;
                PadsPlay::Levels {
                    pad,
                    by: Levels::ALL[(i + delta).rem_euclid(n) as usize],
                }
            }
            (Row::PadsFrom, PadsPlay::Keys { from }) => {
                let (lo, at) = (i32::from(from % 12), i32::from(from));
                let hi = lo + 12 * ((i32::from(MAX_PADS_FROM) - lo) / 12);
                PadsPlay::Keys {
                    from: (at + 12 * delta).clamp(lo, hi) as u8,
                }
            }
            (_, now) => now,
        }
    }

    pub fn granular(&self) -> bool {
        self.settings
            .layers
            .iter()
            .any(|l| l.region.mode == PlayMode::Granular)
    }
}

/// What a touch or control on the tab asks for. The app applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayAction {
    /// Pad `pad`'s settings, changed. `gesture` is the same for every
    /// change of one drag, turn or run of taps on a row, so they merge into
    /// one undo step.
    Pad {
        pad: usize,
        after: Box<Pad>,
        gesture: u64,
    },
    /// What the kit's pads play, changed (one undo step).
    PadsPlay(PadsPlay),
    /// The kit's key fade, set: every keygroup fades over this many notes
    /// and the zones that meet overlap by it (one undo step).
    KeyFade(u8),
    /// Pad `pad` picked (a tap on its zone on the keyboard): every tab
    /// edits it from now on.
    Pick(usize),
    /// A key on the keyboard pressed (`down`) or let go: play the kit's
    /// keygroups at `note`, at [`KEY_VELOCITY`], as a keyboard would.
    Note { note: u8, down: bool },
    /// The scene strip: see [`SceneAction`]. The app keeps the scenes in
    /// the kit and sends the crossfader's values to the sampler.
    Scene(SceneAction),
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
    /// A layer added to pad `pad` just after layer `after`: a copy of it
    /// that takes the upper half of its velocities (one undo step).
    AddLayer { pad: usize, after: usize },
    /// Layer `layer` of pad `pad` taken out (one undo step). A pad keeps
    /// one layer at least.
    RemoveLayer { pad: usize, layer: usize },
    /// Layer `layer` of pad `pad` plays the kit's sound `source` from start
    /// to end, keeping its play mode, gain, tune and channels (one undo
    /// step).
    LayerSound {
        pad: usize,
        layer: usize,
        source: SourceId,
    },
}

/// What the scene strip asks for (`mpfree_sampler::scene`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneAction {
    /// Store the kit's pads as they are as scene A (0) or B (1).
    Store(usize),
    /// The crossfader moved: 0 is A, 1 is B.
    Fade(f32),
    /// Back to scene A: the pads as it stored them (one undo step), and
    /// the crossfader to A.
    Return,
}

impl PlayAction {
    /// Applies a change to the project (one undo step per gesture), keeping
    /// the project's layer regions. Other actions change nothing here.
    pub fn apply(&self, p: &mut Project) -> Result<(), ModelError> {
        let (pad, after, gesture) = match self {
            PlayAction::Pad {
                pad,
                after,
                gesture,
            } => (pad, after, gesture),
            PlayAction::PadsPlay(play) => return p.set_pads_play(*play),
            PlayAction::KeyFade(fade) => return p.set_key_fade(*fade),
            PlayAction::AddLayer { pad, after } => {
                let now = p.pad(*pad).ok_or(ModelError::BadPad)?;
                if now.layers.len() >= MAX_LAYERS {
                    return Err(ModelError::TooManyLayers);
                }
                let next = with_layer_added(now, *after).ok_or(ModelError::BadPad)?;
                return p.set_pad(*pad, Some(next), None);
            }
            PlayAction::RemoveLayer { pad, layer } => {
                let now = p.pad(*pad).ok_or(ModelError::BadPad)?;
                if now.layers.len() < 2 || *layer >= now.layers.len() {
                    return Err(ModelError::BadPad);
                }
                let mut next = now.clone();
                next.layers.remove(*layer);
                return p.set_pad(*pad, Some(next), None);
            }
            PlayAction::LayerSound { pad, layer, source } => {
                let frames = p.source(*source).ok_or(ModelError::UnknownSource)?.frames();
                let mut next = p.pad(*pad).ok_or(ModelError::BadPad)?.clone();
                let l = next.layers.get_mut(*layer).ok_or(ModelError::BadPad)?;
                l.region = sound_region(Some(&l.region), *source, frames);
                return p.set_pad(*pad, Some(next), None);
            }
            PlayAction::Tab(_)
            | PlayAction::Pick(_)
            | PlayAction::Note { .. }
            | PlayAction::Scene(_) => return Ok(()),
        };
        let now = p.pad(*pad).ok_or(ModelError::BadPad)?;
        if now.layers.len() != after.layers.len() {
            return Err(ModelError::BadPad);
        }
        let mut next = (**after).clone();
        for (l, n) in next.layers.iter_mut().zip(&now.layers) {
            l.region = n.region.clone();
        }
        p.set_pad(*pad, Some(next), Some(*gesture))
    }
}

/// `p` with a copy of layer `after` just after it, the copy taking the
/// upper half of its velocities (both keep them all if it plays just one).
/// None if there is no such layer.
pub fn with_layer_added(p: &Pad, after: usize) -> Option<Pad> {
    let mut p = p.clone();
    let mut new = p.layers.get(after)?.clone();
    let (lo, hi) = (new.vel_lo, new.vel_hi);
    if hi > lo {
        let mid = lo + (hi - lo).div_ceil(2);
        p.layers[after].vel_hi = mid - 1;
        new.vel_lo = mid;
    }
    p.layers.insert(after + 1, new);
    Some(p)
}

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Row(Row),
    /// A layer's edge on the map (2k low, 2k + 1 high).
    Layer(usize),
    /// An envelope point.
    Env(usize),
    /// A point of the filter envelope.
    FilterEnv(usize),
    /// The filter's dot.
    Cutoff,
    /// The pad's zone on the keyboard: its lowest key (0), highest (1) or
    /// root (2).
    Zone(usize),
}

/// The Play tab's widgets and what their touches mean. It keeps the layer
/// picked, the last hit's velocity and the undo step being made; the pad's
/// settings come from the [`PlayState`].
pub struct PlayTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`wave_tab::WaveTab::bank`].
    pub bank: Option<char>,
    layer: usize,
    last_velocity: Option<u8>,
    shown_pad: Option<usize>,
    gesture: Option<(Key, u64)>,
    next_gesture: u64,
    /// Whether the page has the keyboard.
    keys_shown: bool,
    /// The section and keyboard the page is laid out for.
    laid_out: (Section, bool),
    /// The crossfader, 0 (A) to 1 (B).
    fade: f32,
    /// The section open.
    section: Section,
}

impl PlayTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &PlayState) -> Result<PlayTab, String> {
        crate::send::add(ui);
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        let mut map = LayerMap::new("Layers");
        map.right = "drag edges to set ranges".into();
        ui.add(LAYERS, map);
        let mut env = EnvelopeView::new("Amp envelope");
        env.right = "drag the points".into();
        ui.add(ENV, env);
        ui.add(FILTER, FilterCurve::new("Filter"));
        ui.add(KEYS, KeyZones::new("Keys"));
        let labels: Vec<&str> = Section::ALL.iter().map(|s| s.label()).collect();
        ui.add(SECTIONS, SectionChips::new(&labels));
        ui.add(LIST, SettingList::new("Layers"));
        let t = ui.theme.clone();
        let colors = [t.accent, t.good, t.warn, Color::rgb(0xC77DFF)];
        for (k, row) in Section::Layers.qlinks().iter().enumerate() {
            ui.add(
                QLINKS[k],
                KnobCard::new(&format!("Q-Link {} · {}", k + 1, row.label()), colors[k])
                    .on(k, Section::Layers.label()),
            );
        }
        let mut scenes = SettingList::new("Scenes");
        scenes.outlined = false;
        scenes.buttons = vec![0, 1, 2];
        ui.add(SCENES, scenes);
        ui.add_page(name, LAYOUT)?;
        let mut tab = PlayTab {
            name: name.to_string(),
            bank: None,
            layer: 0,
            last_velocity: None,
            shown_pad: None,
            gesture: None,
            next_gesture: GESTURE_BASE,
            keys_shown: false,
            laid_out: (Section::Layers, false),
            fade: 0.0,
            section: Section::Layers,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The section open.
    pub fn section(&self) -> Section {
        self.section
    }

    /// Opens section `section`, its first row picked and its picture
    /// across the page.
    pub fn open_section(&mut self, ui: &mut Ui, st: &PlayState, section: Section) {
        if section != self.section {
            self.section = section;
            self.lift();
            if let Some(list) = ui.get_mut::<SettingList>(LIST) {
                list.selected = 0;
                list.editing = false;
            }
        }
        self.lay_out(ui, st.shows_keys());
        self.show(ui, st);
    }

    /// The rows Q-Links 1 to 4 turn now, for the hint line: "attack,
    /// decay, sustain, release", or "layer level, pan, tune, vel low".
    pub fn qlinks_text(&self) -> String {
        let mut said_layer = false;
        let rows = self.section.qlinks().map(|r| {
            let label = r.label().to_lowercase();
            if r.of_layer() && !said_layer {
                said_layer = true;
                format!("layer {label}")
            } else {
                label
            }
        });
        rows.join(", ")
    }

    /// The layer picked on the map, from 0.
    pub fn layer(&self) -> usize {
        self.layer
    }

    /// The layer picked on the map while it showed pad `pad`; None if it
    /// shows another pad, or none yet. The other tabs edit this layer.
    pub fn layer_of(&self, pad: usize) -> Option<usize> {
        (self.shown_pad == Some(pad)).then_some(self.layer)
    }

    /// The row picked in the section's list.
    pub fn row(&self, ui: &Ui) -> Row {
        let rows = self.section.rows();
        ui.get::<SettingList>(LIST)
            .and_then(|l| rows.get(l.selected).copied())
            .unwrap_or(rows[0])
    }

    /// Ends the drag, turn or run of taps being made: the next edit is a
    /// new undo step. Call it when a finger lifts and when a Q-Link or the
    /// wheel rests.
    pub fn lift(&mut self) {
        self.gesture = None;
    }

    /// The pad was hit at `velocity`: the layer map draws a line there.
    pub fn set_velocity(&mut self, ui: &mut Ui, velocity: Option<u8>) {
        self.last_velocity = velocity;
        if let Some(m) = ui.get_mut::<LayerMap>(LAYERS) {
            m.last_velocity = velocity;
        }
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &PlayState) {
        if self.shown_pad != Some(st.pad) {
            self.shown_pad = Some(st.pad);
            self.layer = 0;
            self.last_velocity = None;
        }
        self.layer = self.layer.min(st.settings.layers.len().saturating_sub(1));
        self.lay_out(ui, st.shows_keys());
        self.show(ui, st);
    }

    /// Whether the page shows the keyboard.
    pub fn keys_shown(&self) -> bool {
        self.keys_shown
    }

    /// The hint line for the page as it is.
    pub fn hint(&self) -> &'static str {
        if self.keys_shown {
            KEYS_HINT
        } else {
            HINT
        }
    }

    /// Lays the page out for the section open: its picture across the
    /// page, the keyboard on Pitch and Pad while `keys` ([`layout`]).
    fn lay_out(&mut self, ui: &mut Ui, keys: bool) {
        let s = self.section;
        let keys = keys && matches!(s, Section::Pitch | Section::Pad);
        if self.laid_out == (s, keys) {
            return;
        }
        let mut key = format!("{}.{}", self.name, s.label().to_lowercase());
        if keys {
            key.push_str(".keys");
        }
        let spec = ui.layout_for(&key, &layout(s, keys));
        if ui.set_layout(&self.name, &spec).is_ok() {
            self.laid_out = (s, keys);
            self.keys_shown = keys;
        }
    }

    /// The crossfader's position, 0 (A) to 1 (B).
    pub fn fade(&self) -> f32 {
        self.fade
    }

    /// Puts the crossfader at `x` (the app's, after Return or a new kit).
    pub fn set_fade(&mut self, ui: &mut Ui, st: &PlayState, x: f32) {
        self.fade = if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.show_scenes(ui, st);
    }

    /// The scenes' panel: Store A and B, Return to A, and the crossfader,
    /// which moves once both are stored.
    fn show_scenes(&self, ui: &mut Ui, st: &PlayState) {
        let store = |k: usize| {
            let ab = ["A", "B"][k];
            if st.scenes[k] {
                format!("Store {ab} again")
            } else {
                format!("Store {ab}")
            }
        };
        let (fade, why) = match st.scenes {
            [true, true] => (
                match (self.fade * 100.0).round() as u32 {
                    0 => "A".to_string(),
                    100 => "B".to_string(),
                    n => format!("{n} % to B"),
                },
                None,
            ),
            [false, _] => ("A".to_string(), Some("Store A first")),
            [true, false] => ("A".to_string(), Some("Store B first")),
        };
        let Some(l) = ui.get_mut::<SettingList>(SCENES) else {
            return;
        };
        let button = |text: String| Setting {
            name: text,
            value: String::new(),
        };
        l.rows = vec![
            button(store(0)),
            button(store(1)),
            button("Return to A".into()),
            Setting {
                name: "A → B".into(),
                value: fade,
            },
        ];
        l.controls = vec![
            Control::new(Look::Button(store(0))),
            Control::new(Look::Button(store(1))),
            Control::new(Look::Button("Return to A".into()))
                .faint_because(!st.scenes[0], "Store A first"),
            Control::new(Look::Slider { pos: self.fade })
                .faint_because(why.is_some(), why.unwrap_or_default()),
        ];
        let it = Item::new;
        l.panel = Some(Panel {
            sections: vec![panel::Section::new(
                "Scenes · A to B",
                356,
                vec![vec![it(0), it(1), it(2)], vec![it(3).labelled()]],
            )
            .note(match st.scenes {
                [false, _] => "store A first",
                [true, false] => "store B first",
                _ => "the kit's pads",
            })],
        });
    }

    fn show(&self, ui: &mut Ui, st: &PlayState) {
        self.show_scenes(ui, st);
        let p = &st.settings;
        let n = p.layers.len();
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.pad);
            bar.active = TAB;
            bar.title = format!("Pad {} · {}", pad_text(self.bank, st.pad), p.name);
            let first = &p.layers[0].region;
            let mut about = vec![
                format!("{n} layer{}", if n == 1 { "" } else { "s" }),
                mode_label(first.mode).to_string(),
            ];
            if n > 1 {
                about.push(select_label(p.select).to_lowercase());
            }
            about.push(voices_text(p.polyphony));
            if let Some(s) = st.sounds.first() {
                about.push(format!("{} long", duration_text(first.frames(), s.rate)));
            }
            if let Some(k) = p.keys {
                about.push(format!(
                    "keys {}–{}, root {}",
                    note_name(k.lo),
                    note_name(k.hi),
                    note_name(k.root)
                ));
            }
            match st.pads_play {
                PadsPlay::Keys { from } => {
                    about.push(format!("pads play notes from {}", note_name(from)))
                }
                PadsPlay::Levels { pad, by } => about.push(format!(
                    "pads play 16 Levels of {} by {}",
                    pad_text(self.bank, usize::from(pad)),
                    by.label().to_lowercase()
                )),
                PadsPlay::Pads => {}
            }
            bar.subtitle = about.join(" · ");
        }
        if let Some(m) = ui.get_mut::<LayerMap>(LAYERS) {
            m.layers = p
                .layers
                .iter()
                .enumerate()
                .map(|(k, l)| {
                    let sound = st.sounds.get(k).map_or("", |s| s.name.as_str());
                    let pick = match p.select {
                        _ if n == 1 => "",
                        LayerSelect::All => " · stacked",
                        LayerSelect::Cycle => " · cycle",
                        LayerSelect::Random => " · random",
                    };
                    let mut own = String::new();
                    if l.level_db != 0.0 {
                        own.push_str(&format!(" · {:+.1} dB", l.level_db));
                    }
                    if l.pan != 0.0 {
                        own.push_str(&format!(" · {}", pan_text(l.pan)));
                    }
                    if l.tune_cents != 0 {
                        own.push_str(&format!(" · {}", tune_text(l.tune_cents)));
                    }
                    LayerBar {
                        name: format!("Layer {} · {sound}", k + 1),
                        sub: format!("velocity {}–{}{own}{pick}", l.vel_lo, l.vel_hi),
                        lo: l.vel_lo,
                        hi: l.vel_hi,
                        color: Color::rgb(LAYER_COLORS[k % LAYER_COLORS.len()]),
                    }
                })
                .collect();
            m.selected = self.layer;
            m.last_velocity = self.last_velocity;
            m.fade = p.vel_fade;
        }
        if let Some(e) = ui.get_mut::<EnvelopeView>(ENV) {
            // The Filter section shows the filter's envelope, every other
            // the amp's.
            let filter_env = self.section == Section::Filter;
            e.env = if filter_env { p.filter_env } else { p.amp };
            e.title = if filter_env {
                "Filter envelope"
            } else {
                "Amp envelope"
            }
            .into();
            let (l, s) = (&p.layers[self.layer], st.sounds.get(self.layer));
            e.set_sound(
                s.map(|s| s.view.peaks.clone()),
                l.region.start,
                l.region.end,
                s.map_or(44_100, |s| s.rate),
            );
            let a = &e.env;
            e.caption = format!(
                "A {} · H {} · D {} · S {} % · R {}",
                ms_text(a.attack_ms),
                ms_text(a.hold_ms),
                ms_text(a.decay_ms),
                percent(a.sustain),
                ms_text(a.release_ms)
            );
            if filter_env && p.filter.kind == FilterKind::Off {
                e.caption.push_str(" · the filter is off");
            }
            if n > 1 {
                e.caption
                    .push_str(&format!(" · over layer {}", self.layer + 1));
            }
        }
        if let Some(f) = ui.get_mut::<FilterCurve>(FILTER) {
            f.filter = p.filter;
            f.title = format!("Filter · {}", filter_label(&p.filter));
            if p.filter.kind == FilterKind::Off {
                f.right = String::new();
                f.caption = "Off: pick a type in the Filter row".into();
            } else {
                f.right = "drag the dot".into();
                f.caption = format!(
                    "cutoff {} · res {} % · env {}",
                    hz_text(p.filter.cutoff_hz),
                    percent(p.filter.resonance),
                    octaves_text(p.filter.env_octaves)
                );
            }
        }
        if let Some(z) = ui.get_mut::<KeyZones>(KEYS) {
            self.show_zones(z, st);
        }
        let rows = self
            .section
            .rows()
            .iter()
            .map(|r| Setting {
                name: r.label().into(),
                value: match r {
                    Row::Chokes => pads_text(self.bank, &p.chokes),
                    Row::Links => pads_text(self.bank, &p.links),
                    _ => st.layer_text(self.layer, *r),
                },
            })
            .collect();
        let sound = st.sounds.get(self.layer).map_or("", |s| s.name.as_str());
        let controls = self.controls(st);
        let panel = self.panel(st);
        let buttons = if self.section == Section::Layers {
            vec![Row::AddLayer.place(), Row::RemoveLayer.place()]
        } else {
            Vec::new()
        };
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
            list.controls = controls;
            list.panel = Some(panel);
            list.buttons = buttons;
            list.selected = list.selected.min(list.rows.len() - 1);
            (list.title, list.right) = match self.section {
                Section::Layers if n > 1 => (
                    format!("Layer {} · {sound}", self.layer + 1),
                    format!("of {n} · tap one on the map"),
                ),
                Section::Layers => ("Layer 1".into(), sound.to_string()),
                Section::Pad => ("Pad".into(), "and the kit's pads".into()),
                s => (s.label().into(), "the whole pad".into()),
            };
        }
        if let Some(c) = ui.get_mut::<SectionChips>(SECTIONS) {
            c.selected = Section::ALL
                .iter()
                .position(|s| *s == self.section)
                .unwrap_or(0);
            c.chips = Section::ALL
                .iter()
                .map(|s| SectionChip {
                    name: s.label().into(),
                    sub: section_text(st, *s),
                })
                .collect();
        }
        self.show_cards(ui, st);
    }

    /// How each of the open section's rows looks on the panel, in its
    /// list's order.
    fn controls(&self, st: &PlayState) -> Vec<Control> {
        let p = &st.settings;
        let n = p.layers.len();
        let source = p.layers.get(self.layer).map(|l| l.region.source);
        let times = TIMES_MS.len() as u32 - 1;
        let level = ((LEVEL_RANGE_DB.1 - LEVEL_RANGE_DB.0) / LEVEL_STEP_DB) as u32;
        let tune = (2 * TUNE_LIMIT_CENTS / 100) as u32;
        self.section
            .rows()
            .iter()
            .map(|&row| {
                let knob = |steps: u32, bipolar: bool| Look::Knob {
                    pos: knob_pos(st, self.layer, row),
                    steps,
                    bipolar,
                };
                let look = match row {
                    Row::Layers => Look::Choice {
                        options: SELECTS.iter().map(|s| select_label(*s).into()).collect(),
                        on: SELECTS.iter().position(|s| *s == p.select).unwrap_or(0),
                        off: Vec::new(),
                    },
                    Row::PadsPlay => Look::Choice {
                        options: PADS_PLAY.iter().map(|s| s.to_string()).collect(),
                        on: match st.pads_play {
                            PadsPlay::Pads => 0,
                            PadsPlay::Keys { .. } => 1,
                            PadsPlay::Levels { .. } => 2,
                        },
                        off: Vec::new(),
                    },
                    Row::Level | Row::LayerLevel => knob(level, false),
                    Row::Pan | Row::LayerPan | Row::Depth => knob(40, true),
                    Row::Tune | Row::LayerTune => knob(tune, true),
                    Row::Wow => knob(10, false),
                    Row::Attack
                    | Row::Hold
                    | Row::Decay
                    | Row::Release
                    | Row::EnvAttack
                    | Row::EnvDecay
                    | Row::EnvRelease => knob(times, false),
                    Row::Sustain
                    | Row::EnvSustain
                    | Row::Resonance
                    | Row::VelToLevel
                    | Row::VelToStart => knob(20, false),
                    // A third of an octave a step, 20 Hz to 20 kHz.
                    Row::Cutoff => knob(30, false),
                    Row::FilterEnv => knob((2.0 * ENV_OCTAVES / 0.25) as u32, true),
                    Row::Vintage => Look::Toggle(p.vintage),
                    Row::Keys => Look::Toggle(p.keys.is_some()),
                    Row::Latch => Look::Toggle(p.latch),
                    Row::VelLow
                    | Row::VelHigh
                    | Row::LowKey
                    | Row::HighKey
                    | Row::RootKey
                    | Row::KeyFade
                    | Row::PadsFrom => Look::Stepper,
                    Row::AddLayer | Row::RemoveLayer => Look::Button(row.label().into()),
                    Row::Sound
                    | Row::VelFade
                    | Row::Filter
                    | Row::Pressure
                    | Row::Polyphony
                    | Row::MuteGroup
                    | Row::Chokes
                    | Row::Links
                    | Row::LevelsBy => Look::Chip,
                };
                let why = match row {
                    Row::AddLayer if n >= MAX_LAYERS => {
                        Some(format!("{MAX_LAYERS} layers at most"))
                    }
                    Row::RemoveLayer if n < 2 => Some("A pad keeps one layer at least".into()),
                    Row::Sound if !st.kit_sounds.iter().any(|s| Some(s.id) != source) => {
                        Some("The kit has no other sound".into())
                    }
                    Row::KeyFade if st.zones.is_empty() => {
                        Some("No pad of the kit is a keygroup: turn Keys on".into())
                    }
                    _ => faint_why(st, row).map(str::to_string),
                };
                Control::new(look).faint_because(why.is_some(), why.as_deref().unwrap_or(""))
            })
            .collect()
    }

    /// The open section's panel: its rows as controls, in sections of
    /// their own.
    fn panel(&self, st: &PlayState) -> Panel {
        let p = &st.settings;
        let n = p.layers.len();
        let it = |r: Row| Item::new(r.place());
        let part = panel::Section::new;
        let sections = match self.section {
            Section::Layers => {
                let sound = st.sounds.get(self.layer).map_or("", |s| s.name.as_str());
                vec![
                    part(
                        "Layers",
                        250,
                        vec![vec![it(Row::Layers)], vec![it(Row::VelFade).labelled()]],
                    )
                    .note(if n > 1 {
                        "how a hit picks"
                    } else {
                        "one so far"
                    }),
                    part(
                        &format!("Layer {}", self.layer + 1),
                        500,
                        vec![
                            vec![
                                it(Row::Sound).labelled(),
                                it(Row::VelLow).labelled().wide(124),
                                it(Row::VelHigh).labelled().wide(124),
                            ],
                            vec![
                                it(Row::LayerLevel),
                                it(Row::LayerPan),
                                it(Row::LayerTune),
                                it(Row::AddLayer).wide(112),
                                it(Row::RemoveLayer).wide(112),
                            ],
                        ],
                    )
                    .note(&if n > 1 {
                        format!("of {n} · tap one on the map")
                    } else {
                        sound.to_string()
                    }),
                ]
            }
            Section::Amp => vec![
                part("Level", 220, vec![vec![it(Row::Level), it(Row::Pan)]]).note("the whole pad"),
                part(
                    "Envelope",
                    520,
                    vec![vec![
                        it(Row::Attack),
                        it(Row::Hold),
                        it(Row::Decay),
                        it(Row::Sustain),
                        it(Row::Release),
                    ]],
                )
                .note("drag the points, or turn"),
            ],
            Section::Filter => vec![
                part(
                    "Filter",
                    390,
                    vec![
                        vec![it(Row::Filter).labelled()],
                        vec![it(Row::Cutoff), it(Row::Resonance), it(Row::FilterEnv)],
                    ],
                )
                .note(if p.filter.kind == FilterKind::Off {
                    "off"
                } else {
                    "drag the dot"
                }),
                part(
                    "Filter envelope",
                    350,
                    vec![vec![
                        it(Row::EnvAttack),
                        it(Row::EnvDecay),
                        it(Row::EnvSustain),
                        it(Row::EnvRelease),
                    ]],
                )
                .note("its hold on the picture"),
            ],
            Section::Pitch => vec![
                part(
                    "Pitch",
                    300,
                    vec![vec![it(Row::Tune), it(Row::Wow)], vec![it(Row::Vintage)]],
                )
                .note("the whole pad"),
                part(
                    "Keys",
                    440,
                    vec![
                        vec![
                            it(Row::Keys).wide(150),
                            it(Row::KeyFade).labelled().wide(150),
                        ],
                        vec![
                            it(Row::LowKey).labelled(),
                            it(Row::HighKey).labelled(),
                            it(Row::RootKey).labelled(),
                        ],
                    ],
                )
                .note(if p.keys.is_some() {
                    "a keygroup"
                } else {
                    "on: the pad plays notes"
                }),
            ],
            Section::Mod => vec![
                part(
                    "Velocity",
                    300,
                    vec![vec![it(Row::VelToLevel), it(Row::VelToStart)]],
                )
                .note("how hard a hit is"),
                part(
                    "Pressure",
                    440,
                    vec![vec![it(Row::Pressure).labelled(), it(Row::Depth).wide(96)]],
                )
                .note("the pad held down"),
            ],
            Section::Pad => vec![
                part(
                    "Pad",
                    420,
                    vec![
                        vec![it(Row::Polyphony).labelled(), it(Row::MuteGroup).labelled()],
                        vec![it(Row::Chokes).labelled(), it(Row::Links).labelled()],
                        vec![it(Row::Latch)],
                    ],
                )
                .note("hit a pad to add it"),
                part(
                    "The kit's pads",
                    320,
                    vec![
                        vec![it(Row::PadsPlay)],
                        vec![it(Row::PadsFrom).labelled(), it(Row::LevelsBy).labelled()],
                    ],
                )
                .note("every pad"),
            ],
        };
        Panel { sections }
    }

    fn show_zones(&self, z: &mut KeyZones, st: &PlayState) {
        z.zones = st
            .zones
            .iter()
            .map(|k| {
                // The pad picked shows its settings as they are now.
                let keys = if k.pad == st.pad {
                    st.settings.keys.unwrap_or(k.keys)
                } else {
                    k.keys
                };
                Zone {
                    pad: k.pad,
                    keys,
                    fade: k.fade,
                    label: pad_text(self.bank, k.pad),
                    color: pad_color(k.pad),
                }
            })
            .collect();
        z.picked = st.settings.keys.map(|_| st.pad);
        z.pads_from = match st.pads_play {
            PadsPlay::Keys { from } => Some(from),
            _ => None,
        };
        // Whole octaves round the roots, the pad's zone and the pads' notes.
        let mut notes: Vec<u8> = z.zones.iter().map(|z| z.keys.root).collect();
        if let Some(k) = st.settings.keys {
            notes.extend([
                k.root,
                k.lo.max(k.root.saturating_sub(24)),
                k.hi.min(k.root.saturating_add(24)),
            ]);
        }
        if let Some(from) = z.pads_from {
            notes.extend([from, from.saturating_add(15).min(127)]);
        }
        let lo = notes.iter().copied().min().unwrap_or(48);
        let hi = notes.iter().copied().max().unwrap_or(72);
        z.show_range(lo, hi);
        let n = z.zones.len();
        z.right = match n {
            0 => "no keygroups".into(),
            1 => "1 keygroup".into(),
            n => format!("{n} keygroups"),
        };
        z.caption = match st.settings.keys {
            Some(k) => format!(
                "{}: {}–{}, root {} · drag its edges and root",
                pad_text(self.bank, st.pad),
                note_name(k.lo),
                note_name(k.hi),
                note_name(k.root)
            ),
            None => format!(
                "{} is not a keygroup: Keys On makes it one · tap a zone to pick its pad",
                pad_text(self.bank, st.pad)
            ),
        };
    }

    fn show_cards(&self, ui: &mut Ui, st: &PlayState) {
        let row = self.row(ui);
        for (k, r) in self.section.qlinks().iter().enumerate() {
            let Some(c) = ui.get_mut::<KnobCard>(QLINKS[k]) else {
                continue;
            };
            c.label = if matches!(r, Row::LayerLevel | Row::LayerPan | Row::LayerTune) {
                format!("Q-Link {} · Layer {}", k + 1, r.label().to_lowercase())
            } else if *r == Row::Wow {
                format!("Q-Link {} · Wow", k + 1)
            } else {
                format!("Q-Link {} · {}", k + 1, r.label())
            };
            c.scope = self.section.label().to_string();
            c.focused = *r == row;
            (c.value, c.sub) = card_text(&st.layer_text(self.layer, *r));
            c.pos = knob_pos(st, self.layer, *r);
            let why = faint_why(st, *r);
            c.enabled = why.is_none();
            c.why = why.unwrap_or_default().to_string();
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &PlayState) -> Option<PlayAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(PlayAction::Tab(*i)),
            (SCENES, EventKind::Tab(i @ (0 | 1))) => {
                Some(PlayAction::Scene(SceneAction::Store(*i)))
            }
            (SCENES, EventKind::Tab(2)) if st.scenes[0] => {
                self.fade = 0.0;
                self.show_scenes(ui, st);
                Some(PlayAction::Scene(SceneAction::Return))
            }
            (SCENES, EventKind::Value(x)) => {
                if st.scenes != [true, true] {
                    // Nothing to fade between yet: the fader stays at A.
                    self.show_scenes(ui, st);
                    return None;
                }
                let x = if x.is_finite() {
                    x.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if x == self.fade {
                    return None;
                }
                self.fade = x;
                self.show_scenes(ui, st);
                Some(PlayAction::Scene(SceneAction::Fade(x)))
            }
            (LAYERS, EventKind::Select(k)) => {
                self.layer = (*k).min(st.settings.layers.len() - 1);
                self.lift();
                self.show(ui, st);
                None
            }
            (SECTIONS, EventKind::Tab(i)) => {
                let section = *Section::ALL.get(*i)?;
                self.open_section(ui, st, section);
                None
            }
            (KEYS, EventKind::Select(pad)) => (*pad != st.pad).then_some(PlayAction::Pick(*pad)),
            (KEYS, EventKind::Pad { index, down }) => Some(PlayAction::Note {
                note: u8::try_from(*index).ok()?.min(127),
                down: *down,
            }),
            (KEYS, EventKind::Marker { index, frame }) => {
                let mut after = st.settings.clone();
                let k = after.keys.as_mut()?;
                let note = (*frame).min(127) as u8;
                match index {
                    0 => k.lo = note.min(k.hi),
                    1 => k.hi = note.max(k.lo),
                    _ => k.root = note,
                }
                self.edit(st, Key::Zone(*index), after)
            }
            (LAYERS, EventKind::Marker { index, frame }) => {
                let (k, high) = (index / 2, index % 2 == 1);
                let mut after = st.settings.clone();
                let l = after.layers.get_mut(k)?;
                let v = (*frame).clamp(1, 127) as u8;
                if high {
                    l.vel_hi = v.max(l.vel_lo);
                } else {
                    l.vel_lo = v.min(l.vel_hi);
                }
                self.edit(st, Key::Layer(*index), after)
            }
            (ENV, EventKind::Marker { index, .. }) => {
                let env = ui.get::<EnvelopeView>(ENV)?.env;
                let mut after = st.settings.clone();
                if self.section == Section::Filter {
                    after.filter_env = checked_env(env, 0.0);
                    self.edit(st, Key::FilterEnv(*index), after)
                } else {
                    after.amp = checked_env(env, MIN_RELEASE_MS);
                    self.edit(st, Key::Env(*index), after)
                }
            }
            (FILTER, EventKind::Marker { frame, .. }) => {
                let mut after = st.settings.clone();
                after.filter.cutoff_hz =
                    (*frame as f32).clamp(CUTOFF_RANGE_HZ.0, CUTOFF_RANGE_HZ.1);
                self.edit(st, Key::Cutoff, after)
            }
            (LIST, EventKind::Tab(i)) => match self.section.rows().get(*i)? {
                Row::AddLayer => self.add_layer(st),
                Row::RemoveLayer => self.remove_layer(st),
                _ => None,
            },
            (LIST, EventKind::Step { index, delta }) => {
                let row = *self.section.rows().get(*index)?;
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
                self.pick_row(ui, self.section.qlinks()[k]);
                self.show_cards(ui, st);
                None
            }
            _ => None,
        }
    }

    /// Picks row `row`, if it is in the section open.
    fn pick_row(&self, ui: &mut Ui, row: Row) {
        if row.section() != self.section {
            return;
        }
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.selected = row.place();
        }
    }

    /// A Q-Link turned: `knob` 0 to 3 turns the section's four rows
    /// ([`Section::qlinks`]) by `delta` detents; `fine` (SHIFT held) turns
    /// them finer. A choice steps as the arrows do. A row with nothing to
    /// change (the filter's while it is off) does nothing.
    pub fn qlink(
        &mut self,
        ui: &mut Ui,
        st: &PlayState,
        knob: usize,
        delta: i32,
        fine: bool,
    ) -> Option<PlayAction> {
        let row = *self.section.qlinks().get(knob)?;
        self.pick_row(ui, row);
        self.show_cards(ui, st);
        if delta == 0 || faint_why(st, row).is_some() {
            return None;
        }
        let Some(after) = turned(&st.settings, self.layer, row, delta, fine) else {
            let a = self.step(st, row, delta);
            self.show_cards(ui, st);
            return a;
        };
        self.edit(st, Key::Row(row), after)
    }

    /// The data wheel: steps the row picked by `delta` detents.
    pub fn wheel(&mut self, ui: &mut Ui, st: &PlayState, delta: i32) -> Option<PlayAction> {
        let row = self.row(ui);
        let a = self.step(st, row, delta);
        self.show_cards(ui, st);
        a
    }

    /// Whether a hit on a pad edits the row picked rather than picking
    /// the pad: the Stops and Also plays rows.
    pub fn takes_pads(&self, ui: &Ui) -> bool {
        matches!(self.row(ui), Row::Chokes | Row::Links)
    }

    /// Pad `pad` was hit while [`PlayTab::takes_pads`]: it goes into the
    /// set the row picked edits, or out of it if it was there, as an undo
    /// step of its own. The pad shown itself changes nothing.
    pub fn pad_hit(&mut self, ui: &Ui, st: &PlayState, pad: usize) -> Option<PlayAction> {
        let row = self.row(ui);
        if pad == st.pad || pad >= PADS || !matches!(row, Row::Chokes | Row::Links) {
            return None;
        }
        let mut p = st.settings.clone();
        let set = if row == Row::Chokes {
            &mut p.chokes
        } else {
            &mut p.links
        };
        match set.iter().position(|&x| x == pad) {
            Some(i) => {
                set.remove(i);
            }
            None => {
                set.push(pad);
                set.sort_unstable();
            }
        }
        self.lift();
        let a = self.edit(st, Key::Row(row), p);
        self.lift();
        a
    }

    /// The action for changed settings (None if nothing changed), in the
    /// undo step of the edit being made, or a new one.
    fn edit(&mut self, st: &PlayState, key: Key, after: Pad) -> Option<PlayAction> {
        if after == st.settings {
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
        Some(PlayAction::Pad {
            pad: st.pad,
            after: Box::new(after),
            gesture,
        })
    }

    /// Row `row` stepped by `delta` (an arrow is one step, the wheel one per
    /// detent).
    fn step(&mut self, st: &PlayState, row: Row, delta: i32) -> Option<PlayAction> {
        if delta == 0 {
            return None;
        }
        if row == Row::KeyFade {
            if st.zones.is_empty() {
                return None;
            }
            let now = i32::from(st.key_fade.unwrap_or(0));
            let fade = (now + delta).clamp(0, i32::from(MAX_KEY_FADE)) as u8;
            return (st.key_fade != Some(fade)).then_some(PlayAction::KeyFade(fade));
        }
        if row.of_kit() {
            let play = st.pads_play_stepped(row, delta);
            return (play != st.pads_play).then_some(PlayAction::PadsPlay(play));
        }
        if matches!(row, Row::Chokes | Row::Links) {
            // A pad never stops or plays itself: steps go past it.
            let mut p = st.settings.clone();
            let set = if row == Row::Chokes {
                &mut p.chokes
            } else {
                &mut p.links
            };
            step_pads(set, delta, Some(st.pad));
            return self.edit(st, Key::Row(row), p);
        }
        if row == Row::Sound {
            return self.step_sound(st, delta);
        }
        if row.of_layer() {
            let p = stepped_layer(&st.settings, self.layer, row, delta);
            return self.edit(st, Key::Row(row), p);
        }
        let mut p = stepped(&st.settings, row, delta, st.granular());
        // Keys turned on play the sound as heard at its own note.
        if let (None, Some(k)) = (st.settings.keys, p.keys.as_mut()) {
            k.root = st.heard_root().unwrap_or(k.root);
        }
        self.edit(st, Key::Row(row), p)
    }
}

impl PlayTab {
    /// Add layer: a copy of the layer picked, which the new one becomes.
    fn add_layer(&mut self, st: &PlayState) -> Option<PlayAction> {
        let n = st.settings.layers.len();
        if n >= MAX_LAYERS {
            return None;
        }
        let after = self.layer.min(n - 1);
        self.lift();
        // The new layer is picked once it is there.
        self.layer = after + 1;
        Some(PlayAction::AddLayer { pad: st.pad, after })
    }

    /// Remove layer: the layer picked out, the one before it picked.
    fn remove_layer(&mut self, st: &PlayState) -> Option<PlayAction> {
        let n = st.settings.layers.len();
        if n < 2 {
            return None;
        }
        let layer = self.layer.min(n - 1);
        self.lift();
        self.layer = layer.saturating_sub(1);
        Some(PlayAction::RemoveLayer { pad: st.pad, layer })
    }

    /// The layer picked's Sound row stepped: the next of the kit's sounds,
    /// whole, as one undo step. None when the kit has no other sound.
    fn step_sound(&mut self, st: &PlayState, delta: i32) -> Option<PlayAction> {
        let l = st.settings.layers.get(self.layer)?;
        let n = st.kit_sounds.len() as i32;
        if n == 0 {
            return None;
        }
        let k = match st.kit_sounds.iter().position(|s| s.id == l.region.source) {
            Some(k) => (k as i32 + delta).rem_euclid(n),
            None if delta > 0 => 0,
            None => n - 1,
        };
        let source = st.kit_sounds[k as usize].id;
        if source == l.region.source {
            // The kit's only sound: stepping round to it again would put it
            // on whole, quietly dropping the layer's trim and loop.
            return None;
        }
        self.lift();
        Some(PlayAction::LayerSound {
            pad: st.pad,
            layer: self.layer,
            source,
        })
    }
}

/// `p` with row `row` of layer `layer` stepped by `delta` steps, within
/// its limits. Other rows: [`stepped`].
pub fn stepped_layer(p: &Pad, layer: usize, row: Row, delta: i32) -> Pad {
    let mut p = p.clone();
    let Some(l) = p.layers.get_mut(layer) else {
        return p;
    };
    let d = delta as f32;
    match row {
        Row::VelLow => l.vel_lo = (i32::from(l.vel_lo) + delta).clamp(1, i32::from(l.vel_hi)) as u8,
        Row::VelHigh => {
            l.vel_hi = (i32::from(l.vel_hi) + delta).clamp(i32::from(l.vel_lo), 127) as u8
        }
        Row::LayerLevel => {
            let (lo, hi) = LEVEL_RANGE_DB;
            l.level_db = round_to(l.level_db + d * LEVEL_STEP_DB, LEVEL_STEP_DB).clamp(lo, hi);
        }
        Row::LayerPan => l.pan = round_to(l.pan + d * 0.05, 0.05).clamp(-1.0, 1.0),
        Row::LayerTune => l.tune_cents = step_tune(l.tune_cents, delta),
        _ => return stepped(&p, row, delta, false),
    }
    p
}

/// A tune stepped in whole semitones: a tune between them goes to the
/// next one.
fn step_tune(cents: i32, delta: i32) -> i32 {
    let below = cents.div_euclid(100);
    let semis = match (cents.rem_euclid(100), delta > 0) {
        (0, _) => below + delta,
        (_, true) => below + delta,
        (_, false) => below + 1 + delta,
    };
    (semis * 100).clamp(-TUNE_LIMIT_CENTS, TUNE_LIMIT_CENTS)
}

/// `p` with row `row` (of layer `layer`, for a layer's rows) turned by a
/// Q-Link `delta` detents, finer with `fine`; None for a row that steps as
/// its arrows do (a choice).
fn turned(p: &Pad, layer: usize, row: Row, delta: i32, fine: bool) -> Option<Pad> {
    let mut p = p.clone();
    let d = delta as f32;
    // A fraction (0 to 1, or -1 to 1) moved 1 % a detent, 0.2 % with SHIFT.
    let frac = |x: f32, lo: f32| {
        let step = if fine { 0.002 } else { 0.01 };
        round_to(x + d * step, 0.001).clamp(lo, 1.0)
    };
    let db = |x: f32| {
        let step = if fine { 0.1 } else { LEVEL_STEP_DB };
        round_to(x + d * step, 0.1).clamp(LEVEL_RANGE_DB.0, LEVEL_RANGE_DB.1)
    };
    // Tune: 10 cents a detent, a cent with SHIFT.
    let cents =
        |c: i32| (c + delta * if fine { 1 } else { 10 }).clamp(-TUNE_LIMIT_CENTS, TUNE_LIMIT_CENTS);
    match row {
        Row::Attack => p.amp.attack_ms = turn_ms(p.amp.attack_ms, d, fine, 0.0),
        Row::Hold => p.amp.hold_ms = turn_ms(p.amp.hold_ms, d, fine, 0.0),
        Row::Decay => p.amp.decay_ms = turn_ms(p.amp.decay_ms, d, fine, 0.0),
        Row::Sustain => p.amp.sustain = frac(p.amp.sustain, 0.0),
        Row::Release => p.amp.release_ms = turn_ms(p.amp.release_ms, d, fine, MIN_RELEASE_MS),
        Row::EnvAttack => p.filter_env.attack_ms = turn_ms(p.filter_env.attack_ms, d, fine, 0.0),
        Row::EnvDecay => p.filter_env.decay_ms = turn_ms(p.filter_env.decay_ms, d, fine, 0.0),
        Row::EnvSustain => p.filter_env.sustain = frac(p.filter_env.sustain, 0.0),
        Row::EnvRelease => p.filter_env.release_ms = turn_ms(p.filter_env.release_ms, d, fine, 0.0),
        Row::Cutoff => {
            let semis = if fine { d * 0.1 } else { d };
            let hz = p.filter.cutoff_hz * (semis / 12.0).exp2();
            p.filter.cutoff_hz = hz.round().clamp(CUTOFF_RANGE_HZ.0, CUTOFF_RANGE_HZ.1);
        }
        Row::Resonance => p.filter.resonance = frac(p.filter.resonance, 0.0),
        Row::FilterEnv => {
            let step = if fine { 0.01 } else { 0.05 };
            p.filter.env_octaves =
                round_to(p.filter.env_octaves + d * step, 0.01).clamp(-ENV_OCTAVES, ENV_OCTAVES);
        }
        Row::Level => p.level_db = db(p.level_db),
        Row::Pan => p.pan = frac(p.pan, -1.0),
        Row::Tune => p.tune_cents = cents(p.tune_cents),
        Row::Wow => p.wow = frac(p.wow, 0.0),
        Row::VelToLevel => p.vel_to_level = frac(p.vel_to_level, 0.0),
        Row::VelToStart => p.vel_to_start = frac(p.vel_to_start, 0.0),
        Row::Depth => p.pressure.depth = frac(p.pressure.depth, -1.0),
        Row::LayerLevel | Row::LayerPan | Row::LayerTune => {
            let l = p.layers.get_mut(layer)?;
            match row {
                Row::LayerLevel => l.level_db = db(l.level_db),
                Row::LayerPan => l.pan = frac(l.pan, -1.0),
                _ => l.tune_cents = cents(l.tune_cents),
            }
        }
        Row::VelLow | Row::VelHigh => return Some(stepped_layer(&p, layer, row, delta)),
        _ => return None,
    }
    Some(p)
}

/// Why a row has nothing to change now, for its Q-Link card: the filter's
/// rows while it is off, the keys' while the pad is no keygroup, and so on.
fn faint_why(st: &PlayState, row: Row) -> Option<&'static str> {
    let p = &st.settings;
    match row {
        Row::Cutoff
        | Row::Resonance
        | Row::FilterEnv
        | Row::EnvAttack
        | Row::EnvDecay
        | Row::EnvSustain
        | Row::EnvRelease
            if p.filter.kind == FilterKind::Off =>
        {
            Some("The filter is off: pick one in the Filter row")
        }
        Row::LowKey | Row::HighKey | Row::RootKey if p.keys.is_none() => {
            Some("The pad is no keygroup: turn Keys on")
        }
        Row::Depth if p.pressure.target == PressureTarget::Off => {
            Some("Pressure is off: pick what it changes first")
        }
        Row::PadsFrom if !matches!(st.pads_play, PadsPlay::Keys { .. }) => {
            Some("The pads play their sounds: set Pads play to Notes")
        }
        Row::LevelsBy if !matches!(st.pads_play, PadsPlay::Levels { .. }) => {
            Some("Set Pads play to 16 Levels first")
        }
        _ => None,
    }
}

/// How far round a row's Q-Link knob is lit, 0 to 1.
fn knob_pos(st: &PlayState, layer: usize, row: Row) -> f32 {
    let p = &st.settings;
    let time = |ms: f32| (1.0 + ms).ln() / (1.0 + MAX_MS).ln();
    let level = |db: f32| (db - LEVEL_RANGE_DB.0) / (LEVEL_RANGE_DB.1 - LEVEL_RANGE_DB.0);
    let tune = |c: i32| (c + TUNE_LIMIT_CENTS) as f32 / (2 * TUNE_LIMIT_CENTS) as f32;
    let l = p.layers.get(layer);
    let x = match row {
        Row::Attack => time(p.amp.attack_ms),
        Row::Hold => time(p.amp.hold_ms),
        Row::Decay => time(p.amp.decay_ms),
        Row::Release => time(p.amp.release_ms),
        Row::Sustain => p.amp.sustain,
        Row::EnvAttack => time(p.filter_env.attack_ms),
        Row::EnvDecay => time(p.filter_env.decay_ms),
        Row::EnvRelease => time(p.filter_env.release_ms),
        Row::EnvSustain => p.filter_env.sustain,
        Row::Cutoff => {
            let (lo, hi) = CUTOFF_RANGE_HZ;
            (p.filter.cutoff_hz / lo).ln() / (hi / lo).ln()
        }
        Row::Resonance => p.filter.resonance,
        Row::FilterEnv => (p.filter.env_octaves + ENV_OCTAVES) / (2.0 * ENV_OCTAVES),
        Row::Level => level(p.level_db),
        Row::Pan => (p.pan + 1.0) / 2.0,
        Row::Tune => tune(p.tune_cents),
        Row::Wow => p.wow,
        Row::VelToLevel => p.vel_to_level,
        Row::VelToStart => p.vel_to_start,
        Row::Depth => (p.pressure.depth + 1.0) / 2.0,
        Row::Pressure => {
            let i = PRESSURE.iter().position(|t| *t == p.pressure.target);
            i.unwrap_or(0) as f32 / (PRESSURE.len() - 1) as f32
        }
        Row::Polyphony => {
            let i = POLYPHONY.iter().position(|v| *v >= p.polyphony);
            i.unwrap_or(POLYPHONY.len() - 1) as f32 / (POLYPHONY.len() - 1) as f32
        }
        Row::MuteGroup => f32::from(p.mute_group.unwrap_or(0)) / 16.0,
        Row::LowKey => p.keys.map_or(0.0, |k| f32::from(k.lo) / 127.0),
        Row::HighKey => p.keys.map_or(0.0, |k| f32::from(k.hi) / 127.0),
        Row::PadsPlay => match st.pads_play {
            PadsPlay::Pads => 0.0,
            PadsPlay::Keys { .. } => 0.5,
            PadsPlay::Levels { .. } => 1.0,
        },
        Row::PadsFrom => match st.pads_play {
            PadsPlay::Keys { from } => f32::from(from) / f32::from(MAX_PADS_FROM),
            _ => 0.0,
        },
        Row::VelLow => l.map_or(0.0, |l| f32::from(l.vel_lo) / 127.0),
        Row::VelHigh => l.map_or(0.0, |l| f32::from(l.vel_hi) / 127.0),
        Row::LayerLevel => l.map_or(0.0, |l| level(l.level_db)),
        Row::LayerPan => l.map_or(0.5, |l| (l.pan + 1.0) / 2.0),
        Row::LayerTune => l.map_or(0.5, |l| tune(l.tune_cents)),
        _ => 0.0,
    };
    x.clamp(0.0, 1.0)
}

/// A value on a Q-Link card: "310 ms" as ("310", "ms"); a value without a
/// unit ("Center", "Group 2", "L 10") whole.
fn card_text(text: &str) -> (String, String) {
    const UNITS: [&str; 9] = ["ms", "s", "Hz", "kHz", "%", "dB", "st", "oct", "voices"];
    match text.rsplit_once(' ') {
        Some((v, u)) if UNITS.contains(&u) => (v.to_string(), u.to_string()),
        _ => (text.to_string(), String::new()),
    }
}

/// The line under a section's chip: what is in it now.
fn section_text(st: &PlayState, s: Section) -> String {
    let p = &st.settings;
    match s {
        Section::Layers => match p.layers.len() {
            1 => "1 layer".into(),
            n => format!("{n} · {}", select_label(p.select).to_lowercase()),
        },
        Section::Amp => {
            // Milliseconds without their unit, to fit.
            let ms = |ms: f32| match ms_text(ms).strip_suffix(" ms") {
                Some(n) => n.to_string(),
                None => ms_text(ms),
            };
            format!(
                "A {} · D {} · S {} %",
                ms(p.amp.attack_ms),
                ms(p.amp.decay_ms),
                percent(p.amp.sustain)
            )
        }
        Section::Filter => match p.filter.kind {
            FilterKind::Off => "Off".into(),
            _ => format!(
                "{} · {}",
                filter_label(&p.filter),
                hz_text(p.filter.cutoff_hz)
            ),
        },
        Section::Pitch => {
            let mut t = tune_text(p.tune_cents);
            if let Some(k) = p.keys {
                t.push_str(&format!(" · keys {}–{}", note_name(k.lo), note_name(k.hi)));
            }
            if p.vintage {
                t.push_str(" · vintage");
            }
            t
        }
        Section::Mod => match p.pressure.target {
            PressureTarget::Off => format!("vel → level {} %", percent(p.vel_to_level)),
            t => format!("pressure → {}", pressure_label(t).to_lowercase()),
        },
        Section::Pad => {
            let mut t = voices_text(p.polyphony);
            if let Some(g) = p.mute_group {
                t.push_str(&format!(" · group {g}"));
            }
            if p.latch {
                t.push_str(" · latch");
            }
            t
        }
    }
}

/// `p` with row `row` stepped by `delta` steps, within its limits.
/// `granular` offers the grain pressure targets. A layer's rows:
/// [`stepped_layer`].
pub fn stepped(p: &Pad, row: Row, delta: i32, granular: bool) -> Pad {
    let mut p = p.clone();
    let d = delta as f32;
    match row {
        Row::Polyphony => {
            let i = nearest(
                POLYPHONY.iter().map(|&v| f32::from(v)),
                f32::from(p.polyphony),
            );
            p.polyphony = POLYPHONY[shift(i, delta, POLYPHONY.len())];
        }
        Row::MuteGroup => {
            let g = i32::from(p.mute_group.unwrap_or(0)) + delta;
            p.mute_group = match g.clamp(0, 16) {
                0 => None,
                g => Some(g as u8),
            };
        }
        Row::Chokes => step_pads(&mut p.chokes, delta, None),
        Row::Links => step_pads(&mut p.links, delta, None),
        Row::Latch => p.latch = !p.latch,
        Row::Layers => {
            let all = [LayerSelect::All, LayerSelect::Cycle, LayerSelect::Random];
            let i = all.iter().position(|s| *s == p.select).unwrap_or(0);
            p.select = all[(i as i32 + delta).rem_euclid(3) as usize];
        }
        Row::Level => {
            let (lo, hi) = LEVEL_RANGE_DB;
            p.level_db = round_to(p.level_db + d * LEVEL_STEP_DB, LEVEL_STEP_DB).clamp(lo, hi);
        }
        Row::Pan => p.pan = round_to(p.pan + d * 0.05, 0.05).clamp(-1.0, 1.0),
        Row::Tune => p.tune_cents = step_tune(p.tune_cents, delta),
        Row::Wow => p.wow = round_to(p.wow + d * 0.1, 0.1).clamp(0.0, 1.0),
        Row::Vintage => p.vintage = !p.vintage,
        Row::Keys => {
            // On over the whole keyboard, at middle C until the app knows
            // better; a keygroup plays chords, so it gets the voices for them.
            p.keys = match p.keys {
                Some(_) => None,
                None => {
                    p.polyphony = p.polyphony.max(KEYGROUP_POLYPHONY);
                    Some(Keys::all(60))
                }
            };
        }
        Row::LowKey | Row::HighKey | Row::RootKey => {
            if let Some(k) = p.keys.as_mut() {
                let at = |n: u8, lo: u8, hi: u8| {
                    (i32::from(n) + delta).clamp(lo.into(), hi.into()) as u8
                };
                match row {
                    Row::LowKey => k.lo = at(k.lo, 0, k.hi),
                    Row::HighKey => k.hi = at(k.hi, k.lo, 127),
                    _ => k.root = at(k.root, 0, 127),
                }
            }
        }
        Row::VelFade => {
            let i = nearest(
                VEL_FADES.iter().map(|&v| f32::from(v)),
                f32::from(p.vel_fade),
            );
            p.set_vel_fade(VEL_FADES[shift(i, delta, VEL_FADES.len())]);
        }
        // The kit's rows: see [`PlayState::pads_play_stepped`] and
        // [`PlayAction::KeyFade`].
        Row::PadsPlay | Row::PadsFrom | Row::LevelsBy | Row::KeyFade => {}
        Row::VelToLevel => {
            p.vel_to_level = round_to(p.vel_to_level + d * 0.05, 0.05).clamp(0.0, 1.0)
        }
        Row::VelToStart => {
            p.vel_to_start = round_to(p.vel_to_start + d * 0.05, 0.05).clamp(0.0, 1.0)
        }
        Row::Pressure => {
            let all: Vec<PressureTarget> = PRESSURE
                .iter()
                .copied()
                .filter(|t| !is_grain(*t) || granular || *t == p.pressure.target)
                .collect();
            let i = all
                .iter()
                .position(|t| *t == p.pressure.target)
                .unwrap_or(0) as i32;
            p.pressure.target = all[(i + delta).rem_euclid(all.len() as i32) as usize];
            if p.pressure.target != PressureTarget::Off && p.pressure.depth == 0.0 {
                p.pressure.depth = 0.5;
            }
        }
        Row::Depth => {
            p.pressure.depth = round_to(p.pressure.depth + d * 0.05, 0.05).clamp(-1.0, 1.0)
        }
        Row::Attack => p.amp.attack_ms = step_time(p.amp.attack_ms, delta, 0.0),
        Row::Hold => p.amp.hold_ms = step_time(p.amp.hold_ms, delta, 0.0),
        Row::Decay => p.amp.decay_ms = step_time(p.amp.decay_ms, delta, 0.0),
        Row::Sustain => p.amp.sustain = round_to(p.amp.sustain + d * 0.05, 0.05).clamp(0.0, 1.0),
        Row::Release => p.amp.release_ms = step_time(p.amp.release_ms, delta, MIN_RELEASE_MS),
        Row::Filter => {
            let now = (p.filter.kind, if p.filter.poles >= 4 { 4 } else { 2 });
            let i = FILTERS.iter().position(|f| *f == now).unwrap_or(0);
            (p.filter.kind, p.filter.poles) = FILTERS[shift(i, delta, FILTERS.len())];
            if now.0 == FilterKind::Off
                && p.filter.kind != FilterKind::Off
                && p.filter.cutoff_hz >= CUTOFF_RANGE_HZ.1
            {
                // A low-pass at 20 kHz does nothing: start it where it is heard.
                p.filter.cutoff_hz = if p.filter.kind == FilterKind::HighPass {
                    200.0
                } else {
                    2000.0
                };
            }
        }
        Row::Cutoff => {
            // A third of an octave a step.
            let hz = p.filter.cutoff_hz * (d / 3.0).exp2();
            p.filter.cutoff_hz = hz.round().clamp(CUTOFF_RANGE_HZ.0, CUTOFF_RANGE_HZ.1);
        }
        Row::Resonance => {
            p.filter.resonance = round_to(p.filter.resonance + d * 0.05, 0.05).clamp(0.0, 1.0)
        }
        Row::FilterEnv => {
            p.filter.env_octaves =
                round_to(p.filter.env_octaves + d * 0.25, 0.25).clamp(-ENV_OCTAVES, ENV_OCTAVES);
        }
        Row::EnvAttack => p.filter_env.attack_ms = step_time(p.filter_env.attack_ms, delta, 0.0),
        Row::EnvDecay => p.filter_env.decay_ms = step_time(p.filter_env.decay_ms, delta, 0.0),
        Row::EnvSustain => {
            p.filter_env.sustain = round_to(p.filter_env.sustain + d * 0.05, 0.05).clamp(0.0, 1.0)
        }
        Row::EnvRelease => p.filter_env.release_ms = step_time(p.filter_env.release_ms, delta, 0.0),
        // A layer's rows: see [`stepped_layer`].
        Row::Sound
        | Row::VelLow
        | Row::VelHigh
        | Row::LayerLevel
        | Row::LayerPan
        | Row::LayerTune => {}
        // Buttons: see [`PlayTab::action`].
        Row::AddLayer | Row::RemoveLayer => {}
    }
    p
}

/// The index of the entry of `list` nearest `now`.
fn nearest(list: impl Iterator<Item = f32>, now: f32) -> usize {
    list.enumerate()
        .min_by(|a, b| (a.1 - now).abs().total_cmp(&(b.1 - now).abs()))
        .map_or(0, |(i, _)| i)
}

fn shift(i: usize, delta: i32, len: usize) -> usize {
    (i as i32 + delta).clamp(0, len as i32 - 1) as usize
}

fn round_to(x: f32, step: f32) -> f32 {
    (x / step).round() * step
}

/// An envelope time stepped through [`TIMES_MS`] from the entry nearest it
/// in the direction of `delta` (a time between entries goes to the next).
fn step_time(ms: f32, delta: i32, min: f32) -> f32 {
    let i = if delta > 0 {
        TIMES_MS
            .iter()
            .position(|&t| t > ms)
            .map_or(TIMES_MS.len(), |i| i) as i32
            + delta
            - 1
    } else {
        TIMES_MS
            .iter()
            .rposition(|&t| t < ms)
            .map_or(-1, |i| i as i32)
            + delta
            + 1
    };
    TIMES_MS[i.clamp(0, TIMES_MS.len() as i32 - 1) as usize].max(min)
}

/// An envelope time turned by `d` detents: a doubling every
/// [`TIME_DETENTS`], from 1 ms; `fine` moves 1 ms a detent.
fn turn_ms(ms: f32, d: f32, fine: bool, min: f32) -> f32 {
    let to = if fine {
        ms + d
    } else if ms < 1.0 {
        if d > 0.0 {
            d
        } else {
            0.0
        }
    } else {
        let t = (ms * (d / TIME_DETENTS).exp2()).round();
        // Short times move at least a millisecond a detent.
        if t == ms {
            ms + d.signum()
        } else {
            t
        }
    };
    let to = if to < 1.0 && d < 0.0 { 0.0 } else { to };
    to.round().clamp(min, MAX_MS)
}

/// An envelope dragged on the picture, kept within the tab's limits, its
/// release `min_release` at least.
fn checked_env(e: Ahdsr, min_release: f32) -> Ahdsr {
    let t = |ms: f32| ms.clamp(0.0, MAX_MS);
    Ahdsr {
        attack_ms: t(e.attack_ms),
        hold_ms: t(e.hold_ms),
        decay_ms: t(e.decay_ms),
        sustain: e.sustain.clamp(0.0, 1.0),
        release_ms: t(e.release_ms).max(min_release),
    }
}

// ---------------------------------------------------------------- text

/// A row's value as the list shows it. The kit's rows are
/// [`PlayState::value_text`]'s: empty here.
pub fn value_text(p: &Pad, row: Row) -> String {
    match row {
        Row::Polyphony => voices_text(p.polyphony),
        Row::MuteGroup => p
            .mute_group
            .map_or_else(|| "Off".into(), |g| format!("Group {g}")),
        Row::Chokes => pads_text(None, &p.chokes),
        Row::Links => pads_text(None, &p.links),
        Row::Latch => if p.latch { "On" } else { "Off" }.into(),
        Row::Layers => select_label(p.select).into(),
        Row::Level => format!("{:+.1} dB", p.level_db),
        Row::Pan => pan_text(p.pan),
        Row::Tune => tune_text(p.tune_cents),
        Row::Wow => match percent(p.wow) {
            0 => "Off".into(),
            x => format!("{x} %"),
        },
        Row::Vintage => if p.vintage { "On" } else { "Off" }.into(),
        Row::Keys => if p.keys.is_some() { "On" } else { "Off" }.into(),
        Row::LowKey => p.keys.map_or_else(|| "–".into(), |k| note_name(k.lo)),
        Row::HighKey => p.keys.map_or_else(|| "–".into(), |k| note_name(k.hi)),
        Row::RootKey => p.keys.map_or_else(|| "–".into(), |k| note_name(k.root)),
        Row::KeyFade => match (p.keys, p.key_fade) {
            (None, _) => "–".into(),
            (Some(_), 0) => "Off".into(),
            (Some(_), 1) => "1 note".into(),
            (Some(_), n) => format!("{n} notes"),
        },
        Row::PadsPlay | Row::PadsFrom | Row::LevelsBy => String::new(),
        Row::VelToLevel => format!("{} %", percent(p.vel_to_level)),
        Row::VelToStart => format!("{} %", percent(p.vel_to_start)),
        Row::VelFade => match p.vel_fade {
            0 => "Off".into(),
            n => n.to_string(),
        },
        Row::Pressure => pressure_label(p.pressure.target).into(),
        Row::Depth => format!("{:+} %", (p.pressure.depth * 100.0).round() as i32),
        Row::Attack => ms_text(p.amp.attack_ms),
        Row::Hold => ms_text(p.amp.hold_ms),
        Row::Decay => ms_text(p.amp.decay_ms),
        Row::Sustain => format!("{} %", percent(p.amp.sustain)),
        Row::Release => ms_text(p.amp.release_ms),
        Row::Filter => filter_label(&p.filter),
        Row::Cutoff => hz_text(p.filter.cutoff_hz),
        Row::Resonance => format!("{} %", percent(p.filter.resonance)),
        Row::FilterEnv => octaves_text(p.filter.env_octaves),
        Row::EnvAttack => ms_text(p.filter_env.attack_ms),
        Row::EnvDecay => ms_text(p.filter_env.decay_ms),
        Row::EnvSustain => format!("{} %", percent(p.filter_env.sustain)),
        Row::EnvRelease => ms_text(p.filter_env.release_ms),
        // A layer's rows show the first layer's (the tab shows the layer
        // picked's: [`PlayState::layer_text`]).
        Row::Sound | Row::AddLayer | Row::RemoveLayer => String::new(),
        Row::VelLow | Row::VelHigh | Row::LayerLevel | Row::LayerPan | Row::LayerTune => {
            let Some(l) = p.layers.first() else {
                return "–".into();
            };
            match row {
                Row::VelLow => l.vel_lo.to_string(),
                Row::VelHigh => l.vel_hi.to_string(),
                Row::LayerLevel => format!("{:+.1} dB", l.level_db),
                Row::LayerPan => pan_text(l.pan),
                _ => tune_text(l.tune_cents),
            }
        }
    }
}

fn percent(x: f32) -> i32 {
    (x * 100.0).round() as i32
}

/// A set of pads as the list shows it: "Off", or their names in order.
pub fn pads_text(bank: Option<char>, pads: &[usize]) -> String {
    if pads.is_empty() {
        return "Off".into();
    }
    let names: Vec<String> = pads.iter().map(|&p| pad_text(bank, p)).collect();
    names.join(" ")
}

/// A pad set stepped by an arrow or the wheel: a step up adds the next
/// pad after the highest one in it (wrapping round to the first), never
/// `own`; a step down takes the highest out. Hitting pads is the quicker
/// way ([`PlayTab::pad_hit`]). The set stays sorted.
fn step_pads(set: &mut Vec<usize>, delta: i32, own: Option<usize>) {
    for _ in 0..delta.unsigned_abs() {
        if delta > 0 {
            let from = set.last().map_or(0, |&p| p + 1);
            let next = (0..PADS)
                .map(|i| (from + i) % PADS)
                .find(|p| !set.contains(p) && Some(*p) != own);
            if let Some(next) = next {
                set.push(next);
                set.sort_unstable();
            }
        } else {
            set.pop();
        }
    }
}

pub fn voices_text(n: u8) -> String {
    match n {
        1 => "Mono".into(),
        n => format!("{n} voices"),
    }
}

pub fn select_label(s: LayerSelect) -> &'static str {
    match s {
        LayerSelect::All => "Stacked",
        LayerSelect::Cycle => "Cycle",
        LayerSelect::Random => "Random",
    }
}

pub fn pressure_label(t: PressureTarget) -> &'static str {
    match t {
        PressureTarget::Off => "Off",
        PressureTarget::Level => "Level",
        PressureTarget::Pitch => "Pitch",
        PressureTarget::Filter => "Filter cutoff",
        PressureTarget::GrainDensity => "Density",
        PressureTarget::GrainSize => "Grain size",
        PressureTarget::GrainPosition => "Position",
        PressureTarget::GrainSpray => "Spray",
    }
}

pub fn filter_label(f: &Filter) -> String {
    let kind = match f.kind {
        FilterKind::Off => return "Off".into(),
        FilterKind::LowPass => "LP",
        FilterKind::HighPass => "HP",
        FilterKind::BandPass => "BP",
    };
    format!("{kind} {}-pole", if f.poles >= 4 { 4 } else { 2 })
}

pub fn pan_text(pan: f32) -> String {
    match percent(pan) {
        0 => "Center".into(),
        x if x < 0 => format!("L {}", -x),
        x => format!("R {x}"),
    }
}

pub fn tune_text(cents: i32) -> String {
    if cents % 100 == 0 {
        format!("{:+} st", cents / 100)
    } else {
        format!("{:+.2} st", cents as f32 / 100.0)
    }
}

pub fn ms_text(ms: f32) -> String {
    if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

pub fn hz_text(hz: f32) -> String {
    if hz < 1000.0 {
        format!("{hz:.0} Hz")
    } else if hz < 10_000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{:.1} kHz", hz / 1000.0)
    }
}

pub fn octaves_text(oct: f32) -> String {
    if oct == 0.0 {
        "Off".into()
    } else {
        format!("{oct:+.2} oct")
    }
}

/// What changed between two versions of a pad's settings, for the log:
/// "attack 0 ms -> 12 ms, filter Off -> LP 2-pole". Empty if nothing did.
/// Layer regions are the Wave tab's and aren't compared.
pub fn describe(a: &Pad, b: &Pad) -> String {
    let mut out = Vec::new();
    let mut diff = |name: &str, x: String, y: String| {
        if x != y {
            out.push(format!("{name} {x} -> {y}"));
        }
    };
    diff(
        "polyphony",
        voices_text(a.polyphony),
        voices_text(b.polyphony),
    );
    diff(
        "mute group",
        value_text(a, Row::MuteGroup),
        value_text(b, Row::MuteGroup),
    );
    for row in [Row::Chokes, Row::Links, Row::Latch] {
        diff(
            &row.label().to_lowercase(),
            value_text(a, row),
            value_text(b, row),
        );
    }
    diff(
        "layers",
        select_label(a.select).into(),
        select_label(b.select).into(),
    );
    diff(
        "layers",
        a.layers.len().to_string(),
        b.layers.len().to_string(),
    );
    for (k, (x, y)) in a.layers.iter().zip(&b.layers).enumerate() {
        diff(
            &format!("layer {} velocities", k + 1),
            format!("{}-{}", x.vel_lo, x.vel_hi),
            format!("{}-{}", y.vel_lo, y.vel_hi),
        );
        diff(
            &format!("layer {} level", k + 1),
            format!("{:+.1} dB", x.level_db),
            format!("{:+.1} dB", y.level_db),
        );
        diff(
            &format!("layer {} pan", k + 1),
            pan_text(x.pan),
            pan_text(y.pan),
        );
        diff(
            &format!("layer {} tune", k + 1),
            tune_text(x.tune_cents),
            tune_text(y.tune_cents),
        );
    }
    for row in [
        Row::Level,
        Row::Pan,
        Row::Tune,
        Row::Wow,
        Row::Vintage,
        Row::Keys,
        Row::LowKey,
        Row::HighKey,
        Row::RootKey,
        Row::VelToLevel,
        Row::VelToStart,
        Row::VelFade,
        Row::Pressure,
        Row::Depth,
        Row::Attack,
        Row::Hold,
        Row::Decay,
        Row::Sustain,
        Row::Release,
        Row::Filter,
        Row::Cutoff,
        Row::Resonance,
        Row::FilterEnv,
        Row::EnvAttack,
        Row::EnvDecay,
        Row::EnvSustain,
        Row::EnvRelease,
    ] {
        diff(
            &row.label().to_lowercase(),
            value_text(a, row),
            value_text(b, row),
        );
    }
    out.join(", ")
}
