//! The table every section is drawn as: headings, rows with a title, a
//! note and up to four fields, and a picker that opens over the section.
//!
//! - A tap on a field selects it and does its job: a choice moves to the
//!   next value, a picker field opens its picker. A disabled field is only
//!   selected.
//! - The data wheel moves between rows, the page arrows by a page, and
//!   - and + step the selected field.
//! - The Q-Links take the selected row's fields, left to right.

use std::cell::Cell;
use std::time::Duration;

use mpfree_mixer_ui::{IoInput, IoKnob, IoReply, IoSection};
use mpfree_ui::{Align, FontId, PaintCx, Rect, Touch};

use crate::{IoModel, Shared};

/// How a row's note is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Normal,
    Faint,
    /// Something won't happen (unplugged, not built yet).
    Warn,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    pub text: String,
    pub enabled: bool,
    pub warn: bool,
    /// Where it is among its values (index, count), for its Q-Link's arc.
    pub at: Option<(usize, usize)>,
    /// A tap opens its picker instead of stepping it.
    pub picker: bool,
    /// The name its Q-Link card shows.
    pub knob: &'static str,
}

impl Field {
    pub fn choice(knob: &'static str, text: impl Into<String>, at: usize, of: usize) -> Field {
        Field {
            text: text.into(),
            enabled: true,
            warn: false,
            at: Some((at, of)),
            picker: false,
            knob,
        }
    }

    pub fn off(mut self, enabled: bool) -> Field {
        self.enabled = enabled;
        self
    }

    pub fn warn(mut self, warn: bool) -> Field {
        self.warn = warn;
        self
    }

    pub fn picker(mut self) -> Field {
        self.picker = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// A heading, with its column heads, rather than a row.
    pub heads: Option<Vec<&'static str>>,
    pub title: String,
    pub note: String,
    pub tone: Tone,
    /// An input light, lit or not.
    pub light: Option<bool>,
    pub fields: Vec<Field>,
    /// The key this row stands for, so selection follows it when rows come
    /// and go (a device plugged in).
    pub key: String,
}

impl Row {
    pub fn heading(title: &str, heads: Vec<&'static str>) -> Row {
        Row {
            heads: Some(heads),
            title: title.to_string(),
            note: String::new(),
            tone: Tone::Normal,
            light: None,
            fields: Vec::new(),
            key: format!("#{title}"),
        }
    }

    pub fn item(key: impl Into<String>, title: impl Into<String>) -> Row {
        Row {
            heads: None,
            title: title.into(),
            note: String::new(),
            tone: Tone::Normal,
            light: None,
            fields: Vec::new(),
            key: key.into(),
        }
    }

    pub fn note(mut self, note: impl Into<String>, tone: Tone) -> Row {
        self.note = note.into();
        self.tone = tone;
        self
    }

    pub fn light(mut self, lit: bool) -> Row {
        self.light = Some(lit);
        self
    }

    pub fn field(mut self, f: Field) -> Row {
        self.fields.push(f);
        self
    }

