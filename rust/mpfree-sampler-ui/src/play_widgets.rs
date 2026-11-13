//! The Play tab's three pictures: which layer plays at which velocity, the
//! amp envelope drawn over the sound, and the filter's response. Like the
//! other widgets they show what the page put in their fields and report
//! touches; a drag moves the picture at once, and the page applies the
//! value to the project and shows what was kept.

use std::sync::Arc;

use mpfree_sampler::model::{Ahdsr, Filter, FilterKind};
use mpfree_ui::wave::Peaks;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// Below a titled panel's title, as `PaintCx::titled_panel` lays it out.
fn body(r: Rect) -> Rect {
    r.inset(14).take_top(34).1
}

/// A dot for a point the finger can drag.
fn dot(cx: &mut PaintCx, x: i32, y: i32, size: i32, c: Color) {
    cx.canvas.fill_round_rect(
        Rect::new(x - size / 2, y - size / 2, size, size),
        size / 2,
        c,
    );
}

/// A line through one y per column from `x0`, each joined to the next so
/// steep parts stay unbroken.
fn polyline(cx: &mut PaintCx, x0: i32, ys: &[i32], width: i32, c: Color) {
    for (i, w) in ys.windows(2).enumerate() {
        let (top, bot) = (w[0].min(w[1]), w[0].max(w[1]));
        cx.canvas.fill_rect(
            Rect::new(x0 + i as i32, top - width / 2, width, bot - top + width),
            c,
        );
    }
}

// ---------------------------------------------------------------- layers

/// One layer on the [`LayerMap`].
#[derive(Clone, Debug, PartialEq)]
pub struct LayerBar {
    pub name: String,
    pub sub: String,
    /// The velocities it plays, 1 to 127.
    pub lo: u8,
    pub hi: u8,
    pub color: Color,
}

/// The pad's layers on a velocity axis, one row each, with a line at the
/// last hit's velocity. A tap on a layer gives [`EventKind::Select`]; a drag
/// on its left or right edge gives [`EventKind::Marker`] with `index` 2k
/// (layer k's lowest velocity) or 2k + 1 (its highest) and the velocity as
/// `frame`.
pub struct LayerMap {
    pub title: String,
    pub right: String,
    pub layers: Vec<LayerBar>,
    pub selected: usize,
    pub last_velocity: Option<u8>,
    /// The pad's velocity fade: each bar is cut down along its curve over
    /// this many velocities inside each edge (none at 1 and 127).
    pub fade: u8,
    /// Finger, layer, and whether it holds the high edge.
    grab: Option<(u64, usize, bool)>,
}

const AXIS_H: i32 = 52;
const ROW_GAP: i32 = 14;
const MAX_ROW_H: i32 = 52;
const EDGE: i32 = 16;

impl LayerMap {
    pub fn new(title: &str) -> LayerMap {
        LayerMap {
            title: title.to_string(),
            right: String::new(),
            layers: Vec::new(),
            selected: 0,
            last_velocity: None,
            fade: 0,
            grab: None,
        }
    }

    fn plot(r: Rect) -> Rect {
        let b = body(r);
        Rect::new(
            b.x + 8,
            b.y + 22,
            (b.w - 16).max(1),
            (b.h - 22 - AXIS_H).max(1),
        )
    }

    /// Where velocity `v` starts on the axis.
    fn x_of(plot: Rect, v: f32) -> i32 {
        plot.x + (v / 128.0 * plot.w as f32).round() as i32
    }

    fn velocity_at(plot: Rect, x: i32) -> i32 {
        ((x - plot.x) as f32 / plot.w as f32 * 128.0).round() as i32
    }

    /// Layer `k`'s bar.
    pub fn bar_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let l = self.layers.get(k)?;
        let plot = LayerMap::plot(r);
        let n = self.layers.len() as i32;
        let row = ((plot.h - ROW_GAP * (n - 1)) / n).clamp(8, MAX_ROW_H);
        let x0 = LayerMap::x_of(plot, f32::from(l.lo));
        let x1 = LayerMap::x_of(plot, f32::from(l.hi) + 1.0).max(x0 + 24);
        Some(Rect::new(
            x0,
            plot.y + k as i32 * (row + ROW_GAP),
            x1 - x0,
            row,
        ))
    }
}

