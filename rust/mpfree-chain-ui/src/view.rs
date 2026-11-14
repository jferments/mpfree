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
use mpfree_param::{ParamAddr, Strip};
use mpfree_synth_ui::draw::Mask;
use mpfree_synth_ui::widgets::{knob, Card, Visuals};
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use crate::picker::{Hit, PickerLayout, PickerView, HOLD};
use crate::state::{macro_color, ChainState, StripList, KNOBS_PER_PAGE, STRIP_GROUPS};

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
    /// Add effect `id` (one of [`ChainState::picks`]) at the end of the
    /// chain: `mpfree_fx_ui::make(id)`, then `Edit::Insert`.
    Add(&'static str),
    /// Open insert `slot`'s full effect page (`mpfree_fx_ui::FxPanel`): a
    /// tap on the effect's name or its drawings.
    OpenPage(usize),
    /// Show the selected insert's parameter group `g`.
    Group(usize),
    /// Switch insert `slot` to mode `mode` of its family (an index into
    /// [`crate::state::ModeView::modes`]): within one engine its fixed
    /// values change, across engines the app swaps the engine in place
    /// (`mpfree_fx::families`).
    SetMode {
        slot: usize,
        mode: usize,
    },
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
    /// The strip button opened the strip list on group `g` (of
    /// [`crate::STRIP_GROUPS`]), a group was picked, or the list closed
    /// (None). The app keeps the list's state, so it must hear each.
    Strips(Option<usize>),
    /// A strip in the list: its chain, in the editor's place (its Back
    /// stays where it was).
    OpenStrip(Strip),
    /// "+ Add effect" opened the effect picker (`true`), or its Cancel
    /// closed it (`false`). The app keeps the picker's state, so it must
    /// hear both, or the next state it sends undoes the tap.
    Picker(bool),
    /// Add effect `id` at the end of the chain with its factory preset
    /// `preset` (`mpfree_fx::picker::presets(id)[preset]`) applied.
    AddPreset {
        id: &'static str,
        preset: usize,
    },
    /// The picker's star: make `id` a favourite (`on`) or not. The app
    /// keeps them ([`ChainState::shelf`], `files/fx/favourites.txt`).
    Favourite {
        id: &'static str,
        on: bool,
    },
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
/// The Mode chooser's row, under the title, for an effect with modes.
const MODE_H: i32 = 40;
/// The Mode chooser's arrows, when its modes don't fit the row as chips.
const MODE_ARROW_W: i32 = 64;

/// Where everything goes, from the widget's rectangle.
pub struct Layout {
    pub back: Option<Rect>,
    /// The strip button (the strip's name and which it is): a tap opens
    /// the strip list. None with scopes, or for a chain no strip names.
    pub strip: Option<Rect>,
    /// The selected effect's name: a tap opens its full page.
    pub title: Rect,
    /// The effect page's drawings, when there are any.
    pub picture: Option<Rect>,
    /// The picker's Cancel, its parts ([`crate::picker`]), and the effect
    /// tiles on show (the current section's page; none when the picker is
    /// shut, shows presets, or the chain is full).
    pub cancel: Option<Rect>,
    pub picker: Option<PickerLayout>,
    pub picks: Vec<Rect>,
    pub scopes: Vec<Rect>,
    pub list: Rect,
    pub effect: Rect,
    pub side: Rect,
    pub rows: Vec<Rect>,
    pub add: Option<Rect>,
    pub buttons: [Rect; 3],
    pub tabs: Vec<Rect>,
    /// The Mode chooser: each place to tap and the mode it picks (a chip
    /// per mode, or the two arrows when they don't fit), and the
    /// arrows' label between them.
    pub modes: Vec<(Rect, usize)>,
    pub mode_label: Option<Rect>,
    pub knobs: Vec<Rect>,
    pub macros: [Rect; MACROS],
    pub learn: Rect,
    pub depth_rows: Vec<(Rect, Rect)>,
    pub add_lfo: Rect,
    pub add_follower: Rect,
}

/// Where everything goes. `picture`: whether there are drawings to show
/// (see [`ChainView::layout`]); `pv`: the picker's own state.
pub fn layout(r: Rect, st: &ChainState, picture: bool, pv: &PickerView) -> Layout {
    let (list, rest) = r.take_left(LIST_W);
    let (side, effect) = rest.take_right(SIDE_W);
    let effect = effect.inset_xy(GAP, 0);
    // Chain column.
    let inner = list.inset(12);
    let (head, body) = inner.take_top(HEAD_H);
    let back = st.back.is_some().then(|| Rect::new(head.x, head.y, 44, 34));
    let sx = back.map_or(head.x, |b| b.right() + 8);
    let strip = (st.scopes.is_empty() && !st.strip_name.is_empty())
        .then(|| Rect::new(sx, head.y, head.right() - sx, 34));
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
    let (mut modes, mut mode_label) = (Vec::new(), None);
    let grid = match &st.mode {
        Some(mv) if st.selected.is_some() && !st.picking => {
            let (row, rest) = grid.take_top(MODE_H + 8);
            let row = Rect::new(row.x, row.y + 8, row.w, MODE_H);
            (modes, mode_label) = mode_row(row, mv);
            rest
        }
        _ => grid,
    };
    let (picture, grid) = if picture && st.selected.is_some() && !st.picking {
        let (p, rest) = grid.take_top(PICTURE_H + 8);
        (Some(Rect::new(p.x, p.y + 8, p.w, PICTURE_H)), rest)
    } else {
        (None, grid)
    };
    let grid = grid.inset_xy(0, 8);
    let (mut cancel, mut picker_l, mut pick_rects) = (None, None, Vec::new());
    if st.picking {
        // The picker takes the effect and Modulate columns.
        tabs.clear();
        let (phead, body) = picker_area(effect, side).inset(14).take_top(52);
        cancel = Some(Rect::new(phead.right() - 104, phead.y + 4, 104, 36));
        if st.slots.len() < st.room {
            let pl = crate::picker::layout(body.inset_xy(0, 4), st, pv);
            if pv.presets.is_none() {
                pick_rects = pl.tiles.clone();
            }
            picker_l = Some(pl);
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
        strip,
        title,
        picture,
        cancel,
        picker: picker_l,
        picks: pick_rects,
        scopes,
        list,
        effect,
        side,
        rows,
        add,
        buttons,
        tabs,
        modes,
        mode_label,
        knobs,
        macros,
        learn,
        depth_rows,
        add_lfo,
        add_follower,
    }
}

/// The Mode chooser in `row`: a chip per mode when they fit (as wide as
/// the group tabs), else an arrow each side of the mode's name, which step
/// to the mode before and after (none past the ends).
fn mode_row(row: Rect, mv: &crate::state::ModeView) -> (Vec<(Rect, usize)>, Option<Rect>) {
    let widths: Vec<i32> = mv.modes.iter().map(|m| 24 + 9 * m.len() as i32).collect();
    let need: i32 = widths.iter().sum::<i32>() + 6 * (widths.len() as i32 - 1).max(0);
    if need <= row.w {
        let mut x = row.x;
        let chips = widths
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                let r = Rect::new(x, row.y, w, row.h);
                x += w + 6;
                (r, i)
            })
            .collect();
        return (chips, None);
    }
    let prev = Rect::new(row.x, row.y, MODE_ARROW_W, row.h);
    let next = Rect::new(row.right() - MODE_ARROW_W, row.y, MODE_ARROW_W, row.h);
    let label = Rect::new(prev.right() + 6, row.y, next.x - prev.right() - 12, row.h);
    let mut out = Vec::new();
    match mv.current {
        Some(c) => {
            if c > 0 {
                out.push((prev, c - 1));
            }
            if c + 1 < mv.modes.len() {
                out.push((next, c + 1));
            }
        }
        // In no mode: either arrow goes to the first.
        None => out.extend([(prev, 0), (next, 0)]),
    }
    (out, Some(label))
}

