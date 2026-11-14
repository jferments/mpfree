//! The effect picker: a category browser (Jay, 14 Nov). Sections down the
//! left (Favourites, Recent, then each category with how many effects it
//! holds), the section's effects as tiles on the right (name, a line
//! about it, a star, how many presets it has). A tap on a tile adds the
//! effect at its defaults; a hold (half a second), a tap on its presets
//! line or the wheel's push opens its presets, and a tap on one adds the
//! effect with it. The wheel moves through the tiles and SHIFT + wheel
//! through the sections.
//!
//! What the sections hold, their counts, the presets and the favourites
//! and recents come from `mpfree_fx::picker`; this module only lays them
//! out and draws them. The picker takes the effect and Modulate columns
//! while it is open.

use std::time::{Duration, Instant};

use mpfree_fx::picker::{self, Section};
use mpfree_ui::{Align, FontId, PaintCx, Rect};

use crate::state::{ChainState, Pick};

/// The sections' column.
pub const SECTION_W: i32 = 196;
const SECTION_H: i32 = 44;
const GAP: i32 = 8;
/// An effect tile's height (it doesn't shrink as effects are added; a
/// section that doesn't fit pages).
pub const TILE_H: i32 = 120;
const TILE_MIN_W: i32 = 170;
const PRESET_H: i32 = 64;
const PAGER_H: i32 = 44;
/// The star's square in a tile's bottom right corner.
const STAR: i32 = 44;
/// The presets line's height along a tile's bottom, as tall as the star.
const CHIP_H: i32 = 44;
/// How long a finger stays on a tile to open its presets.
pub const HOLD: Duration = Duration::from_millis(500);

/// What the picker shows beyond the chain's state.
#[derive(Clone, Debug, Default)]
pub struct PickerView {
    /// The section picked (None: the one it opens on, see
    /// [`picker::first_section`]).
    pub section: Option<Section>,
    pub page: usize,
    /// The tile the wheel is on (an index into the section's effects, or
    /// into the presets list while it shows).
    pub highlight: Option<usize>,
    /// The effect whose presets show.
    pub presets: Option<&'static str>,
    /// A finger down on a tile, a preset row or a presets line: the
    /// finger, what it is on, since when. It acts when the finger lifts
    /// on it (or, on a tile held past [`HOLD`], opens the presets while
    /// the finger is still down).
    pub press: Option<(u64, Hit, Instant)>,
}

impl PickerView {
    /// Back to the section's first page, nothing highlighted (when the
    /// picker opens). The section stays.
    pub fn reopen(&mut self) {
        let section = self.section;
        *self = PickerView {
            section,
            ..PickerView::default()
        };
    }

    /// The sections shown and the index of the current one.
    pub fn current(&self, st: &ChainState) -> (Vec<Section>, usize) {
        let all = st.picks();
        let offered = offered(&all);
        let list = picker::sections(all.iter().map(|p| p.category));
        let want = self
            .section
            .filter(|s| list.contains(s))
            .unwrap_or_else(|| picker::first_section(&list, &offered, &st.shelf));
        let at = list.iter().position(|&s| s == want).unwrap_or(0);
        (list, at)
    }

    /// The current section's effects, in the order its tiles show.
    pub fn picks(&self, st: &ChainState) -> Vec<Pick> {
        let (list, at) = self.current(st);
        section_picks(st, list[at])
    }

    /// What the tiles show: the section's effects, or the open presets
    /// list ("Defaults" first).
    fn items(&self, st: &ChainState) -> usize {
        match self.presets {
            Some(id) => 1 + picker::preset_rows(id).len(),
            None => self.picks(st).len(),
        }
    }
}

fn offered(all: &[Pick]) -> Vec<(&str, mpfree_fx::Category)> {
    all.iter().map(|p| (p.id, p.category)).collect()
}

/// A section's effects, in the order its tiles show.
pub fn section_picks(st: &ChainState, s: Section) -> Vec<Pick> {
    let all = st.picks();
    picker::members(s, &offered(&all), &st.shelf)
        .into_iter()
        .map(|i| all[i])
        .collect()
}

