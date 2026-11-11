//! The chain editor's widget.
//!
//! Three columns:
//!
//! - **Chain:** the strip's inserts in order, each with its number, name, a
//!   line about its settings, a dot per macro that moves it, and a power
//!   button (bypass with spillover). A tap selects an insert; the buttons
//!   under the list move it up or down or remove it; the last row adds an
//!   effect.
//! - **Effect:** the selected insert's page: its parameter groups as tabs
//!   and up to eight knobs. A knob shows a ring for how far modulation
//!   moves it and a badge in its macro's colour. Drag a knob up or down to
//!   turn it; touching one makes it the knob the right column works on.
//! - **Modulate:** for the touched knob: which macro moves it (tap a
//!   number; tap it again to take it off), Learn (the next Q-Link turned
//!   takes the knob), and its mod matrix rows (drag a row's bar to set its
//!   depth, x to remove it, + LFO or + Follower to add one).
//!
//! The widget keeps a copy of what it shows and moves it under the finger
//! at once; what the touches mean goes into a queue of [`ChainAction`]s
//! for the app (see [`crate::take_chain_actions`]).

use std::f32::consts::PI;

use mpfree_mod::{Source, MACROS};
use mpfree_param::ParamAddr;
use mpfree_synth_ui::draw::Mask;
use mpfree_synth_ui::widgets::knob;
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Touch, Widget};

use crate::state::{macro_color, on_accent, ChainState, KNOBS_PER_PAGE};

/// What a touch on the chain editor meant, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum ChainAction {
    /// An insert was tapped: show its page.
    Select(usize),
    /// Bypass an insert (`on` false) or switch it back on.
    SetOn { slot: usize, on: bool },
    /// Move an insert (`Mixer` `Edit::Move`).
    Move { from: usize, to: usize },
    /// Take an insert out (`Edit::Remove`).
    Remove(usize),
    /// Open the list of effects to add at the end.
    AddEffect,
    /// Show the selected insert's parameter group `g`.
    Group(usize),
    /// A knob was touched: the modulation column works on it.
    Focus(ParamAddr),
    /// Set a parameter: send it to the mod engine as `ModEvent::Set`.
    Set { addr: ParamAddr, value: f32 },
    /// Put a parameter on macro `m`, or take it off (`None`).
    Macro { target: ParamAddr, macro_: Option<usize> },
    /// Arm learn: the next Q-Link turned takes this parameter.
    Learn(ParamAddr),
    /// Add a matrix row from `source` to the parameter (depth 25 %).
    AddRow { target: ParamAddr, source: Source },
    /// Set row `row`'s depth (-1 to 1).
    Depth { row: usize, depth: f32 },
    RemoveRow(usize),
    /// Back to the screen the editor took the place of (`ChainState::back`).
    Back,
    /// Show chain `i` of `ChainState::scopes` (the pad's or the track's).
    Scope(usize),
}

/// The columns' widths (the effect column takes the rest).
pub const LIST_W: i32 = 270;
pub const SIDE_W: i32 = 280;
const GAP: i32 = 10;
const ROW_H: i32 = 58;
const ROW_GAP: i32 = 6;
const HEAD_H: i32 = 40;
const BUTTON_H: i32 = 44;
/// Drag distance (pixels) for a knob's whole travel.
const DRAG_TRAVEL: f32 = 300.0;

/// Where everything goes, from the widget's rectangle.
pub struct Layout {
    pub back: Option<Rect>,
    pub scopes: Vec<Rect>,
    pub list: Rect,
    pub effect: Rect,
    pub side: Rect,
    pub rows: Vec<Rect>,
    pub add: Option<Rect>,
    pub buttons: [Rect; 3],
    pub tabs: Vec<Rect>,
    pub knobs: Vec<Rect>,
    pub macros: [Rect; MACROS],
    pub learn: Rect,
    pub depth_rows: Vec<(Rect, Rect)>,
    pub add_lfo: Rect,
    pub add_follower: Rect,
}