/// Cuts bar `b` down to an equal-power fade between `x0` and `x1`: rising
/// (the level grows left to right) or falling. What is cut shows `bg`,
/// faintly, so the bar's extent stays visible.
pub fn fade_cut(cx: &mut PaintCx, b: Rect, x0: i32, x1: i32, rising: bool, bg: Color) {
    let (x0, x1) = (x0.max(b.x), x1.min(b.right()));
    if x1 <= x0 {
        return;
    }
    let span = (x1 - x0) as f32;
    for x in x0..x1 {
        let mut t = (x - x0) as f32 / span;
        if !rising {
            t = 1.0 - t;
        }
        let g = (t * std::f32::consts::FRAC_PI_2).sin();
        let h = ((1.0 - g) * b.h as f32).round() as i32;
        if h > 0 {
            cx.canvas
                .fill_rect(Rect::new(x, b.y, 1, h), bg.with_alpha(210));
        }
    }
}

impl Widget for LayerMap {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let plot = LayerMap::plot(r);
        for v in [0, 32, 64, 96, 127] {
            let x = LayerMap::x_of(plot, v as f32);
            cx.canvas
                .fill_rect(Rect::new(x, plot.y, 1, plot.h), t.outline);
            let label = Rect::new(x - 30, plot.bottom() + 6, 60, 20);
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                label,
                Align::Center,
                t.text_faint,
                &v.to_string(),
            );
        }
        let caption = Rect::new(plot.x, plot.bottom() + 28, plot.w, 20);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            caption,
            Align::Right,
            t.text_dim,
            "velocity →",
        );
        for k in 0..self.layers.len() {
            let (Some(b), l) = (self.bar_rect(r, k), &self.layers[k]) else {
                continue;
            };
            let on = k == self.selected;
            cx.canvas
                .fill_round_rect(b, 8, l.color.mix(t.surface, if on { 0.45 } else { 0.72 }));
            cx.canvas.fill_rect(Rect::new(b.x, b.y, 4, b.h), l.color);
            if self.fade > 0 {
                let w = |v: f32| LayerMap::x_of(plot, v);
                let f = f32::from(self.fade);
                if l.lo > 1 {
                    let x0 = w(f32::from(l.lo));
                    fade_cut(cx, b, x0, w(f32::from(l.lo) + f + 1.0), true, t.surface);
                }
                if l.hi < 127 {
                    let x1 = w(f32::from(l.hi) + 1.0);
                    fade_cut(cx, b, w(f32::from(l.hi) - f), x1, false, t.surface);
                }
            }
            if on {
                cx.canvas.stroke_round_rect(b, 8, 2, l.color);
            }
            // The names go inside the bar if they fit, else beside it on
            // the side with more room.
            let need = cx
                .measure(FontId::Bold, t.size_small, &l.name)
                .max(cx.measure(FontId::Sans, t.size_small - 2.0, &l.sub))
                + 16;
            let (text, align) = if b.w >= need {
                (Rect::new(b.x + 12, b.y + 4, b.w - 16, b.h - 8), Align::Left)
            } else if plot.right() - b.right() >= b.x - plot.x {
                (
                    Rect::new(
                        b.right() + 8,
                        b.y + 4,
                        (plot.right() - b.right() - 8).max(0),
                        b.h - 8,
                    ),
                    Align::Left,
                )
            } else {
                (
                    Rect::new(plot.x, b.y + 4, (b.x - plot.x - 8).max(0), b.h - 8),
                    Align::Right,
                )
            };
            let (top, bottom) = text.take_top(text.h / 2 + 2);
            cx.text(FontId::Bold, t.size_small, top, align, t.text, &l.name);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                bottom,
                align,
                t.text_dim,
                &l.sub,
            );
        }
        if let Some(v) = self.last_velocity {
            let x = LayerMap::x_of(plot, f32::from(v) + 0.5);
            cx.canvas
                .fill_rect(Rect::new(x - 1, plot.y - 4, 2, plot.h + 4), t.text);
            let label = Rect::new(x - 60, plot.y - 24, 120, 20);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                label,
                Align::Center,
                t.text,
                &format!("last hit {v}"),
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = Vec::new();
        for k in 0..self.layers.len() {
            let Some(b) = self.bar_rect(r, k) else {
                continue;
            };
            let name = format!("layer {}", k + 1);
            // The middle picks the layer; each end drags its velocity edge.
            let mid = b.inset_xy(EDGE + 1, 0);
            if !mid.is_empty() {
                out.push(Target::tap("layer", k, name.clone(), mid));
            }
            let edge = |x: i32| Rect::new(x - EDGE / 2, b.y, EDGE, b.h);
            out.push(Target::drag("layer-low", k, format!("{name} low"), edge(b.x)).towards(20, 0));
            out.push(
                Target::drag("layer-high", k, format!("{name} high"), edge(b.right()))
                    .towards(-20, 0),
            );
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let k = (0..self.layers.len()).find(|&k| {
                    self.bar_rect(r, k)
                        .is_some_and(|b| b.inset_xy(-EDGE / 2, 0).contains(x, y))
                })?;
                let b = self.bar_rect(r, k)?;
                self.selected = k;
                if (x - b.x).abs() <= EDGE {
                    self.grab = Some((finger, k, false));
                } else if (x - b.right()).abs() <= EDGE {
                    self.grab = Some((finger, k, true));
                }
                Some(EventKind::Select(k))
            }
            Touch::Move => {
                let (g, k, high) = self.grab?;
                if g != finger {
                    return None;
                }
                let v = LayerMap::velocity_at(LayerMap::plot(r), x);
                let l = self.layers.get_mut(k)?;
                let frame = if high {
                    l.hi = (v - 1).clamp(i32::from(l.lo), 127) as u8;
                    l.hi
                } else {
                    l.lo = v.clamp(1, i32::from(l.hi).max(1)) as u8;
                    l.lo
                };
                Some(EventKind::Marker {
                    index: 2 * k + usize::from(high),
                    frame: u64::from(frame),
                })
            }
            Touch::Up | Touch::Cancel => {
                self.grab.take_if(|(g, _, _)| *g == finger);
                None
            }
        }
    }
}

