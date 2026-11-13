//! The widgets the Synth tab adds to mpfree-ui's set. Like mpfree-ui's own,
//! they only show what the panel put in their fields and report touches;
//! the panel ([`crate::view`]) decides what a touch does.

use crate::draw::Mask;
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, Color, EventKind, PaintCx, Rect, Theme, Touch, Widget};
use std::f32::consts::PI;

// Panels build their tiles, cards and nodes without knowing the theme, so
// they pick each colour by what it means, written as the default theme's
// own colour for it (below). The widgets show the theme on screen's colour
// for that meaning in its place ([`themed`]), so the default theme looks
// exactly as written and every other theme gets its own colours.

/// Q-Link colours, 1 to 4: the default theme's first four track colours
/// (blue, green, amber, violet). On screen each is the theme's own
/// `track` colour 1 to 4.
pub const QLINK_COLORS: [Color; 4] = [
    Color::rgb(0x6EA8FE),
    Color::rgb(0x3DDC97),
    Color::rgb(0xFFC145),
    Color::rgb(0xD46CFF),
];
/// The default theme's `text`; on screen, the theme's `text`.
pub const TEXT: Color = Color::rgb(0xE9ECF2);
/// The default theme's `text_dim`; on screen, the theme's `text_dim`.
pub const TEXT_DIM: Color = Color::rgb(0x98A1B3);
/// The default theme's `text_faint`; on screen, the theme's `text_faint`.
pub const TEXT_FAINT: Color = Color::rgb(0x5F687B);
/// Text on a coloured box: the default theme's `on_accent`; on screen, the
/// theme's `on_accent`.
pub const ON_FILL: Color = Color::rgb(0x0B0D12);

/// A colour a panel put in a tile, card or node, as theme `t` shows it:
/// [`QLINK_COLORS`] become `t.track` 1 to 4, and [`TEXT`], [`TEXT_DIM`],
/// [`TEXT_FAINT`] and [`ON_FILL`] become the tokens they name. Their
/// transparency is kept. Any other colour comes back as it is.
pub fn themed(t: &Theme, c: Color) -> Color {
    let same = |a: Color| (a.r, a.g, a.b) == (c.r, c.g, c.b);
    let token = if let Some(i) = QLINK_COLORS.iter().position(|&q| same(q)) {
        t.track(i)
    } else if same(TEXT) {
        t.text
    } else if same(TEXT_DIM) {
        t.text_dim
    } else if same(TEXT_FAINT) {
        t.text_faint
    } else if same(ON_FILL) {
        t.on_accent
    } else {
        return c;
    };
    token.with_alpha((token.a as u32 * c.a as u32 / 255) as u8)
}

/// Matches the default theme's padding, so `touch` (which has no theme)
/// finds the same rectangles `paint` drew.
const PAD: i32 = 14;
/// A panel's title line and the space under it.
const HEAD: i32 = 34;

/// A knob's sweep: 270 degrees, from bottom left round to bottom right.
const A0: f32 = 0.75 * PI;
const A1: f32 = 2.25 * PI;

/// Draws a knob of diameter `d` centred at (cx, cy): the track, the value
/// arc (from the bottom left, or from the top for a bipolar value) and the
/// pointer dot.
#[allow(clippy::too_many_arguments)]
pub fn knob(
    cx: &mut PaintCx,
    x: f32,
    y: f32,
    d: f32,
    pos: f32,
    bipolar: bool,
    track: Color,
    color: Color,
) {
    let rad = d * 0.5 - 3.0;
    let width = (d / 9.0).clamp(3.0, 5.0);
    let bbox = Rect::new(
        (x - d * 0.5) as i32 - 2,
        (y - d * 0.5) as i32 - 2,
        d as i32 + 5,
        d as i32 + 5,
    );
    let mut m = Mask::new(bbox);
    m.arc(x, y, rad, A0, A1, width);
    m.draw(cx.canvas, track);
    let pos = pos.clamp(0.0, 1.0);
    let at = A0 + (A1 - A0) * pos;
    let from = if bipolar { 1.5 * PI } else { A0 };
    let (a, b) = if at < from { (at, from) } else { (from, at) };
    let mut m = Mask::new(bbox);
    if b - a > 0.01 {
        m.arc(x, y, rad, a, b, width);
    }
    m.draw(cx.canvas, color);
    let mut m = Mask::new(bbox);
    m.disc(x + rad * at.cos(), y + rad * at.sin(), width * 0.7);
    m.draw(cx.canvas, cx.theme.text);
}

// ---------------------------------------------------------------- title bar

