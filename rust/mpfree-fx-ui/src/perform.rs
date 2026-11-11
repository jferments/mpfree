//! The Perform FX page: sixteen pads of punch-in effects with their
//! lights, the XY pad (the DJ filter across, the echo up), and the
//! selected pad's settings.
//!
//! - A finger on a screen pad plays its effect while held (it starts on
//!   the next grid line, as the hardware pads do) and selects the pad, so
//!   its settings show in the tiles under the pads.
//! - Each pad lights as the node reports its State: dark while off, ringed
//!   amber while it waits for the grid, lit while on, half lit while it
//!   fades out. The app passes the states in with
//!   [`PerformPanel::set_meters`] a few times a second.
//! - The XY pad plays the DJ filter and the echo together. When the finger
//!   lifts it glides back to the middle and the bottom (filter off, no
//!   echo) unless Hold is on; [`PerformPanel::tick`] moves the glide on and
//!   gives the values to send. A trail shows where the finger went.
//! - Q-Link 1 is the DJ knob (the DJ filter, which stops in the middle on
//!   the way through, so off is easy to find), 2 the echo, 3 the echo's
//!   time, 4 the selected pad's Amount.
//!
//! Everything reaches the node as parameter events and notes: a pad is a
//! NoteOn and NoteOff of its number ([`PerformAction::Press`],
//! [`PerformAction::Release`]); the rest is [`PerformAction::Set`].

use crate::state::FxState;
use mpfree_fx::ids::perform as pf;
use mpfree_fx::{perform_state, XyPad, PERFORM_AMOUNTS, PERFORM_EFFECTS, PERFORM_SLOTS};
use mpfree_node::Curve;
use mpfree_synth_ui::draw::Mask;
use mpfree_synth_ui::format::value_text;
use mpfree_synth_ui::widgets::{
    QCell, QLinkStrip, Tile, TileGrid, TileKind, TitleBar, QLINK_COLORS,
};
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, EventKind, PaintCx, Rect, Touch, Ui, UiEvent, Widget};

pub const TITLE: &str = "perf-title";
pub const PADS: &str = "perf-pads";
pub const XY: &str = "perf-xy";
pub const TILES: &str = "perf-tiles";
pub const QLINKS: &str = "perf-qlinks";

/// The pads beside the XY pad, the selected pad's settings under them.
pub const LAYOUT: &str = "col(header:60, perf-title:64, row(perf-pads:*, perf-xy:520):*, perf-tiles:204, perf-qlinks:78, hint:26)";

/// How many points the XY pad's trail keeps.
pub const TRAIL: usize = 28;

// ---------------------------------------------------------------- pads

/// What one pad shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadView {
    /// The effect ("Stutter"), or empty for Off.
    pub effect: String,
    /// Its length and amount ("1/16 · 100 %").
    pub detail: String,
    /// 0 off, 1 waiting, 2 on, 3 fading (`mpfree_fx::perform_state`).
    pub state: u8,
    /// A finger is on it on the screen.
    pub held: bool,
}

/// The sixteen pads, pad 1 bottom left as on the MPC. A finger down on a
/// pad gives `Pad { index, down: true }`, and its lift `down: false`.
pub struct PerformPads {
    pub pads: Vec<PadView>,
    pub selected: Option<usize>,
    fingers: Vec<(u64, usize)>,
}

impl Default for PerformPads {
    fn default() -> Self {
        PerformPads::new()
    }
}

const PAD_GAP: i32 = 10;

impl PerformPads {
    pub fn new() -> PerformPads {
        PerformPads {
            pads: vec![PadView::default(); PERFORM_SLOTS],
            selected: None,
            fingers: Vec::new(),
        }
    }

    fn body(r: Rect) -> Rect {
        // Inside the panel, under its title line (as `titled_panel` draws).
        let inner = r.inset(14);
        inner.take_top(34).1
    }

