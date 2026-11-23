//! The patch screens (live shows C1, layout spec 4q): what the keys play.
//!
//! - [`paint_keys_play`]: KEYS PLAY on Play's transport bar, the patch
//!   playing and a strip of its zones, while the song has patches.
//! - [`PatchList`]: the song's patches as tiles in place of Play's view; a
//!   tap switches at once, Edit opens the editor, New patch starts one
//!   from a layout.
//! - [`PatchEditor`]: one patch's zones as coloured bars over a keyboard,
//!   with the picked zone's card and the patch's own settings below.
//!
//! The widgets only draw and turn touches into events; the screen's
//! [`crate::patch_screen`] fills them in and makes the edits.

use std::cell::RefCell;

use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The list's widget and page.
pub const LIST: &str = "patch-list";
pub const LIST_PAGE: &str = "Play · patches";
/// The editor's widget and page.
pub const EDIT: &str = "patch-edit";
pub const EDIT_PAGE: &str = "Play · patch";

/// The keys the strips and the editor's keyboard show: C1 to C7.
pub const SHOWN_LOW: u8 = 24;
pub const SHOWN_HIGH: u8 = 96;

/// One zone as the screens draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneLook {
    /// Its instrument's name ("Grand"), or the words it names while that
    /// isn't in the song ("Track 7").
    pub name: String,
    pub color: Color,
    pub low: u8,
    pub high: u8,
    pub vel_low: u8,
    pub vel_high: u8,
    /// Its keys in words: "C3–C7", "up to B2", "every key".
    pub range: String,
    /// Its instrument isn't in the song: hatched red, its keys play
    /// nothing.
    pub missing: bool,
    /// Pedals reach it (off: a crossed pedal on its bar).
    pub pedals: bool,
}

/// The rows each zone takes on a strip: a zone over keys another already
/// has goes on the next row (two layered zones are two thin lines).
fn strip_rows(zones: &[ZoneLook]) -> (Vec<usize>, usize) {
    let mut rows: Vec<usize> = Vec::with_capacity(zones.len());
    for (i, z) in zones.iter().enumerate() {
        let mut row = 0;
        while zones[..i]
            .iter()
            .zip(&rows)
            .any(|(o, &r)| r == row && o.low <= z.high && z.low <= o.high)
        {
            row += 1;
        }
        rows.push(row);
    }
    let n = rows.iter().max().map_or(1, |m| m + 1);
    (rows, n)
}

/// Where key `k`'s left edge falls across `r`, over [`SHOWN_LOW`] to
/// [`SHOWN_HIGH`] (by semitones, as a strip's scale).
fn key_x(r: Rect, k: i32) -> i32 {
    let span = i32::from(SHOWN_HIGH - SHOWN_LOW) + 1;
    let k = (k - i32::from(SHOWN_LOW)).clamp(0, span);
    r.x + k * r.w / span
}

/// A zone's bar across `r` (from its low key to its high key, within C1
/// to C7).
fn bar_across(r: Rect, low: u8, high: u8) -> Option<(i32, i32)> {
    if high < SHOWN_LOW || low > SHOWN_HIGH {
        return None;
    }
    let x0 = key_x(r, i32::from(low));
    let x1 = key_x(r, i32::from(high) + 1);
    Some((x0, (x1 - x0).max(2)))
}

/// Red hatching over `r`: a missing instrument.
fn hatch(cx: &mut PaintCx, r: Rect, ink: Color) {
    let mut y = r.y;
    while y < r.bottom() {
        let h = 2.min(r.bottom() - y);
        let off = (y - r.y) % 8;
        let mut x = r.x - 8 + off;
        while x < r.right() {
            let a = x.max(r.x);
            let b = (x + 3).min(r.right());
            if b > a {
                cx.canvas.fill_rect(Rect::new(a, y, b - a, h), ink);
            }
            x += 8;
        }
        y += 2;
    }
}

/// The zones as thin bars across `r` (C1 to C7), each in its
/// instrument's colour, layered zones on rows of their own; a missing
/// instrument hatched red.
pub fn paint_strip(cx: &mut PaintCx, r: Rect, zones: &[ZoneLook]) {
    let t = cx.theme.clone();
    let (rows, n) = strip_rows(zones);
    let n = n as i32;
    let gap = if r.h / n >= 4 { 1 } else { 0 };
    let h = ((r.h - gap * (n - 1)) / n).max(1);
    for (z, &row) in zones.iter().zip(&rows) {
        let Some((x, w)) = bar_across(r, z.low, z.high) else {
            continue;
        };
        let b = Rect::new(x, r.y + row as i32 * (h + gap), w, h);
        if z.missing {
            cx.canvas.fill_rect(b, t.bad.mix(t.surface, 0.6));
            hatch(cx, b, t.bad);
        } else {
            cx.canvas
                .fill_round_rect(b, (h / 2).min(2), t.swatch(z.color).fill);
        }
    }
}

/// The zones as the Show view's keyboard lines draw them (live shows
/// C1): where each lies across C1 to C7, in thousandths, on the rows
/// [`paint_strip`] gives it. A zone past the strip's keys is left out.
pub fn strip_bars(zones: &[ZoneLook]) -> Vec<mpfree_time_ui::show::KeyBar> {
    let (rows, n) = strip_rows(zones);
    let scale = Rect::new(0, 0, 1000, 1);
    zones
        .iter()
        .zip(&rows)
        .filter_map(|(z, &row)| {
            let (x, w) = bar_across(scale, z.low, z.high)?;
            Some(mpfree_time_ui::show::KeyBar {
                from: x as u16,
                to: (x + w).min(1000) as u16,
                row: row as u8,
                rows: n as u8,
                color: z.color,
                missing: z.missing,
            })
        })
        .collect()
}

// ------------------------------------------------------------- KEYS PLAY

/// What KEYS PLAY shows on Play's transport bar (layout spec 4q).
#[derive(Clone, Debug, PartialEq)]
pub struct KeysPlay {
    /// "3 · Chorus split", or "Keys as routed".
    pub line: String,
    /// The patch playing's zones, for the strip under the line.
    pub zones: Vec<ZoneLook>,
    /// A hand pick waiting for the hands: "Chorus split → Lead".
    pub waiting: Option<String>,
    /// An instrument of the patch playing is missing: a red dot before
    /// its name.
    pub missing: bool,
}

/// KEYS PLAY's width: as wide as its line, within these.
pub const KEYS_MIN_W: i32 = 160;
pub const KEYS_MAX_W: i32 = 200;

impl KeysPlay {
    /// The words the chip's line shows.
    pub fn text(&self) -> &str {
        self.waiting.as_deref().unwrap_or(&self.line)
    }
}

/// KEYS PLAY in `r`: "KEYS PLAY" over the patch and a 6 px strip of its
/// zones; while a pick waits, both names and "when your hands are up",
/// outlined amber.
pub fn paint_keys_play(cx: &mut PaintCx, r: Rect, k: &KeysPlay) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, t.round(10), t.raised);
    if k.waiting.is_some() {
        cx.canvas.stroke_round_rect(r, t.round(10), 2, t.warn);
    }
    let inner = Rect::new(r.x + 12, r.y + 4, r.w - 22, r.h - 8);
    cx.text(
        FontId::Bold,
        11.0,
        Rect::new(inner.x, inner.y, inner.w, 14),
        Align::Left,
        t.text_dim,
        "KEYS PLAY",
    );
    let mut x = inner.x;
    let line = Rect::new(x, inner.y + 15, inner.w, 18);
    if k.missing && k.waiting.is_none() {
        cx.canvas
            .fill_round_rect(Rect::new(x, line.y + 5, 8, 8), 4, t.bad);
        x += 14;
    }
    let text = cut_to(cx, FontId::Bold, 14.0, k.text(), line.right() - x);
    cx.text(
        FontId::Bold,
        14.0,
        Rect::new(x, line.y, line.right() - x, line.h),
        Align::Left,
        t.text,
        &text,
    );
    let under = Rect::new(
        inner.x,
        inner.y + 35,
        inner.w,
        inner.bottom() - inner.y - 35,
    );
    if k.waiting.is_some() {
        cx.text(
            FontId::Sans,
            11.0,
            under,
            Align::Left,
            t.readable(t.warn, t.raised),
            "when your hands are up",
        );
    } else if !k.zones.is_empty() {
        paint_strip(cx, Rect::new(under.x, under.y + 2, under.w, 6), &k.zones);
    }
}