    fn is_item(&self) -> bool {
        self.heads.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub text: String,
    pub on: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Group {
    pub head: String,
    /// Each choice's width.
    pub w: i32,
    pub choices: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Picker {
    pub title: String,
    pub groups: Vec<Group>,
}

/// One section's rows and what its fields do.
pub(crate) trait Form: Send {
    fn label(&self) -> &'static str;
    /// The width of the title column, then of each field column.
    fn widths(&self) -> &'static [i32];
    /// A line over the rows (what isn't built yet, where tuning lives).
    fn banner(&self, m: &IoModel) -> Option<String>;
    fn rows(&self, m: &IoModel, now: Duration) -> Vec<Row>;
    /// Steps field `col` of the row with key `key` by `delta`. True when a
    /// setting changed.
    fn step(&self, m: &mut IoModel, key: &str, col: usize, delta: i32) -> bool;
    fn picker(&self, _m: &IoModel, _key: &str, _col: usize) -> Option<Picker> {
        None
    }
    /// Choice `item` of group `group` tapped in the picker.
    fn pick(&self, _m: &mut IoModel, _key: &str, _col: usize, _group: usize, _item: usize) -> bool {
        false
    }
    /// A number that changes when anything live on the screen changes
    /// (lights, the clock's tempo, volts), so a tick redraws only then.
    fn live(&self, _m: &IoModel, _now: Duration) -> u64 {
        0
    }
}

pub(crate) const BANNER_H: i32 = 40;
pub(crate) const HEAD_H: i32 = 42;
pub(crate) const ROW_H: i32 = 56;
pub(crate) const GAP: i32 = 8;
const FIELD_GAP: i32 = 10;
const SCROLL_W: i32 = 6;

/// The Done button, each choice (group, item, area) and each group's head.
type PickerLayout = (Rect, Vec<(usize, usize, Rect)>, Vec<(usize, Rect)>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Row(usize),
    Field(usize, usize),
    Choice(usize, usize),
    Done,
}

pub(crate) struct FormSection<F: Form> {
    form: F,
    model: Shared,
    /// The selected row's key and field.
    sel: Option<(String, usize)>,
    /// The first row shown.
    top: usize,
    now: Duration,
    live: u64,
    /// The picker open, on (row key, field).
    open: Option<(String, usize)>,
    down: Option<(u64, Option<Hit>)>,
    /// Rows that fit, from the last paint.
    fit: Cell<usize>,
}

impl<F: Form> FormSection<F> {
    pub fn new(form: F, model: Shared) -> FormSection<F> {
        FormSection {
            form,
            model,
            sel: None,
            top: 0,
            now: Duration::ZERO,
            live: 0,
            open: None,
            down: None,
            fit: Cell::new(8),
        }
    }

    fn rows(&self) -> (Option<String>, Vec<Row>) {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        (self.form.banner(&m), self.form.rows(&m, self.now))
    }

    /// The selected row's index, picking the first row when none is (or
    /// the one selected is gone).
    fn sel_index(&self, rows: &[Row]) -> Option<(usize, usize)> {
        let first = rows.iter().position(Row::is_item)?;
        let (i, col) = match &self.sel {
            Some((k, col)) => match rows.iter().position(|r| &r.key == k && r.is_item()) {
                Some(i) => (i, *col),
                None => (first, 0),
            },
            None => (first, 0),
        };
        let n = rows[i].fields.len();
        Some((i, if n == 0 { 0 } else { col.min(n - 1) }))
    }

    fn select(&mut self, rows: &[Row], i: usize, col: usize) {
        self.sel = Some((rows[i].key.clone(), col));
        let fit = self.fit.get().max(1);
        if i < self.top {
            // Show the heading over the first row of a block.
            self.top = if i > 0 && !rows[i - 1].is_item() {
                i - 1
            } else {
                i
            };
        } else if i >= self.top + fit {
            self.top = i + 1 - fit;
        }
    }

    /// Moves the selection by `d` rows (headings skipped).
    fn move_by(&mut self, d: i32) -> IoReply {
        let (_, rows) = self.rows();
        let Some((i, col)) = self.sel_index(&rows) else {
            return IoReply::Ignored;
        };
        let items: Vec<usize> = (0..rows.len()).filter(|&j| rows[j].is_item()).collect();
        let at = items.iter().position(|&j| j == i).unwrap_or(0) as i32;
        let to = (at + d).clamp(0, items.len() as i32 - 1) as usize;
        if items[to] == i && self.sel.is_some() {
            return IoReply::Handled;
        }
        self.select(&rows, items[to], col);
        IoReply::Redraw
    }

    fn step(&mut self, key: &str, col: usize, delta: i32) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if self.form.step(&mut m, key, col, delta) {
            m.gen += 1;
            IoReply::Redraw
        } else {
            IoReply::Handled
        }
    }

    fn step_selected(&mut self, delta: i32) -> IoReply {
        let (_, rows) = self.rows();
        let Some((i, col)) = self.sel_index(&rows) else {
            return IoReply::Ignored;
        };
        match rows[i].fields.get(col) {
            Some(f) if f.enabled => {
                let key = rows[i].key.clone();
                self.step(&key, col, delta)
            }
            _ => IoReply::Handled,
        }
    }

    /// Where each shown row goes in `r`, from `top`, and the field areas.
    fn place(&self, banner: bool, rows: &[Row], r: Rect) -> (Rect, Vec<(usize, Rect)>) {
        let (banner_r, body) = if banner {
            let (b, rest) = r.take_top(BANNER_H);
            (b, rest.take_top(GAP).1)
        } else {
            (Rect::new(r.x, r.y, r.w, 0), r)
        };
        let mut out = Vec::new();
        let mut y = body.y;
        let bottom = body.y + body.h;
        for (i, row) in rows.iter().enumerate().skip(self.top) {
            let h = if row.is_item() { ROW_H } else { HEAD_H };
            if y + h > bottom {
                break;
            }
            out.push((i, Rect::new(body.x, y, body.w - SCROLL_W - GAP, h)));
            y += h + if row.is_item() { GAP } else { 0 };
        }
        (banner_r, out)
    }

    fn field_rects(&self, row: Rect, n: usize) -> Vec<Rect> {
        let w = self.form.widths();
        let mut x = row.x + w[0];
        (0..n)
            .map(|c| {
                let fw = w.get(c + 1).copied().unwrap_or(160);
                let f = Rect::new(x, row.y + 8, fw, row.h - 16);
                x += fw + FIELD_GAP;
                f
            })
            .collect()
    }

    /// Where field `col` of the row `key` is drawn now, if it shows.
    #[cfg(test)]
    pub(crate) fn field_rect(&self, key: &str, col: usize, r: Rect) -> Option<Rect> {
        let (banner, rows) = self.rows();
        let (_, placed) = self.place(banner.is_some(), &rows, r);
        let (i, rr) = placed.into_iter().find(|p| rows[p.0].key == key)?;
        self.field_rects(rr, rows[i].fields.len()).get(col).copied()
    }

    fn picker_layout(p: &Picker, r: Rect) -> PickerLayout {
        let r = r.inset(16);
        let done = Rect::new(r.x + r.w - 120, r.y, 120, 44);
        let mut heads = Vec::new();
        let mut chips = Vec::new();
        let mut y = r.y + 60;
        for (g, group) in p.groups.iter().enumerate() {
            heads.push((g, Rect::new(r.x, y, r.w, 28)));
            y += 32;
            let mut x = r.x;
            for (i, _) in group.choices.iter().enumerate() {
                if x + group.w > r.x + r.w {
                    x = r.x;
                    y += 52;
                }
                chips.push((g, i, Rect::new(x, y, group.w, 44)));
                x += group.w + 8;
            }
            y += 52 + 12;
        }
        (done, chips, heads)
    }

    fn hit(&self, x: i32, y: i32, r: Rect) -> Option<Hit> {
        if let Some((key, col)) = &self.open {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            let p = self.form.picker(&m, key, *col)?;
            let (done, chips, _) = Self::picker_layout(&p, r);
            if done.contains(x, y) {
                return Some(Hit::Done);
            }
            return chips
                .iter()
                .find(|c| c.2.contains(x, y))
                .map(|c| Hit::Choice(c.0, c.1));
        }
        let (banner, rows) = self.rows();
        let (_, placed) = self.place(banner.is_some(), &rows, r);
        let (i, rr) = placed.into_iter().find(|p| p.1.contains(x, y))?;
        if !rows[i].is_item() {
            return None;
        }
        for (c, f) in self
            .field_rects(rr, rows[i].fields.len())
            .iter()
            .enumerate()
        {
            if f.contains(x, y) {
                return Some(Hit::Field(i, c));
            }
        }
        Some(Hit::Row(i))
    }

    fn tap(&mut self, hit: Hit) -> IoReply {
        match hit {
            Hit::Done => {
                self.open = None;
                IoReply::Redraw
            }
            Hit::Choice(g, i) => {
                let Some((key, col)) = self.open.clone() else {
                    return IoReply::Ignored;
                };
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                if self.form.pick(&mut m, &key, col, g, i) {
                    m.gen += 1;
                }
                IoReply::Redraw
            }
            Hit::Row(i) => {
                let (_, rows) = self.rows();
                let col = self.sel_index(&rows).map_or(0, |s| s.1);
                if i < rows.len() {
                    self.select(&rows, i, col);
                }
                IoReply::Redraw
            }
            Hit::Field(i, c) => {
                let (_, rows) = self.rows();
                let Some(row) = rows.get(i) else {
                    return IoReply::Ignored;
                };
                self.select(&rows, i, c);
                let f = &row.fields[c];
                if !f.enabled {
                    return IoReply::Redraw;
                }
                if f.picker {
                    self.open = Some((row.key.clone(), c));
                    return IoReply::Redraw;
                }
                let key = row.key.clone();
                self.step(&key, c, 1);
                IoReply::Redraw
            }
        }
    }

    fn paint_rows(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (banner, rows) = self.rows();
        let (banner_r, placed) = self.place(banner.is_some(), &rows, r);
        let fit = placed.iter().filter(|p| rows[p.0].is_item()).count();
        self.fit.set(fit.max(1));
        if let Some(b) = &banner {
            cx.canvas.fill_round_rect(banner_r, 10, t.surface);
            cx.canvas.fill_round_rect(
                Rect::new(banner_r.x, banner_r.y + 8, 4, banner_r.h - 16),
                2,
                t.warn,
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                banner_r.inset_xy(16, 0),
                Align::Left,
                t.text_dim,
                b,
            );
        }
        let sel = self.sel_index(&rows);
        for (i, rr) in &placed {
            let row = &rows[*i];
            if let Some(heads) = &row.heads {
                let hr = Rect::new(rr.x, rr.y + 14, rr.w, rr.h - 14);
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    hr.inset_xy(4, 0),
                    Align::Left,
                    t.text_dim,
                    &row.title,
                );
                for (h, fr) in heads.iter().zip(self.field_rects(*rr, heads.len())) {
                    let fr = Rect::new(fr.x, hr.y, fr.w, hr.h);
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        fr.inset_xy(4, 0),
                        Align::Left,
                        t.text_faint,
                        h,
                    );
                }
                continue;
            }
            let on = sel.is_some_and(|s| s.0 == *i);
            cx.canvas
                .fill_round_rect(*rr, 10, if on { t.raised } else { t.surface });
            if on {
                cx.canvas
                    .fill_round_rect(Rect::new(rr.x, rr.y + 10, 4, rr.h - 20), 2, t.accent);
            }
            let w0 = self.form.widths()[0];
            let mut tr = Rect::new(rr.x + 14, rr.y + 6, w0 - 20, rr.h - 12);
            if let Some(lit) = row.light {
                let d = Rect::new(tr.x, tr.y + 9, 10, 10);
                cx.canvas
                    .fill_round_rect(d, 5, if lit { t.good } else { t.outline });
                tr = Rect::new(tr.x + 18, tr.y, tr.w - 18, tr.h);
            }
            let (top, bottom) = tr.take_top(tr.h / 2);
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body,
                top,
                Align::Left,
                t.text,
                &row.title,
            );
            let tone = match row.tone {
                Tone::Normal => t.text_dim,
                Tone::Faint => t.text_faint,
                Tone::Warn => t.warn,
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                bottom,
                Align::Left,
                tone,
                &row.note,
            );
            for (c, (f, fr)) in row
                .fields
                .iter()
                .zip(self.field_rects(*rr, row.fields.len()))
                .enumerate()
            {
                let picked = on && sel.is_some_and(|s| s.1 == c);
                cx.canvas.fill_round_rect(fr, 8, t.bg);
                if picked {
                    cx.canvas.stroke_round_rect(fr, 8, 2, t.accent);
                }
                let ink = if !f.enabled {
                    t.text_faint
                } else if f.warn {
                    t.warn
                } else {
                    t.text
                };
                cx.text(
                    FontId::Sans,
                    t.size_body,
                    fr.inset_xy(12, 0),
                    Align::Left,
                    ink,
                    &f.text,
                );
            }
        }
        // The scroll bar, when the rows don't all fit.
        let shown = placed.len();
        if shown < rows.len() {
            let track = Rect::new(r.x + r.w - SCROLL_W, r.y, SCROLL_W, r.h);
            cx.canvas.fill_round_rect(track, 3, t.surface);
            let n = rows.len() as i32;
            let h = (track.h * shown as i32 / n).max(24);
            let y = track.y + (track.h - h) * self.top as i32 / (n - shown as i32).max(1);
            cx.canvas
                .fill_round_rect(Rect::new(track.x, y, SCROLL_W, h), 3, t.text_faint);
        }
    }

