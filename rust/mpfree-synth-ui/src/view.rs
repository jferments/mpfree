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
use crate::pages::{plan, qlink_targets, Page};
use crate::section_view::{Control, Kind, SectionBox, SectionView};
use crate::sections::{arrange, Arrangement, Section};
use crate::visual;
use crate::widgets::{Card, CardNode, TitleBar, Trace, ON_FILL, QLINK_COLORS, TEXT, TEXT_DIM};
use mpfree_node::pressure::{field, FIELDS};
use mpfree_node::{Curve, Node, NodeInfo, ParamInfo};
use mpfree_synth::params as pp;
use mpfree_synth::{fm_params as fp, pressure as sp};
use mpfree_ui::qlink::QLinkCard;
use mpfree_ui::text::FontId;
use mpfree_ui::widgets::{ListItem, ListView};
use mpfree_ui::{Align, Color, EventKind, PaintCx, Rect, Touch, Ui, UiEvent, Widget};

/// Widget names the panel adds. The app's own are `header` and `hint`,
/// and the Q-Link column ([`QLINK_COLUMN`]), which the app fills from
/// [`SynthPanel::qlink_cards`].
pub const TITLE: &str = "synth-title";
pub const TILES: &str = "synth-tiles";
pub const PRESETS: &str = "synth-presets";
/// The instrument card over the presets: the engine, its track, its kind.
pub const INST: &str = "synth-inst";
/// The app's Q-Link column (the Play page's, `play_page::QLINKS`).
pub const QLINK_COLUMN: &str = "qlinks";

