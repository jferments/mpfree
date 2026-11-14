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

use crate::picker::{Hit, PickerLayout, PickerView, HOLD};
use crate::state::{macro_color, ChainState, KNOBS_PER_PAGE};

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
/// The row of page tabs, under the title (and the Mode chooser).
const TABS_H: i32 = 36;
/// The arrows each end of the tab row, when its tabs don't all fit.
const TAB_ARROW_W: i32 = 40;
/// The stage strip on a Main page (the Channel strip's gate, compressor
/// and EQ).
const STAGES_H: i32 = 40;

/// Where everything goes, from the widget's rectangle.
pub struct Layout {
    pub back: Option<Rect>,
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
    /// A tab per page, in the order of `ChainState::groups`; a tab out of
    /// view while the row scrolls is empty (nothing to draw or tap).
    pub tabs: Vec<Rect>,
    /// The arrows to the page before and after, when the tabs don't all
    /// fit the row.
    pub tab_arrows: Option<(Rect, Rect)>,
    /// On a Main page, each stage's name (a tap opens its page) and its
    /// switch.
    pub stages: Vec<(Rect, Rect)>,
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
    // Effect column: the effect's name, its Mode chooser, its page tabs,
    // a Main page's stages, the drawings, knobs in two rows of four. The
    // picker takes the column's place.
    let einner = effect.inset(14);
    let (head, grid) = einner.take_top(52);
    let title = Rect::new(head.x, head.y, head.w, 42);
    let shown = st.selected.is_some() && !st.picking;
    let (mut modes, mut mode_label) = (Vec::new(), None);
    let grid = match &st.mode {
        Some(mv) if shown => {
            let (row, rest) = grid.take_top(MODE_H + 8);
            let row = Rect::new(row.x, row.y + 8, row.w, MODE_H);
            (modes, mode_label) = mode_row(row, mv);
            rest
        }
        _ => grid,
    };
    let (mut tabs, mut tab_arrows) = (Vec::new(), None);
    let grid = if shown && st.groups.len() > 1 {
        let (row, rest) = grid.take_top(TABS_H + 8);
        (tabs, tab_arrows) = tab_row(Rect::new(row.x, row.y + 8, row.w, TABS_H), st);
        rest
    } else {
        grid
    };
    let mut stages = Vec::new();
    let grid = if shown && !st.stages.is_empty() {
        let (row, rest) = grid.take_top(STAGES_H + 8);
        let row = Rect::new(row.x, row.y + 8, row.w, STAGES_H);
        let n = st.stages.len() as i32;
        let w = (row.w - 8 * (n - 1)) / n;
        for i in 0..n {
            let chip = Rect::new(row.x + i * (w + 8), row.y, w, row.h);
            let (switch, name) = chip.take_right(64);
            stages.push((name, switch.inset_xy(6, 6)));
        }
        rest
    } else {
        grid
    };
    let (picture, grid) = if picture && shown {
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
        tab_arrows = None;
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
        tab_arrows,
        stages,
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

/// The page tabs in `row`, left to right. When they don't all fit, an
/// arrow each end steps to the page before and after, and the tabs shown
/// are a run that holds the page shown (the rest are empty rects).
fn tab_row(row: Rect, st: &ChainState) -> (Vec<Rect>, Option<(Rect, Rect)>) {
    let widths: Vec<i32> = st
        .groups
        .iter()
        .map(|g| 24 + 9 * g.chars().count() as i32)
        .collect();
    let need = |a: usize, b: usize| -> i32 {
        widths[a..b].iter().sum::<i32>() + 6 * (b - a).saturating_sub(1) as i32
    };
    let lay = |area: Rect, first: usize| -> Vec<Rect> {
        let mut x = area.x;
        (0..widths.len())
            .map(|i| {
                if i < first {
                    return Rect::new(0, 0, 0, 0);
                }
                if x + widths[i] > area.right() {
                    // This one and every one after it are out of view.
                    x = area.right() + 1;
                    return Rect::new(0, 0, 0, 0);
                }
                let r = Rect::new(x, area.y, widths[i], area.h);
                x += widths[i] + 6;
                r
            })
            .collect()
    };
    if need(0, widths.len()) <= row.w {
        return (lay(row, 0), None);
    }
    let prev = Rect::new(row.x, row.y, TAB_ARROW_W, row.h);
    let next = Rect::new(row.right() - TAB_ARROW_W, row.y, TAB_ARROW_W, row.h);
    let area = Rect::new(prev.right() + 6, row.y, next.x - prev.right() - 12, row.h);
    // The first tab shown: as far left as keeps the page shown in view.
    let sel = st.group.min(widths.len() - 1);
    let mut first = 0;
    while first < sel && need(first, sel + 1) > area.w {
        first += 1;
    }
    (lay(area, first), Some((prev, next)))
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

/// The effect and Modulate columns together, where the picker goes.
fn picker_area(effect: Rect, side: Rect) -> Rect {
    Rect::new(effect.x, effect.y, side.right() - effect.x, effect.h)
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
    /// The picker's section, page, wheel highlight and open presets.
    picker: PickerView,
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
        self.state = st;
        self.state.picking = picking;
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
                let st = &mut self.state;
                let stages = st.stages.iter_mut().map(|s| &mut s.switch);
                for p in st
                    .params
                    .iter_mut()
                    .chain(stages)
                    .filter(|p| p.addr == *addr)
                {
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

    fn tap(&mut self, l: &Layout, finger: u64, x: i32, y: i32) {
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
        if let Some((prev, next)) = l.tab_arrows {
            let n = st.groups.len();
            if prev.contains(x, y) {
                if st.group > 0 {
                    self.push(ChainAction::Group(st.group - 1));
                }
                return;
            }
            if next.contains(x, y) {
                if st.group + 1 < n {
                    self.push(ChainAction::Group(st.group + 1));
                }
                return;
            }
        }
        for ((name, switch), stage) in l.stages.iter().zip(&st.stages) {
            if switch.contains(x, y) {
                let addr = stage.switch.addr;
                let value = if stage.on() { 0.0 } else { 1.0 };
                self.push(ChainAction::Set { addr, value });
                self.push(ChainAction::Release(addr));
                return;
            }
            if name.contains(x, y) {
                self.push(ChainAction::Group(stage.page));
                return;
            }
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
                    if tr.is_empty() {
                        continue;
                    }
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
                if let Some((prev, next)) = l.tab_arrows {
                    let ends = [st.group == 0, st.group + 1 >= st.groups.len()];
                    for ((r, glyph), end) in [(prev, "‹"), (next, "›")].into_iter().zip(ends) {
                        cx.canvas.fill_round_rect(
                            r,
                            9,
                            if end {
                                t.surface.mix(t.raised, 0.3)
                            } else {
                                t.raised
                            },
                        );
                        cx.text(
                            FontId::Bold,
                            t.size_body,
                            r,
                            Align::Center,
                            if end { t.text_faint } else { t.text },
                            glyph,
                        );
                    }
                }
                paint_stages(cx, &l, st);
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
                        &p.label,
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
            if let Some(pl) = &l.picker {
                picker_targets(&self.picker, st, pl, &mut out);
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
            if tr.is_empty() {
                continue;
            }
            let label = st.groups.get(g).map_or("", String::as_str);
            out.push(Target::tap("group", g, label, *tr).disabled_if(g == st.group));
        }
        if let Some((prev, next)) = l.tab_arrows {
            let n = st.groups.len();
            out.push(Target::tap("group-prev", 0, "‹ pages", prev).disabled_if(st.group == 0));
            out.push(Target::tap("group-next", 0, "pages ›", next).disabled_if(st.group + 1 >= n));
        }
        for (i, ((name, switch), stage)) in l.stages.iter().zip(&st.stages).enumerate() {
            out.push(Target::tap("stage", i, format!("{} ›", stage.label), *name));
            let word = if stage.on() { "on" } else { "off" };
            out.push(Target::tap(
                "stage-on",
                i,
                format!("{} {word}", stage.label),
                *switch,
            ));
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
            let label = st.params.get(i).map_or(String::new(), |p| p.label.clone());
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
        t.id == "knob" && self.state.params.get(t.index).is_some()
    }

    /// The data wheel on a knob: a hundredth of its travel per detent (a
    /// switch with few places moves one place), as a tap, a turn and a lift
    /// of a finger would.
    fn step(&mut self, t: &Target, delta: i32, _r: Rect) -> Option<EventKind> {
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
                if let (Touch::Up, Some(pl)) = (phase, &l.picker) {
                    let pl = pl.clone();
                    self.lift_picker(&pl, finger, x, y);
                } else if self.picker.press.is_some_and(|(f, ..)| f == finger) {
                    self.picker.press = None;
                }
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
        &format!("MODULATE · {}", p.label.to_uppercase()),
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
    let title = match (pv.presets, pv.highlight) {
        (Some(_), _) => "PICK A PRESET · OR DEFAULTS".to_string(),
        _ => format!(
            "ADD EFFECT · {} OF {} SLOTS USED · HOLD A TILE FOR ITS PRESETS",
            st.slots.len(),
            st.room
        ),
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

/// A Main page's stages: each one's name with "›" (its page) and its
/// switch, the name dim while the stage is off.
fn paint_stages(cx: &mut PaintCx, l: &Layout, st: &ChainState) {
    let t = cx.theme.clone();
    for ((name, switch), stage) in l.stages.iter().zip(&st.stages) {
        let on = stage.on();
        let chip = Rect::new(name.x, name.y, switch.right() + 6 - name.x, name.h);
        cx.canvas.fill_round_rect(chip, 9, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            name.inset_xy(12, 0),
            Align::Left,
            if on { t.text } else { t.text_dim },
            &format!("{}  ›", stage.label.to_uppercase()),
        );
        cx.canvas.fill_round_rect(
            *switch,
            switch.h / 2,
            if on {
                t.good.mix(t.surface, 0.3)
            } else {
                t.outline
            },
        );
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            *switch,
            Align::Center,
            if on { t.on_accent } else { t.text_dim },
            if on { "ON" } else { "OFF" },
        );
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