/// `s` cut with "…" to fit `w`.
pub fn cut_to(cx: &mut PaintCx, font: FontId, px: f32, s: &str, w: i32) -> String {
    if cx.measure(font, px, s) <= w {
        return s.to_string();
    }
    let mut cut = String::new();
    for c in s.chars() {
        let next = format!("{cut}{c}…");
        if cx.measure(font, px, &next) > w {
            break;
        }
        cut.push(c);
    }
    format!("{}…", cut.trim_end())
}

// ------------------------------------------------------------- the list

/// One tile of the patch list.
#[derive(Clone, Debug, PartialEq)]
pub struct TileLook {
    /// The patch's number (program change in picks it by that), or None:
    /// Keys as routed.
    pub number: Option<usize>,
    pub name: String,
    pub zones: Vec<ZoneLook>,
    /// The instruments its zones play: "Sub Bass C1–B2 · Grand C3–C7".
    pub line: String,
    /// What a missing instrument leaves silent, in red, in the line's
    /// place: "Track 7 is missing: C5 to C7 play nothing".
    pub missing: Option<String>,
    /// The keys play it now.
    pub playing: bool,
}

/// A layout New patch offers: its name, its zones, how many.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutLook {
    pub name: String,
    pub zones: Vec<ZoneLook>,
}

/// The list's events: `Tab(i)` a tap on tile `i` (0: Keys as routed,
/// then the patches), `Toggle(i)` its Edit, and these `Select`s.
pub const CLOSE: usize = 0;
pub const NEW: usize = 1;
/// The layouts' Back, to the list.
pub const BACK: usize = 2;
/// `Select(LAYOUT + k)`: New patch from layout `k`.
pub const LAYOUT: usize = 10;

const COLS: usize = 2;
const GAP: i32 = 10;
const HEAD_H: i32 = 44;
const CLOSE_W: i32 = 112;
const NEW_W: i32 = 140;
const BUTTON_H: i32 = 44;
const TILE_H: i32 = 96;
const EDIT_W: i32 = 84;
/// A finger that moves this far scrolls the list rather than tapping.
const SCROLL_SLOP: i32 = 12;

/// A finger on the list: where it went down, what was under it, the
/// scroll then, and whether it has scrolled.
#[derive(Clone, Debug)]
struct ListFinger {
    id: u64,
    y: i32,
    top: i32,
    on: Option<EventKind>,
    scrolled: bool,
}

/// The patch list (layout spec 4q), in place of Play's view.
#[derive(Default)]
pub struct PatchList {
    /// "5 patches in this song".
    pub count: String,
    pub tiles: Vec<TileLook>,
    /// New patch's layouts, while it asks for one (the tiles' place).
    pub layouts: Option<Vec<LayoutLook>>,
    /// The tile (or layout) the wheel is on.
    pub cursor: Option<usize>,
    /// How far the tiles are scrolled, in px.
    pub top: i32,
    finger: Option<ListFinger>,
}

impl PatchList {
    fn inner(r: Rect) -> Rect {
        r.inset_xy(14, 12)
    }

    fn close(r: Rect) -> Rect {
        let i = PatchList::inner(r);
        Rect::new(i.right() - CLOSE_W, i.y, CLOSE_W, BUTTON_H)
    }

    /// New patch, or the layouts' Back.
    fn new_button(r: Rect) -> Rect {
        let c = PatchList::close(r);
        Rect::new(c.x - 10 - NEW_W, c.y, NEW_W, BUTTON_H)
    }

    /// Where the tiles go.
    fn body(r: Rect) -> Rect {
        let i = PatchList::inner(r);
        Rect::new(i.x, i.y + HEAD_H + 12, i.w, i.h - HEAD_H - 12)
    }

    fn count_shown(&self) -> usize {
        self.layouts.as_ref().map_or(self.tiles.len(), Vec::len)
    }

    /// Tile `i` as scrolled (it may lie outside the body).
    pub fn tile(&self, r: Rect, i: usize) -> Rect {
        let b = PatchList::body(r);
        let w = (b.w - GAP * (COLS as i32 - 1)) / COLS as i32;
        let h = if self.layouts.is_some() { 110 } else { TILE_H };
        let (col, row) = ((i % COLS) as i32, (i / COLS) as i32);
        Rect::new(
            b.x + col * (w + GAP),
            b.y + row * (h + GAP) - self.top,
            w,
            h,
        )
    }

    /// The furthest the tiles scroll.
    pub fn max_top(&self, r: Rect) -> i32 {
        let n = self.count_shown();
        if n == 0 {
            return 0;
        }
        let b = PatchList::body(r);
        let last = self.tile(r, n - 1);
        (last.bottom() + self.top - b.y - b.h + 24).max(0)
    }

    /// Scrolls so tile `i` is all in view.
    pub fn show_tile(&mut self, r: Rect, i: usize) {
        let b = PatchList::body(r);
        let t = self.tile(r, i);
        if t.y < b.y {
            self.top -= b.y - t.y;
        } else if t.bottom() > b.bottom() {
            self.top += t.bottom() - b.bottom();
        }
        self.top = self.top.clamp(0, self.max_top(r));
    }

    fn edit(r: Rect, tile: Rect) -> Rect {
        let _ = r;
        Rect::new(
            tile.right() - 12 - EDIT_W,
            tile.y + (tile.h - BUTTON_H) / 2,
            EDIT_W,
            BUTTON_H,
        )
    }

    /// Tile `i` is on screen (wholly, for a finger's tap).
    fn in_view(&self, r: Rect, i: usize) -> bool {
        let b = PatchList::body(r);
        let t = self.tile(r, i);
        t.y >= b.y && t.bottom() <= b.bottom()
    }

    /// What a finger at (x, y) is on.
    fn at(&self, r: Rect, x: i32, y: i32) -> Option<EventKind> {
        if PatchList::close(r).contains(x, y) {
            return Some(EventKind::Select(CLOSE));
        }
        if PatchList::new_button(r).contains(x, y) {
            return Some(EventKind::Select(if self.layouts.is_some() {
                BACK
            } else {
                NEW
            }));
        }
        if !PatchList::body(r).contains(x, y) {
            return None;
        }
        let i = (0..self.count_shown()).find(|&i| self.tile(r, i).contains(x, y))?;
        if self.layouts.is_some() {
            return Some(EventKind::Select(LAYOUT + i));
        }
        let tile = self.tile(r, i);
        if i > 0 && PatchList::edit(r, tile).contains(x, y) {
            return Some(EventKind::Toggle(i));
        }
        Some(EventKind::Tab(i))
    }

    fn paint_tile(&self, cx: &mut PaintCx, r: Rect, i: usize, tile: &TileLook) {
        let t = cx.theme.clone();
        let tr = self.tile(r, i);
        let (fill, edge) = if tile.playing {
            (t.accent.mix(t.raised, 0.82), Some(t.accent))
        } else {
            (t.raised, None)
        };
        cx.canvas.fill_round_rect(tr, t.round(10), fill);
        if let Some(e) = edge {
            cx.canvas.stroke_round_rect(tr, t.round(10), 2, e);
        }
        if self.cursor == Some(i) {
            cx.canvas
                .stroke_round_rect(tr.inset(-3), t.round(12), 2, t.text);
        }
        // The number in a circle ("–" for Keys as routed).
        let c = Rect::new(tr.x + 14, tr.y + 14, 30, 30);
        let routed = tile.number.is_none();
        if routed {
            cx.canvas.stroke_round_rect(c, 15, 1, t.text_faint);
        } else {
            cx.canvas.fill_round_rect(c, 15, t.bg);
        }
        let num = tile
            .number
            .map_or_else(|| "–".to_string(), |n| n.to_string());
        cx.text(
            FontId::Bold,
            14.0,
            c,
            Align::Center,
            if routed { t.text_dim } else { t.text },
            &num,
        );
        let x = tr.x + 58;
        let right = if routed {
            tr.right() - 14
        } else {
            PatchList::edit(r, tr).x - 14
        };
        let mut name_right = right;
        if tile.playing {
            let pw = 92;
            let pr = Rect::new(right - pw, tr.y + 14, pw, 22);
            cx.canvas.fill_round_rect(pr, 11, t.accent);
            cx.text(
                FontId::Bold,
                11.0,
                pr,
                Align::Center,
                t.ink_on(t.accent),
                "PLAYING",
            );
            name_right = pr.x - 8;
        }
        let name = cut_to(cx, FontId::Bold, 20.0, &tile.name, name_right - x);
        cx.text(
            FontId::Bold,
            20.0,
            Rect::new(x, tr.y + 10, name_right - x, 30),
            Align::Left,
            if routed { t.text_dim } else { t.text },
            &name,
        );
        if !tile.zones.is_empty() {
            paint_strip(cx, Rect::new(x, tr.y + 44, right - x, 10), &tile.zones);
        }
        let (words, ink) = match &tile.missing {
            Some(m) => (m.as_str(), t.readable(t.bad, fill)),
            None => (tile.line.as_str(), t.text_dim),
        };
        let words = cut_to(cx, FontId::Sans, 13.0, words, right - x);
        cx.text(
            FontId::Sans,
            13.0,
            Rect::new(x, tr.bottom() - 32, right - x, 22),
            Align::Left,
            ink,
            &words,
        );
        if !routed {
            let e = PatchList::edit(r, tr);
            cx.canvas
                .fill_round_rect(e, t.round(8), t.bg.mix(fill, 0.5));
            cx.text(FontId::Bold, 15.0, e, Align::Center, t.text, "Edit");
        }
    }

