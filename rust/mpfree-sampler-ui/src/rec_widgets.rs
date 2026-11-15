//! The Rec tab's picture: the line input's level meters over the last half
//! minute of input, scrolling, with the bar lines, the span a skip back
//! would keep and the take being recorded marked on it. The page feeds the
//! [`Strip`] from the sampler's [`Input`] (only the frames that came since
//! the last frame, so it costs next to nothing) and puts what to mark in
//! the fields.

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use mpfree_sampler::record::Input;
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Widget};

/// The recording colour (the take being recorded, the record button's dot).
pub const REC_COLOR: u32 = 0xFF4D5E;

/// The input level the meters and the strip draw from (dBFS) up to 0.
pub const FLOOR_DB: f32 = -60.0;

/// Linear peak to dBFS (−∞ for silence).
pub fn db(x: f32) -> f32 {
    if x > 0.0 {
        20.0 * x.log10()
    } else {
        f32::NEG_INFINITY
    }
}

// ---------------------------------------------------------------- strip

/// The input's peaks in columns of a fixed number of frames, aligned on
/// the input's absolute frames, the latest `len` of them.
pub struct Strip {
    per: u64,
    len: usize,
    /// (column index, [left min, left max, right min, right max]).
    cols: VecDeque<(u64, [f32; 4])>,
    read_to: u64,
    scratch: Vec<f32>,
    peak: [f32; 2],
}

impl Strip {
    /// `len` columns of `per` frames each.
    pub fn new(per: u64, len: usize) -> Strip {
        Strip {
            per: per.max(1),
            len: len.max(1),
            cols: VecDeque::with_capacity(len.max(1)),
            read_to: 0,
            scratch: Vec::with_capacity(2 * per.max(1) as usize),
            peak: [0.0; 2],
        }
    }

    /// Frames per column.
    pub fn per(&self) -> u64 {
        self.per
    }

    /// Frames the strip spans.
    pub fn span(&self) -> u64 {
        self.per * self.len as u64
    }

    /// Forgets everything: the next update reads a new input from its start.
    pub fn reset(&mut self) {
        self.cols.clear();
        self.read_to = 0;
        self.peak = [0.0; 2];
    }

    /// Reads what the input has had since the last update (at first, as
    /// much of the strip's span as the input holds), a column at a time.
    pub fn update(&mut self, input: &Input) {
        let now = input.now();
        if now < self.read_to {
            // Another input, started since.
            self.reset();
        }
        let from = self
            .read_to
            .max(now.saturating_sub(self.span()))
            .max(input.audio.oldest());
        let mut at = from;
        while at < now {
            // To the end of this column, or now.
            let end = ((at / self.per + 1) * self.per).min(now);
            self.scratch.clear();
            if input.audio.copy(at, end, &mut self.scratch).is_err() {
                // Overwritten while we read (we fell more than the buffer
                // behind): start again from what it holds now.
                at = input.audio.oldest().max(end);
                continue;
            }
            let mut c = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
            for f in self.scratch.chunks_exact(2) {
                c[0] = c[0].min(f[0]);
                c[1] = c[1].max(f[0]);
                c[2] = c[2].min(f[1]);
                c[3] = c[3].max(f[1]);
                self.peak[0] = self.peak[0].max(f[0].abs());
                self.peak[1] = self.peak[1].max(f[1].abs());
            }
            let index = at / self.per;
            match self.cols.back_mut() {
                Some((i, last)) if *i == index => {
                    last[0] = last[0].min(c[0]);
                    last[1] = last[1].max(c[1]);
                    last[2] = last[2].min(c[2]);
                    last[3] = last[3].max(c[3]);
                }
                _ => {
                    if self.cols.len() == self.len {
                        self.cols.pop_front();
                    }
                    self.cols.push_back((index, c));
                }
            }
            at = end;
        }
        self.read_to = now.max(self.read_to);
    }

