//! The widgets the sampler's screens add to `mpfree-ui`'s library. Like
//! those, they only show what the page put in their fields and report
//! touches.

use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Touch, Widget};

// ---------------------------------------------------------------- pad bar

/// The bar over the pad editor: the pad's colour and name, a line about its
/// sound, and the editor's tabs on the right. A tap on an enabled tab gives
/// [`EventKind::Tab`].
pub struct PadBar {
    pub color: Color,
    pub title: String,
    pub subtitle: String,
    pub tabs: Vec<String>,
    pub active: usize,
    /// Tabs that can be opened; the others are drawn faint and ignore taps.
    pub enabled: Vec<bool>,
}

const SEG_H: i32 = 40;
const SEG_PAD: i32 = 4;
const SEG_TAB_W: i32 = 84;

impl PadBar {
    pub fn new(tabs: &[&str]) -> PadBar {
        PadBar {
            color: Color::rgb(0xFF8A3D),
            title: String::new(),
            subtitle: String::new(),
            tabs: tabs.iter().map(|s| s.to_string()).collect(),
            active: 0,
            enabled: vec![true; tabs.len()],
        }
    }

    /// The box holding the tabs.
    fn tabs_rect(&self, r: Rect) -> Rect {
        let w = self.tabs.len() as i32 * SEG_TAB_W + 2 * SEG_PAD;
        let h = SEG_H + 2 * SEG_PAD;
        Rect::new(r.right() - 10 - w, r.y + (r.h - h) / 2, w, h)
    }

    /// Tab `i`'s rectangle.
    pub fn tab_rect(&self, r: Rect, i: usize) -> Rect {
        let b = self.tabs_rect(r);
        Rect::new(b.x + SEG_PAD + i as i32 * SEG_TAB_W, b.y + SEG_PAD, SEG_TAB_W, SEG_H)
    }
}

impl Widget for PadBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let sq = 36.min(r.h - 16).max(8);
        let swatch = Rect::new(r.x + t.pad, r.y + (r.h - sq) / 2, sq, sq);
        cx.canvas.fill_round_rect(swatch, 8, self.color);
        let tabs = self.tabs_rect(r);
        let text = Rect::new(swatch.right() + 14, r.y + 6, (tabs.x - swatch.right() - 24).max(0), r.h - 12);
        let (top, bottom) = text.take_top(text.h / 2 + 2);
        cx.text(FontId::Bold, t.size_title + 1.0, top, Align::Left, t.text, &self.title);
        cx.text(FontId::Sans, t.size_small, bottom, Align::Left, t.text_dim, &self.subtitle);
        cx.canvas.fill_round_rect(tabs, 12, t.bg);
        for (i, name) in self.tabs.iter().enumerate() {
            let tr = self.tab_rect(r, i);
            let on = i == self.active;
            let enabled = self.enabled.get(i).copied().unwrap_or(false);
            if on {
                cx.canvas.fill_round_rect(tr, 9, t.raised);
            }
            let color = if on { t.text } else if enabled { t.text_dim } else { t.text_faint };
            cx.text(if on { FontId::Bold } else { FontId::Sans }, t.size_body, tr, Align::Center, color, name);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.tabs.len()).find(|&i| self.tab_rect(r, i).contains(x, y))?;
        self.enabled.get(i).copied().unwrap_or(false).then_some(EventKind::Tab(i))
    }
}

// ---------------------------------------------------------------- knob card

/// A Q-Link's card: a knob showing where its marker sits in the sound, the
/// Q-Link's job, and the marker's position in bars and beats (or seconds).
/// A tap gives [`EventKind::Select`]`(0)`; the page makes it the wheel's
/// focus.
pub struct KnobCard {
    pub label: String,
    pub value: String,
    /// Small text after the value (ticks, or frames).
    pub sub: String,
    /// 0.0 to 1.0: how far round the knob's arc is lit.
    pub pos: f32,
    pub color: Color,
    /// The card the wheel zooms round: outlined.
    pub focused: bool,
    /// Nothing for this Q-Link to move (a loop point without a loop).
    pub enabled: bool,
}

/// The knob's arc: from 135° (down-left) round 270° to 45° (down-right).
const ARC_START: f32 = 135.0;
const ARC_SWEEP: f32 = 270.0;

impl KnobCard {
    pub fn new(label: &str, color: Color) -> KnobCard {
        KnobCard { label: label.to_string(), value: String::new(), sub: String::new(), pos: 0.0, color, focused: false,
                   enabled: true }
    }
}

