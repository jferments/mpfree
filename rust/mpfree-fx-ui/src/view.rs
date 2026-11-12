//! An effect's page: its name and where it sits, its drawings, its
//! parameters as tiles, its presets, and its four Q-Links.
//!
//! The panel works like the Synth tab's: the app keeps an [`FxState`]
//! (from [`FxState::from_node`]), shows it with [`FxPanel::add`] or
//! [`FxPanel::load`], and turns the [`FxAction`]s the panel returns into
//! node calls (through the chain or the mod engine, as the chain editor
//! does). The panel changes its own copy of a value at once, so the screen
//! and the drawing follow a finger without waiting for the audio thread.
//! Meters (the compressor's reduction, Master safety's catches) come in
//! with [`FxPanel::set_meters`].
//!
//! Where the page opens is the Hello thread's call and Jay's (see the
//! crate docs); the panel fills whatever area its layout gets, with the
//! app's own `header` and `hint`.

use crate::lock::Locks;
use crate::pages::{plan, qlinks, Page};
use crate::state::{preset_values, FxState, Scratch};
use crate::visual::{self, has_visuals};
use mpfree_node::Curve;
use mpfree_synth_ui::format::value_text;
use mpfree_synth_ui::pages::tile_label;
use mpfree_synth_ui::widgets::{
    QCell, QLinkStrip, Tile, TileGrid, TileKind, TitleBar, Visuals, QLINK_COLORS,
};
use mpfree_ui::widgets::{ListItem, ListView};
use mpfree_ui::{EventKind, Ui, UiEvent};

/// Widget names the panel adds. The app's own are `header` and `hint`.
pub const TITLE: &str = "fx-title";
pub const VISUALS: &str = "fx-visuals";
pub const TILES: &str = "fx-tiles";
pub const PRESETS: &str = "fx-presets";
pub const QLINKS: &str = "fx-qlinks";

/// An effect with drawings and presets (the reverb, Lo-fi).
pub const LAYOUT: &str = "col(header:60, row(col(fx-title:64, fx-visuals:170, fx-tiles:*):*, fx-presets:300):*, fx-qlinks:78, hint:26)";
/// An effect with drawings and no presets: the drawings take the width.
pub const WIDE_LAYOUT: &str =
    "col(header:60, col(fx-title:64, fx-visuals:170, fx-tiles:*):*, fx-qlinks:78, hint:26)";
/// Any other node (a hosted plugin): tiles only.
pub const PLAIN_LAYOUT: &str =
    "col(header:60, col(fx-title:64, fx-tiles:*):*, fx-qlinks:78, hint:26)";

/// Wheel or Q-Link detents for a knob's whole range.
pub const DETENTS: f32 = 100.0;

/// What a touch, a Q-Link or the wheel asks for; the app applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum FxAction {
    /// `Node::set_param(id, value)`.
    Set { id: u16, value: f32 },
    /// Load preset `i`: send [`crate::preset_values`] to the node (the
    /// panel shows its values already).
    Preset(usize),
    /// Lock mode: lock parameter `id` at `value` on the held steps.
    Lock { id: u16, value: f32 },
    /// Lock mode: take the held steps' lock on `id` off.
    Unlock { id: u16 },
}

pub struct FxPanel {
    name: String,
    pages: Vec<Page>,
    page: usize,
    /// The selected tile, as an index into the node's parameters.
    focus: Option<usize>,
    /// The page a tap on each card, and on each box, opens.
    card_pages: Vec<String>,
    node_pages: Vec<String>,
    scratch: Scratch,
    /// Lock mode: the held step and its locks.
    locks: Option<Locks>,
    pub state: FxState,
}

impl FxPanel {
    /// Adds the panel's widgets and its page `name` to `ui` (the app adds
    /// `header` and `hint` first).
    pub fn add(ui: &mut Ui, name: &str, state: FxState) -> Result<FxPanel, String> {
        ui.add(TITLE, TitleBar::new());
        ui.add(VISUALS, Visuals::new());
        ui.add(TILES, TileGrid::new(""));
        ui.add(PRESETS, ListView::new("Presets"));
        ui.add(QLINKS, QLinkStrip::new());
        let mut panel = FxPanel {
            name: name.to_string(),
            pages: Vec::new(),
            page: 0,
            focus: None,
            card_pages: Vec::new(),
            node_pages: Vec::new(),
            scratch: Scratch::None,
            locks: None,
            state: state.clone(),
        };
        panel.load(ui, state)?;
        Ok(panel)
    }