    /// The loudest sample on each side since the last call (linear).
    pub fn take_peak(&mut self) -> [f32; 2] {
        std::mem::take(&mut self.peak)
    }

    /// Column `index` (absolute frame / [`Strip::per`]), if kept.
    pub fn column(&self, index: u64) -> Option<[f32; 4]> {
        let first = self.cols.front()?.0;
        let (i, c) = self.cols.get(index.checked_sub(first)? as usize)?;
        (*i == index).then_some(*c)
    }

    /// The frame read up to.
    pub fn read_to(&self) -> u64 {
        self.read_to
    }
}

// ---------------------------------------------------------------- input view

/// A span of the input marked on the strip, with its label.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: u64,
    pub end: u64,
    pub label: String,
}

/// The meters and the strip. The strip shows `strip.span()` frames up to
/// `now` on the right.
pub struct InputView {
    pub title: String,
    pub right: String,
    pub strip: Strip,
    pub now: u64,
    pub rate: u32,
    /// The meters' levels (dBFS, falling back slowly).
    pub meter_db: [f32; 2],
    /// The threshold, drawn on the meters and the strip (linear).
    pub threshold: Option<f32>,
    /// Bar lines (absolute frames) to draw.
    pub bars: Vec<u64>,
    /// What a skip back would keep.
    pub skip: Option<Span>,
    /// The take being recorded (its end is `now` until it ends).
    pub take: Option<Span>,
    /// A line over the strip, such as why nothing comes in.
    pub note: String,
    pub caption: String,
    /// The look of the picture last painted ([`InputView::look`]).
    painted: Cell<Option<u64>>,
}

const METER_H: i32 = 52;
/// The meters' "L"/"R" column and their value column.
const LABEL_W: i32 = 22;
const VALUE_W: i32 = 76;

impl InputView {
    pub fn new(title: &str, strip: Strip, rate: u32) -> InputView {
        InputView {
            title: title.into(),
            right: String::new(),
            strip,
            now: 0,
            rate,
            meter_db: [f32::NEG_INFINITY; 2],
            threshold: None,
            bars: Vec::new(),
            skip: None,
            take: None,
            note: String::new(),
            caption: String::new(),
            painted: Cell::new(None),
        }
    }

    /// Reads the input and moves the meters: they jump up to a new peak
    /// and fall back about 20 dB a second (`dt` seconds since the last).
    pub fn update(&mut self, input: &Input, dt: f32) {
        self.strip.update(input);
        self.now = input.now();
        let peak = self.strip.take_peak();
        for (m, p) in self.meter_db.iter_mut().zip(peak) {
            let fallen = if m.is_finite() { *m - 20.0 * dt } else { *m };
            *m = db(p).max(fallen);
            if *m < FLOOR_DB - 6.0 {
                *m = f32::NEG_INFINITY;
            }
        }
    }

    /// What the picture in `r` depends on, as one number: two states with
    /// the same look paint the same pixels. The Rec tab redraws the view
    /// only when its look differs from the one painted
    /// ([`InputView::painted`]), so the strip counts in pixels, not frames:
    /// a silent input scrolling on, with nothing marked, draws nothing new.
    /// (The view spans most of the turned panel's rows, so each redraw
    /// rewrote 904 of its 1280 rows, 30 times a second, until 13 Nov.)
    pub fn look(&self, r: Rect) -> u64 {
        let mut h = DefaultHasher::new();
        (&self.title, &self.right, &self.note, &self.caption).hash(&mut h);
        let (meters, plot, _) = InputView::areas(r);
        let bar_w = meters.w - LABEL_W - VALUE_W;
        for lvl in self.meter_db {
            ((meter_place(lvl) * bar_w as f32) as i32, db_text(lvl)).hash(&mut h);
        }
        self.threshold.map(f32::to_bits).hash(&mut h);
        let first = self.first();
        for (tag, s) in [(0u8, &self.skip), (1, &self.take)] {
            if let Some(s) = s.as_ref().filter(|s| self.shows(s)) {
                let (x0, x1) = (self.x_of(plot, s.start), self.x_of(plot, s.end));
                (tag, x0, x1, s.start < first, &s.label).hash(&mut h);
            }
        }
        let edge = self.edge();
        for &b in &self.bars {
            if b > first && b <= edge {
                self.x_of(plot, b).hash(&mut h);
            }
        }
        // The strip as drawn: each pixel column's reach up and down, and
        // whether it is the take's colour.
        let w = plot.w.max(0) as usize;
        let mut px = vec![None; 2 * w];
        self.columns(plot, |side, x0, x1, up, down, take| {
            for x in x0..x1 {
                let at = usize::try_from(x - plot.x).ok().filter(|&i| i < w);
                if let Some(i) = at {
                    px[side * w + i] = Some((up, down, take));
                }
            }
        });
        px.hash(&mut h);
        h.finish()
    }