/// The instrument's name and swatch, a line about it, and its page tabs on
/// the right. A tap on tab `i` gives `Tab(i)`.
pub struct TitleBar {
    pub swatch: Color,
    pub title: String,
    pub subtitle: String,
    pub tabs: Vec<String>,
    pub active: usize,
}

/// Width of one page tab.
pub const TAB_W: i32 = 92;

/// Below this width a tab gets a smaller font and a width by its name.
const NARROW_TAB: i32 = 70;

impl TitleBar {
    pub fn new() -> TitleBar {
        TitleBar {
            swatch: QLINK_COLORS[1],
            title: String::new(),
            subtitle: String::new(),
            tabs: Vec::new(),
            active: 0,
        }
    }

    /// The pill that holds the tabs, at the right of the bar.
    pub fn tabs_rect(&self, r: Rect) -> Rect {
        let inner = r.inset_xy(10, 8);
        let w = (TAB_W * self.tabs.len() as i32 + 8).min(inner.w * 2 / 3);
        inner.take_right(w).0
    }

    pub fn tab_rect(&self, r: Rect, i: usize) -> Rect {
        let pill = self.tabs_rect(r).inset(4);
        let n = self.tabs.len().max(1) as i32;
        let w = pill.w / n;
        if w >= NARROW_TAB {
            return Rect::new(
                pill.x + i as i32 * w,
                pill.y,
                if i as i32 == n - 1 {
                    pill.w - w * (n - 1)
                } else {
                    w
                },
                pill.h,
            );
        }
        // Many pages (FM has twelve): each tab gets room by its name's
        // length, so "Pressure" and "Kbd 1-2" stay whole beside "Op 1".
        let weight = |k: usize| self.tabs.get(k).map_or(1, |t| t.chars().count() as i32 + 3);
        let total: i32 = (0..self.tabs.len()).map(weight).sum::<i32>().max(1);
        let x0: i32 = (0..i).map(weight).sum();
        let left = pill.x + pill.w * x0 / total;
        let right = if i as i32 == n - 1 {
            pill.right()
        } else {
            pill.x + pill.w * (x0 + weight(i)) / total
        };
        Rect::new(left, pill.y, right - left, pill.h)
    }
}

impl Default for TitleBar {
    fn default() -> TitleBar {
        TitleBar::new()
    }
}

impl Widget for TitleBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let inner = r.inset_xy(PAD, 10);
        let sw = inner.h.min(40);
        cx.canvas.fill_round_rect(
            Rect::new(inner.x, inner.y + (inner.h - sw) / 2, sw, sw),
            10,
            t.content(self.swatch),
        );
        let pill = if self.tabs.is_empty() {
            Rect::new(r.right(), r.y, 0, 0)
        } else {
            self.tabs_rect(r)
        };
        let text = Rect::new(
            inner.x + sw + 14,
            inner.y,
            pill.x - inner.x - sw - 28,
            inner.h,
        );
        let (top, bottom) = text.take_top(text.h * 3 / 5);
        cx.text(
            FontId::Bold,
            t.size_title + 1.0,
            top,
            Align::Left,
            t.text,
            &self.title,
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            bottom,
            Align::Left,
            t.text_dim,
            &self.subtitle,
        );
        if self.tabs.is_empty() {
            return;
        }
        cx.canvas.fill_round_rect(pill, 12, t.bg);
        // Many pages (FM has twelve) make narrow tabs: a smaller font keeps
        // names like "Kbd 1-2" whole.
        let pill = self.tabs_rect(r).inset(4);
        let size = if pill.w / (self.tabs.len() as i32) < NARROW_TAB {
            t.size_small
        } else {
            t.size_body - 1.0
        };
        for (i, name) in self.tabs.iter().enumerate() {
            let c = self.tab_rect(r, i);
            let on = i == self.active;
            if on {
                cx.canvas.fill_round_rect(c, 9, t.raised);
            }
            let font = if on { FontId::Bold } else { FontId::Sans };
            cx.text(
                font,
                size,
                c,
                Align::Center,
                if on { t.text } else { t.text_dim },
                name,
            );
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.tabs.len()).find(|&i| self.tab_rect(r, i).contains(x, y))?;
        self.active = i;
        Some(EventKind::Tab(i))
    }
}

// ---------------------------------------------------------------- visuals

/// One curve on a card, in the unit box (x 0 to 1, y 0 at the bottom).
#[derive(Clone, Debug)]
pub struct Trace {
    pub pts: Vec<(f32, f32)>,
    pub color: Color,
    pub width: f32,
    /// A soft fill down to the bottom of the plot.
    pub fill: bool,
    pub dashed: bool,
}