/// Dots along an arc of the circle at (cx, cy) with radius `rad`, from
/// `from` degrees round `sweep` degrees (clockwise on screen).
fn arc(cx: &mut PaintCx, (x, y): (f32, f32), rad: f32, from: f32, sweep: f32, dot: i32, c: Color) {
    if sweep <= 0.0 {
        return;
    }
    let steps = (sweep / 3.0).ceil().max(1.0) as i32;
    for k in 0..=steps {
        let a = (from + sweep * k as f32 / steps as f32).to_radians();
        let (px, py) = (x + rad * a.cos(), y + rad * a.sin());
        let d = Rect::new((px - dot as f32 / 2.0).round() as i32, (py - dot as f32 / 2.0).round() as i32, dot, dot);
        cx.canvas.fill_round_rect(d, dot / 2, c);
    }
}

impl Widget for KnobCard {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        if self.focused && self.enabled {
            cx.canvas.stroke_round_rect(r, t.radius, 2, t.outline.mix(self.color, 0.6));
        }
        let rad = (r.h as f32 / 2.0 - 22.0).clamp(10.0, 20.0);
        let centre = (r.x as f32 + t.pad as f32 + rad + 2.0, r.y as f32 + r.h as f32 / 2.0);
        arc(cx, centre, rad, ARC_START, ARC_SWEEP, 5, t.raised);
        let color = if self.enabled { self.color } else { t.text_faint };
        if self.enabled {
            arc(cx, centre, rad, ARC_START, ARC_SWEEP * self.pos.clamp(0.0, 1.0), 5, color);
            let a = (ARC_START + ARC_SWEEP * self.pos.clamp(0.0, 1.0)).to_radians();
            let (px, py) = (centre.0 + rad * a.cos(), centre.1 + rad * a.sin());
            cx.canvas.fill_round_rect(Rect::new(px as i32 - 4, py as i32 - 4, 9, 9), 4, t.text);
        }
        let left = (centre.0 + rad + 14.0) as i32;
        let text = Rect::new(left, r.y + 12, (r.right() - t.pad - left).max(0), r.h - 24);
        let (head, body) = text.take_top(22);
        cx.text(FontId::Bold, t.size_small - 1.0, head, Align::Left, t.text_dim, &self.label.to_uppercase());
        let value = if self.enabled { self.value.as_str() } else { "—" };
        let w = cx.text(FontId::Bold, t.size_title + 5.0, body, Align::Left, if self.enabled { t.text } else { t.text_faint },
                        value);
        if self.enabled && !self.sub.is_empty() {
            let sub = Rect::new(body.x + w + 6, body.y + 3, (body.w - w - 6).max(0), body.h);
            cx.text(FontId::Sans, t.size_small, sub, Align::Left, t.text_dim, &self.sub);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, _x: i32, _y: i32, _r: Rect) -> Option<EventKind> {
        (phase == Touch::Down && self.enabled).then_some(EventKind::Select(0))
    }
}

// ---------------------------------------------------------------- settings

/// One setting: its name and its value as text.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub name: String,
    pub value: String,
}

/// A titled list of settings, each with its value between arrows, compact
/// enough to sit beside a waveform. A tap on a row selects it
/// ([`EventKind::Select`]); a tap on an arrow steps it ([`EventKind::Step`]).
/// The list scrolls to keep the selected row in view.
pub struct SettingList {
    pub title: String,
    pub right: String,
    pub rows: Vec<Setting>,
    pub selected: usize,
}

const SET_ROW_H: i32 = 44;
const SET_ARROW_W: i32 = 40;
const SET_VALUE_W: i32 = 210;

impl SettingList {
    pub fn new(title: &str) -> SettingList {
        SettingList { title: title.to_string(), right: String::new(), rows: Vec::new(), selected: 0 }
    }

    fn body(r: Rect) -> Rect {
        // Below the title, as PaintCx::titled_panel lays it out.
        r.inset(14).take_top(34).1
    }

    fn first(&self, visible: usize) -> usize {
        if self.rows.len() <= visible || visible == 0 {
            return 0;
        }
        self.selected.saturating_sub(visible / 2).min(self.rows.len() - visible)
    }

    /// Row `k`'s rectangle and its value box, if it is in view.
    pub fn row_rects(&self, r: Rect, k: usize) -> Option<(Rect, Rect)> {
        let body = SettingList::body(r);
        let visible = (body.h / SET_ROW_H).max(0) as usize;
        let first = self.first(visible);
        if k < first || k >= first + visible || k >= self.rows.len() {
            return None;
        }
        let row = Rect::new(body.x, body.y + (k - first) as i32 * SET_ROW_H, body.w, SET_ROW_H - 4);
        let vw = SET_VALUE_W.min(row.w * 3 / 5);
        Some((row, Rect::new(row.right() - vw - 4, row.y + 4, vw, row.h - 8)))
    }
}