    /// Shows another effect, or the same one after a change made outside
    /// the panel. Keeps the page when the effect is the same kind.
    pub fn load(&mut self, ui: &mut Ui, state: FxState) -> Result<(), String> {
        let same = self.state.info.id == state.info.id && !self.pages.is_empty();
        self.pages = plan(state.info, state.params);
        if !same {
            self.page = 0;
            self.focus = None;
            self.scratch = Scratch::new(&state);
        } else {
            self.scratch.sync(&state);
        }
        self.page = self.page.min(self.pages.len().saturating_sub(1));
        self.state = state;
        ui.add_page(&self.name, self.layout())?;
        self.refresh(ui);
        Ok(())
    }

    /// The layout for the effect shown.
    pub fn layout(&self) -> &'static str {
        match (
            has_visuals(self.state.info.id),
            self.state.presets.is_empty(),
        ) {
            (true, false) => LAYOUT,
            (true, true) => WIDE_LAYOUT,
            (false, _) => PLAIN_LAYOUT,
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

    /// What Q-Links 1 to 4 turn now, as parameter ids.
    pub fn qlink_ids(&self) -> Vec<u16> {
        self.qtargets()
            .iter()
            .map(|&i| self.state.params[i].id)
            .collect()
    }

    fn qtargets(&self) -> Vec<usize> {
        match self.pages.get(self.page) {
            Some(p) => qlinks(self.state.info, p, self.state.params, &self.state.values),
            None => Vec::new(),
        }
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
        self.refresh(ui);
    }

    /// SHIFT + wheel: the next or previous page.
    pub fn step_page(&mut self, ui: &mut Ui, delta: i32) {
        let n = self.pages.len() as i32;
        if n > 0 {
            self.show_page(ui, (self.page as i32 + delta).rem_euclid(n) as usize);
        }
    }

    /// A Q-Link (0 to 3) turned by `ticks` detents.
    pub fn qlink(&mut self, ui: &mut Ui, n: usize, ticks: i32) -> Option<FxAction> {
        let i = *self.qtargets().get(n)?;
        self.turn(ui, i, ticks)
    }

    /// The data wheel turned by `ticks`: turns the selected tile, or
    /// Q-Link 1's when none is selected.
    pub fn wheel(&mut self, ui: &mut Ui, ticks: i32) -> Option<FxAction> {
        let i = match self.focus {
            Some(i) => i,
            None => *self.qtargets().first()?,
        };
        self.turn(ui, i, ticks)
    }

    /// Lock mode on (a step held, with its locks on this effect) or off
    /// (`None`). See [`crate::lock`].
    pub fn set_locks(&mut self, ui: &mut Ui, locks: Option<Locks>) {
        self.locks = locks;
        self.refresh(ui);
    }

    /// The held step and its locks, in lock mode.
    pub fn locks(&self) -> Option<&Locks> {
        self.locks.as_ref()
    }

    /// Lock mode: takes the selected tile's lock off the held step.
    pub fn unlock(&mut self, ui: &mut Ui) -> Option<FxAction> {
        let id = self.state.params[self.focus?].id;
        if !self.locks.as_mut()?.take(id) {
            return None;
        }
        self.refresh(ui);
        Some(FxAction::Unlock { id })
    }

    /// Lock mode: takes every lock the held step has on the effect off.
    pub fn unlock_all(&mut self, ui: &mut Ui) -> Vec<FxAction> {
        let Some(l) = self.locks.as_mut() else {
            return Vec::new();
        };
        let out = l
            .values
            .drain(..)
            .map(|(id, _)| FxAction::Unlock { id })
            .collect();
        self.refresh(ui);
        out
    }

    /// Parameter `i`'s value as the page shows it: the held step's lock in
    /// lock mode, else the effect's own.
    fn shown(&self, i: usize) -> f32 {
        let id = self.state.params[i].id;
        self.locks
            .as_ref()
            .and_then(|l| l.get(id))
            .unwrap_or(self.state.values[i])
    }

    /// Turns parameter `i` by `ticks` detents: one step for whole numbers
    /// and choices, 1/[`DETENTS`] of the range along its curve otherwise.
    fn turn(&mut self, ui: &mut Ui, i: usize, ticks: i32) -> Option<FxAction> {
        let p = &self.state.params[i];
        let v = self.shown(i);
        let new = match p.curve {
            Curve::Stepped | Curve::Choice(_) | Curve::Toggle => v + ticks as f32,
            _ => p.from_unit(p.to_unit(v) + ticks as f32 / DETENTS),
        };
        self.focus = Some(i);
        self.set(ui, i, new)
    }

    fn set(&mut self, ui: &mut Ui, i: usize, v: f32) -> Option<FxAction> {
        if let Some(l) = self.locks.as_mut() {
            let p = self.state.params[i];
            let v = p.clamp(v);
            if p.readonly || l.get(p.id) == Some(v) {
                return None;
            }
            l.put(p.id, v);
            self.refresh(ui);
            return Some(FxAction::Lock { id: p.id, value: v });
        }
        let set = self.state.set(i, v);
        if set.is_some() {
            self.scratch.sync(&self.state);
        }
        self.refresh(ui);
        set.map(|value| FxAction::Set {
            id: self.state.params[i].id,
            value,
        })
    }

    /// Meters the node reports (`(id, value)`; other ids are ignored). The
    /// app calls it a few times a second while the page shows.
    pub fn set_meters(&mut self, ui: &mut Ui, values: &[(u16, f32)]) {
        let mut changed = false;
        for &(id, v) in values {
            if let Some(i) = self.state.index(id) {
                if self.state.params[i].readonly && self.state.values[i] != v {
                    self.state.values[i] = v;
                    changed = true;
                }
            }
        }
        if changed {
            self.refresh(ui);
        }
    }

    /// A touch on one of the panel's widgets; None for events that only
    /// move the view (a page tab, a selection) or are not the panel's.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent) -> Option<FxAction> {
        let tile_param =
            |s: &FxPanel, k: usize| s.pages.get(s.page).and_then(|p| p.params.get(k)).copied();
        match (ev.widget.as_str(), &ev.kind) {
            (TITLE, EventKind::Tab(i)) => {
                self.show_page(ui, *i);
                None
            }
            (VISUALS, EventKind::Tab(i)) => {
                let want = self.card_pages.get(*i)?.clone();
                let page = self.pages.iter().position(|p| p.name == want)?;
                self.show_page(ui, page);
                None
            }
            (VISUALS, EventKind::Select(k)) => {
                // A box on a card: the EQ's band k, whose gain is selected.
                let want = self.node_pages.get(*k)?.clone();
                let page = self.pages.iter().position(|p| p.name == want)?;
                self.show_page(ui, page);
                if self.state.info.id == "mpfree.fx.eq" {
                    self.focus = self.state.index(mpfree_fx::ids::eq::gain(*k));
                    self.refresh(ui);
                }
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
                let v = self.shown(i) + *delta as f32;
                self.set(ui, i, v)
            }
            (QLINKS, EventKind::Select(n)) => {
                self.focus = self.qtargets().get(*n).copied();
                self.refresh(ui);
                None
            }
            (PRESETS, EventKind::Select(i))
                if *i < self.state.presets.len() && self.locks.is_none() =>
            {
                for (id, v) in preset_values(&self.state, *i) {
                    if let Some(k) = self.state.index(id) {
                        self.state.values[k] = v;
                    }
                }
                self.state.preset = Some(*i);
                self.state.edited = false;
                self.scratch.sync(&self.state);
                self.refresh(ui);
                Some(FxAction::Preset(*i))
            }
            _ => None,
        }
    }