    fn paint_layout(&self, cx: &mut PaintCx, r: Rect, i: usize, l: &LayoutLook) {
        let t = cx.theme.clone();
        let tr = self.tile(r, i);
        cx.canvas.fill_round_rect(tr, t.round(10), t.raised);
        if self.cursor == Some(i) {
            cx.canvas
                .stroke_round_rect(tr.inset(-3), t.round(12), 2, t.text);
        }
        let inner = tr.inset_xy(16, 12);
        cx.text(
            FontId::Bold,
            18.0,
            Rect::new(inner.x, inner.y, inner.w, 26),
            Align::Left,
            t.text,
            &l.name,
        );
        paint_strip(cx, Rect::new(inner.x, inner.y + 36, inner.w, 12), &l.zones);
        let n = l.zones.len();
        cx.text(
            FontId::Sans,
            13.0,
            Rect::new(inner.x, inner.bottom() - 20, inner.w, 20),
            Align::Left,
            t.text_dim,
            &format!("{n} zone{}", if n == 1 { "" } else { "s" }),
        );
    }
}

impl Widget for PatchList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let inner = PatchList::inner(r);
        let (title, sub) = if self.layouts.is_some() {
            (
                "NEW PATCH · START FROM",
                "Each zone starts on what the keys play now",
            )
        } else {
            ("KEYS PLAY", self.count.as_str())
        };
        let tw = cx.text(
            FontId::Bold,
            13.0,
            Rect::new(inner.x, inner.y, inner.w, BUTTON_H),
            Align::Left,
            t.text_dim,
            title,
        );
        let nb = PatchList::new_button(r);
        cx.text(
            FontId::Sans,
            14.0,
            Rect::new(
                inner.x + tw + 16,
                inner.y,
                (nb.x - inner.x - tw - 24).max(0),
                BUTTON_H,
            ),
            Align::Left,
            t.text_dim,
            sub,
        );
        cx.canvas.fill_round_rect(nb, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            15.0,
            nb,
            Align::Center,
            t.text,
            if self.layouts.is_some() {
                "Back"
            } else {
                "New patch"
            },
        );
        let close = PatchList::close(r);
        cx.canvas.fill_round_rect(close, t.round(10), t.raised);
        cx.text(FontId::Bold, 15.0, close, Align::Center, t.text, "Close");
        let body = PatchList::body(r);
        let clip = cx.canvas.clip();
        cx.canvas.set_clip(clip.intersect(body));
        match &self.layouts {
            Some(ls) => {
                for (i, l) in ls.iter().enumerate() {
                    self.paint_layout(cx, r, i, l);
                }
            }
            None => {
                for (i, tile) in self.tiles.iter().enumerate() {
                    if self.tile(r, i).intersect(body).is_empty() {
                        continue;
                    }
                    self.paint_tile(cx, r, i, tile);
                }
            }
        }
        cx.canvas.set_clip(clip);
        // Under the tiles, while there's room: how a switch goes.
        let n = self.count_shown();
        if n > 0 {
            let last = self.tile(r, n - 1);
            let y = last.bottom() + 14;
            if y + 20 <= body.bottom() {
                cx.text(
                    FontId::Sans,
                    13.0,
                    Rect::new(body.x + 4, y, body.w, 20),
                    Align::Left,
                    t.text_faint,
                    if self.layouts.is_some() {
                        "Pick each zone's instrument in the editor after."
                    } else {
                        "A tap switches at once. Held notes and the sustain pedal stay on the patch they started on."
                    },
                );
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = Vec::new();
        match &self.layouts {
            Some(ls) => {
                for (i, l) in ls.iter().enumerate() {
                    if self.in_view(r, i) {
                        out.push(Target::tap("layout", i, l.name.clone(), self.tile(r, i)));
                    }
                }
                out.push(Target::tap("back", 0, "Back", PatchList::new_button(r)));
            }
            None => {
                for (i, tile) in self.tiles.iter().enumerate() {
                    if !self.in_view(r, i) {
                        continue;
                    }
                    let tr = self.tile(r, i);
                    out.push(
                        Target::tap("patch", i, tile.name.clone(), tr).disabled_if(tile.playing),
                    );
                    if i > 0 {
                        out.push(Target::tap(
                            "edit",
                            i,
                            format!("Edit {}", tile.name),
                            PatchList::edit(r, tr),
                        ));
                    }
                }
                out.push(Target::tap("new", 0, "New patch", PatchList::new_button(r)));
            }
        }
        out.push(Target::tap("close", 0, "Close", PatchList::close(r)));
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let on = self.at(r, x, y);
                // The buttons answer at once; a tile when the finger lifts
                // without scrolling.
                if matches!(on, Some(EventKind::Select(CLOSE | NEW | BACK))) {
                    return on;
                }
                self.finger = Some(ListFinger {
                    id: finger,
                    y,
                    top: self.top,
                    on,
                    scrolled: false,
                });
                None
            }
            Touch::Move => {
                let max = self.max_top(r);
                let f = self.finger.as_mut().filter(|f| f.id == finger)?;
                if !f.scrolled && (y - f.y).abs() >= SCROLL_SLOP && max > 0 {
                    f.scrolled = true;
                }
                if f.scrolled {
                    self.top = (f.top - (y - f.y)).clamp(0, max);
                }
                None
            }
            Touch::Up => {
                let f = self.finger.take().filter(|f| f.id == finger)?;
                if f.scrolled {
                    return None;
                }
                // Still on what it went down on.
                let now = self.at(r, x, y);
                (now == f.on).then_some(now).flatten()
            }
            Touch::Cancel => {
                self.finger = None;
                None
            }
        }
    }
}

// ------------------------------------------------------------- the editor

/// The editor's rows, in the picked zone's card and the patch's own
/// (`Step { index, delta }` steps one, `Toggle(index)` switches one).
pub const ROW_PLAYS: usize = 0;
pub const ROW_KEYBOARD: usize = 1;
pub const ROW_LOW: usize = 2;
pub const ROW_HIGH: usize = 3;
pub const ROW_TRANSPOSE: usize = 4;
pub const ROW_VEL_LOW: usize = 5;
pub const ROW_VEL_HIGH: usize = 6;
pub const ROW_CURVE: usize = 7;
pub const ROW_PEDALS: usize = 8;
pub const ROW_WHEELS: usize = 9;
pub const ROW_OUTSIDE: usize = 10;
pub const ROW_OUTSIDE_PLAYS: usize = 11;
pub const ROW_CUT: usize = 12;
pub const ROWS: usize = 13;

/// The editor's buttons (`Select`).
pub const E_BACK: usize = 0;
pub const E_COPY: usize = 1;
/// Delete's second tap: the patch goes (the first only asks).
pub const E_DELETE: usize = 2;
pub const E_CLOSE: usize = 3;
pub const E_ADD: usize = 4;
pub const E_REMOVE: usize = 5;
/// Learn low and Learn high (`Toggle(E_LEARN + 0)` and `+ 1`), past the
/// rows.
pub const E_LEARN: usize = 20;

/// The `Span { index, .. }` of zone `z`'s low end dragged is `2 z`, its
/// high end's `2 z + 1`: `from` and `to` are its keys after the drag.
pub const fn grip(z: usize, high: bool) -> usize {
    2 * z + high as usize
}

/// A row as the editor shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct RowLook {
    pub value: String,
    /// A switch (Pedals, Wheels and pressure, Cut the old sound): on or
    /// off.
    pub on: Option<bool>,
    /// It can change (a row with one choice, or at both ends, can't).
    pub enabled: bool,
}