// ---------------------------------------------------------------- envelope

/// The envelope's points, in the order their markers report them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvPoint {
    /// Attack time (drag sideways).
    Attack,
    /// Hold time.
    Hold,
    /// Decay time.
    Decay,
    /// Sustain level (drag up and down), shown where the sound ends.
    Sustain,
    /// Release time, after the sound ends or the pad lifts.
    Release,
}

impl EnvPoint {
    pub const ALL: [EnvPoint; 5] = [
        EnvPoint::Attack,
        EnvPoint::Hold,
        EnvPoint::Decay,
        EnvPoint::Sustain,
        EnvPoint::Release,
    ];

    fn letter(self) -> &'static str {
        match self {
            EnvPoint::Attack => "A",
            EnvPoint::Hold => "H",
            EnvPoint::Decay => "D",
            EnvPoint::Sustain => "S",
            EnvPoint::Release => "R",
        }
    }
}

/// How far a decay or release falls in its time: to about -80 dB.
const ENV_FLOOR_LN: f32 = 9.21;

/// The level of `env` `ms` into a note that is let go at `off_ms`.
pub fn env_level(env: &Ahdsr, ms: f32, off_ms: f32) -> f32 {
    let (a, h, d) = (
        env.attack_ms.max(0.0),
        env.hold_ms.max(0.0),
        env.decay_ms.max(0.0),
    );
    let s = env.sustain.clamp(0.0, 1.0);
    let held = |t: f32| {
        if t < a {
            t / a
        } else if t < a + h {
            1.0
        } else if d > 0.0 {
            s + (1.0 - s) * (-ENV_FLOOR_LN * (t - a - h) / d).exp()
        } else {
            s
        }
    };
    if ms < off_ms {
        return held(ms);
    }
    let at_off = held(off_ms);
    let rel = env.release_ms.max(0.0);
    if rel <= 0.0 {
        0.0
    } else {
        at_off * (-ENV_FLOOR_LN * (ms - off_ms) / rel).exp()
    }
}

/// The amp envelope drawn over the sound it shapes: the sound's waveform
/// from the region's start, the envelope's curve, and its points A, H, D,
/// S and R. Dragging a point gives [`EventKind::Marker`] with `index` the
/// point ([`EnvPoint::ALL`] order) and `frame` its new time in
/// microseconds, or for S its level in thousandths.
pub struct EnvelopeView {
    pub title: String,
    pub right: String,
    /// The line under the picture.
    pub caption: String,
    pub env: Ahdsr,
    peaks: Option<Arc<dyn Peaks>>,
    /// The region's first and last frame, and the source's rate.
    span: (u64, u64),
    rate: u32,
    /// The time axis, kept still while a point is dragged.
    held_axis: Option<f32>,
    grab: Option<(u64, EnvPoint)>,
}