/// Where the picker's parts go.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerLayout {
    /// The section rows and the current one's index.
    pub sections: Vec<Rect>,
    pub section: usize,
    /// The tiles on this page; tile `k` is item `first + k` (an effect of
    /// the section, or a preset row with "Defaults" first).
    pub tiles: Vec<Rect>,
    pub first: usize,
    /// Each effect tile's star and presets line (none on preset rows, or
    /// for an effect with no presets).
    pub stars: Vec<Rect>,
    pub chips: Vec<Option<Rect>>,
    pub page: usize,
    pub pages: usize,
    pub prev: Option<Rect>,
    pub next: Option<Rect>,
    /// The presets list's way back to the effects.
    pub back: Option<Rect>,
    /// The tiles' area.
    pub grid: Rect,
}

/// What a touch at a point is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Section(usize),
    Star(usize),
    Chip(usize),
    /// An effect tile (an index into the section's effects) or a preset
    /// row (an index into the presets list, 0 being "Defaults").
    Tile(usize),
    Prev,
    Next,
    Back,
}

/// Lays the picker out in `body` (under its heading).
pub fn layout(body: Rect, st: &ChainState, pv: &PickerView) -> PickerLayout {
    let (list, section) = pv.current(st);
    let (left, grid) = body.take_left(SECTION_W);
    let grid = grid.take_left(12).1;
    let sections = (0..list.len() as i32)
        .map(|i| Rect::new(left.x, left.y + i * (SECTION_H + 6), left.w, SECTION_H))
        .collect();
    let whole = grid;
    let mut grid = grid;
    let back = pv.presets.map(|_| {
        let (top, rest) = grid.take_top(44 + GAP);
        grid = rest;
        Rect::new(top.x, top.y, 132, 44)
    });
    let n = pv.items(st);
    let tile_h = if pv.presets.is_some() {
        PRESET_H
    } else {
        TILE_H
    };
    let cols = ((grid.w + GAP) / (TILE_MIN_W + GAP)).clamp(1, 3);
    let fit = |h: i32| ((h + GAP) / (tile_h + GAP)).max(1) * cols;
    let (per, pager) = if n as i32 <= fit(grid.h) {
        (fit(grid.h), false)
    } else {
        (fit(grid.h - PAGER_H - GAP), true)
    };
    let per = per as usize;
    let pages = n.div_ceil(per).max(1);
    let page = pv.highlight.map_or(pv.page, |h| h / per).min(pages - 1);
    let first = page * per;
    let w = (grid.w - (cols - 1) * GAP) / cols;
    let mut tiles = Vec::new();
    let (mut stars, mut chips) = (Vec::new(), Vec::new());
    let picks = pv.picks(st);
    for k in 0..(n - first).min(per) {
        let (c, r) = (k as i32 % cols, k as i32 / cols);
        let t = Rect::new(
            grid.x + c * (w + GAP),
            grid.y + r * (tile_h + GAP),
            w,
            tile_h,
        );
        tiles.push(t);
        if pv.presets.is_none() {
            stars.push(Rect::new(t.right() - STAR, t.bottom() - STAR, STAR, STAR));
            let has = !picker::preset_rows(picks[first + k].id).is_empty();
            let w = (t.w - STAR).min(150);
            chips.push(has.then(|| Rect::new(t.x, t.bottom() - CHIP_H, w, CHIP_H)));
        }
    }
    let (prev, next) = if pager {
        let y = grid.bottom() - PAGER_H;
        (
            Some(Rect::new(grid.x, y, 96, PAGER_H)),
            Some(Rect::new(grid.right() - 96, y, 96, PAGER_H)),
        )
    } else {
        (None, None)
    };
    PickerLayout {
        sections,
        section,
        tiles,
        first,
        stars,
        chips,
        page,
        pages,
        prev,
        next,
        back,
        grid: whole,
    }
}

