//! The XY FX page: one of nine effects played from a pad
//! ([`mpfree_fx::XyFx`], `mpfree.fx.xy`).
//!
//! - The nine effects sit in a grid on the left, each with what its X and
//!   Y play ("Time · Feedback"); a tap picks one, and the effect
//!   crossfades to it.
//! - The pad is the Perform page's [`XyView`]. A finger down sets X and Y
//!   where it is and opens the effect (Active on, over Attack); moving
//!   plays both at once; the lift closes it (Active off, over Release),
//!   so an echo or the wash rings out. With **Hold** on, the lift leaves
//!   it open where the finger was. While no finger is on the pad, Hold is
//!   whether the effect is open: a tap on it opens or closes the effect
//!   in place. A beat division across (the Echo's and the Gate's time)
//!   shows its columns with their names.
//! - The tiles under them: X and Y (named for what they play, showing the
//!   effect's own value: "1/8D", "2.4 kHz"), Mix, Attack, Release, Active
//!   and Effect. Q-Links 1 and 2 are X and Y, 3 Mix, 4 Release; the wheel
//!   turns the selected tile, else picks the effect.
//!
//! Everything reaches the node as parameter changes on the same path as
//! the tiles ([`FxAction::Set`]), so the effect's pages and this one stay
//! in step, and moves record as any knob does.
//!
//! In lock mode (a step held, [`XyFxPanel::set_locks`]) a touch on the pad
//! locks X and Y on the step, and turns lock what they turn; the lift and
//! Hold change nothing. See [`crate::lock`].

use crate::lock::Locks;
use crate::perform::XyView;
use crate::state::FxState;
use crate::view::{FxAction, DETENTS};
use mpfree_fx::ids::xyfx as xy;
use mpfree_fx::xyfx::{axes, PadAxis, EFFECTS as XY_EFFECTS};
use mpfree_node::{Curve, ParamInfo, Unit};
use mpfree_synth_ui::format::value_text;
use mpfree_synth_ui::widgets::{
    QCell, QLinkStrip, Tile, TileGrid, TileKind, TitleBar, QLINK_COLORS,
};
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, EventKind, PaintCx, Rect, Target, Touch, Ui, UiEvent, Widget};

pub const TITLE: &str = "xyfx-title";
pub const EFFECTS: &str = "xyfx-effects";
pub const PAD: &str = "xyfx-pad";
pub const TILES: &str = "xyfx-tiles";
pub const QLINKS: &str = "xyfx-qlinks";

/// The effects beside the pad, the tiles under them.
pub const LAYOUT: &str = "col(header:60, xyfx-title:64, row(xyfx-effects:372, xyfx-pad:*):*, xyfx-tiles:204, xyfx-qlinks:78, hint:26)";

/// How many points the pad's trail keeps.
pub const TRAIL: usize = 28;

/// The tiles, in order.
pub const TILE_IDS: [u16; 7] = [
    xy::X,
    xy::Y,
    xy::MIX,
    xy::ATTACK,
    xy::RELEASE,
    xy::ACTIVE,
    xy::EFFECT,
];

/// What Q-Links 1 to 4 turn.
pub const QLINK_IDS: [u16; 4] = [xy::X, xy::Y, xy::MIX, xy::RELEASE];

// ---------------------------------------------------------------- effects

/// The nine effects, three across, as buttons. A tap on one gives
/// `Select(k)`.
pub struct EffectGrid {
    pub names: Vec<String>,
    /// What X and Y play in each ("Time · Feedback").
    pub details: Vec<String>,
    pub selected: usize,
    /// The selected effect is open (lit) or closed (outlined).
    pub open: bool,
}

impl Default for EffectGrid {
    fn default() -> Self {
        EffectGrid::new()
    }
}

const CELL_GAP: i32 = 8;

impl EffectGrid {
    pub fn new() -> EffectGrid {
        EffectGrid {
            names: Vec::new(),
            details: Vec::new(),
            selected: 0,
            open: false,
        }
    }

    /// Effect `i`'s button: three across, the first top left.
    pub fn cell(r: Rect, i: usize) -> Rect {
        let b = r.inset(14).take_top(34).1;
        let w = (b.w - 2 * CELL_GAP) / 3;
        let h = (b.h - 2 * CELL_GAP) / 3;
        let (col, row) = ((i % 3) as i32, (i / 3) as i32);
        Rect::new(b.x + col * (w + CELL_GAP), b.y + row * (h + CELL_GAP), w, h)
    }

