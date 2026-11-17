//! The pad editor's Grain tab: how a layer plays as a grain cloud.
//!
//! - The **cloud** shows the layer's region with the grains playing now
//!   over it: a band where grains may start (the position, spread by the
//!   spray), a line at the position, the voice's reading place, and a bar
//!   for each grain. A tap or drag on it moves the position.
//! - The **touch pad** moves the position (across) and the grain size (up)
//!   together, on a log scale from 2 ms to 2 s.
//! - The **Grain** list holds every setting: the play mode (only Granular
//!   plays grains), scan speed, spray, pitch spray, the window's shape,
//!   reverse chance, stereo spread, pitch quantise, tempo sync, and
//!   position, size, density, pitch and the cloud's level. A tap on a row
//!   picks it and its arrows step it; the **wheel** steps the row picked
//!   (Scan, until another is picked).
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
use mpfree_sampler::engine::GrainView;
use mpfree_sampler::model::{
    GrainMode, GrainParams, GrainSprayShape, GrainWindow, ModelError, Pad, PlayMode, Project,
    Region, Source, TempoSync,
};
use mpfree_ui::{Color, EventKind, Ui, UiEvent};

use crate::chop_tab::pad_text;
use crate::grain_widgets::{GrainCloud, XyPad, GRAIN_COLOR};
use crate::play_tab::LayerSound;
use crate::source::SourceView;
use crate::wave_tab::{self, duration_text, mode_label, pad_color, TABS};
use crate::widgets::{InfoCard, KnobCard, PadBar, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-grain-bar";
pub const CLOUD: &str = "smp-grain-cloud";
pub const XY: &str = "smp-grain-xy";
pub const LIST: &str = "smp-grain-list";
pub const QLINKS: [&str; 4] = [
    "smp-grain-q1",
    "smp-grain-q2",
    "smp-grain-q3",
    "smp-grain-q4",
];
pub const WHEEL: &str = "smp-grain-wheel";

/// The tab's page: the cloud, and beside it the touch pad over the Grain
/// list, between the pad bar and the Q-Link cards.
pub const LAYOUT: &str = "col(header:60, row(smp-grain-bar:*, smp-send:330):60, \
                          row(smp-grain-cloud:*, col(smp-grain-xy:236, smp-grain-list:*, smp-grain-wheel:56):340, \
                          col(smp-grain-q1, smp-grain-q2, smp-grain-q3, smp-grain-q4):128):*, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = 4;

/// The hint line the app shows under the tab.
pub const HINT: &str = "Hit the pad to hear the cloud · drag on the sound for the position, on the pad for position and \
                        size · the wheel picks a row, - / + change it · SHIFT + Q-Link 1 / 4: spray, pitch spray";

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

/// The Grain list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Mode,
    GrainMode,
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
}

impl Row {
    pub const ALL: [Row; 19] = [
        Row::Mode,
        Row::GrainMode,
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
        }
    }

    /// The rows a grain mode shows, in order: every row for a cloud;
    /// for a stretch, what moves it (no randomness, so its grains stay in
    /// step); for a repeat, its tempo-locked rate and length.
    pub fn shown(mode: GrainMode) -> &'static [Row] {
        match mode {
            GrainMode::Cloud => &Row::ALL,
            GrainMode::Stretch => &STRETCH_ROWS,
            GrainMode::Repeat => &REPEAT_ROWS,
            GrainMode::Tone => &TONE_ROWS,
        }
    }

    /// Where this row is in `mode`'s list, if it shows there.
    pub fn index_in(self, mode: GrainMode) -> Option<usize> {
        Row::shown(mode).iter().position(|r| *r == self)
    }
}

