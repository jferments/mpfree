//! The one Sample page's Slice tool (the plan's step 3, in place of the
//! Chop page): cutting the sound into slices and putting them on pads.
//!
//! - **The cuts** are numbered tabs on the wave, one at the start of every
//!   slice: slice 1 starts at the region's Start and the last ends at END,
//!   which stay put (they are Trim's). The slice picked is shaded. With
//!   another tool a sliced pad's cuts are faint lines.
//! - A **tap** on the wave picks the slice under it and plays it once, as
//!   the pad made from it will ([`ChopAction::Audition`]). **Dragging** a
//!   tab moves its cut (onto a zero crossing with Snap).
//! - The **Slice section** finds the slices: the **Method** and its
//!   setting's knob, and **Snap to zero**; the slice picked, **Split**
//!   (cut it in two), **Merge** (join it to the one before) and **Clear**
//!   (one slice again). The methods: **Transient** (where the hits start,
//!   at a sensitivity), **Beat** (on the source's beat grid, a slice every
//!   16th, 8th, beat, two beats or bar; only for a sound with a tempo),
//!   **Grid** (equal slices), **Silence** (one slice per sound, below a
//!   threshold), **Pitch** (where the note changes by a number of
//!   semitones or more: a sung line or a bass part, note by note; the
//!   sound's pitch is followed the first time Pitch is picked on it, off
//!   the screen thread: [`SliceTool::wants_pitch`], [`SourceView::pitch`]),
//!   **Manual** (the cuts as they are) and **Live**: **Start** (Split's
//!   place) plays the region once and every pad played meanwhile drops a
//!   cut where the sound is (the sampler marks it to the frame,
//!   [`ChopAction::Live`]), pulled onto the nearest hit within the **Pull
//!   to hits** window; Stop, or the sound's end, ends it. Changing the
//!   method or its setting chops again; a cut moved, split, merged or
//!   cleared by hand turns the method to Manual, so nothing undoes the hand
//!   edits behind the musician's back.
//! - The **Put on section** puts the slices on pads as one undo step, from
//!   the chopped pad: **Put on** Pads (consecutive pads, the first empty
//!   run after the chopped pad, on past a bank's last pad, or from the
//!   **First pad** picked), One pad (every slice on one sliced pad, R4: a
//!   hit plays its Slice, keys from C2 a slice each), Keys (also keygroups
//!   of one note each from C2, the pads playing them), New kit (an empty
//!   kit's pads from A1 on a new track, R4.6, where the app offers it),
//!   Audio track (the slices in a row on an audio track, a region each,
//!   from the playhead's bar, where the app has audio tracks: step 6,
//!   [`ChopAction::ToAudio`]) or Library (the pad saved as files in the
//!   library, where the app can write them: [`ChopAction::Export`]). **Phrase** (for a sound with a tempo, on
//!   Pads, One pad or New kit) also makes a clip of the slices in time
//!   ([`ChopAction::phrase`]). The main button names what it does ("Chop
//!   to A6–A13", "Slices on A6", "To keys C2–G2", "Phrase on A6–A13",
//!   "To audio track", "Save 8 slices"); the section's note says which
//!   pads it replaces.
//! - **Q-Link 1** turns the method's setting, **2** picks a slice, **3**
//!   moves its start (one pixel of the current zoom per detent, snapped in
//!   the direction of the turn; with SHIFT one frame) and **4** the first
//!   pad. The **wheel** zooms round the slice picked.
//!
//! A take kept on the page (one-page step 2b) is sliced as a pad is and a
//! tap plays its slice; its slices go on pads straight from the page (step
//! 3b, [`ChopAction::apply_take`]), from the first empty pads from A1, and
//! the take leaves the page with them. Live waits until it is on a pad,
//! since it plays a pad.
//!
//! The project keeps the cuts for the sound as the tool last left them
//! ([`SliceTool::take_kept`], `Project::keep_chop`), outside undo: trying
//! methods changes no pad, but a chop made is there when the tool comes
//! back to the sound, after another pad or a project opened again
//! ([`ChopState::kept`]). A chop the tool makes by itself (the first
//! Transient chop of a sound) is not kept. The Wave page gives the tool
//! the [`ChopState`] its [`crate::wave_tab::WaveState`] carries, the app
//! keeps what [`SliceTool::take_kept`] hands it after each touch, and
//! applies the [`ChopAction`]s it gets back ([`ChopAction::apply`]).

use std::ops::Range;
use std::sync::Arc;

use mpfree_analysis::{
    nearest_zero_crossing, next_zero_crossing, note_name, previous_zero_crossing, sound_spans,
    Samples, SilenceSettings,
};
use mpfree_sampler::model::{
    ChopMode, ChopSet, ModelError, Project, Region, SourceId, KEPT_CHOP, MAX_SLICES, PADS,
    SLICE_KEYS_FROM,
};
use mpfree_sampler::phrase::{self, Phrase};
use mpfree_ui::wave::{Marker, MarkerKind};

use crate::command::{Change, Chop, Chopped};
use crate::panel::{Control, Item, Look, PadCell, Panel, Section};
use crate::source::SourceView;
use crate::warp_tool::Card;
use crate::wave_tab::{self, pad_label, position_text, TAKE};
use crate::widgets::Setting;

/// Pads in a bank: PAD BANK A to H each play 16 of a kit's pads.
pub const BANK: usize = 16;
/// The kit's banks, A to H.
const BANKS: usize = PADS / BANK;
/// The most slices the tool cuts: as many as a kit has pads, and as one
/// sliced pad holds (R4).
pub const MAX_CUT_SLICES: usize = MAX_SLICES;

/// The switch in the Phrase row's place on Put on Audio track (audio
/// tracks phase 2): the slices on the song, or the whole sound in a cell.
pub const CELL_SWITCH: [&str; 2] = ["On the song", "In a cell"];
/// Why the pad map and First pad do nothing with Put on Audio track.
const AUDIO_NO_PAD: &str = "An audio track plays the slices in a row: no pad";
/// The hint line the app shows under the page with the Slice tool.
pub const HINT: &str =
    "Tap a slice to pick and hear it · drag a cut's tab to move it · Split, Merge, then Chop to \
                        pads (UNDO takes it back) · SHIFT + Q-Link 3: one frame at a time";
/// Why Live can't start on a take kept on the page.
pub const LIVE_TAKE: &str =
    "Live plays a pad: put the take on a pad first (Send to pad, then hit a pad)";
/// Cuts a method makes are at least this far apart (and from the region's
/// ends), so no slice is a sliver.
const MIN_SLICE_MS: f64 = 10.0;
/// The main button's height in Put on, as drawn.
const PUT_BUTTON_H: i32 = 44;
/// The pad map's width, and its ‹ Bank ›'s height, as drawn.
const MAP_W: i32 = 132;
const BANK_H: i32 = 30;
/// How far a dragged tab looks for a zero crossing: this many frames, or
/// eight pixels of the current zoom if that is more.
const ZERO_RADIUS: u64 = 256;
/// How far a Q-Link turn looks ahead for the next zero crossing.
const ZERO_AHEAD: usize = 8192;
/// Slice lengths for Beat, in beats, with their names.
const BEAT_LENGTHS: [(f64, &str); 5] = [
    (0.25, "16th"),
    (0.5, "8th"),
    (1.0, "Beat"),
    (2.0, "2 beats"),
    (4.0, "Bar"),
];
const SENSITIVITY_STEP: u8 = 5;
const THRESHOLD_RANGE_DB: (i32, i32) = (-72, -12);
const THRESHOLD_STEP_DB: i32 = 3;
/// Live: how far a cut may move to the nearest hit, in ms (0 is Off).
const HIT_WINDOWS_MS: [u16; 5] = [0, 10, 20, 40, 80];
/// Live: the sensitivity the hits it snaps to are found at.
const LIVE_HITS_SENSITIVITY: f32 = 0.6;
/// Pitch: the change in semitones that makes a cut, 1 to an octave.
const PITCH_CHANGES: (u8, u8) = (1, 12);
/// Pitch: the shortest note that counts, ms; shorter ones (a slide's
/// passing notes, a scoop into a note) don't make a cut.
const PITCH_MIN_NOTE_MS: u64 = 50;

/// How the tool cuts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    #[default]
    Transient,
    Beat,
    Grid,
    Silence,
    /// Where the note changes (R4.7).
    Pitch,
    Manual,
    /// Cuts dropped by hitting pads while the sound plays.
    Live,
}

impl Method {
    pub const ALL: [Method; 7] = [
        Method::Transient,
        Method::Beat,
        Method::Grid,
        Method::Silence,
        Method::Pitch,
        Method::Manual,
        Method::Live,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Method::Transient => "Transient",
            Method::Beat => "Beat",
            Method::Grid => "Grid",
            Method::Silence => "Silence",
            Method::Pitch => "Pitch",
            Method::Manual => "Manual",
            Method::Live => "Live",
        }
    }

    /// How the project remembers the chop.
    pub fn mode(self) -> ChopMode {
        match self {
            Method::Transient => ChopMode::Transient,
            Method::Beat => ChopMode::Beat,
            Method::Grid => ChopMode::Grid,
            Method::Silence => ChopMode::Silence,
            Method::Pitch => ChopMode::Pitch,
            Method::Manual | Method::Live => ChopMode::Manual,
        }
    }

    /// The method a chop the project remembers was made with.
    pub fn of_mode(m: ChopMode) -> Method {
        match m {
            ChopMode::Transient => Method::Transient,
            ChopMode::Beat => Method::Beat,
            ChopMode::Grid => Method::Grid,
            ChopMode::Silence => Method::Silence,
            ChopMode::Pitch => Method::Pitch,
            ChopMode::Manual => Method::Manual,
        }
    }
}

