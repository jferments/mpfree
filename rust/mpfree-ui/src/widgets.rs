//! The widget library. Each widget only shows what the app put in its fields
//! and reports touches; none of them knows about MIDI, audio or files.

use crate::color::Color;
use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::theme::Tone;
use crate::ui::{EventKind, PaintCx, Touch, Widget};
use std::cell::RefCell;
use std::collections::VecDeque;

// ---------------------------------------------------------------- header

/// A status pill on the right of the header.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub text: String,
    pub tone: Tone,
}

impl Chip {
    pub fn new(text: impl Into<String>, tone: Tone) -> Chip {
        Chip { text: text.into(), tone }
    }
}

/// The top bar: the MPFree name, page tabs, and status chips.
pub struct Header {
    pub tabs: Vec<String>,
    pub active: usize,
    pub chips: Vec<Chip>,
}

const BRAND_W: i32 = 170;
const TAB_W: i32 = 112;
const TAB_GAP: i32 = 6;

impl Header {
    pub fn new(tabs: &[&str]) -> Header {
        Header { tabs: tabs.iter().map(|s| s.to_string()).collect(), active: 0, chips: Vec::new() }
    }

    fn tab_rect(&self, r: Rect, i: usize) -> Rect {
        let h = 44.min(r.h);
        Rect::new(r.x + BRAND_W + i as i32 * (TAB_W + TAB_GAP), r.y + (r.h - h) / 2, TAB_W, h)
    }
}