    /// Pad `i`'s rectangle (pad 0 bottom left).
    pub fn cell(r: Rect, i: usize) -> Rect {
        let b = PerformPads::body(r);
        let w = (b.w - 3 * PAD_GAP) / 4;
        let h = (b.h - 3 * PAD_GAP) / 4;
        let (col, row) = ((i % 4) as i32, 3 - (i / 4) as i32);
        Rect::new(b.x + col * (w + PAD_GAP), b.y + row * (h + PAD_GAP), w, h)
    }

    fn hit(r: Rect, x: i32, y: i32) -> Option<usize> {
        (0..PERFORM_SLOTS).find(|&i| PerformPads::cell(r, i).contains(x, y))
    }
}

impl Widget for PerformPads {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let on = self.pads.iter().filter(|p| p.state != 0).count();
        let right = if on == 0 {
            "hold a pad to play it".to_string()
        } else {
            format!("{on} playing")
        };
        cx.titled_panel(r, "Perform pads", &right);
        for (i, p) in self.pads.iter().enumerate() {
            let c = PerformPads::cell(r, i);
            let lit = t.accent;
            let (fill, ink, dim) = match p.state {
                2 => (lit, t.bg, t.bg),
                3 => (t.raised.mix(lit, 0.45), t.text, t.text),
                _ if p.effect.is_empty() => {
                    (t.surface.mix(t.raised, 0.5), t.text_faint, t.text_faint)
                }
                _ => (t.raised, t.text, t.text_dim),
            };
            cx.canvas.fill_round_rect(c, 10, fill);
            if p.state == 1 {
                cx.canvas.stroke_round_rect(c, 10, 3, t.warn);
            } else if self.selected == Some(i) {
                cx.canvas.stroke_round_rect(c, 10, 2, t.text);
            } else if p.held {
                cx.canvas.stroke_round_rect(c, 10, 2, lit);
            }
            let inner = c.inset_xy(10, 7);
            let (top, bottom) = inner.take_top(inner.h / 2);
            let name = if p.effect.is_empty() {
                "—"
            } else {
                &p.effect
            };
            cx.text(FontId::Bold, t.size_small, top, Align::Left, ink, name);
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                top,
                Align::Right,
                dim,
                &(i + 1).to_string(),
            );
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                bottom,
                Align::Left,
                dim,
                &p.detail,
            );
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let i = PerformPads::hit(r, x, y)?;
                self.fingers.push((finger, i));
                if let Some(p) = self.pads.get_mut(i) {
                    p.held = true;
                }
                self.selected = Some(i);
                Some(EventKind::Pad {
                    index: i,
                    down: true,
                })
            }
            Touch::Up | Touch::Cancel => {
                let k = self.fingers.iter().position(|f| f.0 == finger)?;
                let (_, i) = self.fingers.remove(k);
                if !self.fingers.iter().any(|f| f.1 == i) {
                    if let Some(p) = self.pads.get_mut(i) {
                        p.held = false;
                    }
                }
                Some(EventKind::Pad {
                    index: i,
                    down: false,
                })
            }
            Touch::Move => None,
        }
    }
}

// ---------------------------------------------------------------- XY pad

/// The XY pad: where it is (from the two parameters' values), where it
/// rests, the trail, and the Hold switch in its corner.
///
/// A finger down or moving on the pad gives `Pad { index: 0, down: true }`
/// with the place in [`XyView::finger`] (0 to 1 each way, y up); its lift
/// gives `down: false`. A tap on Hold gives `Tab(0)`.
pub struct XyView {
    pub title: String,
    /// What the four edges mean: left, right, bottom, top.
    pub edges: [String; 4],
    /// The value line beside the title ("LP 1.2 kHz · echo 40 %").
    pub readout: String,
    /// Where the pad is, 0 to 1 each way (y up).
    pub pos: (f32, f32),
    pub rest: (f32, f32),
    /// The last places it was, oldest first.
    pub trail: Vec<(f32, f32)>,
    pub touched: bool,
    pub hold: bool,
    /// Where the finger is now, for the panel to read after an event.
    pub finger: Option<(f32, f32)>,
    owner: Option<u64>,
}

impl Default for XyView {
    fn default() -> Self {
        XyView::new()
    }
}

