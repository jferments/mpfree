//! The one Sample page's Warp tool (the plan's step 4, in place of the
//! Warp page): where the hits of a warped sound land.
//!
//! - **The wave as it plays.** While the layer stretches to the project
//!   tempo and has pins, the page's wave is drawn as it plays ([`bend`],
//!   [`Waveform::set_bend`](mpfree_ui::wave::Waveform::set_bend)):
//!   stretched between pins, so a pinned hit sits on its beat line. The
//!   bar lines count from the region's start at its warp tempo
//!   ([`beats`]). Outside the region, and with no pins, the wave is as
//!   recorded.
//! - **Hits** found in the sound are ticks along the wave's foot, with
//!   every tool (lit with this one). **Pins** are numbered tabs; with
//!   another tool they are faint lines.
//! - A **tap** on the wave pins the hit nearest the finger to the nearest
//!   grid line, or picks its pin if it has one. **Dragging** a pin's tab
//!   moves it along the grid; the sound between its neighbours stretches
//!   to fit.
//! - The **Warp section** under the wave holds the Warp page's settings in
//!   two parts: **Warp** (Off, Stretch, Repitch; Warp tempo and the grid;
//!   Shift and Process, which makes a new sound of the region) and **Pins**
//!   (how many hits are found, the pin picked and its beat; Pin hits,
//!   Remove pin and Clear pins).
//! - **Q-Link 1** picks a pin, **2** nudges its beat (a 64th of a beat a
//!   detent; with SHIFT, a 960th), **3** fine-tunes the warp tempo (a tenth
//!   of a BPM; with SHIFT, a hundredth) and **4** scrolls the wave.
//!
//! Pins move hits only when the region stretches; repitching, it plays
//! straight through (see [`mpfree_sampler::model::Pin`]). Every change is
//! a region, which the Wave page turns into a [`crate::wave_tab::WaveAction::Region`].

use std::sync::Arc;

use mpfree_sampler::model::{Region, SourceId, Warp, WarpMethod, WARP_BPM};
use mpfree_ui::wave::{bend_through, Beats, Marker, MarkerKind};
use mpfree_ui::Color;

use crate::panel::{Control, Item, Look, Panel, Section};
use crate::wave_tab::{region_tempo, with_warp_tempo, WaveState, TYPED_WARP_TEMPO};
use crate::widgets::Setting;

/// Beats a bar, as the bar lines count them.
pub const PER_BAR: f64 = 4.0;
/// Q-Link 2 with SHIFT nudges a pin's beat this much a detent.
pub const FINE_NUDGE: f64 = 1.0 / 960.0;
/// What a tap on the wave says while the layer isn't warped.
pub const OFF_NOTE: &str = "Warp is off: turn it to Stretch to pin hits to the beat";
/// What touching a pin says while the layer repitches.
pub const REPITCH_NOTE: &str = "Pins count when stretching: pick Stretch";
/// What the Process button says while nothing runs (under its name, "New
/// sound").
pub const PROCESS_TEXT: &str = "Make it";
/// The new sound's pitch (Shift) goes this many semitones either way.
pub const SHIFT_LIMIT: i32 = 24;
/// A hit this near a grid line, in grid steps, is pinned by Pin hits.
pub const REACH: f64 = 0.45;

/// The grid pins snap to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grid {
    Beat,
    Eighth,
    #[default]
    Sixteenth,
    ThirtySecond,
}

impl Grid {
    pub const ALL: [Grid; 4] = [
        Grid::Beat,
        Grid::Eighth,
        Grid::Sixteenth,
        Grid::ThirtySecond,
    ];

    /// Beats between its lines.
    pub fn beats(self) -> f64 {
        match self {
            Grid::Beat => 1.0,
            Grid::Eighth => 0.5,
            Grid::Sixteenth => 0.25,
            Grid::ThirtySecond => 0.125,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Grid::Beat => "1/4",
            Grid::Eighth => "1/8",
            Grid::Sixteenth => "1/16",
            Grid::ThirtySecond => "1/32",
        }
    }
}

/// How many of the hits found in a sound are shown and pinned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hits {
    Strongest,
    #[default]
    Most,
    All,
}

impl Hits {
    pub const ALL: [Hits; 3] = [Hits::Strongest, Hits::Most, Hits::All];