/// The slot a dragged effect's row lands in with the finger at height
/// `y`: the row under it, or the first or last past the ends.
fn slot_at(l: &Layout, y: i32) -> usize {
    let (Some(first), n) = (l.rows.first(), l.rows.len()) else {
        return 0;
    };
    (((y - first.y) / (ROW_H + ROW_GAP)).max(0) as usize).min(n - 1)
}

/// The effect and Modulate columns together, where the picker goes.
fn picker_area(effect: Rect, side: Rect) -> Rect {
    Rect::new(effect.x, effect.y, side.right() - effect.x, effect.h)
}

enum Drag {
    Knob {
        i: usize,
        y0: i32,
        pos0: f32,
    },
    Depth {
        k: usize,
    },
    /// An effect's row held: once the finger has moved [`SLOT_SLOP`] it
    /// lifts and follows (`y` now), and lands at slot `to` on the lift.
    Slot {
        from: usize,
        y0: i32,
        y: i32,
        to: usize,
        moved: bool,
    },
}

/// Pixels a finger moves on an effect's row before the row lifts to be
/// dragged (less is a tap).
const SLOT_SLOP: i32 = 12;

/// The strip list over the editor: the groups down the left, a page of
/// strip tiles (four a row), its Back, and arrows when a group has more
/// strips than a page.
pub struct ListLayout {
    pub panel: Rect,
    pub groups: [Rect; 6],
    pub title: Rect,
    pub back: Rect,
    pub prev: Rect,
    pub next: Rect,
    pub tiles: Vec<Rect>,
}

/// Tiles a row in the strip list.
const LIST_COLS: i32 = 4;
const TILE_H: i32 = 72;

/// Where the strip list goes in `r`, with room for `per_page` tiles at
/// most (see [`list_page`]).
pub fn list_layout(r: Rect) -> ListLayout {
    let panel = r.inset_xy(0, 4);
    let inner = panel.inset(14);
    let (left, right) = inner.take_left(200);
    let right = right.take_left(14).1;
    let groups =
        std::array::from_fn(|i| Rect::new(left.x, left.y + 36 + i as i32 * 56, left.w, 48));
    let (head, grid) = right.take_top(36);
    let back = Rect::new(head.right() - 96, head.y, 96, 36);
    let next = Rect::new(back.x - 12 - 44, head.y, 44, 36);
    let prev = Rect::new(next.x - 6 - 44, head.y, 44, 36);
    let title = Rect::new(head.x, head.y, prev.x - 8 - head.x, 36);
    let grid = grid.take_top(10).1;
    let rows = ((grid.h - 30 + 10) / (TILE_H + 10)).max(1);
    let w = (grid.w - (LIST_COLS - 1) * 10) / LIST_COLS;
    let tiles = (0..rows * LIST_COLS)
        .map(|i| {
            Rect::new(
                grid.x + (i % LIST_COLS) * (w + 10),
                grid.y + (i / LIST_COLS) * (TILE_H + 10),
                w,
                TILE_H,
            )
        })
        .collect();
    ListLayout {
        panel,
        groups,
        title,
        back,
        prev,
        next,
        tiles,
    }
}

/// A strip's colour, as the mixer's strips have it.
fn strip_color(t: &mpfree_ui::Theme, s: Strip) -> mpfree_ui::Color {
    let palette = [
        t.accent,
        t.good,
        t.warn,
        t.bad,
        t.accent.mix(t.bad, 0.5),
        t.good.mix(t.accent, 0.5),
        t.warn.mix(t.bad, 0.4),
        t.accent.mix(t.text, 0.5),
    ];
    match s {
        Strip::Track(i) | Strip::Pad(i) => palette[i as usize % palette.len()],
        Strip::Input(_) => t.bad.mix(t.warn, 0.5),
        Strip::Bus(_) => t.text_dim,
        Strip::Return(_) => t.accent.mix(t.text_dim, 0.6),
        Strip::Master => t.text,
    }
}

