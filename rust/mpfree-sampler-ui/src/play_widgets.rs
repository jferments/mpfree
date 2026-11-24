//! The Play tab's pictures: which layer plays at which velocity, the amp
//! envelope drawn over the sound, the filter's response, and the LFO's
//! wave beside the pitch envelope. Like the
//! other widgets they show what the page put in their fields and report
//! touches; a drag moves the picture at once, and the page applies the
//! value to the project and shows what was kept.

use std::sync::Arc;

use mpfree_sampler::dsp::{comb_db, ladder_db, vowel_db};
use mpfree_sampler::model::{Ahdsr, Filter, FilterKind, LfoShape, PadLfo, PitchEnv, PITCH_ENV_ST};
use mpfree_ui::wave::Peaks;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// Below a titled panel's title, as `PaintCx::titled_panel` lays it out.
fn body(r: Rect) -> Rect {
    r.inset(14).take_top(34).1
}

/// `PaintCx::titled_panel`, leaving out the hint at the right when the
/// panel is too narrow for it beside the title (the keyboard's layout puts
/// three panels side by side).
fn titled(cx: &mut PaintCx, r: Rect, title: &str, right: &str) {
    let px = cx.theme.size_small;
    let room = r.w - 2 * cx.theme.pad;
    let need = cx.measure(FontId::Bold, px, &title.to_uppercase())
        + cx.measure(FontId::Sans, px, right)
        + 12;
    cx.titled_panel(r, title, if need <= room { right } else { "" });
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
/// Under this a bar shows its name only, not its velocities.
const TWO_LINES_H: i32 = 36;
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
        titled(cx, r, &self.title, &self.right);
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
            cx.canvas.fill_round_rect(
                b,
                t.round(8),
                l.color.mix(t.surface, if on { 0.45 } else { 0.72 }),
            );
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
                cx.canvas.stroke_round_rect(b, t.round(8), 2, l.color);
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
            // A bar too short for two lines keeps its name only.
            if b.h < TWO_LINES_H {
                cx.text(FontId::Bold, t.size_small, text, align, t.text, &l.name);
                continue;
            }
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
                // The layer picked already: a tap picks nothing new.
                out.push(
                    Target::tap("layer", k, name.clone(), mid).disabled_if(k == self.selected),
                );
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
    /// Labels down the right edge at envelope levels (0 to 1): the
    /// Filter section's cutoff in Hz at the envelope's peak, sustain and
    /// rest (the filter gap plan's F5). Empty: none, the picture full
    /// width.
    pub marks: Vec<(f32, String)>,
    peaks: Option<Arc<dyn Peaks>>,
    /// The region's first and last frame, and the source's rate.
    span: (u64, u64),
    rate: u32,
    /// The time axis, kept still while a point is dragged.
    held_axis: Option<f32>,
    grab: Option<(u64, EnvPoint)>,
}

const POINT: i32 = 18;
/// The room the envelope's marks take at the right ([`EnvelopeView::marks`]).
const MARKS_W: i32 = 70;

impl EnvelopeView {
    pub fn new(title: &str) -> EnvelopeView {
        EnvelopeView {
            title: title.to_string(),
            right: String::new(),
            caption: String::new(),
            env: Ahdsr::OPEN,
            marks: Vec::new(),
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

    fn plot(&self, r: Rect) -> Rect {
        let b = body(r);
        let marks = if self.marks.is_empty() { 0 } else { MARKS_W };
        Rect::new(
            b.x + POINT / 2,
            b.y + POINT / 2,
            (b.w - POINT - marks).max(1),
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
        let plot = self.plot(r);
        let (ms, level) = self.point(p);
        let x = plot.x + (ms / self.axis_ms() * plot.w as f32).round() as i32;
        let y = plot.bottom() - (level * plot.h as f32).round() as i32;
        (x.min(plot.right()), y)
    }
}

impl Widget for EnvelopeView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        titled(cx, r, &self.title, &self.right);
        let plot = self.plot(r);
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
                t.on_accent,
                p.letter(),
            );
        }
        for (level, text) in &self.marks {
            let y = plot.bottom() - (level.clamp(0.0, 1.0) * plot.h as f32).round() as i32;
            let x = plot.right() + POINT / 2 + 4;
            cx.canvas.fill_rect(Rect::new(x, y, 10, 2), t.warn);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(x + 12, y - 9, MARKS_W - 16, 18),
                Align::Left,
                t.readable(t.warn, t.surface),
                text,
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
                let plot = self.plot(r);
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
/// shapes it at its slope (the Peak too), or its ladder or its two
/// formants (sampler R5.6).
pub fn filter_db(f: &Filter, hz: f32) -> f32 {
    match f.kind {
        FilterKind::Off => return 0.0,
        FilterKind::Ladder => return ladder_db(f.cutoff_hz, f.resonance, hz),
        FilterKind::Vowel => return vowel_db(f.cutoff_hz, f.resonance, hz),
        FilterKind::Comb => return comb_db(f.cutoff_hz, f.resonance, hz),
        _ => {}
    }
    mpfree_sampler::dsp::svf_db(f, hz)
}

/// A button in the [`FilterCurve`]'s title line: On (the bypass),
/// Compare, Copy, Paste and Reset (the filter gap plan's F5). `lit`: drawn
/// as on (On while the filter plays, Compare while comparing); `faint`:
/// nothing to do now (a tap says why).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadButton {
    pub text: String,
    pub lit: bool,
    pub faint: bool,
}

/// The filter's response on a log frequency axis, with a dot at the
/// cutoff. Dragging sideways gives [`EventKind::Marker`] with `index` 0
/// and the new cutoff in Hz as `frame`. While the envelope moves the
/// cutoff (`filter.env_octaves`), a dashed curve in the warning colour
/// shows where its peak takes the filter, the band between shaded. The
/// [`HeadButton`]s in the title line give [`EventKind::Tab`] with their
/// place (a faint one [`EventKind::Select`]). Bypassed, the curve is
/// drawn faint behind a flat line: the sound passes as it is.
pub struct FilterCurve {
    pub title: String,
    /// The title when the full one doesn't fit beside the buttons.
    pub short: String,
    pub right: String,
    pub caption: String,
    pub filter: Filter,
    pub bypassed: bool,
    /// While comparing, the changes Compare set aside, drawn dashed
    /// behind the filter as it was.
    pub ghost: Option<Filter>,
    pub buttons: Vec<HeadButton>,
    grab: Option<u64>,
}

/// The title line's buttons: their height, the gap between them.
const HEAD_BUTTON_H: i32 = 36;
const HEAD_BUTTON_GAP: i32 = 6;

impl FilterCurve {
    pub fn new(title: &str) -> FilterCurve {
        FilterCurve {
            title: title.to_string(),
            short: String::new(),
            right: String::new(),
            caption: String::new(),
            filter: Filter::default(),
            bypassed: false,
            ghost: None,
            buttons: Vec::new(),
            grab: None,
        }
    }

