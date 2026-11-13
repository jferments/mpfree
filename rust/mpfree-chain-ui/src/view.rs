//! The chain editor's widget.
//!
//! Three columns:
//!
//! - **Chain:** the strip's inserts in order, each with its number, name, a
//!   line about its settings, a dot per macro that moves it, and a power
//!   button (bypass with spillover). A tap selects an insert; the buttons
//!   under the list move it up or down or remove it; the last row opens the
//!   effect picker.
//! - **Effect:** the selected insert's page: its name (a tap opens its full
//!   effect page, `mpfree-fx-ui`'s), its parameter groups as tabs, the
//!   effect page's drawings for it (its curve, its delay taps; a tap opens
//!   the page too), and up to eight knobs. A knob shows a ring for how far
//!   modulation moves it and a badge in its macro's colour. Drag a knob up
//!   or down to turn it; touching one makes it the knob the right column
//!   works on. While the picker is open, the column lists the effects the
//!   strip can take, each with a line about it; a tap adds one at the end.
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
use mpfree_synth_ui::widgets::{knob, Card, Visuals};
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use crate::state::{macro_color, picks, ChainState, KNOBS_PER_PAGE};

/// What a touch on the chain editor meant, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum ChainAction {
    /// An insert was tapped: show its page.
    Select(usize),
    /// Bypass an insert (`on` false) or switch it back on.
    SetOn {
        slot: usize,
        on: bool,
    },
    /// Move an insert (`Mixer` `Edit::Move`).
    Move {
        from: usize,
        to: usize,
    },
    /// Take an insert out (`Edit::Remove`).
    Remove(usize),
    /// Add effect `id` (one of [`crate::state::picks`]) at the end of the
    /// chain: `mpfree_fx_ui::make(id)`, then `Edit::Insert`.
    Add(&'static str),
    /// Open insert `slot`'s full effect page (`mpfree_fx_ui::FxPanel`): a
    /// tap on the effect's name or its drawings.
    OpenPage(usize),
    /// Show the selected insert's parameter group `g`.
    Group(usize),
    /// A knob was touched: the modulation column works on it.
    Focus(ParamAddr),
    /// Set a parameter: send it to the mod engine as `ModEvent::hand`
    /// gives (a `Set` unless it has a move lane or its track is
    /// recording, then a `Touch`).
    Set {
        addr: ParamAddr,
        value: f32,
    },
    /// The finger left a knob: send `ModEvent::Untouch` for it (nothing
    /// happens if the turn was a `Set`).
    Release(ParamAddr),
    /// Put a parameter on macro `m`, or take it off (`None`).
    Macro {
        target: ParamAddr,
        macro_: Option<usize>,
    },
    /// Arm learn: the next Q-Link turned takes this parameter.
    Learn(ParamAddr),
    /// Add a matrix row from `source` to the parameter (depth 25 %).
    AddRow {
        target: ParamAddr,
        source: Source,
    },
    /// Set row `row`'s depth (-1 to 1).
    Depth {
        row: usize,
        depth: f32,
    },
    RemoveRow(usize),
    /// Back to the screen the editor took the place of (`ChainState::back`).
    Back,
    /// Show chain `i` of `ChainState::scopes` (the pad's or the track's).
    Scope(usize),
    /// "+ Add effect" opened the effect picker (`true`), or its Cancel
    /// closed it (`false`). The app keeps the picker's state, so it must
    /// hear both, or the next state it sends undoes the tap.
    Picker(bool),
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
/// The effect page's drawings, between the tabs and the knobs.
const PICTURE_H: i32 = 140;
/// The effect picker's tiles.
const PICK_GAP: i32 = 8;

/// Where everything goes, from the widget's rectangle.
pub struct Layout {
    pub back: Option<Rect>,
    /// The selected effect's name: a tap opens its full page.
    pub title: Rect,
    /// The effect page's drawings, when there are any.
    pub picture: Option<Rect>,
    /// The picker's Cancel and its tiles, in [`picks`] order (none when the
    /// picker is shut or the chain is full).
    pub cancel: Option<Rect>,
    pub picks: Vec<Rect>,
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

/// Where everything goes. `picture`: whether there are drawings to show
/// (see [`ChainView::layout`]).
pub fn layout(r: Rect, st: &ChainState, picture: bool) -> Layout {
    let (list, rest) = r.take_left(LIST_W);
    let (side, effect) = rest.take_right(SIDE_W);
    let effect = effect.inset_xy(GAP, 0);
    // Chain column.
    let inner = list.inset(12);
    let (head, body) = inner.take_top(HEAD_H);
    let back = st.back.is_some().then(|| Rect::new(head.x, head.y, 96, 34));
    let sx = back.map_or(head.x, |b| b.right() + 8);
    let n = st.scopes.len() as i32;
    let scopes = (0..n)
        .map(|i| {
            let w = (head.right() - sx) / n.max(1);
            Rect::new(sx + i * w, head.y, w - if i + 1 < n { 4 } else { 0 }, 34)
        })
        .collect();
    let (foot, body) = body.take_bottom(BUTTON_H);
    let mut y = body.y;
    let mut rows = Vec::new();
    for _ in &st.slots {
        rows.push(Rect::new(body.x, y, body.w, ROW_H));
        y += ROW_H + ROW_GAP;
    }
    let add = (st.slots.len() < st.room && y + 44 <= body.bottom())
        .then(|| Rect::new(body.x, y, body.w, 44));
    let bw = (foot.w - 2 * 8) / 3;
    let buttons = [0, 1, 2].map(|i| Rect::new(foot.x + i * (bw + 8), foot.y, bw, BUTTON_H));
    // Effect column: tabs along the top, the drawings, knobs in two rows
    // of four. The picker takes the column's place.
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
    let title = Rect::new(head.x, head.y, x + 6 - head.x - 8, 42);
    let (picture, grid) = if picture && st.selected.is_some() && !st.picking {
        let (p, rest) = grid.take_top(PICTURE_H + 8);
        (Some(Rect::new(p.x, p.y + 8, p.w, PICTURE_H)), rest)
    } else {
        (None, grid)
    };
    let grid = grid.inset_xy(0, 8);
    let (mut cancel, mut pick_rects) = (None, Vec::new());
    if st.picking {
        tabs.clear();
        cancel = Some(Rect::new(head.right() - 104, head.y + 4, 104, 36));
        if st.slots.len() < st.room {
            let n = picks(st.strip).len() as i32;
            let cols = if grid.w >= 420 { 3 } else { 2 };
            let rows_n = (n + cols - 1) / cols;
            let w = (grid.w - (cols - 1) * PICK_GAP) / cols;
            let h = ((grid.h - (rows_n - 1) * PICK_GAP) / rows_n.max(1)).min(120);
            pick_rects = (0..n)
                .map(|i| {
                    Rect::new(
                        grid.x + (i % cols) * (w + PICK_GAP),
                        grid.y + (i / cols) * (h + PICK_GAP),
                        w,
                        h,
                    )
                })
                .collect();
        }
    }
    // Four knobs a row where they fit, else three.
    let n = if st.picking {
        0
    } else {
        st.params.len().min(KNOBS_PER_PAGE) as i32
    };
    let cols = if grid.w >= 440 { 4 } else { 3 };
    let rows_n = ((n + cols - 1) / cols).max(2);
    let (cw, ch) = (grid.w / cols, (grid.h / rows_n).min(250));
    let knobs = (0..n)
        .map(|i| Rect::new(grid.x + (i % cols) * cw, grid.y + (i / cols) * ch, cw, ch).inset(4))
        .collect();
    // Modulate column.
    let sinner = side.inset(14);
    let (_, sbody) = sinner.take_top(30 + 26);
    let chip = (sbody.w - 3 * 8) / 4;
    let macros = std::array::from_fn(|m| {
        Rect::new(
            sbody.x + (m as i32 % 4) * (chip + 8),
            sbody.y + 22 + (m as i32 / 4) * 46,
            chip,
            38,
        )
    });
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
    Layout {
        back,
        title,
        picture,
        cancel,
        picks: pick_rects,
        scopes,
        list,
        effect,
        side,
        rows,
        add,
        buttons,
        tabs,
        knobs,
        macros,
        learn,
        depth_rows,
        add_lfo,
        add_follower,
    }
}

enum Drag {
    Knob { i: usize, y0: i32, pos0: f32 },
    Depth { k: usize },
}

/// See the module docs.
pub struct ChainView {
    state: ChainState,
    /// The selected effect's drawings ([`crate::state::picture`]).
    picture: Visuals,
    actions: Vec<ChainAction>,
    drag: Option<(u64, Drag)>,
}

impl Default for ChainView {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainView {
    pub fn new() -> ChainView {
        ChainView {
            state: ChainState::default(),
            picture: Visuals::new(),
            actions: Vec::new(),
            drag: None,
        }
    }

    pub fn state(&self) -> &ChainState {
        &self.state
    }

    /// Shows a new state. Of the old one it keeps only an open picker
    /// (the editor opened it; a state from the app with `picking` set
    /// opens it too).
    pub fn update(&mut self, st: ChainState) {
        let picking = st.picking || self.state.picking;
        self.state = st;
        self.state.picking = picking;
    }

    /// Shows the selected effect's drawings ([`crate::state::picture`];
    /// empty for none).
    pub fn set_picture(&mut self, cards: Vec<Card>) {
        self.picture.cards = cards;
    }

    /// Where everything goes in `r` as the widget stands.
    pub fn layout(&self, r: Rect) -> Layout {
        layout(r, &self.state, !self.picture.cards.is_empty())
    }

    pub fn take_actions(&mut self) -> Vec<ChainAction> {
        std::mem::take(&mut self.actions)
    }

    fn push(&mut self, a: ChainAction) {
        // Show what can be shown at once.
        match &a {
            ChainAction::Select(i) => self.state.selected = Some(*i),
            ChainAction::SetOn { slot, on } => {
                if let Some(s) = self.state.slots.get_mut(*slot) {
                    s.on = *on;
                }
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
                if let Some(p) = self.state.params.iter_mut().find(|p| p.addr == *target) {
                    p.macro_ = *macro_;
                }
            }
            ChainAction::Learn(_) => self.state.learning = true,
            ChainAction::Depth { row, depth } => {
                if let Some(r) = self.state.rows.iter_mut().find(|r| r.index == *row) {
                    r.depth = *depth;
                }
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
        if self.state.picking {
            if l.cancel.is_some_and(|c| c.contains(x, y)) {
                self.state.picking = false;
                self.push(ChainAction::Picker(false));
                return;
            }
            if let Some(i) = l.picks.iter().position(|r| r.contains(x, y)) {
                if let Some(p) = picks(self.state.strip).get(i) {
                    self.state.picking = false;
                    self.push(ChainAction::Add(p.id));
                }
                return;
            }
            if l.effect.contains(x, y) {
                return;
            }
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
            self.state.picking = true;
            self.push(ChainAction::Picker(true));
            return;
        }
        if let Some(sel) = st.selected {
            let n = st.slots.len();
            if l.buttons[0].contains(x, y) && sel > 0 {
                self.push(ChainAction::Move {
                    from: sel,
                    to: sel - 1,
                });
                self.state.slots.swap(sel, sel - 1);
                self.state.selected = Some(sel - 1);
                return;
            }
            if l.buttons[1].contains(x, y) && sel + 1 < n {
                self.push(ChainAction::Move {
                    from: sel,
                    to: sel + 1,
                });
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
        if let Some(sel) = st.selected.filter(|_| !st.picking) {
            if l.title.contains(x, y) || l.picture.is_some_and(|p| p.contains(x, y)) {
                self.push(ChainAction::OpenPage(sel));
                return;
            }
        }
        let Some(focus) = st.focus else { return };
        if let Some(m) = l.macros.iter().position(|r| r.contains(x, y)) {
            let now = st.focused().and_then(|p| p.macro_);
            self.push(ChainAction::Macro {
                target: focus,
                macro_: if now == Some(m) { None } else { Some(m) },
            });
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
            self.push(ChainAction::AddRow {
                target: focus,
                source: Source::Lfo(0),
            });
        } else if l.add_follower.contains(x, y) && focused_takes_rows(st) {
            self.push(ChainAction::AddRow {
                target: focus,
                source: Source::Follower(0),
            });
        }
    }
}

/// Switches (toggles and choices) take macros but no matrix rows.
fn focused_takes_rows(st: &ChainState) -> bool {
    st.focused().is_some_and(|p| {
        !matches!(
            p.info.curve,
            mpfree_fx::mpfree_node::Curve::Toggle | mpfree_fx::mpfree_node::Curve::Choice(_)
        )
    })
}

fn depth_of(bar: Rect, x: i32) -> f32 {
    let t = (x - bar.x) as f32 / bar.w.max(1) as f32;
    ((t * 2.0 - 1.0).clamp(-1.0, 1.0) * 100.0).round() / 100.0
}

impl Widget for ChainView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let st = &self.state;
        let l = self.layout(r);
        cx.canvas.fill_rect(r, t.bg);
        // ---- Chain column.
        cx.canvas.fill_round_rect(l.list, t.radius, t.surface);
        let inner = l.list.inset(12);
        let (head, _) = inner.take_top(HEAD_H);
        if let (Some(b), Some(to)) = (l.back, &st.back) {
            cx.canvas.fill_round_rect(b, 9, t.raised);
            cx.text(
                FontId::Bold,
                t.size_small,
                b,
                Align::Center,
                t.text,
                &format!("‹  {to}"),
            );
        }
        for (i, (r, name)) in l.scopes.iter().zip(&st.scopes).enumerate() {
            let on = i == st.scope;
            cx.canvas.fill_round_rect(
                *r,
                9,
                if on {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.35)
                },
            );
            if on {
                cx.canvas.fill_round_rect(
                    Rect::new(r.x + 16, r.bottom() - 3, r.w - 32, 2),
                    1,
                    t.accent,
                );
            }
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_small,
                *r,
                Align::Center,
                if on { t.text } else { t.text_dim },
                name,
            );
        }
        if st.scopes.is_empty() {
            let tx = l.back.map_or(head.x, |b| b.right() + 12);
            let title = Rect::new(tx, head.y, head.right() - tx, 34);
            let name = st.title.to_uppercase();
            cx.text(
                FontId::Bold,
                t.size_small,
                title,
                Align::Left,
                t.text_dim,
                &name,
            );
            if let Some(c) = st.cost_pct {
                let cost = format!("{c:.1} %");
                if cx.measure(FontId::Bold, t.size_small, &name)
                    + cx.measure(FontId::Sans, t.size_small, &cost)
                    + 10
                    <= title.w
                {
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        title,
                        Align::Right,
                        t.text_faint,
                        &cost,
                    );
                }
            }
        }
        for (i, (s, row)) in st.slots.iter().zip(&l.rows).enumerate() {
            let sel = st.selected == Some(i);
            cx.canvas.fill_round_rect(
                *row,
                10,
                if sel {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.35)
                },
            );
            if sel {
                cx.canvas
                    .fill_round_rect(Rect::new(row.x, row.y + 8, 3, row.h - 16), 1, t.accent);
            }
            let ink = if s.on { t.text } else { t.text_faint };
            let badge = Rect::new(row.x + 12, row.y + (row.h - 28) / 2, 28, 28);
            cx.canvas.fill_round_rect(
                badge,
                7,
                if s.on {
                    t.accent.mix(t.surface, 0.55)
                } else {
                    t.outline
                },
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                badge,
                Align::Center,
                ink,
                &(i + 1).to_string(),
            );
            let (power, text) = row.take_right(56);
            let text = Rect::new(
                badge.right() + 12,
                row.y,
                text.right() - badge.right() - 12,
                row.h,
            );
            let (top, bottom) = text.take_top(row.h / 2 + 2);
            cx.text(
                FontId::Bold,
                t.size_body,
                top.take_bottom(24).0,
                Align::Left,
                ink,
                &s.name,
            );
            let sub = if s.on {
                s.summary.clone()
            } else {
                "bypassed".to_string()
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                bottom.take_top(22).0,
                Align::Left,
                t.text_dim,
                &sub,
            );
            // A dot per macro that moves it, and a wave mark if the matrix does.
            let mut dx = text.right() - 4;
            for m in s.macros.iter().rev() {
                dx -= 12;
                cx.canvas
                    .fill_round_rect(Rect::new(dx, row.y + 14, 9, 9), 5, macro_color(&t, *m));
            }
            if s.modulated {
                dx -= 22;
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    Rect::new(dx, row.y + 8, 20, 20),
                    Align::Center,
                    t.accent,
                    "~",
                );
            }
            let pill = Rect::new(power.x + 6, power.y + (power.h - 26) / 2, 44, 26);
            cx.canvas.fill_round_rect(
                pill,
                13,
                if s.on {
                    t.good.mix(t.surface, 0.3)
                } else {
                    t.outline
                },
            );
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                pill,
                Align::Center,
                if s.on { t.on_accent } else { t.text_dim },
                if s.on { "ON" } else { "OFF" },
            );
        }
        if let Some(a) = l.add {
            cx.canvas.stroke_round_rect(a, 10, 1, t.outline);
            cx.text(
                FontId::Sans,
                t.size_body,
                a.inset_xy(16, 0),
                Align::Left,
                t.text_dim,
                "+  Add effect",
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                a.inset_xy(14, 0),
                Align::Right,
                t.text_faint,
                &format!("{} of {}", st.slots.len(), st.room),
            );
        }
        let live = st.selected.is_some();
        let sel = st.selected.unwrap_or(0);
        for (b, (label, ok)) in l.buttons.iter().zip([
            ("Up", live && sel > 0),
            ("Down", live && sel + 1 < st.slots.len()),
            ("Remove", live),
        ]) {
            cx.canvas.fill_round_rect(
                *b,
                10,
                if ok {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.3)
                },
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                *b,
                Align::Center,
                if ok { t.text } else { t.text_faint },
                label,
            );
        }
        // ---- Effect column.
        cx.canvas.fill_round_rect(l.effect, t.radius, t.surface);
        let einner = l.effect.inset(14);
        let (head, _) = einner.take_top(52);
        if st.picking {
            paint_picker(cx, &l, st, head);
            paint_side(cx, &l, st);
            return;
        }
        match st.selected.and_then(|s| st.slots.get(s)) {
            None => {
                cx.text(
                    FontId::Sans,
                    t.size_body,
                    l.effect,
                    Align::Center,
                    t.text_dim,
                    "No effects yet: tap  + Add effect",
                );
            }
            Some(s) => {
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    l.title,
                    Align::Left,
                    t.text,
                    &format!("{}  ›", s.name),
                );
                if let Some(pr) = l.picture {
                    self.picture.paint(cx, pr);
                }
                for (g, (tr, name)) in l.tabs.iter().zip(&st.groups).enumerate() {
                    let on = g == st.group;
                    cx.canvas
                        .fill_round_rect(*tr, 9, if on { t.accent } else { t.raised });
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        *tr,
                        Align::Center,
                        if on { t.on_accent } else { t.text_dim },
                        name,
                    );
                }
                for (p, kr) in st.params.iter().zip(&l.knobs) {
                    let focus = st.focus == Some(p.addr);
                    if focus {
                        cx.canvas.fill_round_rect(*kr, 12, t.raised);
                        cx.canvas.stroke_round_rect(*kr, 12, 2, t.accent);
                    }
                    let d = ((kr.w - 24).min(kr.h - 76) as f32).clamp(48.0, 92.0);
                    let (kx, ky) = (
                        kr.x as f32 + kr.w as f32 * 0.5,
                        kr.y as f32 + 14.0 + d * 0.5,
                    );
                    let color = p.macro_.map_or(t.accent, |m| macro_color(&t, m));
                    // The modulation ring: just outside the knob.
                    if let Some((lo, hi)) = p.mod_range {
                        let (a0, a1) = (0.75 * PI + 1.5 * PI * lo, 0.75 * PI + 1.5 * PI * hi);
                        let bbox = Rect::new(
                            (kx - d) as i32,
                            (ky - d) as i32,
                            (2.0 * d) as i32,
                            (2.0 * d) as i32,
                        );
                        let mut m = Mask::new(bbox);
                        m.arc(kx, ky, d * 0.5 + 5.0, a0, a1.max(a0 + 0.02), 3.0);
                        m.draw(cx.canvas, t.good);
                    }
                    let bipolar = p.info.min < 0.0 && p.info.max > 0.0;
                    knob(cx, kx, ky, d, p.pos, bipolar, t.outline, color);
                    let below = Rect::new(kr.x, (ky + d * 0.5) as i32 + 10, kr.w, 24);
                    cx.text(
                        FontId::Bold,
                        t.size_body,
                        below,
                        Align::Center,
                        t.text,
                        &p.text(),
                    );
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        Rect::new(kr.x, below.bottom() + 2, kr.w, 22),
                        Align::Center,
                        t.text_dim,
                        p.info.name,
                    );
                    if let Some(m) = p.macro_ {
                        let b = Rect::new(kr.right() - 40, kr.y + 6, 34, 22);
                        cx.canvas.fill_round_rect(b, 11, macro_color(&t, m));
                        cx.text(
                            FontId::Bold,
                            t.size_small - 2.0,
                            b,
                            Align::Center,
                            t.on_accent,
                            &format!("M{}", m + 1),
                        );
                    }
                    if p.locked {
                        let b = Rect::new(kr.x + 6, kr.y + 6, 34, 22);
                        cx.canvas.fill_round_rect(b, 11, t.warn);
                        cx.text(
                            FontId::Bold,
                            t.size_small - 2.0,
                            b,
                            Align::Center,
                            t.on_accent,
                            "LK",
                        );
                    }
                }
            }
        }
        paint_side(cx, &l, st);
    }

    /// Passes the touch on to the app (an event) whenever it made actions,
    /// as the mixer does: `Ui::touch` drops a widget's touch that answers
    /// None, and the app takes the actions only when it gets the event.
    fn targets(&self, r: Rect) -> Vec<Target> {
        let l = self.layout(r);
        let st = &self.state;
        let mut out = Vec::new();
        if let Some(b) = l.back {
            out.push(Target::tap("back", 0, "‹ Back", b));
        }
        for (i, sr) in l.scopes.iter().enumerate() {
            let label = st.scopes.get(i).cloned().unwrap_or_default();
            out.push(Target::tap("scope", i, label, *sr));
        }
        if st.picking {
            if let Some(c) = l.cancel {
                out.push(Target::tap("cancel", 0, "Cancel", c));
            }
            let names = picks(st.strip);
            for (i, pr) in l.picks.iter().enumerate() {
                let label = names.get(i).map_or("", |p| p.name);
                out.push(Target::tap("pick", i, label, *pr));
            }
        }
        for (i, rr) in l.rows.iter().enumerate() {
            let name = st.slots.get(i).map_or(String::new(), |v| v.name.clone());
            let (power, rest) = (rr.take_right(56).0, rr.take_right(56).1);
            out.push(Target::tap("slot", i, name.clone(), rest));
            out.push(Target::tap("slot-on", i, format!("{name} ON"), power));
        }
        if let Some(a) = l.add {
            out.push(Target::tap("add", 0, "+ Add effect", a));
        }
        if let Some(sel) = st.selected {
            let n = st.slots.len();
            let words = ["Move up", "Move down", "Remove"];
            for (k, b) in l.buttons.iter().enumerate() {
                let off = (k == 0 && sel == 0) || (k == 1 && sel + 1 >= n);
                out.push(
                    Target::tap(["move-up", "move-down", "remove"][k], 0, words[k], *b)
                        .disabled_if(off),
                );
            }
        }
        for (g, tr) in l.tabs.iter().enumerate() {
            let label = st.groups.get(g).copied().unwrap_or("");
            out.push(Target::tap("group", g, label, *tr));
        }
        if st.selected.is_some() && !st.picking {
            out.push(Target::tap("title", 0, "effect page", l.title));
        }
        for (i, kr) in l.knobs.iter().enumerate() {
            let label = st
                .params
                .get(i)
                .map_or(String::new(), |p| p.info.name.to_string());
            out.push(Target::drag("knob", i, label, *kr).towards(0, -40));
        }
        if st.focus.is_some() {
            for (m, mr) in l.macros.iter().enumerate() {
                out.push(Target::tap("macro", m, format!("Macro {}", m + 1), *mr));
            }
            out.push(Target::tap("learn", 0, "Learn", l.learn));
            for (k, (bar, xr)) in l.depth_rows.iter().enumerate() {
                out.push(
                    Target::drag(
                        "depth",
                        k,
                        format!("row {} depth", k + 1),
                        bar.take_left(98).1,
                    )
                    .towards(30, 0),
                );
                out.push(Target::tap(
                    "remove-row",
                    k,
                    format!("row {} ×", k + 1),
                    *xr,
                ));
            }
            let takes = focused_takes_rows(st);
            out.push(Target::tap("add-lfo", 0, "+ LFO", l.add_lfo).disabled_if(!takes));
            out.push(
                Target::tap("add-follower", 0, "+ Follower", l.add_follower).disabled_if(!takes),
            );
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let before = self.actions.len();
        self.touch_inner(finger, phase, x, y, r);
        (self.actions.len() > before).then_some(EventKind::Select(self.actions.len()))
    }
}

