//! M17: touch tuning from measurements.
//!
//! Three parts, all toolkit-neutral and deterministic under test:
//!
//! - [`Tuning`]: the values that adjust touch for the whole app. A position
//!   correction (scale about the screen's centre, then an offset, per axis),
//!   a hold filter (a held finger's moves smaller than this are dropped, so a
//!   still finger stays still), and the gesture thresholds
//!   ([`GestureConfig`]).
//! - [`Filter`]: applies a tuning's correction and hold filter to every
//!   finger, before anything on screen sees it.
//! - [`Measure`]: watches fingers *before* the filter (so a tuning never
//!   hides what it is meant to fix) and keeps what they did: taps, holds,
//!   strokes, report rates and target taps. [`suggest`] turns that into a
//!   tuning, changing only what has enough samples behind it.
//!
//! Every value is bounded ([`Param::min`], [`Param::max`]), so no tuning can
//! move a touch more than [`MAX_OFFSET_PX`] plus a few percent of the screen:
//! the page that changes these stays reachable whatever is set.

use crate::gesture::GestureConfig;
use crate::jitter::{hold_stats, HoldStats};
use crate::mt::{ContactId, Phase};
use crate::pointer::PointerEvent;
use std::collections::VecDeque;

/// The largest offset the correction may add, logical px.
pub const MAX_OFFSET_PX: f32 = 40.0;
/// Scale bounds of the correction (1.0 = none).
pub const MIN_SCALE: f32 = 0.9;
pub const MAX_SCALE: f32 = 1.1;

/// A contact that moved no further than this from where it landed is a tap
/// or a hold; further than [`STROKE_PX`] it is a stroke. In between it is
/// neither and is not counted.
pub const STILL_PX: f32 = 25.0;
pub const STROKE_PX: f32 = 40.0;
/// A still contact held at least this long is a hold, not a tap.
pub const HOLD_US: u64 = 400_000;
/// A hold's first reports are the finger landing, not jitter.
pub const SETTLE_US: u64 = 100_000;

/// How many of each sample [`Measure`] keeps (the newest).
const KEEP_TAPS: usize = 60;
const KEEP_HOLDS: usize = 20;
const KEEP_STROKES: usize = 40;
const KEEP_RATES: usize = 40;
/// Reports kept per live contact.
const KEEP_PATH: usize = 600;

/// Samples a suggestion needs before it changes a value.
pub const NEED_TAPS: usize = 8;
pub const NEED_HOLDS: usize = 3;
pub const NEED_SWIPES: usize = 4;
pub const NEED_TARGETS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub hold_filter_px: f32,
    pub gestures: GestureConfig,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            offset_x: 0.0,
            offset_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            hold_filter_px: 0.0,
            gestures: GestureConfig::default(),
        }
    }
}

/// One adjustable value of a [`Tuning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    OffsetX,
    OffsetY,
    ScaleX,
    ScaleY,
    HoldFilter,
    TapSlop,
    LongestTap,
    LongPress,
    SwipeDistance,
    SwipeSpeed,
}

