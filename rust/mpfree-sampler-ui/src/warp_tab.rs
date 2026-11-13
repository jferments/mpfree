//! The pad editor's Warp tab: where the hits of a warped sound land.
//!
//! - The **picture** draws the layer's region in its own beats (bars and
//!   beats from its warp tempo, and the grid), with the wave as it plays:
//!   stretched between **pins**, each a hit pinned to a beat. The hits
//!   found in the sound are ticks along the bottom.
//! - A **tap** on the wave pins the hit nearest the finger to the nearest
//!   grid line (or picks its pin, if it has one). **Dragging** a pin's
//!   handle moves it along the grid; the sound between its neighbours
//!   stretches to fit. Dragging the wave scrolls it when zoomed in.
//! - **Pin hits** pins every hit near a grid line onto it (quantising a
//!   loop played loosely), **Remove pin** takes the pin picked off and
//!   **Clear pins** all of them.
//! - The **Warp** list holds Warp (Off, Stretch, Repitch) and Warp tempo as
//!   on the Wave tab, the grid, how many hits are found, and the pin picked
//!   and its beat.
//! - **Q-Link 1** picks a pin, **2** nudges its beat (a 64th of a beat a
//!   detent; with SHIFT, a 960th), **3** fine-tunes the warp tempo (a tenth
//!   of a BPM; with SHIFT, a hundredth) and **4** scrolls. The **wheel**
//!   zooms round the pin picked.
//!
//! Pins move hits only when the region stretches; repitching, it plays
//! straight through (see [`mpfree_sampler::model::Pin`]). The tab edits
//! the region as the Wave tab does: every change is a
//! [`WaveAction::Region`], and a drag, a turn or a run of taps on one row
//! is one undo step until the app calls [`WarpTab::lift`].

use std::sync::Arc;

use mpfree_sampler::model::{Region, SourceId, Warp, WarpMethod, WARP_BPM};
use mpfree_ui::{EventKind, Ui, UiEvent};

use crate::warp_widgets::{beat_text, WarpView, PER_BAR, TICKS};
use crate::wave_tab::{self, pad_color, pad_label, region_tempo, WaveAction, WaveState, TABS};
use crate::widgets::{ButtonRow, InfoCard, KnobCard, PadBar, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-warp-bar";
pub const VIEW: &str = "smp-warp-view";
pub const LIST: &str = "smp-warp-list";
pub const BUTTONS: &str = "smp-warp-buttons";
pub const QLINKS: [&str; 4] = ["smp-warp-q1", "smp-warp-q2", "smp-warp-q3", "smp-warp-q4"];
pub const WHEEL: &str = "smp-warp-wheel";

/// The tab's page: the picture beside the Warp list and its buttons,
/// between the pad bar and the Q-Link cards.
pub const LAYOUT: &str = "col(header:60, row(smp-warp-bar:*, smp-send:330):60, \
                          row(smp-warp-view:*, col(smp-warp-list:*, smp-warp-buttons:64):360):*, \
                          row(smp-warp-q1, smp-warp-q2, smp-warp-q3, smp-warp-q4, smp-warp-wheel:210):88, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = 1;

/// The hint line the app shows under the tab.
pub const HINT: &str = "Tap a hit to pin it to the grid · drag a pin's handle along the beats · Pin hits pins them all · \
                        Q-Link 1 picks a pin, 2 nudges it · the wheel zooms";

/// What the picture says while the layer isn't warped.
pub const OFF_NOTE: &str = "Warp is off: turn it to Stretch to pin hits to the beat";

/// A hit this near a grid line, in grid steps, is pinned by Pin hits.
pub const REACH: f64 = 0.45;
/// Gesture ids the tab hands out start here, clear of other screens'.
const GESTURE_BASE: u64 = 0x5741_5250_0000_0000;
/// Each wheel detent zooms by this much.
const WHEEL_ZOOM: f64 = 1.25;
/// The fewest beats the picture shows, zoomed right in.
const MIN_SPAN: f64 = 0.5;

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

/// The Warp list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Warp,
    WarpTempo,
    Grid,
    Hits,
    Pin,
    PinBeat,
}