/// The tool's rows on the panel, in reading order: the Slice section,
/// then Put on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Method,
    Snap,
    /// The method's setting: sensitivity, length, slices, threshold,
    /// change or pull to hits.
    Amount,
    Slice,
    /// The buttons ([`Button`]), as rows the wheel walks and its push
    /// presses.
    Split,
    Merge,
    Clear,
    /// The pads they go on: the bank's pad map, each slice's pad in its
    /// colour; a tap on a pad starts them there (the first one turns).
    Pads,
    /// The bank the pad map shows ("A1–A16"; named Map on the panel). It
    /// turns the map alone: the pads' bank is PAD BANK's, and turning it
    /// would take the page to another pad (`SamplePage::set_pad_page`).
    Bank,
    /// The slices go on the pads, on one sliced pad, on notes as well, on
    /// a new kit, or into files.
    Put,
    /// A phrase of the slices in time as well (for a sound with a tempo,
    /// with Put on at Pads, One pad or New kit).
    Phrase,
    Chop,
}

impl Row {
    /// The panel's order, which the wheel walks.
    pub const ALL: [Row; 12] = [
        Row::Method,
        Row::Snap,
        Row::Amount,
        Row::Slice,
        Row::Split,
        Row::Merge,
        Row::Clear,
        Row::Pads,
        Row::Bank,
        Row::Put,
        Row::Phrase,
        Row::Chop,
    ];

    /// Its place in [`Row::ALL`], the list's row.
    pub fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    /// The button it is, if it is one.
    pub fn button(self) -> Option<Button> {
        match self {
            Row::Split => Some(Button::Split),
            Row::Merge => Some(Button::Merge),
            Row::Clear => Some(Button::Clear),
            Row::Chop => Some(Button::ToPads),
            _ => None,
        }
    }
}

/// Where Put on puts the slices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Put {
    #[default]
    Pads,
    /// One pad holding every slice (R4): a hit plays the pad's Slice, and
    /// on keys from [`SLICE_KEYS_FROM`] each note plays a slice.
    OnePad,
    /// Keygroups of one note each from [`SLICE_KEYS_FROM`].
    Keys,
    /// Library: the pad's sounds, its sidecar and each slice saved in the
    /// library, under Samples › Exports ([`ChopAction::Export`]; only
    /// where the app can write them; the one Sample page's step 6c named
    /// it as drawn, before that it was Files).
    Library,
    /// An empty kit's pads from A1, the kit on a new track (sampler R4.6;
    /// where the app offers it, [`ChopState::new_kit`]).
    NewKit,
    /// An audio track: the slices in a row, a region each, from the
    /// playhead's bar (the one Sample page's step 6; where the app has
    /// audio tracks, [`ChopState::audio`]): [`ChopAction::ToAudio`].
    Audio,
}

impl Put {
    /// Its name on the Put on choice.
    pub fn label(self) -> &'static str {
        match self {
            Put::Pads => "Pads",
            Put::OnePad => "1 pad",
            Put::Keys => "Keys",
            Put::Library => "Library",
            Put::NewKit => "New kit",
            Put::Audio => "Audio track",
        }
    }
}

/// The buttons, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Split; Start or Stop with Live.
    Split,
    Merge,
    /// One slice again: every cut goes.
    Clear,
    /// The slices on the pads (or, as Put on says, on keys too, on a new
    /// kit, or into files).
    ToPads,
}

impl Button {
    pub const ALL: [Button; 4] = [Button::Split, Button::Merge, Button::Clear, Button::ToPads];

    /// Its row on the panel.
    pub fn row(self) -> Row {
        match self {
            Button::Split => Row::Split,
            Button::Merge => Row::Merge,
            Button::Clear => Row::Clear,
            Button::ToPads => Row::Chop,
        }
    }
}

/// Everything the tool chops: one layer of one pad (or of a take kept on
/// the page, its pad [`TAKE`]), its source, and which pads of its kit have
/// a sound.
#[derive(Clone)]
pub struct ChopState {
    pub pad: usize,
    pub layer: usize,
    pub pad_name: String,
    pub region: Region,
    pub source_name: String,
    pub rate: u32,
    pub channels: u8,
    /// The source's length.
    pub frames: u64,
    pub source: Arc<SourceView>,
    /// Which of the kit's pads have a sound, by pad.
    pub used: Vec<bool>,
    /// The app can write files (Put on Library is offered).
    pub files: bool,
    /// The chop kept for the sound ([`Project::kept_chop`]): the slices
    /// as the tool last left them, shown again in place of a fresh chop.
    pub kept: Option<ChopSet>,
    /// Put on New kit (sampler R4.6), where the app offers it: Ok when an
    /// empty kit and a new track can take the slices, or Err saying why
    /// not now (no empty kit left, no room for a track).
    pub new_kit: Option<Result<(), String>>,
    /// The app has audio tracks (Put on Audio track is offered).
    pub audio: bool,
    /// Put in a cell's cell (audio tracks phase 2), as the app picks it
    /// from the Clips grid: the audio track's name and the row (from 0)
    /// the sound would go in; None: a new audio track.
    pub cell: Option<(String, usize)>,
    /// What each of the kit's pads plays of the chopped sound, by pad
    /// (None: another sound, or none): so the tool knows a slice that is
    /// on a pad already ([`SliceTool::slice_pad`]).
    pub plays: Vec<Option<Plays>>,
}

/// What a pad plays of the chopped sound ([`ChopState::plays`]): its first
/// layer's span and cuts, and whether it plays notes (a keygroup, as Put
/// on Keys makes them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plays {
    pub start: u64,
    pub end: u64,
    pub cuts: Vec<u64>,
    pub keys: bool,
}

/// Where a slice lands, for the lane under the wave
/// ([`SliceTool::lands`]): its span, what the lane says (its pad, "A3"),
/// and whether the pad plays it already.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Land {
    pub start: u64,
    pub end: u64,
    pub text: String,
    pub on: bool,
}

impl ChopState {
    /// Layer `layer` of pad `pad`, if there is one and `source` is its
    /// source.
    pub fn from_project(
        p: &Project,
        pad: usize,
        layer: usize,
        source: Arc<SourceView>,
    ) -> Option<ChopState> {
        Self::of_sound(p, pad, p.pad(pad)?, layer, source)
    }

    /// Layer `layer` of take `index` kept on the page (one-page step 2b),
    /// as [`ChopState::from_project`] has a pad's; its pad is [`TAKE`].
    pub fn of_take(
        p: &Project,
        index: usize,
        layer: usize,
        source: Arc<SourceView>,
    ) -> Option<ChopState> {
        Self::of_sound(p, TAKE, p.take(index)?, layer, source)
    }

    fn of_sound(
        p: &Project,
        pad: usize,
        pd: &mpfree_sampler::model::Pad,
        layer: usize,
        source: Arc<SourceView>,
    ) -> Option<ChopState> {
        let region = pd.layers.get(layer)?.region.clone();
        let s = p.source(region.source)?;
        // A sliced pad's own slices (R4) when the sound has no chop kept:
        // the tool opens on them.
        let kept = p.kept_chop(s.id()).cloned().or_else(|| {
            (region.slice_count() > 1)
                .then(|| ChopSet::from_markers(KEPT_CHOP, s, ChopMode::Manual, region.cuts()).ok())
                .flatten()
        });
        (s.id() == source.id).then(|| ChopState {
            pad,
            layer,
            pad_name: pd.name.clone(),
            region,
            source_name: s.name.clone(),
            rate: s.rate(),
            channels: s.channels(),
            frames: s.frames(),
            source,
            used: (0..PADS).map(|i| p.pad(i).is_some()).collect(),
            files: false,
            kept,
            new_kit: None,
            audio: false,
            cell: None,
            plays: (0..PADS)
                .map(|i| {
                    let pd = p.pad(i)?;
                    let r = &pd.layers.first()?.region;
                    (r.source == s.id()).then(|| Plays {
                        start: r.start,
                        end: r.end,
                        cuts: r.cuts().collect(),
                        keys: pd.keys.is_some(),
                    })
                })
                .collect(),
        })
    }

    /// The pads slices may go on: all of the kit's, across its banks.
    pub fn room(&self) -> Range<usize> {
        0..PADS
    }

    /// Whether pad `pad` has a sound.
    fn used(&self, pad: usize) -> bool {
        self.used.get(pad).copied().unwrap_or(false)
    }

    fn frames_per_beat(&self) -> Option<f64> {
        self.source
            .tempo
            .filter(|b| b.is_finite() && *b > 0.0)
            .map(|bpm| f64::from(self.rate) * 60.0 / bpm)
    }

    fn min_slice(&self) -> u64 {
        ((f64::from(self.rate) * MIN_SLICE_MS / 1000.0) as u64).max(wave_tab::MIN_FRAMES)
    }
}

/// The 16 pads `pad` is among.
pub fn bank_of(pad: usize) -> Range<usize> {
    let first = pad / BANK * BANK;
    first..(first + BANK).min(PADS)
}

/// What a touch or control of the tool asks for. The app applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum ChopAction {
    /// Put the slices between `bounds` (the region's start, the cuts, its
    /// end) on pads `to_pad..`, one each, made from pad `pad`'s layer
    /// `layer`, and keep the chop in the project: one undo step.
    ToPads {
        pad: usize,
        layer: usize,
        source: SourceId,
        name: String,
        mode: ChopMode,
        bounds: Vec<u64>,
        to_pad: usize,
        /// As keygroups of one note each from this note, the pads then
        /// playing notes ([`Project::chop_to_keys`]).
        keys_from: Option<u8>,
        /// All on pad `to_pad` as one sliced pad
        /// ([`Project::chop_to_one_pad`]).
        one_pad: bool,
        /// Onto an empty kit's pads from A1, for a new track (sampler
        /// R4.6), not the chopped pad's kit: [`ChopAction::apply_to`].
        new_kit: bool,
        /// Also a phrase of the slices in time, at this many frames a beat
        /// (the sound's tempo): [`ChopAction::phrase`].
        phrase: Option<f64>,
    },
    /// Play source frames `start..end` (a slice) of pad `pad`'s layer
    /// `layer` once, as a pad made from it would: a tap on the slice.
    Audition {
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
    },
    /// Export pad `pad` as files with these slices: its sounds (the
    /// chopped one, of layer `layer`, with the slice points in it), a
    /// sidecar that brings the pad back, and each slice
    /// (`mpfree_sampler::export::pad`). `cuts` are the slice starts after
    /// the first.
    Export {
        pad: usize,
        layer: usize,
        cuts: Vec<u64>,
    },
    /// Put the slices between `bounds` (the region's start, the cuts, its
    /// end; source frames of pad `pad`'s layer `layer`) on an audio track
    /// in a row, a region each, from the playhead's bar: one undo step
    /// there (the one Sample page's step 6). The app copies the sound out
    /// and the audio tracks place it. With `cell` (Put in a cell, audio
    /// tracks phase 2) the whole sound, `bounds` its Start and End only,
    /// goes in an audio track's cell as a clip instead
    /// ([`ChopState::cell`] says which).
    ToAudio {
        pad: usize,
        layer: usize,
        name: String,
        bounds: Vec<u64>,
        cell: bool,
    },
    /// Live chop: play source frames `start..end` of pad `pad`'s layer
    /// `layer` once, and let the pads played meanwhile mark cuts in it
    /// (`on`); or stop that. The app sends the marks back
    /// ([`SliceTool::live_marks`]) and says when the sound ended
    /// ([`SliceTool::live_ended`]).
    Live {
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
        on: bool,
    },
}