impl Widget for SettingList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        for k in 0..self.rows.len() {
            let Some((row, value)) = self.row_rects(r, k) else { continue };
            let on = k == self.selected;
            if on {
                cx.canvas.fill_round_rect(row, 10, t.raised);
                cx.canvas.fill_round_rect(Rect::new(row.x, row.y + 10, 4, row.h - 20), 2, t.accent);
            }
            let name = Rect::new(row.x + 14, row.y, (value.x - row.x - 18).max(0), row.h);
            cx.text(if on { FontId::Bold } else { FontId::Sans }, t.size_body - 1.0, name, Align::Left,
                    if on { t.text } else { t.text_dim }, &self.rows[k].name);
            cx.canvas.fill_round_rect(value, 8, if on { t.surface } else { t.raised });
            let (left, mid) = value.take_left(SET_ARROW_W);
            let (right, mid) = mid.take_right(SET_ARROW_W);
            cx.text(FontId::Bold, t.size_title, left, Align::Center, t.text_dim, "‹");
            cx.text(FontId::Bold, t.size_title, right, Align::Center, t.text_dim, "›");
            cx.text(FontId::Bold, t.size_body - 1.0, mid, Align::Center, if on { t.accent } else { t.text },
                    &self.rows[k].value);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let k = (0..self.rows.len()).find(|&k| self.row_rects(r, k).is_some_and(|(row, _)| row.contains(x, y)))?;
        let (_, value) = self.row_rects(r, k)?;
        self.selected = k;
        if value.contains(x, y) && x < value.x + SET_ARROW_W {
            Some(EventKind::Step { index: k, delta: -1 })
        } else if value.contains(x, y) && x >= value.right() - SET_ARROW_W {
            Some(EventKind::Step { index: k, delta: 1 })
        } else {
            Some(EventKind::Select(k))
        }
    }
}

// ---------------------------------------------------------------- info card

/// A small card with a title and one line, such as what the wheel does now.
pub struct InfoCard {
    pub title: String,
    pub text: String,
}

impl InfoCard {
    pub fn new(title: &str) -> InfoCard {
        InfoCard { title: title.to_string(), text: String::new() }
    }
}

impl Widget for InfoCard {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let inner = Rect::new(r.x + t.pad, r.y + 12, (r.w - 2 * t.pad).max(0), r.h - 24);
        let (head, body) = inner.take_top(22);
        cx.text(FontId::Bold, t.size_small - 1.0, head, Align::Left, t.text_dim, &self.title.to_uppercase());
        cx.text(FontId::Bold, t.size_body, body, Align::Left, t.text, &self.text);
    }
}

// ---------------------------------------------------------------- buttons

/// A row of buttons sharing the width. A tap on an enabled button `i`
/// gives [`EventKind::Tab`]`(i)`; the `primary` one is filled with the
/// accent colour.
pub struct ButtonRow {
    pub labels: Vec<String>,
    pub enabled: Vec<bool>,
    pub primary: Option<usize>,
}

const BUTTON_GAP: i32 = 8;

impl ButtonRow {
    pub fn new(labels: &[&str]) -> ButtonRow {
        ButtonRow { labels: labels.iter().map(|s| s.to_string()).collect(), enabled: vec![true; labels.len()], primary: None }
    }

    /// Button `i`'s rectangle. The primary button takes twice the room.
    pub fn button_rect(&self, r: Rect, i: usize) -> Rect {
        let n = self.labels.len().max(1) as i32;
        let shares = n + i32::from(self.primary.is_some_and(|p| p < self.labels.len()));
        let inner = r.inset(6);
        let unit = (inner.w - BUTTON_GAP * (n - 1)).max(0) / shares.max(1);
        let width = |k: usize| if self.primary == Some(k) { 2 * unit } else { unit };
        let x = inner.x + (0..i).map(|k| width(k) + BUTTON_GAP).sum::<i32>();
        let w = if i + 1 == self.labels.len() { inner.right() - x } else { width(i) };
        Rect::new(x, inner.y, w.max(0), inner.h)
    }
}