    fn plot(r: Rect) -> Rect {
        let b = body(r);
        Rect::new(b.x, b.y + 4, b.w.max(1), (b.h - 4 - 24).max(1))
    }

    /// The title line's buttons, right to left from the panel's right
    /// edge, sized by their text.
    pub fn button_rects(&self, r: Rect) -> Vec<Rect> {
        let mut x = r.right() - 12;
        let mut out: Vec<Rect> = self
            .buttons
            .iter()
            .rev()
            .map(|b| {
                let w = 26 + 8 * b.text.chars().count() as i32;
                x -= w;
                let rect = Rect::new(x, r.y + 8, w, HEAD_BUTTON_H);
                x -= HEAD_BUTTON_GAP;
                rect
            })
            .collect();
        out.reverse();
        out
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

    /// Where the envelope's peak takes the cutoff, when it moves it.
    pub fn env_target(&self) -> Option<f32> {
        let f = &self.filter;
        (f.kind != FilterKind::Off && f.env_octaves != 0.0)
            .then(|| (f.cutoff_hz * f.env_octaves.exp2()).clamp(F_LO, F_HI))
    }
}

impl Widget for FilterCurve {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let rects = self.button_rects(r);
        if self.buttons.is_empty() {
            titled(cx, r, &self.title, &self.right);
        } else {
            cx.panel(r);
            let px = t.size_small;
            let left = rects.first().map_or(r.right(), |b| b.x) - 8;
            let head = Rect::new(r.x + t.pad, r.y + t.pad, (left - r.x - t.pad).max(0), 26);
            let full = self.title.to_uppercase();
            let title = if cx.measure(FontId::Bold, px, &full) <= head.w || self.short.is_empty() {
                full
            } else {
                self.short.to_uppercase()
            };
            cx.text(FontId::Bold, px, head, Align::Left, t.text_dim, &title);
            for (b, rect) in self.buttons.iter().zip(&rects) {
                let (fill, ink) = match (b.lit, b.faint) {
                    (_, true) => (t.raised, t.text_faint),
                    (true, false) => (t.picked(), t.on_picked()),
                    (false, false) => (t.raised, t.text),
                };
                cx.canvas.fill_round_rect(*rect, t.round(8), fill);
                cx.text(FontId::Bold, 14.0, *rect, Align::Center, ink, &b.text);
            }
        }
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
        let zero = FilterCurve::y_of(plot, 0.0);
        cx.canvas
            .fill_rect(Rect::new(plot.x, zero, plot.w, 1), t.outline);
        let off = self.filter.kind == FilterKind::Off;
        // The envelope's reach: the band from the cutoff to where its
        // peak takes it, and the filter there, dashed.
        if let (Some(to), false) = (self.env_target(), self.bypassed) {
            let (x0, x1) = (
                FilterCurve::x_of(plot, self.filter.cutoff_hz),
                FilterCurve::x_of(plot, to),
            );
            let (a, b) = (x0.min(x1), x0.max(x1));
            cx.canvas.fill_rect(
                Rect::new(a, plot.y, (b - a).max(1), plot.h),
                t.warn.with_alpha(26),
            );
            let peak = Filter {
                cutoff_hz: to,
                ..self.filter
            };
            for x in plot.x..plot.right() {
                // Dashes 6 on, 5 off.
                if (x - plot.x) % 11 >= 6 {
                    continue;
                }
                let y0 = FilterCurve::y_of(plot, filter_db(&peak, FilterCurve::hz_at(plot, x)));
                let y1 = FilterCurve::y_of(plot, filter_db(&peak, FilterCurve::hz_at(plot, x + 1)));
                let (top, bot) = (y0.min(y1), y0.max(y1));
                cx.canvas
                    .fill_rect(Rect::new(x, top - 1, 1, bot - top + 2), t.warn);
            }
            let label = format!("env {:+.1} oct → {}", self.filter.env_octaves, hz_label(to));
            let lw = cx.measure(FontId::Sans, t.size_small - 1.0, &label) + 4;
            let lx = if x1 >= x0 {
                (a + 6).min(plot.right() - lw)
            } else {
                (b - lw - 6).max(plot.x)
            };
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(lx, plot.y + 2, lw, 18),
                Align::Left,
                t.readable(t.warn, t.surface),
                &label,
            );
        }
        if let Some(g) = &self.ghost {
            let ys: Vec<i32> = (plot.x..=plot.right())
                .map(|x| FilterCurve::y_of(plot, filter_db(g, FilterCurve::hz_at(plot, x))))
                .collect();
            for (i, w) in ys.windows(2).enumerate() {
                // Dashes 6 on, 5 off.
                if i % 11 >= 6 {
                    continue;
                }
                let (top, bot) = (w[0].min(w[1]), w[0].max(w[1]));
                cx.canvas.fill_rect(
                    Rect::new(plot.x + i as i32, top - 1, 1, bot - top + 2),
                    t.text_dim,
                );
            }
        }
        let color = if off || self.bypassed {
            t.text_faint
        } else {
            t.good
        };
        let ys: Vec<i32> = (plot.x..=plot.right())
            .map(|x| FilterCurve::y_of(plot, filter_db(&self.filter, FilterCurve::hz_at(plot, x))))
            .collect();
        polyline(cx, plot.x, &ys, 3, color);
        if self.bypassed {
            // What is heard: the sound as it is.
            cx.canvas
                .fill_rect(Rect::new(plot.x, zero - 1, plot.w, 2), t.text);
        } else if !off {
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
        let mut out = vec![Target::drag("cutoff", 0, "cutoff", plot)
            .towards(-40, 0)
            .disabled_if(self.filter.kind == FilterKind::Off)];
        for (i, (b, rect)) in self.buttons.iter().zip(self.button_rects(r)).enumerate() {
            out.push(Target::tap("button", i, b.text.clone(), rect).disabled_if(b.faint));
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let plot = FilterCurve::plot(r);
        match phase {
            Touch::Down => {
                if let Some(i) = self.button_rects(r).iter().position(|b| b.contains(x, y)) {
                    let faint = self.buttons.get(i).is_some_and(|b| b.faint);
                    return Some(if faint {
                        EventKind::Select(i)
                    } else {
                        EventKind::Tab(i)
                    });
                }
                if self.filter.kind != FilterKind::Off && plot.inset_xy(-10, -10).contains(x, y) {
                    self.grab = Some(finger);
                }
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

/// A frequency as the pictures label it: "250 Hz", "4.00 kHz", "12.0 kHz".
pub fn hz_label(hz: f32) -> String {
    if hz < 1000.0 {
        format!("{hz:.0} Hz")
    } else if hz < 10_000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{:.1} kHz", hz / 1000.0)
    }
}

// ---------------------------------------------------------------- types

/// One filter type on the [`FilterTypes`] picker: its curve, its name and
/// a line on what it is for.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeTile {
    pub name: String,
    pub line: String,
    /// The curve its icon draws.
    pub filter: Filter,
}

/// One slope chip under the types: "24", lit when the pad has it, faint
/// when the type picked has no such slope.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlopeChip {
    pub text: String,
    pub lit: bool,
    pub faint: bool,
}

/// The filter types over the Filter section's pictures (the filter gap
/// plan's F5): a tile each, the pad's lit, and the slopes as chips under
/// them. A tap on a tile gives [`EventKind::Tab`] with its place; on a
/// slope chip `Tab(SLOPE_BASE + place)` (a faint one
/// [`EventKind::Select`] with the same number, for the page to say why);
/// Done `Tab(DONE)`.
pub struct FilterTypes {
    pub title: String,
    pub note: String,
    pub tiles: Vec<TypeTile>,
    pub picked: usize,
    pub slope_title: String,
    pub slope_note: String,
    pub slopes: Vec<SlopeChip>,
}

/// [`FilterTypes`]' and [`FilterMoves`]' Done: `Tab(DONE)`.
pub const DONE: usize = 99;
/// A slope chip's tap: `Tab(SLOPE_BASE + its place)`.
pub const SLOPE_BASE: usize = 10;

impl FilterTypes {
    pub fn new() -> FilterTypes {
        FilterTypes {
            title: "Pick a filter".into(),
            note: String::new(),
            tiles: Vec::new(),
            picked: 0,
            slope_title: "Slope (dB an octave)".into(),
            slope_note: String::new(),
            slopes: Vec::new(),
        }
    }

    fn done_rect(r: Rect) -> Rect {
        Rect::new(r.right() - 12 - 96, r.y + 8, 96, HEAD_BUTTON_H)
    }

    /// The tiles, side by side across the panel.
    pub fn tile_rects(&self, r: Rect) -> Vec<Rect> {
        let n = self.tiles.len().max(1) as i32;
        let gap = 8;
        let x0 = r.x + 14;
        let w = (r.w - 28 - gap * (n - 1)) / n;
        let top = r.y + 52;
        let h = (r.h - 52 - 92).max(60);
        (0..self.tiles.len() as i32)
            .map(|i| Rect::new(x0 + i * (w + gap), top, w, h))
            .collect()
    }

    /// The slope chips, side by side right of their title.
    pub fn slope_rects(&self, r: Rect) -> Vec<Rect> {
        let n = self.slopes.len().max(1) as i32;
        let gap = 8;
        let x0 = r.x + 14 + 190;
        let w = ((r.w - 28 - 190 - gap * (n - 1)) / n).min(110);
        let y = r.bottom() - 14 - 26 - 44;
        (0..self.slopes.len() as i32)
            .map(|i| Rect::new(x0 + i * (w + gap), y, w, 44))
            .collect()
    }
}

impl Default for FilterTypes {
    fn default() -> FilterTypes {
        FilterTypes::new()
    }
}

/// Splits a tile's line in two at its comma, or at the space nearest its
/// middle, so it fits a narrow tile.
fn two_lines(line: &str) -> (String, String) {
    if let Some((a, b)) = line.split_once(", ") {
        return (format!("{a},"), b.to_string());
    }
    if line.chars().count() <= 14 {
        return (line.to_string(), String::new());
    }
    let mid = line.len() / 2;
    let cut = line
        .match_indices(' ')
        .map(|(i, _)| i)
        .min_by_key(|i| i.abs_diff(mid));
    match cut {
        Some(i) => (line[..i].to_string(), line[i + 1..].to_string()),
        None => (line.to_string(), String::new()),
    }
}

impl Widget for FilterTypes {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        cx.canvas
            .stroke_round_rect(r, t.round(12), 2, t.accent.with_alpha(160));
        let done = FilterTypes::done_rect(r);
        let head = Rect::new(r.x + 14, r.y + 12, done.x - r.x - 24, 26);
        let title = self.title.to_uppercase();
        let tw = cx.measure(FontId::Bold, t.size_small, &title);
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            &title,
        );
        let note = Rect::new(head.x + tw + 14, head.y, (head.w - tw - 14).max(0), 26);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            note,
            Align::Left,
            t.text_faint,
            &self.note,
        );
        cx.canvas.fill_round_rect(done, t.round(8), t.raised);
        cx.text(FontId::Bold, 15.0, done, Align::Center, t.text, "Done");
        for (i, (tile, rect)) in self.tiles.iter().zip(self.tile_rects(r)).enumerate() {
            let lit = i == self.picked;
            cx.canvas.fill_round_rect(rect, t.round(10), t.raised);
            if lit {
                cx.canvas.stroke_round_rect(rect, t.round(10), 2, t.accent);
            }
            // The icon: the type's curve on a small log axis.
            let icon = Rect::new(rect.x + 10, rect.y + 10, rect.w - 20, (rect.h - 70).max(24));
            let ys: Vec<i32> = (icon.x..=icon.right())
                .map(|x| {
                    let u = (x - icon.x) as f32 / icon.w.max(1) as f32;
                    let hz = F_LO * (F_HI / F_LO).powf(u);
                    let db = filter_db(&tile.filter, hz).clamp(-30.0, 18.0);
                    icon.bottom() - ((db + 30.0) / 48.0 * icon.h as f32).round() as i32
                })
                .collect();
            let ink = if lit { t.good } else { t.text_dim };
            polyline(cx, icon.x, &ys, 2, ink);
            let name = Rect::new(rect.x + 4, icon.bottom() + 6, rect.w - 8, 20);
            cx.text(FontId::Bold, 15.0, name, Align::Center, t.text, &tile.name);
            let (a, b) = two_lines(&tile.line);
            for (k, l) in [a, b].iter().enumerate() {
                let lr = Rect::new(
                    rect.x + 4,
                    name.bottom() + 2 + 15 * k as i32,
                    rect.w - 8,
                    15,
                );
                cx.text(FontId::Sans, 12.0, lr, Align::Center, t.text_dim, l);
            }
        }
        let rects = self.slope_rects(r);
        if let Some(first) = rects.first() {
            let label = Rect::new(r.x + 14, first.y, 180, 44);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                label,
                Align::Left,
                t.text_dim,
                &self.slope_title,
            );
            let note = Rect::new(r.x + 14, first.bottom() + 6, r.w - 28, 20);
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                note,
                Align::Left,
                t.text_faint,
                &self.slope_note,
            );
        }
        for (s, rect) in self.slopes.iter().zip(rects) {
            let (fill, ink) = match (s.lit, s.faint) {
                (_, true) => (t.raised, t.text_faint),
                (true, false) => (t.picked(), t.on_picked()),
                (false, false) => (t.raised, t.text),
            };
            cx.canvas.fill_round_rect(rect, t.round(10), fill);
            cx.text(FontId::Bold, 15.0, rect, Align::Center, ink, &s.text);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = vec![Target::tap("done", 0, "Done", FilterTypes::done_rect(r))];
        for (i, (tile, rect)) in self.tiles.iter().zip(self.tile_rects(r)).enumerate() {
            out.push(Target::tap("type", i, tile.name.clone(), rect).disabled_if(i == self.picked));
        }
        for (i, (s, rect)) in self.slopes.iter().zip(self.slope_rects(r)).enumerate() {
            out.push(Target::tap("slope", i, s.text.clone(), rect).disabled_if(s.faint || s.lit));
        }
        out
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if FilterTypes::done_rect(r).contains(x, y) {
            return Some(EventKind::Tab(DONE));
        }
        if let Some(i) = self.tile_rects(r).iter().position(|t| t.contains(x, y)) {
            return Some(EventKind::Tab(i));
        }
        let i = self.slope_rects(r).iter().position(|t| t.contains(x, y))?;
        Some(if self.slopes[i].faint {
            EventKind::Select(SLOPE_BASE + i)
        } else {
            EventKind::Tab(SLOPE_BASE + i)
        })
    }
}

