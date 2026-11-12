//! The pad editor's Chop tab: cutting a pad's sound into slices and putting
//! each slice on a pad of its own.
//!
//! - The tab chops what the pad plays: its region, from Start to End. The
//!   **waveform** shows the region bright, a marker at the start of every
//!   slice (numbered from 1) and END, and shades the slice picked.
//! - The **Chop** list holds the method and its setting: **Transient**
//!   (where the hits start, at a sensitivity), **Beat** (on the source's
//!   beat grid, a slice every 16th, 8th, beat, two beats or bar), **Grid**
//!   (equal slices), **Silence** (one slice per sound, below a threshold)
//!   and **Manual** (the cuts as they are), and **Live**: **Start** plays
//!   the region once and every pad played meanwhile drops a cut where the
//!   sound is (the sampler marks it to the frame, [`ChopAction::Live`]),
//!   moved onto the nearest hit within the **Pull to hits** window; Stop,
//!   or the sound's end, ends it. Changing the method or its
//!   setting chops again; moving, splitting or merging a slice by hand turns
//!   the method to Manual, so nothing undoes the hand edits behind the
//!   user's back.
//! - A **tap** on the wave picks the slice under it and plays it once, as
//!   the pad made from it will ([`ChopAction::Audition`]).
//! - **Split** cuts the slice picked in two, **Merge** joins it to the one
//!   before, and **Chop to pads** puts the slices on consecutive pads of the
//!   pad's bank as one undo step. Each new pad is the chopped pad (level,
//!   envelopes, filter, its sound's gain, tune and channels) playing one
//!   slice once. The button names the pads it fills; the list says how many
//!   of them have a sound already. With **Put on** at Keys, the button
//!   reads **To keys** and also makes each slice a keygroup of one note,
//!   from C2 up, and the kit's pads then play those notes, so the slices
//!   are on the pads and on a keyboard. With **Pads + phrase** (offered
//!   for a sound with a tempo) the button reads **Phrase on** and the app
//!   also makes a clip of the slices in time, from the first one, at the
//!   sound's tempo ([`ChopAction::phrase`]): played from a Clips slot, the
//!   loop comes back whole at any tempo, every hit a note to move or swap.
//!   With **Files** (offered where the app can write files) the button
//!   reads **Export** and gives the pad as files: its sounds with the
//!   slice points in them, a sidecar that brings the pad back, and each
//!   slice ([`ChopAction::Export`]).
//! - **Q-Link 1** picks a slice, **Q-Link 2** moves its start (one pixel of
//!   the current zoom per detent, snapped in the direction of the turn; with
//!   SHIFT one frame), **Q-Link 3** turns the method's setting and **Q-Link
//!   4** the first pad. The **wheel** zooms round the slice picked.
//!
//! The cuts live in the tab until they go on pads, so trying methods
//! changes nothing in the project. The app makes a [`ChopState`] for the pad
//! on show, calls [`ChopTab::update`] when the project changes, and applies
//! the [`ChopAction`]s it gets back ([`ChopAction::apply`]).

use std::ops::Range;
use std::sync::Arc;

use mpfree_analysis::{
    nearest_zero_crossing, note_name, previous_zero_crossing, sound_spans, SilenceSettings,
};
use mpfree_sampler::model::{
    ChopMode, ChopSet, ModelError, Project, Region, SourceId, PADS, SLICE_KEYS_FROM,
};
use mpfree_sampler::phrase::{self, Phrase};
use mpfree_ui::layout;
use mpfree_ui::wave::{Beats, Marker, MarkerKind, WaveOverview, WaveView, Waveform};
use mpfree_ui::{EventKind, Rect, Ui, UiEvent};