    fn paint_picker(&self, cx: &mut PaintCx, r: Rect, p: &Picker) {
        let t = cx.theme.clone();
        let (done, chips, heads) = Self::picker_layout(p, r);
        cx.canvas.fill_round_rect(r, 12, t.surface);
        let title = Rect::new(r.x + 16, r.y + 16, r.w - 160, 44);
        cx.text(
            FontId::Bold,
            t.size_body,
            title,
            Align::Left,
            t.text,
            &p.title,
        );
        cx.canvas.fill_round_rect(done, 10, t.accent);
        cx.text(
            FontId::Bold,
            t.size_body,
            done,
            Align::Center,
            t.on_accent,
            "Done",
        );
        for (g, hr) in heads {
            cx.text(
                FontId::Sans,
                t.size_small,
                hr,
                Align::Left,
                t.text_dim,
                &p.groups[g].head,
            );
        }
        for (g, i, cr) in chips {
            let c = &p.groups[g].choices[i];
            let (fill, ink) = if c.on {
                (t.accent, t.on_accent)
            } else if c.enabled {
                (t.raised, t.text)
            } else {
                (t.bg, t.text_faint)
            };
            cx.canvas.fill_round_rect(cr, 8, fill);
            cx.text(
                FontId::Sans,
                t.size_body,
                cr.inset_xy(10, 0),
                Align::Center,
                ink,
                &c.text,
            );
        }
    }
}