impl Param {
    pub const ALL: [Param; 10] = [
        Param::OffsetX,
        Param::OffsetY,
        Param::ScaleX,
        Param::ScaleY,
        Param::HoldFilter,
        Param::TapSlop,
        Param::LongestTap,
        Param::LongPress,
        Param::SwipeDistance,
        Param::SwipeSpeed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Param::OffsetX => "Shift across",
            Param::OffsetY => "Shift down",
            Param::ScaleX => "Stretch across",
            Param::ScaleY => "Stretch down",
            Param::HoldFilter => "Hold filter",
            Param::TapSlop => "Tap slop",
            Param::LongestTap => "Longest tap",
            Param::LongPress => "Long press",
            Param::SwipeDistance => "Swipe distance",
            Param::SwipeSpeed => "Swipe speed",
        }
    }

    /// The settings key (store keys are `[a-z0-9._-]`).
    pub fn key(self) -> &'static str {
        match self {
            Param::OffsetX => "offset-x",
            Param::OffsetY => "offset-y",
            Param::ScaleX => "scale-x",
            Param::ScaleY => "scale-y",
            Param::HoldFilter => "hold-filter-px",
            Param::TapSlop => "tap-slop-px",
            Param::LongestTap => "tap-max-ms",
            Param::LongPress => "long-press-ms",
            Param::SwipeDistance => "swipe-min-px",
            Param::SwipeSpeed => "swipe-min-speed",
        }
    }

    /// One press of + or -.
    pub fn step(self) -> f32 {
        match self {
            Param::OffsetX | Param::OffsetY | Param::HoldFilter | Param::TapSlop => 1.0,
            Param::ScaleX | Param::ScaleY => 0.005,
            Param::LongestTap | Param::LongPress => 25.0,
            Param::SwipeDistance => 10.0,
            Param::SwipeSpeed => 50.0,
        }
    }

    pub fn min(self) -> f32 {
        match self {
            Param::OffsetX | Param::OffsetY => -MAX_OFFSET_PX,
            Param::ScaleX | Param::ScaleY => MIN_SCALE,
            Param::HoldFilter => 0.0,
            Param::TapSlop => 4.0,
            Param::LongestTap => 100.0,
            Param::LongPress => 250.0,
            Param::SwipeDistance => 30.0,
            Param::SwipeSpeed => 150.0,
        }
    }

    pub fn max(self) -> f32 {
        match self {
            Param::OffsetX | Param::OffsetY => MAX_OFFSET_PX,
            Param::ScaleX | Param::ScaleY => MAX_SCALE,
            Param::HoldFilter => 6.0,
            Param::TapSlop => 40.0,
            Param::LongestTap => 1000.0,
            Param::LongPress => 1500.0,
            Param::SwipeDistance => 400.0,
            Param::SwipeSpeed => 3000.0,
        }
    }

    /// The value as the page shows it, with its unit.
    pub fn show(self, v: f32) -> String {
        match self {
            Param::OffsetX | Param::OffsetY => format!("{v:+.0} px"),
            Param::ScaleX | Param::ScaleY => format!("{:.1} %", v * 100.0),
            Param::HoldFilter | Param::TapSlop | Param::SwipeDistance => format!("{v:.0} px"),
            Param::LongestTap | Param::LongPress => format!("{v:.0} ms"),
            Param::SwipeSpeed => format!("{v:.0} px/s"),
        }
    }

    /// What the value does, for the page's help line.
    pub fn help(self) -> &'static str {
        match self {
            Param::OffsetX => "Moves every touch right (+) or left (-). Applies to the whole app.",
            Param::OffsetY => "Moves every touch down (+) or up (-). Applies to the whole app.",
            Param::ScaleX => {
                "Spreads touches out from the middle (over 100 %) or in. Applies to the whole app."
            }
            Param::ScaleY => {
                "Spreads touches out from the middle (over 100 %) or in. Applies to the whole app."
            }
            Param::HoldFilter => {
                "A held finger's moves smaller than this are dropped. Applies to the whole app."
            }
            Param::TapSlop => "How far a finger may move and still tap.",
            Param::LongestTap => "A press longer than this is not a tap.",
            Param::LongPress => "Holding still this long is a long press.",
            Param::SwipeDistance => "A swipe travels at least this far.",
            Param::SwipeSpeed => "A swipe is moving at least this fast when it lifts.",
        }
    }
}

impl Tuning {
    pub fn get(&self, p: Param) -> f32 {
        let g = &self.gestures;
        match p {
            Param::OffsetX => self.offset_x,
            Param::OffsetY => self.offset_y,
            Param::ScaleX => self.scale_x,
            Param::ScaleY => self.scale_y,
            Param::HoldFilter => self.hold_filter_px,
            Param::TapSlop => g.slop_px,
            Param::LongestTap => g.tap_max_us as f32 / 1000.0,
            Param::LongPress => g.long_press_us as f32 / 1000.0,
            Param::SwipeDistance => g.swipe_min_px,
            Param::SwipeSpeed => g.swipe_min_speed,
        }
    }

    /// Sets a value, kept within its bounds (a non-finite value is ignored).
    pub fn set(&mut self, p: Param, v: f32) {
        if !v.is_finite() {
            return;
        }
        let v = v.clamp(p.min(), p.max());
        let g = &mut self.gestures;
        match p {
            Param::OffsetX => self.offset_x = v,
            Param::OffsetY => self.offset_y = v,
            Param::ScaleX => self.scale_x = v,
            Param::ScaleY => self.scale_y = v,
            Param::HoldFilter => self.hold_filter_px = v,
            Param::TapSlop => g.slop_px = v,
            Param::LongestTap => g.tap_max_us = (v * 1000.0).round() as u64,
            Param::LongPress => g.long_press_us = (v * 1000.0).round() as u64,
            Param::SwipeDistance => g.swipe_min_px = v,
            Param::SwipeSpeed => g.swipe_min_speed = v,
        }
    }

    /// One press of + (1) or - (-1), rounded to the step.
    pub fn step(&mut self, p: Param, d: i32) {
        let s = p.step();
        let v = ((self.get(p) / s).round() + d as f32) * s;
        self.set(p, v);
    }

    /// True when the correction or the hold filter changes positions.
    pub fn moves_touches(&self) -> bool {
        self.offset_x != 0.0
            || self.offset_y != 0.0
            || self.scale_x != 1.0
            || self.scale_y != 1.0
            || self.hold_filter_px > 0.0
    }

    /// Every value as `key = value` pairs, for a settings file.
    pub fn to_pairs(&self) -> Vec<(&'static str, String)> {
        Param::ALL
            .iter()
            .map(|&p| {
                let v = self.get(p);
                let text = match p {
                    Param::ScaleX | Param::ScaleY => format!("{v:.4}"),
                    _ => format!("{v:.1}"),
                };
                (p.key(), text)
            })
            .collect()
    }

