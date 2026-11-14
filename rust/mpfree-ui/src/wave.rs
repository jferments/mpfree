//! Waveform widgets for the sampler's Sound page (the Wave tab in the
//! sampler mockups):
//!
//! - [`Waveform`]: the zoomable waveform, with marker tabs along its top
//!   (start, end, loop points, slices), the region outside start and end
//!   dimmed, the loop span shaded, fades drawn as curves, bar lines and
//!   playheads. Drag a marker by its tab, drag the wave to scroll, pinch to
//!   zoom about the fingers, tap to pick a frame.
//! - [`WaveOverview`]: the whole sound in a strip, with a box round what the
//!   waveform shows. Touch it to move the box.
//!
//! Both draw from a [`Peaks`] source: min, max and RMS for each pixel
//! column. The sampler's source is mpfree-analysis's peak pyramid, which
//! answers any zoom in one lookup per column, so drawing costs the same at
//! every zoom. [`RawPeaks`] reads the samples themselves, for short sounds
//! and tests.
//!
//! A playhead moving redraws only the narrow strips it left and entered,
//! not the whole waveform.

use std::cell::RefCell;
use std::sync::Arc;

use crate::color::Color;
use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::theme::Theme;
use crate::ui::{EventKind, PaintCx, Target, Touch, Widget};

/// One pixel column of a waveform, in full scale (-1 to 1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaveColumn {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

/// Where a waveform's columns come from.
pub trait Peaks: Send + Sync {
    fn frames(&self) -> u64;
    fn channels(&self) -> usize;
    /// The column covering frames `f0` to `f1` of channel `ch`; all zero
    /// past either end of the sound.
    fn column(&self, ch: usize, f0: f64, f1: f64) -> WaveColumn;
}

/// Peaks read straight from interleaved samples. Fine for short sounds and
/// close zooms; long sounds zoomed out want the peak pyramid instead.
pub struct RawPeaks {
    data: Arc<[f32]>,
    channels: usize,
}

impl RawPeaks {
    /// `None` if `channels` is 0 or the data isn't whole frames.
    pub fn new(data: Arc<[f32]>, channels: usize) -> Option<RawPeaks> {
        (channels > 0 && data.len().is_multiple_of(channels)).then_some(RawPeaks { data, channels })
    }
}

impl Peaks for RawPeaks {
    fn frames(&self) -> u64 {
        (self.data.len() / self.channels) as u64
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn column(&self, ch: usize, f0: f64, f1: f64) -> WaveColumn {
        let frames = self.frames() as f64;
        if f1 <= 0.0 || f0 >= frames || ch >= self.channels {
            return WaveColumn::default();
        }
        let a = f0.max(0.0).floor() as usize;
        let b = (f1.min(frames).ceil() as usize).max(a + 1);
        let (mut mn, mut mx, mut sq) = (f32::MAX, f32::MIN, 0.0f32);
        for f in a..b {
            let x = self.data[f * self.channels + ch];
            mn = mn.min(x);
            mx = mx.max(x);
            sq += x * x;
        }
        WaveColumn {
            min: mn,
            max: mx,
            rms: (sq / (b - a) as f32).sqrt(),
        }
    }
}

/// Which frames a waveform shows: the frame at its left edge and how many
/// frames each pixel covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveView {
    pub start: f64,
    pub frames_per_px: f64,
}

impl WaveView {
    /// The closest zoom: 16 pixels per frame.
    pub const MIN_FRAMES_PER_PX: f64 = 1.0 / 16.0;

    /// All of `frames` across `width` pixels.
    pub fn whole(frames: u64, width: i32) -> WaveView {
        WaveView {
            start: 0.0,
            frames_per_px: frames.max(1) as f64 / f64::from(width.max(1)),
        }
    }

    /// The frame under pixel `x` (counted from the waveform's left edge).
    pub fn frame_at(&self, x: f64) -> f64 {
        self.start + x * self.frames_per_px
    }

    /// The pixel (from the left edge) of `frame`.
    pub fn x_of(&self, frame: f64) -> f64 {
        (frame - self.start) / self.frames_per_px
    }

    /// The frame at the right edge of a waveform `width` pixels wide.
    pub fn end(&self, width: i32) -> f64 {
        self.frame_at(f64::from(width))
    }

