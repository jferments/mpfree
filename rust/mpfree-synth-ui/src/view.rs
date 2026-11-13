//! The Synth tab's panel: what it shows, worked out from an instrument's
//! parameters, and what touches, Q-Links and the wheel mean.
//!
//! The panel does not own the instrument. The app keeps an [`SynthState`]
//! (from [`SynthState::from_node`]: the node's parameter list, a copy of its
//! values and its presets), shows it with [`SynthPanel::update`], and turns
//! the [`SynthAction`]s the panel returns into node calls on the audio
//! thread (`Node::set_param`, or loading a preset). The panel changes its own
//! copy of a value at once, so the screen follows a finger without waiting
//! for the audio thread.
//!
//! The header and the hint line belong to the app, as on its other pages.
//!
//! FM draws its algorithm as six boxes (carriers green along the bottom,
//! modulators blue above what they modulate, the feedback loop on the
//! right). The Algo page shows it with the operators' output levels; each
//! operator page rings its box beside its envelope; the keyboard pages ring
//! their two beside their scaling curves. A tap on a box opens that
//! operator's page.
//!
//! Drums draws its kit as the pads lie (a box per pad, tagged with its
//! sound: BD, SD, CH, coloured by what it is made of), the hit on the pad
//! shown (its body, noise and click over time), its pitch sweep and every
//! pad's level. A tap on a pad's box opens its page. Pluck draws, for a
//! string, where it is picked, the harmonics a pluck gives and how fast
//! each dies; for a mallet, its pulse, the material's modes as struck and
//! their decays; all for middle C.
//!
//! An engine with pad pressure (Poly, FM) has a Pressure page: the shaper's
//! settings and four slots, with a drawing of the curve and a live bar for
//! each sounding voice's pressure (the app passes those in with
//! [`SynthPanel::set_pressures`]). A slot's depth reads in its target's
//! unit (+1.5 oct, 9 dB swell).

use crate::fm_visual as fv;
use crate::format::{amount_text, value_text};
use crate::kit_visual as kv;
use crate::pages::{plan, qlink_targets, tile_label, Page};
use crate::visual;
use crate::widgets::{
    Card, CardNode, QCell, QLinkStrip, Tile, TileGrid, TileKind, TitleBar, Trace, Visuals, ON_FILL,
    QLINK_COLORS, TEXT, TEXT_DIM,
};
use mpfree_node::pressure::{field, FIELDS};
use mpfree_node::{Curve, Node, NodeInfo, ParamInfo};
use mpfree_synth::params as pp;
use mpfree_synth::{fm_params as fp, pressure as sp};
use mpfree_ui::widgets::{ListItem, ListView};
use mpfree_ui::{Color, EventKind, Ui, UiEvent};

/// Widget names the panel adds. The app's own are `header` and `hint`.
pub const TITLE: &str = "synth-title";
pub const VISUALS: &str = "synth-visuals";
pub const TILES: &str = "synth-tiles";
pub const PRESETS: &str = "synth-presets";
pub const QLINKS: &str = "synth-qlinks";

/// An instrument with drawn visuals (Poly): the title, the visual strip and
/// the tiles beside the preset list, then the Q-Link strip.
pub const LAYOUT: &str = "col(header:60, row(col(synth-title:64, synth-visuals:150, synth-tiles:*):*, synth-presets:300):*, synth-qlinks:78, hint:26)";
/// Any other node (a hosted plugin, an effect): the same without visuals.
pub const PLAIN_LAYOUT: &str = "col(header:60, row(col(synth-title:64, synth-tiles:*):*, synth-presets:300):*, synth-qlinks:78, hint:26)";

/// Wheel or Q-Link detents for a knob's whole range.
pub const DETENTS: f32 = 100.0;

/// Everything the panel shows.
#[derive(Clone, Debug)]
pub struct SynthState {
    pub info: &'static NodeInfo,
    pub params: &'static [ParamInfo],
    /// The node's values, in `params` order.
    pub values: Vec<f32>,
    /// The track it plays on, such as "Bass", and its colour.
    pub track: String,
    pub color: Color,
    pub presets: Vec<String>,
    /// The preset loaded, and whether it was changed since.
    pub preset: Option<usize>,
    pub edited: bool,
    /// Each sounding voice's pressure now, 0 to 1 (empty when the app has
    /// none to show), for the Pressure page's live bars.
    pub pressures: Vec<f32>,
}

impl SynthState {
    /// What the node holds now.
    pub fn from_node(
        node: &dyn Node,
        track: &str,
        color: Color,
        presets: Vec<String>,
        preset: Option<usize>,
    ) -> SynthState {
        let params = node.params();
        SynthState {
            info: node.info(),
            params,
            values: params.iter().map(|p| node.param(p.id)).collect(),
            track: track.to_string(),
            color,
            presets,
            preset,
            edited: false,
            pressures: Vec::new(),
        }
    }

    pub fn index(&self, id: u16) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }

    /// A value by parameter id (the default for an id the node lacks).
    pub fn value(&self, id: u16) -> f32 {
        self.index(id).map(|i| self.values[i]).unwrap_or(0.0)
    }

    /// Sets a value by index, clamped as the node would. Returns the value
    /// set, or None if nothing changed or the parameter is a meter.
    fn set(&mut self, i: usize, v: f32) -> Option<f32> {
        let p = &self.params[i];
        let v = p.clamp(v);
        if p.readonly || v == self.values[i] {
            return None;
        }
        self.values[i] = v;
        self.edited = true;
        Some(v)
    }
}