impl RowLook {
    pub fn text(v: impl Into<String>) -> RowLook {
        RowLook {
            value: v.into(),
            on: None,
            enabled: true,
        }
    }

    pub fn switch(on: bool) -> RowLook {
        RowLook {
            value: if on { "On" } else { "Off" }.to_string(),
            on: Some(on),
            enabled: true,
        }
    }
}

/// Everything the editor shows of one patch.
#[derive(Clone, Debug, PartialEq)]
pub struct EditLook {
    /// "Chorus split".
    pub name: String,
    /// "patch 3 · program change 3".
    pub sub: String,
    pub zones: Vec<ZoneLook>,
    /// The zone whose card shows.
    pub picked: usize,
    /// The keys held now: note and velocity (lit on the keyboard).
    pub held: Vec<(u8, u8)>,
    /// Per zone: the hardest velocity of the keys held that reach it
    /// (its lane lit, a tick on its VEL bar).
    pub reached: Vec<Option<u8>>,
    /// What came in last: "KeyLab 61 · E4 G4 at 96 → Grand, Strings".
    pub last: String,
    /// Learn low (false) or Learn high (true) waits for a key.
    pub learn: Option<bool>,
    /// The rows, by `ROW_*`.
    pub rows: Vec<RowLook>,
    /// The picked zone's card title: "ZONE 3 · STRINGS".
    pub zone_title: String,
    /// The text under Keys outside the zones (or None: the instrument's
    /// row shows there).
    pub outside_about: Option<String>,
    pub cut_about: String,
    /// The picked zone's instrument is missing: its card's title says so,
    /// red ("ZONE 3 · TRACK 7 ISN'T IN THIS SONG: ...").
    pub missing: Option<String>,
}

impl Default for EditLook {
    fn default() -> Self {
        EditLook {
            name: String::new(),
            sub: String::new(),
            zones: Vec::new(),
            picked: 0,
            held: Vec::new(),
            reached: Vec::new(),
            last: String::new(),
            learn: None,
            rows: (0..ROWS).map(|_| RowLook::text("")).collect(),
            zone_title: String::new(),
            outside_about: None,
            cut_about: String::new(),
            missing: None,
        }
    }
}

const ROW_LABELS: [&str; ROWS] = [
    "Plays",
    "Keyboard",
    "Low key",
    "High key",
    "Transpose",
    "Low velocity",
    "High velocity",
    "Velocity",
    "Pedals",
    "Wheels and pressure",
    "Keys outside the zones",
    "Plays",
    "Cut the old sound",
];

/// The row's label, for the wheel's outline and the hint.
pub fn row_label(row: usize) -> &'static str {
    ROW_LABELS.get(row).copied().unwrap_or("")
}

/// Rows that are a list of words (‹ ›), not a number (– +).
const fn is_list(row: usize) -> bool {
    matches!(
        row,
        ROW_PLAYS | ROW_KEYBOARD | ROW_CURVE | ROW_OUTSIDE | ROW_OUTSIDE_PLAYS
    )
}

const fn is_switch(row: usize) -> bool {
    matches!(row, ROW_PEDALS | ROW_WHEELS | ROW_CUT)
}

const HEAD: i32 = 50;
const PANEL_GAP: i32 = 8;
const CARDS_H: i32 = 246;
const LANE_PITCH: i32 = 30;
const LANE_H: i32 = 24;
const VEL_W: i32 = 76;
const KEYS_H: i32 = 60;
const STEP_H: i32 = 32;
/// How far a finger drags a value for one step.
const DRAG_STEP: i32 = 22;
/// A lane's end grip, each side of the end.
const GRIP: i32 = 14;

/// What a finger holds on the editor.
#[derive(Clone, Copy, Debug)]
enum Held {
    /// A value's middle, dragged across: steps per [`DRAG_STEP`].
    Row { row: usize, x: i32, done: i32 },
    /// A zone's end, dragged along the keys.
    End { zone: usize, high: bool },
}

/// Where the editor's parts sit in `r`.
struct Parts {
    head: Rect,
    zones: Rect,
    card: Rect,
    this: Rect,
}

/// The patch editor (layout spec 4q), in place of Play's view.
#[derive(Default)]
pub struct PatchEditor {
    pub look: EditLook,
    /// Delete was tapped once: it asks.
    pub asking: bool,
    fingers: Vec<(u64, Held)>,
    painted: RefCell<Option<Rect>>,
}

impl PatchEditor {
    fn parts(r: Rect) -> Parts {
        let (head, rest) = r.take_top(HEAD);
        let rest = Rect::new(rest.x, rest.y + PANEL_GAP, rest.w, rest.h - PANEL_GAP);
        let (cards, zones) = rest.take_bottom(CARDS_H);
        let zones = Rect::new(zones.x, zones.y, zones.w, zones.h - PANEL_GAP);
        let this_w = (cards.w * 36 / 100).max(300);
        let card = Rect::new(cards.x, cards.y, cards.w - this_w - PANEL_GAP, cards.h);
        let this = Rect::new(card.right() + PANEL_GAP, cards.y, this_w, cards.h);
        Parts {
            head,
            zones,
            card,
            this,
        }
    }

    /// The head's buttons: ‹ Patches, then Copy, Delete and Close at the
    /// right.
    fn head_buttons(r: Rect) -> [Rect; 4] {
        let h = PatchEditor::parts(r).head.inset_xy(8, 5);
        let back = Rect::new(h.x, h.y, 100, h.h);
        let close = Rect::new(h.right() - 82, h.y, 82, h.h);
        let delete = Rect::new(close.x - 8 - 92, h.y, 92, h.h);
        let copy = Rect::new(delete.x - 8 - 74, h.y, 74, h.h);
        [back, copy, delete, close]
    }

    /// The name, `w` wide, to tap and type.
    fn name_rect(w: i32, r: Rect) -> Rect {
        let h = PatchEditor::parts(r).head.inset_xy(8, 5);
        Rect::new(h.x + 116, h.y, w.max(60), h.h)
    }

    /// The zones panel's insides: the lanes, the VEL column, the keyboard
    /// and the last line.
    fn zone_area(r: Rect) -> (Rect, Rect, Rect, Rect) {
        let z = PatchEditor::parts(r).zones.inset_xy(12, 10);
        let (_, z) = z.take_top(24);
        let (last, z) = z.take_bottom(20);
        let (keys, lanes) = z.take_bottom(KEYS_H);
        let (vel, lanes) = lanes.take_right(VEL_W);
        let lanes = Rect::new(lanes.x, lanes.y, lanes.w - 12, lanes.h - 6);
        (lanes, vel, keys, last)
    }

    /// The lanes' pitch: 30 px, closer when many.
    fn pitch(&self, lanes: Rect) -> i32 {
        let n = self.look.zones.len() as i32 + i32::from(self.can_add());
        (lanes.h / n.max(1)).clamp(12, LANE_PITCH)
    }

    fn can_add(&self) -> bool {
        self.look.zones.len() < mpfree_patch::MAX_ZONES
    }

    /// Zone `z`'s lane (the whole width), and the "+ Zone" lane after the
    /// last.
    fn lane(&self, r: Rect, z: usize) -> Rect {
        let (lanes, _, _, _) = PatchEditor::zone_area(r);
        let p = self.pitch(lanes);
        let h = (p - 6).clamp(8, LANE_H);
        Rect::new(lanes.x, lanes.y + z as i32 * p, lanes.w, h)
    }

    /// Zone `z`'s bar in its lane (its keys across C1 to C7).
    fn bar(&self, r: Rect, z: usize) -> Option<Rect> {
        let lane = self.lane(r, z);
        let zl = self.look.zones.get(z)?;
        let (x, w) = bar_across(lane, zl.low, zl.high)?;
        Some(Rect::new(x, lane.y, w, lane.h))
    }

    /// The key under `x` on the lanes' scale (C1 to C7).
    fn key_at(&self, r: Rect, x: i32) -> u8 {
        let (lanes, _, _, _) = PatchEditor::zone_area(r);
        let span = i32::from(SHOWN_HIGH - SHOWN_LOW) + 1;
        let k = (x - lanes.x) * span / lanes.w.max(1);
        (i32::from(SHOWN_LOW) + k).clamp(0, 127) as u8
    }

    /// Learn low, Learn high and Remove at the zone card's head.
    fn card_buttons(r: Rect) -> [Rect; 3] {
        let c = PatchEditor::parts(r).card.inset_xy(12, 10);
        let remove = Rect::new(c.right() - 74, c.y, 74, STEP_H);
        let high = Rect::new(remove.x - 8 - 94, c.y, 94, STEP_H);
        let low = Rect::new(high.x - 8 - 90, c.y, 90, STEP_H);
        [low, high, remove]
    }