impl Widget for Header {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (brand, _) = r.take_left(BRAND_W);
        let mp = cx.measure(FontId::Bold, t.size_brand, "MP");
        cx.text(FontId::Bold, t.size_brand, brand.inset_xy(4, 0), Align::Left, t.text, "MP");
        cx.text(FontId::Bold, t.size_brand, brand.inset_xy(4, 0).take_left(mp).1, Align::Left, t.accent, "Free");
        for (i, name) in self.tabs.iter().enumerate() {
            let tr = self.tab_rect(r, i);
            let on = i == self.active;
            if on {
                cx.canvas.fill_round_rect(tr, 10, t.raised);
                let bar = Rect::new(tr.x + 20, tr.bottom() - 4, tr.w - 40, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let color = if on { t.text } else { t.text_dim };
            cx.text(if on { FontId::Bold } else { FontId::Sans }, t.size_title, tr, Align::Center, color, name);
        }
        // Chips from the right edge leftwards; stop before the tabs.
        let limit = self.tab_rect(r, self.tabs.len()).x;
        let mut right = r.right();
        let h = 34.min(r.h);
        for chip in &self.chips {
            let tw = cx.measure(FontId::Sans, t.size_small, &chip.text);
            let w = tw + 40;
            if right - w < limit {
                break;
            }
            let cr = Rect::new(right - w, r.y + (r.h - h) / 2, w, h);
            cx.canvas.fill_round_rect(cr, h / 2, t.surface);
            let dot = Rect::new(cr.x + 14, cr.y + h / 2 - 4, 8, 8);
            cx.canvas.fill_round_rect(dot, 4, t.tone(chip.tone));
            let text = Rect::new(cr.x + 28, cr.y, tw + 4, h);
            cx.text(FontId::Sans, t.size_small, text, Align::Left, t.text, &chip.text);
            right -= w + 8;
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

// ---------------------------------------------------------------- pads

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pad {
    /// What the pad plays; empty if nothing.
    pub label: String,
    /// The pad's colour, or None for a pad with nothing on it.
    pub color: Option<Color>,
    /// 0 at rest, 1 when just hit; the app fades it back with `PadGrid::fade`.
    pub level: f32,
    pub held: bool,
    /// Velocity of the last hit, 0-127.
    pub velocity: u8,
}

/// The 4 x 4 pads, laid out as on the MPC: pad 1 bottom left, pad 16 top right.
pub struct PadGrid {
    pub pads: Vec<Pad>,
    pub cols: usize,
    fingers: Vec<(u64, usize)>,
    /// The pads as last drawn, so a change redraws only the pads that changed.
    drawn: RefCell<Option<(Rect, Vec<Pad>)>>,
}

const PAD_GAP: i32 = 12;

impl PadGrid {
    pub fn new(count: usize, cols: usize) -> PadGrid {
        assert!(cols > 0 && count > 0 && count.is_multiple_of(cols));
        PadGrid { pads: vec![Pad::default(); count], cols, fingers: Vec::new(), drawn: RefCell::new(None) }
    }

    fn rows(&self) -> usize {
        self.pads.len() / self.cols
    }

    /// The rectangle of pad `i` (0-based) inside `r`.
    pub fn cell(&self, r: Rect, i: usize) -> Rect {
        let (cols, rows) = (self.cols as i32, self.rows() as i32);
        let w = (r.w - PAD_GAP * (cols - 1)) / cols;
        let h = (r.h - PAD_GAP * (rows - 1)) / rows;
        let col = (i % self.cols) as i32;
        let row_from_bottom = (i / self.cols) as i32;
        let row = rows - 1 - row_from_bottom;
        Rect::new(r.x + col * (w + PAD_GAP), r.y + row * (h + PAD_GAP), w, h)
    }

    /// True while some pad is still fading.
    pub fn fading(&self) -> bool {
        self.pads.iter().any(|p| !p.held && p.level > 0.0)
    }

    /// Fade released pads by `dt` seconds (a hit fades over about 0.35 s).
    pub fn fade(&mut self, dt: f32) {
        for p in self.pads.iter_mut().filter(|p| !p.held) {
            p.level = (p.level - dt / 0.35).max(0.0);
        }
    }

    pub fn hit(&mut self, i: usize, velocity: u8) {
        if let Some(p) = self.pads.get_mut(i) {
            p.held = true;
            p.level = 1.0;
            p.velocity = velocity;
        }
    }

    pub fn release(&mut self, i: usize) {
        if let Some(p) = self.pads.get_mut(i) {
            p.held = false;
        }
    }
}

impl Widget for PadGrid {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, old) = drawn.as_ref()?;
        if *at != r || old.len() != self.pads.len() {
            return None;
        }
        Some((0..self.pads.len()).filter(|&i| old[i] != self.pads[i]).map(|i| self.cell(r, i)).collect())
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        *self.drawn.borrow_mut() = Some((r, self.pads.clone()));
        let clip = cx.canvas.clip();
        for (i, pad) in self.pads.iter().enumerate() {
            let cell = self.cell(r, i);
            if cell.intersect(clip).is_empty() {
                continue;
            }
            let body = cell.inset(3);
            let lit = if pad.held { 1.0 } else { pad.level };
            let (fill, ink, sub) = match pad.color {
                Some(c) => {
                    let fill = t.surface.mix(c, 0.20 + 0.80 * lit);
                    let bright = lit > 0.45;
                    let ink = if bright { t.bg } else { t.text };
                    (fill, ink, if bright { t.bg.with_alpha(170) } else { c.mix(t.text, 0.35) })
                }
                None => (t.surface, t.text_faint, t.text_faint),
            };
            if let (Some(c), true) = (pad.color, lit > 0.0) {
                cx.canvas.stroke_round_rect(cell, t.radius + 3, 3, c.with_alpha((150.0 * lit) as u8));
            }
            cx.canvas.fill_round_rect(body, t.radius, fill);
            if pad.color.is_none() {
                cx.canvas.stroke_round_rect(body, t.radius, 1, t.outline);
            }
            let inner = body.inset(12);
            let (top, rest) = inner.take_top(22);
            cx.text(FontId::Bold, t.size_small, top, Align::Left, sub, &format!("{}", i + 1));
            let (bottom, _) = rest.take_bottom(26);
            let label = if pad.label.is_empty() { "—" } else { pad.label.as_str() };
            cx.text(FontId::Sans, t.size_body, bottom, Align::Left, ink, label);
            if lit > 0.0 && pad.velocity > 0 {
                let w = ((body.w - 24) as f32 * pad.velocity as f32 / 127.0) as i32;
                let bar = Rect::new(body.x + 12, body.bottom() - 9, w.max(4), 4);
                cx.canvas.fill_round_rect(bar, 2, ink.with_alpha((200.0 * lit) as u8));
            }
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let i = (0..self.pads.len()).find(|&i| self.cell(r, i).contains(x, y))?;
                self.fingers.push((finger, i));
                Some(EventKind::Pad { index: i, down: true })
            }
            Touch::Move => None,
            Touch::Up | Touch::Cancel => {
                let k = self.fingers.iter().position(|(f, _)| *f == finger)?;
                let (_, i) = self.fingers.remove(k);
                Some(EventKind::Pad { index: i, down: false })
            }
        }
    }
}

// ---------------------------------------------------------------- list

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListItem {
    pub text: String,
    pub detail: String,
    pub color: Option<Color>,
}

/// A titled list with one selected row, kept in view.
pub struct ListView {
    pub title: String,
    pub items: Vec<ListItem>,
    pub selected: usize,
}

const ROW_H: i32 = 46;

impl ListView {
    pub fn new(title: &str) -> ListView {
        ListView { title: title.to_string(), items: Vec::new(), selected: 0 }
    }