use crate::source::SourceView;
use crate::wave_tab::{
    self, duration_text, next_zero_crossing, pad_color, pad_label, position_text, TABS,
};
use crate::widgets::{ButtonRow, InfoCard, KnobCard, PadBar, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-chop-bar";
pub const OVERVIEW: &str = "smp-chop-overview";
pub const WAVE: &str = "smp-chop-wave";
pub const LIST: &str = "smp-chop-list";
pub const BUTTONS: &str = "smp-chop-buttons";
pub const QLINKS: [&str; 4] = ["smp-chop-q1", "smp-chop-q2", "smp-chop-q3", "smp-chop-q4"];
pub const WHEEL: &str = "smp-chop-wheel";

/// The tab's page: as the Wave tab's, with the Chop list and its buttons
/// beside the waveform.
pub const LAYOUT: &str = "col(header:60, row(smp-chop-bar:*, smp-send:330):60, smp-chop-overview:56, \
                          row(smp-chop-wave:*, col(smp-chop-list:*, smp-chop-buttons:64):360):*, \
                          row(smp-chop-q1, smp-chop-q2, smp-chop-q3, smp-chop-q4, smp-chop-wheel:210):88, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = 2;

/// The hint line the app shows under the tab.
pub const HINT: &str = "Tap a slice to pick and hear it · drag a marker to move a cut · Split, Merge, then Chop to pads (UNDO takes \
                        it back) · SHIFT + Q-Link 2: one frame at a time";

/// Pads in a bank: the most slices one chop can put on pads.
pub const BANK: usize = 16;
/// The most slices the tab cuts.
pub const MAX_CUT_SLICES: usize = 64;
/// Cuts a method makes are at least this far apart (and from the region's
/// ends), so no slice is a sliver.
const MIN_SLICE_MS: f64 = 10.0;
/// How far a dragged marker looks for a zero crossing: this many frames, or
/// eight pixels of the current zoom if that is more.
const ZERO_RADIUS: u64 = 256;
/// How far a Q-Link turn looks ahead for the next zero crossing.
const ZERO_AHEAD: usize = 8192;
const WHEEL_ZOOM: f64 = 1.25;
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

/// How the tab cuts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    #[default]
    Transient,
    Beat,
    Grid,
    Silence,
    Manual,
    /// Cuts dropped by hitting pads while the sound plays.
    Live,
}

impl Method {
    pub const ALL: [Method; 6] = [
        Method::Transient,
        Method::Beat,
        Method::Grid,
        Method::Silence,
        Method::Manual,
        Method::Live,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Method::Transient => "Transient",
            Method::Beat => "Beat",
            Method::Grid => "Grid",
            Method::Silence => "Silence",
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
            Method::Manual | Method::Live => ChopMode::Manual,
        }
    }
}

/// The Chop list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Method,
    /// The method's setting: sensitivity, length, slices or threshold.
    Amount,
    Snap,
    Slice,
    Pads,
    /// The slices go on the pads, on notes as well, or on the pads with a
    /// phrase that plays them in time.
    Put,
}

/// Where the Put row puts the slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Put {
    Pads,
    /// Keygroups of one note each from [`SLICE_KEYS_FROM`].
    Keys,
    /// The pads, and a clip of the slices in time (only for a sound with
    /// a tempo).
    Phrase,
    /// Files: the pad's sounds, its sidecar and each slice
    /// ([`ChopAction::Export`]; only where the app can write them).
    Files,
}

impl Row {
    pub const ALL: [Row; 6] = [
        Row::Method,
        Row::Amount,
        Row::Snap,
        Row::Slice,
        Row::Pads,
        Row::Put,
    ];
}

/// The buttons, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Split,
    Merge,
    /// The slices on the pads (or, as the Put row says, on keys too).
    ToPads,
}

impl Button {
    pub const ALL: [Button; 3] = [Button::Split, Button::Merge, Button::ToPads];
}

/// Everything the tab chops: one layer of one pad, its source, and which
/// pads of its bank have a sound.
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
    /// Which pads of the bank (the 16 the pad is among) have a sound.
    pub used: Vec<bool>,
    /// The app can write files (Put on Files is offered).
    pub files: bool,
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
        let pd = p.pad(pad)?;
        let region = pd.layers.get(layer)?.region.clone();
        let s = p.source(region.source)?;
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
            used: bank_of(pad).map(|i| p.pad(i).is_some()).collect(),
            files: false,
        })
    }

    /// The pads slices may go on: the 16 of the pad's bank.
    pub fn bank(&self) -> Range<usize> {
        bank_of(self.pad)
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

/// What a touch or control on the tab asks for. The app applies it.
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
    /// Live chop: play source frames `start..end` of pad `pad`'s layer
    /// `layer` once, and let the pads played meanwhile mark cuts in it
    /// (`on`); or stop that. The app sends the marks back
    /// ([`ChopTab::live_marks`]) and says when the sound ended
    /// ([`ChopTab::live_ended`]).
    Live {
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
        on: bool,
    },
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
}