    /// Row `row`'s control (a stepper or a switch) and its label's place.
    fn row_rect(&self, r: Rect, row: usize) -> Option<(Rect, Rect)> {
        let p = PatchEditor::parts(r);
        let c = p.card.inset_xy(12, 10);
        let col = |n: i32, i: i32, y: i32| {
            let w = (c.w - (n - 1) * 12) / n;
            let x = c.x + i * (w + 12);
            (Rect::new(x, y + 16, w, STEP_H), Rect::new(x, y, w, 16))
        };
        let y0 = c.y + STEP_H + 10;
        let line = STEP_H + 22;
        Some(match row {
            ROW_PLAYS => col(2, 0, y0),
            ROW_KEYBOARD => col(2, 1, y0),
            ROW_LOW => col(3, 0, y0 + line),
            ROW_HIGH => col(3, 1, y0 + line),
            ROW_TRANSPOSE => col(3, 2, y0 + line),
            ROW_VEL_LOW => col(3, 0, y0 + 2 * line),
            ROW_VEL_HIGH => col(3, 1, y0 + 2 * line),
            ROW_CURVE => col(3, 2, y0 + 2 * line),
            ROW_PEDALS | ROW_WHEELS => {
                let y = y0 + 3 * line + 2;
                let (x, w) = if row == ROW_PEDALS {
                    (c.x, 150)
                } else {
                    (c.x + 170, 230)
                };
                (Rect::new(x, y, w, STEP_H), Rect::new(x, y, w, STEP_H))
            }
            ROW_OUTSIDE | ROW_OUTSIDE_PLAYS | ROW_CUT => {
                let t = p.this.inset_xy(12, 10);
                let y = t.y + 28;
                match row {
                    ROW_OUTSIDE => (
                        Rect::new(t.x, y + 18, t.w, STEP_H),
                        Rect::new(t.x, y, t.w, 16),
                    ),
                    ROW_OUTSIDE_PLAYS => (
                        Rect::new(t.x, y + 18 + STEP_H + 8, t.w, STEP_H),
                        Rect::new(t.x, y + 18 + STEP_H + 8, t.w, STEP_H),
                    ),
                    _ => {
                        let y = y + 18 + 2 * STEP_H + 26;
                        let sw = Rect::new(t.x + 150, y, 52, 28);
                        (sw, Rect::new(t.x, y, 150, 28))
                    }
                }
            }
            _ => return None,
        })
    }

    /// The rows shown now (the instrument for Keys outside the zones only
    /// while it plays one; the zone's rows only while it has zones).
    fn rows_shown(&self) -> Vec<usize> {
        let zone = !self.look.zones.is_empty();
        (0..ROWS)
            .filter(|&row| match row {
                ROW_OUTSIDE_PLAYS => self.look.outside_about.is_none(),
                ROW_OUTSIDE | ROW_CUT => true,
                _ => zone,
            })
            .collect()
    }

    fn paint_button(cx: &mut PaintCx, b: Rect, label: &str, on: bool, warn: bool) {
        let t = cx.theme.clone();
        let fill = if warn {
            t.bad
        } else if on {
            t.picked()
        } else {
            t.raised
        };
        cx.canvas.fill_round_rect(b, t.round(8), fill);
        let ink = if warn || on { t.ink_on(fill) } else { t.text };
        cx.text(FontId::Bold, 14.0, b, Align::Center, ink, label);
    }

    fn paint_head(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let p = PatchEditor::parts(r);
        cx.panel(p.head);
        let [back, copy, delete, close] = PatchEditor::head_buttons(r);
        PatchEditor::paint_button(cx, back, "‹ Patches", false, false);
        let nw = cx
            .measure(FontId::Bold, 20.0, &self.look.name)
            .min(copy.x - back.right() - 230);
        let name = PatchEditor::name_rect(nw, r);
        let shown = cut_to(cx, FontId::Bold, 20.0, &self.look.name, name.w);
        cx.text(FontId::Bold, 20.0, name, Align::Left, t.text, &shown);
        // Dotted under the name: tap to type.
        let mut x = name.x;
        while x < name.x + nw {
            cx.canvas
                .fill_rect(Rect::new(x, name.bottom() - 6, 3, 2), t.text_dim);
            x += 6;
        }
        cx.text(
            FontId::Sans,
            13.0,
            Rect::new(
                name.x + nw + 20,
                name.y,
                (copy.x - name.x - nw - 28).max(0),
                name.h,
            ),
            Align::Left,
            t.text_dim,
            &self.look.sub,
        );
        PatchEditor::paint_button(cx, copy, "Copy", false, false);
        if self.asking {
            PatchEditor::paint_button(cx, delete, "Delete?", false, true);
        } else {
            PatchEditor::paint_button(cx, delete, "Delete", false, false);
        }
        PatchEditor::paint_button(cx, close, "Close", false, false);
    }