/// A stretch's rows: how fast it moves through the sound (Scan speed),
/// where, its grains' size, overlap and shape, its pitch and its width.
const STRETCH_ROWS: [Row; 10] = [
    Row::Mode,
    Row::GrainMode,
    Row::Scan,
    Row::Position,
    Row::Size,
    Row::Density,
    Row::Pitch,
    Row::Shape,
    Row::Spread,
    Row::Level,
];
/// A repeat's rows: where, how often (a beat division) and how long, the
/// edges, pitch, a chance of playing backwards and a little spray.
const REPEAT_ROWS: [Row; 10] = [
    Row::Mode,
    Row::GrainMode,
    Row::Position,
    Row::Density,
    Row::Size,
    Row::Shape,
    Row::Pitch,
    Row::Reverse,
    Row::Spray,
    Row::Level,
];
/// A tone's rows: its note (the density), how many cycles a grain holds
/// (the size), where in the sound and at what pitch it reads (its
/// colour), and what moves or widens it.
const TONE_ROWS: [Row; 12] = [
    Row::Mode,
    Row::GrainMode,
    Row::Density,
    Row::Size,
    Row::Position,
    Row::Pitch,
    Row::Shape,
    Row::Scan,
    Row::Spray,
    Row::PitchSpray,
    Row::Spread,
    Row::Level,
];
/// The Grain mode row's words, in the order it steps.
const GRAIN_MODES: [(&str, GrainMode); 4] = [
    ("Cloud", GrainMode::Cloud),
    ("Stretch", GrainMode::Stretch),
    ("Repeat", GrainMode::Repeat),
    ("Tone", GrainMode::Tone),
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
        let mut cloud = GrainCloud::new("Grains over the sound");
        cloud.right = "drag for the position".into();
        ui.add(CLOUD, cloud);
        ui.add(XY, XyPad::new("Touch: position × size", "position", "size"));
        let mut list = SettingList::new("Grain");
        list.right = "this layer".into();
        list.selected = Row::Scan.index_in(GrainMode::Cloud).unwrap_or(0);
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
        ui.add(WHEEL, InfoCard::new("The row picked"));
        ui.add_page(name, LAYOUT)?;
        let mut tab = GrainTab {
            name: name.to_string(),
            bank: None,
            shown: None,
            mode: GrainMode::Cloud,
            gesture: None,
            next_gesture: GESTURE_BASE,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The row picked in the Grain list.
    pub fn row(&self, ui: &Ui) -> Row {
        ui.get::<SettingList>(LIST)
            .and_then(|l| Row::shown(self.mode).get(l.selected).copied())
            .unwrap_or(Row::Scan)
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
            self.set_cloud(ui, st, &[], None, 0);
        }
        let mode = st.region.grain_mode;
        if mode != self.mode {
            // The same row stays picked if the new mode shows it, else the
            // Grain mode row, which changed it.
            let row = self.row(ui);
            self.mode = mode;
            let i = row
                .index_in(mode)
                .or(Row::GrainMode.index_in(mode))
                .unwrap_or(0);
            if let Some(list) = ui.get_mut::<SettingList>(LIST) {
                list.selected = i;
            }
        }
        self.show(ui, st);
    }

    /// The grains the engine plays now for the layer shown (positions 0 to
    /// 1 across its region), the voice's reading place, and how many
    /// grains play in all, out of how many may.
    pub fn set_cloud(
        &self,
        ui: &mut Ui,
        st: &GrainState,
        grains: &[GrainView],
        playhead: Option<f32>,
        live: usize,
    ) {
        let g = &st.region.grain;
        let Some(c) = ui.get_mut::<GrainCloud>(CLOUD) else {
            return;
        };
        let playing = !grains.is_empty();
        c.grains.clear();
        c.grains.extend_from_slice(grains);
        c.playhead = playhead;
        let mut caption = vec![
            format!("position {} %", percent(g.position)),
            format!("spray ±{} %", percent(g.spray * 0.5)),
        ];
        if st.granular() {
            caption.push(match (playing, grains.len()) {
                (false, _) => "hit the pad to hear it".to_string(),
                (true, 1) => "1 grain playing".to_string(),
                (true, n) => format!("{n} grains playing"),
            });
        }
        caption.push(format!("scan {}", scan_text(g.scan)));
        c.caption = caption.join(" · ");
        c.right = if live > 0 {
            format!("{live} grains on all pads")
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
        let rows = Row::shown(self.mode)
            .iter()
            .map(|r| Setting {
                name: r.label().into(),
                value: value_text(&st.region, *r),
            })
            .collect();
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
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
        let text = format!("{}: {}", row.label(), value_text(&st.region, row));
        if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
            card.text = text;
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
            (LIST, EventKind::Step { index, delta }) => {
                let row = *Row::shown(self.mode).get(*index)?;
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
        if let (Some(list), Some(i)) = (ui.get_mut::<SettingList>(LIST), row.index_in(self.mode)) {
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

/// `r` with row `row` stepped by `delta` steps (an arrow, or a detent of
/// the wheel), within its limits. `bpm`, the project tempo, is where
/// turning Tempo sync on starts: the beats nearest the size and density
/// heard.
pub fn stepped(r: &Region, row: Row, delta: i32, bpm: f64) -> Region {
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
    }
    r
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
    let kept = GrainParams {
        position: g.position,
        gain: g.gain,
        ..GrainParams::default()
    };
    match mode {
        GrainMode::Cloud => GrainParams {
            spray: 0.1,
            spread: 0.5,
            size_ms: 100.0,
            ..kept
        },
        GrainMode::Stretch => GrainParams {
            scan: 1.0,
            size_ms: 90.0,
            density: 45.0,
            timing: 0.0,
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
    Row::ALL
        .iter()
        .filter_map(|&row| {
            let (x, y) = (value_text(a, row), value_text(b, row));
            (x != y).then(|| format!("{} {x} -> {y}", row.label().to_lowercase()))
        })
        .collect::<Vec<_>>()
        .join(", ")
}