/// What a touch, a Q-Link or the wheel asks for. The app applies it to the
/// node.
#[derive(Clone, Debug, PartialEq)]
pub enum SynthAction {
    /// `Node::set_param(id, value)`.
    Set { id: u16, value: f32 },
    /// Load preset `i` of [`SynthState::presets`].
    Preset(usize),
}

pub struct SynthPanel {
    name: String,
    pages: Vec<Page>,
    page: usize,
    /// The selected tile, as an index into the node's parameters.
    focus: Option<usize>,
    /// The wheel's push opened the selected tile: the wheel turns it (the
    /// app's wheel navigation); otherwise the wheel picks tiles.
    editing: bool,
    visuals: bool,
    /// The page a tap on each visual card opens, and on each box.
    card_pages: Vec<String>,
    node_pages: Vec<String>,
    pub state: SynthState,
    /// Changes the page's layout before it is added, for an app that puts
    /// widgets of its own on the page (the pad player's pads heading).
    wrap: Option<fn(&str) -> String>,
}

/// The visual strip's cards, with the page a tap on each card (and on each
/// box, counted across the cards) opens.
struct Strip {
    cards: Vec<Card>,
    pages: Vec<String>,
    node_pages: Vec<String>,
}

impl Strip {
    fn new() -> Strip {
        Strip {
            cards: Vec::new(),
            pages: Vec::new(),
            node_pages: Vec::new(),
        }
    }

    fn push(&mut self, card: Card, page: &str) {
        self.cards.push(card);
        self.pages.push(page.to_string());
    }
}

/// Whether the panel draws visuals for this node on this page: Poly, FM,
/// Drums, Pluck and Wavetable on every page, any other engine with
/// pressure on its Pressure page.
fn has_visuals(info: &NodeInfo, page: &str) -> bool {
    matches!(
        info.id,
        "mpfree.poly" | "mpfree.fm" | "mpfree.drums" | "mpfree.pluck" | "mpfree.wavetable"
    ) || (page == "Pressure" && press_base(info).is_some())
}

/// The first pressure parameter of an engine that has them, and what its
/// slots can drive.
pub fn press_base(info: &NodeInfo) -> Option<u16> {
    match info.id {
        "mpfree.poly" => Some(pp::PRESS_BASE),
        "mpfree.fm" => Some(fp::PRESS_BASE),
        "mpfree.wavetable" => Some(mpfree_synth::wavetable::params::PRESS_BASE),
        _ => None,
    }
}

fn press_targets(info: &NodeInfo) -> &'static [&'static str] {
    match info.id {
        "mpfree.poly" => sp::POLY_TARGETS,
        "mpfree.fm" => sp::FM_TARGETS,
        "mpfree.wavetable" => sp::WT_TARGETS,
        _ => &[],
    }
}

/// A parameter's value text and unit. A pressure slot's depth reads in its
/// target's unit (Cutoff 37.5 % reads +1.5 oct).
fn param_text(st: &SynthState, i: usize) -> (String, String) {
    let p = &st.params[i];
    let v = st.values[i];
    if let Some(base) = press_base(st.info) {
        let off = p.id.wrapping_sub(base);
        if (field::depth(0)..FIELDS as u16).contains(&off)
            && (off - field::depth(0)).is_multiple_of(2)
        {
            let n = usize::from((off - field::depth(0)) / 2);
            let target = press_targets(st.info)
                .get(st.value(base + field::target(n)) as usize)
                .copied()
                .unwrap_or("Off");
            if let Some((a, unit)) = sp::amount(target, v) {
                let (t, u) = amount_text(target, a, unit);
                return (t, u.to_string());
            }
            return ("—".to_string(), String::new());
        }
    }
    let (t, u) = value_text(p, v);
    (t, u.to_string())
}

fn param_line(st: &SynthState, i: usize) -> String {
    let (n, u) = param_text(st, i);
    if u.is_empty() {
        n
    } else {
        format!("{n} {u}")
    }
}

impl SynthPanel {
    /// Adds the panel's widgets and its page `name` to `ui` (the app adds
    /// `header` and `hint` first).
    pub fn add(ui: &mut Ui, name: &str, state: SynthState) -> Result<SynthPanel, String> {
        ui.add(TITLE, TitleBar::new());
        ui.add(VISUALS, Visuals::new());
        ui.add(TILES, TileGrid::new(""));
        ui.add(PRESETS, ListView::new("Presets"));
        ui.add(QLINKS, QLinkStrip::new());
        let mut panel = SynthPanel {
            name: name.to_string(),
            pages: Vec::new(),
            page: 0,
            focus: None,
            editing: false,
            visuals: false,
            card_pages: Vec::new(),
            node_pages: Vec::new(),
            state,
            wrap: None,
        };
        panel.load(ui, panel.state.clone())?;
        Ok(panel)
    }