impl Widget for ButtonRow {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        for (i, label) in self.labels.iter().enumerate() {
            let b = self.button_rect(r, i);
            let enabled = self.enabled.get(i).copied().unwrap_or(false);
            let primary = self.primary == Some(i) && enabled;
            cx.canvas.fill_round_rect(b, 10, if primary { t.accent } else { t.raised });
            let color = if primary { t.bg } else if enabled { t.text } else { t.text_faint };
            cx.text(FontId::Bold, t.size_body - 1.0, b, Align::Center, color, label);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.labels.len()).find(|&i| self.button_rect(r, i).contains(x, y))?;
        self.enabled.get(i).copied().unwrap_or(false).then_some(EventKind::Tab(i))
    }
}

// ---------------------------------------------------------------- pad map

/// One pad on the [`PadMap`].
#[derive(Clone, Debug, PartialEq)]
pub struct PadCell {
    /// The pad's name on the MPC, such as "A5".
    pub label: String,
    /// What the pad is called; empty for a pad without a sound.
    pub name: String,
    /// A short line under the name, such as "3 layers · Loop".
    pub detail: String,
    /// The clip the pad launches, if any.
    pub clip: Option<String>,
    pub color: Color,
}

/// A kit's pads, four to a row as on the MPC (pad 1 bottom left), small
/// enough to sit beside the pad's settings. A tap on a pad picks it
/// ([`EventKind::Select`]); a tap on the pad already picked plays it while
/// the finger is down ([`EventKind::Pad`]).
pub struct PadMap {
    pub title: String,
    pub right: String,
    pub cells: Vec<PadCell>,
    pub selected: usize,
    /// Pads held down now (on the hardware or here).
    pub lit: Vec<bool>,
    held: Option<(u64, usize)>,
}

const MAP_COLS: usize = 4;
const MAP_GAP: i32 = 8;

impl PadMap {
    pub fn new(title: &str) -> PadMap {
        PadMap { title: title.to_string(), right: String::new(), cells: Vec::new(), selected: 0, lit: Vec::new(), held: None }
    }

    /// Pad `i`'s rectangle.
    pub fn cell_rect(&self, r: Rect, i: usize) -> Rect {
        let b = r.inset(14).take_top(34).1;
        let rows = self.cells.len().div_ceil(MAP_COLS).max(1) as i32;
        let cols = MAP_COLS as i32;
        let w = (b.w - MAP_GAP * (cols - 1)) / cols;
        let h = (b.h - MAP_GAP * (rows - 1)) / rows;
        let (col, row) = ((i % MAP_COLS) as i32, rows - 1 - (i / MAP_COLS) as i32);
        Rect::new(b.x + col * (w + MAP_GAP), b.y + row * (h + MAP_GAP), w.max(0), h.max(0))
    }
}

impl Widget for PadMap {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        for (i, c) in self.cells.iter().enumerate() {
            let b = self.cell_rect(r, i);
            let filled = !c.name.is_empty();
            let lit = self.lit.get(i).copied().unwrap_or(false);
            let fill = match (lit, filled) {
                (true, _) => c.color.mix(t.surface, 0.25),
                (false, true) => c.color.mix(t.surface, 0.8),
                (false, false) => t.raised,
            };
            cx.canvas.fill_round_rect(b, 10, fill);
            if filled {
                cx.canvas.fill_rect(Rect::new(b.x, b.y + 8, 4, b.h - 16), c.color);
            }
            if i == self.selected {
                cx.canvas.stroke_round_rect(b, 10, 3, t.accent);
            }
            let inner = b.inset_xy(10, 6);
            let (top, rest) = inner.take_top(20);
            cx.text(FontId::Bold, t.size_small - 1.0, top, Align::Left, t.text_dim, &c.label);
            if let Some(clip) = &c.clip {
                cx.text(FontId::Bold, t.size_small - 2.0, top, Align::Right, t.good, &format!("▶ {clip}"));
            }
            let (name, detail) = rest.take_top(rest.h / 2 + 4);
            if filled {
                cx.text(FontId::Bold, t.size_small, name, Align::Left, t.text, &c.name);
                cx.text(FontId::Sans, t.size_small - 2.0, detail, Align::Left, t.text_dim, &c.detail);
            } else {
                cx.text(FontId::Sans, t.size_small - 1.0, name, Align::Left, t.text_faint, "empty");
            }
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let k = (0..self.cells.len()).find(|&k| self.cell_rect(r, k).contains(x, y))?;
                if k == self.selected {
                    self.held = Some((finger, k));
                    Some(EventKind::Pad { index: k, down: true })
                } else {
                    self.selected = k;
                    Some(EventKind::Select(k))
                }
            }
            Touch::Up | Touch::Cancel => {
                let (_, k) = self.held.take_if(|(f, _)| *f == finger)?;
                Some(EventKind::Pad { index: k, down: false })
            }
            Touch::Move => None,
        }
    }
}