pub fn layout(r: Rect, st: &ChainState) -> Layout {
    let (list, rest) = r.take_left(LIST_W);
    let (side, effect) = rest.take_right(SIDE_W);
    let effect = effect.inset_xy(GAP, 0);
    // Chain column.
    let inner = list.inset(12);
    let (head, body) = inner.take_top(HEAD_H);
    let back = st.back.is_some().then(|| Rect::new(head.x, head.y, 96, 34));
    let sx = back.map_or(head.x, |b| b.right() + 8);
    let n = st.scopes.len() as i32;
    let scopes = (0..n).map(|i| {
        let w = (head.right() - sx) / n.max(1);
        Rect::new(sx + i * w, head.y, w - if i + 1 < n { 4 } else { 0 }, 34)
    }).collect();
    let (foot, body) = body.take_bottom(BUTTON_H);
    let mut y = body.y;
    let mut rows = Vec::new();
    for _ in &st.slots {
        rows.push(Rect::new(body.x, y, body.w, ROW_H));
        y += ROW_H + ROW_GAP;
    }
    let add = (st.slots.len() < st.room && y + 44 <= body.bottom()).then(|| Rect::new(body.x, y, body.w, 44));
    let bw = (foot.w - 2 * 8) / 3;
    let buttons = [0, 1, 2].map(|i| Rect::new(foot.x + i * (bw + 8), foot.y, bw, BUTTON_H));
    // Effect column: tabs along the top, knobs in two rows of four.
    let einner = effect.inset(14);
    let (head, grid) = einner.take_top(52);
    let mut tabs = Vec::new();
    let mut x = head.right();
    for g in st.groups.iter().rev() {
        let w = 24 + 9 * g.len() as i32;
        x -= w;
        tabs.push(Rect::new(x, head.y + 8, w, 34));
        x -= 6;
    }
    tabs.reverse();
    let grid = grid.inset_xy(0, 8);
    // Four knobs a row where they fit, else three.
    let n = st.params.len().min(KNOBS_PER_PAGE) as i32;
    let cols = if grid.w >= 440 { 4 } else { 3 };
    let rows_n = ((n + cols - 1) / cols).max(2);
    let (cw, ch) = (grid.w / cols, (grid.h / rows_n).min(250));
    let knobs = (0..n).map(|i| Rect::new(grid.x + (i % cols) * cw, grid.y + (i / cols) * ch, cw, ch).inset(4)).collect();
    // Modulate column.
    let sinner = side.inset(14);
    let (_, sbody) = sinner.take_top(30 + 26);
    let chip = (sbody.w - 3 * 8) / 4;
    let macros = std::array::from_fn(|m| Rect::new(sbody.x + (m as i32 % 4) * (chip + 8), sbody.y + 22 + (m as i32 / 4) * 46, chip, 38));
    let learn = Rect::new(sbody.x, sbody.y + 22 + 2 * 46 + 30, sbody.w, 44);
    let mut ry = learn.bottom() + 52;
    let mut depth_rows = Vec::new();
    for _ in &st.rows {
        let row = Rect::new(sbody.x, ry, sbody.w, 44);
        let (x, bar) = row.take_right(40);
        depth_rows.push((bar, x));
        ry += 50;
    }
    let half = (sbody.w - 8) / 2;
    let add_lfo = Rect::new(sbody.x, ry + 4, half, 40);
    let add_follower = Rect::new(sbody.x + half + 8, ry + 4, half, 40);
    Layout { back, scopes, list, effect, side, rows, add, buttons, tabs, knobs, macros, learn, depth_rows, add_lfo, add_follower }
}

enum Drag {
    Knob { i: usize, y0: i32, pos0: f32 },
    Depth { k: usize },
}

/// See the module docs.
pub struct ChainView {
    state: ChainState,
    actions: Vec<ChainAction>,
    drag: Option<(u64, Drag)>,
}

