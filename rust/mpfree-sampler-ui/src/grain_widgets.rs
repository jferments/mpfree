//! The Grain tab's two pictures: the region with the grain cloud playing
//! over it, and the touch pad that moves the cloud's position and the
//! grains' size together. Like the other widgets they show what the page
//! put in their fields and report touches; a drag moves the picture at
//! once, and the page applies the value to the project and shows what was
//! kept.

use std::sync::Arc;

use mpfree_sampler::engine::GrainView;
use mpfree_ui::wave::Peaks;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Touch, Widget};

/// The granular colour (violet), shared by the tab's pictures and cards.
pub const GRAIN_COLOR: u32 = 0xC77DFF;
/// The colour of grains that play backwards.
pub const REVERSE_COLOR: u32 = 0x4DD4E8;

/// Below a titled panel's title, as `PaintCx::titled_panel` lays it out.
fn body(r: Rect) -> Rect {
    r.inset(14).take_top(34).1
}

// ---------------------------------------------------------------- cloud

/// The region's sound with the grain cloud over it: a band where grains
/// may start (round where the voice reads, spread by the spray), a line at
/// the position,
/// and each grain playing now as a short bar where it is reading, brighter
/// at the top of its window. A tap or drag on it moves the position: it
/// gives [`EventKind::Value`] with the place across the region, 0 to 1.
pub struct GrainCloud {
    pub title: String,
    pub right: String,
    peaks: Option<Arc<dyn Peaks>>,
    /// The region, in source frames.
    span: (u64, u64),
    /// Where grains read from, 0 to 1 across the region, and how far they
    /// may stray from it each way (the spray, as a share of the region).
    pub position: f32,
    pub spray: f32,
    /// Where the voice is reading now (before spray), 0 to 1, while it plays.
    pub playhead: Option<f32>,
    pub grains: Vec<GrainView>,
    /// A grain's length as a share of the region, for the bars' width.
    pub grain_len: f32,
    pub caption: String,
    /// Drawn faint, with `note` over it, while the layer doesn't play
    /// grains.
    pub granular: bool,
    pub note: String,
    grab: Option<u64>,
}

impl GrainCloud {
    pub fn new(title: &str) -> GrainCloud {
        GrainCloud {
            title: title.into(),
            right: String::new(),
            peaks: None,
            span: (0, 0),
            position: 0.0,
            spray: 0.0,
            playhead: None,
            grains: Vec::new(),
            grain_len: 0.0,
            caption: String::new(),
            granular: true,
            note: String::new(),
            grab: None,
        }
    }

    /// The sound to draw: `peaks` from frame `start` to `end`.
    pub fn set_sound(&mut self, peaks: Option<Arc<dyn Peaks>>, start: u64, end: u64) {
        self.peaks = peaks;
        self.span = (start, end.max(start + 1));
    }

    /// The area the sound fills.
    pub fn plot(r: Rect) -> Rect {
        body(r).take_bottom(26).1
    }

    fn place_at(r: Rect, x: i32) -> f32 {
        let p = GrainCloud::plot(r);
        ((x - p.x) as f32 / (p.w - 1).max(1) as f32).clamp(0.0, 1.0)
    }
}

/// A steady small number from `k`, for spreading the grains' bars up and
/// down the picture.
fn scatter(k: usize) -> f32 {
    let mut x = (k as u32).wrapping_mul(0x9E37_79B9) ^ 0x5bd1_e995;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x % 1000) as f32 / 1000.0
}