    /// Lays the page out again with `wrap` applied to its layout, now and
    /// whenever the layout changes. The widgets `wrap` adds must be in `ui`
    /// already.
    pub fn set_layout_wrap(&mut self, ui: &mut Ui, wrap: fn(&str) -> String) -> Result<(), String> {
        self.wrap = Some(wrap);
        ui.add_page(&self.name, &self.spec())
    }

    fn spec(&self) -> String {
        self.wrap
            .map_or_else(|| self.layout().to_string(), |f| f(self.layout()))
    }

    /// Shows another node, or the same node after a preset load or any
    /// change made outside the panel. Keeps the page when the node is the same.
    pub fn load(&mut self, ui: &mut Ui, state: SynthState) -> Result<(), String> {
        let same = self.state.info.id == state.info.id && !self.pages.is_empty();
        self.pages = plan(state.info, state.params);
        if !same {
            self.page = 0;
            self.focus = None;
        }
        self.page = self.page.min(self.pages.len().saturating_sub(1));
        self.state = state;
        self.visuals = self.wants_visuals();
        ui.add_page(&self.name, &self.spec())?;
        self.refresh(ui);
        Ok(())
    }

    fn wants_visuals(&self) -> bool {
        has_visuals(
            self.state.info,
            self.pages.get(self.page).map_or("", |p| p.name.as_str()),
        )
    }

    /// Each sounding voice's pressure, 0 to 1, for the Pressure page's live
    /// bars. The app calls it a few times a second while the page shows.
    pub fn set_pressures(&mut self, ui: &mut Ui, values: &[f32]) {
        if self.state.pressures != values {
            self.state.pressures = values.to_vec();
            if self.visuals {
                self.refresh(ui);
            }
        }
    }

    pub fn layout(&self) -> &'static str {
        if self.visuals {
            LAYOUT
        } else {
            PLAIN_LAYOUT
        }
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn page(&self) -> usize {
        self.page
    }

    /// The selected parameter's id.
    pub fn focus(&self) -> Option<u16> {
        self.focus.map(|i| self.state.params[i].id)
    }

    /// Shows page `i` (wrapping).
    pub fn show_page(&mut self, ui: &mut Ui, i: usize) {
        if self.pages.is_empty() {
            return;
        }
        let i = i % self.pages.len();
        if i != self.page {
            self.page = i;
            self.focus = None;
        }
        // A node may show the visual strip on some pages only.
        let visuals = self.wants_visuals();
        if visuals != self.visuals {
            self.visuals = visuals;
            // Every widget the layouts name is added in `add` (or before
            // `set_layout_wrap`), so this holds.
            let _ = ui.add_page(&self.name, &self.spec());
        }
        self.refresh(ui);
    }

    /// SHIFT + wheel: the next or previous page.
    pub fn step_page(&mut self, ui: &mut Ui, delta: i32) {
        let n = self.pages.len() as i32;
        if n > 0 {
            self.show_page(ui, (self.page as i32 + delta).rem_euclid(n) as usize);
        }
    }

    /// The wheel picking tiles: the page's tile `delta` along from the
    /// selected one (stopping at the ends; from the first or last when
    /// none is selected), selected as a tap on it would.
    pub fn pick_tile(&mut self, ui: &mut Ui, delta: i32) {
        let Some(page) = self.pages.get(self.page) else {
            return;
        };
        let n = page.params.len() as i64;
        if n == 0 || delta == 0 {
            return;
        }
        let at = self
            .focus
            .and_then(|f| page.params.iter().position(|&i| i == f))
            .map_or(if delta > 0 { -1 } else { n }, |k| k as i64);
        let k = (at + i64::from(delta)).clamp(0, n - 1) as usize;
        self.focus = Some(page.params[k]);
        self.refresh(ui);
    }

    /// The wheel's push opened (true) or closed the selected tile.
    pub fn set_editing(&mut self, ui: &mut Ui, on: bool) {
        if self.editing != on {
            self.editing = on;
            self.refresh(ui);
        }
    }

    /// A Q-Link (0 to 3) turned by `ticks` detents.
    pub fn qlink(&mut self, ui: &mut Ui, n: usize, ticks: i32) -> Option<SynthAction> {
        let page = self.pages.get(self.page)?;
        let i = *qlink_targets(page, self.state.params).get(n)?;
        self.turn(ui, i, ticks)
    }

    /// The data wheel turned by `ticks`: turns the selected tile, or the
    /// first Q-Link's when none is selected.
    pub fn wheel(&mut self, ui: &mut Ui, ticks: i32) -> Option<SynthAction> {
        let i = match self.focus {
            Some(i) => i,
            None => *qlink_targets(self.pages.get(self.page)?, self.state.params).first()?,
        };
        self.turn(ui, i, ticks)
    }

    /// Turns parameter `i` by `ticks` detents: one step for whole numbers and
    /// choices, 1/[`DETENTS`] of the range along its curve otherwise.
    fn turn(&mut self, ui: &mut Ui, i: usize, ticks: i32) -> Option<SynthAction> {
        let p = &self.state.params[i];
        let v = self.state.values[i];
        let new = match p.curve {
            Curve::Stepped | Curve::Choice(_) | Curve::Toggle => v + ticks as f32,
            _ => p.from_unit(p.to_unit(v) + ticks as f32 / DETENTS),
        };
        self.focus = Some(i);
        self.set(ui, i, new)
    }