/// One card of the visual strip: a title, a caption and its curves.
#[derive(Clone, Debug, Default)]
pub struct Card {
    pub title: String,
    pub caption: String,
    pub traces: Vec<Trace>,
    /// Faint vertical lines at these x positions, with an optional label
    /// along the bottom.
    pub grid: Vec<(f32, String)>,
    /// A faint line across the middle (for waves).
    pub midline: bool,
    /// A marker dot on the first curve, at this x.
    pub dot: Option<f32>,
    /// Vertical bars from the bottom, each 0 to 1 high, spread across the
    /// plot (live pressure per voice). An empty slot shows as a faint stub.
    pub bars: Vec<f32>,
    /// The bars' colour.
    pub bar_color: Option<Color>,
    /// A colour per bar, over `bar_color` (carriers and modulators).
    pub bar_colors: Vec<Color>,
    /// A label under each bar ("1" to "6").
    pub bar_labels: Vec<String>,
    /// Boxes drawn over the curves (an FM algorithm's operators). A tap on
    /// one gives `Select(k)`, `k` counted across the strip's cards.
    pub nodes: Vec<CardNode>,
    /// The card belongs to the page shown.
    pub active: bool,
}

/// A labelled box on a card, centred at `(x, y)` in the unit box.
#[derive(Clone, Debug)]
pub struct CardNode {
    pub x: f32,
    pub y: f32,
    pub label: String,
    pub fill: Color,
    pub text: Color,
    /// Drawn with a ring in this colour: the operator the page is about (on
    /// a keyboard page, the colour of its curve).
    pub ring: Option<Color>,
    /// Faded: switched off or silent.
    pub dim: bool,
    /// A feedback loop from this box back into node `k` of the same card
    /// (itself for a self loop).
    pub feedback: Option<usize>,
}

/// A card node's box, in pixels.
pub const NODE_W: i32 = 28;
pub const NODE_H: i32 = 15;
/// How far from a node's centre a tap still picks it (a box is small for a
/// finger).
const NODE_HIT: f32 = 22.0;

/// The strip of drawn visuals over the tiles. A tap on card `i` gives
/// `Tab(i)`.
pub struct Visuals {
    pub cards: Vec<Card>,
}

impl Visuals {
    pub fn new() -> Visuals {
        Visuals { cards: Vec::new() }
    }

    pub fn card_rect(&self, r: Rect, i: usize) -> Rect {
        let n = self.cards.len().max(1) as i32;
        let gap = 12;
        let w = (r.w - gap * (n - 1)) / n;
        Rect::new(
            r.x + i as i32 * (w + gap),
            r.y,
            if i as i32 == n - 1 {
                r.right() - (r.x + i as i32 * (w + gap))
            } else {
                w
            },
            r.h,
        )
    }

    /// Where a card's curves go: below its title, inside its padding.
    pub fn plot_rect(card: Rect) -> Rect {
        Rect::new(card.x + 12, card.y + 38, card.w - 24, card.h - 38 - 12)
    }
}

impl Default for Visuals {
    fn default() -> Visuals {
        Visuals::new()
    }
}

fn to_screen(p: Rect, pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    pts.iter()
        .map(|&(x, y)| {
            (
                p.x as f32 + x * p.w as f32,
                p.bottom() as f32 - 1.0 - y * (p.h - 2) as f32,
            )
        })
        .collect()
}