impl Widget for GrainCloud {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let plot = GrainCloud::plot(r);
        let violet = Color::rgb(GRAIN_COLOR);
        let wave = if self.granular { t.text_faint } else { t.text_faint.mix(t.surface, 0.5) };
        let mid = plot.y + plot.h / 2;
        let half = (plot.h / 2) as f32;
        // The band grains start in, round where the voice reads (the
        // position, until scanning moves it), wrapping round the ends.
        if self.granular {
            let band = t.surface.mix(violet, 0.18);
            let centre = self.playhead.unwrap_or(self.position);
            let lo = centre - self.spray * 0.5;
            let hi = centre + self.spray * 0.5;
            let x_of = |p: f32| plot.x + (p * plot.w as f32).round() as i32;
            let mut fill = |a: f32, b: f32| {
                let (x0, x1) = (x_of(a.clamp(0.0, 1.0)), x_of(b.clamp(0.0, 1.0)));
                if x1 > x0 {
                    cx.canvas.fill_rect(Rect::new(x0, plot.y, x1 - x0, plot.h), band);
                }
            };
            if self.spray >= 1.0 {
                fill(0.0, 1.0);
            } else {
                fill(lo, hi);
                if lo < 0.0 {
                    fill(1.0 + lo, 1.0);
                }
                if hi > 1.0 {
                    fill(0.0, hi - 1.0);
                }
            }
        }
        // The sound.
        if let Some(p) = &self.peaks {
            let (s0, s1) = (self.span.0 as f64, self.span.1 as f64);
            let fpp = (s1 - s0) / plot.w.max(1) as f64;
            for x in 0..plot.w {
                let f0 = s0 + f64::from(x) * fpp;
                let mut c = p.column(0, f0, f0 + fpp);
                if p.channels() > 1 {
                    let c2 = p.column(1, f0, f0 + fpp);
                    (c.min, c.max) = (c.min.min(c2.min), c.max.max(c2.max));
                }
                if c.max < c.min {
                    continue;
                }
                let top = mid - (c.max.clamp(-1.0, 1.0) * half) as i32;
                let bot = mid - (c.min.clamp(-1.0, 1.0) * half) as i32;
                cx.canvas.fill_rect(Rect::new(plot.x + x, top, 1, (bot - top).max(1)), wave);
            }
        }
        cx.canvas.fill_rect(Rect::new(plot.x, mid, plot.w, 1), t.outline);
        if self.granular {
            // The position, and where the voice reads now.
            let x = plot.x + (self.position.clamp(0.0, 1.0) * (plot.w - 1) as f32).round() as i32;
            cx.canvas.fill_rect(Rect::new(x - 1, plot.y, 2, plot.h), violet);
            if let Some(h) = self.playhead {
                let x = plot.x + (h.clamp(0.0, 1.0) * (plot.w - 1) as f32).round() as i32;
                cx.canvas.fill_rect(Rect::new(x, plot.y, 1, plot.h), t.text_dim);
            }
            // The grains: a bar each, as long as a grain, brighter while
            // its window is open.
            let w = ((self.grain_len * plot.w as f32).round() as i32).clamp(6, 60);
            let rows = (plot.h - 16).max(1) as f32;
            for (k, g) in self.grains.iter().enumerate() {
                let x = plot.x + (g.position.clamp(0.0, 1.0) * (plot.w - 1) as f32).round() as i32;
                let y = plot.y + 8 + (scatter(k) * rows) as i32;
                let c = if g.reverse { Color::rgb(REVERSE_COLOR) } else { violet };
                let c = t.surface.mix(c, 0.35 + 0.65 * g.level.clamp(0.0, 1.0));
                let bar = Rect::new(x - w / 2, y - 2, w, 5).intersect(plot);
                cx.canvas.fill_round_rect(bar, 2, c);
            }
        } else if !self.note.is_empty() {
            let note = Rect::new(plot.x, mid - 30, plot.w, 26);
            cx.text(FontId::Bold, t.size_body, note, Align::Center, t.text, &self.note);
        }
        let caption = Rect::new(plot.x, plot.bottom() + 6, plot.w, 20);
        cx.text(FontId::Sans, t.size_small - 1.0, caption, Align::Center, t.text_dim, &self.caption);
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                if !GrainCloud::plot(r).contains(x, y) {
                    return None;
                }
                self.grab = Some(finger);
                self.position = GrainCloud::place_at(r, x);
                Some(EventKind::Value(self.position))
            }
            Touch::Move if self.grab == Some(finger) => {
                self.position = GrainCloud::place_at(r, x);
                Some(EventKind::Value(self.position))
            }
            Touch::Up | Touch::Cancel => {
                self.grab.take_if(|g| *g == finger);
                None
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- touch pad

/// A square to drag a dot in: `x` and `y` from 0 to 1 (bottom left is 0,
/// 0). A touch or drag moves the dot there and gives
/// [`EventKind::Marker`] with `index` 0; the page reads `x` and `y`.
pub struct XyPad {
    pub title: String,
    pub right: String,
    pub x_label: String,
    pub y_label: String,
    pub x: f32,
    pub y: f32,
    pub enabled: bool,
    grab: Option<u64>,
}

/// The dot's size, and the halo round it.
const DOT: i32 = 22;
const HALO: i32 = 68;

impl XyPad {
    pub fn new(title: &str, x_label: &str, y_label: &str) -> XyPad {
        XyPad {
            title: title.into(),
            right: String::new(),
            x_label: x_label.into(),
            y_label: y_label.into(),
            x: 0.5,
            y: 0.5,
            enabled: true,
            grab: None,
        }
    }

    /// The square the dot moves in.
    pub fn plot(r: Rect) -> Rect {
        let b = body(r);
        let side = b.w.min(b.h);
        Rect::new(b.x + (b.w - side) / 2, b.y, side, side)
    }

    /// Where `(x, y)` (0 to 1) is on screen.
    pub fn point(r: Rect, x: f32, y: f32) -> (i32, i32) {
        let p = XyPad::plot(r);
        let (m, w, h) = (DOT / 2, (p.w - DOT) as f32, (p.h - DOT) as f32);
        (p.x + m + (x.clamp(0.0, 1.0) * w).round() as i32, p.bottom() - m - (y.clamp(0.0, 1.0) * h).round() as i32)
    }

    fn set_at(&mut self, r: Rect, px: i32, py: i32) {
        let p = XyPad::plot(r);
        let (m, w, h) = (DOT / 2, (p.w - DOT).max(1) as f32, (p.h - DOT).max(1) as f32);
        self.x = ((px - p.x - m) as f32 / w).clamp(0.0, 1.0);
        self.y = ((p.bottom() - m - py) as f32 / h).clamp(0.0, 1.0);
    }
}

impl Widget for XyPad {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let p = XyPad::plot(r);
        cx.canvas.fill_round_rect(p, 10, t.bg);
        for k in 1..4 {
            let x = p.x + p.w * k / 4;
            let y = p.y + p.h * k / 4;
            cx.canvas.fill_rect(Rect::new(x, p.y + 4, 1, p.h - 8), t.outline);
            cx.canvas.fill_rect(Rect::new(p.x + 4, y, p.w - 8, 1), t.outline);
        }
        let small = t.size_small - 1.0;
        let xl = Rect::new(p.x + 8, p.bottom() - 24, p.w - 16, 18);
        cx.text(FontId::Sans, small, xl, Align::Right, t.text_dim, &format!("{} →", self.x_label));
        let yl = Rect::new(p.x + 8, p.y + 6, p.w - 16, 18);
        cx.text(FontId::Sans, small, yl, Align::Left, t.text_dim, &format!("↑ {}", self.y_label));
        let violet = Color::rgb(GRAIN_COLOR);
        let (c, halo) = if self.enabled { (violet, t.bg.mix(violet, 0.3)) } else { (t.text_faint, t.bg.mix(t.text_faint, 0.2)) };
        let (x, y) = XyPad::point(r, self.x, self.y);
        cx.canvas.fill_round_rect(Rect::new(x - HALO / 2, y - HALO / 2, HALO, HALO), HALO / 2, halo);
        cx.canvas.fill_round_rect(Rect::new(x - DOT / 2, y - DOT / 2, DOT, DOT), DOT / 2, c);
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                if !self.enabled || !XyPad::plot(r).contains(x, y) {
                    return None;
                }
                self.grab = Some(finger);
                self.set_at(r, x, y);
                Some(EventKind::Marker { index: 0, frame: 0 })
            }
            Touch::Move if self.grab == Some(finger) => {
                self.set_at(r, x, y);
                Some(EventKind::Marker { index: 0, frame: 0 })
            }
            Touch::Up | Touch::Cancel => {
                self.grab.take_if(|g| *g == finger);
                None
            }
            _ => None,
        }
    }
}