impl ChopAction {
    /// The chop a [`ChopAction::ToPads`] asks for ([`Chop`], the sampler
    /// command's), and the pad or take it is made from (`take`: the take
    /// kept on the page). None for the other actions.
    pub fn chop(&self, take: Option<usize>) -> Option<(Chopped, Chop)> {
        let ChopAction::ToPads {
            pad,
            layer,
            source,
            name,
            mode,
            bounds,
            to_pad,
            keys_from,
            one_pad,
            ..
        } = self
        else {
            return None;
        };
        let from = match take {
            Some(i) => Chopped::Take(i),
            None => Chopped::Pad(*pad),
        };
        Some((
            from,
            Chop {
                source: *source,
                name: name.clone(),
                mode: *mode,
                bounds: bounds.clone(),
                layer: *layer,
                to_pad: *to_pad,
                keys_from: *keys_from,
                one_pad: *one_pad,
            },
        ))
    }

    /// Applies [`ChopAction::ToPads`] to the project (one undo step) and
    /// returns the pads it filled; other actions change nothing here.
    pub fn apply(&self, p: &mut Project) -> Result<Range<usize>, ModelError> {
        self.apply_from(p, None)
    }

    /// Applies a [`ChopAction::ToPads`] made on take `index` kept on the
    /// page (its pad [`TAKE`]; the one Sample page's step 3b): the slices
    /// are made from the take as [`ChopAction::apply`] makes them from a
    /// pad, and the take goes from the page with them, all one undo step
    /// ([`Project::chop_take`]). Returns the pads it filled.
    pub fn apply_take(&self, p: &mut Project, index: usize) -> Result<Range<usize>, ModelError> {
        self.apply_from(p, Some(index))
    }

    fn apply_from(&self, p: &mut Project, take: Option<usize>) -> Result<Range<usize>, ModelError> {
        let Some((from, chop)) = self.chop(take) else {
            return Ok(0..0);
        };
        Change::Chop {
            from,
            chop: Box::new(chop.clone()),
        }
        .apply(p, None)?;
        Ok(chop.pads())
    }

    /// Applies a [`ChopAction::ToPads`] made with `new_kit` from `from`
    /// (the chopped pad's kit) onto `to` (the empty kit): the slices on
    /// its pads from `to_pad`, made from the chopped pad as Chop to pads
    /// makes them, and the sound added to it, as one undo step of `to`.
    /// `from` doesn't change. Returns the pads it filled.
    pub fn apply_to(&self, from: &Project, to: &mut Project) -> Result<Range<usize>, ModelError> {
        match self.chop(None) {
            Some((made_from, chop)) => chop.place_in(from, made_from, to),
            None => Ok(0..0),
        }
    }

    /// As [`ChopAction::apply_to`], for take `index` kept on `from`'s page:
    /// the slices made from the take. `from` doesn't change; the app lets
    /// the take go there ([`Project::remove_take`]).
    pub fn apply_take_to(
        &self,
        from: &Project,
        index: usize,
        to: &mut Project,
    ) -> Result<Range<usize>, ModelError> {
        match self.chop(Some(index)) {
            Some((made_from, chop)) => chop.place_in(from, made_from, to),
            None => Ok(0..0),
        }
    }

    /// The phrase a [`ChopAction::ToPads`] asks for: its slices as hits in
    /// beats of the sound's tempo, the first slice on the first pad (or,
    /// on one pad, slice 1 of it). None when it asks for none.
    pub fn phrase(&self, p: &Project) -> Result<Option<Phrase>, ModelError> {
        let ChopAction::ToPads {
            phrase: Some(frames_per_beat),
            ..
        } = self
        else {
            return Ok(None);
        };
        let Some((_, chop)) = self.chop(None) else {
            return Ok(None);
        };
        let (c, first, count) = chop.slices(p)?;
        phrase::phrase(&c, first, count, *frames_per_beat).map(Some)
    }
}

/// `pad` named in the bank the app shows, as "A5".
pub(crate) fn pad_text(bank: Option<char>, pad: usize) -> String {
    match bank {
        Some(b) => format!("{b}{}", pad % BANK + 1),
        None => pad_label(pad),
    }
}

/// The Slice tool's own state, the chop being made: the method and its
/// settings, the cuts and the slice picked, and where they go. Everything
/// else comes from the [`ChopState`].
#[derive(Clone, Debug)]
pub struct SliceTool {
    /// The bank letter shown in pad names, as the Wave page's.
    pub bank: Option<char>,
    method: Method,
    /// Transient: 0 to 100 %.
    sensitivity: u8,
    /// Beat: an index into [`BEAT_LENGTHS`].
    beat_len: usize,
    /// Grid: how many slices.
    grid: usize,
    /// Silence: dBFS.
    threshold_db: i32,
    /// Live: an index into [`HIT_WINDOWS_MS`].
    hit_window: usize,
    /// Pitch: the change in semitones that makes a cut.
    semitones: u8,
    /// Pitch chopped before the sound's pitch was followed: the cuts come
    /// when it is ([`SliceTool::follow`]), and until then there is nothing
    /// to keep.
    pitch_pending: bool,
    /// Live chop is on: the hits its cuts snap to.
    live: Option<Vec<u64>>,
    /// Live chop was stopped by a change here: the app hears of it with
    /// the next action.
    stop_due: bool,
    /// Why the last touch changed nothing, once ([`SliceTool::take_note`]).
    note: Option<String>,
    /// Snap cuts made by hand, and by Beat, Grid and Pitch, to zero
    /// crossings.
    snap: bool,
    /// Slice starts strictly inside the region, sorted.
    cuts: Vec<u64>,
    selected: usize,
    /// The first pad chosen with Q-Link 4 or the Pads row, or filled by
    /// the last Chop to pads, for the chopped pad named (so chopping it
    /// again replaces those slices); otherwise the first run of empty pads
    /// after the chopped one.
    first_pad: Option<(usize, usize)>,
    /// The bank the pad map shows, for the chopped pad named, once its
    /// arrows have turned it; otherwise the bank the slices start in.
    map_bank: Option<(usize, usize)>,
    /// Put on: the pads; one sliced pad; also notes from
    /// [`SLICE_KEYS_FROM`], as keygroups of a note each, the pads then
    /// playing those notes; a new kit; or files.
    put: Put,
    /// The Phrase row: a phrase of the slices in time as well, where one
    /// can be made ([`SliceTool::phrase_on`]).
    phrase: bool,
    /// The Phrase row's place on Audio track: On the song (false) or In
    /// a cell ([`SliceTool::in_cell`]).
    cell: bool,
    /// The source and region the cuts were made for, and whether the
    /// source's hits and tempo were worked out then: another pad, a moved
    /// Start or End, or the analysis coming in chops again.
    made_for: Option<(SourceId, u64, u64, bool)>,
    /// The method and cuts as the project last knew them (kept, or made
    /// by the tool itself): [`SliceTool::take_kept`] hands on a change from
    /// it. By [`ChopMode`], so live chopping (kept as Manual) isn't a
    /// change.
    told: Option<(SourceId, ChopMode, Vec<u64>)>,
    /// The chop kept for the sound as the tool last saw or kept it, every
    /// marker: one the project has that differs (another project opened
    /// on the same sound) is shown in place of the tool's cuts.
    heard: Option<(SourceId, ChopMode, Vec<u64>)>,
}

impl Default for SliceTool {
    fn default() -> SliceTool {
        SliceTool {
            bank: None,
            method: Method::default(),
            sensitivity: 50,
            beat_len: 2,
            grid: 8,
            threshold_db: SilenceSettings::default().threshold_db as i32,
            hit_window: 3,
            semitones: PITCH_CHANGES.0,
            pitch_pending: false,
            live: None,
            stop_due: false,
            note: None,
            snap: true,
            cuts: Vec::new(),
            selected: 0,
            first_pad: None,
            map_bank: None,
            put: Put::Pads,
            phrase: false,
            cell: false,
            made_for: None,
            told: None,
            heard: None,
        }
    }
}

impl SliceTool {
    pub fn method(&self) -> Method {
        self.method
    }

    /// The slice picked, from 0.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Whether cuts land on zero crossings.
    pub fn snap(&self) -> bool {
        self.snap
    }

    /// Where the slices go: files only where the app writes them, a new
    /// kit only where it offers one, an audio track only where it has
    /// them.
    pub fn put(&self, st: &ChopState) -> Put {
        match self.put {
            Put::Library if !st.files => Put::Pads,
            Put::NewKit if st.new_kit.is_none() => Put::Pads,
            Put::Audio if !st.audio => Put::Pads,
            p => p,
        }
    }