const POINT: i32 = 18;

impl EnvelopeView {
    pub fn new(title: &str) -> EnvelopeView {
        EnvelopeView {
            title: title.to_string(),
            right: String::new(),
            caption: String::new(),
            env: Ahdsr::OPEN,
            peaks: None,
            span: (0, 0),
            rate: 44_100,
            held_axis: None,
            grab: None,
        }
    }

    /// The sound under the envelope: frames `start..end` of `peaks` at `rate`.
    pub fn set_sound(&mut self, peaks: Option<Arc<dyn Peaks>>, start: u64, end: u64, rate: u32) {
        self.peaks = peaks;
        self.span = (start, end.max(start));
        self.rate = rate.max(1);
    }

    /// How long the sound plays, in ms: where S sits.
    pub fn length_ms(&self) -> f32 {
        (self.span.1 - self.span.0) as f32 * 1000.0 / self.rate as f32
    }

    /// Where S sits: the sound's end, or after the decay if that is later.
    fn sustain_ms(&self) -> f32 {
        let e = &self.env;
        self.length_ms().max(e.attack_ms + e.hold_ms + e.decay_ms)
    }

    /// The time across the picture, in ms.
    pub fn axis_ms(&self) -> f32 {
        self.held_axis
            .unwrap_or_else(|| ((self.sustain_ms() + self.env.release_ms.max(1.0)) * 1.04).max(1.0))
    }

    fn plot(r: Rect) -> Rect {
        let b = body(r);
        Rect::new(
            b.x + POINT / 2,
            b.y + POINT / 2,
            (b.w - POINT).max(1),
            (b.h - POINT - 24).max(1),
        )
    }

    /// Point `p`'s time (ms) and level.
    pub fn point(&self, p: EnvPoint) -> (f32, f32) {
        let e = &self.env;
        let s = e.sustain.clamp(0.0, 1.0);
        match p {
            EnvPoint::Attack => (e.attack_ms, 1.0),
            EnvPoint::Hold => (e.attack_ms + e.hold_ms, 1.0),
            EnvPoint::Decay => (e.attack_ms + e.hold_ms + e.decay_ms, s),
            EnvPoint::Sustain => (self.sustain_ms(), s),
            EnvPoint::Release => (self.sustain_ms() + e.release_ms, 0.0),
        }
    }

    /// Point `p` on the screen.
    pub fn point_xy(&self, r: Rect, p: EnvPoint) -> (i32, i32) {
        let plot = EnvelopeView::plot(r);
        let (ms, level) = self.point(p);
        let x = plot.x + (ms / self.axis_ms() * plot.w as f32).round() as i32;
        let y = plot.bottom() - (level * plot.h as f32).round() as i32;
        (x.min(plot.right()), y)
    }
}

