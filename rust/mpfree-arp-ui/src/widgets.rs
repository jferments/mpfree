//! Widgets the arpeggiator's pages add to mpfree-ui's set. Like mpfree-ui's
//! own, they only show what the page put in their fields and report touches.

use mpfree_ui::{Align, EventKind, PaintCx, Rect, Tone, Touch, Widget};
use mpfree_ui::text::FontId;

// ---------------------------------------------------------------- segments

/// A row of equal tabs, one of them active (Direct, Repeat, Arp, Pattern).
/// A tap on tab `i` gives `Tab(i)`.
pub struct Segments {
    pub items: Vec<String>,
    pub active: usize,
}

impl Segments {
    pub fn new(items: &[&str]) -> Segments {
        Segments { items: items.iter().map(|s| s.to_string()).collect(), active: 0 }
    }

    pub fn cell(&self, r: Rect, i: usize) -> Rect {
        let n = self.items.len().max(1) as i32;
        let w = r.w / n;
        Rect::new(r.x + i as i32 * w, r.y, if i as i32 == n - 1 { r.w - w * (n - 1) } else { w }, r.h)
    }
}

impl Widget for Segments {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        for (i, name) in self.items.iter().enumerate() {
            let c = self.cell(r, i).inset_xy(3, 2);
            let on = i == self.active;
            if on {
                cx.canvas.fill_round_rect(c, 10, t.raised);
                let bar = Rect::new(c.x + 8, c.bottom() - 3, c.w - 16, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let font = if on { FontId::Bold } else { FontId::Sans };
            cx.text(font, t.size_body, c, Align::Center, if on { t.text } else { t.text_dim }, name);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.items.len()).find(|&i| self.cell(r, i).contains(x, y))?;
        self.active = i;
        Some(EventKind::Tab(i))
    }
}

// ---------------------------------------------------------------- sliders

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slider {
    pub label: String,
    /// 0.0 to 1.0.
    pub value: f32,
    pub text: String,
}

/// A titled panel of horizontal sliders, such as the four Q-Links. A finger
/// that goes down on row `i` gives `Select(i)`, then `Value(v)` for that row
/// as it moves, until it lifts.
pub struct SliderStack {
    pub title: String,
    pub rows: Vec<Slider>,
    /// The row that is being turned (by a Q-Link or a finger), highlighted.
    pub active: Option<usize>,
    grabbed: Option<(u64, usize)>,
}

const SLIDER_HEAD: i32 = 20;

impl SliderStack {
    pub fn new(title: &str, labels: &[&str]) -> SliderStack {
        SliderStack {
            title: title.to_string(),
            rows: labels.iter().map(|l| Slider { label: l.to_string(), ..Slider::default() }).collect(),
            active: None,
            grabbed: None,
        }
    }

    fn body(r: Rect) -> Rect {
        // Matches PaintCx::titled_panel with the default theme's padding.
        r.inset(14).take_top(34).1
    }

    /// Row `i`'s area and its track.
    pub fn row_rects(&self, r: Rect, i: usize) -> (Rect, Rect) {
        let body = SliderStack::body(r);
        let n = self.rows.len().max(1) as i32;
        let h = body.h / n;
        let row = Rect::new(body.x, body.y + i as i32 * h, body.w, h);
        let track = Rect::new(row.x + 11, row.y + SLIDER_HEAD + (h - SLIDER_HEAD - 8) / 2, (row.w - 22).max(1), 8);
        (row, track)
    }

    fn value_at(&self, r: Rect, i: usize, x: i32) -> f32 {
        let (_, track) = self.row_rects(r, i);
        ((x - track.x) as f32 / track.w as f32).clamp(0.0, 1.0)
    }
}

impl Widget for SliderStack {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, "");
        for (i, s) in self.rows.iter().enumerate() {
            let (row, track) = self.row_rects(r, i);
            let on = self.active == Some(i);
            let head = Rect::new(row.x, row.y, row.w, SLIDER_HEAD);
            let ink = if on { t.accent } else { t.text_dim };
            cx.text(FontId::Bold, t.size_small - 1.0, head, Align::Left, ink, &s.label.to_uppercase());
            cx.text(FontId::Bold, t.size_body, head, Align::Right, t.text, &s.text);
            cx.canvas.fill_round_rect(track, 4, t.raised);
            let v = s.value.clamp(0.0, 1.0);
            let fill = Rect::new(track.x, track.y, ((track.w as f32) * v) as i32, track.h);
            cx.canvas.fill_round_rect(fill, 4, t.accent);
            let kx = track.x + fill.w;
            cx.canvas.fill_round_rect(Rect::new(kx - 9, track.y - 5, 18, 18), 9, t.text);
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let i = (0..self.rows.len()).find(|&i| self.row_rects(r, i).0.contains(x, y))?;
                self.grabbed = Some((finger, i));
                self.active = Some(i);
                self.rows[i].value = self.value_at(r, i, x);
                Some(EventKind::Select(i))
            }
            Touch::Move => {
                let (f, i) = self.grabbed?;
                if f != finger {
                    return None;
                }
                let v = self.value_at(r, i, x);
                self.rows[i].value = v;
                Some(EventKind::Value(v))
            }
            Touch::Up | Touch::Cancel => {
                if self.grabbed.is_some_and(|(f, _)| f == finger) {
                    self.grabbed = None;
                }
                None
            }
        }
    }
}