// ---------------------------------------------------------------- moves

/// The octaves either way the [`FilterMoves`] bars show round the cutoff.
pub const MOVES_OCTAVES: f32 = 4.0;

/// One row of [`FilterMoves`]: a name and what it does, a knob (or,
/// set elsewhere, its value and a jump there) and, for the cutoff's, a bar
/// of how far it can take the cutoff, in octaves.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRow {
    pub name: String,
    pub what: String,
    pub value: String,
    /// The page's row number its knob steps and a tap picks; None for a
    /// row set in another section (no knob).
    pub index: Option<usize>,
    /// The knob, lit this far round (0 to 1), from the middle if bipolar.
    pub pos: f32,
    pub bipolar: bool,
    /// How many steps take the knob from end to end ([`crate::panel::step_px`]).
    pub steps: u32,
    /// Octaves below and above the cutoff it can reach; None: no bar.
    pub span: Option<(f32, f32)>,
    /// A line beside the bar ("keys C2 to C6").
    pub aside: String,
    /// A button to where it is set ("Movement ›") and the number its tap
    /// gives as [`EventKind::Tab`].
    pub jump: Option<(String, usize)>,
    /// The Q-Link (0 to 3) that turns it, whose colour the knob and bar
    /// take: Q1's accent, Q2's good, Q3's warning and Q4's violet, as
    /// their cards.
    pub qlink: Option<usize>,
    pub faint: bool,
    pub picked: bool,
}