    fn set(&mut self, ui: &mut Ui, i: usize, v: f32) -> Option<SynthAction> {
        let set = self.state.set(i, v);
        self.refresh(ui);
        set.map(|value| SynthAction::Set {
            id: self.state.params[i].id,
            value,
        })
    }

    /// A touch on one of the panel's widgets; None for events that only move
    /// the view (a page tab, a selection) or are not the panel's.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent) -> Option<SynthAction> {
        let tile_param =
            |s: &SynthPanel, k: usize| s.pages.get(s.page).and_then(|p| p.params.get(k)).copied();
        match (ev.widget.as_str(), &ev.kind) {
            (TITLE, EventKind::Tab(i)) => {
                self.show_page(ui, *i);
                None
            }
            (VISUALS, EventKind::Tab(i)) => {
                let want = self.card_pages.get(*i)?;
                let page = self.pages.iter().position(|p| &p.name == want)?;
                self.show_page(ui, page);
                None
            }
            (VISUALS, EventKind::Select(k)) => {
                let want = self.node_pages.get(*k)?;
                let page = self.pages.iter().position(|p| &p.name == want)?;
                self.show_page(ui, page);
                None
            }
            (TILES, EventKind::Select(k)) => {
                self.focus = tile_param(self, *k);
                self.refresh(ui);
                None
            }
            (TILES, EventKind::Value(pos)) => {
                let i = self.focus?;
                let p = self.state.params[i];
                self.set(ui, i, p.from_unit(*pos))
            }
            (TILES, EventKind::Step { index, delta }) => {
                let i = tile_param(self, *index)?;
                self.focus = Some(i);
                let v = self.state.values[i] + *delta as f32;
                self.set(ui, i, v)
            }
            (QLINKS, EventKind::Select(n)) => {
                let page = self.pages.get(self.page)?;
                self.focus = qlink_targets(page, self.state.params).get(*n).copied();
                self.refresh(ui);
                None
            }
            (PRESETS, EventKind::Select(i)) if *i < self.state.presets.len() => {
                self.state.preset = Some(*i);
                self.state.edited = false;
                self.refresh(ui);
                Some(SynthAction::Preset(*i))
            }
            _ => None,
        }
    }

    /// Puts the state into the widgets.
    pub fn refresh(&mut self, ui: &mut Ui) {
        let st = &self.state;
        let page = self.pages.get(self.page).cloned().unwrap_or(Page {
            name: String::new(),
            params: Vec::new(),
            qlinks: Vec::new(),
        });
        let qtargets = qlink_targets(&page, st.params);
        let preset = st.preset.and_then(|i| st.presets.get(i)).cloned();
        if let Some(w) = ui.get_mut::<TitleBar>(TITLE) {
            w.swatch = st.color;
            w.title = match &preset {
                Some(p) => format!("{} · {}", st.info.name, p),
                None => st.info.name.to_string(),
            };
            w.subtitle = subtitle(st);
            // Many pages (the drum synth's one per pad) show short tabs:
            // "Pad 12" becomes "12".
            let many = self.pages.len() > 8;
            w.tabs = self
                .pages
                .iter()
                .map(|p| match p.name.strip_prefix("Pad ") {
                    Some(n) if many => n.to_string(),
                    _ => p.name.clone(),
                })
                .collect();
            w.active = self.page;
        }
        if self.visuals {
            let strip = match press_base(st.info) {
                Some(base) if page.name == "Pressure" => pressure_cards(st, base),
                _ if st.info.id == "mpfree.fm" => fm_cards(st, &page.name),
                _ if st.info.id == "mpfree.drums" => drum_cards(st, &page.name),
                _ if st.info.id == "mpfree.pluck" => pluck_cards(st, &page.name),
                _ if st.info.id == "mpfree.wavetable" => wt_cards(st, &page.name),
                _ => poly_cards(st, &page.name),
            };
            self.card_pages = strip.pages;
            self.node_pages = strip.node_pages;
            if let Some(w) = ui.get_mut::<Visuals>(VISUALS) {
                w.cards = strip.cards;
            }
        }
        if let Some(w) = ui.get_mut::<TileGrid>(TILES) {
            w.title = page.name.clone();
            w.caption = if qtargets.is_empty() {
                String::new()
            } else {
                "Q-Links turn the marked tiles · drag to turn".to_string()
            };
            w.tiles = page
                .params
                .iter()
                .map(|&i| tile(st, &page.name, i, qtargets.iter().position(|&q| q == i)))
                .collect();
            w.selected = self
                .focus
                .and_then(|f| page.params.iter().position(|&i| i == f));
            w.editing = self.editing;
        }
        if let Some(w) = ui.get_mut::<ListView>(PRESETS) {
            w.title = format!("Presets · {}", st.presets.len());
            w.items = st
                .presets
                .iter()
                .enumerate()
                .map(|(k, name)| ListItem {
                    text: name.clone(),
                    detail: if st.preset == Some(k) && st.edited {
                        "edited".to_string()
                    } else {
                        String::new()
                    },
                    color: None,
                })
                .collect();
            w.selected = st
                .preset
                .unwrap_or(0)
                .min(st.presets.len().saturating_sub(1));
        }
        if let Some(w) = ui.get_mut::<QLinkStrip>(QLINKS) {
            w.cells = qtargets
                .iter()
                .map(|&i| {
                    let p = &st.params[i];
                    QCell {
                        label: p.name.to_string(),
                        value: param_line(st, i),
                        pos: p.to_unit(st.values[i]),
                        bipolar: p.min < 0.0 && p.max > 0.0,
                    }
                })
                .collect();
            match self.focus {
                Some(i) => {
                    let p = &st.params[i];
                    w.wheel_label = format!("Wheel · {}", tile_label(&page.name, p));
                    w.wheel_value = param_line(st, i);
                }
                None => {
                    w.wheel_label = "Wheel".to_string();
                    w.wheel_value = "Tap a tile to turn it".to_string();
                }
            }
        }
    }
}