impl Row {
    pub const ALL: [Row; 6] = [
        Row::Warp,
        Row::WarpTempo,
        Row::Grid,
        Row::Hits,
        Row::Pin,
        Row::PinBeat,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Warp => "Warp",
            Row::WarpTempo => "Warp tempo",
            Row::Grid => "Grid",
            Row::Hits => "Hits",
            Row::Pin => "Pin",
            Row::PinBeat => "Pin beat",
        }
    }

    pub fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Row(Row),
    Drag(usize),
    Nudge,
    Tempo,
    Tap,
    Button(usize),
}

/// The Warp tab's widgets, its grid and hits settings, the pin picked and
/// the undo step being made; the region comes from the [`WaveState`].
pub struct WarpTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`wave_tab::WaveTab::bank`].
    pub bank: Option<char>,
    pub grid: Grid,
    pub hits: Hits,
    selected: Option<usize>,
    gesture: Option<(Key, u64)>,
    next_gesture: u64,
    /// The hits found, for the source and setting they were found for.
    found: Option<(SourceId, Hits, Arc<[u64]>)>,
    /// The region last shown, so a new one starts zoomed out.
    shown: Option<(usize, usize, SourceId)>,
}

impl WarpTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &WaveState) -> Result<WarpTab, String> {
        crate::send::add(ui);
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        let mut view = WarpView::new("Warp");
        view.accent = ui.theme.warn;
        ui.add(VIEW, view);
        let mut list = SettingList::new("Warp");
        list.selected = Row::Warp.index();
        ui.add(LIST, list);
        ui.add(
            BUTTONS,
            ButtonRow::new(&["Pin hits", "Remove pin", "Clear pins"]),
        );
        let t = ui.theme.clone();
        let cards = [
            ("Q-Link 1 · pin", t.warn),
            ("Q-Link 2 · pin beat", t.warn.mix(t.bad, 0.4)),
            ("Q-Link 3 · warp tempo", t.accent),
            ("Q-Link 4 · scroll", t.good),
        ];
        for (k, (label, color)) in cards.iter().enumerate() {
            ui.add(QLINKS[k], KnobCard::new(label, *color));
        }
        ui.add(WHEEL, InfoCard::new("Wheel · zoom"));
        ui.add_page(name, LAYOUT)?;
        let mut tab = WarpTab {
            name: name.to_string(),
            bank: None,
            grid: Grid::default(),
            hits: Hits::default(),
            selected: None,
            gesture: None,
            next_gesture: GESTURE_BASE,
            found: None,
            shown: None,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The pin picked.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The row picked in the Warp list.
    pub fn row(&self, ui: &Ui) -> Row {
        ui.get::<SettingList>(LIST)
            .and_then(|l| Row::ALL.get(l.selected).copied())
            .unwrap_or(Row::Warp)
    }

    /// Ends the drag or turn being made: the next edit is a new undo step.
    pub fn lift(&mut self) {
        self.gesture = None;
    }

    /// Where the pad's voices are playing, in source frames.
    pub fn set_playheads(&self, ui: &mut Ui, frames: &[u64]) {
        if let Some(v) = ui.get_mut::<WarpView>(VIEW) {
            v.set_playheads(frames);
        }
    }

    /// The hits found in the layer's sound, in source frames (worked out
    /// once per source and setting).
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
                self.found = Some((st.source.id, self.hits, hits.clone()));
                hits
            }
        }
    }

    /// The warp the tab shows for `r`: its own, or (not warped) one at the
    /// tempo turning Warp on would give it.
    fn warp_of(st: &WaveState, r: &Region) -> Warp {
        r.warp
            .clone()
            .unwrap_or_else(|| Warp::new(region_tempo(st, r)))
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &WaveState) {
        let r = &st.region;
        let w = WarpTab::warp_of(st, r);
        let pins_on = r
            .warp
            .as_ref()
            .is_some_and(|w| w.method == WarpMethod::Stretch);
        if self.selected.is_some_and(|i| i >= w.pins.len()) {
            self.selected = w.pins.len().checked_sub(1);
        }
        let hits = self.hits_of(st);
        let here = (st.pad, st.layer, st.source.id);
        let fresh = self.shown != Some(here);
        self.shown = Some(here);
        if fresh {
            self.selected = None;
        }
        let total = w.beats(r.frames(), st.rate);
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.pad);
            bar.active = TAB;
            let pad = match self.bank {
                Some(b) => format!("{b}{}", st.pad % 16 + 1),
                None => pad_label(st.pad),
            };
            bar.title = format!("Pad {pad} · {}", st.pad_name);
            let bars = total / PER_BAR;
            let len = if (bars - bars.round()).abs() < 0.01 && bars.round() >= 1.0 {
                let n = bars.round();
                format!("{n} bar{}", if n == 1.0 { "" } else { "s" })
            } else {
                format!("{total:.2} beats")
            };
            bar.subtitle = match &r.warp {
                None => format!("{len} at {:.1} BPM · not warped", w.bpm),
                Some(x) => format!(
                    "{len} at {:.1} BPM · {}",
                    x.bpm,
                    match x.method {
                        WarpMethod::Stretch => "stretched to the project tempo",
                        WarpMethod::Repitch => "repitched to the project tempo",
                    }
                ),
            };
        }
        let grid = self.grid.beats();
        let selected = self.selected;
        if let Some(v) = ui.get_mut::<WarpView>(VIEW) {
            v.peaks = Some(st.source.peaks.clone());
            v.rate = st.rate;
            v.start = r.start;
            v.end = r.end;
            v.warp = w.clone();
            v.pins_on = pins_on;
            v.grid = grid;
            v.hits = hits.to_vec();
            v.selected = selected;
            if fresh {
                v.span = 0.0;
                v.first = 0.0;
            }
            v.title = format!("Warp · {:.1} BPM", w.bpm);
            v.right = match (&r.warp, w.pins.len()) {
                (Some(x), n) if x.method == WarpMethod::Repitch && n > 0 => {
                    "pins count when stretching".to_string()
                }
                (_, 0) => format!("{} hits found", hits.len()),
                (_, 1) => "1 pin".to_string(),
                (_, n) => format!("{n} pins"),
            };
            v.note = if r.warp.is_none() {
                OFF_NOTE.to_string()
            } else {
                String::new()
            };
            v.changed();
        }
        let rows = Row::ALL
            .iter()
            .map(|k| Setting {
                name: k.label().to_string(),
                value: self.value_text(st, *k),
            })
            .collect();
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
            list.right = self.grid.label().to_string();
        }
        let warped = r.warp.is_some();
        let has_pins = !w.pins.is_empty();
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.enabled = vec![warped && !hits.is_empty(), selected.is_some(), has_pins];
        }
        let cards = [
            match selected {
                Some(i) => (
                    format!("{} of {}", i + 1, w.pins.len()),
                    String::new(),
                    (i + 1) as f32 / w.pins.len().max(1) as f32,
                ),
                None => ("none".to_string(), format!("{} pins", w.pins.len()), 0.0),
            },
            match selected.and_then(|i| w.pins.get(i)) {
                Some(p) => (
                    beat_text(p.beat),
                    String::new(),
                    (p.beat / total.max(1e-9)) as f32,
                ),
                None => ("—".to_string(), String::new(), 0.0),
            },
            (
                format!("{:.2} BPM", w.bpm),
                String::new(),
                ((w.bpm - WARP_BPM.start()) / (300.0 - WARP_BPM.start())).clamp(0.0, 1.0),
            ),
            match ui.get::<WarpView>(VIEW) {
                Some(v) if v.span > 0.0 => {
                    let first = v.shown().0.max(0.0);
                    // The sixteenth it starts in.
                    let text = beat_text((first * 4.0).floor() / 4.0);
                    (
                        format!("from {text}"),
                        String::new(),
                        (first / total.max(1e-9)) as f32,
                    )
                }
                _ => ("—".to_string(), "zoom in to scroll".to_string(), 0.0),
            },
        ];
        let zoomed = ui.get::<WarpView>(VIEW).is_some_and(|v| v.span > 0.0);
        let enabled = [has_pins, selected.is_some(), warped, zoomed];
        for (k, (value, sub, pos)) in cards.into_iter().enumerate() {
            if let Some(card) = ui.get_mut::<KnobCard>(QLINKS[k]) {
                card.value = value;
                card.sub = sub;
                card.pos = pos.clamp(0.0, 1.0);
                card.enabled = enabled[k];
            }
        }
        let wheel = match ui.get::<WarpView>(VIEW).map(WarpView::shown) {
            Some((_, span)) if span < total => format!("{span:.1} of {total:.1} beats"),
            _ => "whole region".to_string(),
        };
        if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
            card.text = wheel;
        }
    }

    /// A row's value as text.
    pub fn value_text(&self, st: &WaveState, row: Row) -> String {
        let r = &st.region;
        let w = r.warp.as_ref();
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

    /// `st`'s region with its warp replaced by `w`.
    fn with_warp(st: &WaveState, w: Warp) -> Region {
        Region {
            warp: Some(w),
            ..st.region.clone()
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &WaveState) -> Option<WaveAction> {
        let r = &st.region;
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(WaveAction::Tab(*i)),
            (VIEW, EventKind::Select(i)) => {
                self.selected = Some(*i);
                self.update(ui, st);
                None
            }
            (VIEW, EventKind::Marker { index, frame }) => {
                let w = r.warp.as_ref()?;
                let beat = snap(*frame as f64 / TICKS, self.grid.beats());
                self.selected = Some(*index);
                let n = w.pin_moved(*index, beat, r.start, r.end, st.rate);
                let out = self.edit(st, Key::Drag(*index), WarpTab::with_warp(st, n));
                if out.is_none() {
                    self.update(ui, st);
                }
                out
            }
            (VIEW, EventKind::Frame(f)) => self.tap(ui, st, *f),
            (VIEW, EventKind::View { .. }) => {
                self.update(ui, st);
                None
            }
            (BUTTONS, EventKind::Tab(b)) => self.button(ui, st, *b),
            (LIST, EventKind::Step { index, delta }) => {
                self.step(ui, st, *Row::ALL.get(*index)?, *delta)
            }
            (LIST, EventKind::Select(_)) => None,
            (name, EventKind::Select(_)) if QLINKS.contains(&name) => None,
            _ => None,
        }
    }

    /// A tap on the wave at source frame `f`: the hit nearest it is pinned
    /// to the nearest grid line, or its pin picked.
    fn tap(&mut self, ui: &mut Ui, st: &WaveState, f: u64) -> Option<WaveAction> {
        let r = &st.region;
        let w = r.warp.as_ref()?;
        let line = if w.method == WarpMethod::Stretch {
            w.clone()
        } else {
            Warp {
                pins: Vec::new(),
                ..w.clone()
            }
        };
        let at = |x: u64| line.beat_at(r.start, r.end, st.rate, x as f64);
        let tapped = at(f);
        let grid = self.grid.beats();
        let hits = self.hits_of(st);
        // The nearest hit within half a grid step, else the frame itself.
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
            self.update(ui, st);
            return None;
        }
        let beat = snap(at(hit), grid);
        let (n, i) = w.pin_added(hit, beat, r.start, r.end, st.rate)?;
        self.selected = Some(i);
        self.edit(st, Key::Tap, WarpTab::with_warp(st, n))
    }

    fn button(&mut self, ui: &mut Ui, st: &WaveState, b: usize) -> Option<WaveAction> {
        let r = &st.region;
        let w = r.warp.as_ref()?;
        let n = match b {
            0 => {
                let hits = self.hits_of(st);
                let grid = self.grid.beats();
                let n = w.pinned_to_grid(&hits, grid, grid * REACH, r.start, r.end, st.rate);
                self.selected = None;
                n
            }
            1 => {
                let i = self.selected?;
                let mut n = w.clone();
                if i < n.pins.len() {
                    n.pins.remove(i);
                }
                self.selected = None;
                n
            }
            _ => {
                self.selected = None;
                Warp {
                    pins: Vec::new(),
                    ..w.clone()
                }
            }
        };
        let out = self.edit(st, Key::Button(b), WarpTab::with_warp(st, n));
        self.lift();
        if out.is_none() {
            self.update(ui, st);
        }
        out
    }

    /// An arrow on a Warp row.
    fn step(&mut self, ui: &mut Ui, st: &WaveState, row: Row, delta: i32) -> Option<WaveAction> {
        let r = &st.region;
        let d = delta.signum();
        let mut n = r.clone();
        match row {
            Row::Warp => {
                let states = [None, Some(WarpMethod::Stretch), Some(WarpMethod::Repitch)];
                let now = states
                    .iter()
                    .position(|m| *m == r.warp.as_ref().map(|w| w.method))
                    .unwrap_or(0) as i32;
                let next = states[(now + d).rem_euclid(3) as usize];
                let was = WarpTab::warp_of(st, r);
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
                self.update(ui, st);
                return None;
            }
            Row::Hits => {
                self.hits = stepped(&Hits::ALL, self.hits, d);
                self.update(ui, st);
                return None;
            }
            Row::Pin => {
                self.pick(d, st);
                self.update(ui, st);
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
        }
        let out = self.edit(st, Key::Row(row), n);
        if out.is_none() {
            self.update(ui, st);
        }
        out
    }

    /// Picks the pin `d` along from the one picked.
    fn pick(&mut self, d: i32, st: &WaveState) {
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

    /// A Q-Link turned by `delta` detents; `fine` with SHIFT held.
    pub fn qlink(
        &mut self,
        ui: &mut Ui,
        st: &WaveState,
        knob: usize,
        delta: i32,
        fine: bool,
    ) -> Option<WaveAction> {
        if delta == 0 {
            return None;
        }
        let r = &st.region;
        match knob {
            0 => {
                self.pick(delta, st);
                self.update(ui, st);
                None
            }
            1 => {
                let w = r.warp.as_ref()?;
                let i = self.selected?;
                let p = w.pins.get(i)?;
                let step = if fine { 1.0 / TICKS } else { 1.0 / 64.0 };
                let n = w.pin_moved(i, p.beat + f64::from(delta) * step, r.start, r.end, st.rate);
                self.edit(st, Key::Nudge, WarpTab::with_warp(st, n))
            }
            2 => {
                let mut w = r.warp.clone()?;
                let step = if fine { 0.01 } else { 0.1 };
                let bpm = (f64::from(w.bpm) + f64::from(delta) * step) / step;
                w.bpm = ((bpm.round() * step) as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end());
                w.fit(r.start, r.end, st.rate);
                self.edit(st, Key::Tempo, WarpTab::with_warp(st, w))
            }
            3 => {
                if let Some(v) = ui.get_mut::<WarpView>(VIEW) {
                    let (first, span) = v.shown();
                    if v.span > 0.0 {
                        let max = (v.total() - span * 0.5).max(0.0);
                        v.first = (first + f64::from(delta) * span / 16.0).clamp(-span * 0.5, max);
                    }
                }
                self.update(ui, st);
                None
            }
            _ => None,
        }
    }

    /// The data wheel: zooms in (turned up) or out round the pin picked,
    /// or round the middle of what is shown.
    pub fn wheel(&mut self, ui: &mut Ui, st: &WaveState, delta: i32) {
        let selected = self.selected;
        if let Some(v) = ui.get_mut::<WarpView>(VIEW) {
            let total = v.total();
            let (first, span) = v.shown();
            let centre = selected
                .and_then(|i| v.warp.pins.get(i))
                .map_or(first + span / 2.0, |p| p.beat);
            // Never `clamp` here: a sound shorter than MIN_SPAN beats (a
            // 0.02 s hit at 120 BPM is 0.05 beats) puts the ceiling under
            // the floor, and clamp panics on that (13 Nov live test).
            let new = (span / WHEEL_ZOOM.powi(delta))
                .max(MIN_SPAN)
                .min(total * 1.02);
            if new >= total {
                v.span = 0.0;
                v.first = 0.0;
            } else {
                let place = ((centre - first) / span).clamp(0.0, 1.0);
                v.span = new;
                v.first = (centre - place * new).clamp(-new * 0.5, (total - new * 0.5).max(0.0));
            }
        }
        self.update(ui, st);
    }
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
}