impl Widget for Visuals {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        for (i, card) in self.cards.iter().enumerate() {
            let c = self.card_rect(r, i);
            cx.panel(c);
            if card.active {
                cx.canvas
                    .stroke_round_rect(c, t.radius, 2, t.accent.with_alpha(160));
            }
            let head = Rect::new(c.x + 12, c.y + 8, c.w - 24, 24);
            cx.text(
                FontId::Bold,
                t.size_small,
                head,
                Align::Left,
                t.text_dim,
                &card.title.to_uppercase(),
            );
            let tw = cx.measure(FontId::Bold, t.size_small, &card.title.to_uppercase()) + 12;
            let cap = head.take_left(tw).1;
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                cap,
                Align::Right,
                t.text_faint,
                &card.caption,
            );
            let p = Visuals::plot_rect(c);
            for (x, label) in &card.grid {
                let gx = p.x + (x * p.w as f32) as i32;
                cx.canvas.fill_rect(Rect::new(gx, p.y, 1, p.h), t.outline);
                if !label.is_empty() {
                    let lr = Rect::new(gx + 4, p.bottom() - 16, 40, 14);
                    cx.text(FontId::Sans, 11.0, lr, Align::Left, t.text_faint, label);
                }
            }
            if card.midline {
                cx.canvas
                    .fill_rect(Rect::new(p.x, p.y + p.h / 2, p.w, 1), t.outline);
            }
            if !card.bars.is_empty() {
                let n = card.bars.len() as i32;
                let slot = (p.w / n).max(1);
                let bw = (slot * 2 / 3).clamp(2, 40);
                let labels = if card.bar_labels.is_empty() { 0 } else { 16 };
                let (under, bars) = p.take_bottom(labels);
                for (k, &b) in card.bars.iter().enumerate() {
                    let h = ((b.clamp(0.0, 1.0) * bars.h as f32).round() as i32).max(2);
                    let x = p.x + k as i32 * slot + (slot - bw) / 2;
                    let color = card
                        .bar_colors
                        .get(k)
                        .copied()
                        .or(card.bar_color)
                        .map_or(t.accent, |c| themed(&t, c));
                    let c = if b > 0.0 { color } else { t.outline };
                    cx.canvas
                        .fill_rect(Rect::new(x, bars.bottom() - h, bw, h), c);
                    if let Some(l) = card.bar_labels.get(k) {
                        let lr = Rect::new(p.x + k as i32 * slot, under.y + 2, slot, under.h - 2);
                        cx.text(FontId::Sans, 11.0, lr, Align::Center, t.text_faint, l);
                    }
                }
            }
            let clip = cx.canvas.clip();
            cx.canvas.set_clip(clip.intersect(p.inset_xy(-2, -2)));
            for tr in &card.traces {
                let pts = to_screen(p, &tr.pts);
                if tr.fill {
                    let mut m = Mask::new(p);
                    m.fill_under(&pts, p.bottom() as f32, 0.16);
                    m.draw(cx.canvas, themed(&t, tr.color));
                }
                let mut m = Mask::new(p.inset_xy(-2, -2));
                if tr.dashed {
                    m.dashed(&pts, tr.width, 6.0);
                } else {
                    m.polyline(&pts, tr.width);
                }
                m.draw(cx.canvas, themed(&t, tr.color));
            }
            if let (Some(x), Some(tr)) = (card.dot, card.traces.first()) {
                if let Some(&(_, y)) = tr
                    .pts
                    .iter()
                    .min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs()))
                {
                    let s = to_screen(p, &[(x, y)])[0];
                    let mut m = Mask::new(p.inset_xy(-6, -6));
                    m.disc(s.0, s.1, 4.5);
                    m.draw(cx.canvas, t.text);
                }
            }
            paint_nodes(cx, p, &card.nodes);
            cx.canvas.set_clip(clip);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.cards.len()).find(|&i| self.card_rect(r, i).contains(x, y))?;
        // A node near the finger, else the card.
        let p = Visuals::plot_rect(self.card_rect(r, i));
        let first: usize = self.cards[..i].iter().map(|c| c.nodes.len()).sum();
        let near = self.cards[i]
            .nodes
            .iter()
            .enumerate()
            .map(|(k, n)| {
                let (nx, ny) = to_screen(p, &[(n.x, n.y)])[0];
                (k, (nx - x as f32).hypot(ny - y as f32))
            })
            .filter(|&(_, d)| d <= NODE_HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        Some(match near {
            Some((k, _)) => EventKind::Select(first + k),
            None => EventKind::Tab(i),
        })
    }
}

/// A node's box on screen.
pub fn node_rect(plot: Rect, n: &CardNode) -> Rect {
    let (x, y) = to_screen(plot, &[(n.x, n.y)])[0];
    Rect::new(
        x.round() as i32 - NODE_W / 2,
        y.round() as i32 - NODE_H / 2,
        NODE_W,
        NODE_H,
    )
}