impl XyView {
    pub fn new() -> XyView {
        XyView {
            title: "XY".to_string(),
            edges: Default::default(),
            readout: String::new(),
            pos: (0.5, 0.0),
            rest: (0.5, 0.0),
            trail: Vec::new(),
            touched: false,
            hold: false,
            finger: None,
            owner: None,
        }
    }

    /// The playing area.
    pub fn area(r: Rect) -> Rect {
        let inner = r.inset(14);
        inner.take_top(34).1
    }

    /// The Hold switch, in the area's top right corner.
    pub fn hold_rect(r: Rect) -> Rect {
        let a = XyView::area(r);
        Rect::new(a.right() - 84, a.y + 8, 76, 32)
    }

    fn to_screen(a: Rect, p: (f32, f32)) -> (f32, f32) {
        (
            a.x as f32 + p.0 * a.w as f32,
            a.bottom() as f32 - p.1 * a.h as f32,
        )
    }

    fn from_screen(a: Rect, x: i32, y: i32) -> (f32, f32) {
        (
            ((x - a.x) as f32 / a.w.max(1) as f32).clamp(0.0, 1.0),
            ((a.bottom() - y) as f32 / a.h.max(1) as f32).clamp(0.0, 1.0),
        )
    }
}

impl Widget for XyView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.readout);
        let a = XyView::area(r);
        cx.canvas.fill_round_rect(a, 10, t.raised);
        // Quarter lines, and stronger lines through where it rests.
        for k in 1..4 {
            let x = a.x + a.w * k / 4;
            let y = a.y + a.h * k / 4;
            cx.canvas
                .fill_rect(Rect::new(x, a.y + 6, 1, a.h - 12), t.outline);
            cx.canvas
                .fill_rect(Rect::new(a.x + 6, y, a.w - 12, 1), t.outline);
        }
        let (rx, ry) = XyView::to_screen(a, self.rest);
        let rest_line = t.text_faint;
        if self.rest.0 > 0.0 && self.rest.0 < 1.0 {
            cx.canvas
                .fill_rect(Rect::new(rx as i32, a.y + 6, 2, a.h - 12), rest_line);
        }
        if self.rest.1 > 0.0 && self.rest.1 < 1.0 {
            cx.canvas
                .fill_rect(Rect::new(a.x + 6, ry as i32, a.w - 12, 2), rest_line);
        }
        // The edges' names.
        let small = t.size_small - 1.0;
        let pad = 10;
        let mid = |h: i32| Rect::new(a.x + pad, a.y + (a.h - h) / 2, a.w - 2 * pad, h);
        cx.text(
            FontId::Bold,
            small,
            mid(20),
            Align::Left,
            t.text_dim,
            &self.edges[0],
        );
        cx.text(
            FontId::Bold,
            small,
            mid(20),
            Align::Right,
            t.text_dim,
            &self.edges[1],
        );
        // Beside the line through where it rests, so neither covers it.
        let lx = (rx as i32 + 12).min(a.right() - 60);
        let bottom = Rect::new(lx, a.bottom() - 26, a.right() - lx, 20);
        cx.text(
            FontId::Bold,
            small,
            bottom,
            Align::Left,
            t.text_dim,
            &self.edges[2],
        );
        let top = Rect::new(lx, a.y + 10, a.right() - lx, 20);
        cx.text(
            FontId::Bold,
            small,
            top,
            Align::Left,
            t.text_dim,
            &self.edges[3],
        );
        // Where it rests: a small ring.
        let mut m = Mask::new(a.inset(-8));
        m.arc(
            rx,
            ry.min(a.bottom() as f32 - 9.0),
            7.0,
            0.0,
            2.0 * std::f32::consts::PI,
            2.0,
        );
        m.draw(cx.canvas, t.text_faint);
        // The trail, fading towards its oldest end.
        let n = self.trail.len();
        for k in 1..n {
            let a0 = XyView::to_screen(a, self.trail[k - 1]);
            let a1 = XyView::to_screen(a, self.trail[k]);
            let alpha = (40 + 180 * k / n.max(1)) as u8;
            let mut m = Mask::new(a);
            m.segment(a0, a1, 2.0 + 4.0 * k as f32 / n as f32);
            m.draw(cx.canvas, t.accent.with_alpha(alpha));
        }
        // The dot: filled under a finger, a ring while it glides or holds.
        let (px, py) = XyView::to_screen(a, self.pos);
        let mut m = Mask::new(a.inset(-20));
        if self.touched {
            m.disc(px, py, 16.0);
            m.draw(cx.canvas, t.accent);
        } else {
            m.arc(px, py, 14.0, 0.0, 2.0 * std::f32::consts::PI, 4.0);
            m.draw(cx.canvas, if self.hold { t.warn } else { t.accent });
        }
        // Hold.
        let h = XyView::hold_rect(r);
        if self.hold {
            cx.canvas.fill_round_rect(h, 9, t.warn);
            cx.text(FontId::Bold, t.size_small, h, Align::Center, t.bg, "Hold");
        } else {
            cx.canvas.fill_round_rect(h, 9, t.surface);
            cx.canvas.stroke_round_rect(h, 9, 1, t.outline);
            cx.text(
                FontId::Sans,
                t.size_small,
                h,
                Align::Center,
                t.text_dim,
                "Hold",
            );
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down if XyView::hold_rect(r).inset(-6).contains(x, y) => Some(EventKind::Tab(0)),
            Touch::Down | Touch::Move => {
                if phase == Touch::Down {
                    if self.owner.is_some() {
                        return None;
                    }
                    self.owner = Some(finger);
                } else if self.owner != Some(finger) {
                    return None;
                }
                self.finger = Some(XyView::from_screen(XyView::area(r), x, y));
                Some(EventKind::Pad {
                    index: 0,
                    down: true,
                })
            }
            Touch::Up | Touch::Cancel => {
                if self.owner != Some(finger) {
                    return None;
                }
                self.owner = None;
                self.finger = None;
                Some(EventKind::Pad {
                    index: 0,
                    down: false,
                })
            }
        }
    }
}