// ---------------------------------------------------------------- steps

/// What kind of step a cell shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellKind {
    #[default]
    Note,
    /// Several notes at once.
    Chord,
    Rest,
    /// Holds the note before it.
    Tie,
    /// Not known ahead (a random pick).
    Random,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepCell {
    pub kind: CellKind,
    /// The big text: the pad, the note or the pick ("1", "x", "?").
    pub label: String,
    /// The small line under it ("oct +1", "×3", "70 %").
    pub sub: String,
    pub accent: bool,
}

/// The steps of the arp's order or of the pattern, in one row, with the
/// step that is playing marked. A tap on cell `i` gives `Select(first + i)`.
pub struct StepStrip {
    pub cells: Vec<StepCell>,
    /// The step number of the first cell, when the pattern is longer than
    /// the strip shows.
    pub first: usize,
    /// The cell that is playing, if any.
    pub playing: Option<usize>,
    /// The cell being edited or recorded next, if any.
    pub cursor: Option<usize>,
    /// Cells shown even when fewer steps exist, so cells keep their width.
    pub min_cells: usize,
}

impl StepStrip {
    pub fn new(min_cells: usize) -> StepStrip {
        StepStrip { cells: Vec::new(), first: 0, playing: None, cursor: None, min_cells }
    }

    pub fn cell(&self, r: Rect, i: usize) -> Rect {
        let n = self.cells.len().max(self.min_cells).max(1) as i32;
        let gap = 6;
        let w = (r.w - gap * (n - 1)) / n;
        Rect::new(r.x + i as i32 * (w + gap), r.y, w, r.h)
    }
}

impl Widget for StepStrip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        if self.cells.is_empty() {
            cx.text(FontId::Sans, t.size_body, r, Align::Center, t.text_faint, "hold pads to hear the steps");
            return;
        }
        for (i, c) in self.cells.iter().enumerate() {
            let cell = self.cell(r, i);
            let quiet = matches!(c.kind, CellKind::Rest | CellKind::Tie);
            cx.canvas.fill_round_rect(cell, 8, if quiet { t.surface } else { t.raised });
            if self.cursor == Some(i) {
                cx.canvas.stroke_round_rect(cell, 8, 2, t.accent);
            }
            if self.playing == Some(i) {
                cx.canvas.stroke_round_rect(cell, 8, 3, t.good);
            }
            if c.accent {
                let dot = Rect::new(cell.right() - 14, cell.y + 7, 7, 7);
                cx.canvas.fill_round_rect(dot, 3, t.warn);
            }
            let ink = match c.kind {
                CellKind::Rest | CellKind::Tie => t.text_faint,
                CellKind::Random => t.text_dim,
                CellKind::Chord => t.accent,
                CellKind::Note => t.text,
            };
            if c.sub.is_empty() {
                cx.text(FontId::Sans, t.size_title, cell, Align::Center, ink, &c.label);
            } else {
                let (top, bottom) = cell.inset_xy(2, 6).take_top(cell.h / 2 + 2);
                cx.text(FontId::Sans, t.size_title, top, Align::Center, ink, &c.label);
                cx.text(FontId::Mono, t.size_small - 2.0, bottom, Align::Center, t.text_dim, &c.sub);
            }
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.cells.len()).find(|&i| self.cell(r, i).contains(x, y))?;
        Some(EventKind::Select(self.first + i))
    }
}

// ---------------------------------------------------------------- buttons

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Button {
    pub label: String,
    pub tone: Tone,
    /// Shown greyed out, and taps do nothing.
    pub disabled: bool,
}

/// A panel of buttons that wrap onto as many lines as they need. A tap on
/// button `i` gives `Tab(i)`.
pub struct ButtonRow {
    pub buttons: Vec<Button>,
}

const BUTTON_H: i32 = 44;

impl ButtonRow {
    pub fn new(labels: &[(&str, Tone)]) -> ButtonRow {
        ButtonRow {
            buttons: labels
                .iter()
                .map(|(l, tone)| Button { label: l.to_string(), tone: *tone, disabled: false })
                .collect(),
        }
    }

    /// Buttons share each line equally, two per line.
    pub fn button_rect(&self, r: Rect, i: usize) -> Rect {
        let inner = r.inset(12);
        let per_line = 2;
        let gap = 10;
        let w = (inner.w - gap * (per_line - 1)) / per_line;
        let (line, col) = ((i as i32) / per_line, (i as i32) % per_line);
        Rect::new(inner.x + col * (w + gap), inner.y + line * (BUTTON_H + gap), w, BUTTON_H)
    }
}

impl Widget for ButtonRow {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        for (i, b) in self.buttons.iter().enumerate() {
            let br = self.button_rect(r, i);
            if br.bottom() > r.bottom() {
                break;
            }
            let fill = if b.disabled { t.surface } else { t.bg.mix(t.tone(b.tone), 0.18) };
            cx.canvas.fill_round_rect(br, 10, fill);
            if b.disabled {
                cx.canvas.stroke_round_rect(br, 10, 1, t.outline);
            }
            let ink = if b.disabled { t.text_faint } else { t.tone(b.tone) };
            cx.text(FontId::Bold, t.size_body, br, Align::Center, ink, &b.label);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        (0..self.buttons.len())
            .find(|&i| !self.buttons[i].disabled && self.button_rect(r, i).contains(x, y))
            .map(EventKind::Tab)
    }
}