    /// The onset picker's sensitivity.
    pub fn sensitivity(self) -> f32 {
        match self {
            Hits::Strongest => 0.25,
            Hits::Most => 0.5,
            Hits::All => 0.85,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Hits::Strongest => "Strongest",
            Hits::Most => "Most",
            Hits::All => "All",
        }
    }
}

/// The Warp section's rows, in order (the list's rows, which the wheel
/// walks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Warp,
    WarpTempo,
    Grid,
    Shift,
    Process,
    Hits,
    Pin,
    PinBeat,
    PinHits,
    RemovePin,
    ClearPins,
}

impl Row {
    pub const ALL: [Row; 11] = [
        Row::Warp,
        Row::WarpTempo,
        Row::Grid,
        Row::Shift,
        Row::Process,
        Row::Hits,
        Row::Pin,
        Row::PinBeat,
        Row::PinHits,
        Row::RemovePin,
        Row::ClearPins,
    ];
    /// The rows that are buttons.
    pub const BUTTONS: [Row; 4] = [Row::Process, Row::PinHits, Row::RemovePin, Row::ClearPins];

    pub fn label(self) -> &'static str {
        match self {
            Row::Warp => "Warp",
            Row::WarpTempo => "Warp tempo",
            Row::Grid => "Grid",
            // Named for the sound Process makes, so it can't pass for a
            // live pitch beside the Sound section's Tune (Jay, 12 Nov).
            Row::Shift => "New sound's pitch",
            Row::Process => "New sound",
            Row::Hits => "Hits",
            Row::Pin => "Pin",
            Row::PinBeat => "Pin beat",
            Row::PinHits => "Pin hits",
            Row::RemovePin => "Remove pin",
            Row::ClearPins => "Clear pins",
        }
    }

    pub fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Row(Row),
    Drag(usize),
    Nudge,
    Tempo,
    Tap,
    Button(Row),
}

/// What a tap on the wave did.
#[derive(Clone, Debug, PartialEq)]
pub enum Tapped {
    /// Picked the pin already on that hit.
    Picked,
    /// Pinned the hit: the region with the new pin.
    Pinned(Box<Region>),
    /// Nothing: say this.
    Say(&'static str),
}

/// One Q-Link card: its name, value, unit, place round the knob, whether
/// it does anything now and, if not, why.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub label: String,
    pub value: String,
    pub sub: String,
    pub pos: f32,
    pub enabled: bool,
    pub why: String,
}

/// The Warp tool's own state: the grid and hits settings, the pin picked,
/// Process's shift and how far one under way has got. The region is the
/// [`WaveState`]'s.
#[derive(Clone, Debug, Default)]
pub struct WarpTool {
    pub grid: Grid,
    pub hits: Hits,
    /// Process moves the new sound this many semitones.
    pub shift: i32,
    selected: Option<usize>,
    /// The hits found, for the source and setting they were found for.
    found: Option<(SourceId, Hits, Arc<[u64]>)>,
    /// How far a Process under way has got (0 to 1), while one runs.
    processing: Option<f32>,
    /// The layer last shown, so a new one starts with no pin picked.
    shown: Option<(usize, usize, SourceId)>,
}

impl WarpTool {
    /// The pin picked.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Picks pin `i` (None: none).
    pub fn select(&mut self, i: Option<usize>) {
        self.selected = i;
    }

    /// How far a Process under way has got, while one runs.
    pub fn processing(&self) -> Option<f32> {
        self.processing
    }

    /// A Process runs and has got this far (0 to 1), or none runs; true if
    /// that changed what the Process button says.
    pub fn set_processing(&mut self, done: Option<f32>) -> bool {
        let changed = self.processing != done;
        self.processing = done;
        changed
    }

    /// Before showing `st`: a new layer or sound starts with no pin
    /// picked, and a pin picked that has gone is let go.
    pub fn follow(&mut self, st: &WaveState) {
        let here = (st.pad, st.layer, st.source.id);
        if self.shown != Some(here) {
            self.shown = Some(here);
            self.selected = None;
        }
        let n = st.region.warp.as_ref().map_or(0, |w| w.pins.len());
        if self.selected.is_some_and(|i| i >= n) {
            self.selected = n.checked_sub(1);
        }
    }