impl MoveRow {
    pub fn new(name: &str, what: &str, value: &str) -> MoveRow {
        MoveRow {
            name: name.into(),
            what: what.into(),
            value: value.into(),
            index: None,
            pos: 0.0,
            bipolar: false,
            steps: 20,
            span: None,
            aside: String::new(),
            jump: None,
            qlink: None,
            faint: false,
            picked: false,
        }
    }
}

/// What moves the filter (the filter gap plan's F5), under the section
/// chips in place of the pictures and panels: left, the cutoff's moves,
/// each with a knob and a bar on one octave scale round the cutoff;
/// right, the resonance's and the envelope's, and a line on where a hit
/// takes the filter. A knob dragged gives [`EventKind::Step`] with its
/// row's `index`; a tap on a row [`EventKind::Select`]; a jump
/// [`EventKind::Tab`] with its number; Done `Tab(DONE)`.
pub struct FilterMoves {
    pub title: String,
    pub note: String,
    /// The cutoff in Hz, the bars' middle.
    pub cutoff_hz: f32,
    pub cutoff: Vec<MoveRow>,
    /// Headed groups on the right.
    pub right: Vec<(String, Vec<MoveRow>)>,
    /// Under the rows: where the hardest and softest hits take it.
    pub hits: String,
    drag: Option<(u64, usize, i32)>,
}

