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

/// Where a marker's tab sits on its line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabSide {
    /// Centred on it.
    #[default]
    Centre,
    /// Ending at it, to its left.
    Before,
    /// Starting at it, to its right.
    After,
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
    /// Drawn faint: another tool's mark (the one Sample page draws every
    /// tool's marks, the tool's bright and the rest faint lines with no
    /// tab, [`Marker::line_only`]).
    pub faint: bool,
    /// Its colour, in place of its kind's.
    pub color: Option<Color>,
    /// Where its tab sits: a pair hugging a span from outside (the grain
    /// window's) keep clear of each other however close they come.
    pub side: TabSide,
    /// Has a tab. Without one it is only a line down the wave, which no
    /// finger takes hold of ([`Marker::line_only`]).
    pub tab: bool,
}

impl Marker {
    pub fn new(frame: u64, label: &str, kind: MarkerKind) -> Marker {
        Marker {
            frame,
            label: label.to_string(),
            kind,
            fixed: false,
            faint: false,
            color: None,
            side: TabSide::Centre,
            tab: true,
        }
    }

    /// The same marker, faint and staying put: another tool's.
    pub fn faint(mut self) -> Marker {
        self.faint = true;
        self.fixed = true;
        self
    }

    /// The same marker in colour `c`.
    pub fn colored(mut self, c: Color) -> Marker {
        self.color = Some(c);
        self
    }

    /// The same marker with its tab on side `side`.
    pub fn tab_on(mut self, side: TabSide) -> Marker {
        self.side = side;
        self
    }

    /// The same marker, staying put ([`Marker::fixed`]).
    pub fn fixed(mut self) -> Marker {
        self.fixed = true;
        self
    }

    /// The same marker as a line only, with no tab: another tool's mark,
    /// as the strip holds only the tabs of the tool picked.
    pub fn line_only(mut self) -> Marker {
        self.tab = false;
        self.fixed = true;
        self
    }
}

/// Follows the line through `points` (x, y), in order of x, from `x`:
/// between two points along the line joining them, before the first and
/// after the last one to one. No points: `x` itself.
pub fn bend_through(points: &[(f64, f64)], x: f64) -> f64 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return x;
    };
    if x <= first.0 {
        return first.1 + (x - first.0);
    }
    if x >= last.0 {
        return last.1 + (x - last.0);
    }
    let i = points
        .partition_point(|p| p.0 <= x)
        .clamp(1, points.len() - 1);
    let (a, b) = (points[i - 1], points[i]);
    if b.0 - a.0 <= 0.0 {
        a.1
    } else {
        a.1 + (x - a.0) * (b.1 - a.1) / (b.0 - a.0)
    }
}

/// A grain window over the wave ([`Waveform::set_window`]): where grains
/// start (`at`), how long each reads (`len` frames), and the band the
/// spray scatters their starts over round `at` (`spray` frames wide,
/// wrapping round the region `within`), in `color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainSpan {
    pub at: u64,
    pub len: u64,
    pub spray: u64,
    pub within: (u64, u64),
    pub color: Color,
}

/// Bar lines: the frame of the first downbeat, frames per beat, beats per
/// bar, and grid lines a beat (1: none between the beats), drawn faint once
/// zoomed in far enough to tell them apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beats {
    pub first: f64,
    pub frames_per_beat: f64,
    pub per_bar: u32,
    pub per_beat: u32,
}

/// One cell of the lane along the wave's foot ([`Waveform::set_lane`]):
/// frames `start..end` as drawn, what it says (the pad a slice lands on),
/// in `color`; `lit` drawn stronger (the pad plays it already), `picked`
/// outlined.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneCell {
    pub start: u64,
    pub end: u64,
    pub text: String,
    pub color: Color,
    pub lit: bool,
    pub picked: bool,
}