    /// The look of the picture last painted, by whatever asked for it (the
    /// Rec tab, a change of settings, the whole screen redrawn): what the
    /// screen shows now. None before the first paint.
    pub fn painted(&self) -> Option<u64> {
        self.painted.get()
    }

    /// The frame on the strip's right edge: `now` back to the last whole
    /// column, so the strip moves a column at a time and a silent input
    /// draws the same picture until something comes (with `now` itself,
    /// the newest column's edge crept and jumped back between columns).
    fn edge(&self) -> u64 {
        let per = self.strip.per();
        self.now / per * per
    }

    /// The oldest frame the strip shows.
    fn first(&self) -> u64 {
        self.edge().saturating_sub(self.strip.span().max(1))
    }

    /// Where frame `f` falls across `plot`: the edge on the right, the
    /// strip's span back on the left. On whole columns too, so a mark a
    /// fixed time back from now (Skip back) stays put.
    fn x_of(&self, plot: Rect, f: u64) -> i32 {
        let span = self.strip.span().max(1);
        let per = self.strip.per();
        let edge = self.edge();
        let back = edge - (f / per * per).clamp(self.first(), edge);
        plot.right() - (back as f64 / span as f64 * f64::from(plot.w)).round() as i32
    }

    /// True if any of `s` is on the strip.
    fn shows(&self, s: &Span) -> bool {
        s.end > self.first() && s.start <= self.edge()
    }

    /// The two lanes' middle line and half height in `plot` (left above,
    /// right below).
    fn lane(plot: Rect, side: usize) -> (i32, f32) {
        let lane_h = (plot.h - 2) / 2;
        let mid = plot.y + 1 + side as i32 * (lane_h + 2) + lane_h / 2;
        (mid, (lane_h / 2 - 2).max(1) as f32)
    }

    /// Each column kept, as drawn on each side: `each(side, x0, x1, up,
    /// down, in the take)`, oldest first.
    fn columns(&self, plot: Rect, mut each: impl FnMut(usize, i32, i32, i32, i32, bool)) {
        let span = self.strip.span().max(1);
        let first = self.first();
        let per = self.strip.per();
        let cols = (span / per) as i32;
        for side in 0..2usize {
            let (_, half) = InputView::lane(plot, side);
            for k in 0..cols {
                let index = first / per + k as u64;
                let Some(c) = self.strip.column(index) else {
                    continue;
                };
                let (lo, hi) = (c[2 * side], c[2 * side + 1]);
                if hi < lo {
                    continue;
                }
                let x0 = self.x_of(plot, index * per);
                let x1 = self.x_of(plot, (index + 1) * per).max(x0 + 1);
                let up = (lane_place(hi.max(0.0)) * half) as i32;
                let down = (lane_place(lo.min(0.0)) * half) as i32;
                let take = self
                    .take
                    .as_ref()
                    .is_some_and(|s| index * per >= s.start && index * per < s.end);
                each(side, x0, x1, up, down, take);
            }
        }
    }