fn subtitle(st: &SynthState) -> String {
    let kind = if st.info.id.starts_with("mpfree.") {
        "Native"
    } else {
        "Plugin"
    };
    let mut parts = vec![kind.to_string()];
    if !st.track.is_empty() {
        // "Pads" reads "Pads track"; a track's own name ("Track 2") stands
        // as it is.
        let track = if st.track.to_ascii_lowercase().contains("track") {
            st.track.clone()
        } else {
            format!("{} track", st.track)
        };
        parts.insert(0, track);
    }
    if st.info.id == "mpfree.poly" {
        let mode = pp::VOICE_MODES
            .get(st.value(pp::VOICE_MODE) as usize)
            .copied()
            .unwrap_or("");
        parts.push(mode.to_lowercase());
        if st.value(pp::GLIDE) > 0.0 {
            parts.push("glide".to_string());
        }
        if st.value(pp::UNISON) > 1.0 {
            parts.push(format!("unison {}", st.value(pp::UNISON) as i32));
        }
    }
    parts.join(" · ")
}

/// Parameter `i` as a tile.
fn tile(st: &SynthState, page: &str, i: usize, qlink: Option<usize>) -> Tile {
    let p = &st.params[i];
    let v = st.values[i];
    let (value, unit) = param_text(st, i);
    let kind = if p.readonly {
        TileKind::Meter
    } else {
        match p.curve {
            Curve::Choice(names) => TileKind::Choice {
                names: names.iter().map(|s| s.to_string()).collect(),
                index: v as usize,
            },
            Curve::Toggle => TileKind::Toggle(v >= 0.5),
            _ => TileKind::Knob {
                pos: p.to_unit(v),
                bipolar: p.min < 0.0 && p.max > 0.0,
            },
        }
    };
    let color = match (qlink, &kind) {
        (Some(q), _) => QLINK_COLORS[q % 4],
        (None, TileKind::Knob { .. }) => TEXT_DIM,
        (None, _) => QLINK_COLORS[0],
    };
    Tile {
        label: tile_label(page, p),
        value,
        unit,
        kind,
        qlink,
        color,
    }
}

/// The Pressure page's two cards: the curve (shaped against raw, with the
/// threshold and full-at points marked) and a live bar per sounding voice.
fn pressure_cards(st: &SynthState, base: u16) -> Strip {
    let v = |id: u16| st.value(id);
    let (blue, green) = (QLINK_COLORS[0], QLINK_COLORS[1]);
    let thr = v(base + field::THRESHOLD) / 100.0;
    let full = (v(base + field::FULL_AT) / 100.0).max(thr + 1.0 / 127.0);
    let mode = v(base + field::MODE).round() as usize;
    let voices = match mode {
        2 => "off".to_string(),
        1 => "highest held".to_string(),
        _ if st.pressures.is_empty() => "play a pad".to_string(),
        _ => format!("{} sounding", st.pressures.len()),
    };
    // Up to 16 bars; empty slots show as stubs so the card keeps its shape.
    let mut bars = st.pressures.clone();
    bars.truncate(16);
    let slots = bars.len().clamp(8, 16);
    bars.resize(slots, 0.0);
    let mut s = Strip::new();
    s.push(
        Card {
            title: "Pressure".into(),
            caption: visual::pressure_caption(&v, base),
            traces: vec![Trace {
                pts: visual::pressure_curve(&v, base),
                color: blue,
                width: 2.5,
                fill: true,
                dashed: false,
            }],
            grid: vec![(thr, "thr".to_string()), (full, "full".to_string())],
            active: true,
            ..Card::default()
        },
        "Pressure",
    );
    s.push(
        Card {
            title: "Voices".into(),
            caption: voices,
            bars,
            bar_color: Some(green),
            active: true,
            ..Card::default()
        },
        "Pressure",
    );
    s
}