    /// The hits found in the layer's sound, in source frames (worked out
    /// once per source and setting; none while the sound's analysis is
    /// still under way).
    pub fn hits_of(&mut self, st: &WaveState) -> Arc<[u64]> {
        match &self.found {
            Some((id, h, hits)) if *id == st.source.id && *h == self.hits => hits.clone(),
            _ => {
                let hits: Arc<[u64]> = st
                    .source
                    .onsets
                    .as_ref()
                    .map(|o| o.pick(self.hits.sensitivity()))
                    .unwrap_or_default()
                    .into();
                // Kept only once the hits are worked out: until then there
                // are none, and the next look finds them.
                if st.source.ready {
                    self.found = Some((st.source.id, self.hits, hits.clone()));
                }
                hits
            }
        }
    }

    /// The section's rows, controls and panel.
    pub fn section(&self, st: &WaveState, no_hits: bool) -> (Vec<Setting>, Vec<Control>, Panel) {
        let rows = Row::ALL
            .iter()
            .map(|k| Setting {
                name: k.label().to_string(),
                value: self.value_text(st, *k),
            })
            .collect();
        (
            rows,
            self.controls(st, no_hits),
            panel(&st.region, self.shift),
        )
    }

    /// How each row looks on the panel, in [`Row::ALL`]'s order.
    fn controls(&self, st: &WaveState, no_hits: bool) -> Vec<Control> {
        let r = &st.region;
        let warped = r.warp.is_some();
        let picked = self.selected.is_some();
        let has_pins = r.warp.as_ref().is_some_and(|w| !w.pins.is_empty());
        Row::ALL
            .iter()
            .map(|k| match k {
                Row::Warp => Control::new(Look::Choice {
                    options: ["Off", "Stretch", "Repitch"].map(String::from).to_vec(),
                    on: match r.warp.as_ref().map(|w| w.method) {
                        None => 0,
                        Some(WarpMethod::Stretch) => 1,
                        Some(WarpMethod::Repitch) => 2,
                    },
                    off: Vec::new(),
                }),
                Row::WarpTempo => {
                    Control::new(Look::Stepper).faint_because(!warped, "Turn Warp on first")
                }
                Row::Grid | Row::Hits => Control::new(Look::Chip),
                Row::Shift => Control::new(Look::Stepper),
                Row::Process => Control::new(Look::Button(self.value_text(st, *k))),
                Row::PinHits => Control::new(Look::Button(k.label().into())).faint_because(
                    !warped || no_hits,
                    if warped {
                        "No hits found to pin"
                    } else {
                        "Turn Warp on first"
                    },
                ),
                Row::Pin => Control::new(Look::Chip)
                    .faint_because(!has_pins, "No pins yet: tap a hit, or Pin hits"),
                Row::PinBeat => Control::new(Look::Chip).faint_because(!picked, "Tap a pin first"),
                Row::RemovePin => Control::new(Look::Button(k.label().into()))
                    .faint_because(!picked, "Tap a pin first"),
                Row::ClearPins => Control::new(Look::Button(k.label().into()))
                    .faint_because(!has_pins, "No pins to clear"),
            })
            .collect()
    }

    /// A row's value as text.
    pub fn value_text(&self, st: &WaveState, row: Row) -> String {
        let w = st.region.warp.as_ref();
        match row {
            Row::Warp => match w.map(|w| w.method) {
                None => "Off",
                Some(WarpMethod::Stretch) => "Stretch",
                Some(WarpMethod::Repitch) => "Repitch",
            }
            .to_string(),
            Row::WarpTempo => match w {
                Some(w) => format!("{:.1} BPM", w.bpm),
                None => "—".to_string(),
            },
            Row::Grid => self.grid.label().to_string(),
            Row::Hits => self.hits.label().to_string(),
            Row::Pin => match (w, self.selected) {
                (Some(w), Some(i)) => format!("{} of {}", i + 1, w.pins.len()),
                (Some(w), None) if !w.pins.is_empty() => format!("none of {}", w.pins.len()),
                _ => "none".to_string(),
            },
            Row::PinBeat => match (w, self.selected) {
                (Some(w), Some(i)) => {
                    w.pins.get(i).map_or("—".to_string(), |p| beat_text(p.beat))
                }
                _ => "—".to_string(),
            },
            Row::Shift => match self.shift {
                0 => "0 st".to_string(),
                n => format!("{n:+} st"),
            },
            Row::PinHits | Row::RemovePin | Row::ClearPins => row.label().to_string(),
            Row::Process => match self.processing {
                Some(done) => format!("Stop · {:.0} %", done.clamp(0.0, 1.0) * 100.0),
                None => PROCESS_TEXT.to_string(),
            },
        }
    }