impl ChainView {
    fn touch_inner(
        &mut self,
        finger: u64,
        phase: Touch,
        x: i32,
        y: i32,
        r: Rect,
    ) -> Option<EventKind> {
        let l = self.layout(r);
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
                if let Some(k) = l
                    .depth_rows
                    .iter()
                    .position(|(bar, _)| bar.take_left(98).1.contains(x, y))
                {
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
                let Some((f, d)) = &self.drag else {
                    return None;
                };
                if *f != finger {
                    return None;
                }
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
                    if let Some((_, Drag::Knob { i, .. })) = self.drag.take() {
                        if let Some(p) = self.state.params.get(i) {
                            let addr = p.addr;
                            self.push(ChainAction::Release(addr));
                        }
                    }
                }
                None
            }
        }
    }
}

/// The modulation column.
fn paint_side(cx: &mut PaintCx, l: &Layout, st: &ChainState) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(l.side, t.radius, t.surface);
    let sinner = l.side.inset(14);
    let (head, sbody) = sinner.take_top(30);
    let Some(p) = st.focused() else {
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            "MODULATE",
        );
        cx.text(
            FontId::Sans,
            t.size_body,
            sbody.take_top(60).0,
            Align::Left,
            t.text_dim,
            "Touch a knob to modulate it",
        );
        return;
    };
    cx.text(
        FontId::Bold,
        t.size_small,
        head,
        Align::Left,
        t.text_dim,
        &format!("MODULATE · {}", p.info.name.to_uppercase()),
    );
    cx.text(
        FontId::Sans,
        t.size_small,
        sbody.take_top(26).0.take_bottom(20).0,
        Align::Left,
        t.text_faint,
        "MACRO",
    );
    for (m, mr) in l.macros.iter().enumerate() {
        let on = p.macro_ == Some(m);
        let c = macro_color(&t, m);
        cx.canvas
            .fill_round_rect(*mr, 10, if on { c } else { t.raised });
        if !on {
            cx.canvas
                .fill_round_rect(Rect::new(mr.x + 8, mr.y + mr.h / 2 - 4, 8, 8), 4, c);
        }
        cx.text(
            FontId::Bold,
            t.size_body,
            *mr,
            Align::Center,
            if on { t.on_accent } else { t.text },
            &(m + 1).to_string(),
        );
    }
    let named = p
        .macro_
        .and_then(|m| st.macro_names.get(m))
        .map_or("No macro: tap a number".to_string(), |n| format!("On {n}"));
    cx.text(
        FontId::Sans,
        t.size_small,
        Rect::new(sbody.x, l.macros[4].bottom() + 4, sbody.w, 24),
        Align::Left,
        t.text_dim,
        &named,
    );
    cx.canvas
        .fill_round_rect(l.learn, 10, if st.learning { t.accent } else { t.raised });
    let learn = if st.learning {
        // 207 px in Inter Bold at 14 px: fits the 252 px button (M5).
        "Turn a Q-Link or a MIDI control"
    } else {
        "Learn to a Q-Link"
    };
    cx.text(
        FontId::Bold,
        t.size_small,
        l.learn,
        Align::Center,
        if st.learning { t.on_accent } else { t.text },
        learn,
    );
    let mh = Rect::new(sbody.x, l.learn.bottom() + 18, sbody.w, 24);
    cx.text(
        FontId::Sans,
        t.size_small,
        mh,
        Align::Left,
        t.text_faint,
        "MOD MATRIX",
    );
    if !focused_takes_rows(st) {
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(mh.x, mh.bottom() + 4, mh.w, 24),
            Align::Left,
            t.text_dim,
            "A switch: macros only",
        );
        return;
    }
    for (rv, (bar, xr)) in st.rows.iter().zip(&l.depth_rows) {
        let (name, track) = bar.take_left(98);
        cx.text(
            FontId::Sans,
            t.size_small,
            name,
            Align::Left,
            t.text,
            &rv.source,
        );
        let track = track.inset_xy(4, 14);
        cx.canvas.fill_round_rect(track, 4, t.raised);
        let mid = track.x + track.w / 2;
        let end = mid + (rv.depth * track.w as f32 * 0.5) as i32;
        let (a, b) = if end < mid { (end, mid) } else { (mid, end) };
        cx.canvas
            .fill_round_rect(Rect::new(a, track.y, (b - a).max(2), track.h), 4, t.good);
        cx.canvas
            .fill_rect(Rect::new(mid, track.y - 3, 1, track.h + 6), t.text_dim);
        cx.text(
            FontId::Mono,
            t.size_small - 2.0,
            Rect::new(track.x, track.bottom() + 1, track.w, 14),
            Align::Center,
            t.text_dim,
            &format!("{:+.0} %", rv.depth * 100.0),
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            *xr,
            Align::Center,
            t.text_faint,
            "x",
        );
    }
    for (b, label) in [(l.add_lfo, "+ LFO"), (l.add_follower, "+ Follower")] {
        cx.canvas.stroke_round_rect(b, 10, 1, t.outline);
        cx.text(
            FontId::Bold,
            t.size_small,
            b,
            Align::Center,
            t.text_dim,
            label,
        );
    }
}