    /// The area for rows: the panel without its title line.
    fn body(r: Rect) -> Rect {
        // Matches PaintCx::titled_panel with the default theme's padding.
        r.inset(14).take_top(34).1
    }

    fn first(&self, visible: usize) -> usize {
        if self.items.len() <= visible || visible == 0 {
            return 0;
        }
        let half = visible / 2;
        self.selected.saturating_sub(half).min(self.items.len() - visible)
    }
}

impl Widget for ListView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let count = if self.items.is_empty() { String::new() } else { format!("{} / {}", self.selected + 1, self.items.len()) };
        let body = cx.titled_panel(r, &self.title, &count);
        let visible = (body.h / ROW_H).max(0) as usize;
        let first = self.first(visible);
        for (k, item) in self.items.iter().enumerate().skip(first).take(visible) {
            let row = Rect::new(body.x, body.y + (k - first) as i32 * ROW_H, body.w, ROW_H - 4);
            if k == self.selected {
                cx.canvas.fill_round_rect(row, 10, t.raised);
                cx.canvas.fill_round_rect(Rect::new(row.x, row.y + 10, 4, row.h - 20), 2, t.accent);
            }
            let mut text = row.inset_xy(14, 0);
            if let Some(c) = item.color {
                cx.canvas.fill_round_rect(Rect::new(text.x, row.y + row.h / 2 - 6, 12, 12), 4, c);
                text = text.take_left(22).1;
            }
            let dw = if item.detail.is_empty() { 0 } else { cx.measure(FontId::Sans, t.size_small, &item.detail) + 12 };
            let (detail, name) = text.take_right(dw);
            let font = if k == self.selected { FontId::Bold } else { FontId::Sans };
            cx.text(font, t.size_body, name, Align::Left, t.text, &item.text);
            cx.text(FontId::Sans, t.size_small, detail, Align::Right, t.text_dim, &item.detail);
        }
        if self.items.is_empty() {
            cx.text(FontId::Sans, t.size_body, body.take_top(ROW_H).0, Align::Left, t.text_faint, "nothing here");
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let body = ListView::body(r);
        if !body.contains(x, y) {
            return None;
        }
        let visible = (body.h / ROW_H).max(0) as usize;
        let k = self.first(visible) + ((y - body.y) / ROW_H) as usize;
        if k < self.items.len() && k < self.first(visible) + visible {
            self.selected = k;
            Some(EventKind::Select(k))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------- fader

/// A horizontal slider with a label and a value text.
pub struct Fader {
    pub label: String,
    /// 0.0 to 1.0.
    pub value: f32,
    pub text: String,
}

impl Fader {
    pub fn new(label: &str) -> Fader {
        Fader { label: label.to_string(), value: 0.0, text: String::new() }
    }

    fn track(r: Rect) -> Rect {
        let inner = r.inset(14);
        let body = inner.take_top(34).1;
        let y = body.y + (body.h - 10) / 2;
        Rect::new(body.x + 11, y.max(body.y), (body.w - 22).max(1), 10)
    }
}

impl Widget for Fader {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let inner = r.inset(t.pad);
        let (head, _) = inner.take_top(26);
        cx.text(FontId::Bold, t.size_small, head, Align::Left, t.text_dim, &self.label.to_uppercase());
        cx.text(FontId::Bold, t.size_body, head, Align::Right, t.text, &self.text);
        let tr = Fader::track(r);
        cx.canvas.fill_round_rect(tr, 5, t.raised);
        let v = self.value.clamp(0.0, 1.0);
        let fill = Rect::new(tr.x, tr.y, ((tr.w as f32) * v) as i32, tr.h);
        cx.canvas.fill_round_rect(fill, 5, t.accent);
        let kx = tr.x + fill.w;
        cx.canvas.fill_round_rect(Rect::new(kx - 11, tr.y - 6, 22, 22), 11, t.text);
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, _y: i32, r: Rect) -> Option<EventKind> {
        if !matches!(phase, Touch::Down | Touch::Move) {
            return None;
        }
        let tr = Fader::track(r);
        self.value = ((x - tr.x) as f32 / tr.w as f32).clamp(0.0, 1.0);
        Some(EventKind::Value(self.value))
    }
}

// ---------------------------------------------------------------- stats

/// A titled list of name/value rows.
pub struct StatList {
    pub title: String,
    pub rows: Vec<(String, String, Tone)>,
}

impl StatList {
    pub fn new(title: &str) -> StatList {
        StatList { title: title.to_string(), rows: Vec::new() }
    }

    pub fn set(&mut self, rows: Vec<(String, String, Tone)>) {
        self.rows = rows;
    }
}

impl Widget for StatList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let mut body = cx.titled_panel(r, &self.title, "");
        for (k, v, tone) in &self.rows {
            let (line, rest) = body.take_top(26);
            if line.h < 20 {
                break;
            }
            cx.text(FontId::Sans, t.size_small, line, Align::Left, t.text_dim, k);
            let kw = cx.measure(FontId::Sans, t.size_small, k) + 16;
            cx.text(FontId::Bold, t.size_small, line.take_left(kw).1, Align::Right, t.tone(*tone), v);
            body = rest;
        }
    }
}