    /// The Q-Link cards: Pin, Pin beat, Warp tempo, Scroll. `scroll` is
    /// the first beat on the wave and whether it is zoomed in.
    pub fn cards(&self, st: &WaveState, scroll: (f64, bool)) -> [Card; 4] {
        let r = &st.region;
        let w = warp_of(st, r);
        let total = w.beats(r.frames(), st.rate).max(1e-9);
        let warped = r.warp.is_some();
        let n = w.pins.len();
        let card = |label: &str, value: String, sub: String, pos: f64, enabled, why: &str| Card {
            label: label.to_string(),
            value,
            sub,
            pos: pos.clamp(0.0, 1.0) as f32,
            enabled,
            why: why.to_string(),
        };
        let pin = self.selected.and_then(|i| w.pins.get(i));
        let (first, zoomed) = scroll;
        [
            match self.selected {
                Some(i) => card(
                    "Q-Link 1 · Pin",
                    format!("{} of {n}", i + 1),
                    String::new(),
                    (i + 1) as f64 / n.max(1) as f64,
                    n > 0,
                    "",
                ),
                None => card(
                    "Q-Link 1 · Pin",
                    "none".to_string(),
                    format!("{n} pins"),
                    0.0,
                    n > 0,
                    "No pins yet: tap a hit, or Pin hits",
                ),
            },
            match pin {
                Some(p) => card(
                    "Q-Link 2 · Pin beat",
                    beat_text(p.beat),
                    String::new(),
                    p.beat / total,
                    true,
                    "",
                ),
                None => card(
                    "Q-Link 2 · Pin beat",
                    "—".to_string(),
                    String::new(),
                    0.0,
                    false,
                    "Tap a pin first",
                ),
            },
            card(
                "Q-Link 3 · Warp tempo",
                format!("{:.2}", w.bpm),
                "BPM".to_string(),
                f64::from((w.bpm - WARP_BPM.start()) / (300.0 - WARP_BPM.start())),
                warped,
                "Turn Warp on first",
            ),
            if zoomed {
                card(
                    "Q-Link 4 · Scroll",
                    beat_text((first.max(0.0) * 4.0).floor() / 4.0),
                    "from".to_string(),
                    first / total,
                    true,
                    "",
                )
            } else {
                card(
                    "Q-Link 4 · Scroll",
                    "—".to_string(),
                    "zoom in to scroll".to_string(),
                    0.0,
                    false,
                    "Zoom in to scroll",
                )
            },
        ]
    }