const MOVE_ROW_H: i32 = 56;
const MOVE_NAME_W: i32 = 200;
const MOVE_KNOB_W: i32 = 130;
const MOVE_BAR_W: i32 = 300;
/// The line beside a cutoff row's bar ([`MoveRow::aside`]).
const MOVE_ASIDE_W: i32 = 160;

impl FilterMoves {
    pub fn new() -> FilterMoves {
        FilterMoves {
            title: "What moves the filter".into(),
            note: String::new(),
            cutoff_hz: 1000.0,
            cutoff: Vec::new(),
            right: Vec::new(),
            hits: String::new(),
            drag: None,
        }
    }

    fn done_rect(r: Rect) -> Rect {
        Rect::new(r.right() - 12 - 96, r.y + 8, 96, HEAD_BUTTON_H)
    }

    /// The left column's width: name, knob, bar and the line beside it,
    /// leaving the right column room for its rows.
    fn left_w(r: Rect) -> i32 {
        (MOVE_NAME_W + MOVE_KNOB_W + MOVE_BAR_W + MOVE_ASIDE_W).min(r.w - 28 - 24 - 300)
    }

    /// Every row's rect, with its row, in drawing order: the cutoff's,
    /// then the right's under their headings.
    fn row_rects(&self, r: Rect) -> Vec<(Rect, &MoveRow)> {
        let mut out = Vec::new();
        let lw = FilterMoves::left_w(r);
        // Under the title and the scale's labels.
        let mut y = r.y + 52 + 30;
        for row in &self.cutoff {
            out.push((Rect::new(r.x + 14, y, lw, MOVE_ROW_H), row));
            y += MOVE_ROW_H;
        }
        let x = r.x + 14 + lw + 24;
        let w = r.right() - 14 - x;
        let mut y = r.y + 52;
        for (_, rows) in &self.right {
            y += 30;
            for row in rows {
                out.push((Rect::new(x, y, w, MOVE_ROW_H), row));
                y += MOVE_ROW_H;
            }
            y += 8;
        }
        out
    }