    /// Kept inside the sound: no closer than [`Self::MIN_FRAMES_PER_PX`], no
    /// further out than the whole sound, and not scrolled past either end.
    pub fn clamped(self, frames: u64, width: i32) -> WaveView {
        let whole = WaveView::whole(frames, width);
        let fpp = if self.frames_per_px.is_finite() {
            self.frames_per_px
        } else {
            whole.frames_per_px
        };
        let fpp = fpp.clamp(
            Self::MIN_FRAMES_PER_PX.min(whole.frames_per_px),
            whole.frames_per_px,
        );
        let span = fpp * f64::from(width.max(1));
        let start = if self.start.is_finite() {
            self.start
        } else {
            0.0
        };
        WaveView {
            start: start.clamp(0.0, (frames.max(1) as f64 - span).max(0.0)),
            frames_per_px: fpp,
        }
    }

    /// Zoomed in by `factor` (2 is twice as close) with `frame` staying
    /// under pixel `x`, then [`clamped`](Self::clamped).
    pub fn zoomed(self, factor: f64, frame: f64, x: f64, frames: u64, width: i32) -> WaveView {
        let fpp = self.frames_per_px / factor.max(1e-9);
        WaveView {
            start: frame - x * fpp,
            frames_per_px: fpp,
        }
        .clamped(frames, width)
    }
}

/// What a marker marks; it sets the colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    /// A region's start or end.
    Edge,
    /// A loop point.
    Loop,
    /// A slice of a chop.
    Slice,
    /// Anything else the app wants to point at.
    Cue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub frame: u64,
    pub label: String,
    pub kind: MarkerKind,
    /// Stays put: its tab shows, a touch on it still comes as
    /// [`EventKind::Marker`] (so the owner can say why), but a drag doesn't
    /// move it.
    pub fixed: bool,
}

impl Marker {
    pub fn new(frame: u64, label: &str, kind: MarkerKind) -> Marker {
        Marker {
            frame,
            label: label.to_string(),
            kind,
            fixed: false,
        }
    }

    /// The same marker, staying put ([`Marker::fixed`]).
    pub fn fixed(mut self) -> Marker {
        self.fixed = true;
        self
    }
}

/// Bar lines: the frame of the first downbeat, frames per beat and beats
/// per bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beats {
    pub first: f64,
    pub frames_per_beat: f64,
    pub per_bar: u32,
}

/// The strip of marker tabs along the top.
const TABS_H: i32 = 30;
/// A touch that moves less than this is a tap.
const TAP_SLOP: i32 = 8;
/// Width of the strip redrawn round a playhead.
const PLAYHEAD_W: i32 = 2;