/// The frame Jay approved on 14 Nov: the instrument and its presets on the
/// left (220), the page in the middle (844), the Q-Link column on the
/// right (168), the same column as on Play.
pub const LAYOUT: &str = "col(header:60, row(col(synth-inst:72, synth-presets:*):220, col(synth-title:56, synth-tiles:*):*, qlinks:168):*, hint:26)";
/// The presets' slot in [`LAYOUT`], for an app that puts a widget over
/// them (the pad player's pads heading).
pub const PRESETS_SLOT: &str = "synth-presets:*";

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
    /// The page's sections ([`crate::sections`]).
    arrangement: Arrangement,
    /// The selected control, as an index into the node's parameters.
    focus: Option<usize>,
    /// The wheel's push opened the selected control: the wheel turns it
    /// (the app's wheel navigation); otherwise the wheel picks controls.
    editing: bool,
    /// The page each `Tab(n)` from the sections opens (a section's
    /// "Open ›", a box of a drawing).
    links: Vec<String>,
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
    /// Adds the panel's widgets and its page `name` to `ui`. The app adds
    /// `header`, `hint` and the Q-Link column ([`QLINK_COLUMN`]) first.
    pub fn add(ui: &mut Ui, name: &str, state: SynthState) -> Result<SynthPanel, String> {
        ui.add(TITLE, TitleBar::new());
        ui.add(INST, InstCard::new());
        ui.add(TILES, SectionView::new());
        ui.add(PRESETS, ListView::new("Presets"));
        let mut panel = SynthPanel {
            name: name.to_string(),
            pages: Vec::new(),
            page: 0,
            arrangement: Arrangement {
                spec: String::new(),
                sections: Vec::new(),
            },
            focus: None,
            editing: false,
            links: Vec::new(),
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
        self.arrange();
        ui.add_page(&self.name, &self.spec())?;
        self.refresh(ui);
        Ok(())
    }

    fn arrange(&mut self) {
        let empty = Page {
            name: String::new(),
            params: Vec::new(),
            qlinks: Vec::new(),
        };
        let page = self.pages.get(self.page).unwrap_or(&empty);
        self.arrangement = arrange(self.state.info, self.state.params, page);
    }

    /// Each sounding voice's pressure, 0 to 1, for the Pressure page's live
    /// bars. The app calls it a few times a second while the page shows.
    pub fn set_pressures(&mut self, ui: &mut Ui, values: &[f32]) {
        if self.state.pressures != values {
            self.state.pressures = values.to_vec();
            if self
                .pages
                .get(self.page)
                .is_some_and(|p| p.name == "Pressure")
            {
                self.refresh(ui);
            }
        }
    }

    /// The page's layout: the same for every node and page.
    pub fn layout(&self) -> &'static str {
        LAYOUT
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn page(&self) -> usize {
        self.page
    }

    /// The page's sections.
    pub fn arrangement(&self) -> &Arrangement {
        &self.arrangement
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
            self.arrange();
        }
        self.refresh(ui);
    }

    /// Shows the page called `name`, if there is one.
    pub fn show_page_named(&mut self, ui: &mut Ui, name: &str) -> bool {
        match self.pages.iter().position(|p| p.name == name) {
            Some(i) => {
                self.show_page(ui, i);
                true
            }
            None => false,
        }
    }

    /// SHIFT + wheel: the next or previous page.
    pub fn step_page(&mut self, ui: &mut Ui, delta: i32) {
        let n = self.pages.len() as i32;
        if n > 0 {
            self.show_page(ui, (self.page as i32 + delta).rem_euclid(n) as usize);
        }
    }

    /// The wheel picking controls: the control `delta` along from the
    /// selected one in the sections' order (stopping at the ends; from the
    /// first or last when none is selected), selected as a tap on it would.
    pub fn pick_tile(&mut self, ui: &mut Ui, delta: i32) {
        let order = self.order();
        let n = order.len() as i64;
        if n == 0 || delta == 0 {
            return;
        }
        let at = self
            .focus
            .and_then(|f| order.iter().position(|&i| i == f))
            .map_or(if delta > 0 { -1 } else { n }, |k| k as i64);
        let k = (at + i64::from(delta)).clamp(0, n - 1) as usize;
        self.focus = Some(order[k]);
        self.refresh(ui);
    }

    /// The controls on screen that can be changed, in the sections' order.
    pub fn order(&self) -> Vec<usize> {
        self.arrangement
            .order()
            .into_iter()
            .filter(|&i| !self.state.params[i].readonly)
            .collect()
    }

    /// The wheel's push opened (true) or closed the selected control.
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

    /// What the four Q-Links turn on this page, for the app's Q-Link
    /// column: each target's name as its section shows it, the section's
    /// name, and its value. A Q-Link with nothing here is blank.
    pub fn qlink_cards(&self) -> [QLinkCard; 4] {
        let st = &self.state;
        let mut cards: [QLinkCard; 4] = Default::default();
        let Some(page) = self.pages.get(self.page) else {
            return cards;
        };
        for (card, &i) in cards.iter_mut().zip(&qlink_targets(page, st.params)) {
            let p = &st.params[i];
            let (name, scope) = match self
                .arrangement
                .sections
                .iter()
                .find(|s| s.params.contains(&i))
            {
                Some(s) => (s.label(i, p), s.name.clone()),
                None => (p.name.to_string(), page.name.clone()),
            };
            *card = QLinkCard::new(&name, &scope, &param_line(st, i), p.to_unit(st.values[i]));
            card.bipolar = p.min < 0.0 && p.max > 0.0;
        }
        cards
    }

    /// The selected control and its value ("Cutoff 420 Hz"), for the
    /// app's hint line; None when nothing is selected.
    pub fn wheel_text(&self) -> Option<String> {
        let i = self.focus?;
        let p = &self.state.params[i];
        let name = self
            .arrangement
            .sections
            .iter()
            .find(|s| s.params.contains(&i))
            .map_or_else(|| p.name.to_string(), |s| s.label(i, p));
        Some(format!("{name} {}", param_line(&self.state, i)))
    }

    /// The data wheel turned by `ticks`: turns the selected control, or the
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
        let known = |s: &SynthPanel, i: usize| (i < s.state.params.len()).then_some(i);
        match (ev.widget.as_str(), &ev.kind) {
            (TITLE, EventKind::Tab(i)) => {
                self.show_page(ui, *i);
                None
            }
            (TILES, EventKind::Tab(n)) => {
                let want = self.links.get(*n)?.clone();
                self.show_page_named(ui, &want);
                None
            }
            (TILES, EventKind::Select(i)) => {
                self.focus = known(self, *i);
                self.refresh(ui);
                None
            }
            (TILES, EventKind::Value(pos)) => {
                let i = self.focus?;
                let p = self.state.params[i];
                self.set(ui, i, p.from_unit(*pos))
            }
            (TILES, EventKind::Step { index, delta }) => {
                let i = known(self, *index)?;
                self.focus = Some(i);
                let v = self.state.values[i] + *delta as f32;
                self.set(ui, i, v)
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

    /// The engine's drawings for the page shown, each with the page it
    /// belongs to and the page each of its boxes opens.
    fn drawings(&self, page: &str) -> Vec<(Card, String, Vec<String>)> {
        let st = &self.state;
        let strip = match press_base(st.info) {
            Some(base) if page == "Pressure" => pressure_cards(st, base),
            _ if st.info.id == "mpfree.poly" => poly_cards(st, page),
            _ if st.info.id == "mpfree.fm" => fm_cards(st, page),
            _ if st.info.id == "mpfree.drums" => drum_cards(st, page),
            _ if st.info.id == "mpfree.pluck" => pluck_cards(st, page),
            _ if st.info.id == "mpfree.wavetable" => wt_cards(st, page),
            _ if crate::inst_visual::draws(st.info.id) => inst_cards(st, page),
            _ => Strip::new(),
        };
        let mut first = 0;
        strip
            .cards
            .into_iter()
            .zip(strip.pages)
            .map(|(card, home)| {
                let n = card.nodes.len();
                let nodes = strip
                    .node_pages
                    .get(first..first + n)
                    .map(|s| s.to_vec())
                    .unwrap_or_default();
                first += n;
                (card, home, nodes)
            })
            .collect()
    }

    /// Puts the state into the widgets.
    pub fn refresh(&mut self, ui: &mut Ui) {
        let page = self.pages.get(self.page).cloned().unwrap_or(Page {
            name: String::new(),
            params: Vec::new(),
            qlinks: Vec::new(),
        });
        let drawings = self.drawings(&page.name);
        let st = &self.state;
        let qtargets = qlink_targets(&page, st.params);
        let preset = st.preset.and_then(|i| st.presets.get(i)).cloned();
        if let Some(w) = ui.get_mut::<TitleBar>(TITLE) {
            w.show_swatch = false;
            w.swatch = st.color;
            let edited = if st.edited { " · edited" } else { "" };
            (w.title, w.subtitle) = match &preset {
                Some(p) => (p.clone(), format!("{}{edited}", st.info.name)),
                None => (
                    st.info.name.to_string(),
                    format!("{}{edited}", kind_name(st)),
                ),
            };
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
        if let Some(w) = ui.get_mut::<InstCard>(INST) {
            w.swatch = st.color;
            w.name = st.info.name.to_string();
            w.detail = subtitle(st);
        }
        let mut links: Vec<String> = Vec::new();
        let mut link = |to: &str| match links.iter().position(|l| l == to) {
            Some(n) => n,
            None => {
                links.push(to.to_string());
                links.len() - 1
            }
        };
        let mut boxes = Vec::new();
        for sec in &self.arrangement.sections {
            let mut b = SectionBox::new(&sec.key, &sec.name, sec.job);
            b.rows = sec.rows;
            let mut caption = String::new();
            let mut home = None;
            if let Some((card, at, nodes)) = sec
                .drawing
                .as_ref()
                .and_then(|t| drawings.iter().find(|(c, _, _)| &c.title == t))
            {
                let mut card = card.clone();
                card.active = false;
                caption = card.caption.clone();
                // One curve of a card that draws two (the filter envelope
                // of "amp, filter"): drawn in the section's colour, and the
                // card's caption no longer fits it.
                if let Some(k) = sec.trace {
                    card.traces = card.traces.get(k).cloned().into_iter().collect();
                    for tr in &mut card.traces {
                        tr.color = QLINK_COLORS[0];
                    }
                    if k != 0 {
                        card.dot = None;
                    }
                    caption.clear();
                }
                b.node_links = nodes.iter().map(|p| link(p)).collect();
                b.cards.push(card);
                home = Some(at.clone());
            }
            // A section that is only a drawing opens the drawing's page;
            // one with controls links only where its plan says.
            let to = sec
                .link
                .clone()
                .or(home.filter(|h| sec.params.is_empty() && *h != page.name && !h.is_empty()));
            b.link = to
                .filter(|t| *t != page.name && self.pages.iter().any(|p| &p.name == t))
                .map(|t| link(&t));
            let mut why = None;
            for &i in &sec.params {
                let reason = inactive(st, i);
                if why.is_none() {
                    why = reason.map(|r| format!("{} · {r}", sec.label(i, &st.params[i])));
                }
                b.controls.push(control(
                    st,
                    sec,
                    i,
                    qtargets.iter().position(|&q| q == i),
                    reason.is_some(),
                ));
            }
            b.note = why.or_else(|| sec.note.clone()).unwrap_or(caption);
            boxes.push(b);
        }
        self.links = links;
        if let Some(w) = ui.get_mut::<SectionView>(TILES) {
            w.spec = self.arrangement.spec.clone();
            w.sections = boxes;
            w.selected = self.focus;
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
    }
}

/// "Native" for the app's own engines, "Plugin" for a hosted one.
fn kind_name(st: &SynthState) -> &'static str {
    if st.info.id.starts_with("mpfree.") {
        "Native"
    } else {
        "Plugin"
    }
}

/// Parameter `i` as a control of section `sec`.
fn control(st: &SynthState, sec: &Section, i: usize, qlink: Option<usize>, dim: bool) -> Control {
    let p = &st.params[i];
    let v = st.values[i];
    let (value, unit) = param_text(st, i);
    let pos = p.to_unit(v);
    let bipolar = p.min < 0.0 && p.max > 0.0;
    let fader = sec.faders.contains(&i);
    let kind = if p.readonly {
        Kind::Meter
    } else {
        match p.curve {
            Curve::Choice(names) => Kind::Choice {
                names: names.iter().map(|s| s.to_string()).collect(),
                index: v as usize,
            },
            Curve::Toggle => Kind::Toggle(v >= 0.5),
            _ if fader && sec.drawbars => Kind::Drawbar { pos },
            _ if fader => Kind::Fader { pos, bipolar },
            Curve::Stepped if p.max - p.min <= 48.0 => Kind::Stepper,
            _ if sec.rows => Kind::Slider { pos, bipolar },
            _ => Kind::Knob { pos, bipolar },
        }
    };
    // A drawbar's family: the sub-octaves in the section's colour, the
    // octaves white, the fifths and thirds grey.
    let tint = if fader && sec.drawbars {
        let bar = sec.faders.iter().position(|&f| f == i).unwrap_or(0);
        match crate::inst_visual::drawbar_family(bar) {
            1 => Some(TEXT),
            2 => Some(TEXT_DIM),
            _ => None,
        }
    } else {
        None
    };
    Control {
        index: i,
        label: sec.label(i, p),
        value,
        unit,
        kind,
        tier: sec.tier(i, qlink.is_some()),
        qlink,
        dim,
        tint,
    }
}

/// Why parameter `i` does nothing as the sound is set (a pulse width with
/// no pulse wave, a unison detune in poly mode), if it doesn't.
pub fn inactive(st: &SynthState, i: usize) -> Option<&'static str> {
    let p = &st.params[i];
    let v = |id: u16| st.value(id);
    let id = p.id;
    if let Some(base) = press_base(st.info) {
        let off = id.wrapping_sub(base);
        if off < FIELDS as u16 {
            if off != field::MODE && v(base + field::MODE).round() as usize == 2 {
                return Some("pressure is off");
            }
            for n in 0..4 {
                if off == field::depth(n) && v(base + field::target(n)) < 0.5 {
                    return Some("no target");
                }
            }
            return None;
        }
    }
    match st.info.id {
        "mpfree.analog" => {
            use mpfree_synth::analog_params as an;
            let pulse = |w: u16| matches!(v(w).round() as i32, 1 | 2);
            match id {
                an::OSC1_PW if !pulse(an::OSC1_WAVE) => Some("pulse waves only"),
                an::OSC2_PW if !pulse(an::OSC2_WAVE) => Some("pulse waves only"),
                an::LFO_PW | an::PM_PW if !pulse(an::OSC1_WAVE) && !pulse(an::OSC2_WAVE) => {
                    Some("no pulse wave")
                }
                an::UNI_DETUNE if v(an::VOICE_MODE) < 0.5 => Some("Unison only"),
                an::F_DECAY if v(an::F_SUSTAIN) >= 100.0 => Some("sustain is full"),
                an::A_DECAY if v(an::A_SUSTAIN) >= 100.0 => Some("sustain is full"),
                _ => None,
            }
        }
        "mpfree.poly" | "mpfree.wavetable" => {
            use mpfree_synth::params as pp;
            use mpfree_synth::wavetable::params as wp;
            let poly = st.info.id == "mpfree.poly";
            match id {
                pp::OSC1_PW if poly && v(pp::OSC1_WAVE).round() != 1.0 => Some("pulse only"),
                pp::OSC2_PW if poly && v(pp::OSC2_WAVE).round() != 1.0 => Some("pulse only"),
                pp::UNISON_DETUNE | pp::UNISON_SPREAD if poly && v(pp::UNISON) <= 1.0 => {
                    Some("one voice")
                }
                wp::UNISON_DETUNE | wp::UNISON_SPREAD if !poly && v(wp::UNISON) <= 1.0 => {
                    Some("one voice")
                }
                pp::LFO_RATE if v(pp::LFO_SYNC) >= 0.5 => Some("synced to tempo"),
                pp::AMP_D if v(pp::AMP_S) >= 100.0 => Some("sustain is full"),
                pp::FENV_D if poly && v(pp::FENV_S) >= 100.0 => Some("sustain is full"),
                wp::MENV_D if !poly && v(wp::MENV_S) >= 100.0 => Some("sustain is full"),
                _ if !poly
                    && (id == wp::osc_id(0, wp::WARP) && v(wp::osc_id(0, wp::WARP_MODE)) < 0.5
                        || id == wp::osc_id(1, wp::WARP)
                            && v(wp::osc_id(1, wp::WARP_MODE)) < 0.5) =>
                {
                    Some("warp is off")
                }
                _ => None,
            }
        }
        "mpfree.drawbar" => {
            use mpfree_synth::drawbar_params as db;
            match id {
                db::PERC_HARM | db::PERC_DECAY | db::PERC_VOL if v(db::PERC) < 0.5 => {
                    Some("percussion is off")
                }
                db::SCANNER if v(db::VIBRATO) < 0.5 => Some("vibrato is off"),
                db::SPEED | db::SPREAD if v(db::ROTARY) < 0.5 => Some("rotary is off"),
                _ => None,
            }
        }
        "mpfree.keys" => {
            use mpfree_synth::keys_params as ek;
            match id {
                ek::TREM_RATE | ek::TREM_MODE if v(ek::TREMOLO) <= 0.0 => Some("tremolo is off"),
                _ => None,
            }
        }
        "mpfree.pluck" => {
            use mpfree_synth::pluck_params as kp;
            let mallet = v(kp::MODEL) >= 0.5;
            match id {
                kp::BRIGHTNESS | kp::PICK | kp::POSITION if mallet => Some("for the string"),
                kp::MATERIAL | kp::HARDNESS | kp::STRIKE | kp::STRETCH | kp::HIGH_DAMP
                    if !mallet =>
                {
                    Some("for the mallet")
                }
                _ => None,
            }
        }
        "mpfree.fm" => {
            let op = (1..=6).find(|&n| {
                let base = fp::op_id(n, 0);
                (base..base + 32).contains(&id)
            })?;
            (id != fp::op_id(op, fp::op::ON) && v(fp::op_id(op, fp::op::ON)) < 0.5)
                .then_some("operator is off")
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- instrument card

/// The instrument over the presets: its swatch, its name and a line about
/// it ("Keys track · Native · poly").
pub struct InstCard {
    pub swatch: Color,
    pub name: String,
    pub detail: String,
}

impl InstCard {
    pub fn new() -> InstCard {
        InstCard {
            swatch: QLINK_COLORS[1],
            name: String::new(),
            detail: String::new(),
        }
    }
}

impl Default for InstCard {
    fn default() -> InstCard {
        InstCard::new()
    }
}

impl Widget for InstCard {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let inner = r.inset_xy(14, 12);
        let sw = inner.h.min(40);
        cx.canvas.fill_round_rect(
            Rect::new(inner.x, inner.y + (inner.h - sw) / 2, sw, sw),
            10,
            t.content(self.swatch),
        );
        let text = Rect::new(inner.x + sw + 12, inner.y, inner.w - sw - 12, inner.h);
        let (top, bottom) = text.take_top(text.h * 11 / 20);
        cx.text(
            FontId::Bold,
            t.size_body,
            top,
            Align::Left,
            t.text,
            &self.name,
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            bottom,
            Align::Left,
            t.text_dim,
            &self.detail,
        );
    }

    fn touch(&mut self, _f: u64, _phase: Touch, _x: i32, _y: i32, _r: Rect) -> Option<EventKind> {
        None
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

/// Keys', Drawbar's, Squelch's and Analog's cards
/// ([`crate::inst_visual`]).
fn inst_cards(st: &SynthState, page: &str) -> Strip {
    let v = |id: u16| st.value(id);
    let mut s = Strip::new();
    for (c, home) in crate::inst_visual::cards(st.info.id, &v, page) {
        s.push(c, home);
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