impl Widget for EnvelopeView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let plot = EnvelopeView::plot(r);
        let axis = self.axis_ms();
        let ms_at = |x: i32| (x - plot.x) as f32 / plot.w as f32 * axis;
        // The sound, faint, from the region's start.
        if let Some(p) = &self.peaks {
            let mid = plot.y + plot.h / 2;
            let half = (plot.h / 2) as f32;
            let fpms = self.rate as f64 / 1000.0;
            let len = self.length_ms();
            for x in plot.x..plot.right() {
                let (m0, m1) = (ms_at(x), ms_at(x + 1));
                if m0 >= len {
                    break;
                }
                let f0 = self.span.0 as f64 + f64::from(m0) * fpms;
                let f1 = self.span.0 as f64 + f64::from(m1.min(len)) * fpms;
                let mut c = p.column(0, f0, f1);
                if p.channels() > 1 {
                    let c2 = p.column(1, f0, f1);
                    (c.min, c.max) = (c.min.min(c2.min), c.max.max(c2.max));
                }
                if c.max < c.min {
                    continue;
                }
                let top = mid - (c.max.clamp(-1.0, 1.0) * half) as i32;
                let bot = mid - (c.min.clamp(-1.0, 1.0) * half) as i32;
                cx.canvas
                    .fill_rect(Rect::new(x, top, 1, (bot - top).max(1)), t.text_faint);
            }
            cx.canvas
                .fill_rect(Rect::new(plot.x, mid, plot.w, 1), t.outline);
        }
        // Where the sound ends (or the pad lifts): release from here.
        let (sx, _) = self.point_xy(r, EnvPoint::Sustain);
        cx.canvas
            .fill_rect(Rect::new(sx, plot.y, 1, plot.h), t.outline);
        let off = self.sustain_ms();
        let ys: Vec<i32> = (plot.x..=plot.right())
            .map(|x| {
                plot.bottom() - (env_level(&self.env, ms_at(x), off) * plot.h as f32).round() as i32
            })
            .collect();
        polyline(cx, plot.x, &ys, 3, t.accent);
        for p in EnvPoint::ALL {
            let (x, y) = self.point_xy(r, p);
            dot(cx, x, y, POINT, t.accent);
            cx.text(
                FontId::Bold,
                11.0,
                Rect::new(x - POINT / 2, y - POINT / 2, POINT, POINT),
                Align::Center,
                t.bg,
                p.letter(),
            );
        }
        let caption = Rect::new(plot.x, plot.bottom() + POINT / 2 + 2, plot.w, 20);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            caption,
            Align::Left,
            t.text_dim,
            &self.caption,
        );
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        EnvPoint::ALL
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let (px, py) = self.point_xy(r, p);
                let pr = Rect::new(px - POINT / 2, py - POINT / 2, POINT, POINT);
                let towards = if p == EnvPoint::Sustain {
                    (0, -30)
                } else {
                    (30, 0)
                };
                Target::drag("point", i, format!("{p:?}"), pr).towards(towards.0, towards.1)
            })
            .collect()
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                // The nearest point, the later one winning a tie (points
                // that sit together are pulled apart from the right).
                let (p, d) = EnvPoint::ALL
                    .iter()
                    .rev()
                    .map(|&p| {
                        let (px, py) = self.point_xy(r, p);
                        (p, (px - x).pow(2) + (py - y).pow(2))
                    })
                    .min_by_key(|(_, d)| *d)?;
                if d > (POINT * 2).pow(2) {
                    return None;
                }
                self.held_axis = Some(self.axis_ms());
                self.grab = Some((finger, p));
                None
            }
            Touch::Move => {
                let (g, p) = self.grab?;
                if g != finger {
                    return None;
                }
                let plot = EnvelopeView::plot(r);
                let ms = ((x - plot.x) as f32 / plot.w as f32 * self.axis_ms()).max(0.0);
                let length = self.length_ms();
                let e = &mut self.env;
                let value = match p {
                    EnvPoint::Attack => {
                        e.attack_ms = ms.round();
                        e.attack_ms
                    }
                    EnvPoint::Hold => {
                        e.hold_ms = (ms - e.attack_ms).max(0.0).round();
                        e.hold_ms
                    }
                    EnvPoint::Decay => {
                        e.decay_ms = (ms - e.attack_ms - e.hold_ms).max(0.0).round();
                        e.decay_ms
                    }
                    EnvPoint::Sustain => {
                        e.sustain = ((plot.bottom() - y) as f32 / plot.h as f32).clamp(0.0, 1.0);
                        return Some(EventKind::Marker {
                            index: 3,
                            frame: (e.sustain * 1000.0).round() as u64,
                        });
                    }
                    EnvPoint::Release => {
                        let off = length.max(e.attack_ms + e.hold_ms + e.decay_ms);
                        e.release_ms = (ms - off).max(1.0).round();
                        e.release_ms
                    }
                };
                let index = EnvPoint::ALL.iter().position(|q| *q == p).unwrap_or(0);
                Some(EventKind::Marker {
                    index,
                    frame: (value * 1000.0).round() as u64,
                })
            }
            Touch::Up | Touch::Cancel => {
                if self.grab.take_if(|(g, _)| *g == finger).is_some() {
                    self.held_axis = None;
                }
                None
            }
        }
    }
}

// ---------------------------------------------------------------- filter

const F_LO: f32 = 20.0;
const F_HI: f32 = 20_000.0;
const DB_TOP: f32 = 24.0;
const DB_BOTTOM: f32 = -36.0;

/// The filter's gain in dB at `hz`, as the pad's state-variable filter
/// shapes it (4 poles are two 2-pole stages).
pub fn filter_db(f: &Filter, hz: f32) -> f32 {
    if f.kind == FilterKind::Off {
        return 0.0;
    }
    let w = hz / f.cutoff_hz.max(1.0);
    let k = 2.0 - 1.95 * f.resonance.clamp(0.0, 1.0);
    let den = (1.0 - w * w).powi(2) + (k * w).powi(2);
    let power = match f.kind {
        FilterKind::LowPass => 1.0 / den,
        FilterKind::BandPass => w * w / den,
        FilterKind::HighPass => w.powi(4) / den,
        FilterKind::Off => 1.0,
    };
    let stages = if f.poles >= 4 { 2.0 } else { 1.0 };
    10.0 * power.max(1e-12).log10() * stages
}