/// The strip of marker tabs along the top.
const TABS_H: i32 = 30;
/// The lane along the foot, with the gap over it.
const LANE_H: i32 = 30;
const LANE_GAP: i32 = 2;
/// A touch that moves less than this is a tap.
const TAP_SLOP: i32 = 8;
/// Width of the strip redrawn round a playhead.
const PLAYHEAD_W: i32 = 2;
/// A grain's mark's width ([`Waveform::set_grains`]).
const GRAIN_W: i32 = 4;

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
    window: Option<GrainSpan>,
    /// The grains playing now: each one's frame and its window's level
    /// (0 to 1), drawn over the wave in the window's colour
    /// ([`Waveform::set_grains`]).
    grains: Vec<(u64, f32)>,
    fades: (u64, u64),
    beats: Option<Beats>,
    /// The wave drawn as it plays ([`Waveform::set_bend`]).
    bend: Vec<(f64, f64)>,
    /// Hits found in the sound, ticks along the wave's foot, and whether
    /// they are lit.
    ticks: (Vec<u64>, bool),
    /// The lane under the wave (where each slice lands); none: no lane.
    lane: Vec<LaneCell>,
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
            window: None,
            grains: Vec::new(),
            fades: (0, 0),
            beats: None,
            bend: Vec::new(),
            ticks: (Vec::new(), false),
            lane: Vec::new(),
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

    /// The frames drawn bright ([`Waveform::set_region`]).
    pub fn region(&self) -> Option<(u64, u64)> {
        self.region
    }

    /// Where the playheads are ([`Waveform::set_playheads`]).
    pub fn playheads(&self) -> &[u64] {
        &self.playheads
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

    /// The grain window shaded, if one is.
    pub fn window(&self) -> Option<GrainSpan> {
        self.window
    }

    /// Shades a grain window (None: none).
    pub fn set_window(&mut self, window: Option<GrainSpan>) {
        if self.window != window {
            self.window = window;
            self.changed();
        }
    }

    /// The grains playing now, each at its frame with its window's level
    /// (0 to 1): a mark over the wave for each, taller and brighter as its
    /// window opens. Empty: none.
    pub fn set_grains(&mut self, grains: Vec<(u64, f32)>) {
        if self.grains != grains {
            self.grains = grains;
            self.changed();
        }
    }

    /// The grains drawn ([`Waveform::set_grains`]).
    pub fn grains(&self) -> &[(u64, f32)] {
        &self.grains
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

    /// The wave drawn as it plays: `points` are (frame as drawn, source
    /// frame) in order, and between two the sound is drawn stretched to
    /// fit (a warped region pinned to the beat, so its hits sit on the bar
    /// lines). Before the first and after the last, one to one. Empty: the
    /// sound as it is. Every other frame the widget is given or gives back
    /// (markers, the region, the loop, playheads, taps) is a frame as
    /// drawn.
    pub fn set_bend(&mut self, points: Vec<(f64, f64)>) {
        if self.bend != points {
            self.bend = points;
            self.changed();
        }
    }

    pub fn bend(&self) -> &[(f64, f64)] {
        &self.bend
    }

    /// The hits found in the sound (frames as drawn), as ticks along the
    /// wave's foot; `lit` with the tool that pins them.
    pub fn set_ticks(&mut self, ticks: Vec<u64>, lit: bool) {
        if self.ticks.0 != ticks || self.ticks.1 != lit {
            self.ticks = (ticks, lit);
            self.changed();
        }
    }

    pub fn ticks(&self) -> &[u64] {
        &self.ticks.0
    }

    /// A lane along the wave's foot, under it, of cells over the frames
    /// they name (the Sample page's Slice tool: where each slice lands);
    /// empty: no lane, and the wave fills the room. A tap on the lane is a
    /// tap on the wave above it.
    pub fn set_lane(&mut self, cells: Vec<LaneCell>) {
        if self.lane != cells {
            self.lane = cells;
            self.changed();
        }
    }

    pub fn lane(&self) -> &[LaneCell] {
        &self.lane
    }

    /// The part of `r` the wave is drawn in: [`Waveform::wave_rect`], less
    /// the lane along its foot when there is one.
    fn area(&self, r: Rect) -> Rect {
        let w = Self::wave_rect(r);
        if self.lane.is_empty() {
            w
        } else {
            w.take_bottom(LANE_H).1
        }
    }

    /// The lane's strip under the wave in `r`.
    fn lane_rect(&self, r: Rect) -> Rect {
        let w = Self::wave_rect(r);
        let lane = w.take_bottom(LANE_H).0;
        Rect::new(lane.x, lane.y + LANE_GAP, lane.w, lane.h - LANE_GAP)
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

    /// The tab of marker `i`, `w` pixels wide, on its side of the marker
    /// ([`Marker::side`]) and kept inside the strip; None when a slice or
    /// cue is off screen (start, end and loop tabs stay pinned to the
    /// edge, to show which way they are).
    fn tab_rect(&self, r: Rect, i: usize, w: i32) -> Option<Rect> {
        let m = &self.markers[i];
        if !m.tab {
            return None;
        }
        let wave = Self::wave_rect(r);
        let x = self.x_in(wave, m.frame as f64).round() as i32;
        let pinned = matches!(m.kind, MarkerKind::Edge | MarkerKind::Loop);
        if !pinned && (x < wave.x || x > wave.right()) {
            return None;
        }
        // The line is two pixels wide, x - 1 to x + 1.
        let left = match m.side {
            TabSide::Centre => x - w / 2,
            TabSide::Before => x + 1 - w,
            TabSide::After => x - 1,
        };
        let left = left.clamp(r.x + 4, (r.right() - 4 - w).max(r.x + 4));
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
        let wave = self.area(r);
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
        if let Some(w) = self.window {
            let shade = |cx: &mut PaintCx, a: u64, b: u64, alpha: u8| {
                let (xa, xb) = (
                    (self.x_in(wave, a as f64).round() as i32).max(wave.x),
                    (self.x_in(wave, b as f64).round() as i32).min(wave.right()),
                );
                if xb > xa {
                    cx.canvas.fill_rect(
                        Rect::new(xa, wave.y, xb - xa, wave.h),
                        w.color.with_alpha(alpha),
                    );
                }
            };
            // The spray's band round the start, wrapping round the region.
            let (lo, hi) = w.within;
            let span = hi.saturating_sub(lo).max(1);
            let half = w.spray.min(span) / 2;
            if w.spray >= span {
                shade(cx, lo, hi, 26);
            } else if half > 0 {
                let (a, b) = (w.at as i64 - half as i64, w.at + half);
                shade(cx, a.max(lo as i64) as u64, b.min(hi), 26);
                if a < lo as i64 {
                    shade(cx, (hi as i64 + a - lo as i64) as u64, hi, 26);
                }
                if b > hi {
                    shade(cx, lo, lo + (b - hi), 26);
                }
            }
            // One grain, from the start.
            shade(cx, w.at, w.at + w.len.max(1), 60);
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
            // The grid between the beats, under them.
            let sub = b.per_beat.max(1);
            if sub > 1 && px_per_beat / f64::from(sub) >= 12.0 && (last - first) <= 400 {
                let faint = t.outline.mix(t.surface, 0.5);
                for beat in first..last {
                    for k in 1..sub {
                        let f = b.first
                            + (beat as f64 + f64::from(k) / f64::from(sub)) * b.frames_per_beat;
                        let x = self.x_in(wave, f).round() as i32;
                        if x >= wave.x && x < wave.right() {
                            cx.canvas.fill_rect(Rect::new(x, wave.y, 1, wave.h), faint);
                        }
                    }
                }
            }
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
                    // As drawn; the sound's own frames through the bend.
                    let (s0, s1) = (
                        bend_through(&self.bend, f0),
                        bend_through(&self.bend, f0 + fpp),
                    );
                    let c = p.column(ch, s0, s1.max(s0));
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

        // The hits: ticks along the wave's foot.
        let tick = if self.ticks.1 {
            t.text_dim
        } else {
            t.text_faint
        };
        for &f in &self.ticks.0 {
            let x = self.x_in(wave, f as f64).round() as i32;
            if x >= x0 && x < x1 {
                cx.canvas
                    .fill_rect(Rect::new(x - 1, wave.bottom() - 8, 2, 8), tick);
            }
        }

        // The grains playing: a mark round the wave's middle for each, as
        // tall and as bright as its window is open.
        if !self.grains.is_empty() {
            let c = self.window.map_or(t.warn, |w| w.color);
            let mid = wave.y + wave.h / 2;
            for &(f, level) in &self.grains {
                let x = self.x_in(wave, f as f64).round() as i32;
                if x < x0 || x + GRAIN_W > x1 {
                    continue;
                }
                let level = level.clamp(0.0, 1.0);
                let h = (6.0 + (wave.h - 12) as f32 * 0.8 * level).round() as i32;
                let alpha = (90.0 + 165.0 * level).round() as u8;
                cx.canvas.fill_round_rect(
                    Rect::new(x - GRAIN_W / 2, mid - h / 2, GRAIN_W, h),
                    GRAIN_W / 2,
                    c.with_alpha(alpha),
                );
            }
        }

        // Markers: a line down the wave and a tab in the strip.
        let mut tabs = Vec::with_capacity(self.markers.len());
        for i in 0..self.markers.len() {
            let m = &self.markers[i];
            let c = m.color.unwrap_or_else(|| marker_color(&t, m.kind));
            let c = if m.faint { c.mix(t.surface, 0.62) } else { c };
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

        // The lane under the wave: each cell over its frames, its text
        // where it fits.
        if !self.lane.is_empty() {
            let lane = self.lane_rect(r);
            for c in &self.lane {
                let xa = (self.x_in(wave, c.start as f64).round() as i32).max(lane.x);
                let xb = (self.x_in(wave, c.end as f64).round() as i32).min(lane.right());
                if xb - xa < 4 || xb < clip.x || xa > clip.right() {
                    continue;
                }
                let cell = Rect::new(xa + 1, lane.y, xb - xa - 2, lane.h);
                let fill = c.color.with_alpha(if c.lit { 84 } else { 46 });
                cx.canvas.fill_round_rect(cell, 6, fill);
                if c.picked {
                    cx.canvas.stroke_round_rect(cell, 6, 1, c.color);
                }
                if cx.measure(FontId::Bold, 13.0, &c.text) + 6 <= cell.w {
                    cx.text(FontId::Bold, 13.0, cell, Align::Center, c.color, &c.text);
                }
            }
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
        let wave = self.area(r);
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

    /// A finger resting on a marker's tab puts the marker under the wheel
    /// (Jay, 17 Nov), with the same room round the tab as a drag has.
    fn held(&self, x: i32, y: i32, r: Rect) -> Option<Target> {
        self.peaks.as_ref()?;
        if y >= Self::wave_rect(r).y {
            return None;
        }
        let painted = self.painted.borrow();
        let tabs = &painted.tabs;
        let i = (0..self.markers.len().min(tabs.len()))
            .rev()
            .find(|&i| tabs[i].is_some_and(|t| t.inset_xy(-6, -3).contains(x, y)))?;
        let tab = tabs[i]?;
        (!self.markers[i].fixed)
            .then(|| Target::drag("marker", i, self.markers[i].label.clone(), tab))
    }

    /// The wheel nudges the held marker one screen pixel a detent at the
    /// zoom showing (at least a frame), one frame with SHIFT, and says so
    /// as a drag would.
    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, _r: Rect) -> Option<EventKind> {
        let m = self.markers.get(t.index).filter(|m| !m.fixed)?;
        let each = if fine {
            1.0
        } else {
            self.view.frames_per_px.round().max(1.0)
        };
        let to = (m.frame as f64 + f64::from(delta) * each)
            .round()
            .clamp(0.0, self.frames() as f64) as u64;
        if to == m.frame {
            return None;
        }
        self.set_marker_frame(t.index, to);
        Some(EventKind::Marker {
            index: t.index,
            frame: to,
        })
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
    /// A few words in its top right corner ("1.0× · start"): what the
    /// zoom is, where the page has no card for it.
    label: String,
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
            label: String::new(),
        }
    }

    /// The words in its top right corner (empty: none).
    pub fn set_label(&mut self, label: &str) {
        if self.label != label {
            self.label = label.to_string();
        }
    }

    pub fn label(&self) -> &str {
        &self.label
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
        if !self.label.is_empty() {
            let w = cx.measure(FontId::Sans, 11.0, &self.label) + 12;
            let tag = Rect::new(r.right() - w - 4, r.y + 4, w, 16);
            cx.canvas.fill_round_rect(tag, 6, t.surface.with_alpha(220));
            cx.text(
                FontId::Sans,
                11.0,
                tag,
                Align::Center,
                t.text_dim,
                &self.label,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if self.peaks.is_none() {
            return Vec::new();
        }
        // Zoomed all the way out the view has nowhere to move (to within
        // half a frame: the view's end is worked out in floating point, and
        // some widths land a hair short).
        let whole = self.shown.0 <= 0.5 && self.shown.1 >= self.frames() as f64 - 0.5;
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