impl ChopAction {
    /// Applies [`ChopAction::ToPads`] to the project (one undo step) and
    /// returns the pads it filled; other actions change nothing here.
    pub fn apply(&self, p: &mut Project) -> Result<Range<usize>, ModelError> {
        let ChopAction::ToPads {
            pad,
            layer,
            to_pad,
            keys_from,
            ..
        } = self
        else {
            return Ok(0..0);
        };
        let mut template = p.pad(*pad).ok_or(ModelError::BadPad)?.clone();
        if *layer >= template.layers.len() {
            return Err(ModelError::BadPad);
        }
        // The chopped layer's settings carry over: make it the first.
        template.layers.swap(0, *layer);
        let (c, first, count) = self.slices(p)?;
        match keys_from {
            Some(from) => p.chop_to_keys(c, first, count, *to_pad, *from, Some(&template))?,
            None => p.chop_to_pads(c, first, count, *to_pad, Some(&template))?,
        };
        Ok(*to_pad..*to_pad + count)
    }

    /// The phrase a [`ChopAction::ToPads`] asks for: its slices as hits in
    /// beats of the sound's tempo, the first slice on the first pad. None
    /// when it asks for none.
    pub fn phrase(&self, p: &Project) -> Result<Option<Phrase>, ModelError> {
        let ChopAction::ToPads {
            phrase: Some(frames_per_beat),
            ..
        } = self
        else {
            return Ok(None);
        };
        let (c, first, count) = self.slices(p)?;
        phrase::phrase(&c, first, count, *frames_per_beat).map(Some)
    }

    /// The chop a [`ChopAction::ToPads`] makes, and which of its slices
    /// (from `first`, `count` of them) lie between its bounds.
    fn slices(&self, p: &Project) -> Result<(ChopSet, usize, usize), ModelError> {
        let ChopAction::ToPads {
            source,
            name,
            mode,
            bounds,
            ..
        } = self
        else {
            return Err(ModelError::BadSlice);
        };
        if bounds.len() < 2 || bounds.windows(2).any(|w| w[0] >= w[1]) {
            return Err(ModelError::BadSlice);
        }
        let s = p.source(*source).ok_or(ModelError::UnknownSource)?;
        let c = ChopSet::from_markers(name, s, *mode, bounds.iter().copied())?;
        let first = usize::from(bounds[0] > 0);
        let count = bounds.len() - 1;
        if c.bounds(first).map(|b| b.0) != Some(bounds[0])
            || c.bounds(first + count - 1).map(|b| b.1) != bounds.last().copied()
        {
            return Err(ModelError::BadSlice);
        }
        Ok((c, first, count))
    }
}

/// `pad` named in the bank the app shows, as "A5".
pub(crate) fn pad_text(bank: Option<char>, pad: usize) -> String {
    match bank {
        Some(b) => format!("{b}{}", pad % BANK + 1),
        None => pad_label(pad),
    }
}

/// The Chop tab's widgets and the chop being made: the method and its
/// settings, the cuts and the slice picked. Everything else comes from the
/// [`ChopState`].
pub struct ChopTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`wave_tab::WaveTab::bank`].
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
    /// Live chop is on: the hits its cuts snap to.
    live: Option<Vec<u64>>,
    /// Live chop was stopped by a change here: the app hears of it with
    /// the next action.
    stop_due: bool,
    /// Snap cuts made by hand, and by Beat and Grid, to zero crossings.
    snap: bool,
    /// Slice starts strictly inside the region, sorted.
    cuts: Vec<u64>,
    selected: usize,
    /// The first pad chosen with Q-Link 4 or the Pads row; None takes the
    /// pads after the chopped one.
    first_pad: Option<usize>,
    /// The Put row: the pads; also notes from [`SLICE_KEYS_FROM`], as
    /// keygroups of a note each, the pads then playing those notes; or the
    /// pads and a phrase of them in time.
    put: Put,
    /// The source and region the cuts were made for: another pad, or a
    /// moved Start or End, chops again.
    made_for: Option<(SourceId, u64, u64)>,
    shown: Option<SourceId>,
    wave_w: i32,
}