    /// Reads values by key; missing or unreadable keys keep the default, and
    /// each value is kept within its bounds.
    pub fn from_lookup(get: impl Fn(&str) -> Option<f64>) -> Tuning {
        let mut t = Tuning::default();
        for p in Param::ALL {
            if let Some(v) = get(p.key()) {
                t.set(p, v as f32);
            }
        }
        // A long press must outlast the longest tap.
        if t.gestures.long_press_us < t.gestures.tap_max_us {
            t.gestures.long_press_us = t.gestures.tap_max_us;
        }
        t
    }

    /// One journal line with every value.
    pub fn line(&self) -> String {
        let parts: Vec<String> = self
            .to_pairs()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        parts.join(" ")
    }

    /// Where a position goes under this tuning's correction, on a screen of
    /// `size` logical px (the scale is about the screen's centre).
    pub fn correct(&self, size: (f32, f32), x: f32, y: f32) -> (f32, f32) {
        (
            axis(x, size.0, self.scale_x, self.offset_x),
            axis(y, size.1, self.scale_y, self.offset_y),
        )
    }
}

/// One axis of the correction; an axis with none is passed on exactly.
fn axis(v: f32, len: f32, scale: f32, offset: f32) -> f32 {
    if scale == 1.0 && offset == 0.0 {
        return v;
    }
    let c = len / 2.0;
    (c + (v - c) * scale + offset).clamp(0.0, (len - 1.0).max(0.0))
}

// ---------------------------------------------------------------- the filter

/// Applies a [`Tuning`]'s correction and hold filter to fingers.
#[derive(Debug, Clone)]
pub struct Filter {
    tuning: Tuning,
    size: (f32, f32),
    /// Each finger down and the position last passed on.
    last: Vec<(u64, (f32, f32))>,
}

impl Filter {
    pub fn new(size: (f32, f32)) -> Filter {
        Filter {
            tuning: Tuning::default(),
            size,
            last: Vec::new(),
        }
    }

    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    /// Takes effect from the next event (fingers down keep their place).
    pub fn set_tuning(&mut self, t: Tuning) {
        self.tuning = t;
    }