    /// The meters' area and the strip's, inside the panel's body.
    pub fn areas(r: Rect) -> (Rect, Rect, Rect) {
        let body = r.inset(14).take_top(34).1;
        let (meters, rest) = body.take_top(METER_H);
        let (caption, strip) = rest.take_bottom(24);
        (
            meters,
            Rect::new(strip.x, strip.y + 8, strip.w, strip.h - 8),
            caption,
        )
    }
}

/// Place of `db` across a meter, 0 to 1.
fn meter_place(db: f32) -> f32 {
    if !db.is_finite() {
        return 0.0;
    }
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// How far up a lane a level reaches (linear, as a waveform is drawn).
fn lane_place(x: f32) -> f32 {
    x.abs().min(1.0)
}

/// A meter's level as text: whole dB, "0 dB" at full scale, "+1 dB" over.
fn db_text(db: f32) -> String {
    if !db.is_finite() {
        return "silent".to_string();
    }
    let d = db.round();
    if d >= 1.0 {
        format!("+{d:.0} dB")
    } else if d > -1.0 {
        "0 dB".to_string()
    } else {
        format!("−{:.0} dB", -d)
    }
}

impl Widget for InputView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let (meters, plot, caption) = InputView::areas(r);
        let rec = Color::rgb(REC_COLOR);
        let small = t.size_small - 1.0;
        // The meters: one bar a side, green, then amber above −12, red
        // above −3, with the threshold as a line across.
        let (label_w, value_w) = (LABEL_W, VALUE_W);
        let row_h = (meters.h - 8) / 2;
        for side in 0..2 {
            let y = meters.y + side as i32 * (row_h + 6);
            let lr = Rect::new(meters.x, y, label_w, row_h);
            cx.text(
                FontId::Bold,
                small,
                lr,
                Align::Left,
                t.text_dim,
                if side == 0 { "L" } else { "R" },
            );
            let bar = Rect::new(
                meters.x + label_w,
                y + 2,
                meters.w - label_w - value_w,
                row_h - 4,
            );
            cx.canvas.fill_round_rect(bar, 4, t.bg);
            let lvl = self.meter_db[side];
            let w = (meter_place(lvl) * bar.w as f32) as i32;
            for (from_db, color) in [(FLOOR_DB, t.good), (-12.0, t.warn), (-3.0, t.bad)] {
                let x0 = bar.x + (meter_place(from_db) * bar.w as f32) as i32;
                let x1 = bar.x + w;
                if x1 > x0 {
                    cx.canvas
                        .fill_rect(Rect::new(x0, bar.y, x1 - x0, bar.h), color);
                }
            }
            for tick in [-48.0, -36.0, -24.0, -12.0, -6.0] {
                let x = bar.x + (meter_place(tick) * bar.w as f32) as i32;
                cx.canvas
                    .fill_rect(Rect::new(x, bar.y, 1, bar.h), t.outline);
            }
            if let Some(th) = self.threshold {
                let x = bar.x + (meter_place(db(th)) * bar.w as f32) as i32;
                cx.canvas
                    .fill_rect(Rect::new(x - 1, bar.y - 2, 3, bar.h + 4), t.accent);
            }
            let text = db_text(lvl);
            let vr = Rect::new(bar.right() + 8, y, value_w - 8, row_h);
            cx.text(FontId::Mono, small, vr, Align::Right, t.text_dim, &text);
        }
        // The strip: left above, right below, newest on the right.
        cx.canvas.fill_round_rect(plot, 8, t.bg);
        let span = self.strip.span().max(1);
        let (first, edge) = (self.first(), self.edge());
        // Now is on the right edge, `span` frames back on the left.
        let x_of = |f: u64| self.x_of(plot, f);
        let mark = |cx: &mut PaintCx, s: &Span, fill: Color, line: Color| {
            if !self.shows(s) {
                return;
            }
            let (x0, x1) = (x_of(s.start), x_of(s.end));
            cx.canvas
                .fill_rect(Rect::new(x0, plot.y, (x1 - x0).max(2), plot.h), fill);
            cx.canvas.fill_rect(Rect::new(x0, plot.y, 2, plot.h), line);
            let label = if s.start < first {
                format!("← {}", s.label)
            } else {
                s.label.clone()
            };
            let lr = Rect::new(x0 + 6, plot.y + 4, (plot.right() - x0 - 10).max(0), 18);
            cx.text(FontId::Bold, small, lr, Align::Left, line, &label);
        };
        if let Some(s) = &self.skip {
            mark(cx, s, t.bg.mix(t.accent, 0.16), t.accent);
        }
        if let Some(s) = &self.take {
            mark(cx, s, t.bg.mix(rec, 0.22), rec);
        }
        // Bar lines: a faint line through the strip (and, over the
        // waveform, a tick at each edge).
        let bar_line = t.bg.mix(t.text_faint, 0.35);
        for &b in &self.bars {
            if b > first && b <= edge {
                cx.canvas
                    .fill_rect(Rect::new(x_of(b), plot.y, 1, plot.h), bar_line);
            }
        }
        let wave = t.text_dim;
        // Each lane's middle line and the threshold; the lanes do not
        // overlap, so the columns can go on after both.
        for side in 0..2usize {
            let (mid, half) = InputView::lane(plot, side);
            cx.canvas
                .fill_rect(Rect::new(plot.x + 4, mid, plot.w - 8, 1), t.outline);
            if let Some(th) = self.threshold {
                let dy = (lane_place(th) * half) as i32;
                for y in [mid - dy, mid + dy] {
                    let mut x = plot.x + 4;
                    while x < plot.right() - 4 {
                        cx.canvas.fill_rect(Rect::new(x, y, 6, 1), t.accent);
                        x += 12;
                    }
                }
            }
        }
        self.columns(plot, |side, x0, x1, up, down, take| {
            let (mid, _) = InputView::lane(plot, side);
            let color = if take { rec } else { wave };
            cx.canvas
                .fill_rect(Rect::new(x0, mid - up, x1 - x0, (up + down).max(1)), color);
        });
        // Seconds back from now under the strip.
        for &b in &self.bars {
            if b > first && b <= edge {
                let x = x_of(b);
                for y in [plot.y, plot.bottom() - 8] {
                    cx.canvas.fill_rect(Rect::new(x - 1, y, 3, 8), t.text_dim);
                }
            }
        }
        let rate = u64::from(self.rate.max(1));
        let secs = span / rate;
        let step = if secs > 40 { 10 } else { 5 };
        for s in (0..=secs).step_by(step as usize) {
            let x = plot.right() - (s * rate) as i32 * plot.w / span as i32;
            let text = if s == 0 {
                "now".to_string()
            } else {
                format!("−{s} s")
            };
            let w = 60;
            let lr = Rect::new(
                (x - w / 2).clamp(caption.x, (caption.right() - w).max(caption.x)),
                caption.y,
                w,
                caption.h,
            );
            cx.text(FontId::Sans, small, lr, Align::Center, t.text_faint, &text);
        }
        if !self.note.is_empty() {
            let nr = Rect::new(plot.x + 20, plot.y + plot.h / 2 - 16, plot.w - 40, 32);
            cx.canvas.fill_round_rect(nr.inset(-6), 8, t.surface);
            cx.text(
                FontId::Bold,
                t.size_body,
                nr,
                Align::Center,
                t.text,
                &self.note,
            );
        }
        if !self.caption.is_empty() {
            let cr = Rect::new(plot.x + 8, plot.bottom() - 24, plot.w - 16, 20);
            cx.text(
                FontId::Sans,
                small,
                cr,
                Align::Right,
                t.text_dim,
                &self.caption,
            );
        }
        self.painted.set(Some(self.look(r)));
    }
}