    /// The pins as markers on the wave (frames as drawn through `bend`):
    /// numbered tabs, the one picked in `picked`'s colour and the rest in
    /// `color`, when `lit` (the Warp tool shows); faint lines otherwise.
    /// Pins that don't count (the layer repitches) are faint tabs.
    pub fn markers(
        &self,
        st: &WaveState,
        bend: &[(f64, f64)],
        lit: bool,
        color: Color,
        picked: Color,
    ) -> Vec<Marker> {
        let Some(w) = st.region.warp.as_ref() else {
            return Vec::new();
        };
        let counts = w.method == WarpMethod::Stretch;
        w.pins
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let at = played(bend, p.frame).round().max(0.0) as u64;
                let c = if self.selected == Some(i) {
                    picked
                } else {
                    color
                };
                let m = Marker::new(at, &(i + 1).to_string(), MarkerKind::Cue).colored(c);
                match (lit, counts) {
                    (true, true) => m,
                    (true, false) => m.faint(),
                    (false, _) => m.faint().line_only(),
                }
            })
            .collect()
    }

    /// A tap on the wave at source frame `f`: the hit nearest it (within
    /// half a grid step) is pinned to the nearest grid line, or its pin
    /// picked; with no hit near, the frame itself.
    pub fn tap(&mut self, st: &WaveState, f: u64) -> Tapped {
        let r = &st.region;
        let Some(w) = r.warp.as_ref() else {
            return Tapped::Say(OFF_NOTE);
        };
        if w.method != WarpMethod::Stretch {
            return Tapped::Say(REPITCH_NOTE);
        }
        let at = |x: u64| w.beat_at(r.start, r.end, st.rate, x as f64);
        let tapped = at(f);
        let grid = self.grid.beats();
        let hits = self.hits_of(st);
        let hit = hits
            .iter()
            .copied()
            .filter(|&h| h > r.start && h < r.end)
            .map(|h| (h, (at(h) - tapped).abs()))
            .filter(|(_, d)| *d <= grid / 2.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(f, |(h, _)| h);
        if let Some(i) = w.pins.iter().position(|p| p.frame == hit) {
            self.selected = Some(i);
            return Tapped::Picked;
        }
        match w.pin_added(hit, snap(at(hit), grid), r.start, r.end, st.rate) {
            Some((n, i)) => {
                self.selected = Some(i);
                Tapped::Pinned(Box::new(with_warp(r, n)))
            }
            None => Tapped::Say("No room for a pin there"),
        }
    }

    /// Pin `i`'s tab touched or dragged to `beat` (the beat under the
    /// finger): it is picked, and moves to the grid line nearest.
    pub fn dragged(&mut self, st: &WaveState, i: usize, beat: f64) -> Option<Region> {
        let r = &st.region;
        let w = r.warp.as_ref()?;
        self.selected = Some(i);
        let to = snap(beat, self.grid.beats());
        Some(with_warp(r, w.pin_moved(i, to, r.start, r.end, st.rate)))
    }

    /// A button pressed: Pin hits, Remove pin or Clear pins (Process is
    /// the page's).
    pub fn button(&mut self, st: &WaveState, row: Row) -> Option<Region> {
        let r = &st.region;
        let w = r.warp.as_ref()?;
        let n = match row {
            Row::PinHits => {
                let hits = self.hits_of(st);
                let grid = self.grid.beats();
                self.selected = None;
                w.pinned_to_grid(&hits, grid, grid * REACH, r.start, r.end, st.rate)
            }
            Row::RemovePin => {
                let i = self.selected?;
                let mut n = w.clone();
                if i < n.pins.len() {
                    n.pins.remove(i);
                }
                self.selected = None;
                n
            }
            Row::ClearPins => {
                self.selected = None;
                Warp {
                    pins: Vec::new(),
                    ..w.clone()
                }
            }
            _ => return None,
        };
        Some(with_warp(r, n))
    }

    /// A row stepped `delta` steps: an arrow or - / + (one), or a Warp
    /// button tapped (from the one on to that one). The new region, or
    /// None when only the tool's own settings changed (the grid, the hits,
    /// the pin picked, the shift) or nothing did.
    pub fn step(&mut self, st: &WaveState, row: Row, d: i32) -> Option<Region> {
        let r = &st.region;
        if d == 0 {
            return None;
        }
        let mut n = r.clone();
        match row {
            Row::Warp => {
                let states = [None, Some(WarpMethod::Stretch), Some(WarpMethod::Repitch)];
                let now = states
                    .iter()
                    .position(|m| *m == r.warp.as_ref().map(|w| w.method))
                    .unwrap_or(0) as i32;
                let next = states[(now + d).rem_euclid(3) as usize];
                let was = warp_of(st, r);
                n.warp = next.map(|method| Warp { method, ..was });
            }
            Row::WarpTempo => {
                let mut w = r.warp.clone()?;
                w.bpm = (w.bpm + d as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end());
                w.fit(r.start, r.end, st.rate);
                n.warp = Some(w);
            }
            Row::Grid => {
                self.grid = stepped(&Grid::ALL, self.grid, d);
                return None;
            }
            Row::Hits => {
                self.hits = stepped(&Hits::ALL, self.hits, d);
                return None;
            }
            Row::Pin => {
                self.pick(d, st);
                return None;
            }
            Row::PinBeat => {
                let w = r.warp.as_ref()?;
                let i = self.selected?;
                let p = w.pins.get(i)?;
                let grid = self.grid.beats();
                let to = snap(p.beat, grid) + f64::from(d) * grid;
                n.warp = Some(w.pin_moved(i, to, r.start, r.end, st.rate));
            }
            Row::Shift => {
                self.shift = (self.shift + d).clamp(-SHIFT_LIMIT, SHIFT_LIMIT);
                return None;
            }
            // Buttons: pressed, not stepped.
            Row::PinHits | Row::RemovePin | Row::ClearPins | Row::Process => return None,
        }
        Some(n)
    }

    /// Picks the pin `d` along from the one picked.
    pub fn pick(&mut self, d: i32, st: &WaveState) {
        let n = st.region.warp.as_ref().map_or(0, |w| w.pins.len());
        if n == 0 {
            self.selected = None;
            return;
        }
        // From none, the first detent picks the first (or, turned down,
        // the last) pin.
        let at = match self.selected {
            Some(i) => i as i64 + i64::from(d),
            None if d < 0 => n as i64 + i64::from(d),
            None => i64::from(d) - 1,
        };
        self.selected = Some(at.clamp(0, n as i64 - 1) as usize);
    }

    /// Q-Link 2: the pin picked nudged `delta` detents (a 64th of a beat,
    /// or with SHIFT a 960th).
    pub fn nudged(&self, st: &WaveState, delta: i32, fine: bool) -> Option<Region> {
        let r = &st.region;
        let w = r.warp.as_ref()?;
        let i = self.selected?;
        let p = w.pins.get(i)?;
        let step = if fine { FINE_NUDGE } else { 1.0 / 64.0 };
        let n = w.pin_moved(i, p.beat + f64::from(delta) * step, r.start, r.end, st.rate);
        Some(with_warp(r, n))
    }

    /// Q-Link 3: the warp tempo turned `delta` detents (a tenth of a BPM,
    /// or with SHIFT a hundredth).
    pub fn tempo_turned(st: &WaveState, delta: i32, fine: bool) -> Option<Region> {
        let r = &st.region;
        let mut w = r.warp.clone()?;
        let step = if fine { 0.01 } else { 0.1 };
        let bpm = (f64::from(w.bpm) + f64::from(delta) * step) / step;
        w.bpm = ((bpm.round() * step) as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end());
        w.fit(r.start, r.end, st.rate);
        Some(with_warp(r, w))
    }

    /// A number typed for the field `key` asked for.
    pub fn typed(st: &WaveState, key: &str, value: f64) -> Option<Region> {
        match key {
            TYPED_WARP_TEMPO => with_warp_tempo(st, &st.region, value),
            _ => None,
        }
    }
}