/// Draws a card's boxes and their feedback loops: a loop leaves a box's
/// right side, runs up past the box it feeds and comes down into its top.
fn paint_nodes(cx: &mut PaintCx, p: Rect, nodes: &[CardNode]) {
    let t = cx.theme.clone();
    for n in nodes {
        let Some(to) = n.feedback.and_then(|k| nodes.get(k)) else {
            continue;
        };
        let (a, b) = (node_rect(p, n), node_rect(p, to));
        let right = (a.right().max(b.right()) + 7) as f32;
        let (ay, top) = ((a.y + a.h / 2) as f32, (b.y - 5) as f32);
        let bx = (b.x + b.w / 2) as f32;
        let pts = [
            (a.right() as f32, ay),
            (right, ay),
            (right, top),
            (bx, top),
            (bx, b.y as f32),
        ];
        let mut m = Mask::new(p.inset_xy(-12, -12));
        m.polyline(&pts, 1.5);
        m.draw(cx.canvas, if n.dim { t.text_faint } else { t.text_dim });
    }
    for n in nodes {
        let b = node_rect(p, n);
        let (fill, text) = if n.dim {
            (t.raised, t.text_faint)
        } else {
            (themed(&t, n.fill), themed(&t, n.text))
        };
        if let Some(c) = n.ring {
            cx.canvas
                .stroke_round_rect(b.inset(-3), 7, 2, themed(&t, c));
        }
        cx.canvas.fill_round_rect(b, 5, fill);
        cx.text(FontId::Bold, 11.0, b, Align::Center, text, &n.label);
    }
}

// ---------------------------------------------------------------- tiles

#[derive(Clone, Debug, PartialEq)]
pub enum TileKind {
    /// A knob at `pos` (0 to 1). A bipolar knob fills from the top.
    Knob {
        pos: f32,
        bipolar: bool,
    },
    /// Named choices, `index` chosen. Up to four show as buttons; more show
    /// as the chosen name with arrows.
    Choice {
        names: Vec<String>,
        index: usize,
    },
    Toggle(bool),
    /// A live value the node reports; it can't be changed.
    Meter,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub label: String,
    pub value: String,
    pub unit: String,
    pub kind: TileKind,
    /// Q-Link 1 to 4 turns it (0 to 3).
    pub qlink: Option<usize>,
    pub color: Color,
}

struct Drag {
    finger: u64,
    tile: usize,
    y0: i32,
    pos0: f32,
}

/// The page's parameters as tiles, four to a row.
///
/// - A finger down on a tile selects it: `Select(i)`. The wheel then turns it.
/// - Dragging a knob up or down turns it: `Value(pos)` for the selected
///   tile, [`DRAG_PX`] pixels for the whole range.
/// - Lifting on a choice's button, or on an arrow, gives
///   `Step { index: i, delta }` (the change of the choice's index); lifting
///   on a switch gives `Step` with +1 or -1.
pub struct TileGrid {
    pub title: String,
    pub caption: String,
    pub tiles: Vec<Tile>,
    pub selected: Option<usize>,
    /// The wheel's push opened the selected tile: it is outlined more
    /// strongly while the wheel turns it.
    pub editing: bool,
    drag: Option<Drag>,
}

pub const COLS: usize = 4;
/// Pixels of drag for a knob's whole range.
pub const DRAG_PX: f32 = 320.0;
const TILE_GAP: i32 = 10;
/// Narrower choice buttons than this give way to arrows.
const MIN_BUTTON_W: i32 = 58;
const TILE_MAX_H: i32 = 96;

impl TileGrid {
    pub fn new(title: &str) -> TileGrid {
        TileGrid {
            title: title.to_string(),
            caption: String::new(),
            tiles: Vec::new(),
            selected: None,
            editing: false,
            drag: None,
        }
    }

    pub fn body(r: Rect) -> Rect {
        r.inset(PAD).take_top(HEAD).1
    }

    pub fn tile_rect(&self, r: Rect, i: usize) -> Rect {
        let body = TileGrid::body(r);
        let rows = self.tiles.len().div_ceil(COLS).max(1) as i32;
        let w = (body.w - TILE_GAP * (COLS as i32 - 1)) / COLS as i32;
        let h = ((body.h - TILE_GAP * (rows - 1)) / rows).min(TILE_MAX_H);
        let (row, col) = ((i / COLS) as i32, (i % COLS) as i32);
        Rect::new(
            body.x + col * (w + TILE_GAP),
            body.y + row * (h + TILE_GAP),
            w,
            h,
        )
    }

    /// The parts of a choice tile: its label line, then its buttons (or,
    /// for long lists, the previous arrow, the name and the next arrow).
    pub fn choice_rects(tile: Rect, n: usize) -> Vec<Rect> {
        let area = Rect::new(tile.x + 10, tile.y + 28, tile.w - 20, tile.h - 36);
        if n > 4 {
            let aw = 44.min(area.w / 4);
            return vec![
                Rect::new(area.x, area.y, aw, area.h),
                Rect::new(area.right() - aw, area.y, aw, area.h),
            ];
        }
        let (cols, rows) = if n == 4 && area.h >= 52 {
            (2, 2)
        } else {
            (n.max(1) as i32, 1)
        };
        // Too narrow for names like "Triangle": arrows instead.
        if (area.w - 6 * (cols - 1)) / cols < MIN_BUTTON_W {
            return TileGrid::choice_rects(tile, 5);
        }
        let (cw, ch) = (
            (area.w - 6 * (cols - 1)) / cols,
            (area.h - 4 * (rows - 1)) / rows,
        );
        (0..n as i32)
            .map(|k| {
                Rect::new(
                    area.x + (k % cols) * (cw + 6),
                    area.y + (k / cols) * (ch + 4),
                    cw,
                    ch,
                )
            })
            .collect()
    }