impl ChopTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &ChopState) -> Result<ChopTab, String> {
        crate::send::add(ui);
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        ui.add(OVERVIEW, WaveOverview::new());
        ui.add(WAVE, Waveform::new());
        ui.add(LIST, SettingList::new("Chop"));
        let mut buttons = ButtonRow::new(&["Split", "Merge", "Chop to pads"]);
        buttons.primary = Some(2);
        ui.add(BUTTONS, buttons);
        let t = ui.theme.clone();
        let cards = [
            ("Slice", t.accent),
            ("Slice start", t.text),
            ("Sensitivity", t.good),
            ("First pad", t.warn),
        ];
        for (k, (label, color)) in cards.iter().enumerate() {
            ui.add(
                QLINKS[k],
                KnobCard::new(&format!("Q-Link {} · {label}", k + 1), *color),
            );
        }
        ui.add(WHEEL, InfoCard::new("Wheel · zoom"));
        ui.add_page(name, LAYOUT)?;
        let wave = slot(ui, WAVE).ok_or("no waveform slot")?;
        let mut tab = ChopTab {
            name: name.to_string(),
            bank: None,
            method: Method::default(),
            sensitivity: 50,
            beat_len: 2,
            grid: 8,
            threshold_db: SilenceSettings::default().threshold_db as i32,
            hit_window: 3,
            live: None,
            stop_due: false,
            snap: true,
            cuts: Vec::new(),
            selected: 0,
            first_pad: None,
            put: Put::Pads,
            made_for: None,
            shown: None,
            wave_w: Waveform::wave_rect(wave).w,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    pub fn method(&self) -> Method {
        self.method
    }

    /// The slice picked, from 0.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Where the slices go: a phrase only while the sound has a tempo.
    fn put(&self, st: &ChopState) -> Put {
        match self.put {
            Put::Phrase if st.frames_per_beat().is_none() => Put::Pads,
            Put::Files if !st.files => Put::Pads,
            p => p,
        }
    }

    /// What the Put on row says.
    pub fn put_text(&self) -> &'static str {
        match self.put {
            Put::Pads => "Pads",
            Put::Keys => "Keys",
            Put::Phrase => "Pads + phrase",
            Put::Files => "Files",
        }
    }