/// What the point (x, y) is on.
pub fn hit(l: &PickerLayout, x: i32, y: i32) -> Option<Hit> {
    if let Some(i) = l.sections.iter().position(|r| r.contains(x, y)) {
        return Some(Hit::Section(i));
    }
    if l.back.is_some_and(|b| b.contains(x, y)) {
        return Some(Hit::Back);
    }
    if l.prev.is_some_and(|b| b.contains(x, y)) {
        return Some(Hit::Prev);
    }
    if l.next.is_some_and(|b| b.contains(x, y)) {
        return Some(Hit::Next);
    }
    let k = l.tiles.iter().position(|t| t.contains(x, y))?;
    if l.stars.get(k).is_some_and(|s| s.contains(x, y)) {
        return Some(Hit::Star(l.first + k));
    }
    if l.chips
        .get(k)
        .copied()
        .flatten()
        .is_some_and(|c| c.contains(x, y))
    {
        return Some(Hit::Chip(l.first + k));
    }
    Some(Hit::Tile(l.first + k))
}

/// Draws the picker (the heading is the caller's).
pub fn paint(cx: &mut PaintCx, l: &PickerLayout, st: &ChainState, pv: &PickerView) {
    let t = cx.theme.clone();
    let (list, _) = pv.current(st);
    let all = st.picks();
    let counts = picker::counts(&list, &offered(&all), &st.shelf);
    for (i, (r, s)) in l.sections.iter().zip(&list).enumerate() {
        let on = i == l.section;
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
            cx.canvas
                .fill_round_rect(Rect::new(r.x, r.y + 10, 3, r.h - 20), 1, t.accent);
        }
        let inner = r.inset_xy(14, 0);
        let (count, name) = inner.take_right(30);
        cx.text(
            if on { FontId::Bold } else { FontId::Sans },
            t.size_small,
            name,
            Align::Left,
            if on { t.text } else { t.text_dim },
            s.name(),
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            count,
            Align::Right,
            t.text_faint,
            &counts[i].to_string(),
        );
    }
    let picks = pv.picks(st);
    if let (Some(id), Some(b)) = (pv.presets, l.back) {
        cx.canvas.fill_round_rect(b, 9, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            b,
            Align::Center,
            t.text,
            "‹  Effects",
        );
        let name = picks.iter().find(|p| p.id == id).map_or(id, |p| p.name);
        let label = Rect::new(b.right() + 14, b.y, 400, b.h);
        cx.text(
            FontId::Bold,
            t.size_body,
            label,
            Align::Left,
            t.text,
            &format!("{name} presets"),
        );
        let presets = picker::preset_rows(id);
        for (k, r) in l.tiles.iter().enumerate() {
            let i = l.first + k;
            let lit = pv.highlight == Some(i);
            tile_back(cx, *r, lit);
            // A family's presets name the mode they play in.
            let (title, sub) = match i.checked_sub(1).and_then(|i| presets.get(i)) {
                Some(p) => (p.name, p.mode),
                None => ("Defaults", "As it starts"),
            };
            let inner = r.inset_xy(14, 0);
            if sub.is_empty() {
                cx.text(FontId::Bold, t.size_body, inner, Align::Left, t.text, title);
            } else {
                let (a, b) = inner.inset_xy(0, 10).take_top(24);
                cx.text(FontId::Bold, t.size_body, a, Align::Left, t.text, title);
                cx.text(FontId::Sans, t.size_small, b, Align::Left, t.text_dim, sub);
            }
        }
    } else if picks.is_empty() {
        let (a, _) = l.grid.take_top(80);
        let empty = match pv.current(st) {
            (list, at) if list[at] == Section::Favourites => {
                "No favourites yet: tap the star on an effect"
            }
            (list, at) if list[at] == Section::Recent => "Effects you add show here",
            _ => "Nothing here for this strip",
        };
        cx.text(FontId::Sans, t.size_body, a, Align::Left, t.text_dim, empty);
    } else {
        for (k, r) in l.tiles.iter().enumerate() {
            let p = &picks[l.first + k];
            let lit = pv.highlight == Some(l.first + k);
            tile_back(cx, *r, lit);
            let inner = r.inset_xy(14, 10);
            let (name, rest) = inner.take_top(26);
            cx.text(FontId::Bold, t.size_body, name, Align::Left, t.text, p.name);
            let fav = st.shelf.is_favourite(p.id);
            let s = l.stars[k];
            star(
                cx,
                (s.x as f32 + s.w as f32 * 0.5, s.y as f32 + s.h as f32 * 0.5),
                9.5,
                fav,
                if fav { t.accent } else { t.text_faint },
            );
            let size = t.size_small - 1.0;
            for (j, line) in crate::view::wrap(cx, size, p.about, rest.w)
                .iter()
                .take(2)
                .enumerate()
            {
                cx.text(
                    FontId::Sans,
                    size,
                    Rect::new(rest.x, rest.y + 4 + j as i32 * 19, rest.w, 19),
                    Align::Left,
                    t.text_dim,
                    line,
                );
            }
            if let Some(c) = l.chips[k] {
                let n = picker::preset_rows(p.id).len();
                cx.text(
                    FontId::Sans,
                    size,
                    c.inset_xy(14, 0),
                    Align::Left,
                    t.accent,
                    &format!("{n} preset{} ›", if n == 1 { "" } else { "s" }),
                );
            }
        }
    }
    if let (Some(p), Some(n)) = (l.prev, l.next) {
        for (r, label, ok) in [(p, "‹", l.page > 0), (n, "›", l.page + 1 < l.pages)] {
            cx.canvas.fill_round_rect(
                r,
                9,
                if ok {
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
                if ok { t.text } else { t.text_faint },
                label,
            );
        }
        let mid = Rect::new(p.right(), p.y, n.x - p.right(), p.h);
        cx.text(
            FontId::Sans,
            t.size_small,
            mid,
            Align::Center,
            t.text_dim,
            &format!("Page {} of {}", l.page + 1, l.pages),
        );
    }
}