/// Poly's four cards, from its values. The card for the page shown is marked.
fn poly_cards(st: &SynthState, page: &str) -> Strip {
    let v = |id: u16| st.value(id);
    let (blue, green, amber) = (QLINK_COLORS[0], QLINK_COLORS[1], QLINK_COLORS[2]);
    let (white, grey) = (TEXT, TEXT_DIM);
    let trace = |pts: Vec<(f32, f32)>, color: Color, fill: bool, dashed: bool| Trace {
        pts,
        color,
        width: if dashed { 1.5 } else { 2.5 },
        fill,
        dashed,
    };
    let [o1, o2] = visual::oscillators(&v);
    let [f, moved] = visual::filter(&v);
    let [amp, fenv] = visual::envelopes(&v);
    let mode = pp::FILTER_MODES
        .get(v(pp::FILTER_MODE) as usize)
        .copied()
        .unwrap_or("");
    let freq_grid = [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")]
        .iter()
        .map(|&(hz, s)| (visual::freq_x(hz), s.to_string()))
        .collect();
    let cards = vec![
        Card {
            title: "Osc".into(),
            caption: visual::osc_caption(&v),
            traces: [(o1, blue), (o2, green)]
                .into_iter()
                .filter(|(p, _)| !p.is_empty())
                .map(|(p, c)| trace(p, c, false, false))
                .collect(),
            midline: true,
            active: page == "Osc",
            ..Card::default()
        },
        Card {
            title: "Filter".into(),
            caption: mode.to_string(),
            traces: [trace(f, blue, true, false)]
                .into_iter()
                .chain((!moved.is_empty()).then(|| trace(moved, grey, false, true)))
                .collect(),
            grid: freq_grid,
            dot: Some(visual::freq_x(v(pp::CUTOFF))),
            active: page == "Filter",
            ..Card::default()
        },
        Card {
            title: "Env".into(),
            caption: "amp, filter".into(),
            traces: vec![
                trace(amp, white, true, false),
                trace(fenv, blue, false, false),
            ],
            active: page == "Env",
            ..Card::default()
        },
        Card {
            title: "LFO".into(),
            caption: visual::lfo_caption(&v),
            traces: vec![trace(visual::lfo(&v), amber, false, false)],
            midline: true,
            active: page == "LFO",
            ..Card::default()
        },
    ];
    let mut s = Strip::new();
    for c in cards {
        let title = c.title.clone();
        s.push(c, &title);
    }
    s
}

/// Drums' families' colours: a tuned body, noise over a body, metal.
const FAMILY: [Color; 3] = [QLINK_COLORS[1], QLINK_COLORS[2], QLINK_COLORS[0]];

/// Drums' four cards: the kit (a box per pad, the pad shown ringed), the
/// hit and the pitch sweep of the pad shown (pad 1 on the Kit page), and
/// every pad's level.
fn drum_cards(st: &SynthState, page: &str) -> Strip {
    use mpfree_synth::drum_params::{pad_id, BASE_NOTE, LEVEL, PADS};
    let v = |id: u16| st.value(id);
    let pad = page
        .strip_prefix("Pad ")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=PADS).contains(n))
        .map(|n| n - 1);
    let shown = pad.unwrap_or(0);
    let shape = kv::pad_hit(&v, shown);
    let pad_page = format!("Pad {}", shown + 1);
    let mut s = Strip::new();
    let nodes = (0..PADS)
        .map(|p| {
            let model = kv::pad_model(&v, p);
            let (x, y) = kv::pad_xy(p);
            CardNode {
                x,
                y,
                label: kv::model_tag(model).to_string(),
                fill: FAMILY[kv::model_family(model)],
                text: ON_BOX,
                ring: (pad == Some(p)).then_some(WHITE),
                dim: v(pad_id(p, mpfree_synth::drum_params::field::LEVEL)) <= -60.0,
                feedback: None,
            }
        })
        .collect();
    s.node_pages.extend((1..=PADS).map(|n| format!("Pad {n}")));
    s.push(
        Card {
            title: "Kit".into(),
            caption: match pad {
                Some(p) => format!("Pad {} · {}", p + 1, kv::pad_model(&v, p)),
                None => format!("Pad 1 plays note {:.0}", v(BASE_NOTE)),
            },
            nodes,
            active: page == "Kit",
            ..Card::default()
        },
        "Kit",
    );
    let secs = kv::hit_window(&shape);
    let [body, noise, click] = kv::hit_parts(&shape, secs);
    let part = |pts: Vec<(f32, f32)>, color: Color, fill: bool| Trace {
        pts,
        color,
        width: 2.0,
        fill,
        dashed: false,
    };
    s.push(
        Card {
            title: "Hit".into(),
            caption: kv::hit_caption(&v, shown),
            traces: [
                (body, WHITE, true),
                (noise, QLINK_COLORS[0], false),
                (click, QLINK_COLORS[2], false),
            ]
            .into_iter()
            .filter(|(p, _, _)| !p.is_empty())
            .map(|(p, c, f)| part(p, c, f))
            .collect(),
            grid: kv::time_grid(secs),
            active: pad.is_some(),
            ..Card::default()
        },
        &pad_page,
    );
    let sweep = kv::sweep_curve(&shape);
    s.push(
        Card {
            title: "Sweep".into(),
            caption: kv::sweep_caption(&v, shown, &shape),
            traces: (!sweep.is_empty())
                .then(|| part(sweep, QLINK_COLORS[2], true))
                .into_iter()
                .collect(),
            grid: kv::time_grid(kv::SWEEP_SECS),
            active: pad.is_some(),
            ..Card::default()
        },
        &pad_page,
    );
    let level = |p: usize| v(pad_id(p, mpfree_synth::drum_params::field::LEVEL));
    s.push(
        Card {
            title: "Levels".into(),
            caption: match pad {
                Some(p) => format!("Pad {} at {:+.1} dB", p + 1, level(p)),
                None => format!("kit at {:+.1} dB", v(LEVEL)),
            },
            bars: kv::pad_levels(&v),
            bar_colors: (0..PADS)
                .map(|p| {
                    if pad == Some(p) {
                        WHITE
                    } else {
                        FAMILY[kv::model_family(kv::pad_model(&v, p))]
                    }
                })
                .collect(),
            bar_labels: (1..=PADS)
                .map(|n| {
                    if n == 1 || n % 4 == 0 {
                        n.to_string()
                    } else {
                        String::new()
                    }
                })
                .collect(),
            active: page == "Kit",
            ..Card::default()
        },
        "Kit",
    );
    s
}