    fn knob_rect(row: Rect) -> Rect {
        Rect::new(row.x + MOVE_NAME_W, row.y + 6, 44, 44)
    }

    fn jump_rect(row: Rect, text: &str) -> Rect {
        let w = 24 + 8 * text.chars().count() as i32;
        Rect::new(row.x + MOVE_NAME_W + MOVE_KNOB_W - w - 8, row.y + 13, w, 30)
    }

    /// The bar's place in a cutoff row.
    fn bar_rect(row: Rect) -> Rect {
        let x = row.x + MOVE_NAME_W + MOVE_KNOB_W;
        Rect::new(x, row.y, (row.right() - x).min(MOVE_BAR_W), MOVE_ROW_H)
    }

    fn x_of(bar: Rect, oct: f32) -> i32 {
        let u = (oct.clamp(-MOVES_OCTAVES, MOVES_OCTAVES) + MOVES_OCTAVES) / (2.0 * MOVES_OCTAVES);
        bar.x + (u * bar.w as f32).round() as i32
    }
}

impl Default for FilterMoves {
    fn default() -> FilterMoves {
        FilterMoves::new()
    }
}

/// A frequency as short as the scale labels it: "62 Hz", "250", "1 kHz", "4k".
fn scale_label(hz: f32, first: bool) -> String {
    let (n, unit) = if hz >= 1000.0 {
        let k = hz / 1000.0;
        let n = if k >= 10.0 || k.fract() < 0.05 {
            format!("{k:.0}")
        } else {
            format!("{k:.1}")
        };
        (n, if first { " kHz" } else { "k" })
    } else {
        (format!("{hz:.0}"), if first { " Hz" } else { "" })
    };
    format!("{n}{unit}")
}