// ---------------------------------------------------------------- log

/// A titled, scrolling list of lines, newest at the bottom.
pub struct LogPanel {
    pub title: String,
    pub right: String,
    pub mono: bool,
    lines: VecDeque<(String, Tone)>,
    max: usize,
}

impl LogPanel {
    pub fn new(title: &str, mono: bool, max: usize) -> LogPanel {
        LogPanel { title: title.to_string(), right: String::new(), mono, lines: VecDeque::new(), max: max.max(1) }
    }

    pub fn push(&mut self, line: &str, tone: Tone) {
        if self.lines.len() >= self.max {
            self.lines.pop_front();
        }
        let clean: String = line.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        self.lines.push_back((clean, tone));
    }

    pub fn lines(&self) -> impl Iterator<Item = &(String, Tone)> {
        self.lines.iter()
    }
}

impl Widget for LogPanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let body = cx.titled_panel(r, &self.title, &self.right);
        let (font, px) = if self.mono { (FontId::Mono, t.size_mono) } else { (FontId::Sans, t.size_small) };
        let lh = (px * 1.45).round() as i32;
        let visible = (body.h / lh).max(0) as usize;
        let skip = self.lines.len().saturating_sub(visible);
        for (k, (line, tone)) in self.lines.iter().skip(skip).enumerate() {
            let row = Rect::new(body.x, body.y + k as i32 * lh, body.w, lh);
            cx.text(font, px, row, Align::Left, t.tone(*tone), line);
        }
    }
}

// ---------------------------------------------------------------- label

/// One line of text, no panel.
pub struct Label {
    pub text: String,
    pub tone: Tone,
    pub align: Align,
}

impl Label {
    pub fn new(text: &str, tone: Tone, align: Align) -> Label {
        Label { text: text.to_string(), tone, align }
    }
}

impl Widget for Label {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let (px, c) = (cx.theme.size_small, cx.theme.tone(self.tone));
        cx.text(FontId::Sans, px, r.inset_xy(6, 0), self.align, c, &self.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fonts, SoftCanvas, Theme, Ui};

    #[test]
    fn pads_are_laid_out_like_the_mpc() {
        let g = PadGrid::new(16, 4);
        let r = Rect::new(0, 0, 400, 400);
        let p1 = g.cell(r, 0);
        let p16 = g.cell(r, 15);
        let p13 = g.cell(r, 12);
        assert!(p1.x == 0 && p1.bottom() > 380, "pad 1 bottom left: {p1:?}");
        assert!(p16.right() > 380 && p16.y == 0, "pad 16 top right: {p16:?}");
        assert_eq!((p13.x, p13.y), (0, 0));
    }

    #[test]
    fn touching_pads_presses_and_releases_them() {
        let mut g = PadGrid::new(16, 4);
        let r = Rect::new(0, 0, 400, 400);
        let c = g.cell(r, 5);
        assert_eq!(g.touch(3, Touch::Down, c.x + 5, c.y + 5, r), Some(EventKind::Pad { index: 5, down: true }));
        assert_eq!(g.touch(3, Touch::Move, 0, 0, r), None);
        assert_eq!(g.touch(3, Touch::Up, 0, 0, r), Some(EventKind::Pad { index: 5, down: false }));
        assert_eq!(g.touch(3, Touch::Up, 0, 0, r), None);
        // The gap between pads plays nothing.
        assert_eq!(g.touch(4, Touch::Down, c.right() + 2, c.y + 5, r), None);
    }