impl Default for ChainView {
    fn default() -> Self { Self::new() }
}

impl ChainView {
    pub fn new() -> ChainView { ChainView { state: ChainState::default(), actions: Vec::new(), drag: None } }

    pub fn state(&self) -> &ChainState { &self.state }

    /// Shows a new state (keeps nothing of the old one).
    pub fn update(&mut self, st: ChainState) { self.state = st; }

    pub fn take_actions(&mut self) -> Vec<ChainAction> { std::mem::take(&mut self.actions) }

    fn push(&mut self, a: ChainAction) {
        // Show what can be shown at once.
        match &a {
            ChainAction::Select(i) => self.state.selected = Some(*i),
            ChainAction::SetOn { slot, on } => {
                if let Some(s) = self.state.slots.get_mut(*slot) { s.on = *on; }
            }
            ChainAction::Group(g) => self.state.group = *g,
            ChainAction::Scope(i) => self.state.scope = *i,
            ChainAction::Focus(addr) => self.state.focus = Some(*addr),
            ChainAction::Set { addr, value } => {
                if let Some(p) = self.state.params.iter_mut().find(|p| p.addr == *addr) {
                    p.value = *value;
                    p.pos = p.info.to_unit(*value);
                }
            }
            ChainAction::Macro { target, macro_ } => {
                if let Some(p) = self.state.params.iter_mut().find(|p| p.addr == *target) { p.macro_ = *macro_; }
            }
            ChainAction::Learn(_) => self.state.learning = true,
            ChainAction::Depth { row, depth } => {
                if let Some(r) = self.state.rows.iter_mut().find(|r| r.index == *row) { r.depth = *depth; }
            }
            _ => {}
        }
        self.actions.push(a);
    }

    fn tap(&mut self, l: &Layout, x: i32, y: i32) {
        if l.back.is_some_and(|b| b.contains(x, y)) {
            self.push(ChainAction::Back);
            return;
        }
        if let Some(i) = l.scopes.iter().position(|r| r.contains(x, y)) {
            self.push(ChainAction::Scope(i));
            return;
        }
        let st = &self.state;
        for (i, r) in l.rows.iter().enumerate() {
            if r.contains(x, y) {
                let power = r.take_right(56).0;
                if power.contains(x, y) {
                    let on = !st.slots[i].on;
                    self.push(ChainAction::SetOn { slot: i, on });
                } else {
                    self.push(ChainAction::Select(i));
                }
                return;
            }
        }
        if l.add.is_some_and(|r| r.contains(x, y)) {
            self.push(ChainAction::AddEffect);
            return;
        }
        if let Some(sel) = st.selected {
            let n = st.slots.len();
            if l.buttons[0].contains(x, y) && sel > 0 {
                self.push(ChainAction::Move { from: sel, to: sel - 1 });
                self.state.slots.swap(sel, sel - 1);
                self.state.selected = Some(sel - 1);
                return;
            }
            if l.buttons[1].contains(x, y) && sel + 1 < n {
                self.push(ChainAction::Move { from: sel, to: sel + 1 });
                self.state.slots.swap(sel, sel + 1);
                self.state.selected = Some(sel + 1);
                return;
            }
            if l.buttons[2].contains(x, y) {
                self.push(ChainAction::Remove(sel));
                return;
            }
        }
        if let Some(g) = l.tabs.iter().position(|r| r.contains(x, y)) {
            self.push(ChainAction::Group(g));
            return;
        }
        let Some(focus) = st.focus else { return };
        if let Some(m) = l.macros.iter().position(|r| r.contains(x, y)) {
            let now = st.focused().and_then(|p| p.macro_);
            self.push(ChainAction::Macro { target: focus, macro_: if now == Some(m) { None } else { Some(m) } });
            return;
        }
        if l.learn.contains(x, y) {
            self.push(ChainAction::Learn(focus));
            return;
        }
        for (k, (_, xr)) in l.depth_rows.iter().enumerate() {
            if xr.contains(x, y) {
                let row = self.state.rows[k].index;
                self.push(ChainAction::RemoveRow(row));
                return;
            }
        }
        if l.add_lfo.contains(x, y) && focused_takes_rows(st) {
            self.push(ChainAction::AddRow { target: focus, source: Source::Lfo(0) });
        } else if l.add_follower.contains(x, y) && focused_takes_rows(st) {
            self.push(ChainAction::AddRow { target: focus, source: Source::Follower(0) });
        }
    }
}