/// See the module docs.
pub struct ChainView {
    state: ChainState,
    /// The selected effect's drawings ([`crate::state::picture`]).
    picture: Visuals,
    actions: Vec<ChainAction>,
    drag: Option<(u64, Drag)>,
    /// The picker's section, page, wheel highlight and open presets.
    picker: PickerView,
    /// The strip list's group while it is open (the view keeps it open;
    /// the app fills it), and the first tile shown.
    list_open: Option<usize>,
    list_first: usize,
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
            picker: PickerView::default(),
            list_open: None,
            list_first: 0,
        }
    }

    pub fn state(&self) -> &ChainState {
        &self.state
    }

    /// The picker's own state (its section, page, highlight, presets).
    pub fn picker(&self) -> &PickerView {
        &self.picker
    }

    /// Shows a new state. Of the old one it keeps only an open picker
    /// (the editor opened it; a state from the app with `picking` set
    /// opens it too).
    pub fn update(&mut self, st: ChainState) {
        let opening = st.picking && !self.state.picking;
        let picking = st.picking || self.state.picking;
        let old = self.state.list.take();
        self.state = st;
        self.state.picking = picking;
        // The strip list stays as the view has it: open on its group (the
        // app's strips once they come), or shut.
        self.state.list = self.list_open.map(|g| match self.state.list.take() {
            Some(l) if l.group == g => l,
            _ => old.filter(|l| l.group == g).unwrap_or(StripList {
                group: g,
                ..StripList::default()
            }),
        });
        if opening {
            self.open_picker();
        }
    }

    /// The picker opens on the section it showed last (the first time:
    /// Favourites if there are any, else the first category), on its first
    /// page, nothing highlighted. The section is kept from here, so a star
    /// doesn't move the picker to Favourites under the finger.
    fn open_picker(&mut self) {
        self.picker.reopen();
        let (list, at) = self.picker.current(&self.state);
        self.picker.section = Some(list[at]);
    }

    /// The strip list's group, while it is open.
    pub fn list_open(&self) -> Option<usize> {
        self.list_open
    }

    /// Opens the strip list on group `g`, or shuts it (None), and tells
    /// the app.
    fn show_list(&mut self, g: Option<usize>) {
        if g != self.list_open {
            self.list_first = 0;
        }
        self.list_open = g;
        self.state.list = g.map(|g| match self.state.list.take() {
            Some(mut l) => {
                if l.group != g {
                    l.strips.clear();
                }
                l.group = g;
                l
            }
            None => StripList {
                group: g,
                ..StripList::default()
            },
        });
        self.push(ChainAction::Strips(g));
    }

    /// The strip list over the editor.
    fn paint_list(&self, cx: &mut PaintCx, r: Rect, list: &StripList) {
        let t = cx.theme.clone();
        let ll = list_layout(r);
        cx.canvas.fill_round_rect(ll.panel, 14, t.surface);
        cx.canvas.stroke_round_rect(ll.panel, 14, 1, t.outline);
        let label = Rect::new(ll.groups[0].x, ll.panel.y + 14, ll.groups[0].w, 28);
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            label,
            Align::Left,
            t.text_dim,
            "STRIPS",
        );
        for (i, (b, name)) in ll.groups.iter().zip(STRIP_GROUPS).enumerate() {
            let on = i == list.group;
            cx.canvas.fill_round_rect(
                *b,
                10,
                if on {
                    t.raised
                } else {
                    t.surface.mix(t.raised, 0.35)
                },
            );
            if on {
                cx.canvas
                    .fill_rect(Rect::new(b.x, b.y + 8, 3, b.h - 16), t.accent);
            }
            let inner = b.inset_xy(14, 0);
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body - 2.0,
                inner,
                Align::Left,
                if on { t.text } else { t.text_dim },
                name,
            );
            if i < 5 {
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    inner,
                    Align::Right,
                    t.text_faint,
                    &list.counts[i].to_string(),
                );
            }
        }
        let group = STRIP_GROUPS[list.group.min(STRIP_GROUPS.len() - 1)];
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            ll.title,
            Align::Left,
            t.text_dim,
            &format!("{} · TAP ONE TO OPEN ITS CHAIN", group.to_uppercase()),
        );
        cx.canvas.fill_round_rect(ll.back, 10, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            ll.back,
            Align::Center,
            t.text,
            "Back",
        );
        let n = list.strips.len();
        let page = ll.tiles.len().max(1);
        if n > page {
            for (b, s, live) in [
                (ll.prev, "<", self.list_first > 0),
                (ll.next, ">", self.list_first + page < n),
            ] {
                cx.canvas
                    .fill_round_rect(b, 10, if live { t.raised } else { t.surface });
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    b,
                    Align::Center,
                    if live { t.text } else { t.text_faint },
                    s,
                );
            }
        }
        let numbered = list.group <= 1;
        for (i, tile) in self.list_tiles(&ll) {
            let p = &list.strips[i];
            cx.canvas.fill_round_rect(tile, 10, t.raised);
            if p.strip == self.state.strip {
                cx.canvas.stroke_round_rect(tile, 10, 2, t.accent);
            }
            cx.canvas.fill_round_rect(
                Rect::new(tile.x, tile.y + 12, 4, tile.h - 24),
                2,
                strip_color(&t, p.strip),
            );
            let inner = Rect::new(tile.x + 18, tile.y + 12, tile.w - 30, tile.h - 24);
            let (top, bottom) = inner.take_top(inner.h / 2 + 2);
            let name = if numbered {
                format!("{}  {}", i + 1, p.name)
            } else {
                p.name.clone()
            };
            let muted = p.sub.starts_with("muted");
            cx.text(
                FontId::Bold,
                t.size_small,
                top,
                Align::Left,
                if muted { t.text_dim } else { t.text },
                &name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                bottom,
                Align::Left,
                t.text_dim,
                &p.sub,
            );
        }
        if n == 0 {
            let area = Rect::new(
                ll.title.x,
                ll.title.bottom() + 10,
                ll.back.right() - ll.title.x,
                TILE_H,
            );
            cx.text(
                FontId::Sans,
                t.size_body,
                area,
                Align::Left,
                t.text_dim,
                "No strips here",
            );
        }
    }

    /// The group the strip button opens the list on: the open strip's.
    fn group_of(s: Strip) -> usize {
        match s {
            Strip::Track(_) => 0,
            Strip::Pad(_) => 1,
            Strip::Input(_) => 2,
            Strip::Return(_) => 3,
            Strip::Bus(_) => 4,
            Strip::Master => 5,
        }
    }

    /// The tiles shown on the list's page: (strip index, tile).
    fn list_tiles(&self, ll: &ListLayout) -> Vec<(usize, Rect)> {
        let n = self.state.list.as_ref().map_or(0, |l| l.strips.len());
        (self.list_first..n).zip(ll.tiles.iter().copied()).collect()
    }

    /// A tap while the strip list is open: a group, a strip (its chain),
    /// the arrows, or Back (anywhere off the panel shuts it too).
    fn tap_list(&mut self, r: Rect, x: i32, y: i32) {
        let ll = list_layout(r);
        if !ll.panel.contains(x, y) || ll.back.contains(x, y) {
            self.show_list(None);
            return;
        }
        if let Some(g) = ll.groups.iter().position(|b| b.contains(x, y)) {
            if Some(g) != self.list_open {
                self.show_list(Some(g));
            }
            return;
        }
        let n = self.state.list.as_ref().map_or(0, |l| l.strips.len());
        let page = ll.tiles.len().max(1);
        if ll.prev.contains(x, y) && n > page {
            self.list_first = self.list_first.saturating_sub(page);
            return;
        }
        if ll.next.contains(x, y) && n > page {
            if self.list_first + page < n {
                self.list_first += page;
            }
            return;
        }
        let hit = self
            .list_tiles(&ll)
            .into_iter()
            .find(|(_, t)| t.contains(x, y))
            .map(|(i, _)| i);
        let strip = hit.and_then(|i| self.state.list.as_ref()?.strips.get(i).map(|p| p.strip));
        if let Some(s) = strip {
            self.list_open = None;
            self.state.list = None;
            self.push(ChainAction::OpenStrip(s));
        }
    }

    /// Shows the selected effect's drawings ([`crate::state::picture`];
    /// empty for none).
    pub fn set_picture(&mut self, cards: Vec<Card>) {
        self.picture.cards = cards;
    }

    /// Where everything goes in `r` as the widget stands.
    pub fn layout(&self, r: Rect) -> Layout {
        layout(r, &self.state, !self.picture.cards.is_empty(), &self.picker)
    }

    /// The data wheel while the picker is open: through the section's
    /// tiles (or the presets list), or with SHIFT through the sections.
    /// False when the picker is shut, so the wheel keeps its other job.
    pub fn wheel(&mut self, delta: i32, shift: bool) -> bool {
        if !self.state.picking || delta == 0 {
            return false;
        }
        let pv = &mut self.picker;
        if shift {
            let (list, at) = pv.current(&self.state);
            let to = (at as i32 + delta).clamp(0, list.len() as i32 - 1) as usize;
            pv.section = Some(list[to]);
            pv.page = 0;
            pv.presets = None;
            pv.highlight = None;
            return true;
        }
        let n = match pv.presets {
            Some(id) => 1 + mpfree_fx::picker::preset_rows(id).len(),
            None => pv.picks(&self.state).len(),
        };
        if n > 0 {
            let at = pv.highlight.map_or(-1, |h| h as i32);
            pv.highlight = Some((at + delta).clamp(0, n as i32 - 1) as usize);
        }
        true
    }

    /// The data wheel's push while the picker is open: on an effect, open
    /// its presets (or add it, when it has none); on the presets list, add
    /// the effect with the one highlighted ("Defaults" first). False when
    /// the picker is shut.
    pub fn push_wheel(&mut self) -> bool {
        if !self.state.picking {
            return false;
        }
        let Some(h) = self.picker.highlight else {
            // The first push lights the first tile.
            self.picker.highlight = Some(0);
            return true;
        };
        match self.picker.presets {
            Some(id) => self.add_preset(id, h),
            None => {
                if let Some(p) = self.picker.picks(&self.state).get(h) {
                    self.open_or_add(p.id);
                }
            }
        }
        true
    }

    /// Opens an effect's presets, or adds it when it has none.
    fn open_or_add(&mut self, id: &'static str) {
        if mpfree_fx::picker::preset_rows(id).is_empty() {
            self.state.picking = false;
            self.push(ChainAction::Add(id));
        } else {
            self.picker.presets = Some(id);
            self.picker.highlight = Some(0);
            self.picker.page = 0;
        }
    }

    /// Adds `id` with presets list row `row` (0 is "Defaults"). A row of a
    /// family's list adds the engine of its mode.
    fn add_preset(&mut self, id: &'static str, row: usize) {
        self.state.picking = false;
        self.picker.presets = None;
        let rows = mpfree_fx::picker::preset_rows(id);
        self.push(match row.checked_sub(1).and_then(|r| rows.get(r)) {
            Some(r) => ChainAction::AddPreset {
                id: r.id,
                preset: r.preset,
            },
            None => ChainAction::Add(id),
        });
    }

    /// A touch in the open picker, on the finger's way down. Sections,
    /// pages, Back and the star act at once; a tile, a preset row and a
    /// presets line wait for the finger to lift on them, so a slip adds
    /// nothing (and a tile tells a tap from a hold).
    fn tap_picker(&mut self, pl: &PickerLayout, finger: u64, x: i32, y: i32) {
        let Some(hit) = crate::picker::hit(pl, x, y) else {
            return;
        };
        let pv = &mut self.picker;
        match hit {
            Hit::Section(i) => {
                let (list, _) = pv.current(&self.state);
                pv.section = list.get(i).copied();
                pv.page = 0;
                pv.highlight = None;
                pv.presets = None;
            }
            Hit::Prev | Hit::Next => {
                let step = if hit == Hit::Next { 1 } else { -1 };
                pv.page = (pl.page as i32 + step).clamp(0, pl.pages as i32 - 1) as usize;
                pv.highlight = None;
            }
            Hit::Back => {
                pv.presets = None;
                pv.highlight = None;
                pv.page = 0;
            }
            Hit::Star(i) => {
                if let Some(p) = pv.picks(&self.state).get(i) {
                    let on = !self.state.shelf.is_favourite(p.id);
                    self.state.shelf.set_favourite(p.id, on);
                    self.push(ChainAction::Favourite { id: p.id, on });
                }
            }
            Hit::Chip(_) | Hit::Tile(_) => {
                pv.press = Some((finger, hit, std::time::Instant::now()))
            }
        }
    }

    /// A finger lifted on what it went down on: a tile adds its effect (or
    /// opens its presets if it was held, or adds an effect that has none);
    /// a preset row adds the effect with that preset; a presets line opens
    /// the presets.
    fn lift_picker(&mut self, pl: &PickerLayout, finger: u64, x: i32, y: i32) {
        let Some((f, hit, since)) = self.picker.press.take() else {
            return;
        };
        if f != finger || crate::picker::hit(pl, x, y) != Some(hit) {
            return;
        }
        let pick = |s: &ChainView, i: usize| s.picker.picks(&s.state).get(i).copied();
        match (hit, self.picker.presets) {
            (Hit::Tile(row), Some(id)) => self.add_preset(id, row),
            (Hit::Tile(i), None) => {
                let Some(p) = pick(self, i) else {
                    return;
                };
                if since.elapsed() >= HOLD {
                    self.open_or_add(p.id);
                } else {
                    self.state.picking = false;
                    self.push(ChainAction::Add(p.id));
                }
            }
            (Hit::Chip(i), None) => {
                if let Some(p) = pick(self, i) {
                    self.open_or_add(p.id);
                }
            }
            _ => {}
        }
    }

    /// A finger has stayed on an effect's tile past [`HOLD`] and the
    /// effect has presets: [`ChainView::hold_tick`] opens them now.
    pub fn hold_due(&self) -> bool {
        let Some((_, Hit::Tile(i), since)) = self.picker.press else {
            return false;
        };
        self.state.picking
            && self.picker.presets.is_none()
            && since.elapsed() >= HOLD
            && self
                .picker
                .picks(&self.state)
                .get(i)
                .is_some_and(|p| !mpfree_fx::picker::preset_rows(p.id).is_empty())
    }

    /// Call every frame while the picker is open: a finger held on a tile
    /// opens its presets while it is still down, as other holds do, and
    /// its lift then does nothing more. True when it opened them. (An
    /// effect with no presets waits for the lift, which adds it.)
    pub fn hold_tick(&mut self) -> bool {
        if !self.hold_due() {
            return false;
        }
        let Some((_, Hit::Tile(i), _)) = self.picker.press.take() else {
            return false;
        };
        let Some(p) = self.picker.picks(&self.state).get(i).copied() else {
            return false;
        };
        self.open_or_add(p.id);
        true
    }

    /// Tests: the finger on a tile has been there `secs` longer.
    #[cfg(test)]
    pub(crate) fn age_press(&mut self, secs: f32) {
        if let Some((_, _, since)) = &mut self.picker.press {
            *since -= std::time::Duration::from_secs_f32(secs);
        }
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

    /// Moves insert `from` to slot `to` (Up, Down, a drag or the wheel):
    /// asks the app, and shows it moved, picked, at once.
    fn move_slot(&mut self, from: usize, to: usize) {
        if from == to || from >= self.state.slots.len() || to >= self.state.slots.len() {
            return;
        }
        self.push(ChainAction::Move { from, to });
        let s = self.state.slots.remove(from);
        self.state.slots.insert(to, s);
        self.state.selected = Some(to);
    }

    /// The effect whose row (not its ON pill) is under `(x, y)`, when the
    /// chain can be rearranged (not while the picker is up).
    fn slot_under(&self, l: &Layout, x: i32, y: i32) -> Option<usize> {
        if self.state.picking {
            return None;
        }
        l.rows
            .iter()
            .position(|r| r.contains(x, y) && !r.take_right(56).0.contains(x, y))
    }

    fn tap(&mut self, l: &Layout, finger: u64, x: i32, y: i32) {
        if l.back.is_some_and(|b| b.contains(x, y)) {
            self.push(ChainAction::Back);
            return;
        }
        if l.strip.is_some_and(|b| b.contains(x, y)) {
            self.show_list(Some(ChainView::group_of(self.state.strip)));
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
            if let Some(pl) = &l.picker {
                if picker_area(l.effect, l.side).contains(x, y) {
                    let pl = pl.clone();
                    self.tap_picker(&pl, finger, x, y);
                    return;
                }
            }
            if l.effect.contains(x, y) || l.side.contains(x, y) {
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
            self.open_picker();
            self.push(ChainAction::Picker(true));
            return;
        }
        if let Some(sel) = st.selected {
            let n = st.slots.len();
            if l.buttons[0].contains(x, y) && sel > 0 {
                self.move_slot(sel, sel - 1);
                return;
            }
            if l.buttons[1].contains(x, y) && sel + 1 < n {
                self.move_slot(sel, sel + 1);
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
        if let (Some(&(_, mode)), Some(slot)) =
            (l.modes.iter().find(|(r, _)| r.contains(x, y)), st.selected)
        {
            if st.mode.as_ref().is_some_and(|m| m.current != Some(mode)) {
                self.push(ChainAction::SetMode { slot, mode });
            }
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

/// The strip button: the strip's colour, its name and which strip it is,
/// and a chevron (a tap opens the strip list).
/// One effect's row in the chain column: its number, name, summary (or
/// "bypassed"), macro dots and wave mark, and its ON pill.
fn paint_slot(
    cx: &mut PaintCx,
    t: &mpfree_ui::Theme,
    row: Rect,
    i: usize,
    s: &crate::state::SlotView,
    sel: bool,
) {
    cx.canvas.fill_round_rect(
        row,
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
    let ink = t.button_ink(s.on);
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
    // The name stops short of the macro dots and the wave mark drawn
    // below (a long "Reverb · Echoes" is cut with "…" rather than run
    // under them): the dots are 9 px in 12 px steps from the right,
    // the wave's ink about 5 px into its 20 px box left of them.
    let dots = text.right() - 4 - 12 * s.macros.len() as i32;
    let marks = match (s.modulated, s.macros.is_empty()) {
        (true, _) => dots - 22 + 5 - 4,
        (false, false) => dots - 4,
        (false, true) => text.right(),
    };
    let name = top.take_bottom(24).0;
    cx.text(
        FontId::Bold,
        t.size_body,
        Rect::new(name.x, name.y, marks - name.x, name.h),
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
            .fill_round_rect(Rect::new(dx, row.y + 14, 9, 9), 5, macro_color(t, *m));
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

fn paint_strip_button(cx: &mut PaintCx, b: Rect, st: &ChainState) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(b, 9, t.raised);
    cx.canvas.stroke_round_rect(b, 9, 1, t.outline);
    let dot = Rect::new(b.x + 10, b.y + b.h / 2 - 4, 8, 8);
    cx.canvas.fill_round_rect(dot, 4, strip_color(&t, st.strip));
    let inner = Rect::new(dot.right() + 8, b.y, b.w - 26 - 28, b.h);
    let name_w = cx.measure(FontId::Bold, t.size_small, &st.strip_name);
    cx.text(
        FontId::Bold,
        t.size_small,
        inner,
        Align::Left,
        t.text,
        &st.strip_name,
    );
    if st.strip_kind != st.strip_name && name_w + 8 < inner.w {
        let kind = Rect::new(inner.x + name_w + 8, inner.y, inner.w - name_w - 8, inner.h);
        cx.text(
            FontId::Sans,
            t.size_small - 3.0,
            kind,
            Align::Left,
            t.text_dim,
            &st.strip_kind,
        );
    }
    // The chevron: two short diagonals.
    let (cx0, cy0) = (b.right() - 22, b.y + b.h / 2 - 3);
    for i in 0..5 {
        cx.canvas
            .fill_rect(Rect::new(cx0 + i, cy0 + i, 2, 2), t.text_dim);
        cx.canvas
            .fill_rect(Rect::new(cx0 + 9 - i, cy0 + i, 2, 2), t.text_dim);
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
        if let Some(b) = l.back {
            cx.canvas.fill_round_rect(b, 9, t.raised);
            cx.text(FontId::Bold, t.size_title, b, Align::Center, t.text, "‹");
        }
        if let Some(b) = l.strip {
            paint_strip_button(cx, b, st);
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
        if st.scopes.is_empty() && l.strip.is_none() {
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
        // A row being dragged: its own place empty but outlined, a line
        // where it will land, and the row itself under the finger.
        let lifted = match self.drag {
            Some((
                _,
                Drag::Slot {
                    from,
                    y0,
                    y,
                    to,
                    moved: true,
                },
            )) => Some((from, y - y0, to)),
            _ => None,
        };
        for (i, (s, row)) in st.slots.iter().zip(&l.rows).enumerate() {
            if lifted.is_some_and(|(from, ..)| from == i) {
                cx.canvas.fill_round_rect(*row, 10, t.surface);
                cx.canvas.stroke_round_rect(*row, 10, 2, t.outline);
                continue;
            }
            paint_slot(cx, &t, *row, i, s, st.selected == Some(i));
        }
        if let (Some((from, dy, to)), Some(first), Some(last)) =
            (lifted, l.rows.first(), l.rows.last())
        {
            // The row follows the finger within the list, but stays clear
            // of the landing line: below it on the way up, above it on the
            // way down, so the line is never hidden under the row.
            let (mut lo, mut hi) = (first.y, last.y);
            if to != from {
                let at = &l.rows[to];
                let y = if to < from {
                    at.y - ROW_GAP / 2 - 2
                } else {
                    at.bottom() + ROW_GAP / 2 - 2
                };
                cx.canvas
                    .fill_round_rect(Rect::new(at.x, y, at.w, 4), 2, t.accent);
                for x in [at.x - 6, at.right() - 6] {
                    cx.canvas
                        .fill_round_rect(Rect::new(x, y - 4, 12, 12), 6, t.accent);
                }
                if to < from {
                    lo = y + 14;
                } else {
                    hi = y - 10 - ROW_H;
                }
            }
            if let (Some(s), Some(home)) = (st.slots.get(from), l.rows.get(from)) {
                let y = (home.y + dy).clamp(lo, hi.max(lo));
                let row = Rect::new(home.x + 8, y, home.w, home.h);
                paint_slot(cx, &t, row, from, s, true);
                cx.canvas.stroke_round_rect(row, 10, 2, t.accent);
            }
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
                &match st.cost_pct.filter(|_| l.strip.is_some()) {
                    // The strip button took the title's place: the cost,
                    // while profiling, comes here.
                    Some(c) => format!("{} of {} · {c:.1} %", st.slots.len(), st.room),
                    None => format!("{} of {}", st.slots.len(), st.room),
                },
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
                t.button_ink(ok),
                label,
            );
        }
        // ---- Effect column.
        cx.canvas.fill_round_rect(l.effect, t.radius, t.surface);
        if st.picking {
            paint_picker(cx, &l, st, &self.picker);
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
                if let Some(mv) = &st.mode {
                    paint_modes(cx, &l, mv);
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
        if let Some(list) = &st.list {
            self.paint_list(cx, r, list);
        }
    }

    /// Passes the touch on to the app (an event) whenever it made actions,
    /// as the mixer does: `Ui::touch` drops a widget's touch that answers
    /// None, and the app takes the actions only when it gets the event.
    fn targets(&self, r: Rect) -> Vec<Target> {
        let l = self.layout(r);
        let st = &self.state;
        let mut out = Vec::new();
        if let Some(list) = &st.list {
            // The strip list takes the editor while it is open.
            let ll = list_layout(r);
            for (i, (b, name)) in ll.groups.iter().zip(STRIP_GROUPS).enumerate() {
                out.push(Target::tap("strips-group", i, name, *b).disabled_if(i == list.group));
            }
            let (n, page) = (list.strips.len(), ll.tiles.len().max(1));
            if n > page {
                out.push(
                    Target::tap("strips-prev", 0, "<", ll.prev).disabled_if(self.list_first == 0),
                );
                out.push(
                    Target::tap("strips-next", 0, ">", ll.next)
                        .disabled_if(self.list_first + page >= n),
                );
            }
            for (i, tile) in self.list_tiles(&ll) {
                out.push(Target::tap(
                    "strips-pick",
                    i,
                    list.strips[i].name.clone(),
                    tile,
                ));
            }
            out.push(Target::tap("strips-back", 0, "Back", ll.back));
            return out;
        }
        if let Some(b) = l.back {
            out.push(Target::tap("back", 0, "‹ Back", b));
        }
        if let Some(b) = l.strip {
            out.push(Target::tap("strip", 0, st.strip_name.clone(), b));
        }
        for (i, sr) in l.scopes.iter().enumerate() {
            let label = st.scopes.get(i).cloned().unwrap_or_default();
            out.push(Target::tap("scope", i, label, *sr));
        }
        if st.picking {
            if let Some(c) = l.cancel {
                out.push(Target::tap("cancel", 0, "Cancel", c));
            }
            if let Some(pl) = &l.picker {
                picker_targets(&self.picker, st, pl, &mut out);
            }
        }
        for (i, rr) in l.rows.iter().enumerate() {
            let name = st.slots.get(i).map_or(String::new(), |v| v.name.clone());
            let (power, rest) = (rr.take_right(56).0, rr.take_right(56).1);
            out.push(
                Target::tap("slot", i, name.clone(), rest)
                    .moves_if(st.selected == Some(i) && !st.picking),
            );
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
        if let Some(mv) = &st.mode {
            for &(r, m) in &l.modes {
                let name = mv.modes.get(m).copied().unwrap_or("");
                out.push(
                    Target::tap("mode", m, format!("{} mode {name}", mv.family), r)
                        .disabled_if(mv.current == Some(m)),
                );
            }
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

    fn steps(&self, t: &Target) -> bool {
        match t.id {
            "knob" => self.state.params.get(t.index).is_some(),
            "slot" => t.index < self.state.slots.len() && !self.state.picking,
            _ => false,
        }
    }

    /// The data wheel on a knob: a hundredth of its travel per detent (a
    /// switch with few places moves one place), as a tap, a turn and a lift
    /// of a finger would. On a grabbed effect: one place along the chain
    /// per detent (turned up, later in the chain), as Up and Down do.
    fn step(&mut self, t: &Target, delta: i32, _r: Rect) -> Option<EventKind> {
        if t.id == "slot" {
            let n = self.state.slots.len();
            let from = self.state.selected.unwrap_or(t.index);
            let to = (from as i64 + delta as i64).clamp(0, n.max(1) as i64 - 1) as usize;
            if to == from || from >= n {
                return None;
            }
            self.move_slot(from, to);
            return Some(EventKind::Select(self.actions.len()));
        }
        let p = self.state.params.get(t.index)?;
        let (addr, pos, from) = (p.addr, p.pos, p.value);
        let value = (1..=100)
            .map(|n| p.info.from_unit(pos + (delta * n) as f32 * WHEEL_STEP))
            .find(|v| *v != from)?;
        if self.state.focus != Some(addr) {
            self.push(ChainAction::Focus(addr));
        }
        self.push(ChainAction::Set { addr, value });
        self.push(ChainAction::Release(addr));
        Some(EventKind::Select(self.actions.len()))
    }

    /// A grabbed effect the wheel moved is listed at its new place.
    fn focus_index(&self, t: &Target) -> usize {
        match (t.id, self.state.selected) {
            ("slot", Some(sel)) => sel,
            _ => t.index,
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let before = self.actions.len();
        self.touch_inner(finger, phase, x, y, r);
        (self.actions.len() > before).then_some(EventKind::Select(self.actions.len()))
    }
}

/// A knob's step per detent of the data wheel, as a share of its travel.
const WHEEL_STEP: f32 = 0.01;

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
        if self.list_open.is_some() {
            // The strip list takes every touch while it is open.
            if phase == Touch::Down {
                self.tap_list(r, x, y);
            }
            return None;
        }
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
                self.tap(&l, finger, x, y);
                // A finger on an effect's row (not its ON pill) may drag
                // it up or down the chain.
                if let Some(from) = self.slot_under(&l, x, y) {
                    self.drag = Some((
                        finger,
                        Drag::Slot {
                            from,
                            y0: y,
                            y,
                            to: from,
                            moved: false,
                        },
                    ));
                }
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
                    Drag::Slot {
                        from, y0, moved, ..
                    } => {
                        let moved = moved || (y - y0).abs() >= SLOT_SLOP;
                        let to = if moved { slot_at(&l, y) } else { from };
                        self.drag = Some((
                            finger,
                            Drag::Slot {
                                from,
                                y0,
                                y,
                                to,
                                moved,
                            },
                        ));
                    }
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
                if let (Touch::Up, Some(pl)) = (phase, &l.picker) {
                    let pl = pl.clone();
                    self.lift_picker(&pl, finger, x, y);
                } else if self.picker.press.is_some_and(|(f, ..)| f == finger) {
                    self.picker.press = None;
                }
                if self.drag.as_ref().is_some_and(|(f, _)| *f == finger) {
                    match self.drag.take() {
                        Some((_, Drag::Knob { i, .. })) => {
                            if let Some(p) = self.state.params.get(i) {
                                let addr = p.addr;
                                self.push(ChainAction::Release(addr));
                            }
                        }
                        // Lifted somewhere else in the chain: it moves
                        // there (a cancelled touch leaves it).
                        Some((
                            _,
                            Drag::Slot {
                                from,
                                to,
                                moved: true,
                                ..
                            },
                        )) if phase == Touch::Up && to != from => self.move_slot(from, to),
                        _ => {}
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

/// The effect picker, over the effect and Modulate columns.
fn paint_picker(cx: &mut PaintCx, l: &Layout, st: &ChainState, pv: &PickerView) {
    let t = cx.theme.clone();
    let area = picker_area(l.effect, l.side);
    cx.canvas.fill_rect(
        Rect::new(
            l.effect.right(),
            area.y,
            l.side.x - l.effect.right(),
            area.h,
        ),
        t.bg,
    );
    cx.canvas.fill_round_rect(area, t.radius, t.surface);
    let (head, _) = area.inset(14).take_top(52);
    // Where the effect goes (Jay's Mix tab card, 14 Nov): the strip, and
    // the slot it lands in (the end of the chain).
    let strip = if st.strip_name.is_empty() {
        st.title.trim_end_matches(" FX").to_string()
    } else {
        st.strip_name.clone()
    }
    .to_uppercase();
    let slot = st.slots.len() + 1;
    let title = match (pv.presets, slot > st.room) {
        (_, true) => format!(
            "{strip} IS FULL · {} OF {} SLOTS USED",
            st.slots.len(),
            st.room
        ),
        (Some(_), _) => format!("PICK A PRESET OR DEFAULTS · IT GOES IN SLOT {slot} OF {strip}"),
        _ => format!("ADD TO {strip} · IT GOES IN SLOT {slot} OF {}", st.room),
    };
    cx.text(
        FontId::Bold,
        t.size_small,
        head.take_top(42).0,
        Align::Left,
        t.text_dim,
        &title,
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
    match &l.picker {
        Some(pl) => crate::picker::paint(cx, pl, st, pv),
        None => {
            cx.text(
                FontId::Sans,
                t.size_body,
                area,
                Align::Center,
                t.text_dim,
                "This chain is full: remove an effect first",
            );
        }
    }
}

/// The Mode chooser: chips with the current mode lit, or the arrows and
/// the mode's name ("Tube   3 of 10"; "Reverb mode" when in none).
fn paint_modes(cx: &mut PaintCx, l: &Layout, mv: &crate::state::ModeView) {
    let t = cx.theme.clone();
    match l.mode_label {
        None => {
            for &(r, m) in &l.modes {
                let on = mv.current == Some(m);
                cx.canvas
                    .fill_round_rect(r, 9, if on { t.accent } else { t.raised });
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    r,
                    Align::Center,
                    if on { t.on_accent } else { t.text },
                    mv.modes[m],
                );
            }
        }
        Some(label) => {
            let row = Rect::new(
                label.x - MODE_ARROW_W - 6,
                label.y,
                label.w + 2 * (MODE_ARROW_W + 6),
                label.h,
            );
            let prev = Rect::new(row.x, row.y, MODE_ARROW_W, row.h);
            let next = Rect::new(row.right() - MODE_ARROW_W, row.y, MODE_ARROW_W, row.h);
            for (r, glyph) in [(prev, "‹"), (next, "›")] {
                let live = l.modes.iter().any(|(m, _)| *m == r);
                cx.canvas.fill_round_rect(
                    r,
                    9,
                    if live {
                        t.raised
                    } else {
                        t.surface.mix(t.raised, 0.3)
                    },
                );
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    r,
                    Align::Center,
                    if live { t.text } else { t.text_faint },
                    glyph,
                );
            }
            let text = match mv.current {
                Some(c) => format!("{}   {} of {}", mv.modes[c], c + 1, mv.modes.len()),
                None => format!("{} mode", mv.family),
            };
            cx.text(
                FontId::Bold,
                t.size_small,
                label,
                Align::Center,
                t.text,
                &text,
            );
        }
    }
}

/// `text` in lines no wider than `w` (a word wider than `w` gets a line of
/// its own).
pub(crate) fn wrap(cx: &mut PaintCx, size: f32, text: &str, w: i32) -> Vec<String> {
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

/// The effect picker's controls, as it draws them: its sections, then the
/// section's effect tiles with their stars and presets lines (or the open
/// presets list with its way back), then the pager.
fn picker_targets(pv: &PickerView, st: &ChainState, pl: &PickerLayout, out: &mut Vec<Target>) {
    let (list, _) = pv.current(st);
    for (i, r) in pl.sections.iter().enumerate() {
        let name = list.get(i).map_or("", |s| s.name());
        out.push(Target::tap("section", i, name, *r).disabled_if(i == pl.section));
    }
    match pv.presets {
        Some(id) => {
            if let Some(b) = pl.back {
                out.push(Target::tap("presets-back", 0, "‹ Effects", b));
            }
            let presets = mpfree_fx::picker::preset_rows(id);
            for (k, r) in pl.tiles.iter().enumerate() {
                let item = pl.first + k;
                let label = match item {
                    0 => "Defaults",
                    _ => presets.get(item - 1).map_or("", |p| p.name),
                };
                out.push(Target::tap("preset", item, label, *r));
            }
        }
        None => {
            let picks = pv.picks(st);
            for (k, r) in pl.tiles.iter().enumerate() {
                let i = pl.first + k;
                let name = picks.get(i).map_or("", |p| p.name);
                out.push(Target::tap("pick", i, name, *r));
                if let Some(star) = pl.stars.get(k) {
                    out.push(Target::tap("star", i, format!("{name} favourite"), *star));
                }
                if let Some(Some(chip)) = pl.chips.get(k) {
                    out.push(Target::tap("presets", i, format!("{name} presets"), *chip));
                }
            }
        }
    }
    if let Some(r) = pl.prev {
        out.push(Target::tap("page-prev", 0, "‹", r).disabled_if(pl.page == 0));
    }
    if let Some(r) = pl.next {
        out.push(Target::tap("page-next", 0, "›", r).disabled_if(pl.page + 1 >= pl.pages));
    }
}