    fn hit(&self, r: Rect, x: i32, y: i32) -> Option<usize> {
        (0..self.names.len()).find(|&i| EffectGrid::cell(r, i).contains(x, y))
    }
}

impl Widget for EffectGrid {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, "Effect", "tap to play another");
        for (i, name) in self.names.iter().enumerate() {
            let c = EffectGrid::cell(r, i);
            let picked = i == self.selected;
            let (fill, ink, dim) = match (picked, self.open) {
                (true, true) => (t.accent, t.bg, t.bg),
                _ => (t.raised, t.text, t.text_dim),
            };
            cx.canvas.fill_round_rect(c, 10, fill);
            if picked && !self.open {
                cx.canvas.stroke_round_rect(c, 10, 2, t.accent);
            }
            let inner = c.inset_xy(10, 6);
            let (top, bottom) = inner.take_top(inner.h / 2);
            cx.text(FontId::Bold, t.size_small, top, Align::Left, ink, name);
            if let Some(d) = self.details.get(i) {
                cx.text(
                    FontId::Sans,
                    t.size_small - 4.0,
                    bottom,
                    Align::Left,
                    dim,
                    d,
                );
            }
        }
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = self.hit(r, x, y)?;
        self.selected = i;
        Some(EventKind::Select(i))
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                Target::tap("effect", i, n.clone(), EffectGrid::cell(r, i))
                    .disabled_if(i == self.selected)
            })
            .collect()
    }
}

// ---------------------------------------------------------------- text

/// A pad direction's value at `pos` as the page shows it: a column's
/// name, the filter's side ("LP 40 %", "off"), or the effect's own value
/// and unit ("2.4", "kHz").
pub fn axis_text(axis: &PadAxis, pos: f32) -> (String, &'static str) {
    if let Some(name) = axis.step_name(pos) {
        return (name.to_string(), "");
    }
    let v = axis.value(pos);
    let (lo, hi) = (axis.value(0.0), axis.value(1.0));
    if lo < 0.0 && hi > 0.0 {
        // The DJ filter: low-pass one way, high-pass the other.
        return match v {
            v if v <= -0.5 => (format!("LP {:.0}", -v), "%"),
            v if v >= 0.5 => (format!("HP {v:.0}"), "%"),
            _ => ("off".to_string(), ""),
        };
    }
    let p = ParamInfo {
        id: 0,
        name: axis.name,
        group: "",
        min: lo.min(hi),
        max: lo.max(hi),
        default: lo,
        unit: axis.unit,
        // A plain number (the bits) reads as a whole one.
        curve: if axis.unit == Unit::None {
            Curve::Stepped
        } else {
            Curve::Linear
        },
        readonly: false,
    };
    value_text(&p, v)
}