fn marker_color(t: &Theme, k: MarkerKind) -> Color {
    match k {
        MarkerKind::Edge => t.accent,
        MarkerKind::Loop => t.good,
        MarkerKind::Slice => t.text,
        MarkerKind::Cue => t.warn,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// Dragging marker `index`.
    Marker(usize),
    /// Maybe a tap, until it moves; then a scroll from this view.
    Wave {
        x0: i32,
        view: WaveView,
        moved: bool,
    },
}

/// The zoomable waveform. The app sets what it shows; touches come back as
/// [`EventKind::Marker`] (a marker dragged to a frame; the widget has moved
/// it already, and the app may move it back), [`EventKind::View`] (scrolled
/// or zoomed; the widget's view has changed already) and
/// [`EventKind::Frame`] (a tap on that frame).
pub struct Waveform {
    peaks: Option<Arc<dyn Peaks>>,
    view: WaveView,
    markers: Vec<Marker>,
    region: Option<(u64, u64)>,
    loop_span: Option<(u64, u64)>,
    fades: (u64, u64),
    beats: Option<Beats>,
    playheads: Vec<u64>,
    /// Fingers on the wave, by id, with their x.
    fingers: Vec<(u64, i32)>,
    grab: Option<(u64, Grab)>,
    /// Two fingers: their distance, the view and the frame between them when the pinch began.
    pinch: Option<(f64, WaveView, f64)>,
    /// What was painted last: the wave's width and the playheads' x, so a
    /// playhead move redraws only its strips.
    painted: RefCell<Painted>,
}

#[derive(Default)]
struct Painted {
    /// True once a full paint has happened and only playheads changed since.
    only_playheads: bool,
    xs: Vec<i32>,
    /// Each marker's tab as drawn, for touches (None: not drawn).
    tabs: Vec<Option<Rect>>,
}

impl Default for Waveform {
    fn default() -> Self {
        Waveform::new()
    }
}

impl Waveform {
    pub fn new() -> Waveform {
        Waveform {
            peaks: None,
            view: WaveView {
                start: 0.0,
                frames_per_px: 1.0,
            },
            markers: Vec::new(),
            region: None,
            loop_span: None,
            fades: (0, 0),
            beats: None,
            playheads: Vec::new(),
            fingers: Vec::new(),
            grab: None,
            pinch: None,
            painted: RefCell::new(Painted::default()),
        }
    }

    fn changed(&mut self) {
        self.painted.get_mut().only_playheads = false;
    }

    /// What to draw, shown whole across `width` pixels (the width of
    /// [`Waveform::wave_rect`]).
    pub fn set_peaks(&mut self, peaks: Option<Arc<dyn Peaks>>, width: i32) {
        self.view = WaveView::whole(peaks.as_ref().map_or(1, |p| p.frames()), width);
        self.peaks = peaks;
        self.changed();
    }

    pub fn frames(&self) -> u64 {
        self.peaks.as_ref().map_or(0, |p| p.frames())
    }

    pub fn view(&self) -> WaveView {
        self.view
    }

    /// Shows `view` (clamped to the sound) on a waveform `width` pixels wide.
    pub fn set_view(&mut self, view: WaveView, width: i32) {
        self.view = view.clamped(self.frames(), width);
        self.changed();
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    pub fn set_markers(&mut self, markers: Vec<Marker>) {
        self.markers = markers;
        self.changed();
    }

    /// Moves one marker (when the model refuses a drag, put it back).
    pub fn set_marker_frame(&mut self, index: usize, frame: u64) {
        if let Some(m) = self.markers.get_mut(index) {
            m.frame = frame;
            self.changed();
        }
    }

    /// Frames outside `start..end` are dimmed.
    pub fn set_region(&mut self, region: Option<(u64, u64)>) {
        self.region = region;
        self.changed();
    }

    /// Shades the loop.
    pub fn set_loop(&mut self, span: Option<(u64, u64)>) {
        self.loop_span = span;
        self.changed();
    }

    /// Fade-in and fade-out lengths in frames, drawn from the region's ends.
    pub fn set_fades(&mut self, fade_in: u64, fade_out: u64) {
        self.fades = (fade_in, fade_out);
        self.changed();
    }

    pub fn beats(&self) -> Option<Beats> {
        self.beats
    }

    pub fn set_beats(&mut self, beats: Option<Beats>) {
        self.beats = beats
            .filter(|b| b.frames_per_beat.is_finite() && b.frames_per_beat > 0.0 && b.per_bar > 0);
        self.changed();
    }

    /// Where the playing voices are. Only this changing redraws only the
    /// playheads' strips.
    pub fn set_playheads(&mut self, frames: &[u64]) {
        self.playheads.clear();
        self.playheads.extend_from_slice(frames);
    }

    /// Zooms by `factor` about the middle of the view, as the data wheel
    /// does; returns the new view.
    pub fn zoom(&mut self, factor: f64, width: i32) -> WaveView {
        let mid = f64::from(width) / 2.0;
        self.view = self
            .view
            .zoomed(factor, self.view.frame_at(mid), mid, self.frames(), width);
        self.changed();
        self.view
    }

    /// The part of `r` the wave itself fills: below the marker tabs, clear
    /// of the panel's rounded corners. Views are measured in its width.
    pub fn wave_rect(r: Rect) -> Rect {
        let below = r.take_top(TABS_H).1;
        Rect::new(
            below.x + 4,
            below.y,
            (below.w - 8).max(1),
            (below.h - 4).max(1),
        )
    }

    fn x_in(&self, wave: Rect, frame: f64) -> f64 {
        f64::from(wave.x) + self.view.x_of(frame)
    }

    /// The tab of marker `i`, `w` pixels wide, centred on the marker and kept
    /// inside the strip; None when a slice or cue is off screen (start, end
    /// and loop tabs stay pinned to the edge, to show which way they are).
    fn tab_rect(&self, r: Rect, i: usize, w: i32) -> Option<Rect> {
        let m = &self.markers[i];
        let wave = Self::wave_rect(r);
        let x = self.x_in(wave, m.frame as f64).round() as i32;
        let pinned = matches!(m.kind, MarkerKind::Edge | MarkerKind::Loop);
        if !pinned && (x < wave.x || x > wave.right()) {
            return None;
        }
        let left = (x - w / 2).clamp(r.x + 4, (r.right() - 4 - w).max(r.x + 4));
        Some(Rect::new(left, r.y + 3, w, TABS_H - 6))
    }

    fn lanes(&self, wave: Rect) -> Vec<Rect> {
        let wave = wave.inset_xy(0, 6);
        let n = self.peaks.as_ref().map_or(1, |p| p.channels().clamp(1, 2)) as i32;
        let gap = if n > 1 { 6 } else { 0 };
        let h = (wave.h - gap * (n - 1)) / n;
        (0..n)
            .map(|i| Rect::new(wave.x, wave.y + i * (h + gap), wave.w, h))
            .collect()
    }

    fn bar_label(b: &Beats, beat: i64) -> String {
        let per = i64::from(b.per_bar);
        format!("{}.{}", beat.div_euclid(per) + 1, beat.rem_euclid(per) + 1)
    }
}

impl Widget for Waveform {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let clip = cx.canvas.clip().intersect(r);
        cx.canvas.fill_round_rect(r, t.radius, t.surface);
        // The tab strip: the panel's top, darker, keeping its round corners.
        let strip = r.take_top(TABS_H).0;
        cx.canvas.set_clip(clip.intersect(strip));
        cx.canvas
            .fill_round_rect(r, t.radius, t.raised.with_alpha(110));
        cx.canvas.set_clip(clip);
        let wave = Self::wave_rect(r);
        let frames = self.frames() as f64;
        let fpp = self.view.frames_per_px;
        let (x0, x1) = (clip.x.max(wave.x), clip.right().min(wave.right()));

        // Behind the wave: the loop span and bar lines.
        if let Some((a, b)) = self.loop_span {
            let (xa, xb) = (
                self.x_in(wave, a as f64).round() as i32,
                self.x_in(wave, b as f64).round() as i32,
            );
            let span = Rect::new(
                xa.max(wave.x),
                wave.y,
                (xb.min(wave.right()) - xa.max(wave.x)).max(0),
                wave.h,
            );
            cx.canvas.fill_rect(span, t.good.with_alpha(34));
        }
        if let Some(b) = self.beats {
            let first = ((self.view.start - b.first) / b.frames_per_beat).floor() as i64;
            let last = ((self.view.end(wave.w) - b.first) / b.frames_per_beat).ceil() as i64;
            let px_per_beat = b.frames_per_beat / fpp;
            // Beats when they are far enough apart, else only bars.
            let every = if px_per_beat >= 24.0 {
                1
            } else {
                i64::from(b.per_bar)
            };
            if (last - first) / every <= 400 {
                for beat in first..=last {
                    if beat.rem_euclid(every) != 0 {
                        continue;
                    }
                    let x = self
                        .x_in(wave, b.first + beat as f64 * b.frames_per_beat)
                        .round() as i32;
                    if x < wave.x || x >= wave.right() {
                        continue;
                    }
                    let bar = beat.rem_euclid(i64::from(b.per_bar)) == 0;
                    cx.canvas.fill_rect(
                        Rect::new(x, wave.y, 1, wave.h),
                        if bar { t.text_faint } else { t.outline },
                    );
                    if bar && b.frames_per_beat * f64::from(b.per_bar) / fpp >= 48.0 {
                        let label = Rect::new(x + 5, wave.y + 4, 60, 18);
                        cx.text(
                            FontId::Mono,
                            13.0,
                            label,
                            Align::Left,
                            t.text_faint,
                            &Self::bar_label(&b, beat),
                        );
                    }
                }
            }
        }

        // The wave, one column at a time, only where the clip needs it.
        if let Some(p) = &self.peaks {
            let (on, off) = (t.accent, t.accent.mix(t.surface, 0.62));
            let (on_rms, off_rms) = (on.mix(t.text, 0.35), off.mix(t.text, 0.15));
            let (rs, re) = self
                .region
                .map_or((0.0, frames), |(a, b)| (a as f64, b as f64));
            for (ch, lane) in self.lanes(wave).into_iter().enumerate() {
                let mid = lane.y + lane.h / 2;
                let half = (lane.h / 2 - 2).max(1) as f32;
                let y = |v: f32| mid - (v.clamp(-1.0, 1.0) * half).round() as i32;
                cx.canvas
                    .fill_rect(Rect::new(x0, mid, (x1 - x0).max(0), 1), t.outline);
                for x in x0..x1 {
                    let f0 = self.view.frame_at(f64::from(x - wave.x));
                    let c = p.column(ch, f0, f0 + fpp);
                    if c.max < c.min {
                        continue;
                    }
                    let inside = f0 + fpp > rs && f0 < re;
                    let (top, bot) = (y(c.max), y(c.min));
                    cx.canvas.fill_rect(
                        Rect::new(x, top, 1, (bot - top).max(1)),
                        if inside { on } else { off },
                    );
                    let (rt, rb) = (y(c.rms.min(c.max)), y((-c.rms).max(c.min)));
                    if rb > rt {
                        cx.canvas.fill_rect(
                            Rect::new(x, rt, 1, rb - rt),
                            if inside { on_rms } else { off_rms },
                        );
                    }
                }
            }
            // Fades, as gain curves from the region's ends.
            let (fi, fo) = self.fades;
            let curve = |cx: &mut PaintCx, a: f64, b: f64, rising: bool| {
                if b <= a {
                    return;
                }
                let xa = (self.x_in(wave, a).floor() as i32).max(x0);
                let xb = (self.x_in(wave, b).ceil() as i32).min(x1);
                let y_at = |x: i32| {
                    let f = self.view.frame_at(f64::from(x - wave.x));
                    let g = ((f - a) / (b - a)).clamp(0.0, 1.0);
                    let g = if rising { g } else { 1.0 - g };
                    wave.bottom() - 2 - ((wave.h - 4) as f64 * g).round() as i32
                };
                // Each column joins its point to the next, so steep curves stay unbroken.
                for x in xa..xb {
                    let (y0, y1) = (y_at(x), y_at(x + 1));
                    let (top, bot) = (y0.min(y1), y0.max(y1));
                    cx.canvas
                        .fill_rect(Rect::new(x, top - 1, 2, bot - top + 2), t.warn);
                }
            };
            if fi > 0 {
                curve(cx, rs, rs + fi as f64, true);
            }
            if fo > 0 {
                curve(cx, re - fo as f64, re, false);
            }
        }

        // Markers: a line down the wave and a tab in the strip.
        let mut tabs = Vec::with_capacity(self.markers.len());
        for i in 0..self.markers.len() {
            let m = &self.markers[i];
            let c = marker_color(&t, m.kind);
            let x = self.x_in(wave, m.frame as f64).round() as i32;
            if x >= wave.x && x <= wave.right() {
                cx.canvas.fill_rect(Rect::new(x - 1, wave.y, 2, wave.h), c);
            }
            let w = (cx.measure(FontId::Bold, 13.0, &m.label) + 18).max(24);
            let tab = self.tab_rect(r, i, w);
            if let Some(tab) = tab {
                cx.canvas.fill_round_rect(tab, 6, c);
                cx.text(FontId::Bold, 13.0, tab, Align::Center, t.bg, &m.label);
            }
            tabs.push(tab);
        }

        // Playheads last, on top.
        let mut painted = self.painted.borrow_mut();
        if clip.intersect(strip) == strip {
            painted.tabs = tabs;
        }
        painted.xs.clear();
        for &f in &self.playheads {
            let x = self.x_in(wave, f as f64).round() as i32;
            if x >= wave.x && x < wave.right() {
                cx.canvas
                    .fill_rect(Rect::new(x, wave.y, PLAYHEAD_W, wave.h), t.text);
                painted.xs.push(x);
            }
        }
        painted.only_playheads = true;
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let painted = self.painted.borrow();
        if !painted.only_playheads {
            return None;
        }
        let wave = Self::wave_rect(r);
        let mut xs: Vec<i32> = painted.xs.clone();
        for &f in &self.playheads {
            let x = self.x_in(wave, f as f64).round() as i32;
            if x >= wave.x && x < wave.right() {
                xs.push(x);
            }
        }
        // A strip a pixel wider each side, and the marker tabs above stay put.
        Some(
            xs.into_iter()
                .map(|x| Rect::new(x - 1, wave.y, PLAYHEAD_W + 2, wave.h))
                .collect(),
        )
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if self.peaks.is_none() {
            return Vec::new();
        }
        let wave = Self::wave_rect(r);
        // A tap picks a frame (and plays from it), a drag scrolls; the
        // marker tabs drag their marker.
        let mut out = vec![Target::tap("wave", 0, "waveform", wave)];
        let painted = self.painted.borrow();
        for (i, tab) in painted.tabs.iter().enumerate().take(self.markers.len()) {
            if let Some(t) = tab {
                out.push(
                    Target::drag("marker", i, self.markers[i].label.clone(), *t)
                        .disabled_if(self.markers[i].fixed),
                );
            }
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let wave = Self::wave_rect(r);
        let frames = self.frames();
        match phase {
            Touch::Down => {
                self.peaks.as_ref()?;
                self.fingers.retain(|(f, _)| *f != finger);
                self.fingers.push((finger, x));
                if self.fingers.len() == 2 {
                    // A second finger: a pinch, whatever the first was doing.
                    let (a, b) = (self.fingers[0].1, self.fingers[1].1);
                    let mid = f64::from(a + b) / 2.0 - f64::from(wave.x);
                    self.pinch = Some((
                        f64::from((a - b).abs()).max(1.0),
                        self.view,
                        self.view.frame_at(mid),
                    ));
                    self.grab = None;
                    return None;
                }
                if self.fingers.len() > 2 {
                    return None;
                }
                if y < wave.y {
                    // The tab under the finger (the last drawn is on top),
                    // with a little room round it for a fingertip.
                    let tabs = &self.painted.get_mut().tabs;
                    let hit = (0..self.markers.len().min(tabs.len()))
                        .rev()
                        .find(|&i| tabs[i].is_some_and(|t| t.inset_xy(-6, -3).contains(x, y)));
                    if let Some(i) = hit {
                        self.grab = Some((finger, Grab::Marker(i)));
                        return Some(EventKind::Marker {
                            index: i,
                            frame: self.markers[i].frame,
                        });
                    }
                }
                self.grab = Some((
                    finger,
                    Grab::Wave {
                        x0: x,
                        view: self.view,
                        moved: false,
                    },
                ));
                None
            }
            Touch::Move => {
                if let Some(f) = self.fingers.iter_mut().find(|(f, _)| *f == finger) {
                    f.1 = x;
                }
                if let (Some((d0, v0, frame)), 2) = (self.pinch, self.fingers.len()) {
                    let (a, b) = (self.fingers[0].1, self.fingers[1].1);
                    let d = f64::from((a - b).abs()).max(1.0);
                    let mid = f64::from(a + b) / 2.0 - f64::from(wave.x);
                    self.view = v0.zoomed(d / d0, frame, mid, frames, wave.w);
                    self.changed();
                    return Some(EventKind::View {
                        start: self.view.start,
                        frames_per_px: self.view.frames_per_px,
                    });
                }
                match self.grab {
                    Some((g, Grab::Marker(i))) if g == finger => {
                        if self.markers.get(i).is_some_and(|m| m.fixed) {
                            return None;
                        }
                        let f = self
                            .view
                            .frame_at(f64::from(x - wave.x))
                            .round()
                            .clamp(0.0, frames as f64) as u64;
                        self.set_marker_frame(i, f);
                        Some(EventKind::Marker { index: i, frame: f })
                    }
                    Some((g, Grab::Wave { x0, view, moved })) if g == finger => {
                        if !moved && (x - x0).abs() < TAP_SLOP {
                            return None;
                        }
                        self.grab = Some((
                            g,
                            Grab::Wave {
                                x0,
                                view,
                                moved: true,
                            },
                        ));
                        let start = view.start - f64::from(x - x0) * view.frames_per_px;
                        self.view = WaveView { start, ..view }.clamped(frames, wave.w);
                        self.changed();
                        Some(EventKind::View {
                            start: self.view.start,
                            frames_per_px: self.view.frames_per_px,
                        })
                    }
                    _ => None,
                }
            }
            Touch::Up | Touch::Cancel => {
                self.fingers.retain(|(f, _)| *f != finger);
                if self.fingers.len() < 2 {
                    self.pinch = None;
                }
                let grab = self.grab.take_if(|(g, _)| *g == finger);
                match grab {
                    // A tap picks the frame where it went down.
                    Some((
                        _,
                        Grab::Wave {
                            x0, moved: false, ..
                        },
                    )) if phase == Touch::Up && wave.contains(x, y) => {
                        let f = self
                            .view
                            .frame_at(f64::from(x0 - wave.x))
                            .round()
                            .clamp(0.0, frames as f64) as u64;
                        Some(EventKind::Frame(f))
                    }
                    _ => None,
                }
            }
        }
    }
}

/// The whole sound in a strip, with a box round the part the waveform
/// shows. A touch moves the box there: [`EventKind::Frame`] with the frame
/// the view should centre on.
pub struct WaveOverview {
    peaks: Option<Arc<dyn Peaks>>,
    /// The frames the waveform shows.
    shown: (f64, f64),
    region: Option<(u64, u64)>,
}

impl Default for WaveOverview {
    fn default() -> Self {
        WaveOverview::new()
    }
}

impl WaveOverview {
    pub fn new() -> WaveOverview {
        WaveOverview {
            peaks: None,
            shown: (0.0, 0.0),
            region: None,
        }
    }

    pub fn set_peaks(&mut self, peaks: Option<Arc<dyn Peaks>>) {
        self.peaks = peaks;
    }

    /// The waveform's view, from its left edge to its right.
    pub fn set_shown(&mut self, view: WaveView, wave_width: i32) {
        self.shown = (view.start, view.end(wave_width));
    }

    pub fn set_region(&mut self, region: Option<(u64, u64)>) {
        self.region = region;
    }

    fn frames(&self) -> u64 {
        self.peaks.as_ref().map_or(0, |p| p.frames())
    }

    fn inner(r: Rect) -> Rect {
        r.inset_xy(6, 6)
    }
}

impl Widget for WaveOverview {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(r, t.radius, t.surface);
        let Some(p) = &self.peaks else { return };
        let inner = Self::inner(r);
        let frames = p.frames().max(1) as f64;
        let fpp = frames / f64::from(inner.w.max(1));
        let mid = inner.y + inner.h / 2;
        let half = (inner.h / 2 - 1).max(1) as f32;
        let (rs, re) = self
            .region
            .map_or((0.0, frames), |(a, b)| (a as f64, b as f64));
        let (on, off) = (t.text_dim, t.text_faint.mix(t.surface, 0.4));
        for x in 0..inner.w {
            let f0 = f64::from(x) * fpp;
            // Both channels in one lane: the widest of them.
            let mut c = WaveColumn {
                min: f32::MAX,
                max: f32::MIN,
                rms: 0.0,
            };
            for ch in 0..p.channels().clamp(1, 2) {
                let k = p.column(ch, f0, f0 + fpp);
                c.min = c.min.min(k.min);
                c.max = c.max.max(k.max);
            }
            if c.max < c.min {
                continue;
            }
            let top = mid - (c.max.clamp(-1.0, 1.0) * half).round() as i32;
            let bot = mid - (c.min.clamp(-1.0, 1.0) * half).round() as i32;
            let inside = f0 + fpp > rs && f0 < re;
            cx.canvas.fill_rect(
                Rect::new(inner.x + x, top, 1, (bot - top).max(1)),
                if inside { on } else { off },
            );
        }
        let xa = inner.x + (self.shown.0 / fpp).round() as i32;
        let xb = inner.x + (self.shown.1 / fpp).round() as i32;
        let b = Rect::new(
            xa.max(inner.x) - 2,
            r.y + 3,
            (xb.min(inner.right()) - xa.max(inner.x)).max(4) + 4,
            r.h - 6,
        );
        cx.canvas.fill_round_rect(b, 6, t.accent.with_alpha(40));
        cx.canvas.stroke_round_rect(b, 6, 2, t.accent);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if self.peaks.is_none() {
            return Vec::new();
        }
        // Zoomed all the way out the view has nowhere to move.
        let whole = self.shown.0 <= 0.0 && self.shown.1 >= self.frames() as f64;
        vec![Target::tap("overview", 0, "overview", Self::inner(r)).disabled_if(whole)]
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, _y: i32, r: Rect) -> Option<EventKind> {
        if !matches!(phase, Touch::Down | Touch::Move) || self.peaks.is_none() {
            return None;
        }
        let inner = Self::inner(r);
        let frames = self.frames();
        let f = (f64::from(x - inner.x) / f64::from(inner.w.max(1)) * frames as f64)
            .round()
            .clamp(0.0, frames as f64) as u64;
        let span = self.shown.1 - self.shown.0;
        self.shown = (f as f64 - span / 2.0, f as f64 + span / 2.0);
        Some(EventKind::Frame(f))
    }
}