    /// Where a finger event goes, or None for a move the hold filter drops.
    /// An Up or Cancel always passes, at the last place passed on if the
    /// finger moved less than the filter since.
    pub fn apply(&mut self, finger: u64, phase: Phase, x: f32, y: f32) -> Option<(f32, f32)> {
        let p = self.tuning.correct(self.size, x, y);
        let f = self.tuning.hold_filter_px;
        match phase {
            Phase::Down => {
                self.last.retain(|(id, _)| *id != finger);
                self.last.push((finger, p));
                Some(p)
            }
            Phase::Move => match self.last.iter_mut().find(|(id, _)| *id == finger) {
                Some((_, at)) => {
                    if f > 0.0 && dist(*at, p) < f {
                        None
                    } else {
                        *at = p;
                        Some(p)
                    }
                }
                None => {
                    self.last.push((finger, p));
                    Some(p)
                }
            },
            Phase::Up | Phase::Cancel => {
                let i = self.last.iter().position(|(id, _)| *id == finger);
                let at = i.map(|i| self.last.remove(i).1);
                match at {
                    Some(at) if f > 0.0 && dist(at, p) < f => Some(at),
                    _ => Some(p),
                }
            }
        }
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

// ---------------------------------------------------------------- measuring

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TapSample {
    pub duration_us: u64,
    /// Furthest from where it landed, logical px.
    pub travel_px: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    /// Straight distance from landing to lift, logical px.
    pub distance_px: f32,
    /// Speed over the last [`GestureConfig::velocity_window_us`] before the
    /// lift, px/s (0 if the finger paused longer than that).
    pub speed: f32,
    /// The main axis over the other, in distance (large = straight).
    pub axis_ratio: f32,
    /// Made while the page asked for swipes.
    pub asked_swipe: bool,
}

/// What a lifted contact turned out to be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sample {
    Tap(TapSample),
    Hold(HoldStats),
    Stroke(Stroke),
    /// Moved between [`STILL_PX`] and [`STROKE_PX`], or was cancelled.
    Other,
}

impl Sample {
    /// The `touch-measure:` journal line for it.
    pub fn line(&self) -> String {
        match self {
            Sample::Tap(t) => format!(
                "touch-measure: tap ms={:.0} travel_px={:.1}",
                t.duration_us as f32 / 1000.0,
                t.travel_px
            ),
            Sample::Hold(h) => format!(
                "touch-measure: hold ms={:.0} reports={} rate_hz={:.0} std_px={:.2},{:.2} max_dev_px={:.2}",
                h.duration_us as f32 / 1000.0,
                h.samples,
                h.rate_hz,
                h.std_x,
                h.std_y,
                h.max_dev
            ),
            Sample::Stroke(s) => format!(
                "touch-measure: stroke px={:.0} speed={:.0} ratio={:.1} asked={}",
                s.distance_px,
                s.speed,
                s.axis_ratio,
                if s.asked_swipe { "swipe" } else { "free" }
            ),
            Sample::Other => "touch-measure: other".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct Live {
    finger: u64,
    path: Vec<PointerEvent>,
    travel: f32,
}

/// What fingers did, measured before any tuning.
#[derive(Debug, Clone, Default)]
pub struct Measure {
    live: Vec<Live>,
    pub taps: VecDeque<TapSample>,
    pub holds: VecDeque<HoldStats>,
    pub strokes: VecDeque<Stroke>,
    /// Reports per second of moving contacts.
    pub rates: VecDeque<f32>,
    /// Target taps: (where the target is, where the finger landed), logical
    /// px before any correction.
    pub targets: Vec<((f32, f32), (f32, f32))>,
    /// Strokes made now count as swipes asked for.
    pub asking_swipes: bool,
    velocity_window_us: u64,
}

impl Measure {
    pub fn new() -> Measure {
        Measure {
            velocity_window_us: GestureConfig::default().velocity_window_us,
            ..Measure::default()
        }
    }

    /// Forget every sample (fingers down are kept).
    pub fn clear(&mut self) {
        self.taps.clear();
        self.holds.clear();
        self.strokes.clear();
        self.rates.clear();
        self.targets.clear();
    }

    pub fn fingers_down(&self) -> usize {
        self.live.len()
    }

    /// One finger event, in logical px before any tuning, with the kernel's
    /// time. On a lift: what the contact was.
    pub fn event(
        &mut self,
        finger: u64,
        phase: Phase,
        x: f32,
        y: f32,
        time_us: u64,
    ) -> Option<Sample> {
        let ev = PointerEvent {
            id: ContactId(finger),
            phase,
            x,
            y,
            time_us,
        };
        match phase {
            Phase::Down => {
                self.live.retain(|l| l.finger != finger);
                self.live.push(Live {
                    finger,
                    path: vec![ev],
                    travel: 0.0,
                });
                None
            }
            Phase::Move => {
                let l = self.live.iter_mut().find(|l| l.finger == finger)?;
                let d = (x - l.path[0].x).hypot(y - l.path[0].y);
                l.travel = l.travel.max(d);
                if l.path.len() < KEEP_PATH {
                    l.path.push(ev);
                }
                None
            }
            Phase::Up | Phase::Cancel => {
                let i = self.live.iter().position(|l| l.finger == finger)?;
                let mut l = self.live.remove(i);
                if phase == Phase::Cancel {
                    return Some(Sample::Other);
                }
                let d = (x - l.path[0].x).hypot(y - l.path[0].y);
                l.travel = l.travel.max(d);
                let s = self.classify(&l, ev);
                match s {
                    Sample::Tap(t) => push(&mut self.taps, t, KEEP_TAPS),
                    Sample::Hold(h) => push(&mut self.holds, h, KEEP_HOLDS),
                    Sample::Stroke(st) => push(&mut self.strokes, st, KEEP_STROKES),
                    Sample::Other => {}
                }
                Some(s)
            }
        }
    }

    fn classify(&mut self, l: &Live, up: PointerEvent) -> Sample {
        let down = l.path[0];
        let duration = up.time_us.saturating_sub(down.time_us);
        // Report rate while moving: moves over their span.
        let moves: Vec<&PointerEvent> = l.path.iter().skip(1).collect();
        if moves.len() >= 5 {
            let span = moves[moves.len() - 1]
                .time_us
                .saturating_sub(moves[0].time_us);
            if span > 0 && l.travel > STILL_PX {
                let hz = (moves.len() - 1) as f32 / (span as f32 / 1e6);
                push(&mut self.rates, hz, KEEP_RATES);
            }
        }
        if l.travel <= STILL_PX {
            if duration < HOLD_US {
                return Sample::Tap(TapSample {
                    duration_us: duration,
                    travel_px: l.travel,
                });
            }
            // A hold: its reports after the finger settled.
            let settled: Vec<PointerEvent> = l
                .path
                .iter()
                .copied()
                .filter(|e| e.time_us >= down.time_us + SETTLE_US)
                .collect();
            let reports = if settled.len() >= 2 {
                settled
            } else {
                l.path.clone()
            };
            return match hold_stats(&reports) {
                Some(h) => Sample::Hold(HoldStats {
                    duration_us: duration,
                    ..h
                }),
                // A held finger the panel never reported again: no wander.
                None => Sample::Hold(HoldStats {
                    samples: reports.len(),
                    duration_us: duration,
                    rate_hz: 0.0,
                    std_x: 0.0,
                    std_y: 0.0,
                    max_dev: 0.0,
                }),
            };
        }
        if l.travel < STROKE_PX {
            return Sample::Other;
        }
        let (dx, dy) = (up.x - down.x, up.y - down.y);
        let (big, small) = (dx.abs().max(dy.abs()), dx.abs().min(dy.abs()));
        Sample::Stroke(Stroke {
            distance_px: dx.hypot(dy),
            speed: self.release_speed(&l.path, up),
            axis_ratio: if small > 0.0 {
                big / small
            } else {
                f32::INFINITY
            },
            asked_swipe: self.asking_swipes,
        })
    }

    /// Speed over the window before the lift, from the oldest report in it.
    fn release_speed(&self, path: &[PointerEvent], up: PointerEvent) -> f32 {
        let from = up.time_us.saturating_sub(self.velocity_window_us);
        // The lift's own position is the last move's (the panel sends no new
        // position with a lift), so the newest report is the end point.
        let end = path.last().copied().unwrap_or(up);
        if up.time_us.saturating_sub(end.time_us) > self.velocity_window_us {
            return 0.0;
        }
        let Some(start) = path.iter().find(|e| e.time_us >= from) else {
            return 0.0;
        };
        let dt = end.time_us.saturating_sub(start.time_us);
        if dt == 0 {
            return 0.0;
        }
        (end.x - start.x).hypot(end.y - start.y) / (dt as f32 / 1e6)
    }

    /// A tap on a target at `target` that landed at `at`.
    pub fn target(&mut self, target: (f32, f32), at: (f32, f32)) {
        self.targets.push((target, at));
    }

    pub fn swipes(&self) -> impl Iterator<Item = &Stroke> {
        self.strokes.iter().filter(|s| s.asked_swipe)
    }

    /// Target misses: (mean, largest) distance from the target, logical px,
    /// under `tuning`'s correction on a screen of `size`.
    pub fn target_error(&self, tuning: &Tuning, size: (f32, f32)) -> Option<(f32, f32)> {
        if self.targets.is_empty() {
            return None;
        }
        let errs: Vec<f32> = self
            .targets
            .iter()
            .map(|&(t, at)| dist(t, tuning.correct(size, at.0, at.1)))
            .collect();
        let mean = errs.iter().sum::<f32>() / errs.len() as f32;
        Some((mean, errs.iter().copied().fold(0.0, f32::max)))
    }
}

fn push<T>(q: &mut VecDeque<T>, v: T, keep: usize) {
    if q.len() >= keep {
        q.pop_front();
    }
    q.push_back(v);
}

/// The `q`th quantile (0 to 1) of some values, nearest rank.
pub fn quantile(values: impl Iterator<Item = f32>, q: f32) -> Option<f32> {
    let mut v: Vec<f32> = values.filter(|v| v.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let i = ((q.clamp(0.0, 1.0) * v.len() as f32).ceil() as usize).clamp(1, v.len()) - 1;
    Some(v[i])
}

fn round_to(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

/// A least-squares line `t - c = s * (m - c) + o` through (m, t) pairs, as
/// (s, o); None without two distinct `m`.
fn fit_axis(pairs: &[(f32, f32)], c: f32) -> Option<(f32, f32)> {
    let n = pairs.len() as f32;
    if pairs.len() < 2 {
        return None;
    }
    let mx = pairs.iter().map(|p| p.0 - c).sum::<f32>() / n;
    let my = pairs.iter().map(|p| p.1 - c).sum::<f32>() / n;
    let sxx: f32 = pairs.iter().map(|p| (p.0 - c - mx).powi(2)).sum();
    let sxy: f32 = pairs.iter().map(|p| (p.0 - c - mx) * (p.1 - c - my)).sum();
    if sxx < 1.0 {
        return None;
    }
    let s = sxy / sxx;
    Some((s, my - s * mx))
}

/// Why a value was or was not suggested.
#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    pub param: Param,
    pub text: String,
}

/// A tuning suggested by measurements: `current` with each value that has
/// enough samples changed, and a reason for each value.
pub fn suggest(current: &Tuning, m: &Measure, size: (f32, f32)) -> (Tuning, Vec<Reason>) {
    let mut t = *current;
    let mut why = Vec::new();
    let mut say = |p: Param, text: String| why.push(Reason { param: p, text });

    // Position: a line per axis through the target taps.
    if m.targets.len() >= NEED_TARGETS {
        let xs: Vec<(f32, f32)> = m.targets.iter().map(|&(tg, at)| (at.0, tg.0)).collect();
        let ys: Vec<(f32, f32)> = m.targets.iter().map(|&(tg, at)| (at.1, tg.1)).collect();
        for (pairs, c, sp, op) in [
            (&xs, size.0 / 2.0, Param::ScaleX, Param::OffsetX),
            (&ys, size.1 / 2.0, Param::ScaleY, Param::OffsetY),
        ] {
            match fit_axis(pairs, c) {
                Some((s, o)) => {
                    t.set(sp, round_to(s, 0.005));
                    t.set(op, o.round());
                    say(sp, format!("fitted to {} target taps", m.targets.len()));
                    say(op, format!("fitted to {} target taps", m.targets.len()));
                }
                None => say(op, "the target taps do not spread across the screen".into()),
            }
        }
    } else {
        let left = NEED_TARGETS - m.targets.len();
        for p in [Param::OffsetX, Param::OffsetY, Param::ScaleX, Param::ScaleY] {
            say(p, format!("needs {left} more target taps"));
        }
    }

    // Hold filter: what most held fingers wander.
    let hold_dev = quantile(m.holds.iter().map(|h| h.max_dev), 0.9);
    match hold_dev {
        Some(d) if m.holds.len() >= NEED_HOLDS => {
            let f = if d < 1.0 { 0.0 } else { d.ceil() };
            t.set(Param::HoldFilter, f);
            say(
                Param::HoldFilter,
                format!(
                    "held fingers wander up to {d:.1} px (9 in 10 of {} holds)",
                    m.holds.len()
                ),
            );
        }
        _ => say(
            Param::HoldFilter,
            format!(
                "needs {} more holds",
                NEED_HOLDS - m.holds.len().min(NEED_HOLDS)
            ),
        ),
    }

    if m.taps.len() >= NEED_TAPS {
        let travel = quantile(m.taps.iter().map(|s| s.travel_px), 0.95).unwrap_or(0.0);
        let wander = hold_dev.unwrap_or(0.0);
        let slop = (travel * 1.5 + 2.0).max(wander * 2.0 + 2.0).ceil();
        t.set(Param::TapSlop, slop);
        say(
            Param::TapSlop,
            format!(
                "taps move up to {travel:.1} px (19 in 20 of {}), plus room",
                m.taps.len()
            ),
        );
        let ms =
            quantile(m.taps.iter().map(|s| s.duration_us as f32 / 1000.0), 0.95).unwrap_or(0.0);
        t.set(Param::LongestTap, round_to(ms + 100.0, 25.0).max(150.0));
        say(
            Param::LongestTap,
            format!("taps last up to {ms:.0} ms (19 in 20), plus 100 ms"),
        );
        let lp = round_to((ms + 250.0).max(400.0), 50.0);
        t.set(Param::LongPress, lp);
        say(
            Param::LongPress,
            format!("well past the longest taps ({ms:.0} ms)"),
        );
    } else {
        let left = NEED_TAPS - m.taps.len();
        for p in [Param::TapSlop, Param::LongestTap, Param::LongPress] {
            say(p, format!("needs {left} more taps"));
        }
    }
    if t.gestures.long_press_us < t.gestures.tap_max_us {
        t.gestures.long_press_us = t.gestures.tap_max_us;
    }

    let swipes: Vec<&Stroke> = m.swipes().collect();
    if swipes.len() >= NEED_SWIPES {
        let speed = quantile(swipes.iter().map(|s| s.speed), 0.5).unwrap_or(0.0);
        let px = quantile(swipes.iter().map(|s| s.distance_px), 0.5).unwrap_or(0.0);
        t.set(Param::SwipeSpeed, round_to(speed * 0.5, 50.0));
        t.set(Param::SwipeDistance, round_to(px * 0.5, 10.0));
        say(
            Param::SwipeSpeed,
            format!(
                "half your swipes' middle speed ({speed:.0} px/s, {} swipes)",
                swipes.len()
            ),
        );
        say(
            Param::SwipeDistance,
            format!("half your swipes' middle length ({px:.0} px)"),
        );
    } else {
        let left = NEED_SWIPES - swipes.len();
        for p in [Param::SwipeDistance, Param::SwipeSpeed] {
            say(p, format!("needs {left} more swipes (Swipes mode)"));
        }
    }
    (t, why)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f32, f32) = (1280.0, 800.0);

    #[test]
    fn every_value_round_trips_through_its_settings_pairs() {
        let mut t = Tuning::default();
        t.set(Param::OffsetX, 7.0);
        t.set(Param::OffsetY, -3.0);
        t.set(Param::ScaleX, 1.025);
        t.set(Param::HoldFilter, 2.0);
        t.set(Param::LongPress, 650.0);
        t.set(Param::SwipeSpeed, 700.0);
        let pairs = t.to_pairs();
        let back = Tuning::from_lookup(|k| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .and_then(|(_, v)| v.parse().ok())
        });
        for p in Param::ALL {
            assert!((back.get(p) - t.get(p)).abs() < 1e-3, "{p:?}");
        }
    }

    #[test]
    fn values_stay_within_bounds_whatever_is_read() {
        let t = Tuning::from_lookup(|k| match k {
            "offset-x" => Some(500.0),
            "scale-y" => Some(0.1),
            "hold-filter-px" => Some(99.0),
            "long-press-ms" => Some(10.0),
            _ => None,
        });
        assert_eq!(t.offset_x, MAX_OFFSET_PX);
        assert_eq!(t.scale_y, MIN_SCALE);
        assert_eq!(t.hold_filter_px, 6.0);
        // Raised to the bound, then to the longest tap.
        assert!(t.gestures.long_press_us >= t.gestures.tap_max_us);
        let mut u = Tuning::default();
        u.set(Param::OffsetY, f32::NAN);
        assert_eq!(u.offset_y, 0.0);
    }

    #[test]
    fn steps_snap_to_the_grid() {
        let mut t = Tuning::default();
        t.step(Param::ScaleX, 1);
        assert!((t.scale_x - 1.005).abs() < 1e-6);
        t.step(Param::LongPress, -1);
        assert_eq!(t.gestures.long_press_us, 475_000);
        for _ in 0..100 {
            t.step(Param::OffsetX, -1);
        }
        assert_eq!(t.offset_x, -MAX_OFFSET_PX);
    }

    #[test]
    fn the_default_tuning_moves_nothing() {
        let mut f = Filter::new(SIZE);
        assert!(!f.tuning().moves_touches());
        assert_eq!(f.apply(1, Phase::Down, 100.0, 200.0), Some((100.0, 200.0)));
        assert_eq!(f.apply(1, Phase::Move, 100.4, 200.0), Some((100.4, 200.0)));
        assert_eq!(f.apply(1, Phase::Up, 101.0, 200.0), Some((101.0, 200.0)));
    }

    #[test]
    fn the_correction_scales_about_the_centre_then_shifts() {
        let mut t = Tuning::default();
        t.set(Param::ScaleX, 1.1);
        t.set(Param::OffsetY, 10.0);
        assert_eq!(t.correct(SIZE, 640.0, 400.0), (640.0, 410.0));
        let (x, _) = t.correct(SIZE, 1240.0, 400.0);
        assert!((x - 1279.0).abs() < 1e-3, "kept on screen: {x}");
        let (x, _) = t.correct(SIZE, 140.0, 400.0);
        assert!((x - 90.0).abs() < 1e-3, "{x}");
    }

    #[test]
    fn the_hold_filter_drops_small_moves_and_lifts_where_it_last_was() {
        let mut f = Filter::new(SIZE);
        let mut t = Tuning::default();
        t.set(Param::HoldFilter, 3.0);
        f.set_tuning(t);
        assert_eq!(f.apply(4, Phase::Down, 300.0, 300.0), Some((300.0, 300.0)));
        assert_eq!(f.apply(4, Phase::Move, 301.0, 301.0), None);
        assert_eq!(f.apply(4, Phase::Move, 302.0, 299.0), None);
        assert_eq!(f.apply(4, Phase::Move, 304.0, 300.0), Some((304.0, 300.0)));
        assert_eq!(f.apply(4, Phase::Up, 305.0, 301.0), Some((304.0, 300.0)));
        // A second finger is filtered on its own.
        assert_eq!(f.apply(5, Phase::Down, 10.0, 10.0), Some((10.0, 10.0)));
        assert_eq!(f.apply(5, Phase::Up, 30.0, 10.0), Some((30.0, 10.0)));
    }

    fn stroke(
        m: &mut Measure,
        id: u64,
        t0: u64,
        from: (f32, f32),
        to: (f32, f32),
        steps: u64,
        step_us: u64,
    ) -> Option<Sample> {
        m.event(id, Phase::Down, from.0, from.1, t0);
        for i in 1..=steps {
            let k = i as f32 / steps as f32;
            m.event(
                id,
                Phase::Move,
                from.0 + (to.0 - from.0) * k,
                from.1 + (to.1 - from.1) * k,
                t0 + i * step_us,
            );
        }
        m.event(id, Phase::Up, to.0, to.1, t0 + steps * step_us + 1_000)
    }

    #[test]
    fn lifts_become_taps_holds_and_strokes() {
        let mut m = Measure::new();
        let s = stroke(&mut m, 1, 0, (100.0, 100.0), (103.0, 100.0), 3, 20_000);
        assert!(matches!(s, Some(Sample::Tap(t)) if t.travel_px == 3.0 && t.duration_us == 61_000));
        let s = stroke(
            &mut m,
            2,
            1_000_000,
            (200.0, 200.0),
            (202.0, 201.0),
            10,
            80_000,
        );
        assert!(
            matches!(s, Some(Sample::Hold(h)) if h.max_dev > 0.0 && h.duration_us == 801_000),
            "{s:?}"
        );
        // 300 px in 30 reports 9 ms apart: about 1100 px/s at the lift.
        let s = stroke(
            &mut m,
            3,
            5_000_000,
            (100.0, 400.0),
            (400.0, 410.0),
            30,
            9_000,
        );
        match s {
            Some(Sample::Stroke(st)) => {
                assert!((st.speed - 1111.0).abs() < 30.0, "{}", st.speed);
                assert!(st.axis_ratio > 20.0);
                assert!(!st.asked_swipe);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!((m.taps.len(), m.holds.len(), m.strokes.len()), (1, 1, 1));
        assert_eq!(m.rates.len(), 1);
        assert!((m.rates[0] - 111.0).abs() < 2.0, "{}", m.rates[0]);
        assert!(s
            .unwrap()
            .line()
            .starts_with("touch-measure: stroke px=300"));
        // A cancelled contact counts as nothing.
        m.event(9, Phase::Down, 0.0, 0.0, 0);
        assert_eq!(m.event(9, Phase::Cancel, 0.0, 0.0, 10), Some(Sample::Other));
        assert_eq!(m.fingers_down(), 0);
    }

    #[test]
    fn a_finger_that_paused_before_lifting_has_no_release_speed() {
        let mut m = Measure::new();
        m.event(1, Phase::Down, 0.0, 0.0, 0);
        for i in 1..=20u64 {
            m.event(1, Phase::Move, i as f32 * 10.0, 0.0, i * 9_000);
        }
        let s = m.event(1, Phase::Up, 200.0, 0.0, 20 * 9_000 + 300_000);
        assert!(
            matches!(s, Some(Sample::Stroke(st)) if st.speed == 0.0),
            "{s:?}"
        );
    }

    #[test]
    fn nothing_is_suggested_without_enough_samples() {
        let t = Tuning::default();
        let (s, why) = suggest(&t, &Measure::new(), SIZE);
        assert_eq!(s, t);
        assert_eq!(why.len(), Param::ALL.len());
        assert!(why.iter().all(|r| r.text.starts_with("needs")));
    }

    #[test]
    fn a_shifted_and_stretched_panel_is_fitted_back() {
        let mut m = Measure::new();
        // The panel reads 2 % wide about the centre and 6 px high.
        for gy in [100.0, 400.0, 700.0] {
            for gx in [100.0, 640.0, 1180.0] {
                let at = (640.0 + (gx - 640.0) * 1.02, gy - 6.0);
                m.target((gx, gy), at);
            }
        }
        let t = Tuning::default();
        let (mean, _) = m.target_error(&t, SIZE).unwrap();
        assert!(mean > 5.0);
        let (s, _) = suggest(&t, &m, SIZE);
        assert!((s.scale_x - 0.98).abs() < 0.0051, "{}", s.scale_x);
        assert_eq!(s.offset_y, 6.0);
        let (mean, worst) = m.target_error(&s, SIZE).unwrap();
        assert!(worst < 1.5, "mean {mean} worst {worst}");
    }

    #[test]
    fn taps_holds_and_swipes_suggest_thresholds() {
        let mut m = Measure::new();
        for i in 0..10u64 {
            // Taps of 80-170 ms moving up to 4 px.
            let t0 = i * 1_000_000;
            m.event(i, Phase::Down, 500.0, 500.0, t0);
            m.event(i, Phase::Move, 500.0 + (i % 5) as f32, 500.0, t0 + 40_000);
            m.event(i, Phase::Up, 500.0, 500.0, t0 + 80_000 + i * 10_000);
        }
        for i in 0..3u64 {
            // Held 1 s, wandering 2.5 px either side.
            let t0 = 20_000_000 + i * 2_000_000;
            m.event(20 + i, Phase::Down, 300.0, 300.0, t0);
            for k in 1..=10u64 {
                let x = 300.0 + [0.0, 2.5, 0.0, -2.5][k as usize % 4];
                m.event(20 + i, Phase::Move, x, 300.0, t0 + k * 100_000);
            }
            m.event(20 + i, Phase::Up, 300.0, 300.0, t0 + 1_050_000);
        }
        m.asking_swipes = true;
        for i in 0..4u64 {
            stroke(
                &mut m,
                40 + i,
                40_000_000 + i * 1_000_000,
                (200.0, 400.0),
                (600.0, 400.0),
                20,
                9_000,
            );
        }
        let (s, why) = suggest(&Tuning::default(), &m, SIZE);
        assert_eq!(s.hold_filter_px, 3.0, "{why:?}");
        assert_eq!(s.gestures.slop_px, 8.0, "{why:?}");
        assert_eq!(s.gestures.tap_max_us, 275_000);
        assert_eq!(s.gestures.long_press_us, 400_000);
        assert!(s.gestures.long_press_us >= s.gestures.tap_max_us);
        // 400 px in 180 ms: about 2200 px/s at the lift, so 1100 suggested.
        assert!(
            (s.gestures.swipe_min_speed - 1100.0).abs() <= 50.0,
            "{}",
            s.gestures.swipe_min_speed
        );
        assert_eq!(s.gestures.swipe_min_px, 200.0);
        // Position needs targets; it is left alone.
        assert_eq!((s.offset_x, s.scale_x), (0.0, 1.0));
        assert!(why
            .iter()
            .any(|r| r.param == Param::OffsetX && r.text.contains("target")));
    }

    #[test]
    fn quantiles_use_the_nearest_rank() {
        let v = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(quantile(v.iter().copied(), 0.5), Some(3.0));
        assert_eq!(quantile(v.iter().copied(), 0.95), Some(5.0));
        assert_eq!(quantile(v.iter().copied(), 0.0), Some(1.0));
        assert_eq!(quantile(std::iter::empty(), 0.5), None);
    }
}