impl Widget for FilterMoves {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let done = FilterMoves::done_rect(r);
        let head = Rect::new(r.x + 14, r.y + 12, done.x - r.x - 24, 26);
        let title = self.title.to_uppercase();
        let tw = cx.measure(FontId::Bold, t.size_small, &title);
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            &title,
        );
        let note = Rect::new(head.x + tw + 14, head.y, (head.w - tw - 14).max(0), 26);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            note,
            Align::Left,
            t.text_faint,
            &self.note,
        );
        cx.canvas.fill_round_rect(done, t.round(8), t.raised);
        cx.text(FontId::Bold, 15.0, done, Align::Center, t.text, "Done");
        let lw = FilterMoves::left_w(r);
        // Headings: Cutoff with its scale; the right's groups.
        let heading = |cx: &mut PaintCx, x: i32, y: i32, w: i32, text: &str| {
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                Rect::new(x, y, w, 24),
                Align::Left,
                t.text_dim,
                &text.to_uppercase(),
            );
        };
        let top = r.y + 52;
        heading(cx, r.x + 14, top, MOVE_NAME_W, "Cutoff");
        let rows = self.row_rects(r);
        if let Some((first, _)) = rows.first().filter(|_| !self.cutoff.is_empty()) {
            let bar = FilterMoves::bar_rect(*first);
            for (k, oct) in [-4.0f32, -2.0, 0.0, 2.0, 4.0].into_iter().enumerate() {
                let x = FilterMoves::x_of(bar, oct);
                let label = scale_label(self.cutoff_hz * oct.exp2(), k == 0);
                let w = cx.measure(FontId::Sans, t.size_small - 2.0, &label) + 2;
                let lx = (x - w / 2).clamp(bar.x, bar.right() - w);
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    Rect::new(lx, top + 4, w, 18),
                    Align::Left,
                    t.text_faint,
                    &label,
                );
            }
        }
        let mut y = top;
        let rx = r.x + 14 + lw + 24;
        for (name, group) in &self.right {
            heading(cx, rx, y, r.right() - 14 - rx, name);
            y += 30 + MOVE_ROW_H * group.len() as i32 + 8;
        }
        for (rect, row) in &rows {
            cx.canvas
                .fill_rect(Rect::new(rect.x, rect.y, rect.w, 1), t.outline);
            if row.picked {
                cx.canvas
                    .fill_round_rect(rect.inset(1), t.round(6), t.raised.with_alpha(160));
            }
            let ink = if row.faint { t.text_faint } else { t.text };
            let tint = match row.qlink {
                Some(1) => t.good,
                Some(2) => t.warn,
                Some(3) => crate::widgets::PAD_COLORS[4],
                _ => t.accent,
            };
            cx.text(
                FontId::Bold,
                16.0,
                Rect::new(rect.x + 8, rect.y + 7, MOVE_NAME_W - 12, 22),
                Align::Left,
                ink,
                &row.name,
            );
            cx.text(
                FontId::Sans,
                12.0,
                Rect::new(rect.x + 8, rect.y + 30, MOVE_NAME_W - 12, 18),
                Align::Left,
                t.text_dim,
                &row.what,
            );
            let kr = FilterMoves::knob_rect(*rect);
            if row.index.is_some() {
                let lit = if row.faint { t.text_faint } else { tint };
                crate::panel::knob(
                    cx,
                    (kr.x as f32 + 22.0, kr.y as f32 + 22.0),
                    40.0,
                    row.pos,
                    row.bipolar,
                    [t.outline, lit, ink],
                );
                cx.text(
                    FontId::Bold,
                    15.0,
                    Rect::new(kr.right() + 6, rect.y, MOVE_KNOB_W - 50, MOVE_ROW_H),
                    Align::Left,
                    ink,
                    &row.value,
                );
            } else {
                cx.text(
                    FontId::Bold,
                    15.0,
                    Rect::new(kr.x, rect.y, 60, MOVE_ROW_H),
                    Align::Left,
                    ink,
                    &row.value,
                );
            }
            if let Some((text, _)) = &row.jump {
                let jr = FilterMoves::jump_rect(*rect, text);
                cx.canvas.fill_round_rect(jr, t.round(8), t.raised);
                cx.text(FontId::Sans, 13.0, jr, Align::Center, t.text, text);
            }
            // The bar: how far this one can take the cutoff.
            if rect.x < rx - 10 {
                let bar = FilterMoves::bar_rect(*rect);
                let mid = bar.y + bar.h / 2;
                cx.canvas
                    .fill_rect(Rect::new(bar.x, mid - 1, bar.w, 2), t.outline);
                let cx0 = FilterMoves::x_of(bar, 0.0);
                cx.canvas
                    .fill_rect(Rect::new(cx0 - 1, mid - 14, 2, 28), t.text_faint);
                if let Some((lo, hi)) = row.span.filter(|(lo, hi)| hi > lo) {
                    let (a, b) = (FilterMoves::x_of(bar, lo), FilterMoves::x_of(bar, hi));
                    let fill = if row.faint {
                        t.text_faint.with_alpha(120)
                    } else {
                        tint.with_alpha(200)
                    };
                    cx.canvas
                        .fill_round_rect(Rect::new(a, mid - 7, (b - a).max(4), 14), 7, fill);
                }
                if !row.aside.is_empty() {
                    let ax = bar.right() + 10;
                    cx.text(
                        FontId::Sans,
                        12.0,
                        Rect::new(ax, rect.y, (rect.right() - ax).max(0), MOVE_ROW_H),
                        Align::Left,
                        t.text_dim,
                        &row.aside,
                    );
                }
            }
        }
        let foot = Rect::new(r.x + 14, r.bottom() - 14 - 40, r.w - 28, 40);
        cx.canvas.fill_round_rect(foot, t.round(8), t.raised);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            foot.inset(10),
            Align::Left,
            t.text,
            &self.hits,
        );
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = vec![Target::tap("done", 0, "Done", FilterMoves::done_rect(r))];
        for (rect, row) in self.row_rects(r) {
            if let Some(i) = row.index {
                out.push(
                    Target::drag("knob", i, row.name.clone(), FilterMoves::knob_rect(rect))
                        .towards(0, -60)
                        .disabled_if(row.faint),
                );
            }
            if let Some((text, k)) = &row.jump {
                out.push(Target::tap(
                    "jump",
                    *k,
                    text.clone(),
                    FilterMoves::jump_rect(rect, text),
                ));
            }
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                if FilterMoves::done_rect(r).contains(x, y) {
                    return Some(EventKind::Tab(DONE));
                }
                let (rect, row) = self
                    .row_rects(r)
                    .into_iter()
                    .find(|(rect, _)| rect.contains(x, y))
                    .map(|(rect, row)| (rect, row.clone()))?;
                if let Some((text, k)) = &row.jump {
                    if FilterMoves::jump_rect(rect, text).contains(x, y) {
                        return Some(EventKind::Tab(*k));
                    }
                }
                let i = row.index?;
                if !row.faint
                    && FilterMoves::knob_rect(rect)
                        .inset_xy(-10, -6)
                        .contains(x, y)
                {
                    self.drag = Some((finger, i, y));
                }
                Some(EventKind::Select(i))
            }
            Touch::Move => {
                let (f, i, from) = self.drag?;
                if f != finger {
                    return None;
                }
                let steps = self
                    .row_rects(r)
                    .iter()
                    .find(|(_, row)| row.index == Some(i))
                    .map_or(20, |(_, row)| row.steps);
                let px = crate::panel::step_px(steps);
                let n = (from - y) / px;
                if n == 0 {
                    return None;
                }
                self.drag = Some((f, i, from - n * px));
                Some(EventKind::Step { index: i, delta: n })
            }
            Touch::Up | Touch::Cancel => {
                self.drag.take_if(|(f, ..)| *f == finger);
                None
            }
        }
    }
}