    /// What the Put on row says.
    pub fn put_text(&self) -> &'static str {
        match self.put {
            Put::Pads => "Pads",
            Put::OnePad => "One pad",
            Put::Keys => "Keys",
            Put::Library => "Library",
            Put::NewKit => "New kit",
            Put::Audio => "Audio track",
        }
    }

    /// The Put on choices for `st`, in the mockup's order: New kit where
    /// the app offers it, Audio track where it has them, and Library where
    /// it writes them.
    fn puts(st: &ChopState) -> Vec<Put> {
        let mut all = vec![Put::Pads, Put::OnePad, Put::Keys];
        if st.new_kit.is_some() {
            all.push(Put::NewKit);
        }
        if st.audio {
            all.push(Put::Audio);
        }
        if st.files {
            all.push(Put::Library);
        }
        all
    }

    /// Whether a phrase can go with the slices: the sound has a tempo and
    /// they go on pads or on one pad (keygroups would need a track that
    /// plays notes).
    fn phrase_can(&self, st: &ChopState) -> bool {
        self.phrase_why(st).is_none()
    }

    /// Why no phrase can go with the slices now, or None when one can.
    fn phrase_why(&self, st: &ChopState) -> Option<&'static str> {
        if st.frames_per_beat().is_none() {
            Some("A phrase needs a beat: no tempo was found in this sound")
        } else if matches!(self.put(st), Put::Pads | Put::OnePad | Put::NewKit) {
            None
        } else if self.put(st) == Put::Audio {
            Some("Phrase is for pads: on an audio track the slices already play one after another")
        } else if st.new_kit.is_some() {
            Some("A phrase goes with Pads, One pad or New kit, not keys or files")
        } else {
            Some("A phrase goes with Pads or One pad, not keys or files")
        }
    }

    /// Whether Chop to makes a phrase too: the Phrase row on, where one
    /// can be made.
    pub fn phrase_on(&self, st: &ChopState) -> bool {
        self.phrase && self.phrase_can(st)
    }

    /// Put in a cell (audio tracks phase 2): Put on Audio track with the
    /// switch in the Phrase row's place on In a cell. The whole sound,
    /// Start to End, goes in an audio track's cell as a clip.
    pub fn in_cell(&self, st: &ChopState) -> bool {
        self.cell && self.put(st) == Put::Audio
    }

    /// What Put in a cell's clip does, for the section's note: "the whole
    /// sound · loops 2 bars at 92 BPM" for a sound with a tempo (its
    /// length to the nearest beat), else "plays once · 5.2 s".
    fn cell_note(st: &ChopState) -> String {
        let frames = st.region.end.saturating_sub(st.region.start) as f64;
        match (st.frames_per_beat(), st.source.tempo) {
            (Some(fpb), Some(bpm)) => {
                let beats = (frames / fpb).round().max(1.0) as u64;
                let length = match beats {
                    1 => "1 beat".to_string(),
                    4 => "1 bar".to_string(),
                    b if b % 4 == 0 => format!("{} bars", b / 4),
                    b => format!("{b} beats"),
                };
                let tenths = (bpm * 10.0).round();
                let bpm = if tenths % 10.0 == 0.0 {
                    format!("{}", tenths / 10.0)
                } else {
                    format!("{:.1}", tenths / 10.0)
                };
                format!("the whole sound · loops {length} at {bpm} BPM")
            }
            _ => format!("plays once · {:.1} s", frames / f64::from(st.rate.max(1))),
        }
    }

    /// Where the slices start and end: the region's start, the cuts, its end.
    pub fn bounds(&self, st: &ChopState) -> Vec<u64> {
        let mut b = Vec::with_capacity(self.cuts.len() + 2);
        b.push(st.region.start);
        b.extend_from_slice(&self.cuts);
        b.push(st.region.end);
        b
    }

    pub fn slices(&self) -> usize {
        self.cuts.len() + 1
    }

    /// The slice picked's start and end (the shaded span).
    pub fn picked(&self, st: &ChopState) -> (u64, u64) {
        let b = self.bounds(st);
        let i = self.selected.min(b.len() - 2);
        (b[i], b[i + 1])
    }

    /// The pads Chop to pads fills, if the slices fit on the kit's pads.
    /// Unless a First pad was picked, they start after the chopped pad, at
    /// the first run of empty pads long enough (into the next banks: B1
    /// after A16); with none, right after the chopped pad, or as late as
    /// they fit. A take kept on the page has no pad: from A1.
    ///
    /// With Put on at One pad, the one pad: the chopped pad itself (it
    /// becomes sliced), or the First pad picked.
    pub fn pads(&self, st: &ChopState) -> Option<Range<usize>> {
        let n = self.slices();
        let room = st.room();
        if self.put(st) == Put::NewKit {
            // The new kit's pads from A1.
            return (n <= room.len()).then_some(0..n);
        }
        let take = st.pad == TAKE;
        if self.put(st) == Put::OnePad {
            let own = if take {
                room.clone().find(|&i| !st.used(i)).unwrap_or(room.start)
            } else {
                st.pad
            };
            let at = self
                .first_pad
                .filter(|&(pad, _)| pad == st.pad)
                .map_or(own, |(_, f)| f.clamp(room.start, room.end - 1));
            return (n <= MAX_SLICES).then_some(at..at + 1);
        }
        if n > room.len() {
            return None;
        }
        let last = room.end - n;
        let after = if take { room.start } else { st.pad + 1 };
        let first = match self.first_pad.filter(|&(pad, _)| pad == st.pad) {
            Some((_, f)) => f.clamp(room.start, last),
            None => (after..=last)
                .find(|&f| !(f..f + n).any(|i| st.used(i)))
                .unwrap_or_else(|| after.clamp(room.start, last)),
        };
        Some(first..first + n)
    }

    /// The pad that plays slice `k` now, as Put on put it there: on Pads
    /// or Keys, the pad it lands on when that pad plays just the slice (a
    /// keygroup for Keys); on One pad, that pad when it plays these slices.
    /// None before the slices are put on, once they are cut again, and on
    /// New kit or Library (not this kit's pads). The Sound section follows
    /// it (the one Sample page's step 3b).
    pub fn slice_pad(&self, st: &ChopState, k: usize) -> Option<usize> {
        let b = self.bounds(st);
        let n = b.len() - 1;
        if k >= n {
            return None;
        }
        let r = self.pads(st)?;
        let plays = |pad: usize| st.plays.get(pad).and_then(Option::as_ref);
        match self.put(st) {
            put @ (Put::Pads | Put::Keys) => {
                let pad = r.start + k;
                plays(pad)
                    .filter(|p| {
                        (p.start, p.end) == (b[k], b[k + 1])
                            && p.cuts.is_empty()
                            && p.keys == (put == Put::Keys)
                    })
                    .map(|_| pad)
            }
            Put::OnePad => plays(r.start)
                .filter(|p| (p.start, p.end) == (b[0], b[n]) && p.cuts[..] == b[1..n])
                .map(|_| r.start),
            Put::NewKit | Put::Library | Put::Audio => None,
        }
    }

    /// Every slice is on the pad Put on puts it on, as it is cut now
    /// ([`SliceTool::slice_pad`]), and no phrase is asked for: Chop to
    /// would change nothing.
    fn all_on(&self, st: &ChopState) -> bool {
        !self.phrase_on(st) && (0..self.slices()).all(|k| self.slice_pad(st, k).is_some())
    }

    /// Where each slice lands, for the lane under the wave: its pad ("A3";
    /// on One pad, the pad and the slice's number, "A5 · 3"; on a new kit,
    /// its pads from A1), and whether that pad plays it already. Empty when
    /// they go into files or onto an audio track, or don't fit.
    pub fn lands(&self, st: &ChopState) -> Vec<Land> {
        let Some(r) = self.pads(st).filter(|_| self.on_pads(st)) else {
            return Vec::new();
        };
        let b = self.bounds(st);
        (0..b.len() - 1)
            .map(|k| Land {
                start: b[k],
                end: b[k + 1],
                text: match self.put(st) {
                    Put::OnePad => format!("{} · {}", pad_text(self.bank, r.start), k + 1),
                    Put::NewKit => pad_label(r.start + k),
                    _ => pad_text(self.bank, r.start + k),
                },
                on: self.slice_pad(st, k).is_some(),
            })
            .collect()
    }

    /// The bank the pad map shows: the one its arrows turned to, else the
    /// one the slices start in (the chopped pad's, when they don't fit).
    pub fn bank_shown(&self, st: &ChopState) -> usize {
        let own = self
            .pads(st)
            .map_or(st.pad.min(PADS - 1), |r| r.start)
            .min(PADS - 1)
            / BANK;
        self.map_bank
            .filter(|&(pad, _)| pad == st.pad)
            .map_or(own, |(_, b)| b.min(BANKS - 1))
    }

    /// Whether the slices go on pads (of this kit or a new one): not into
    /// files or onto an audio track.
    fn on_pads(&self, st: &ChopState) -> bool {
        !matches!(self.put(st), Put::Library | Put::Audio)
    }

    /// Why the pad map does nothing now, or None.
    fn map_why(&self, st: &ChopState) -> Option<&'static str> {
        match self.put(st) {
            Put::NewKit => Some("On a new kit the slices start on A1"),
            Put::Library => Some("Library: the slices go on no pad"),
            Put::Audio => Some(AUDIO_NO_PAD),
            _ if self.pads(st).is_none() => Some("The slices don't fit on the pads: fewer slices"),
            _ => None,
        }
    }

    /// The pad map's sixteen pads, pad 1 of the bank shown first: each
    /// slice's pad numbered in its colour, the picked one outlined; the
    /// pads with a sound raised. On One pad, the one pad. On New kit, the
    /// new kit's pads from A1 (none has a sound yet), the map drawn faint.
    pub fn map_cells(&self, st: &ChopState) -> Vec<PadCell> {
        let first = self.bank_shown(st) * BANK;
        let new_kit = self.put(st) == Put::NewKit;
        let lands = self.pads(st).filter(|_| self.on_pads(st));
        let n = self.slices();
        let used = |pad: usize| !new_kit && st.used(pad);
        (first..first + BANK)
            .map(|pad| {
                let k = lands
                    .as_ref()
                    .filter(|r| r.contains(&pad))
                    .map(|r| pad - r.start);
                match (k, self.put(st)) {
                    (Some(_), Put::OnePad) => PadCell {
                        text: if n == 1 {
                            "1".into()
                        } else {
                            format!("1–{n}")
                        },
                        tint: Some(self.selected),
                        used: used(pad),
                        picked: true,
                    },
                    (Some(k), _) => PadCell {
                        text: (k + 1).to_string(),
                        tint: Some(k),
                        used: used(pad),
                        picked: k == self.selected,
                    },
                    (None, _) => PadCell {
                        used: used(pad),
                        ..PadCell::default()
                    },
                }
            })
            .collect()
    }

    /// The pads Chop to pads fills that have a sound already.
    pub fn replaced(&self, st: &ChopState) -> Vec<usize> {
        if !self.on_pads(st) || self.put(st) == Put::NewKit {
            return Vec::new();
        }
        self.pads(st)
            .map_or_else(Vec::new, |r| r.filter(|&i| st.used(i)).collect())
    }

    /// Whether live chop is on (Start pressed, the sound not yet over).
    pub fn live(&self) -> bool {
        self.live.is_some()
    }

    /// Live chop stopped here (another method, pad or region): the
    /// [`ChopAction::Live`] that tells the app, once.
    pub fn take_stop(&mut self, st: &ChopState) -> Option<ChopAction> {
        std::mem::take(&mut self.stop_due).then_some(ChopAction::Live {
            pad: st.pad,
            layer: st.layer,
            start: st.region.start,
            end: st.region.end,
            on: false,
        })
    }

    /// Live chop's cuts, where the sound was at each pad played (source
    /// frames): each moves onto the nearest hit within the Pull to hits
    /// window, or else onto a zero crossing if Snap says so, and is kept if
    /// it falls inside the region, apart from the other cuts. The slice it
    /// starts is picked. Returns how many were kept.
    pub fn live_marks(&mut self, st: &ChopState, marks: &[u64]) -> usize {
        let Some(hits) = self.live.clone() else {
            return 0;
        };
        let window = u64::from(HIT_WINDOWS_MS[self.hit_window]) * u64::from(st.rate) / 1000;
        let gap = st.min_slice();
        let (rs, re) = (st.region.start, st.region.end);
        let mut kept = 0;
        for &m in marks {
            let near = hits
                .iter()
                .copied()
                .filter(|h| h.abs_diff(m) <= window)
                .min_by_key(|h| h.abs_diff(m));
            // A hit is cut where Transient would cut it; the rest go onto
            // a zero crossing if Snap says so.
            let c = near.unwrap_or_else(|| self.snapped(st, m, 0, 1.0));
            let apart = self.cuts.iter().all(|&x| x.abs_diff(c) >= gap);
            if c < rs + gap || c + gap > re || !apart || self.slices() >= MAX_CUT_SLICES {
                continue;
            }
            let at = self.cuts.partition_point(|&x| x < c);
            self.cuts.insert(at, c);
            self.selected = at + 1;
            kept += 1;
        }
        kept
    }

    /// The sound live chop played ended: Start again starts a new take.
    /// Whether it was on.
    pub fn live_ended(&mut self) -> bool {
        self.live.take().is_some()
    }

    /// Another tool picked: live chop stops ([`SliceTool::take_stop`]
    /// tells the app).
    pub fn leave(&mut self) {
        if self.live.take().is_some() {
            self.stop_due = true;
        }
    }

    /// Follows `st`, chopping again if it is another pad or region, or
    /// once the sound's hits and tempo are worked out.
    pub fn follow(&mut self, st: &ChopState) {
        let key = (
            st.source.id,
            st.region.start,
            st.region.end,
            st.source.ready,
        );
        // The chop kept for the sound, and its slices inside the region,
        // shown in place of the tool's own cuts when the tool comes to the
        // sound or region (a pad come back to, a project opened). One made
        // on another part of the sound, none of its cuts inside, is left
        // for that part.
        let kept = st.kept.as_ref().filter(|k| k.source == key.0);
        let heard = kept.map(|k| (k.source, k.mode, k.markers().to_vec()));
        let restore = kept.and_then(|k| {
            let cuts = SliceTool::inside(st, k.markers());
            (!cuts.is_empty() || k.markers().is_empty()).then_some((k.mode, cuts))
        });
        if self.made_for != Some(key) {
            let same_source = self.made_for.is_some_and(|k| k.0 == key.0);
            self.made_for = Some(key);
            if self.method == Method::Beat && st.frames_per_beat().is_none() && st.source.ready {
                self.method = Method::Transient;
            }
            if self.live.take().is_some() {
                // Another pad or region: live chop stops.
                self.stop_due = true;
            }
            if let Some((mode, cuts)) = restore {
                // The slices as the tool last left them on this sound.
                self.method = Method::of_mode(mode);
                self.cuts = cuts;
                self.pitch_pending = false;
            } else if matches!(self.method, Method::Manual | Method::Live) && same_source {
                // Keep the hand-made cuts that are still inside.
                self.cuts = SliceTool::inside(st, &self.cuts);
            } else {
                if self.method == Method::Manual {
                    self.method = Method::Transient;
                }
                self.cuts = match self.method {
                    // Live cuts are made on the sound: a new one starts bare.
                    Method::Live => Vec::new(),
                    _ => self.chop(st),
                };
            }
            self.selected = 0;
            // What the tool made or found by itself is nothing to keep.
            self.told = Some((key.0, self.method.mode(), self.cuts.clone()));
        } else if heard.is_some() && heard != self.heard {
            // The same sound and region, but the project keeps another
            // chop for it: show that one.
            if let Some((mode, cuts)) = restore {
                if self.method.mode() != mode {
                    self.method = Method::of_mode(mode);
                }
                self.cuts = cuts;
                self.pitch_pending = false;
                self.told = Some((key.0, self.method.mode(), self.cuts.clone()));
            }
        }
        self.heard = heard;
        if self.pitch_pending && self.method == Method::Pitch && st.source.pitch.is_some() {
            // The sound's pitch came in: the chop Pitch was waiting for.
            self.rechop(st);
        }
        self.selected = self.selected.min(self.cuts.len());
    }

    /// The chop to keep for the sound (how it was made, and the slice
    /// starts), once a touch, a Q-Link, the wheel or a live cut has
    /// changed the method or the cuts since the project last heard; None
    /// otherwise. The app keeps it ([`Project::keep_chop`]), so the slices
    /// are there when the tool comes back to the sound, and in the saved
    /// project.
    pub fn take_kept(&mut self, st: &ChopState) -> Option<(ChopMode, Vec<u64>)> {
        if self.pitch_pending {
            // Nothing to keep until the sound's pitch is followed.
            return None;
        }
        let now = (st.source.id, self.method.mode(), self.cuts.clone());
        if self.told.as_ref() == Some(&now) {
            return None;
        }
        self.told = Some(now.clone());
        // What the project will keep: no change to show back.
        self.heard = Some(now);
        Some((self.method.mode(), self.cuts.clone()))
    }

    /// `markers` (sorted) that start a slice of the region: those inside
    /// it, by the rule hand-made cuts keep to.
    fn inside(st: &ChopState, markers: &[u64]) -> Vec<u64> {
        let (lo, hi) = (
            st.region.start + wave_tab::MIN_FRAMES,
            st.region.end.saturating_sub(wave_tab::MIN_FRAMES),
        );
        markers
            .iter()
            .copied()
            .filter(|&c| c >= lo && c <= hi)
            .collect()
    }

    /// The cuts the method makes in the region.
    fn chop(&mut self, st: &ChopState) -> Vec<u64> {
        self.pitch_pending = false;
        let (rs, re) = (st.region.start, st.region.end);
        let raw: Vec<u64> = match self.method {
            Method::Manual | Method::Live => return self.cuts.clone(),
            Method::Transient => st
                .source
                .onsets
                .as_ref()
                .map_or_else(Vec::new, |o| o.pick(f32::from(self.sensitivity) / 100.0)),
            Method::Beat => {
                let Some(fpb) = st.frames_per_beat() else {
                    return Vec::new();
                };
                let step = fpb * BEAT_LENGTHS[self.beat_len].0;
                let first = (rs as f64 / step).floor() as u64 + 1;
                let last = (re as f64 / step).ceil() as u64;
                (first..last)
                    .map(|k| self.snapped(st, (k as f64 * step).round() as u64, 0, 1.0))
                    .collect()
            }
            Method::Grid => {
                let n = self.grid.min(((re - rs) / st.min_slice()).max(1) as usize) as u64;
                (1..n)
                    .map(|k| self.snapped(st, rs + (re - rs) * k / n, 0, 1.0))
                    .collect()
            }
            Method::Silence => {
                let settings = SilenceSettings {
                    threshold_db: self.threshold_db as f32,
                    ..SilenceSettings::default()
                };
                let Some(mono) = &st.source.mono else {
                    return Vec::new();
                };
                sound_spans(mono, 1, st.rate, settings)
                    .map_or_else(|_| Vec::new(), |s| s.iter().map(|s| s.0).collect())
            }
            Method::Pitch => {
                let Some(track) = st.source.pitch.clone() else {
                    self.pitch_pending = true;
                    return Vec::new();
                };
                let min = u64::from(st.rate) * PITCH_MIN_NOTE_MS / 1000;
                track
                    .changes(f32::from(self.semitones), min)
                    .into_iter()
                    .filter(|c| (rs..re).contains(c))
                    .map(|c| self.snapped(st, c, 0, 1.0))
                    .collect()
            }
        };
        // Inside the region, apart, and no more than the tool cuts.
        let gap = st.min_slice();
        let mut cuts: Vec<u64> = Vec::with_capacity(raw.len());
        for c in raw {
            if c >= rs + gap && c + gap <= re && cuts.last().is_none_or(|&l| c >= l + gap) {
                cuts.push(c);
            }
        }
        cuts.truncate(MAX_CUT_SLICES - 1);
        cuts
    }

    /// Whether the tool waits for the sound's pitch to be followed: Pitch
    /// is the method and the sound has a mono mix but no pitch yet. The
    /// app then follows it off the screen thread
    /// ([`SourceView::follow_pitch`]) and shows the page the view with it.
    pub fn wants_pitch(&self, st: &ChopState) -> bool {
        self.method == Method::Pitch && st.source.pitch.is_none() && st.source.mono.is_some()
    }

    /// The cuts as tabs on the wave: one at the start of every slice,
    /// numbered from 1, and END. The first and END are the region's Start
    /// and End (they stay put); the slice picked is marked as a cue.
    pub fn markers(&self, st: &ChopState) -> Vec<Marker> {
        let bounds = self.bounds(st);
        let last = bounds.len() - 1;
        bounds
            .iter()
            .enumerate()
            .map(|(k, &f)| {
                let (label, kind) = match k {
                    0 => ("1".to_string(), MarkerKind::Edge),
                    k if k == last => ("END".to_string(), MarkerKind::Edge),
                    k if k == self.selected => ((k + 1).to_string(), MarkerKind::Cue),
                    k => ((k + 1).to_string(), MarkerKind::Slice),
                };
                let m = Marker::new(f, &label, kind);
                if kind == MarkerKind::Edge {
                    m.fixed()
                } else {
                    m
                }
            })
            .collect()
    }

    /// What button `b` says.
    pub fn button_text(&self, st: &ChopState, b: Button) -> String {
        let n = self.slices();
        match b {
            Button::Split => match (self.method == Method::Live, self.live.is_some()) {
                (false, _) => "Split",
                (true, false) => "Start",
                (true, true) => "Stop",
            }
            .into(),
            Button::Merge => "Merge".into(),
            Button::Clear => "Clear".into(),
            Button::ToPads => match (self.pads(st).map(|p| self.range_text(&p)), self.put(st)) {
                (_, Put::Library) if n == 1 => "Save the pad".to_string(),
                (_, Put::Library) => format!("Save {n} slices"),
                (_, Put::Audio) if self.cell => match &st.cell {
                    Some((track, row)) => format!("Into {track}, row {}", row + 1),
                    None => "Into a new audio track".to_string(),
                },
                (_, Put::Audio) => "To audio track".to_string(),
                (None, Put::OnePad) => format!("Max {MAX_SLICES} slices"),
                (None, _) => format!("Max {PADS} slices"),
                (Some(_), Put::NewKit) if self.phrase_on(st) => "Phrase to new kit".to_string(),
                (Some(_), Put::NewKit) => "Chop to new kit".to_string(),
                (Some(t), Put::Pads | Put::OnePad) if self.phrase_on(st) => {
                    format!("Phrase on {t}")
                }
                (Some(t), Put::Pads) => format!("Chop to {t}"),
                (Some(t), Put::OnePad) => format!("Slices on {t}"),
                (Some(_), Put::Keys) => {
                    let k = SLICE_KEYS_FROM;
                    format!(
                        "To keys {}–{}",
                        note_name(k),
                        note_name(k.saturating_add(n as u8 - 1).min(127))
                    )
                }
            },
        }
    }

    /// Why button `b` can't be pressed now, or None when it can.
    pub fn button_why(&self, st: &ChopState, b: Button) -> Option<String> {
        let bounds = self.bounds(st);
        let n = self.slices();
        let (s0, s1) = (bounds[self.selected], bounds[self.selected + 1]);
        let live = self.method == Method::Live;
        let take = st.pad == TAKE;
        match b {
            Button::Split if live && take => Some(LIVE_TAKE.to_string()),
            Button::Split if live || s1 - s0 >= 2 * wave_tab::MIN_FRAMES && n < MAX_CUT_SLICES => {
                None
            }
            Button::Split if n >= MAX_CUT_SLICES => Some(format!("{MAX_CUT_SLICES} slices is the most")),
            Button::Split => Some("This slice is too short to split".to_string()),
            Button::Merge if n > 1 => None,
            Button::Merge => Some("One slice: nothing to merge".to_string()),
            Button::Clear if n > 1 => None,
            Button::Clear => Some("One slice: nothing to clear".to_string()),
            // Each slice on its pad already: chopping again would change
            // nothing but put back the settings those pads were given since
            // (the Sound section edits them). A phrase can still be asked
            // for again.
            Button::ToPads if self.all_on(st) => Some(format!(
                "The slices are on {} already: move a cut to chop again",
                self.pads(st).map_or_else(String::new, |r| self.range_text(&r))
            )),
            Button::ToPads if self.put(st) == Put::NewKit => match (&st.new_kit, self.pads(st)) {
                (Some(Err(why)), _) => Some(why.clone()),
                (_, None) => Some(format!("A kit has {PADS} pads: fewer slices")),
                _ => None,
            },
            Button::ToPads if self.pads(st).is_some() || !self.on_pads(st) => None,
            Button::ToPads => Some(
                "The slices don't fit on the pads from the first pad: fewer slices, or an earlier First pad"
                    .to_string(),
            ),
        }
    }

    /// The section's rows, controls and panel (Slice, then Put on), for
    /// the Wave page's panel.
    pub fn section(&self, st: &ChopState) -> (Vec<Setting>, Vec<Control>, Panel) {
        (self.rows(st), self.controls(st), self.panel(st))
    }

    /// What the Put on section's note says: the pads the slices replace,
    /// or that a new kit goes on a new track.
    pub fn put_note(&self, st: &ChopState) -> String {
        let replaced = self.replaced(st);
        match (self.pads(st).is_some(), replaced.len()) {
            (true, _) if self.put(st) == Put::NewKit => match &st.new_kit {
                Some(Ok(())) => "on a new track".into(),
                _ => "not now".into(),
            },
            _ if self.put(st) == Put::Library => "into Samples › Exports".into(),
            _ if self.in_cell(st) => Self::cell_note(st),
            _ if self.put(st) == Put::Audio => "a region each, from the playhead's bar".into(),
            _ if self.all_on(st) => "on their pads".into(),
            (false, _) => format!("{} slices: more than the kit's {PADS} pads", self.slices()),
            (true, 0) => "no pad replaced".into(),
            (true, 1..=3) => {
                let names: Vec<String> = replaced.iter().map(|&p| pad_text(self.bank, p)).collect();
                format!("replaces {}", names.join(", "))
            }
            (true, k) => format!("replaces {k} pads"),
        }
    }

    /// How each row looks on the panel, in [`Row::ALL`]'s order.
    fn controls(&self, st: &ChopState) -> Vec<Control> {
        let (_, _, pos) = self.amount();
        let steps = match self.method {
            Method::Transient => 100 / u32::from(SENSITIVITY_STEP),
            Method::Beat => BEAT_LENGTHS.len() as u32 - 1,
            Method::Grid => MAX_CUT_SLICES as u32 - 2,
            Method::Silence => {
                ((THRESHOLD_RANGE_DB.1 - THRESHOLD_RANGE_DB.0) / THRESHOLD_STEP_DB) as u32
            }
            Method::Pitch => u32::from(PITCH_CHANGES.1 - PITCH_CHANGES.0),
            Method::Manual => 1,
            Method::Live => HIT_WINDOWS_MS.len() as u32 - 1,
        };
        let puts = Self::puts(st);
        Row::ALL
            .iter()
            .map(|row| match row {
                Row::Method | Row::Slice => Control::new(Look::Chip),
                Row::Snap => Control::new(Look::Toggle(self.snap)),
                Row::Amount => Control::new(Look::Knob {
                    pos,
                    steps,
                    bipolar: false,
                })
                .faint_because(
                    self.method == Method::Manual,
                    "Manual chops have no setting: pick another Method",
                ),
                Row::Pads => Control::new(Look::Pads(self.map_cells(st)))
                    .faint_because(self.map_why(st).is_some(), self.map_why(st).unwrap_or("")),
                Row::Bank => Control::new(Look::Chip)
                    .faint_because(self.map_why(st).is_some(), self.map_why(st).unwrap_or("")),
                // On Audio track: where the sound goes, On the song (a
                // region a slice) or In a cell (the whole sound as a clip).
                Row::Phrase if self.put(st) == Put::Audio => Control::new(Look::Choice {
                    options: CELL_SWITCH.iter().map(|o| o.to_string()).collect(),
                    on: usize::from(self.cell),
                    off: Vec::new(),
                }),
                Row::Phrase => Control::new(Look::Toggle(self.phrase_on(st)))
                    .faint_because(!self.phrase_can(st), self.phrase_why(st).unwrap_or("")),
                Row::Put => Control::new(Look::Grid {
                    options: puts.iter().map(|p| p.label().to_string()).collect(),
                    on: puts.iter().position(|p| *p == self.put(st)).unwrap_or(0),
                    off: Vec::new(),
                    cols: 2,
                }),
                Row::Split | Row::Merge | Row::Clear | Row::Chop => {
                    let b = row.button().unwrap_or(Button::Split);
                    let text = self.button_text(st, b);
                    let look = if b == Button::ToPads {
                        Look::Primary(text)
                    } else {
                        Look::Button(text)
                    };
                    let why = self.button_why(st, b);
                    Control::new(look).faint_because(why.is_some(), &why.unwrap_or_default())
                }
            })
            .collect()
    }

    /// The panel: Slice and Put on side by side in the band under the
    /// wave, up to the Sound section (the mockup's Slice and Put on).
    fn panel(&self, st: &ChopState) -> Panel {
        let it = |k: Row| Item::new(k.index());
        let n = self.slices();
        let slices = format!(
            "{n} slice{} · {}",
            if n == 1 { "" } else { "s" },
            match self.method {
                Method::Manual => "by hand".to_string(),
                Method::Live => "live".to_string(),
                m => format!("by {}", m.label().to_lowercase()),
            }
        );
        Panel {
            sections: vec![
                // As drawn: Method over Snap to zero beside the setting's
                // knob, then the slice picked and the buttons.
                Section::new(
                    "Slice",
                    330,
                    vec![
                        vec![
                            it(Row::Method).labelled(),
                            it(Row::Snap).under(),
                            it(Row::Amount).wide(92),
                        ],
                        vec![it(Row::Slice)],
                        vec![it(Row::Split), it(Row::Merge), it(Row::Clear)],
                    ],
                )
                .note(&slices),
                // As drawn: the bank's pad map over its ‹ Bank ›, beside
                // the places, Phrase and the main button.
                Section::new(
                    "Put on",
                    420,
                    vec![vec![
                        it(Row::Pads).wide(MAP_W),
                        it(Row::Bank).under().high(BANK_H),
                        it(Row::Put),
                        it(Row::Phrase).under(),
                        it(Row::Chop).under().high(PUT_BUTTON_H),
                    ]],
                )
                .note(&self.put_note(st)),
            ],
        }
    }

    /// The methods offered: Beat only for a sound with a tempo.
    fn methods(st: &ChopState) -> Vec<Method> {
        Method::ALL
            .iter()
            .copied()
            .filter(|m| *m != Method::Beat || st.frames_per_beat().is_some())
            .collect()
    }

    /// Pads `p` as "A6–A13".
    fn range_text(&self, p: &Range<usize>) -> String {
        let (a, b) = (pad_text(self.bank, p.start), pad_text(self.bank, p.end - 1));
        if p.len() == 1 {
            a
        } else {
            format!("{a}–{b}")
        }
    }

    /// The method's setting: its name, its value and its place round the
    /// knob.
    pub fn amount(&self) -> (&'static str, String, f32) {
        match self.method {
            Method::Transient => (
                "Sensitivity",
                format!("{} %", self.sensitivity),
                f32::from(self.sensitivity) / 100.0,
            ),
            Method::Beat => (
                "Length",
                BEAT_LENGTHS[self.beat_len].1.into(),
                self.beat_len as f32 / (BEAT_LENGTHS.len() - 1) as f32,
            ),
            Method::Grid => (
                "Slices",
                self.grid.to_string(),
                (self.grid - 2) as f32 / (MAX_CUT_SLICES - 2) as f32,
            ),
            Method::Silence => {
                let (lo, hi) = THRESHOLD_RANGE_DB;
                (
                    "Threshold",
                    format!("{} dB", self.threshold_db),
                    (self.threshold_db - lo) as f32 / (hi - lo) as f32,
                )
            }
            Method::Pitch => {
                let (lo, hi) = PITCH_CHANGES;
                (
                    "Change",
                    match self.semitones {
                        1 => "1 semi".into(),
                        12 => "Octave".into(),
                        n => format!("{n} semis"),
                    },
                    f32::from(self.semitones - lo) / f32::from(hi - lo),
                )
            }
            Method::Manual => ("Setting", "—".into(), 0.0),
            Method::Live => (
                "Pull to hits",
                match HIT_WINDOWS_MS[self.hit_window] {
                    0 => "Off".into(),
                    ms => format!("{ms} ms"),
                },
                self.hit_window as f32 / (HIT_WINDOWS_MS.len() - 1) as f32,
            ),
        }
    }

    fn rows(&self, st: &ChopState) -> Vec<Setting> {
        let (amount, value, _) = self.amount();
        let row = |name: &str, value: String| Setting {
            name: name.into(),
            value,
        };
        Row::ALL
            .iter()
            .map(|r| match r {
                Row::Method => row("Method", self.method.label().into()),
                Row::Snap => row("Snap to zero", if self.snap { "On" } else { "Off" }.into()),
                Row::Amount => row(amount, value.clone()),
                // Drawn with no name over it: the value says it.
                Row::Slice => row(
                    "Slice",
                    format!("Slice {} of {}", self.selected + 1, self.slices()),
                ),
                Row::Split => row("Split", self.button_text(st, Button::Split)),
                Row::Merge => row("Merge", self.button_text(st, Button::Merge)),
                Row::Clear => row("Clear", self.button_text(st, Button::Clear)),
                Row::Put => row(
                    "Put on",
                    match self.put(st) {
                        Put::Keys => format!("Keys from {}", note_name(SLICE_KEYS_FROM)),
                        p => match p {
                            Put::OnePad => "One pad",
                            Put::Library => "Library",
                            Put::NewKit => "New kit",
                            Put::Audio => "Audio track",
                            _ => "Pads",
                        }
                        .into(),
                    },
                ),
                Row::Pads => row(
                    "Pads",
                    match self.pads(st) {
                        _ if !self.on_pads(st) => "None".into(),
                        Some(p) => self.range_text(&p),
                        None => "Too many".into(),
                    },
                ),
                // Named by the pads it shows, not "Bank B": it turns the
                // map alone, never the pads' bank (PAD BANK's), so the two
                // can't be taken for one setting.
                Row::Bank => row("Map", {
                    let b = (b'A' + self.bank_shown(st) as u8) as char;
                    format!("{b}1–{b}16")
                }),
                Row::Phrase if self.put(st) == Put::Audio => {
                    row("Goes", CELL_SWITCH[usize::from(self.cell)].into())
                }
                Row::Phrase => row(
                    "Phrase",
                    if self.phrase_on(st) { "On" } else { "Off" }.into(),
                ),
                Row::Chop => row("Chop", self.button_text(st, Button::ToPads)),
            })
            .collect()
    }

    /// The Q-Link cards: the method's setting, the slice picked, its start
    /// and the first pad.
    pub fn cards(&self, st: &ChopState) -> [Card; 4] {
        let bounds = self.bounds(st);
        let n = self.slices();
        let (amount, value, pos) = self.amount();
        let card = |k: usize, label: &str| Card {
            label: format!("Q-Link {} · {label}", k + 1),
            value: String::new(),
            sub: String::new(),
            pos: 0.0,
            enabled: true,
            why: String::new(),
        };
        let mut setting = card(0, amount);
        (setting.value, setting.pos) = (value, pos);
        if self.method == Method::Manual {
            setting.enabled = false;
            setting.why = "Manual chops have no setting: pick another Method".to_string();
        }
        let mut slice = card(1, "Slice");
        slice.value = (self.selected + 1).to_string();
        slice.sub = format!("of {n}");
        slice.pos = if n > 1 {
            self.selected as f32 / (n - 1) as f32
        } else {
            0.0
        };
        let mut start = card(2, "Slice start");
        let f = bounds[self.selected];
        let (v, sub) = position_text(f, st.rate, st.source.tempo);
        (start.value, start.sub, start.pos) = (v, sub, (f as f64 / st.frames.max(1) as f64) as f32);
        if self.selected == 0 {
            start.enabled = false;
            start.why = "The first slice starts where the sound does: Start, with Trim".to_string();
        }
        let mut first = card(3, "First pad");
        let room = st.room();
        match self.pads(st) {
            _ if self.put(st) == Put::NewKit => {
                first.enabled = false;
                first.why = "On a new kit the slices start on A1".to_string();
            }
            _ if self.put(st) == Put::Audio => {
                first.enabled = false;
                first.why = AUDIO_NO_PAD.to_string();
            }
            Some(p) => {
                first.value = pad_text(self.bank, p.start);
                if p.len() > 1 {
                    first.sub = format!("to {}", pad_text(self.bank, p.end - 1));
                }
                let free = room.len() - p.len();
                first.pos = if free > 0 {
                    (p.start - room.start) as f32 / free as f32
                } else {
                    0.0
                };
            }
            None => {
                first.enabled = false;
                first.why = "The slices don't fit on the pads: fewer slices".to_string();
            }
        }
        [setting, slice, start, first]
    }

    /// Why the last touch changed nothing, once (a drag on a tab that
    /// stays put).
    pub fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    /// Tab `k` dragged to `frame` (`px` frames a pixel at the zoom shown):
    /// tab k starts slice k; the first and END are the region's Start and
    /// End, which stay put. Returns where the tab is now.
    pub fn dragged(&mut self, st: &ChopState, k: usize, frame: u64, px: f64) -> Option<u64> {
        if k >= 1 && k <= self.cuts.len() {
            let to = self.snapped(st, frame, 0, px);
            self.move_cut(st, k - 1, to);
            self.selected = k;
        } else {
            // Say so rather than do nothing (Jay, 13 Nov).
            self.note = Some(
                if k == 0 {
                    "Slice 1 starts at the sound's Start: move it with Trim"
                } else {
                    "END is the sound's End: move it with Trim"
                }
                .to_string(),
            );
        }
        self.bounds(st).get(k).copied()
    }

    /// A tap on the wave at source frame `f`: picks the slice under it and
    /// plays it once.
    pub fn tap(&mut self, st: &ChopState, f: u64) -> Option<ChopAction> {
        let b = self.bounds(st);
        let i = (0..self.slices()).find(|&i| f >= b[i] && f < b[i + 1])?;
        self.selected = i;
        Some(ChopAction::Audition {
            pad: st.pad,
            layer: st.layer,
            start: b[i],
            end: b[i + 1],
        })
    }

    /// A button pressed (as a tap on it would); `px` is frames a pixel at
    /// the zoom shown.
    pub fn press(&mut self, st: &ChopState, b: Button, px: f64) -> Option<ChopAction> {
        let bounds = self.bounds(st);
        if let Some(why) = self.button_why(st, b) {
            self.note = Some(why);
            return None;
        }
        match b {
            Button::Split if self.method == Method::Live => {
                let on = self.live.is_none();
                self.live = on.then(|| {
                    st.source
                        .onsets
                        .as_ref()
                        .map_or_else(Vec::new, |o| o.pick(LIVE_HITS_SENSITIVITY))
                });
                if on {
                    // A new take: the old cuts go.
                    self.cuts.clear();
                    self.selected = 0;
                }
                self.stop_due = false;
                Some(ChopAction::Live {
                    pad: st.pad,
                    layer: st.layer,
                    start: st.region.start,
                    end: st.region.end,
                    on,
                })
            }
            Button::Split => {
                let (s0, s1) = (bounds[self.selected], bounds[self.selected + 1]);
                let min = wave_tab::MIN_FRAMES;
                let mid = self.snapped(st, s0 + (s1 - s0) / 2, 0, px);
                if self.slices() < MAX_CUT_SLICES && mid >= s0 + min && mid + min <= s1 {
                    let at = self.cuts.partition_point(|&c| c < mid);
                    self.cuts.insert(at, mid);
                    self.hand_edited();
                }
                None
            }
            Button::Merge => {
                // Join with the slice before; the first joins the next.
                let cut = self.selected.max(1) - 1;
                self.cuts.remove(cut);
                self.selected = cut;
                self.hand_edited();
                None
            }
            Button::Clear => {
                self.cuts.clear();
                self.selected = 0;
                self.hand_edited();
                self.take_stop(st)
            }
            Button::ToPads if self.put(st) == Put::Library => Some(ChopAction::Export {
                pad: st.pad,
                layer: st.layer,
                cuts: self.cuts.clone(),
            }),
            Button::ToPads if self.put(st) == Put::Audio => Some(ChopAction::ToAudio {
                pad: st.pad,
                layer: st.layer,
                name: st.pad_name.clone(),
                // In a cell: the whole sound, its slices aside.
                bounds: if self.cell {
                    vec![st.region.start, st.region.end]
                } else {
                    bounds
                },
                cell: self.cell,
            }),
            Button::ToPads => {
                let pads = self.pads(st)?;
                // Chopping this pad again replaces these slices (not on a
                // new kit: those are in another kit).
                if self.put(st) != Put::NewKit {
                    self.first_pad = Some((st.pad, pads.start));
                }
                Some(ChopAction::ToPads {
                    pad: st.pad,
                    layer: st.layer,
                    source: st.source.id,
                    name: st.pad_name.clone(),
                    mode: self.method.mode(),
                    bounds,
                    to_pad: pads.start,
                    keys_from: (self.put(st) == Put::Keys).then_some(SLICE_KEYS_FROM),
                    one_pad: self.put(st) == Put::OnePad,
                    new_kit: self.put(st) == Put::NewKit,
                    phrase: st.frames_per_beat().filter(|_| self.phrase_on(st)),
                })
            }
        }
    }

    /// A cut made, moved or taken out by hand: the method becomes Manual
    /// (Live stays Live, its cuts being hand-made too; Clear stops it).
    fn hand_edited(&mut self) {
        if self.method != Method::Live {
            self.method = Method::Manual;
        }
    }

    /// Moves cut `i` as near `to` as its neighbours allow; the method
    /// becomes Manual.
    fn move_cut(&mut self, st: &ChopState, i: usize, to: u64) {
        let b = self.bounds(st);
        let min = wave_tab::MIN_FRAMES;
        // Cut i is boundary i + 1, between boundaries i and i + 2.
        let (lo, hi) = (b[i] + min, b[i + 2].saturating_sub(min));
        if lo <= hi {
            let to = to.clamp(lo, hi);
            if to != self.cuts[i] {
                self.cuts[i] = to;
                self.hand_edited();
            }
        }
    }

    /// A setting stepped `delta` steps: an arrow or - / + (one), the
    /// setting's knob dragged (as many as it passed), a switch tapped, or
    /// a Put on button tapped (from the one on to that one). Returns the
    /// [`ChopAction::Live`] that stops live chop, when a change stopped it.
    pub fn step(&mut self, st: &ChopState, row: Row, delta: i32) -> Option<ChopAction> {
        let d = delta;
        if d == 0 {
            return None;
        }
        match row {
            Row::Method => {
                let all = Self::methods(st);
                let i = all.iter().position(|m| *m == self.method).unwrap_or(0) as i32;
                self.method = all[(i + d).rem_euclid(all.len() as i32) as usize];
                if self.live.take().is_some() {
                    self.stop_due = true;
                }
                self.rechop(st);
            }
            Row::Amount => self.turn_amount(st, d),
            Row::Snap => {
                self.snap ^= d % 2 != 0;
                if matches!(self.method, Method::Beat | Method::Grid | Method::Pitch) {
                    self.rechop(st);
                }
            }
            Row::Slice => {
                self.selected = (self.selected as i32 + d).clamp(0, self.cuts.len() as i32) as usize
            }
            Row::Pads => self.turn_first_pad(st, d),
            // Round the banks, as Method and Put on go round theirs.
            Row::Bank => {
                let b = (self.bank_shown(st) as i32 + d).rem_euclid(BANKS as i32) as usize;
                self.map_bank = Some((st.pad, b));
            }
            Row::Phrase if self.put(st) == Put::Audio => self.cell ^= d % 2 != 0,
            Row::Phrase => match self.phrase_why(st) {
                None => self.phrase ^= d % 2 != 0,
                Some(why) => self.note = Some(why.into()),
            },
            Row::Put => {
                let all = Self::puts(st);
                let i = all.iter().position(|p| *p == self.put(st)).unwrap_or(0) as i32;
                self.put = all[(i + d).rem_euclid(all.len() as i32) as usize];
            }
            // Buttons: pressed, not stepped.
            Row::Split | Row::Merge | Row::Clear | Row::Chop => {}
        }
        self.take_stop(st)
    }

    fn rechop(&mut self, st: &ChopState) {
        self.cuts = self.chop(st);
        self.selected = self.selected.min(self.cuts.len());
    }

    /// The method's setting turned by `delta` steps.
    fn turn_amount(&mut self, st: &ChopState, delta: i32) {
        match self.method {
            Method::Transient => {
                let s = i32::from(self.sensitivity) + delta * i32::from(SENSITIVITY_STEP);
                self.sensitivity = s.clamp(0, 100) as u8;
            }
            Method::Beat => {
                self.beat_len =
                    (self.beat_len as i32 + delta).clamp(0, BEAT_LENGTHS.len() as i32 - 1) as usize
            }
            Method::Grid => {
                self.grid = (self.grid as i32 + delta).clamp(2, MAX_CUT_SLICES as i32) as usize
            }
            Method::Silence => {
                let (lo, hi) = THRESHOLD_RANGE_DB;
                self.threshold_db = (self.threshold_db + delta * THRESHOLD_STEP_DB).clamp(lo, hi);
            }
            Method::Pitch => {
                let (lo, hi) = PITCH_CHANGES;
                let n = i32::from(self.semitones) + delta;
                self.semitones = n.clamp(i32::from(lo), i32::from(hi)) as u8;
            }
            Method::Manual => return,
            Method::Live => {
                self.hit_window = (self.hit_window as i32 + delta)
                    .clamp(0, HIT_WINDOWS_MS.len() as i32 - 1)
                    as usize;
                return;
            }
        }
        self.rechop(st);
    }

    fn turn_first_pad(&mut self, st: &ChopState, delta: i32) {
        if matches!(self.put(st), Put::NewKit | Put::Audio) {
            // A new kit's slices start on A1; an audio track has no pads.
            return;
        }
        let Some(p) = self.pads(st) else { return };
        let room = st.room();
        let last = (room.end - p.len()) as i32;
        let first = (p.start as i32 + delta).clamp(room.start as i32, last) as usize;
        self.first_pad = Some((st.pad, first));
        // The map follows the slices to their bank.
        self.map_bank = None;
    }

    /// A pad of the map tapped (`col`, its place in the bank shown): the
    /// slices start there (on One pad, that pad takes them), as far on as
    /// they fit. Says why when the map is faint.
    pub fn cell(&mut self, st: &ChopState, col: usize) {
        if let Some(why) = self.map_why(st) {
            self.note = Some(why.to_string());
            return;
        }
        let Some(p) = self.pads(st) else { return };
        let room = st.room();
        let pad = (self.bank_shown(st) * BANK + col.min(BANK - 1)).min(room.end - 1);
        let last = room.end - p.len();
        let first = pad.clamp(room.start, last);
        if first == p.start {
            // Nothing to move: say where they go, so the tap isn't lost.
            let at = pad_text(self.bank, first);
            self.note = Some(match self.put(st) {
                Put::OnePad => format!("The slices go on {at} already"),
                _ => format!("Slice 1 lands on {at} already"),
            });
            return;
        }
        self.first_pad = Some((st.pad, first));
    }

    /// A Q-Link turned: `knob` 0 turns the method's setting, 1 picks a
    /// slice, 2 moves its start and 3 the first pad, by `delta` detents.
    /// `fine` (SHIFT held) moves a start one frame per detent, unsnapped;
    /// `px` is frames a pixel at the zoom shown.
    pub fn qlink(&mut self, st: &ChopState, knob: usize, delta: i32, fine: bool, px: f64) {
        if delta == 0 {
            return;
        }
        match knob {
            0 => self.turn_amount(st, delta),
            1 => {
                self.selected =
                    (self.selected as i32 + delta).clamp(0, self.cuts.len() as i32) as usize
            }
            2 if self.selected > 0 => {
                let now = self.cuts[self.selected - 1];
                let to = if fine {
                    now.saturating_add_signed(i64::from(delta))
                } else {
                    let raw = (now as f64 + f64::from(delta) * px.max(1.0))
                        .round()
                        .max(0.0) as u64;
                    self.snapped(st, raw, delta.signum(), px)
                };
                self.move_cut(st, self.selected - 1, to);
            }
            3 => self.turn_first_pad(st, delta),
            _ => {}
        }
    }

    /// `to` on a zero crossing when snapping: the nearest (`dir` 0, within
    /// eight pixels at `px` frames a pixel) or the next one in the
    /// direction of `dir`.
    fn snapped(&self, st: &ChopState, to: u64, dir: i32, px: f64) -> u64 {
        let to = to.min(st.frames);
        let Some(x) = &st.source.mono else {
            return to;
        };
        if !self.snap || x.len() < 2 || to >= x.len() as u64 {
            return to;
        }
        let at = to as usize;
        let found = match dir {
            0 => nearest_zero_crossing(x, at, ZERO_RADIUS.max((px * 8.0) as u64) as usize),
            d if d > 0 => next_zero_crossing(x, at, ZERO_AHEAD),
            _ => previous_zero_crossing(x, at, ZERO_AHEAD),
        };
        found.map_or(to, |f| f as u64)
    }
}