// ---------------------------------------------------------------- panel

/// What the page asks the app to do to the Perform FX node.
#[derive(Clone, Debug, PartialEq)]
pub enum PerformAction {
    /// `Node::set_param(id, value)` (or a parameter event on the audio
    /// thread).
    Set { id: u16, value: f32 },
    /// Pad `k` pressed: `NoteOn { note: k }`.
    Press(usize),
    /// Pad `k` let go: `NoteOff { note: k }`.
    Release(usize),
}

pub struct PerformPanel {
    name: String,
    /// The selected pad.
    selected: usize,
    /// The selected tile, as an index into the parameters.
    focus: Option<usize>,
    xy: XyPad,
    trail: Vec<(f32, f32)>,
    pub state: FxState,
}

/// The tiles under the pads: the selected pad's three settings and State,
/// then the page's own four.
fn tile_ids(pad: usize) -> [u16; 8] {
    let s = pf::slot(pad);
    [
        s + pf::EFFECT,
        s + pf::LENGTH,
        s + pf::AMOUNT,
        pf::QUANTISE,
        pf::FADE,
        pf::ECHO_TIME,
        pf::DJ_FILTER,
        pf::ECHO,
    ]
}

impl PerformPanel {
    /// Adds the page's widgets and its page `name` to `ui` (the app adds
    /// `header` and `hint` first). `state` must be a Perform FX node's.
    pub fn add(ui: &mut Ui, name: &str, state: FxState) -> Result<PerformPanel, String> {
        if state.info.id != "mpfree.fx.perform" {
            return Err(format!("{} is not Perform FX", state.info.id));
        }
        ui.add(TITLE, TitleBar::new());
        ui.add(PADS, PerformPads::new());
        ui.add(XY, XyView::new());
        ui.add(TILES, TileGrid::new(""));
        ui.add(QLINKS, QLinkStrip::new());
        ui.add_page(name, LAYOUT)?;
        let mut p = PerformPanel {
            name: name.to_string(),
            selected: 0,
            focus: None,
            xy: XyPad::perform(),
            trail: Vec::new(),
            state,
        };
        p.refresh(ui);
        Ok(p)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects pad `k` for the tiles (as a touch on it does).
    pub fn select(&mut self, ui: &mut Ui, k: usize) {
        if k < PERFORM_SLOTS {
            self.selected = k;
            self.focus = None;
            self.refresh(ui);
        }
    }

    pub fn xy(&self) -> &XyPad {
        &self.xy
    }

    /// What Q-Links 1 to 4 turn: the DJ filter, the echo, its time, and
    /// the selected pad's Amount.
    pub fn qlink_ids(&self) -> [u16; 4] {
        [
            pf::DJ_FILTER,
            pf::ECHO,
            pf::ECHO_TIME,
            pf::slot(self.selected) + pf::AMOUNT,
        ]
    }

    /// A Q-Link (0 to 3) turned by `ticks` detents. Q-Link 1, the DJ knob,
    /// stops in the middle on its way through.
    pub fn qlink(&mut self, ui: &mut Ui, n: usize, ticks: i32) -> Option<PerformAction> {
        let id = *self.qlink_ids().get(n)?;
        let i = self.state.index(id)?;
        let old = self.state.values[i];
        let p = self.state.params[i];
        let mut new = match p.curve {
            Curve::Stepped | Curve::Choice(_) | Curve::Toggle => old + ticks as f32,
            _ => p.from_unit(p.to_unit(old) + ticks as f32 / crate::view::DETENTS),
        };
        if id == pf::DJ_FILTER && old != 0.0 && (old < 0.0) != (new < 0.0) {
            new = 0.0;
        }
        self.focus = Some(i);
        self.set(ui, i, new)
    }

    /// The data wheel: turns the selected tile, else the DJ filter.
    pub fn wheel(&mut self, ui: &mut Ui, ticks: i32) -> Option<PerformAction> {
        match self.focus {
            Some(i) => {
                let p = self.state.params[i];
                let v = self.state.values[i];
                let new = match p.curve {
                    Curve::Stepped | Curve::Choice(_) | Curve::Toggle => v + ticks as f32,
                    _ => p.from_unit(p.to_unit(v) + ticks as f32 / crate::view::DETENTS),
                };
                self.set(ui, i, new)
            }
            None => self.qlink(ui, 0, ticks),
        }
    }

    fn set(&mut self, ui: &mut Ui, i: usize, v: f32) -> Option<PerformAction> {
        let set = self.state.set(i, v);
        self.refresh(ui);
        set.map(|value| PerformAction::Set {
            id: self.state.params[i].id,
            value,
        })
    }

    /// The pads' states and any other value the node reports, `(id,
    /// value)`; a few times a second while the page shows.
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

    /// Moves the XY pad's glide on by `ms` (call it every frame while the
    /// page shows); gives the values to send while it moves.
    pub fn tick(&mut self, ui: &mut Ui, ms: f32) -> Vec<PerformAction> {
        if !self.xy.tick(ms) {
            if !self.xy.touched() && !self.trail.is_empty() {
                // Let the trail run out once the pad has stopped.
                self.trail.remove(0);
                self.refresh(ui);
            }
            return Vec::new();
        }
        self.follow_xy(ui)
    }

    /// Sends the XY pad's values (those that changed) and draws it.
    fn follow_xy(&mut self, ui: &mut Ui) -> Vec<PerformAction> {
        let mut out = Vec::new();
        for (id, v) in self.xy.values() {
            if let Some(i) = self.state.index(id) {
                if let Some(value) = self.state.set(i, v) {
                    out.push(PerformAction::Set { id, value });
                }
            }
        }
        self.trail.push(self.xy.position());
        if self.trail.len() > TRAIL {
            self.trail.remove(0);
        }
        self.refresh(ui);
        out
    }

    /// A touch on one of the page's widgets.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent) -> Vec<PerformAction> {
        let tiles = tile_ids(self.selected);
        let tile_param =
            |s: &PerformPanel, k: usize| tiles.get(k).and_then(|&id| s.state.index(id));
        match (ev.widget.as_str(), &ev.kind) {
            (PADS, EventKind::Pad { index, down: true }) => {
                self.selected = *index;
                self.focus = None;
                self.refresh(ui);
                vec![PerformAction::Press(*index)]
            }
            (PADS, EventKind::Pad { index, down: false }) => {
                self.refresh(ui);
                vec![PerformAction::Release(*index)]
            }
            (XY, EventKind::Pad { down: true, .. }) => {
                let Some((x, y)) = ui.get::<XyView>(XY).and_then(|w| w.finger) else {
                    return Vec::new();
                };
                self.xy.touch(x, y);
                self.follow_xy(ui)
            }
            (XY, EventKind::Pad { down: false, .. }) => {
                self.xy.release();
                self.refresh(ui);
                Vec::new()
            }
            (XY, EventKind::Tab(0)) => {
                let on = !self.xy.hold;
                self.xy.set_hold(on);
                self.refresh(ui);
                Vec::new()
            }
            (TILES, EventKind::Select(k)) => {
                self.focus = tile_param(self, *k);
                self.refresh(ui);
                Vec::new()
            }
            (TILES, EventKind::Value(pos)) => {
                let Some(i) = self.focus else {
                    return Vec::new();
                };
                let p = self.state.params[i];
                self.set(ui, i, p.from_unit(*pos)).into_iter().collect()
            }
            (TILES, EventKind::Step { index, delta }) => {
                let Some(i) = tile_param(self, *index) else {
                    return Vec::new();
                };
                self.focus = Some(i);
                let v = self.state.values[i] + *delta as f32;
                self.set(ui, i, v).into_iter().collect()
            }
            (QLINKS, EventKind::Select(n)) => {
                let id = self.qlink_ids()[(*n).min(3)];
                self.focus = self.state.index(id);
                self.refresh(ui);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The text for parameter `id`'s value.
    fn line(&self, id: u16) -> String {
        let Some(i) = self.state.index(id) else {
            return String::new();
        };
        let (n, u) = value_text(&self.state.params[i], self.state.values[i]);
        if u.is_empty() {
            n
        } else {
            format!("{n} {u}")
        }
    }

    /// Puts the state into the widgets.
    pub fn refresh(&mut self, ui: &mut Ui) {
        let st = &self.state;
        let v = |id: u16| st.value(id);
        let playing: Vec<String> = (0..PERFORM_SLOTS)
            .filter(|&k| v(pf::slot(k) + pf::STATE) == perform_state::ON)
            .map(|k| effect_name(v(pf::slot(k) + pf::EFFECT)))
            .collect();
        if let Some(w) = ui.get_mut::<TitleBar>(TITLE) {
            w.swatch = st.color;
            w.title = st.info.name.to_string();
            let mut sub = vec![
                st.place.clone(),
                format!("punch in on {}", self.line(pf::QUANTISE)),
            ];
            sub.retain(|s| !s.is_empty());
            if !playing.is_empty() {
                sub.push(format!("playing: {}", playing.join(", ")));
            }
            w.subtitle = sub.join(" · ");
            w.tabs = Vec::new();
        }
        if let Some(w) = ui.get_mut::<PerformPads>(PADS) {
            for (k, p) in w.pads.iter_mut().enumerate() {
                let s = pf::slot(k);
                let fx = v(s + pf::EFFECT);
                p.effect = if fx < 1.0 {
                    String::new()
                } else {
                    effect_name(fx)
                };
                p.detail = if fx < 1.0 {
                    String::new()
                } else {
                    format!(
                        "{} · {}",
                        text_of(st, s + pf::LENGTH),
                        text_of(st, s + pf::AMOUNT)
                    )
                };
                p.state = v(s + pf::STATE).round().clamp(0.0, 3.0) as u8;
            }
            w.selected = Some(self.selected);
        }
        let dj = v(pf::DJ_FILTER);
        let echo = v(pf::ECHO);
        let (xa, ya) = (self.xy.x, self.xy.y);
        let readout = {
            let filter = if dj == 0.0 {
                "filter off".to_string()
            } else if dj < 0.0 {
                format!("LP {:.0} %", -dj)
            } else {
                format!("HP {dj:.0} %")
            };
            format!("{filter} · echo {echo:.0} % · {}", self.line(pf::ECHO_TIME))
        };
        let pos = (xa.position(dj), ya.position(echo));
        let (hold, touched, trail) = (self.xy.hold, self.xy.touched(), self.trail.clone());
        if let Some(w) = ui.get_mut::<XyView>(XY) {
            w.title = "XY · DJ filter and echo".to_string();
            w.edges = [
                "LP".to_string(),
                "HP".to_string(),
                "dry".to_string(),
                "echo".to_string(),
            ];
            w.readout = readout;
            w.pos = pos;
            w.rest = (xa.rest, ya.rest);
            w.trail = trail;
            w.touched = touched;
            w.hold = hold;
        }
        let ids = tile_ids(self.selected);
        let qids = self.qlink_ids();
        let theme = ui.theme.clone();
        let fx_now = v(pf::slot(self.selected) + pf::EFFECT) as usize;
        if let Some(w) = ui.get_mut::<TileGrid>(TILES) {
            w.title = format!("Pad {}", self.selected + 1);
            w.caption = match PERFORM_AMOUNTS.get(fx_now) {
                Some(m) if !m.is_empty() => format!("Amount: {m}"),
                _ => "Q-Links: DJ filter, echo, echo time, amount".to_string(),
            };
            w.tiles = ids
                .iter()
                .filter_map(|&id| st.index(id))
                .map(|i| {
                    let p = &st.params[i];
                    let q = qids.iter().position(|&q| q == p.id);
                    let (value, unit) = value_text(p, st.values[i]);
                    let kind = match p.curve {
                        Curve::Choice(names) => TileKind::Choice {
                            names: names.iter().map(|s| s.to_string()).collect(),
                            index: st.values[i] as usize,
                        },
                        Curve::Toggle => TileKind::Toggle(st.values[i] >= 0.5),
                        _ => TileKind::Knob {
                            pos: p.to_unit(st.values[i]),
                            bipolar: p.min < 0.0 && p.max > 0.0,
                        },
                    };
                    Tile {
                        label: p.name.to_string(),
                        value,
                        unit: unit.to_string(),
                        color: match (q, &kind) {
                            (Some(q), _) => QLINK_COLORS[q],
                            (None, TileKind::Knob { .. }) => theme.text_dim,
                            (None, _) => theme.accent,
                        },
                        // A choice's arrows sit where the badge would.
                        qlink: q.filter(|_| !matches!(kind, TileKind::Choice { .. })),
                        kind,
                    }
                })
                .collect();
            w.selected = self
                .focus
                .and_then(|f| ids.iter().position(|&id| st.index(id) == Some(f)));
        }
        let cells: Vec<QCell> = qids
            .iter()
            .filter_map(|&id| st.index(id))
            .map(|i| {
                let p = &st.params[i];
                let label = match p.id {
                    pf::DJ_FILTER => "DJ filter".to_string(),
                    id if id == pf::slot(self.selected) + pf::AMOUNT => "Amount".to_string(),
                    _ => p.name.to_string(),
                };
                QCell {
                    label,
                    value: self.line(p.id),
                    pos: p.to_unit(st.values[i]),
                    bipolar: p.min < 0.0 && p.max > 0.0,
                }
            })
            .collect();
        let wheel = match self.focus {
            Some(i) => (
                format!("Wheel · {}", st.params[i].name),
                self.line(st.params[i].id),
            ),
            None => ("Wheel · DJ filter".to_string(), self.line(pf::DJ_FILTER)),
        };
        if let Some(w) = ui.get_mut::<QLinkStrip>(QLINKS) {
            w.cells = cells;
            (w.wheel_label, w.wheel_value) = wheel;
        }
    }
}

fn effect_name(v: f32) -> String {
    PERFORM_EFFECTS
        .get(v as usize)
        .copied()
        .unwrap_or("?")
        .to_string()
}

fn text_of(st: &FxState, id: u16) -> String {
    let Some(i) = st.index(id) else {
        return String::new();
    };
    let (n, u) = value_text(&st.params[i], st.values[i]);
    if u.is_empty() {
        n
    } else {
        format!("{n} {u}")
    }
}