/// The hint line under the view while a section shows.
pub(crate) const HINT: &str = "Tap a setting to change it  ·  wheel: select a row  ·  - and +: step it  ·  Q-Links: the row's settings  ·  the I/O icon or TRACK MIX closes";
/// The hint line while a picker is open.
pub(crate) const HINT_PICKER: &str = "Tap a choice  ·  Done or Back closes";

impl<F: Form> IoSection for FormSection<F> {
    fn label(&self) -> &str {
        self.form.label()
    }

    fn hint(&self) -> Option<&str> {
        Some(if self.open.is_some() {
            HINT_PICKER
        } else {
            HINT
        })
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let picker = self.open.as_ref().and_then(|(key, col)| {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            self.form.picker(&m, key, *col)
        });
        match picker {
            Some(p) => self.paint_picker(cx, r, &p),
            None => self.paint_rows(cx, r),
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> IoReply {
        match phase {
            Touch::Down => {
                let h = self.hit(x, y, r);
                self.down = Some((finger, h));
                IoReply::Handled
            }
            Touch::Move => IoReply::Handled,
            Touch::Cancel => {
                self.down = None;
                IoReply::Handled
            }
            Touch::Up => {
                let Some((f, Some(h))) = self.down.take() else {
                    return IoReply::Handled;
                };
                if f != finger || self.hit(x, y, r) != Some(h) {
                    return IoReply::Handled;
                }
                self.tap(h)
            }
        }
    }

    fn input(&mut self, ev: IoInput) -> IoReply {
        if self.open.is_some() {
            return IoReply::Handled;
        }
        match ev {
            IoInput::Wheel(d) => self.move_by(d),
            IoInput::Page(d) => self.move_by(d * self.fit.get() as i32),
            IoInput::Step(d) => self.step_selected(d),
        }
    }

    fn tick(&mut self, now: Duration) -> IoReply {
        self.now = now;
        let live = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            self.form.live(&m, now)
        };
        if live != self.live {
            self.live = live;
            IoReply::Redraw
        } else {
            IoReply::Ignored
        }
    }

    fn shown(&mut self, on: bool) {
        if !on {
            self.open = None;
            self.down = None;
        }
    }

    fn back(&mut self) -> bool {
        self.open.take().is_some()
    }

    fn qlinks(&self) -> Option<[IoKnob; 4]> {
        if self.open.is_some() {
            return None;
        }
        let (_, rows) = self.rows();
        let (i, _) = self.sel_index(&rows)?;
        let row = &rows[i];
        if row.fields.is_empty() {
            return None;
        }
        Some(std::array::from_fn(|k| match row.fields.get(k) {
            Some(f) => IoKnob {
                label: f.knob.to_string(),
                text: f.text.clone(),
                pos: f.at.map_or(0.0, |(a, n)| {
                    if n > 1 {
                        a as f32 / (n - 1) as f32
                    } else {
                        0.0
                    }
                }),
                enabled: f.enabled,
            },
            None => IoKnob {
                label: String::new(),
                text: String::new(),
                pos: 0.0,
                enabled: false,
            },
        }))
    }

    fn qlink_turn(&mut self, knob: usize, detents: i32, fine: bool) -> IoReply {
        if self.open.is_some() || detents == 0 {
            return IoReply::Ignored;
        }
        let (_, rows) = self.rows();
        let Some((i, _)) = self.sel_index(&rows) else {
            return IoReply::Ignored;
        };
        let row = &rows[i];
        match row.fields.get(knob) {
            Some(f) if f.enabled => {
                let key = row.key.clone();
                let d = if fine { detents.signum() } else { detents };
                self.step(&key, knob, d)
            }
            _ => IoReply::Ignored,
        }
    }
}

/// Steps `i` by `d` through `n` values, stopping at the ends.
pub(crate) fn clamp_step(i: usize, d: i32, n: usize) -> usize {
    (i as i64 + d as i64).clamp(0, n.saturating_sub(1) as i64) as usize
}

#[cfg(test)]
pub(crate) mod testing {
    //! Drives a section the way the I/O view does.
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme};

    /// The section's area on the 1280 x 800 screen (as the mixer gives it).
    pub const AREA: Rect = Rect::new(186, 64, 924, 692);

    pub fn paint(s: &dyn IoSection) -> SoftCanvas {
        let mut c = SoftCanvas::new(1280, 800);
        let theme = Theme::dark();
        let mut fonts = Fonts::embedded();
        let mut cx = PaintCx {
            canvas: &mut c,
            theme: &theme,
            fonts: &mut fonts,
        };
        s.paint(&mut cx, AREA);
        c
    }

    pub fn tap(s: &mut dyn IoSection, x: i32, y: i32) -> IoReply {
        s.touch(1, Touch::Down, x, y, AREA);
        s.touch(1, Touch::Up, x, y, AREA)
    }
}