// ---------------------------------------------------------------- movement

/// How far the pitch envelope has fallen at the end of its decay, as a
/// power of e: the engine's ([`mpfree_sampler::engine`]), a ten-thousandth.
const ENV_FALL: f32 = 9.21;
/// The random waves' values, one a cycle, for the picture.
const RANDOM: [f32; 4] = [0.55, -0.35, 0.9, -0.7];

/// The LFO's wave at `phase` cycles from its start (-1 to 1), as the
/// picture draws it: the random waves from a fixed run of values.
pub fn lfo_shape(shape: LfoShape, phase: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let f = phase.rem_euclid(1.0);
    let cycle = phase.max(0.0) as usize;
    let held = |k: usize| RANDOM[k % RANDOM.len()];
    match shape {
        LfoShape::Sine => (f * TAU).sin(),
        LfoShape::Triangle => 1.0 - 4.0 * ((f + 0.25).fract() - 0.5).abs(),
        LfoShape::SawUp => 2.0 * f - 1.0,
        LfoShape::SawDown => 1.0 - 2.0 * f,
        LfoShape::Square => {
            if f < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        LfoShape::Hold => held(cycle + 1),
        LfoShape::Drift => {
            let t = 0.5 - 0.5 * (f * PI).cos();
            held(cycle) + (held(cycle + 1) - held(cycle)) * t
        }
    }
}

/// The Movement section's picture: two cycles of the LFO's wave at its
/// depth (left), and the pitch envelope falling back to the pad's pitch
/// over its decay (right), each faint when it does nothing. It only
/// shows: the controls are on the panel.
pub struct MotionView {
    pub title: String,
    pub right: String,
    pub caption: String,
    pub lfo: PadLfo,
    pub env: PitchEnv,
    /// Whether the LFO and the envelope are heard on this pad.
    pub lfo_on: bool,
    pub env_on: bool,
}

impl MotionView {
    pub fn new() -> MotionView {
        MotionView {
            title: "Movement".into(),
            right: String::new(),
            caption: String::new(),
            lfo: PadLfo::default(),
            env: PitchEnv::default(),
            lfo_on: false,
            env_on: false,
        }
    }

    /// The two plots: the LFO's and the envelope's.
    fn plots(r: Rect) -> (Rect, Rect) {
        let b = body(r);
        let plot = Rect::new(b.x, b.y + 4, b.w.max(1), (b.h - 4 - 24).max(1));
        let gap = 24;
        let left = (plot.w - gap) * 3 / 5;
        (
            Rect::new(plot.x, plot.y, left.max(1), plot.h),
            Rect::new(
                plot.x + left + gap,
                plot.y,
                (plot.w - left - gap).max(1),
                plot.h,
            ),
        )
    }
}

impl Default for MotionView {
    fn default() -> MotionView {
        MotionView::new()
    }
}

impl Widget for MotionView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        titled(cx, r, &self.title, &self.right);
        let (lp, ep) = MotionView::plots(r);
        let half = |p: Rect| (p.h / 2 - 6).max(1) as f32;
        for (p, name) in [(lp, "LFO · 2 cycles"), (ep, "Pitch envelope")] {
            let mid = p.y + p.h / 2;
            cx.canvas.fill_rect(Rect::new(p.x, mid, p.w, 1), t.outline);
            cx.canvas.fill_rect(Rect::new(p.x, p.y, 1, p.h), t.outline);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(p.x + 6, p.y, p.w - 6, 16),
                Align::Left,
                t.text_faint,
                name,
            );
        }
        // The LFO: two cycles across, up for a positive depth.
        let color = if self.lfo_on { t.good } else { t.text_faint };
        let (mid, amp) = (lp.y + lp.h / 2, half(lp));
        let depth = self.lfo.depth.clamp(-1.0, 1.0);
        let ys: Vec<i32> = (0..=lp.w)
            .map(|i| {
                let phase = 2.0 * i as f32 / lp.w.max(1) as f32;
                let v = lfo_shape(self.lfo.shape, phase.min(1.9999)) * depth;
                mid - (v * amp).round() as i32
            })
            .collect();
        polyline(cx, lp.x, &ys, 3, color);
        // The envelope: from its start back to the pad's pitch over its
        // decay, on the scale of the furthest start (24 semitones).
        let color = if self.env_on { t.accent } else { t.text_faint };
        let (mid, amp) = (ep.y + ep.h / 2, half(ep));
        let start = (self.env.semitones / PITCH_ENV_ST).clamp(-1.0, 1.0);
        let ys: Vec<i32> = (0..=ep.w)
            .map(|i| {
                let u = i as f32 / ep.w.max(1) as f32;
                mid - (start * (-ENV_FALL * u).exp() * amp).round() as i32
            })
            .collect();
        polyline(cx, ep.x, &ys, 3, color);
        if self.env_on {
            dot(cx, ep.x, ys[0], 12, t.accent);
        }
        let caption = Rect::new(lp.x, lp.bottom() + 6, ep.right() - lp.x, 20);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            caption,
            Align::Left,
            t.text_dim,
            &self.caption,
        );
    }
}