/// The filter's response on a log frequency axis, with a dot at the
/// cutoff. Dragging sideways gives [`EventKind::Marker`] with `index` 0
/// and the new cutoff in Hz as `frame`.
pub struct FilterCurve {
    pub title: String,
    pub right: String,
    pub caption: String,
    pub filter: Filter,
    grab: Option<u64>,
}

impl FilterCurve {
    pub fn new(title: &str) -> FilterCurve {
        FilterCurve {
            title: title.to_string(),
            right: String::new(),
            caption: String::new(),
            filter: Filter::default(),
            grab: None,
        }
    }

    fn plot(r: Rect) -> Rect {
        let b = body(r);
        Rect::new(b.x, b.y + 4, b.w.max(1), (b.h - 4 - 24).max(1))
    }

    fn x_of(plot: Rect, hz: f32) -> i32 {
        let u = (hz.clamp(F_LO, F_HI) / F_LO).log10() / (F_HI / F_LO).log10();
        plot.x + (u * plot.w as f32).round() as i32
    }

    fn hz_at(plot: Rect, x: i32) -> f32 {
        let u = ((x - plot.x) as f32 / plot.w as f32).clamp(0.0, 1.0);
        F_LO * (F_HI / F_LO).powf(u)
    }

    fn y_of(plot: Rect, db: f32) -> i32 {
        let u = (db.clamp(DB_BOTTOM, DB_TOP) - DB_BOTTOM) / (DB_TOP - DB_BOTTOM);
        plot.bottom() - (u * plot.h as f32).round() as i32
    }

    /// The cutoff's dot on the screen.
    pub fn dot_xy(&self, r: Rect) -> (i32, i32) {
        let plot = FilterCurve::plot(r);
        let hz = self.filter.cutoff_hz;
        (
            FilterCurve::x_of(plot, hz),
            FilterCurve::y_of(plot, filter_db(&self.filter, hz)),
        )
    }
}

impl Widget for FilterCurve {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let plot = FilterCurve::plot(r);
        for (hz, label) in [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")] {
            let x = FilterCurve::x_of(plot, hz);
            cx.canvas
                .fill_rect(Rect::new(x, plot.y, 1, plot.h), t.outline);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(x + 4, plot.bottom() - 18, 40, 16),
                Align::Left,
                t.text_faint,
                label,
            );
        }
        cx.canvas.fill_rect(
            Rect::new(plot.x, FilterCurve::y_of(plot, 0.0), plot.w, 1),
            t.outline,
        );
        let off = self.filter.kind == FilterKind::Off;
        let color = if off { t.text_faint } else { t.good };
        let ys: Vec<i32> = (plot.x..=plot.right())
            .map(|x| FilterCurve::y_of(plot, filter_db(&self.filter, FilterCurve::hz_at(plot, x))))
            .collect();
        polyline(cx, plot.x, &ys, 3, color);
        if !off {
            let (x, y) = self.dot_xy(r);
            dot(cx, x, y, 16, t.good);
        }
        let caption = Rect::new(plot.x, plot.bottom() + 6, plot.w, 20);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            caption,
            Align::Left,
            t.text_dim,
            &self.caption,
        );
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let plot = FilterCurve::plot(r);
        vec![Target::drag("cutoff", 0, "cutoff", plot)
            .towards(-40, 0)
            .disabled_if(self.filter.kind == FilterKind::Off)]
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let plot = FilterCurve::plot(r);
        match phase {
            Touch::Down
                if self.filter.kind != FilterKind::Off
                    && plot.inset_xy(-10, -10).contains(x, y) =>
            {
                self.grab = Some(finger);
                None
            }
            Touch::Move if self.grab == Some(finger) => {
                self.filter.cutoff_hz = FilterCurve::hz_at(plot, x).round();
                Some(EventKind::Marker {
                    index: 0,
                    frame: self.filter.cutoff_hz as u64,
                })
            }
            Touch::Up | Touch::Cancel => {
                self.grab.take_if(|g| *g == finger);
                None
            }
            _ => None,
        }
    }
}