/// The section: Warp and Pins side by side in the band under the wave, up
/// to the Sound section.
fn panel(r: &Region, shift: i32) -> Panel {
    let it = |k: Row| Item::new(k.index());
    let pins = match r.warp.as_ref().map_or(0, |w| w.pins.len()) {
        0 => String::new(),
        1 => "1 pin".to_string(),
        n => format!("{n} pins"),
    };
    Panel {
        sections: vec![
            Section::new(
                "Warp",
                300,
                vec![
                    vec![it(Row::Warp)],
                    vec![
                        it(Row::WarpTempo).labelled(),
                        it(Row::Grid).labelled().wide(96),
                    ],
                    vec![it(Row::Shift).labelled(), it(Row::Process).labelled()],
                ],
            )
            .note(&match (shift, r.warp.as_ref().map(|w| w.method)) {
                (0, None) => "off".to_string(),
                (0, Some(WarpMethod::Stretch)) => "stretch to the tempo".to_string(),
                (0, Some(WarpMethod::Repitch)) => "repitch to the tempo".to_string(),
                (n, _) => format!(
                    "new sound: {} st {}, tap Make it",
                    n.abs(),
                    if n > 0 { "up" } else { "down" }
                ),
            }),
            Section::new(
                "Pins",
                450,
                vec![
                    vec![
                        it(Row::Hits).labelled().wide(150),
                        it(Row::Pin).labelled(),
                        it(Row::PinBeat).labelled(),
                    ],
                    vec![it(Row::PinHits), it(Row::RemovePin), it(Row::ClearPins)],
                ],
            )
            .note(&pins),
        ],
    }
}

/// The warp the tool shows for `r`: its own, or (not warped) one at the
/// tempo turning Warp on would give it.
pub fn warp_of(st: &WaveState, r: &Region) -> Warp {
    r.warp
        .clone()
        .unwrap_or_else(|| Warp::new(region_tempo(st, r)))
}