    fn hit(&self, r: Rect, x: i32, y: i32) -> Option<usize> {
        (0..self.tiles.len()).find(|&i| self.tile_rect(r, i).contains(x, y))
    }
}

impl Widget for TileGrid {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let head = r.inset(PAD).take_top(26).0;
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            &self.title.to_uppercase(),
        );
        let tw = cx.measure(FontId::Bold, t.size_small, &self.title.to_uppercase()) + 16;
        cx.text(
            FontId::Sans,
            t.size_small,
            head.take_left(tw).1,
            Align::Right,
            t.text_faint,
            &self.caption,
        );
        let fill = t.surface.mix(t.raised, 0.55);
        for (i, tile) in self.tiles.iter().enumerate() {
            let tr = self.tile_rect(r, i);
            let color = themed(&t, tile.color);
            let on = self.selected == Some(i);
            cx.canvas
                .fill_round_rect(tr, 12, if on && self.editing { t.raised } else { fill });
            if on {
                cx.canvas
                    .stroke_round_rect(tr, 12, if self.editing { 3 } else { 2 }, t.accent);
            }
            let label = tile.label.to_uppercase();
            match &tile.kind {
                TileKind::Knob { pos, bipolar } => {
                    let d = (tr.h - 22).clamp(28, 50) as f32;
                    let (kx, ky) = (
                        tr.x as f32 + 12.0 + d * 0.5,
                        tr.y as f32 + tr.h as f32 * 0.5,
                    );
                    knob(cx, kx, ky, d, *pos, *bipolar, t.outline, color);
                    let tx = tr.x + 12 + d as i32 + 12;
                    let text = Rect::new(tx, tr.y, tr.right() - tx - 8, tr.h);
                    value_lines(cx, text, &label, &tile.value, &tile.unit);
                }
                TileKind::Choice { names, index } => {
                    cx.text(
                        FontId::Bold,
                        t.size_small - 1.0,
                        Rect::new(tr.x + 12, tr.y + 6, tr.w - 24, 20),
                        Align::Left,
                        t.text_dim,
                        &label,
                    );
                    let parts = TileGrid::choice_rects(tr, names.len());
                    if parts.len() != names.len() || names.len() > 4 {
                        for (k, arrow) in ["‹", "›"].iter().enumerate() {
                            cx.canvas.fill_round_rect(parts[k], 8, t.raised);
                            cx.text(
                                FontId::Bold,
                                t.size_title,
                                parts[k],
                                Align::Center,
                                t.text,
                                arrow,
                            );
                        }
                        let mid = Rect::new(
                            parts[0].right() + 6,
                            parts[0].y,
                            parts[1].x - parts[0].right() - 12,
                            parts[0].h,
                        );
                        cx.text(
                            FontId::Bold,
                            t.size_title,
                            mid,
                            Align::Center,
                            t.text,
                            &tile.value,
                        );
                    } else {
                        for (k, name) in names.iter().enumerate() {
                            let on = k == *index;
                            cx.canvas.fill_round_rect(
                                parts[k],
                                8,
                                if on { color } else { t.raised },
                            );
                            let font = if on { FontId::Bold } else { FontId::Sans };
                            cx.text(
                                font,
                                t.size_small,
                                parts[k],
                                Align::Center,
                                if on { t.on_accent } else { t.text_dim },
                                name,
                            );
                        }
                    }
                }
                TileKind::Toggle(on) => {
                    let text = Rect::new(tr.x + 12, tr.y, tr.w - 90, tr.h);
                    value_lines(cx, text, &label, &tile.value, "");
                    let sw = Rect::new(tr.right() - 68, tr.y + tr.h / 2 - 15, 52, 30);
                    cx.canvas
                        .fill_round_rect(sw, 15, if *on { color } else { t.outline });
                    let kx = if *on { sw.right() - 26 } else { sw.x + 4 };
                    cx.canvas
                        .fill_round_rect(Rect::new(kx, sw.y + 4, 22, 22), 11, t.text);
                }
                TileKind::Meter => {
                    let text = Rect::new(tr.x + 12, tr.y, tr.w - 24, tr.h);
                    value_lines(cx, text, &label, &tile.value, &tile.unit);
                    let tag = Rect::new(tr.right() - 52, tr.y + 8, 42, 18);
                    cx.text(FontId::Sans, 12.0, tag, Align::Right, t.text_faint, "live");
                }
            }
            if let Some(q) = tile.qlink {
                let badge = Rect::new(tr.right() - 40, tr.bottom() - 26, 32, 20);
                cx.canvas.fill_round_rect(badge, 6, t.track(q % 4));
                cx.text(
                    FontId::Bold,
                    12.0,
                    badge,
                    Align::Center,
                    t.on_accent,
                    &format!("Q{}", q + 1),
                );
            }
        }
        if self.tiles.is_empty() {
            let body = TileGrid::body(r);
            cx.text(
                FontId::Sans,
                t.size_body,
                body.take_top(40).0,
                Align::Left,
                t.text_faint,
                "no parameters",
            );
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let i = self.hit(r, x, y)?;
                self.selected = Some(i);
                self.drag = match self.tiles[i].kind {
                    TileKind::Knob { pos, .. } => Some(Drag {
                        finger,
                        tile: i,
                        y0: y,
                        pos0: pos,
                    }),
                    _ => None,
                };
                Some(EventKind::Select(i))
            }
            Touch::Move => {
                let d = self.drag.as_ref().filter(|d| d.finger == finger)?;
                let (i, pos) = (
                    d.tile,
                    (d.pos0 + (d.y0 - y) as f32 / DRAG_PX).clamp(0.0, 1.0),
                );
                if let TileKind::Knob { pos: p, .. } = &mut self.tiles[i].kind {
                    if (*p - pos).abs() < 1e-6 {
                        return None;
                    }
                    *p = pos;
                }
                self.selected = Some(i);
                Some(EventKind::Value(pos))
            }
            Touch::Up => {
                if self.drag.as_ref().is_some_and(|d| d.finger == finger) {
                    self.drag = None;
                    return None;
                }
                let i = self.hit(r, x, y)?;
                if self.selected != Some(i) {
                    return None;
                }
                let tr = self.tile_rect(r, i);
                match &self.tiles[i].kind {
                    TileKind::Choice { names, index } => {
                        let parts = TileGrid::choice_rects(tr, names.len());
                        let k = parts.iter().position(|p| p.contains(x, y))?;
                        let arrows = parts.len() != names.len() || names.len() > 4;
                        let delta = if arrows {
                            if k == 0 {
                                -1
                            } else {
                                1
                            }
                        } else {
                            k as i32 - *index as i32
                        };
                        (delta != 0).then_some(EventKind::Step { index: i, delta })
                    }
                    TileKind::Toggle(on) => Some(EventKind::Step {
                        index: i,
                        delta: if *on { -1 } else { 1 },
                    }),
                    _ => None,
                }
            }
            Touch::Cancel => {
                self.drag = None;
                None
            }
        }
    }
}