/// Switches (toggles and choices) take macros but no matrix rows.
fn focused_takes_rows(st: &ChainState) -> bool {
    st.focused().is_some_and(|p| !matches!(p.info.curve, mpfree_fx::mpfree_node::Curve::Toggle | mpfree_fx::mpfree_node::Curve::Choice(_)))
}

fn depth_of(bar: Rect, x: i32) -> f32 {
    let t = (x - bar.x) as f32 / bar.w.max(1) as f32;
    ((t * 2.0 - 1.0).clamp(-1.0, 1.0) * 100.0).round() / 100.0
}

impl Widget for ChainView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let st = &self.state;
        let l = layout(r, st);
        cx.canvas.fill_rect(r, t.bg);
        // ---- Chain column.
        cx.canvas.fill_round_rect(l.list, t.radius, t.surface);
        let inner = l.list.inset(12);
        let (head, _) = inner.take_top(HEAD_H);
        if let (Some(b), Some(to)) = (l.back, &st.back) {
            cx.canvas.fill_round_rect(b, 9, t.raised);
            cx.text(FontId::Bold, t.size_small, b, Align::Center, t.text, &format!("‹  {to}"));
        }
        for (i, (r, name)) in l.scopes.iter().zip(&st.scopes).enumerate() {
            let on = i == st.scope;
            cx.canvas.fill_round_rect(*r, 9, if on { t.raised } else { t.surface.mix(t.raised, 0.35) });
            if on {
                cx.canvas.fill_round_rect(Rect::new(r.x + 16, r.bottom() - 3, r.w - 32, 2), 1, t.accent);
            }
            cx.text(if on { FontId::Bold } else { FontId::Sans }, t.size_small, *r, Align::Center, if on { t.text } else { t.text_dim }, name);
        }
        if st.scopes.is_empty() {
            let tx = l.back.map_or(head.x, |b| b.right() + 12);
            let title = Rect::new(tx, head.y, head.right() - tx, 34);
            let name = st.title.to_uppercase();
            cx.text(FontId::Bold, t.size_small, title, Align::Left, t.text_dim, &name);
            if let Some(c) = st.cost_pct {
                let cost = format!("{c:.1} %");
                if cx.measure(FontId::Bold, t.size_small, &name) + cx.measure(FontId::Sans, t.size_small, &cost) + 10 <= title.w {
                    cx.text(FontId::Sans, t.size_small, title, Align::Right, t.text_faint, &cost);
                }
            }
        }
        for (i, (s, row)) in st.slots.iter().zip(&l.rows).enumerate() {
            let sel = st.selected == Some(i);
            cx.canvas.fill_round_rect(*row, 10, if sel { t.raised } else { t.surface.mix(t.raised, 0.35) });
            if sel {
                cx.canvas.fill_round_rect(Rect::new(row.x, row.y + 8, 3, row.h - 16), 1, t.accent);
            }
            let ink = if s.on { t.text } else { t.text_faint };
            let badge = Rect::new(row.x + 12, row.y + (row.h - 28) / 2, 28, 28);
            cx.canvas.fill_round_rect(badge, 7, if s.on { t.accent.mix(t.surface, 0.55) } else { t.outline });
            cx.text(FontId::Bold, t.size_small, badge, Align::Center, ink, &(i + 1).to_string());
            let (power, text) = row.take_right(56);
            let text = Rect::new(badge.right() + 12, row.y, text.right() - badge.right() - 12, row.h);
            let (top, bottom) = text.take_top(row.h / 2 + 2);
            cx.text(FontId::Bold, t.size_body, top.take_bottom(24).0, Align::Left, ink, &s.name);
            let sub = if s.on { s.summary.clone() } else { "bypassed".to_string() };
            cx.text(FontId::Sans, t.size_small, bottom.take_top(22).0, Align::Left, t.text_dim, &sub);
            // A dot per macro that moves it, and a wave mark if the matrix does.
            let mut dx = text.right() - 4;
            for m in s.macros.iter().rev() {
                dx -= 12;
                cx.canvas.fill_round_rect(Rect::new(dx, row.y + 14, 9, 9), 5, macro_color(&t, *m));
            }
            if s.modulated {
                dx -= 22;
                cx.text(FontId::Bold, t.size_small - 2.0, Rect::new(dx, row.y + 8, 20, 20), Align::Center, t.accent, "~");
            }
            let pill = Rect::new(power.x + 6, power.y + (power.h - 26) / 2, 44, 26);
            cx.canvas.fill_round_rect(pill, 13, if s.on { t.good.mix(t.surface, 0.3) } else { t.outline });
            cx.text(FontId::Bold, t.size_small - 2.0, pill, Align::Center, if s.on { on_accent(&t) } else { t.text_dim }, if s.on { "ON" } else { "OFF" });
        }
        if let Some(a) = l.add {
            cx.canvas.stroke_round_rect(a, 10, 1, t.outline);
            cx.text(FontId::Sans, t.size_body, a.inset_xy(16, 0), Align::Left, t.text_dim, "+  Add effect");
            cx.text(FontId::Sans, t.size_small, a.inset_xy(14, 0), Align::Right, t.text_faint, &format!("{} of {}", st.slots.len(), st.room));
        }
        let live = st.selected.is_some();
        let sel = st.selected.unwrap_or(0);
        for (b, (label, ok)) in l.buttons.iter().zip([("Up", live && sel > 0), ("Down", live && sel + 1 < st.slots.len()), ("Remove", live)]) {
            cx.canvas.fill_round_rect(*b, 10, if ok { t.raised } else { t.surface.mix(t.raised, 0.3) });
            cx.text(FontId::Bold, t.size_small, *b, Align::Center, if ok { t.text } else { t.text_faint }, label);
        }
        // ---- Effect column.
        cx.canvas.fill_round_rect(l.effect, t.radius, t.surface);
        let einner = l.effect.inset(14);
        let (head, _) = einner.take_top(52);
        match st.selected.and_then(|s| st.slots.get(s)) {
            None => {
                cx.text(FontId::Sans, t.size_body, l.effect, Align::Center, t.text_dim, "No effects yet: tap  + Add effect");
            }
            Some(s) => {
                cx.text(FontId::Bold, t.size_title, head.take_top(42).0, Align::Left, t.text, &s.name);
                for (g, (tr, name)) in l.tabs.iter().zip(&st.groups).enumerate() {
                    let on = g == st.group;
                    cx.canvas.fill_round_rect(*tr, 9, if on { t.accent } else { t.raised });
                    cx.text(FontId::Bold, t.size_small, *tr, Align::Center, if on { on_accent(&t) } else { t.text_dim }, name);
                }
                for (p, kr) in st.params.iter().zip(&l.knobs) {
                    let focus = st.focus == Some(p.addr);
                    if focus {
                        cx.canvas.fill_round_rect(*kr, 12, t.raised);
                        cx.canvas.stroke_round_rect(*kr, 12, 2, t.accent);
                    }
                    let d = ((kr.w - 24).min(kr.h - 76) as f32).clamp(48.0, 92.0);
                    let (kx, ky) = (kr.x as f32 + kr.w as f32 * 0.5, kr.y as f32 + 14.0 + d * 0.5);
                    let color = p.macro_.map_or(t.accent, |m| macro_color(&t, m));
                    // The modulation ring: just outside the knob.
                    if let Some((lo, hi)) = p.mod_range {
                        let (a0, a1) = (0.75 * PI + 1.5 * PI * lo, 0.75 * PI + 1.5 * PI * hi);
                        let bbox = Rect::new((kx - d) as i32, (ky - d) as i32, (2.0 * d) as i32, (2.0 * d) as i32);
                        let mut m = Mask::new(bbox);
                        m.arc(kx, ky, d * 0.5 + 5.0, a0, a1.max(a0 + 0.02), 3.0);
                        m.draw(cx.canvas, t.good);
                    }
                    let bipolar = p.info.min < 0.0 && p.info.max > 0.0;
                    knob(cx, kx, ky, d, p.pos, bipolar, t.outline, color);
                    let below = Rect::new(kr.x, (ky + d * 0.5) as i32 + 10, kr.w, 24);
                    cx.text(FontId::Bold, t.size_body, below, Align::Center, t.text, &p.text());
                    cx.text(FontId::Sans, t.size_small, Rect::new(kr.x, below.bottom() + 2, kr.w, 22), Align::Center, t.text_dim, p.info.name);
                    if let Some(m) = p.macro_ {
                        let b = Rect::new(kr.right() - 40, kr.y + 6, 34, 22);
                        cx.canvas.fill_round_rect(b, 11, macro_color(&t, m));
                        cx.text(FontId::Bold, t.size_small - 2.0, b, Align::Center, on_accent(&t), &format!("M{}", m + 1));
                    }
                    if p.locked {
                        let b = Rect::new(kr.x + 6, kr.y + 6, 34, 22);
                        cx.canvas.fill_round_rect(b, 11, t.warn);
                        cx.text(FontId::Bold, t.size_small - 2.0, b, Align::Center, on_accent(&t), "LK");
                    }
                }
            }
        }
        // ---- Modulate column.
        cx.canvas.fill_round_rect(l.side, t.radius, t.surface);
        let sinner = l.side.inset(14);
        let (head, sbody) = sinner.take_top(30);
        let Some(p) = st.focused() else {
            cx.text(FontId::Bold, t.size_small, head, Align::Left, t.text_dim, "MODULATE");
            cx.text(FontId::Sans, t.size_body, sbody.take_top(60).0, Align::Left, t.text_dim, "Touch a knob to modulate it");
            return;
        };
        cx.text(FontId::Bold, t.size_small, head, Align::Left, t.text_dim, &format!("MODULATE · {}", p.info.name.to_uppercase()));
        cx.text(FontId::Sans, t.size_small, sbody.take_top(26).0.take_bottom(20).0, Align::Left, t.text_faint, "MACRO");
        for (m, mr) in l.macros.iter().enumerate() {
            let on = p.macro_ == Some(m);
            let c = macro_color(&t, m);
            cx.canvas.fill_round_rect(*mr, 10, if on { c } else { t.raised });
            if !on {
                cx.canvas.fill_round_rect(Rect::new(mr.x + 8, mr.y + mr.h / 2 - 4, 8, 8), 4, c);
            }
            cx.text(FontId::Bold, t.size_body, *mr, Align::Center, if on { on_accent(&t) } else { t.text }, &(m + 1).to_string());
        }
        let named = p.macro_.and_then(|m| st.macro_names.get(m)).map_or("No macro: tap a number".to_string(), |n| format!("On {n}"));
        cx.text(FontId::Sans, t.size_small, Rect::new(sbody.x, l.macros[4].bottom() + 4, sbody.w, 24), Align::Left, t.text_dim, &named);
        cx.canvas.fill_round_rect(l.learn, 10, if st.learning { t.accent } else { t.raised });
        let learn = if st.learning { "Turn a Q-Link to take it" } else { "Learn to a Q-Link" };
        cx.text(FontId::Bold, t.size_small, l.learn, Align::Center, if st.learning { on_accent(&t) } else { t.text }, learn);
        let mh = Rect::new(sbody.x, l.learn.bottom() + 18, sbody.w, 24);
        cx.text(FontId::Sans, t.size_small, mh, Align::Left, t.text_faint, "MOD MATRIX");
        if !focused_takes_rows(st) {
            cx.text(FontId::Sans, t.size_small, Rect::new(mh.x, mh.bottom() + 4, mh.w, 24), Align::Left, t.text_dim, "A switch: macros only");
            return;
        }
        for (rv, (bar, xr)) in st.rows.iter().zip(&l.depth_rows) {
            let (name, track) = bar.take_left(98);
            cx.text(FontId::Sans, t.size_small, name, Align::Left, t.text, &rv.source);
            let track = track.inset_xy(4, 14);
            cx.canvas.fill_round_rect(track, 4, t.raised);
            let mid = track.x + track.w / 2;
            let end = mid + (rv.depth * track.w as f32 * 0.5) as i32;
            let (a, b) = if end < mid { (end, mid) } else { (mid, end) };
            cx.canvas.fill_round_rect(Rect::new(a, track.y, (b - a).max(2), track.h), 4, t.good);
            cx.canvas.fill_rect(Rect::new(mid, track.y - 3, 1, track.h + 6), t.text_dim);
            cx.text(FontId::Mono, t.size_small - 2.0, Rect::new(track.x, track.bottom() + 1, track.w, 14), Align::Center, t.text_dim, &format!("{:+.0} %", rv.depth * 100.0));
            cx.text(FontId::Bold, t.size_body, *xr, Align::Center, t.text_faint, "x");
        }
        for (b, label) in [(l.add_lfo, "+ LFO"), (l.add_follower, "+ Follower")] {
            cx.canvas.stroke_round_rect(b, 10, 1, t.outline);
            cx.text(FontId::Bold, t.size_small, b, Align::Center, t.text_dim, label);
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let l = layout(r, &self.state);
        match phase {
            Touch::Down => {
                if let Some(i) = l.knobs.iter().position(|k| k.contains(x, y)) {
                    let p = &self.state.params[i];
                    let (addr, pos0) = (p.addr, p.pos);
                    if self.state.focus != Some(addr) {
                        self.push(ChainAction::Focus(addr));
                    }
                    self.drag = Some((finger, Drag::Knob { i, y0: y, pos0 }));
                    return None;
                }
                if let Some(k) = l.depth_rows.iter().position(|(bar, _)| bar.take_left(98).1.contains(x, y)) {
                    self.drag = Some((finger, Drag::Depth { k }));
                    let (bar, _) = l.depth_rows[k];
                    let depth = depth_of(bar.take_left(98).1.inset_xy(4, 0), x);
                    let row = self.state.rows[k].index;
                    self.push(ChainAction::Depth { row, depth });
                    return None;
                }
                self.tap(&l, x, y);
                None
            }
            Touch::Move => {
                let Some((f, d)) = &self.drag else { return None };
                if *f != finger { return None; }
                match *d {
                    Drag::Knob { i, y0, pos0 } => {
                        let p = self.state.params.get(i)?;
                        let pos = (pos0 + (y0 - y) as f32 / DRAG_TRAVEL).clamp(0.0, 1.0);
                        let value = p.info.from_unit(pos);
                        if value != p.value {
                            let addr = p.addr;
                            self.push(ChainAction::Set { addr, value });
                        }
                    }
                    Drag::Depth { k } => {
                        let (bar, _) = *l.depth_rows.get(k)?;
                        let depth = depth_of(bar.take_left(98).1.inset_xy(4, 0), x);
                        let row = self.state.rows.get(k)?.index;
                        if self.state.rows[k].depth != depth {
                            self.push(ChainAction::Depth { row, depth });
                        }
                    }
                }
                None
            }
            Touch::Up | Touch::Cancel => {
                if self.drag.as_ref().is_some_and(|(f, _)| *f == finger) {
                    self.drag = None;
                }
                None
            }
        }
    }
}