    fn paint_zones(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let p = PatchEditor::parts(r);
        cx.panel(p.zones);
        let z = p.zones.inset_xy(12, 10);
        let n = self.look.zones.len();
        let tw = cx.text(
            FontId::Bold,
            12.0,
            Rect::new(z.x, z.y, 80, 20),
            Align::Left,
            t.text_dim,
            "ZONES",
        );
        cx.text(
            FontId::Sans,
            12.0,
            Rect::new(z.x + tw + 12, z.y, 80, 20),
            Align::Left,
            t.text_dim,
            &format!("{n} of {}", mpfree_patch::MAX_ZONES),
        );
        let (lanes, vel, keys, last) = PatchEditor::zone_area(r);
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(vel.x, z.y, vel.w, 20),
            Align::Left,
            t.text_dim,
            "VEL",
        );
        for (i, zl) in self.look.zones.iter().enumerate() {
            let lane = self.lane(r, i);
            let reached = self.look.reached.get(i).copied().flatten();
            if let Some(b) = self.bar(r, i) {
                let sw = t.swatch(zl.color);
                let (fill, ink) = if zl.missing {
                    (t.bad.mix(t.surface, 0.6), t.readable(t.bad, t.surface))
                } else if reached.is_some() || i == self.look.picked {
                    (sw.fill, sw.ink)
                } else {
                    (sw.fill.mix(t.surface, 0.25), sw.ink)
                };
                cx.canvas.fill_round_rect(b, t.round(6), fill);
                if zl.missing {
                    hatch(cx, b, t.bad);
                }
                if i == self.look.picked {
                    cx.canvas
                        .stroke_round_rect(b.inset(-2), t.round(8), 2, t.text);
                }
                if reached.is_some() {
                    cx.canvas
                        .stroke_round_rect(b.inset(-1), t.round(7), 1, t.accent);
                }
                let fs = if lane.h >= 18 { 13.0 } else { 10.0 };
                let mut right = b.right() - 8;
                if !zl.pedals && b.w > 80 {
                    // A crossed pedal: Pedals off.
                    let pr = Rect::new(right - 70, b.y + (b.h - 12) / 2, 12, 12);
                    cx.canvas.stroke_round_rect(pr, 6, 1, ink);
                    for d in 0..12 {
                        cx.canvas
                            .fill_rect(Rect::new(pr.right() - 1 - d, pr.y + d, 1, 1), ink);
                    }
                }
                if b.w > 120 {
                    let rw = cx.measure(FontId::Sans, fs - 2.0, &zl.range);
                    cx.text(
                        FontId::Sans,
                        fs - 2.0,
                        Rect::new(right - rw, b.y, rw, b.h),
                        Align::Left,
                        ink,
                        &zl.range,
                    );
                    right -= rw + 12;
                }
                let name = cut_to(cx, FontId::Bold, fs, &zl.name, (right - b.x - 10).max(0));
                cx.text(
                    FontId::Bold,
                    fs,
                    Rect::new(b.x + 10, b.y, (right - b.x - 10).max(0), b.h),
                    Align::Left,
                    ink,
                    &name,
                );
            }
            // Its velocities, 1 to 127, at the right; a held key's as a
            // tick.
            let vr = Rect::new(vel.x, lane.y + lane.h / 2 - 3, vel.w - 6, 6);
            cx.canvas.fill_round_rect(vr, 3, t.raised);
            let vx = |v: u8| vr.x + (i32::from(v.clamp(1, 127)) - 1) * vr.w / 126;
            let (a, b) = (vx(zl.vel_low), vx(zl.vel_high));
            cx.canvas.fill_round_rect(
                Rect::new(a, vr.y, (b - a).max(3), vr.h),
                3,
                t.swatch(zl.color).fill,
            );
            if let Some(v) = reached {
                cx.canvas
                    .fill_rect(Rect::new(vx(v) - 1, vr.y - 5, 2, vr.h + 10), t.text);
            }
        }
        if self.can_add() {
            let add = self.lane(r, n);
            let mut x = add.x;
            while x < add.right() {
                cx.canvas
                    .fill_rect(Rect::new(x, add.y, 4.min(add.right() - x), 1), t.outline);
                cx.canvas.fill_rect(
                    Rect::new(x, add.bottom() - 1, 4.min(add.right() - x), 1),
                    t.outline,
                );
                x += 8;
            }
            cx.text(
                FontId::Sans,
                12.0,
                add.inset_xy(10, 0),
                Align::Left,
                t.text_dim,
                if self.look.learn == Some(false) && n > 0 {
                    "+ Zone: play the key it starts on"
                } else {
                    "+ Zone: press Learn low, then play a key"
                },
            );
        }
        let lanes_keys = Rect::new(lanes.x, keys.y, lanes.w, keys.h);
        paint_keyboard(cx, lanes_keys, &self.look.held);
        let line = cut_to(cx, FontId::Sans, 12.0, &self.look.last, last.w);
        cx.text(FontId::Sans, 12.0, last, Align::Left, t.text_dim, &line);
    }

    fn paint_stepper(&self, cx: &mut PaintCx, b: Rect, row: usize) {
        let t = cx.theme.clone();
        let Some(look) = self.look.rows.get(row) else {
            return;
        };
        cx.canvas.fill_round_rect(b, t.round(8), t.raised);
        let (less, more) = if is_list(row) {
            ("‹", "›")
        } else {
            ("–", "+")
        };
        let side = STEP_H.min(b.w / 4);
        let ink = if look.enabled { t.text } else { t.text_faint };
        if is_list(row) {
            cx.text(
                FontId::Bold,
                15.0,
                Rect::new(b.x, b.y, side, b.h),
                Align::Center,
                ink,
                less,
            );
            cx.text(
                FontId::Bold,
                15.0,
                Rect::new(b.right() - side, b.y, side, b.h),
                Align::Center,
                ink,
                more,
            );
        } else {
            for (x, s) in [(b.x, less), (b.right() - side, more)] {
                let br = Rect::new(x, b.y, side, b.h);
                cx.canvas
                    .fill_round_rect(br, t.round(8), t.bg.mix(t.raised, 0.5));
                cx.text(FontId::Bold, 16.0, br, Align::Center, ink, s);
            }
        }
        let mid = Rect::new(b.x + side, b.y, b.w - 2 * side, b.h);
        let v = cut_to(cx, FontId::Bold, 14.0, &look.value, mid.w - 4);
        cx.text(FontId::Bold, 14.0, mid, Align::Center, ink, &v);
    }

    fn paint_switch(cx: &mut PaintCx, b: Rect, on: bool) {
        let t = cx.theme.clone();
        let s = Rect::new(b.x, b.y + (b.h - 26) / 2, 48, 26);
        let fill = if on { t.picked() } else { t.raised };
        cx.canvas.fill_round_rect(s, 13, fill);
        let knob = if on {
            Rect::new(s.right() - 23, s.y + 3, 20, 20)
        } else {
            Rect::new(s.x + 3, s.y + 3, 20, 20)
        };
        cx.canvas.fill_round_rect(knob, 10, t.ink_on(fill));
    }

    fn paint_cards(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let p = PatchEditor::parts(r);
        cx.panel(p.card);
        cx.panel(p.this);
        let c = p.card.inset_xy(12, 10);
        if self.look.zones.is_empty() {
            cx.text(
                FontId::Sans,
                14.0,
                Rect::new(c.x, c.y, c.w, 60),
                Align::Left,
                t.text_dim,
                "No zones: every key plays as Keys outside the zones says. Tap + Zone to add one.",
            );
        } else {
            let color = self
                .look
                .zones
                .get(self.look.picked)
                .map_or(t.text_dim, |z| t.swatch(z.color).fill);
            cx.canvas
                .fill_round_rect(Rect::new(c.x, c.y + 12, 8, 8), 4, color);
            let [low, high, remove] = PatchEditor::card_buttons(r);
            // A missing instrument says so in the title's place, red.
            let (title, ink) = match &self.look.missing {
                Some(m) => (m.as_str(), t.readable(t.bad, t.surface)),
                None => (self.look.zone_title.as_str(), t.text_dim),
            };
            let title = cut_to(cx, FontId::Bold, 12.0, title, low.x - c.x - 20);
            cx.text(
                FontId::Bold,
                12.0,
                Rect::new(c.x + 14, c.y, low.x - c.x - 20, STEP_H),
                Align::Left,
                ink,
                &title,
            );
            PatchEditor::paint_button(cx, low, "Learn low", self.look.learn == Some(false), false);
            PatchEditor::paint_button(cx, high, "Learn high", self.look.learn == Some(true), false);
            PatchEditor::paint_button(cx, remove, "Remove", false, false);
        }
        for row in self.rows_shown() {
            let Some((b, label)) = self.row_rect(r, row) else {
                continue;
            };
            if is_switch(row) {
                let words = row_label(row);
                let lw = cx.text(
                    FontId::Sans,
                    13.0,
                    Rect::new(label.x, label.y, label.w, label.h),
                    Align::Left,
                    t.text,
                    words,
                );
                let on = self.look.rows.get(row).and_then(|l| l.on).unwrap_or(false);
                let sw = if row == ROW_CUT {
                    b
                } else {
                    Rect::new(label.x + lw + 12, b.y, 52, b.h)
                };
                PatchEditor::paint_switch(cx, sw, on);
                continue;
            }
            if row != ROW_OUTSIDE_PLAYS {
                cx.text(
                    FontId::Sans,
                    12.0,
                    label,
                    Align::Left,
                    t.text_dim,
                    row_label(row),
                );
            }
            self.paint_stepper(cx, b, row);
        }
        // This patch: its title and what its settings mean.
        let th = p.this.inset_xy(12, 10);
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(th.x, th.y, th.w, 22),
            Align::Left,
            t.text_dim,
            "THIS PATCH",
        );
        if let Some(about) = &self.look.outside_about {
            if let Some((b, _)) = self.row_rect(r, ROW_OUTSIDE) {
                paint_lines(cx, Rect::new(b.x, b.bottom() + 4, b.w, 30), about);
            }
        }
        if let Some((sw, _)) = self.row_rect(r, ROW_CUT) {
            paint_lines(
                cx,
                Rect::new(th.x, sw.bottom() + 4, th.w, 30),
                &self.look.cut_about,
            );
        }
    }

    /// What a finger going down at (x, y) is on: an event at once, or a
    /// hold that answers as it moves.
    fn down_at(&self, r: Rect, x: i32, y: i32) -> (Option<EventKind>, Option<Held>) {
        let [back, copy, delete, close] = PatchEditor::head_buttons(r);
        if back.contains(x, y) {
            return (Some(EventKind::Select(E_BACK)), None);
        }
        if copy.contains(x, y) {
            return (Some(EventKind::Select(E_COPY)), None);
        }
        if delete.contains(x, y) {
            return (Some(EventKind::Select(E_DELETE)), None);
        }
        if close.contains(x, y) {
            return (Some(EventKind::Select(E_CLOSE)), None);
        }
        let p = PatchEditor::parts(r);
        if p.head.contains(x, y) && x > back.right() && x < copy.x {
            return (Some(EventKind::Type(0)), None);
        }
        if !self.look.zones.is_empty() {
            let [low, high, remove] = PatchEditor::card_buttons(r);
            if low.contains(x, y) {
                return (Some(EventKind::Toggle(E_LEARN)), None);
            }
            if high.contains(x, y) {
                return (Some(EventKind::Toggle(E_LEARN + 1)), None);
            }
            if remove.contains(x, y) {
                return (Some(EventKind::Select(E_REMOVE)), None);
            }
        }
        // A zone's ends, then its lane.
        for z in 0..self.look.zones.len() {
            let lane = self.lane(r, z);
            if !lane.inset_xy(0, -2).contains(x, y) {
                continue;
            }
            if let Some(b) = self.bar(r, z) {
                if (x - b.x).abs() <= GRIP {
                    return (
                        None,
                        Some(Held::End {
                            zone: z,
                            high: false,
                        }),
                    );
                }
                if (x - b.right()).abs() <= GRIP {
                    return (
                        None,
                        Some(Held::End {
                            zone: z,
                            high: true,
                        }),
                    );
                }
            }
            return (Some(EventKind::Tab(z)), None);
        }
        if self.can_add() && self.lane(r, self.look.zones.len()).contains(x, y) {
            return (Some(EventKind::Select(E_ADD)), None);
        }
        for row in self.rows_shown() {
            let Some((b, _)) = self.row_rect(r, row) else {
                continue;
            };
            if is_switch(row) {
                // The switch and its words.
                let (_, label) = self.row_rect(r, row).unwrap_or((b, b));
                if b.contains(x, y) || label.contains(x, y) {
                    return (Some(EventKind::Toggle(row)), None);
                }
                continue;
            }
            if !b.contains(x, y) {
                continue;
            }
            let side = STEP_H.min(b.w / 4);
            if x < b.x + side {
                return (
                    Some(EventKind::Step {
                        index: row,
                        delta: -1,
                    }),
                    None,
                );
            }
            if x >= b.right() - side {
                return (
                    Some(EventKind::Step {
                        index: row,
                        delta: 1,
                    }),
                    None,
                );
            }
            return (None, Some(Held::Row { row, x, done: 0 }));
        }
        (None, None)
    }

    /// The switch's or row's target rectangle (a switch's words and
    /// switch together).
    fn switch_rect(&self, r: Rect, row: usize) -> Option<Rect> {
        let (b, label) = self.row_rect(r, row)?;
        Some(if row == ROW_CUT {
            label.union(b)
        } else {
            Rect::new(label.x, b.y, 150.max(label.w), b.h)
        })
    }
}