    #[test]
    fn only_changed_pads_are_redrawn() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 640, 400);
        ui.add("pads", PadGrid::new(16, 4));
        ui.add_page("p", "pads").unwrap();
        let mut c = SoftCanvas::new(640, 400);
        ui.paint(&mut c);
        ui.get_mut::<PadGrid>("pads").unwrap().hit(4, 90);
        let r = ui.rect_of("pads").unwrap();
        let cell = ui.get::<PadGrid>("pads").unwrap().cell(r, 4);
        assert_eq!(ui.paint(&mut c), vec![cell]);
        ui.get_mut::<PadGrid>("pads").unwrap(); // touched, nothing changed
        assert!(ui.paint(&mut c).is_empty());
    }

    #[test]
    fn pads_fade_after_release() {
        let mut g = PadGrid::new(16, 4);
        g.hit(2, 100);
        g.fade(1.0);
        assert_eq!(g.pads[2].level, 1.0, "held pads stay lit");
        g.release(2);
        assert!(g.fading());
        g.fade(0.1);
        assert!(g.pads[2].level < 1.0 && g.pads[2].level > 0.0);
        g.fade(1.0);
        assert!(!g.fading());
    }

    #[test]
    fn list_rows_select_and_scroll() {
        let mut l = ListView::new("Kits");
        l.items = (0..20).map(|i| ListItem { text: format!("kit {i}"), ..Default::default() }).collect();
        let r = Rect::new(0, 0, 300, 14 * 2 + 34 + ROW_H * 5);
        let body = ListView::body(r);
        assert_eq!(l.touch(1, Touch::Down, body.x + 5, body.y + ROW_H + 5, r), Some(EventKind::Select(1)));
        l.selected = 15;
        assert_eq!(l.first(5), 13);
        l.selected = 19;
        assert_eq!(l.first(5), 15);
        assert_eq!(l.touch(1, Touch::Down, body.x + 5, body.y + 5, r), Some(EventKind::Select(15)));
    }

    #[test]
    fn faders_follow_the_finger() {
        let mut f = Fader::new("Level");
        let r = Rect::new(0, 0, 300, 90);
        let tr = Fader::track(r);
        assert_eq!(f.touch(1, Touch::Down, tr.x, 50, r), Some(EventKind::Value(0.0)));
        assert_eq!(f.touch(1, Touch::Move, tr.right() + 50, 50, r), Some(EventKind::Value(1.0)));
        assert_eq!(f.touch(1, Touch::Up, 0, 50, r), None);
    }

    #[test]
    fn tabs_switch() {
        let mut h = Header::new(&["Play", "Debug"]);
        let r = Rect::new(0, 0, 1280, 60);
        let t1 = h.tab_rect(r, 1);
        assert_eq!(h.touch(1, Touch::Down, t1.x + 3, t1.y + 3, r), Some(EventKind::Tab(1)));
        assert_eq!(h.active, 1);
        assert_eq!(h.touch(1, Touch::Down, 5, 5, r), None);
    }

    #[test]
    fn logs_keep_the_newest_lines() {
        let mut l = LogPanel::new("Events", true, 3);
        for i in 0..5 {
            l.push(&format!("line {i}\t!"), Tone::Normal);
        }
        let got: Vec<&str> = l.lines().map(|(s, _)| s.as_str()).collect();
        assert_eq!(got, vec!["line 2 !", "line 3 !", "line 4 !"]);
    }

    #[test]
    fn every_widget_paints_inside_its_rectangle() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 640, 400);
        let mut h = Header::new(&["Play", "Debug"]);
        h.chips.push(Chip::new("session", Tone::Good));
        ui.add("header", h);
        let mut g = PadGrid::new(16, 4);
        g.pads[0] = Pad { label: "Kick".into(), color: Some(Color::rgb(0xFF0000)), level: 1.0, held: true, velocity: 90 };
        ui.add("pads", g);
        let mut l = ListView::new("Kits");
        l.items.push(ListItem { text: "House".into(), detail: "16 pads".into(), color: Some(Color::rgb(0x00FF00)) });
        ui.add("kits", l);
        ui.add("level", Fader::new("Level"));
        let mut s = StatList::new("Timing");
        s.set(vec![("Underruns".into(), "0".into(), Tone::Good)]);
        ui.add("stats", s);
        let mut log = LogPanel::new("MIDI", true, 10);
        log.push("99 25 40 pad 1", Tone::Normal);
        ui.add("log", log);
        ui.add("hint", Label::new("wheel: kit", Tone::Dim, Align::Center));
        ui.add_page("p", "col(header:60, row(pads:*, col(kits, level:90, stats, log)):*, hint:24)").unwrap();
        let mut c = SoftCanvas::new(640, 400);
        ui.paint(&mut c);
        // Nothing drawn in the outer margin.
        let bg = ui.theme.bg;
        for x in 0..640 {
            assert_eq!(c.pixel(x, 2), bg, "top margin at x {x}");
            assert_eq!(c.pixel(x, 397), bg, "bottom margin at x {x}");
        }
    }
}