/// Pluck's three cards, for the model it plays: a string (the pick, the
/// harmonics, their decays) or a mallet (its pulse, the modes, their
/// decays).
fn pluck_cards(st: &SynthState, page: &str) -> Strip {
    let v = |id: u16| st.value(id);
    let mallet = kv::is_mallet(&v);
    let home = if mallet { "Mallet" } else { "String" };
    let active = page == home;
    let parts = kv::note_partials(&v);
    let mut s = Strip::new();
    if mallet {
        let (pulse, len) = kv::mallet_pulse(&v);
        s.push(
            Card {
                title: "Mallet".into(),
                caption: format!("{:.1} ms on the bar", len * 1000.0),
                traces: vec![Trace {
                    pts: pulse,
                    color: QLINK_COLORS[2],
                    width: 2.5,
                    fill: true,
                    dashed: false,
                }],
                grid: kv::time_grid(0.008),
                active,
                ..Card::default()
            },
            home,
        );
        let material = mpfree_synth::pluck_params::MATERIALS
            .get(v(mpfree_synth::pluck_params::MATERIAL) as usize)
            .copied()
            .unwrap_or("");
        s.push(
            Card {
                title: "Modes".into(),
                caption: format!("{material}, as struck"),
                traces: kv::mode_stems(&parts)
                    .into_iter()
                    .map(|pts| Trace {
                        pts,
                        color: QLINK_COLORS[1],
                        width: 3.0,
                        fill: false,
                        dashed: false,
                    })
                    .collect(),
                grid: [1.0, 2.0, 4.0, 8.0, 16.0]
                    .iter()
                    .map(|&r| (kv::ratio_x(r), format!("{r}x")))
                    .collect(),
                active,
                ..Card::default()
            },
            home,
        );
    } else {
        let (string, x) = kv::pick_shape(&v);
        s.push(
            Card {
                title: "Pick".into(),
                caption: kv::pick_caption(&v),
                traces: vec![Trace {
                    pts: string,
                    color: WHITE,
                    width: 2.0,
                    fill: false,
                    dashed: false,
                }],
                dot: Some(x),
                active,
                ..Card::default()
            },
            home,
        );
        s.push(
            Card {
                title: "Harmonics".into(),
                caption: "a pluck on average".into(),
                bars: parts.iter().map(|p| kv::level_height(p.level)).collect(),
                bar_color: Some(QLINK_COLORS[1]),
                bar_labels: (1..=parts.len())
                    .map(|n| {
                        if n == 1 || n % 4 == 0 {
                            n.to_string()
                        } else {
                            String::new()
                        }
                    })
                    .collect(),
                active,
                ..Card::default()
            },
            home,
        );
    }
    let picked = kv::decay_partials(&parts, mallet);
    let secs = kv::decay_window(&picked);
    let fade = [WHITE, QLINK_COLORS[1], QLINK_COLORS[0], QLINK_COLORS[3]];
    s.push(
        Card {
            title: "Decay".into(),
            caption: format!(
                "{} at middle C, {}",
                kv::secs_text(picked.first().map_or(0.0, |p| p.t60)),
                if mallet { "modes 1-4" } else { "harm. 1 2 4 8" }
            ),
            traces: picked
                .iter()
                .zip(fade)
                .map(|(p, color)| Trace {
                    pts: kv::decay_curve(p, secs),
                    color,
                    width: 2.0,
                    fill: false,
                    dashed: false,
                })
                .collect(),
            grid: kv::time_grid(secs),
            active,
            ..Card::default()
        },
        home,
    );
    s
}

/// The Wavetable synth's cards ([`crate::wt_visual`]).
fn wt_cards(st: &SynthState, page: &str) -> Strip {
    let v = |id: u16| st.value(id);
    let mut s = Strip::new();
    for c in crate::wt_visual::cards(&v, page) {
        let title = c.title.clone();
        s.push(c, &title);
    }
    s
}

/// Carriers' colour, modulators' colour, and the text on both.
const CARRIER: Color = QLINK_COLORS[1];
const MODULATOR: Color = QLINK_COLORS[0];
const ON_BOX: Color = ON_FILL;
const LINE: Color = TEXT_DIM;
const WHITE: Color = TEXT;

fn line(pts: Vec<(f32, f32)>, color: Color) -> Trace {
    Trace {
        pts,
        color,
        width: 2.5,
        fill: false,
        dashed: false,
    }
}