/// The wave as the layer plays it, for
/// [`Waveform::set_bend`](mpfree_ui::wave::Waveform::set_bend): (frame as
/// drawn, source frame) at the region's start, each pin and its end, a
/// beat drawn as long as a beat at the warp tempo. Empty unless the layer
/// stretches and has pins (then it plays as recorded).
pub fn bend(st: &WaveState) -> Vec<(f64, f64)> {
    let r = &st.region;
    match r.warp.as_ref() {
        Some(w) if w.pinned() => {
            let fpb = w.frames_per_beat(st.rate);
            let start = r.start as f64;
            w.points(r.start, r.end, st.rate)
                .into_iter()
                .map(|(x, beat)| (start + beat * fpb, start + x))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Where source frame `f` is drawn through `bend`.
pub fn played(bend: &[(f64, f64)], f: u64) -> f64 {
    if bend.is_empty() {
        return f as f64;
    }
    let back: Vec<(f64, f64)> = bend.iter().map(|&(a, b)| (b, a)).collect();
    bend_through(&back, f as f64)
}

/// The source frame drawn at `f` through `bend`.
pub fn source(bend: &[(f64, f64)], f: u64) -> u64 {
    bend_through(bend, f as f64).round().max(0.0) as u64
}

/// The bar lines with the Warp tool: from the region's start, a beat at
/// its warp tempo (the tempo Warp would start at, while it is off), and
/// the grid pins snap to between them.
pub fn beats(st: &WaveState, grid: Grid) -> Beats {
    let w = warp_of(st, &st.region);
    Beats {
        first: st.region.start as f64,
        frames_per_beat: w.frames_per_beat(st.rate),
        per_bar: PER_BAR as u32,
        per_beat: (1.0 / grid.beats()).round() as u32,
    }
}

/// The beat drawn at frame `f` (as drawn) with the Warp tool.
pub fn beat_drawn(st: &WaveState, f: f64) -> f64 {
    let b = beats(st, Grid::Beat);
    (f - b.first) / b.frames_per_beat
}

/// `r` with its warp replaced by `w`.
fn with_warp(r: &Region, w: Warp) -> Region {
    Region {
        warp: Some(w),
        ..r.clone()
    }
}

/// A beat as bar.beat.sixteenth, counted from 1, and how far off the
/// sixteenth in hundredths of one.
pub fn beat_text(beat: f64) -> String {
    let sixteenths = (beat * 4.0).round().max(0.0) as i64;
    let rest = beat * 4.0 - sixteenths as f64;
    let (bar, in_bar) = (sixteenths / 16, sixteenths % 16);
    let mut s = format!("{}.{}.{}", bar + 1, in_bar / 4 + 1, in_bar % 4 + 1);
    let off = (rest * 100.0).round() as i64;
    if off != 0 {
        s.push_str(&format!(" {off:+}"));
    }
    s
}

/// `beat` on the nearest line of a grid every `grid` beats.
pub fn snap(beat: f64, grid: f64) -> f64 {
    if grid > 0.0 {
        (beat / grid).round() * grid
    } else {
        beat
    }
}

/// The item `d` along from `now` in `all`, stopping at the ends.
fn stepped<T: Copy + PartialEq>(all: &[T], now: T, d: i32) -> T {
    let i = all.iter().position(|x| *x == now).unwrap_or(0) as i64;
    all[(i + i64::from(d)).clamp(0, all.len() as i64 - 1) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beats_snap_to_the_grid() {
        assert_eq!(snap(1.13, 0.25), 1.25);
        assert_eq!(snap(1.12, 0.25), 1.0);
        assert_eq!(snap(1.12, 0.0), 1.12);
        assert_eq!(stepped(&Grid::ALL, Grid::Beat, -1), Grid::Beat);
        assert_eq!(stepped(&Grid::ALL, Grid::Beat, 1), Grid::Eighth);
        assert_eq!(Grid::ThirtySecond.beats(), 0.125);
    }

    #[test]
    fn a_beat_reads_as_bar_beat_sixteenth() {
        assert_eq!(beat_text(0.0), "1.1.1");
        assert_eq!(beat_text(4.25), "2.1.2");
        assert_eq!(beat_text(1.26), "1.2.2 +4");
    }

    #[test]
    fn the_bend_runs_both_ways_and_one_to_one_outside_it() {
        let bend = vec![(100.0, 100.0), (300.0, 250.0), (500.0, 500.0)];
        assert_eq!(source(&bend, 300), 250);
        assert_eq!(played(&bend, 250), 300.0);
        assert_eq!(source(&bend, 50), 50);
        assert_eq!(played(&bend, 700), 700.0);
        assert_eq!(played(&[], 42), 42.0);
    }
}