    /// Puts the state into the widgets.
    pub fn refresh(&mut self, ui: &mut Ui) {
        // In lock mode the page shows what the held step plays.
        let over = self.locks.as_ref().map(|l| l.over(&self.state));
        self.scratch.sync(over.as_ref().unwrap_or(&self.state));
        let st = over.as_ref().unwrap_or(&self.state);
        let locks = self.locks.as_ref();
        let page = self.pages.get(self.page).cloned().unwrap_or(Page {
            name: String::new(),
            params: Vec::new(),
            qlinks: Vec::new(),
        });
        let qtargets = self.qtargets();
        if let Some(w) = ui.get_mut::<TitleBar>(TITLE) {
            w.swatch = st.color;
            w.title = match st.preset.and_then(|i| st.presets.get(i)) {
                Some(p) => format!("{} · {}", st.info.name, p),
                None => st.info.name.to_string(),
            };
            w.subtitle = match locks {
                Some(l) => format!("Locking {}", l.line()),
                None => subtitle(st),
            };
            // A one-page effect needs no tab.
            w.tabs = if self.pages.len() > 1 {
                self.pages.iter().map(|p| p.name.clone()).collect()
            } else {
                Vec::new()
            };
            w.active = self.page;
        }
        if has_visuals(st.info.id) {
            let theme = ui.theme.clone();
            let strip = visual::strip(st, &page.name, self.focus, &self.scratch, &theme);
            self.card_pages = strip.pages;
            self.node_pages = strip.node_pages;
            if let Some(w) = ui.get_mut::<Visuals>(VISUALS) {
                w.cards = strip.cards;
            }
        }
        let theme = ui.theme.clone();
        if let Some(w) = ui.get_mut::<TileGrid>(TILES) {
            w.title = page.name.clone();
            w.caption = match locks {
                Some(l) => format!("{}: a turn locks it · amber tiles are locked", l.held),
                None if qtargets.is_empty() => String::new(),
                None => "Q-Links turn the marked tiles · drag to turn".to_string(),
            };
            w.tiles = page
                .params
                .iter()
                .map(|&i| {
                    let mut t = tile(st, &page, i, qtargets.iter().position(|&q| q == i), &theme);
                    if locks.is_some_and(|l| l.get(st.params[i].id).is_some()) {
                        t.color = theme.warn;
                    }
                    t
                })
                .collect();
            w.selected = self
                .focus
                .and_then(|f| page.params.iter().position(|&i| i == f));
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
                    w.wheel_label = format!("Wheel · {}", label(st, &page, i));
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

fn param_line(st: &FxState, i: usize) -> String {
    let (n, u) = value_text(&st.params[i], st.values[i]);
    if u.is_empty() {
        n
    } else {
        format!("{n} {u}")
    }
}

/// The line under the effect's name: where it sits, and what kind it is.
fn subtitle(st: &FxState) -> String {
    let mut parts = Vec::new();
    if !st.place.is_empty() {
        parts.push(st.place.clone());
    }
    parts.push(
        if st.info.id.starts_with("mpfree.") {
            "Native effect"
        } else {
            "Plugin"
        }
        .to_string(),
    );
    parts.join(" · ")
}

/// A tile's label: the parameter's name, with its group when another
/// parameter on the page has the same name ("Band 2 · Gain"), or when the
/// page is built from groups (a plugin's).
fn label(st: &FxState, page: &Page, i: usize) -> String {
    let p = &st.params[i];
    let twin = page
        .params
        .iter()
        .any(|&k| k != i && st.params[k].name == p.name);
    if twin || !crate::pages::named(st.info) {
        tile_label(&page.name, p)
    } else {
        p.name.to_string()
    }
}

/// Parameter `i` as a tile.
fn tile(st: &FxState, page: &Page, i: usize, qlink: Option<usize>, t: &mpfree_ui::Theme) -> Tile {
    let p = &st.params[i];
    let v = st.values[i];
    let (value, unit) = value_text(p, v);
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
        (None, TileKind::Knob { .. }) => t.text_dim,
        (None, _) => t.accent,
    };
    Tile {
        label: label(st, page, i),
        value,
        unit: unit.to_string(),
        kind,
        qlink,
        color,
    }
}