/// FM's cards for a page: the algorithm with the operators' levels (Algo),
/// with an operator's envelope (Op n) or with two operators' scaling
/// (Kbd), or the pitch envelope and the LFO (Mod).
fn fm_cards(st: &SynthState, page: &str) -> Strip {
    let v = |id: u16| st.value(id);
    let mut s = Strip::new();
    let op_page = page
        .strip_prefix("Op ")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=6).contains(n));
    let kbd = page.strip_prefix("Kbd ").and_then(|r| {
        let (a, b) = r.split_once('-')?;
        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
    });
    match (page, op_page, kbd) {
        ("Mod", _, _) => {
            s.push(
                Card {
                    title: "Pitch env".into(),
                    caption: fv::pitch_env_caption(&v),
                    traces: vec![line(fv::pitch_env(&v), QLINK_COLORS[2])],
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                "Mod",
            );
            s.push(
                Card {
                    title: "LFO".into(),
                    caption: fv::lfo_caption(&v),
                    traces: vec![line(fv::lfo(&v), QLINK_COLORS[1])],
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                "Mod",
            );
        }
        (_, Some(n), _) => {
            algorithm_card(&mut s, st, &[(n, WHITE)], false);
            let l = fv::algorithm_layout(st.value(fp::ALGORITHM) as usize);
            let carrier = l.carriers & (1 << (n - 1)) != 0;
            let caption = if fv::op_sounds(&v, n) {
                format!(
                    "{} · output {}",
                    fv::op_role(&l, n),
                    v(fp::op_id(n, fp::op::LEVEL)).round()
                )
            } else {
                format!("{} · off", fv::op_role(&l, n))
            };
            let color = if carrier { CARRIER } else { MODULATOR };
            s.push(
                Card {
                    title: format!("Op {n} env"),
                    caption,
                    traces: vec![Trace {
                        pts: fv::op_env(&v, n),
                        color,
                        width: 2.5,
                        fill: true,
                        dashed: false,
                    }],
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        (_, _, Some((a, b))) => {
            // Each operator's ring is the colour of its curve.
            let (ca, cb) = (WHITE, QLINK_COLORS[2]);
            algorithm_card(&mut s, st, &[(a, ca), (b, cb)], false);
            let grid = [36u8, 60, 84]
                .iter()
                .map(|&k| (fv::key_x(k), fv::note_name(k)))
                .collect();
            let range = fv::kbd_range_db(&v, &[a, b]);
            let caption = format!(
                "breaks {}, {} · ±{range} dB",
                fv::note_name(fv::break_note(&v, a)),
                fv::note_name(fv::break_note(&v, b))
            );
            s.push(
                Card {
                    title: "Scaling".into(),
                    caption,
                    traces: vec![
                        line(fv::kbd_curve(&v, a, range), ca),
                        line(fv::kbd_curve(&v, b, range), cb),
                    ],
                    grid,
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        _ => {
            algorithm_card(&mut s, st, &[], true);
            let l = fv::algorithm_layout(st.value(fp::ALGORITHM) as usize);
            let carriers = l.carriers.count_ones();
            let off = (1..=6).filter(|&n| !fv::op_sounds(&v, n)).count();
            let mut caption = format!("{carriers} carrier{}", if carriers == 1 { "" } else { "s" });
            if off > 0 {
                caption += &format!(" · {off} off");
            }
            s.push(
                Card {
                    title: "Levels".into(),
                    caption,
                    bars: (1..=6)
                        .map(|n| {
                            if fv::op_sounds(&v, n) {
                                v(fp::op_id(n, fp::op::LEVEL)) / 99.0
                            } else {
                                0.0
                            }
                        })
                        .collect(),
                    bar_colors: (0..6)
                        .map(|j| {
                            if l.carriers & (1 << j) != 0 {
                                CARRIER
                            } else {
                                MODULATOR
                            }
                        })
                        .collect(),
                    bar_labels: (1..=6).map(|n| n.to_string()).collect(),
                    active: page == "Algo",
                    ..Card::default()
                },
                "Algo",
            );
        }
    }
    s
}

/// The algorithm card: the operators as boxes, those in `ring` ringed in
/// their colour, a tap on a box opening its page. A tap elsewhere on the
/// card opens the Algo page.
fn algorithm_card(s: &mut Strip, st: &SynthState, ring: &[(usize, Color)], active: bool) {
    let v = |id: u16| st.value(id);
    let l = fv::algorithm_layout(st.value(fp::ALGORITHM) as usize);
    let fb = v(fp::FEEDBACK) > 0.0;
    let nodes = (0..6)
        .map(|j| {
            let carrier = l.carriers & (1 << j) != 0;
            CardNode {
                x: l.ops[j].0,
                y: l.ops[j].1,
                label: (j + 1).to_string(),
                fill: if carrier { CARRIER } else { MODULATOR },
                text: ON_BOX,
                ring: ring.iter().find(|r| r.0 == j + 1).map(|r| r.1),
                dim: !fv::op_sounds(&v, j + 1),
                feedback: (fb && j == l.route.fb_from).then_some(l.route.fb_to),
            }
        })
        .collect();
    let traces = fv::algorithm_lines(&l)
        .into_iter()
        .map(|pts| Trace {
            pts,
            color: LINE,
            width: 1.5,
            fill: false,
            dashed: false,
        })
        .collect();
    s.node_pages.extend((1..=6).map(|n| format!("Op {n}")));
    s.push(
        Card {
            title: "Algorithm".into(),
            caption: fv::algorithm_caption(&v),
            traces,
            nodes,
            active,
            ..Card::default()
        },
        "Algo",
    );
}