/// The effect picker, over the effect column.
fn paint_picker(cx: &mut PaintCx, l: &Layout, st: &ChainState, head: Rect) {
    let t = cx.theme.clone();
    cx.text(
        FontId::Bold,
        t.size_small,
        head.take_top(42).0,
        Align::Left,
        t.text_dim,
        &format!("ADD EFFECT · {} OF {} SLOTS USED", st.slots.len(), st.room),
    );
    if let Some(c) = l.cancel {
        cx.canvas.fill_round_rect(c, 9, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            c,
            Align::Center,
            t.text,
            "Cancel",
        );
    }
    if l.picks.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_body,
            l.effect,
            Align::Center,
            t.text_dim,
            "This chain is full: remove an effect first",
        );
        return;
    }
    for (p, r) in picks(st.strip).iter().zip(&l.picks) {
        cx.canvas.fill_round_rect(*r, 10, t.raised);
        let inner = r.inset_xy(12, 10);
        let (name, about) = inner.take_top(26);
        cx.text(FontId::Bold, t.size_body, name, Align::Left, t.text, p.name);
        let size = t.size_small - 1.0;
        for (k, line) in wrap(cx, size, p.about, about.w).iter().take(2).enumerate() {
            cx.text(
                FontId::Sans,
                size,
                Rect::new(about.x, about.y + 4 + k as i32 * 19, about.w, 19),
                Align::Left,
                t.text_dim,
                line,
            );
        }
    }
}

/// `text` in lines no wider than `w` (a word wider than `w` gets a line of
/// its own).
fn wrap(cx: &mut PaintCx, size: f32, text: &str, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if cx.measure(FontId::Sans, size, &format!("{l} {word}")) <= w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}