fn joined((n, u): (String, &'static str)) -> String {
    if u.is_empty() {
        n
    } else {
        format!("{n} {u}")
    }
}

// ---------------------------------------------------------------- panel

pub struct XyFxPanel {
    name: String,
    /// The selected tile, as an index into the parameters.
    focus: Option<usize>,
    /// Keep the effect open when the finger lifts.
    hold: bool,
    /// A finger is on the pad.
    touched: bool,
    trail: Vec<(f32, f32)>,
    /// Lock mode: the held step and its locks.
    locks: Option<Locks>,
    pub state: FxState,
}

impl XyFxPanel {
    /// Adds the page's widgets and its page `name` to `ui` (the app adds
    /// `header` and `hint` first). `state` must be an XY FX node's.
    pub fn add(ui: &mut Ui, name: &str, state: FxState) -> Result<XyFxPanel, String> {
        if state.info.id != "mpfree.fx.xy" {
            return Err(format!("{} is not XY FX", state.info.id));
        }
        ui.add(TITLE, TitleBar::new());
        ui.add(EFFECTS, EffectGrid::new());
        ui.add(PAD, XyView::new());
        ui.add(TILES, TileGrid::new(""));
        ui.add(QLINKS, QLinkStrip::new());
        ui.add_page(name, LAYOUT)?;
        // An effect that is open as the page opens stays open: Hold is lit.
        let hold = state.value(xy::ACTIVE) >= 0.5;
        let mut p = XyFxPanel {
            name: name.to_string(),
            focus: None,
            hold,
            touched: false,
            trail: Vec::new(),
            locks: None,
            state,
        };
        p.refresh(ui);
        Ok(p)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The effect playing, an index into [`mpfree_fx::xyfx::EFFECTS`].
    pub fn effect(&self) -> usize {
        (self.shown_id(xy::EFFECT) as usize).min(XY_EFFECTS.len() - 1)
    }

    pub fn hold(&self) -> bool {
        self.hold
    }

    pub fn touched(&self) -> bool {
        self.touched
    }

    /// The selected tile's parameter.
    pub fn focus(&self) -> Option<u16> {
        self.focus.map(|i| self.state.params[i].id)
    }

    /// Lock mode on (a step held, with its locks on this XY FX) or off
    /// (`None`). See [`crate::lock`].
    pub fn set_locks(&mut self, ui: &mut Ui, locks: Option<Locks>) {
        self.locks = locks;
        self.refresh(ui);
    }

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

    /// Lock mode: takes every lock the held step has on XY FX off.
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

    fn shown_id(&self, id: u16) -> f32 {
        self.state.index(id).map_or(0.0, |i| self.shown(i))
    }

    fn turned(&self, i: usize, ticks: i32) -> f32 {
        let p = self.state.params[i];
        let v = self.shown(i);
        match p.curve {
            Curve::Stepped | Curve::Choice(_) | Curve::Toggle => v + ticks as f32,
            _ => p.from_unit(p.to_unit(v) + ticks as f32 / DETENTS),
        }
    }

    /// A Q-Link (0 to 3) turned by `ticks` detents: X, Y, Mix, Release.
    pub fn qlink(&mut self, ui: &mut Ui, n: usize, ticks: i32) -> Option<FxAction> {
        let i = self.state.index(*QLINK_IDS.get(n)?)?;
        self.focus = Some(i);
        let v = self.turned(i, ticks);
        self.set(ui, i, v)
    }

    /// The data wheel: turns the selected tile, else picks the effect.
    pub fn wheel(&mut self, ui: &mut Ui, ticks: i32) -> Option<FxAction> {
        let i = match self.focus {
            Some(i) => i,
            None => self.state.index(xy::EFFECT)?,
        };
        let v = self.turned(i, ticks);
        self.set(ui, i, v)
    }

    /// Sets parameter `i` (locks it in lock mode) and draws the page.
    fn set(&mut self, ui: &mut Ui, i: usize, v: f32) -> Option<FxAction> {
        let out = self.put(i, v);
        self.refresh(ui);
        out
    }

    /// Sets parameter `i` without drawing.
    fn put(&mut self, i: usize, v: f32) -> Option<FxAction> {
        let p = self.state.params[i];
        if let Some(l) = self.locks.as_mut() {
            let v = p.clamp(v);
            if p.readonly || l.get(p.id) == Some(v) {
                return None;
            }
            l.put(p.id, v);
            return Some(FxAction::Lock { id: p.id, value: v });
        }
        let value = self.state.set(i, v)?;
        if p.id == xy::ACTIVE && !self.touched {
            // With no finger on the pad, Hold is whether it is open.
            self.hold = value >= 0.5;
        }
        Some(FxAction::Set { id: p.id, value })
    }

    fn put_id(&mut self, id: u16, v: f32) -> Option<FxAction> {
        let i = self.state.index(id)?;
        self.put(i, v)
    }

    /// The trail runs out once the finger has lifted (call it every frame
    /// while the page shows).
    pub fn tick(&mut self, ui: &mut Ui, _ms: f32) {
        if !self.touched && !self.trail.is_empty() {
            self.trail.remove(0);
            self.refresh(ui);
        }
    }

    /// A touch on one of the page's widgets.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent) -> Vec<FxAction> {
        let tile_param =
            |s: &XyFxPanel, k: usize| TILE_IDS.get(k).and_then(|&id| s.state.index(id));
        let mut out = Vec::new();
        match (ev.widget.as_str(), &ev.kind) {
            (PAD, EventKind::Pad { down: true, .. }) => {
                let Some((x, y)) = ui.get::<XyView>(PAD).and_then(|w| w.finger) else {
                    return out;
                };
                let first = !self.touched;
                self.touched = true;
                out.extend(self.put_id(xy::X, x * 100.0));
                out.extend(self.put_id(xy::Y, y * 100.0));
                if first && self.locks.is_none() {
                    out.extend(self.put_id(xy::ACTIVE, 1.0));
                }
                self.trail.push((x, y));
                if self.trail.len() > TRAIL {
                    self.trail.remove(0);
                }
            }
            (PAD, EventKind::Pad { down: false, .. }) => {
                self.touched = false;
                if self.locks.is_none() && !self.hold {
                    out.extend(self.put_id(xy::ACTIVE, 0.0));
                }
            }
            (PAD, EventKind::Tab(0)) => {
                self.hold = !self.hold;
                if self.locks.is_none() && !self.touched {
                    out.extend(self.put_id(xy::ACTIVE, if self.hold { 1.0 } else { 0.0 }));
                }
            }
            (EFFECTS, EventKind::Select(k)) => {
                out.extend(self.put_id(xy::EFFECT, *k as f32));
            }
            (TILES, EventKind::Select(k)) => {
                self.focus = tile_param(self, *k);
            }
            (TILES, EventKind::Value(pos)) => {
                if let Some(i) = self.focus {
                    let p = self.state.params[i];
                    out.extend(self.put(i, p.from_unit(*pos)));
                }
            }
            (TILES, EventKind::Step { index, delta }) => {
                if let Some(i) = tile_param(self, *index) {
                    self.focus = Some(i);
                    let v = self.shown(i) + *delta as f32;
                    out.extend(self.put(i, v));
                }
            }
            (QLINKS, EventKind::Select(n)) => {
                self.focus = self.state.index(QLINK_IDS[(*n).min(3)]);
            }
            _ => return out,
        }
        self.refresh(ui);
        out
    }

    /// The text for parameter `id`'s value as shown (X and Y as what they
    /// play in the effect).
    fn line(&self, st: &FxState, id: u16) -> String {
        let (ax, ay) = axes(self.effect());
        match id {
            xy::X => joined(axis_text(ax, st.value(xy::X) / 100.0)),
            xy::Y => joined(axis_text(ay, st.value(xy::Y) / 100.0)),
            _ => match st.index(id) {
                Some(i) => joined(value_text(&st.params[i], st.values[i])),
                None => String::new(),
            },
        }
    }

    /// Puts the state into the widgets.
    pub fn refresh(&mut self, ui: &mut Ui) {
        // In lock mode the page shows what the held step plays.
        let over = self.locks.as_ref().map(|l| l.over(&self.state));
        let st = over.as_ref().unwrap_or(&self.state);
        let locks = self.locks.as_ref();
        let k = self.effect();
        let (ax, ay) = axes(k);
        let effect = XY_EFFECTS[k];
        let open = st.value(xy::ACTIVE) >= 0.5;
        let pos = (st.value(xy::X) / 100.0, st.value(xy::Y) / 100.0);
        if let Some(w) = ui.get_mut::<TitleBar>(TITLE) {
            w.swatch = st.color;
            w.title = st.info.name.to_string();
            let doing = match (locks, self.touched, open) {
                (Some(l), _, _) => format!("Locking {}", l.line()),
                (None, true, _) => format!("playing {effect}"),
                (None, false, true) => format!("{effect} open (Hold)"),
                (None, false, false) => format!("{effect} · touch the pad to play it"),
            };
            let mut sub = vec![st.place.clone(), doing];
            sub.retain(|s| !s.is_empty());
            w.subtitle = sub.join(" · ");
            w.tabs = Vec::new();
        }
        if let Some(w) = ui.get_mut::<EffectGrid>(EFFECTS) {
            w.names = XY_EFFECTS.iter().map(|s| s.to_string()).collect();
            w.details = (0..XY_EFFECTS.len())
                .map(|e| {
                    let (x, y) = axes(e);
                    format!("{} · {}", x.name, y.name)
                })
                .collect();
            w.selected = k;
            w.open = open;
        }
        let readout = format!(
            "{} {} · {} {}",
            ax.name,
            self.line(st, xy::X),
            ay.name,
            self.line(st, xy::Y)
        );
        let stepped = !ax.steps.is_empty();
        let edges = [
            if stepped {
                String::new()
            } else {
                joined(axis_text(ax, 0.0))
            },
            if stepped {
                String::new()
            } else {
                joined(axis_text(ax, 1.0))
            },
            joined(axis_text(ay, 0.0)),
            joined(axis_text(ay, 1.0)),
        ];
        let (hold, touched, trail) = (self.hold, self.touched, self.trail.clone());
        if let Some(w) = ui.get_mut::<XyView>(PAD) {
            w.title = format!("XY · {effect}");
            w.edges = edges;
            w.readout = readout;
            w.pos = pos;
            w.rest = pos;
            w.show_rest = false;
            w.columns = ax.steps.iter().map(|s| s.to_string()).collect();
            w.trail = trail;
            w.touched = touched;
            w.hold = hold;
        }
        let theme = ui.theme.clone();
        let tiles: Vec<Tile> = TILE_IDS
            .iter()
            .filter_map(|&id| st.index(id))
            .map(|i| {
                let p = &st.params[i];
                let q = QLINK_IDS.iter().position(|&q| q == p.id);
                let (label, (value, unit)) = match p.id {
                    xy::X => (ax.name.to_string(), axis_text(ax, pos.0)),
                    xy::Y => (ay.name.to_string(), axis_text(ay, pos.1)),
                    _ => (p.name.to_string(), value_text(p, st.values[i])),
                };
                let bipolar = p.id == xy::X && ax.value(0.0) < 0.0 && ax.value(1.0) > 0.0;
                let kind = match p.curve {
                    Curve::Choice(names) => TileKind::Choice {
                        names: names.iter().map(|s| s.to_string()).collect(),
                        index: st.values[i] as usize,
                    },
                    Curve::Toggle => TileKind::Toggle(st.values[i] >= 0.5),
                    _ => TileKind::Knob {
                        pos: p.to_unit(st.values[i]),
                        bipolar,
                    },
                };
                let locked = locks.is_some_and(|l| l.get(p.id).is_some());
                Tile {
                    label,
                    value,
                    unit: unit.to_string(),
                    color: match (q, &kind) {
                        _ if locked => theme.warn,
                        (Some(q), _) => QLINK_COLORS[q],
                        (None, TileKind::Knob { .. }) => theme.text_dim,
                        (None, _) => theme.accent,
                    },
                    qlink: q.filter(|_| !matches!(kind, TileKind::Choice { .. })),
                    kind,
                }
            })
            .collect();
        let selected = self
            .focus
            .and_then(|f| TILE_IDS.iter().position(|&id| st.index(id) == Some(f)));
        if let Some(w) = ui.get_mut::<TileGrid>(TILES) {
            w.title = effect.to_string();
            w.caption = match locks {
                Some(l) => format!("{}: a turn locks it · amber tiles are locked", l.held),
                None => "Q-Links: X, Y, mix, release".to_string(),
            };
            w.tiles = tiles;
            w.selected = selected;
        }
        let cells: Vec<QCell> = QLINK_IDS
            .iter()
            .filter_map(|&id| st.index(id))
            .map(|i| {
                let p = &st.params[i];
                let label = match p.id {
                    xy::X => ax.name.to_string(),
                    xy::Y => ay.name.to_string(),
                    _ => p.name.to_string(),
                };
                QCell {
                    label,
                    value: self.line(st, p.id),
                    pos: p.to_unit(st.values[i]),
                    bipolar: p.id == xy::X && ax.value(0.0) < 0.0 && ax.value(1.0) > 0.0,
                }
            })
            .collect();
        let wheel = match self.focus {
            Some(i) => {
                let id = st.params[i].id;
                let name = match id {
                    xy::X => ax.name,
                    xy::Y => ay.name,
                    _ => st.params[i].name,
                };
                (format!("Wheel · {name}"), self.line(st, id))
            }
            None => ("Wheel · Effect".to_string(), effect.to_string()),
        };
        if let Some(w) = ui.get_mut::<QLinkStrip>(QLINKS) {
            w.cells = cells;
            (w.wheel_label, w.wheel_value) = wheel;
        }
    }
}