fn tile_back(cx: &mut PaintCx, r: Rect, lit: bool) {
    let t = cx.theme.clone();
    if lit {
        cx.canvas.fill_round_rect(r, 10, t.accent);
        cx.canvas.fill_round_rect(r.inset(2), 9, t.raised);
    } else {
        cx.canvas.fill_round_rect(r, 10, t.raised);
    }
}

/// A five-pointed star at `c`, `radius` to its points: filled for a
/// favourite, outlined otherwise.
fn star(cx: &mut PaintCx, c: (f32, f32), radius: f32, filled: bool, col: mpfree_ui::Color) {
    use mpfree_synth_ui::draw::Mask;
    let pts: Vec<(f32, f32)> = (0..10)
        .map(|i| {
            let a = std::f32::consts::PI * (i as f32 / 5.0 - 0.5);
            let r = if i % 2 == 0 { radius } else { radius * 0.45 };
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect();
    let b = radius.ceil() as i32 + 2;
    let mut m = Mask::new(Rect::new(c.0 as i32 - b, c.1 as i32 - b, 2 * b, 2 * b));
    if filled {
        // Four by four samples a pixel, inside by the crossing rule.
        let inside = |x: f32, y: f32| {
            let mut odd = false;
            for k in 0..pts.len() {
                let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                    odd = !odd;
                }
            }
            odd
        };
        for py in 0..m.r.h {
            for px in 0..m.r.w {
                let mut n = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let x = (m.r.x + px) as f32 + (sx as f32 + 0.5) / 4.0;
                        let y = (m.r.y + py) as f32 + (sy as f32 + 0.5) / 4.0;
                        n += inside(x, y) as u32;
                    }
                }
                m.px[(py * m.r.w + px) as usize] = (n * 255 / 16) as u8;
            }
        }
    } else {
        let mut ring = pts.clone();
        ring.push(pts[0]);
        m.polyline(&ring, 1.6);
    }
    m.draw(cx.canvas, col);
}