/// `s` in small dim words over two lines of `r` at most.
fn paint_lines(cx: &mut PaintCx, r: Rect, s: &str) {
    let ink = cx.theme.text_dim;
    for (i, line) in wrap_words(cx, s, r.w, 2).iter().enumerate() {
        cx.text(
            FontId::Sans,
            11.0,
            Rect::new(r.x, r.y + i as i32 * 15, r.w, 15),
            Align::Left,
            ink,
            line,
        );
    }
}

/// `s` broken into at most `lines` lines of `w` (the rest cut).
fn wrap_words(cx: &mut PaintCx, s: &str, w: i32, lines: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if cx.measure(FontId::Sans, 11.0, &next) > w && !line.is_empty() {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.len() > lines {
        out.truncate(lines);
        if let Some(l) = out.last_mut() {
            l.push('…');
        }
    }
    out
}

/// The keyboard from C1 to C7 across `r`, the keys `held` lit.
fn paint_keyboard(cx: &mut PaintCx, r: Rect, held: &[(u8, u8)]) {
    let t = cx.theme.clone();
    let k = t.keys();
    let whites: Vec<u8> = (SHOWN_LOW..=SHOWN_HIGH).filter(|&n| !is_black(n)).collect();
    let n = whites.len() as i32;
    let ww = r.w / n;
    let lit = |note: u8| held.iter().any(|&(h, _)| h == note);
    for (i, &note) in whites.iter().enumerate() {
        let x = r.x + i as i32 * ww;
        let kr = Rect::new(x, r.y, ww - 1, r.h);
        let fill = if lit(note) { t.accent } else { k.white };
        cx.canvas.fill_round_rect(kr, 2, fill);
        if note % 12 == 0 {
            cx.text(
                FontId::Sans,
                9.0,
                Rect::new(kr.x + 2, kr.bottom() - 13, kr.w, 12),
                Align::Left,
                t.ink_on(fill),
                &mpfree_midi_ui::note_name(note),
            );
        }
    }
    for (i, &note) in whites.iter().enumerate() {
        let black = note + 1;
        if black > SHOWN_HIGH || !is_black(black) {
            continue;
        }
        let x = r.x + (i as i32 + 1) * ww - ww * 3 / 10;
        let kr = Rect::new(x, r.y, ww * 6 / 10, r.h * 6 / 10);
        let fill = if lit(black) { t.accent } else { k.black };
        cx.canvas.fill_round_rect(kr, 2, fill);
    }
}

fn is_black(n: u8) -> bool {
    matches!(n % 12, 1 | 3 | 6 | 8 | 10)
}

impl Widget for PatchEditor {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        self.paint_head(cx, r);
        self.paint_zones(cx, r);
        self.paint_cards(cx, r);
        *self.painted.borrow_mut() = Some(r);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let [back, copy, delete, close] = PatchEditor::head_buttons(r);
        let mut out = vec![
            Target::tap("back", 0, "‹ Patches", back),
            Target::tap(
                "name",
                0,
                self.look.name.clone(),
                Rect::new(back.right() + 8, back.y, copy.x - back.right() - 16, back.h),
            ),
            Target::tap("copy", 0, "Copy", copy),
            Target::tap(
                "delete",
                0,
                if self.asking { "Delete?" } else { "Delete" },
                delete,
            ),
            Target::tap("close", 0, "Close", close),
        ];
        for (z, zl) in self.look.zones.iter().enumerate() {
            let lane = self.lane(r, z);
            out.push(
                Target::tap("zone", z, zl.name.clone(), lane).disabled_if(z == self.look.picked),
            );
            if let Some(b) = self.bar(r, z) {
                for (high, x) in [(false, b.x), (true, b.right())] {
                    let g = Rect::new(x - GRIP / 2, lane.y, GRIP, lane.h);
                    let (id, what) = if high {
                        ("zone-high", "high end")
                    } else {
                        ("zone-low", "low end")
                    };
                    out.push(
                        Target::drag(id, z, format!("{} {what}", zl.name), g)
                            .towards(if high { -20 } else { 20 }, 0),
                    );
                }
            }
        }
        if self.can_add() {
            out.push(Target::tap(
                "add-zone",
                0,
                "+ Zone",
                self.lane(r, self.look.zones.len()),
            ));
        }
        if !self.look.zones.is_empty() {
            let [low, high, remove] = PatchEditor::card_buttons(r);
            out.push(Target::tap("learn-low", 0, "Learn low", low));
            out.push(Target::tap("learn-high", 0, "Learn high", high));
            out.push(Target::tap("remove", 0, "Remove", remove));
        }
        for row in self.rows_shown() {
            let label = format!(
                "{} {}",
                row_label(row),
                self.look.rows.get(row).map_or("", |l| l.value.as_str())
            );
            if is_switch(row) {
                if let Some(b) = self.switch_rect(r, row) {
                    out.push(Target::tap("switch", row, label, b));
                }
                continue;
            }
            if let Some((b, _)) = self.row_rect(r, row) {
                let enabled = self.look.rows.get(row).is_none_or(|l| l.enabled);
                out.push(Target::drag("row", row, label, b).disabled_if(!enabled));
            }
        }
        out
    }

    fn steps(&self, t: &Target) -> bool {
        matches!(t.id, "row" | "zone-low" | "zone-high")
    }

    fn step(&mut self, t: &Target, delta: i32, _r: Rect) -> Option<EventKind> {
        match t.id {
            "row" => Some(EventKind::Step {
                index: t.index,
                delta,
            }),
            "zone-low" | "zone-high" => {
                let z = self.look.zones.get(t.index)?;
                let high = t.id == "zone-high";
                let (lo, hi) = if high {
                    (
                        z.low,
                        (i32::from(z.high) + delta).clamp(i32::from(z.low), 127) as u8,
                    )
                } else {
                    (
                        (i32::from(z.low) + delta).clamp(0, i32::from(z.high)) as u8,
                        z.high,
                    )
                };
                ((lo, hi) != (z.low, z.high)).then_some(EventKind::Span {
                    index: grip(t.index, high),
                    from: i64::from(lo),
                    to: i64::from(hi),
                })
            }
            _ => None,
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let (ev, held) = self.down_at(r, x, y);
                if !matches!(ev, Some(EventKind::Select(E_DELETE))) {
                    self.asking = false;
                }
                if let Some(h) = held {
                    self.fingers.retain(|f| f.0 != finger);
                    self.fingers.push((finger, h));
                }
                ev
            }
            Touch::Move => {
                let i = self.fingers.iter().position(|f| f.0 == finger)?;
                match self.fingers[i].1 {
                    Held::Row { row, x: x0, done } => {
                        let want = (x - x0) / DRAG_STEP;
                        if want == done {
                            return None;
                        }
                        self.fingers[i].1 = Held::Row {
                            row,
                            x: x0,
                            done: want,
                        };
                        Some(EventKind::Step {
                            index: row,
                            delta: want - done,
                        })
                    }
                    Held::End { zone, high } => {
                        let z = self.look.zones.get(zone)?;
                        let k = self.key_at(r, x);
                        let (lo, hi) = if high {
                            (z.low, k.max(z.low))
                        } else {
                            (k.min(z.high), z.high)
                        };
                        if (lo, hi) == (z.low, z.high) {
                            return None;
                        }
                        // Drawn moved at once; the screen keeps it.
                        if let Some(z) = self.look.zones.get_mut(zone) {
                            (z.low, z.high) = (lo, hi);
                        }
                        Some(EventKind::Span {
                            index: grip(zone, high),
                            from: i64::from(lo),
                            to: i64::from(hi),
                        })
                    }
                }
            }
            Touch::Up | Touch::Cancel => {
                self.fingers.retain(|f| f.0 != finger);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};

    fn zone(name: &str, low: u8, high: u8) -> ZoneLook {
        ZoneLook {
            name: name.into(),
            color: Color::rgb(0x3FC1A5),
            low,
            high,
            vel_low: 1,
            vel_high: 127,
            range: String::new(),
            missing: false,
            pedals: true,
        }
    }

    /// Layered zones go on rows of their own; a split shares one.
    #[test]
    fn a_strip_puts_layers_on_rows_of_their_own() {
        let split = [zone("Bass", 0, 47), zone("Keys", 48, 127)];
        assert_eq!(strip_rows(&split), (vec![0, 0], 1));
        let chorus = [
            zone("Bass", 0, 47),
            zone("Grand", 48, 127),
            zone("Strings", 48, 127),
        ];
        assert_eq!(strip_rows(&chorus), (vec![0, 0, 1], 2));
    }

    fn list() -> PatchList {
        let tile = |n: Option<usize>, name: &str| TileLook {
            number: n,
            name: name.into(),
            zones: vec![zone(name, 0, 127)],
            line: format!("{name}, every key"),
            missing: None,
            playing: n == Some(3),
        };
        let mut tiles = vec![tile(None, "Keys as routed")];
        tiles.extend((1..=12).map(|n| tile(Some(n), &format!("Patch {n}"))));
        PatchList {
            count: "12 patches in this song".into(),
            tiles,
            ..PatchList::default()
        }
    }

    /// A tap on a tile picks it when the finger lifts; Edit edits it;
    /// a finger that moves scrolls instead; Close and New patch answer at
    /// once.
    #[test]
    fn the_list_taps_edits_and_scrolls() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add(LIST, list());
        ui.add_full_page("p", "col(patch-list:*)").unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let r = ui.rect_of(LIST).unwrap();
        let l = ui.get::<PatchList>(LIST).unwrap();
        let t2 = l.tile(r, 2);
        assert!(t2.w >= 480 && t2.h == TILE_H, "{t2:?}");
        let edit = PatchList::edit(r, t2);
        let close = PatchList::close(r);
        assert!(l.max_top(r) > 0, "13 tiles don't fit");
        let tap = |ui: &mut Ui, x, y| {
            let a = ui.touch(1, Touch::Down, x, y).map(|e| e.kind);
            let b = ui.touch(1, Touch::Up, x, y).map(|e| e.kind);
            a.or(b)
        };
        assert_eq!(tap(&mut ui, t2.x + 20, t2.y + 20), Some(EventKind::Tab(2)));
        assert_eq!(
            tap(&mut ui, edit.x + 10, edit.y + 10),
            Some(EventKind::Toggle(2))
        );
        assert_eq!(
            tap(&mut ui, close.x + 10, close.y + 10),
            Some(EventKind::Select(CLOSE))
        );
        // A drag up scrolls, and picks nothing.
        ui.touch(2, Touch::Down, t2.x + 20, t2.y + 60);
        ui.touch(2, Touch::Move, t2.x + 20, t2.y);
        assert_eq!(
            ui.touch(2, Touch::Up, t2.x + 20, t2.y).map(|e| e.kind),
            None
        );
        assert!(ui.get::<PatchList>(LIST).unwrap().top > 0);
        // The targets are only the tiles in view.
        let l = ui.get::<PatchList>(LIST).unwrap();
        for tg in l.targets(r) {
            assert!(r.contains(tg.rect.x, tg.rect.y), "{tg:?}");
        }
    }

    fn editor() -> PatchEditor {
        let mut look = EditLook {
            name: "Chorus split".into(),
            sub: "patch 3 · program change 3".into(),
            zones: vec![
                zone("Sub Bass", 0, 47),
                zone("Grand", 48, 127),
                zone("Strings", 48, 127),
            ],
            picked: 2,
            held: vec![(36, 52), (64, 96), (67, 96)],
            reached: vec![Some(52), Some(96), Some(96)],
            last: "KeyLab 61 · C2 at 52 → Sub Bass".into(),
            zone_title: "ZONE 3 · STRINGS".into(),
            outside_about: Some("Keys no zone takes play nothing.".into()),
            cut_about: "Off: notes ring on.".into(),
            ..EditLook::default()
        };
        look.rows[ROW_LOW] = RowLook::text("C3");
        look.rows[ROW_PEDALS] = RowLook::switch(true);
        PatchEditor {
            look,
            ..PatchEditor::default()
        }
    }

    /// The editor's parts each answer: the buttons, a lane, an end
    /// dragged along the keys, a row's – and + and its middle dragged,
    /// a switch, + Zone; Delete asks before it goes.
    #[test]
    fn the_editor_answers_each_part() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add(EDIT, editor());
        ui.add_full_page("p", "col(patch-edit:*)").unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let r = ui.rect_of(EDIT).unwrap();
        let e = ui.get::<PatchEditor>(EDIT).unwrap();
        let (lane, bar, add) = (e.lane(r, 1), e.bar(r, 1).unwrap(), e.lane(r, 3));
        let (low, _) = e.row_rect(r, ROW_LOW).unwrap();
        let sw = e.switch_rect(r, ROW_PEDALS).unwrap();
        let ev = |ui: &mut Ui, x: i32, y: i32| {
            let a = ui.touch(1, Touch::Down, x, y).map(|e| e.kind);
            ui.touch(1, Touch::Up, x, y);
            a
        };
        let [back, _, delete, _] = PatchEditor::head_buttons(r);
        assert_eq!(
            ev(&mut ui, back.x + 5, back.y + 5),
            Some(EventKind::Select(E_BACK))
        );
        assert_eq!(
            ev(&mut ui, delete.x + 5, delete.y + 5),
            Some(EventKind::Select(E_DELETE))
        );
        assert_eq!(
            ev(&mut ui, bar.x + bar.w / 2, lane.y + 5),
            Some(EventKind::Tab(1))
        );
        assert_eq!(
            ev(&mut ui, add.x + 40, add.y + 5),
            Some(EventKind::Select(E_ADD))
        );
        assert_eq!(
            ev(&mut ui, low.x + 4, low.y + 4),
            Some(EventKind::Step {
                index: ROW_LOW,
                delta: -1
            })
        );
        assert_eq!(
            ev(&mut ui, low.right() - 4, low.y + 4),
            Some(EventKind::Step {
                index: ROW_LOW,
                delta: 1
            })
        );
        // The middle dragged right two steps.
        let mid = (low.x + low.w / 2, low.y + 10);
        assert_eq!(ui.touch(3, Touch::Down, mid.0, mid.1), None);
        assert_eq!(
            ui.touch(3, Touch::Move, mid.0 + 2 * DRAG_STEP, mid.1)
                .map(|e| e.kind),
            Some(EventKind::Step {
                index: ROW_LOW,
                delta: 2
            })
        );
        ui.touch(3, Touch::Up, mid.0, mid.1);
        assert_eq!(
            ev(&mut ui, sw.x + 4, sw.y + 4),
            Some(EventKind::Toggle(ROW_PEDALS))
        );
        // Grand's low end dragged up an octave.
        let lane0 = lane;
        let k = |n: u8| key_x(lane0, i32::from(n));
        ui.touch(4, Touch::Down, bar.x + 2, lane0.y + 5);
        let got = ui
            .touch(4, Touch::Move, k(60) + 2, lane0.y + 5)
            .map(|e| e.kind);
        assert_eq!(
            got,
            Some(EventKind::Span {
                index: grip(1, false),
                from: 60,
                to: 127
            })
        );
        ui.touch(4, Touch::Up, k(60), lane0.y + 5);
        // Every target answers where it says.
        let e = ui.get::<PatchEditor>(EDIT).unwrap();
        for tg in e.targets(r) {
            assert!(r.contains(tg.rect.x, tg.rect.y), "{tg:?}");
        }
    }
}