/// A small upper-case label over a big value and its unit.
fn value_lines(cx: &mut PaintCx, r: Rect, label: &str, value: &str, unit: &str) {
    let t = cx.theme.clone();
    let mid = r.y + r.h / 2;
    cx.text(
        FontId::Bold,
        t.size_small - 1.0,
        Rect::new(r.x, mid - 24, r.w, 18),
        Align::Left,
        t.text_dim,
        label,
    );
    let vr = Rect::new(r.x, mid, r.w, 26);
    let w = cx.text(
        FontId::Bold,
        t.size_title + 2.0,
        vr,
        Align::Left,
        t.text,
        value,
    );
    if !unit.is_empty() {
        let ur = Rect::new(r.x + w + 5, mid + 3, (r.w - w - 5).max(0), 24);
        cx.text(
            FontId::Sans,
            t.size_small,
            ur,
            Align::Left,
            t.text_dim,
            unit,
        );
    }
}

// ---------------------------------------------------------------- Q-Link strip

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QCell {
    pub label: String,
    pub value: String,
    pub pos: f32,
    pub bipolar: bool,
}

/// What the four Q-Links and the wheel turn on this page. A tap on Q-Link
/// cell `i` gives `Select(i)`.
pub struct QLinkStrip {
    pub cells: Vec<QCell>,
    pub wheel_label: String,
    pub wheel_value: String,
}

/// Width of the wheel's cell at the right.
pub const WHEEL_W: i32 = 260;