    /// The Put on choices for `st`: Pads + phrase with a tempo, Files
    /// where the app writes them.
    fn puts(st: &ChopState) -> Vec<Put> {
        let mut all = vec![Put::Pads, Put::Keys];
        if st.frames_per_beat().is_some() {
            all.push(Put::Phrase);
        }
        if st.files {
            all.push(Put::Files);
        }
        all
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

    /// The pads Chop to pads fills, if the slices fit in the bank.
    pub fn pads(&self, st: &ChopState) -> Option<Range<usize>> {
        let n = self.slices();
        let bank = st.bank();
        if n > bank.len() {
            return None;
        }
        let last = bank.end - n;
        let first = match self.first_pad {
            Some(f) => f.clamp(bank.start, last),
            None if st.pad < last => st.pad + 1,
            None => last,
        };
        Some(first..first + n)
    }

    /// The pads Chop to pads fills that have a sound already.
    pub fn replaced(&self, st: &ChopState) -> Vec<usize> {
        let bank = st.bank();
        self.pads(st).map_or_else(Vec::new, |r| {
            r.filter(|&i| st.used.get(i - bank.start).copied().unwrap_or(false))
                .collect()
        })
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
    pub fn live_marks(&mut self, ui: &mut Ui, st: &ChopState, marks: &[u64]) -> usize {
        let Some(hits) = self.live.as_ref() else {
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
            let c = near.unwrap_or_else(|| self.snapped(st, m, 0, None));
            let apart = self.cuts.iter().all(|&x| x.abs_diff(c) >= gap);
            if c < rs + gap || c + gap > re || !apart || self.slices() >= MAX_CUT_SLICES {
                continue;
            }
            let at = self.cuts.partition_point(|&x| x < c);
            self.cuts.insert(at, c);
            self.selected = at + 1;
            kept += 1;
        }
        if kept > 0 {
            self.show(ui, st);
        }
        kept
    }

    /// The sound live chop played ended: Start again starts a new take.
    pub fn live_ended(&mut self, ui: &mut Ui, st: &ChopState) {
        if self.live.take().is_some() {
            self.show(ui, st);
        }
    }

    /// The waveform's view.
    pub fn view(&self, ui: &Ui) -> Option<WaveView> {
        ui.get::<Waveform>(WAVE).map(Waveform::view)
    }

    /// Where the pad's voices are playing, in source frames.
    pub fn set_playheads(&self, ui: &mut Ui, frames: &[u64]) {
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            w.set_playheads(frames);
        }
    }

    /// Shows `st`, chopping again if it is another pad or region.
    pub fn update(&mut self, ui: &mut Ui, st: &ChopState) {
        let key = (st.source.id, st.region.start, st.region.end);
        if self.made_for != Some(key) {
            let same_source = self.made_for.is_some_and(|k| k.0 == key.0);
            self.made_for = Some(key);
            if self.method == Method::Beat && st.frames_per_beat().is_none() {
                self.method = Method::Transient;
            }
            if self.live.take().is_some() {
                // Another pad or region: live chop stops.
                self.stop_due = true;
            }
            if matches!(self.method, Method::Manual | Method::Live) && same_source {
                // Keep the hand-made cuts that are still inside.
                let (lo, hi) = (
                    st.region.start + wave_tab::MIN_FRAMES,
                    st.region.end.saturating_sub(wave_tab::MIN_FRAMES),
                );
                self.cuts.retain(|&c| c >= lo && c <= hi);
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
        }
        self.selected = self.selected.min(self.cuts.len());
        self.show(ui, st);
    }

    /// The cuts the method makes in the region.
    fn chop(&self, st: &ChopState) -> Vec<u64> {
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
                    .map(|k| self.snapped(st, (k as f64 * step).round() as u64, 0, None))
                    .collect()
            }
            Method::Grid => {
                let n = self.grid.min(((re - rs) / st.min_slice()).max(1) as usize) as u64;
                (1..n)
                    .map(|k| self.snapped(st, rs + (re - rs) * k / n, 0, None))
                    .collect()
            }
            Method::Silence => {
                let settings = SilenceSettings {
                    threshold_db: self.threshold_db as f32,
                    ..SilenceSettings::default()
                };
                sound_spans(&st.source.mono, 1, st.rate, settings)
                    .map_or_else(|_| Vec::new(), |s| s.iter().map(|s| s.0).collect())
            }
        };
        // Inside the region, apart, and no more than the tab cuts.
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

    fn show(&self, ui: &mut Ui, st: &ChopState) {
        let r = &st.region;
        let src = &st.source;
        let bounds = self.bounds(st);
        let n = self.slices();
        let pads = self.pads(st);
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.pad);
            bar.active = TAB;
            bar.title = format!("Pad {} · {}", pad_text(self.bank, st.pad), st.pad_name);
            let mut about = vec![
                format!("{n} slice{}", if n == 1 { "" } else { "s" }),
                self.method.label().to_string(),
            ];
            if let Some(bpm) = src.tempo {
                about.push(format!("{bpm:.1} BPM"));
            }
            about.push(format!("region {}", duration_text(r.frames(), st.rate)));
            if st.source_name != st.pad_name {
                about.push(format!("source {}", st.source_name));
            }
            bar.subtitle = about.join(" · ");
        }
        let width = self.wave_w;
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            if self.shown != Some(src.id) {
                w.set_peaks(Some(src.peaks.clone()), width);
            }
            w.set_markers(markers(&bounds, self.selected));
            w.set_region(Some((r.start, r.end)));
            w.set_loop(Some((bounds[self.selected], bounds[self.selected + 1])));
            w.set_fades(0, 0);
            w.set_beats(st.frames_per_beat().map(|fpb| Beats {
                first: 0.0,
                frames_per_beat: fpb,
                per_bar: 4,
            }));
        }
        if let Some(o) = ui.get_mut::<WaveOverview>(OVERVIEW) {
            o.set_peaks(Some(src.peaks.clone()));
            o.set_region(Some((r.start, r.end)));
        }
        self.sync_overview(ui);
        let replaced = self.replaced(st);
        let rows = self.rows(st);
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.right = match (pads.is_some(), replaced.len()) {
                (false, _) => format!("{n} slices: too many for a bank"),
                (true, 0) => "no pad replaced".into(),
                (true, 1..=3) => {
                    let names: Vec<String> =
                        replaced.iter().map(|&p| pad_text(self.bank, p)).collect();
                    format!("replaces {}", names.join(", "))
                }
                (true, k) => format!("replaces {k} pads"),
            };
            list.rows = rows;
        }
        let range = pads.clone().map(|p| self.range_text(&p));
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            let (s0, s1) = (bounds[self.selected], bounds[self.selected + 1]);
            let files = self.put(st) == Put::Files;
            let live = self.method == Method::Live;
            b.enabled = vec![
                live || s1 - s0 >= 2 * wave_tab::MIN_FRAMES && n < MAX_CUT_SLICES,
                n > 1,
                pads.is_some() || files,
            ];
            b.labels[0] = match (live, self.live.is_some()) {
                (false, _) => "Split",
                (true, false) => "Start",
                (true, true) => "Stop",
            }
            .into();
            b.labels[2] = match (&range, self.put(st)) {
                (_, Put::Files) if n == 1 => "Export the pad".to_string(),
                (_, Put::Files) => format!("Export {n} slices"),
                (None, _) => format!("Max {BANK} slices"),
                (Some(t), Put::Pads) => format!("Chop to {t}"),
                (Some(t), Put::Phrase) => format!("Phrase on {t}"),
                (Some(_), Put::Keys) => {
                    let k = SLICE_KEYS_FROM;
                    format!(
                        "To keys {}–{}",
                        note_name(k),
                        note_name(k.saturating_add(n as u8 - 1).min(127))
                    )
                }
            };
        }
        self.show_cards(ui, st);
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

    fn amount(&self) -> (&'static str, String, f32) {
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
        Row::ALL
            .iter()
            .map(|row| match row {
                Row::Method => Setting {
                    name: "Method".into(),
                    value: self.method.label().into(),
                },
                Row::Amount => Setting {
                    name: amount.into(),
                    value: value.clone(),
                },
                Row::Snap => Setting {
                    name: "Snap".into(),
                    value: if self.snap { "Zero crossing" } else { "Off" }.into(),
                },
                Row::Slice => Setting {
                    name: "Slice".into(),
                    value: format!("{} of {}", self.selected + 1, self.slices()),
                },
                Row::Pads => Setting {
                    name: "Pads".into(),
                    value: match self.pads(st) {
                        _ if self.put(st) == Put::Files => "None (files)".into(),
                        Some(p) => self.range_text(&p),
                        None => "Too many".into(),
                    },
                },
                Row::Put => Setting {
                    name: "Put on".into(),
                    value: match self.put(st) {
                        Put::Pads => "Pads".into(),
                        Put::Keys => format!("Keys from {}", note_name(SLICE_KEYS_FROM)),
                        Put::Phrase => "Pads + phrase".into(),
                        Put::Files => "Files".into(),
                    },
                },
            })
            .collect()
    }

    fn show_cards(&self, ui: &mut Ui, st: &ChopState) {
        let bounds = self.bounds(st);
        let n = self.slices();
        let (amount, value, pos) = self.amount();
        let pads = self.pads(st);
        let bank = st.bank();
        if let Some(c) = ui.get_mut::<KnobCard>(QLINKS[0]) {
            c.value = (self.selected + 1).to_string();
            c.sub = format!("of {n}");
            c.pos = if n > 1 {
                self.selected as f32 / (n - 1) as f32
            } else {
                0.0
            };
        }
        if let Some(c) = ui.get_mut::<KnobCard>(QLINKS[1]) {
            let f = bounds[self.selected];
            let (value, sub) = position_text(f, st.rate, st.source.tempo);
            (c.value, c.sub, c.pos) = (value, sub, (f as f64 / st.frames.max(1) as f64) as f32);
            // The first slice starts where the pad does: Start on the Wave tab.
            c.enabled = self.selected > 0;
        }
        if let Some(c) = ui.get_mut::<KnobCard>(QLINKS[2]) {
            c.label = format!("Q-Link 3 · {amount}");
            (c.value, c.sub, c.pos) = (value, String::new(), pos);
            c.enabled = self.method != Method::Manual;
        }
        if let Some(c) = ui.get_mut::<KnobCard>(QLINKS[3]) {
            match &pads {
                Some(p) => {
                    c.value = pad_text(self.bank, p.start);
                    c.sub = if p.len() > 1 {
                        format!("to {}", pad_text(self.bank, p.end - 1))
                    } else {
                        String::new()
                    };
                    let room = bank.len() - p.len();
                    c.pos = if room > 0 {
                        (p.start - bank.start) as f32 / room as f32
                    } else {
                        0.0
                    };
                    c.enabled = true;
                }
                None => c.enabled = false,
            }
        }
        self.update_wheel(ui, st);
    }

    fn update_wheel(&self, ui: &mut Ui, st: &ChopState) {
        let Some(view) = self.view(ui) else { return };
        let whole = st.frames.max(1) as f64 / f64::from(self.wave_w.max(1));
        let zoom = whole / view.frames_per_px.max(1e-9);
        if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
            card.text = format!("{zoom:.1}× · slice {}", self.selected + 1);
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
    /// that only change the chop or what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &ChopState) -> Option<ChopAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(ChopAction::Tab(*i)),
            (WAVE, EventKind::Marker { index, frame }) => {
                // Marker k starts slice k; the last is END. The first and END
                // are the pad's Start and End: they stay put.
                let k = *index;
                if k >= 1 && k <= self.cuts.len() {
                    let to = self.snapped(st, *frame, 0, Some(ui));
                    self.move_cut(st, k - 1, to);
                    self.selected = k;
                }
                let at = self.bounds(st).get(k).copied();
                if let (Some(w), Some(f)) = (ui.get_mut::<Waveform>(WAVE), at) {
                    w.set_marker_frame(k, f);
                }
                self.show(ui, st);
                None
            }
            (WAVE, EventKind::View { .. }) => {
                self.sync_overview(ui);
                self.update_wheel(ui, st);
                None
            }
            (WAVE, EventKind::Frame(f)) => {
                let b = self.bounds(st);
                let i = (0..self.slices()).find(|&i| *f >= b[i] && *f < b[i + 1])?;
                self.selected = i;
                self.show(ui, st);
                Some(ChopAction::Audition {
                    pad: st.pad,
                    layer: st.layer,
                    start: b[i],
                    end: b[i + 1],
                })
            }
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
            (LIST, EventKind::Step { index, delta }) => {
                let row = *Row::ALL.get(*index)?;
                self.step(st, row, *delta);
                self.show(ui, st);
                self.take_stop(st)
            }
            (BUTTONS, EventKind::Tab(i)) => self.press(ui, st, *Button::ALL.get(*i)?),
            _ => None,
        }
    }

    /// A button pressed (as a tap on it would).
    pub fn press(&mut self, ui: &mut Ui, st: &ChopState, b: Button) -> Option<ChopAction> {
        let bounds = self.bounds(st);
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
                self.show(ui, st);
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
                let mid = self.snapped(st, s0 + (s1 - s0) / 2, 0, Some(ui));
                if self.slices() < MAX_CUT_SLICES && mid >= s0 + min && mid + min <= s1 {
                    let at = self.cuts.partition_point(|&c| c < mid);
                    self.cuts.insert(at, mid);
                    self.hand_edited();
                }
                self.show(ui, st);
                None
            }
            Button::Merge => {
                if !self.cuts.is_empty() {
                    // Join with the slice before; the first joins the next.
                    let cut = self.selected.max(1) - 1;
                    self.cuts.remove(cut);
                    self.selected = cut;
                    self.hand_edited();
                }
                self.show(ui, st);
                None
            }
            Button::ToPads if self.put(st) == Put::Files => Some(ChopAction::Export {
                pad: st.pad,
                layer: st.layer,
                cuts: self.cuts.clone(),
            }),
            Button::ToPads => {
                let pads = self.pads(st)?;
                Some(ChopAction::ToPads {
                    pad: st.pad,
                    layer: st.layer,
                    source: st.source.id,
                    name: st.pad_name.clone(),
                    mode: self.method.mode(),
                    bounds,
                    to_pad: pads.start,
                    keys_from: (self.put(st) == Put::Keys).then_some(SLICE_KEYS_FROM),
                    phrase: st.frames_per_beat().filter(|_| self.put(st) == Put::Phrase),
                })
            }
        }
    }

    /// A cut made, moved or taken out by hand: the method becomes Manual
    /// (Live stays Live, its cuts being hand-made too).
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

    /// An arrow on a row of the Chop list.
    fn step(&mut self, st: &ChopState, row: Row, delta: i32) {
        let d = delta.signum();
        match row {
            Row::Method => {
                let all: Vec<Method> = Method::ALL
                    .iter()
                    .copied()
                    .filter(|m| *m != Method::Beat || st.frames_per_beat().is_some())
                    .collect();
                let i = all.iter().position(|m| *m == self.method).unwrap_or(0) as i32;
                self.method = all[(i + d).rem_euclid(all.len() as i32) as usize];
                if self.live.take().is_some() {
                    self.stop_due = true;
                }
                self.rechop(st);
            }
            Row::Amount => self.turn_amount(st, d),
            Row::Snap => {
                self.snap = !self.snap;
                if matches!(self.method, Method::Beat | Method::Grid) {
                    self.rechop(st);
                }
            }
            Row::Slice => {
                self.selected = (self.selected as i32 + d).clamp(0, self.cuts.len() as i32) as usize
            }
            Row::Pads => self.turn_first_pad(st, d),
            Row::Put => {
                let all = Self::puts(st);
                let i = all.iter().position(|p| *p == self.put(st)).unwrap_or(0) as i32;
                self.put = all[(i + d).rem_euclid(all.len() as i32) as usize];
            }
        }
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
        let Some(p) = self.pads(st) else { return };
        let bank = st.bank();
        let last = (bank.end - p.len()) as i32;
        self.first_pad = Some((p.start as i32 + delta).clamp(bank.start as i32, last) as usize);
    }

    /// A Q-Link turned: `knob` 0 picks a slice, 1 moves its start, 2 turns
    /// the method's setting and 3 the first pad, by `delta` detents. `fine`
    /// (SHIFT held) moves a start one frame per detent, unsnapped.
    pub fn qlink(&mut self, ui: &mut Ui, st: &ChopState, knob: usize, delta: i32, fine: bool) {
        if delta == 0 {
            return;
        }
        match knob {
            0 => {
                self.selected =
                    (self.selected as i32 + delta).clamp(0, self.cuts.len() as i32) as usize
            }
            1 if self.selected > 0 => {
                let now = self.cuts[self.selected - 1];
                let to = if fine {
                    now.saturating_add_signed(i64::from(delta))
                } else {
                    let px = self.view(ui).map_or(1.0, |v| v.frames_per_px).max(1.0);
                    let raw = (now as f64 + f64::from(delta) * px).round().max(0.0) as u64;
                    self.snapped(st, raw, delta.signum(), Some(ui))
                };
                self.move_cut(st, self.selected - 1, to);
            }
            2 => self.turn_amount(st, delta),
            3 => self.turn_first_pad(st, delta),
            _ => return,
        }
        self.show(ui, st);
    }

    /// The data wheel: zooms in (turned up) or out round the slice picked.
    pub fn wheel(&mut self, ui: &mut Ui, st: &ChopState, delta: i32) {
        let width = self.wave_w;
        let b = self.bounds(st);
        let at = (b[self.selected] + b[self.selected + 1]) as f64 / 2.0;
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            let v = w.view();
            let mid = f64::from(width) / 2.0;
            let x = v.x_of(at);
            let (frame, x) = if (0.0..=f64::from(width)).contains(&x) {
                (at, x)
            } else {
                (v.frame_at(mid), mid)
            };
            let nv = v.zoomed(WHEEL_ZOOM.powi(delta), frame, x, st.frames, width);
            w.set_view(nv, width);
        }
        self.sync_overview(ui);
        self.update_wheel(ui, st);
    }

    /// `to` on a zero crossing when snapping: the nearest (`dir` 0) or the
    /// next one in the direction of `dir`.
    fn snapped(&self, st: &ChopState, to: u64, dir: i32, ui: Option<&Ui>) -> u64 {
        let to = to.min(st.frames);
        let x = &st.source.mono;
        if !self.snap || x.len() < 2 || to >= x.len() as u64 {
            return to;
        }
        let at = to as usize;
        let found = match dir {
            0 => {
                let px = ui
                    .and_then(|ui| self.view(ui))
                    .map_or(1.0, |v| v.frames_per_px);
                nearest_zero_crossing(x, at, ZERO_RADIUS.max((px * 8.0) as u64) as usize)
            }
            d if d > 0 => next_zero_crossing(x, at, ZERO_AHEAD),
            _ => previous_zero_crossing(x, at, ZERO_AHEAD),
        };
        found.map_or(to, |f| f as u64)
    }
}

/// A marker at the start of every slice, numbered from 1, and END. The
/// first is the pad's Start; the slice picked is marked as a cue.
fn markers(bounds: &[u64], selected: usize) -> Vec<Marker> {
    let last = bounds.len() - 1;
    bounds
        .iter()
        .enumerate()
        .map(|(k, &f)| {
            let (label, kind) = match k {
                0 => ("1".to_string(), MarkerKind::Edge),
                k if k == last => ("END".to_string(), MarkerKind::Edge),
                k if k == selected => ((k + 1).to_string(), MarkerKind::Cue),
                k => ((k + 1).to_string(), MarkerKind::Slice),
            };
            Marker::new(f, &label, kind)
        })
        .collect()
}

/// Where `name` sits on a page laid out by [`LAYOUT`].
fn slot(ui: &Ui, name: &str) -> Option<Rect> {
    let node = layout::parse(LAYOUT).ok()?;
    let (w, h) = ui.size();
    let g = ui.theme.gap;
    layout::solve(&node, Rect::new(0, 0, w, h).inset(g), g)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| r)
}