impl QLinkStrip {
    pub fn new() -> QLinkStrip {
        QLinkStrip {
            cells: Vec::new(),
            wheel_label: "Wheel".to_string(),
            wheel_value: String::new(),
        }
    }

    pub fn cell_rect(r: Rect, i: usize) -> Rect {
        let gap = 12;
        if i >= 4 {
            return r.take_right(WHEEL_W).0;
        }
        let area = r.take_right(WHEEL_W + gap).1;
        let w = (area.w - 3 * gap) / 4;
        Rect::new(area.x + i as i32 * (w + gap), r.y, w, r.h)
    }
}

impl Default for QLinkStrip {
    fn default() -> QLinkStrip {
        QLinkStrip::new()
    }
}

impl Widget for QLinkStrip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        for i in 0..4 {
            let color = t.track(i);
            let c = QLinkStrip::cell_rect(r, i);
            cx.panel(c);
            let Some(cell) = self.cells.get(i) else {
                let lr = Rect::new(c.x + 16, c.y, c.w - 32, c.h);
                cx.text(
                    FontId::Bold,
                    t.size_small - 1.0,
                    lr,
                    Align::Left,
                    t.text_faint,
                    &format!("Q-LINK {} · NOT USED", i + 1),
                );
                continue;
            };
            let d = (c.h - 24).clamp(26, 44) as f32;
            let (kx, ky) = (c.x as f32 + 14.0 + d * 0.5, c.y as f32 + c.h as f32 * 0.5);
            knob(cx, kx, ky, d, cell.pos, cell.bipolar, t.outline, color);
            let tx = c.x + 14 + d as i32 + 14;
            let text = Rect::new(tx, c.y, c.right() - tx - 10, c.h);
            let mid = text.y + text.h / 2;
            let label = format!("Q-LINK {} · {}", i + 1, cell.label.to_uppercase());
            cx.text(
                FontId::Bold,
                12.0,
                Rect::new(text.x, mid - 20, text.w, 16),
                Align::Left,
                t.text_dim,
                &label,
            );
            cx.text(
                FontId::Bold,
                t.size_title,
                Rect::new(text.x, mid + 1, text.w, 24),
                Align::Left,
                t.text,
                &cell.value,
            );
        }
        let w = QLinkStrip::cell_rect(r, 4);
        cx.panel(w);
        let text = w.inset_xy(18, 0);
        let mid = text.y + text.h / 2;
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(text.x, mid - 20, text.w, 16),
            Align::Left,
            t.text_dim,
            &self.wheel_label.to_uppercase(),
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            Rect::new(text.x, mid + 1, text.w, 24),
            Align::Left,
            t.text,
            &self.wheel_value,
        );
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        (0..self.cells.len().min(4))
            .find(|&i| QLinkStrip::cell_rect(r, i).contains(x, y))
            .map(EventKind::Select)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_colours_are_the_default_themes_own() {
        let d = Theme::dark();
        assert_eq!(
            QLINK_COLORS,
            [d.track[0], d.track[1], d.track[2], d.track[3]]
        );
        assert_eq!(
            [TEXT, TEXT_DIM, TEXT_FAINT, ON_FILL],
            [d.text, d.text_dim, d.text_faint, d.on_accent]
        );
        // So the default theme shows every colour as the panel wrote it.
        let other = Color::rgb(0x123456);
        for c in QLINK_COLORS
            .into_iter()
            .chain([TEXT, TEXT_DIM, TEXT_FAINT, ON_FILL, other])
        {
            assert_eq!(themed(&d, c), c);
            assert_eq!(themed(&d, c.with_alpha(90)), c.with_alpha(90));
        }
    }

    #[test]
    fn other_themes_show_their_own_tokens() {
        for name in ["console", "synthwave", "spaceship"] {
            let t = Theme::builtin(name).unwrap();
            for (i, &q) in QLINK_COLORS.iter().enumerate() {
                assert_eq!(themed(&t, q), t.track[i], "{name} Q{}", i + 1);
            }
            assert_eq!(themed(&t, TEXT), t.text);
            assert_eq!(themed(&t, TEXT_DIM), t.text_dim);
            assert_eq!(themed(&t, TEXT_FAINT), t.text_faint);
            assert_eq!(themed(&t, ON_FILL), t.on_accent);
            // Transparency is kept.
            assert_eq!(themed(&t, TEXT.with_alpha(51)), t.text.with_alpha(51));
            // A colour with no meaning here (a kit's) is left alone.
            let kit = Color::rgb(0xF5A524);
            assert_eq!(themed(&t, kit), kit);
        }
    }
}
