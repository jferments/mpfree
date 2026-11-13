//! The Dev tab's Touch page (M17): test the touchscreen, measure it, and tune
//! touch for the whole app from what was measured.
//!
//! - The test area ([`TouchTest`]) draws every finger where the app sees it
//!   (after the tuning), with where it was before the tuning and what the
//!   panel reported, and names the gestures the current thresholds make of
//!   it. It measures every finger *before* the tuning (`mpfree_touch::tuning`):
//!   taps, holds, strokes and report rates. Its modes: Free (touch anything),
//!   Targets (nine targets over the whole screen, for the position
//!   correction) and Swipes (strokes count as swipes, for the swipe
//!   thresholds).
//! - The tuning list ([`TuneList`]) shows each value now and what the
//!   measurements suggest, with why. The wheel picks a row, - / + or the
//!   row's own buttons change it, and it applies at once. "Use suggested"
//!   takes every suggestion, "Defaults" goes back to the built-in values,
//!   and "Save" keeps them in the store ("touch" settings), read at the next
//!   start.
//!
//! The position correction and the hold filter apply to every touch in the
//! app. The gesture thresholds apply to the gesture recognisers (this page,
//! the Touch lab) and are kept for the widgets that use them. Every value is
//! bounded (`Param::min`/`max`), so the page stays usable whatever is set.

use crate::touchlab_page::describe;
use mpfree_touch::gesture::{Gesture, GestureConfig, GestureRecognizer};
use mpfree_touch::mt::{ContactId, Phase};
use mpfree_touch::pointer::PointerEvent;
use mpfree_touch::profile::MPC_ONE;
use mpfree_touch::tuning::{suggest, Measure, Param, Reason, Sample, Tuning};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Tone, Touch, Widget};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

pub const TEST: &str = "ttest";
pub const TUNE: &str = "ttune";
pub const PAGE: &str = "Touch";
/// Targets mode: the test area over the whole page.
pub const TARGETS_PAGE: &str = "Touch targets";
pub const LAYOUT: &str = "col(header:60, dtabs:52, row(ttest:*, ttune:520):*, hint:26)";
pub const TARGETS_LAYOUT: &str = "col(header:60, dtabs:52, ttest:*, hint:26)";

/// The test area's modes, in its bar, then Clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Free,
    Targets,
    Swipes,
}

const MODES: [&str; 4] = ["Free", "Targets", "Swipes", "Clear"];
const BAR_H: i32 = 44;
/// Targets: how far the outer targets sit from the area's edges, their ring,
/// and how near a tap must land to count.
const TARGET_INSET: i32 = 56;
const TARGET_RADIUS: i32 = 30;
const TARGET_REACH: f32 = 110.0;
/// Trail points kept per finger.
const TRAIL: usize = 48;
const GESTURES_SHOWN: usize = 5;

fn wall_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

fn to_phase(t: Touch) -> Phase {
    match t {
        Touch::Down => Phase::Down,
        Touch::Move => Phase::Move,
        Touch::Up => Phase::Up,
        Touch::Cancel => Phase::Cancel,
    }
}

struct Finger {
    id: u64,
    /// Where the app sees it (after the tuning), newest last.
    trail: VecDeque<(i32, i32)>,
    /// Where it was before the tuning.
    raw: (f32, f32),
    /// Down on the mode bar: which entry.
    bar: Option<usize>,
}

/// The test area: fingers, gestures, measurements and the targets.
pub struct TouchTest {
    pub mode: Mode,
    pub measure: Measure,
    recognizer: GestureRecognizer,
    config: GestureConfig,
    fingers: Vec<Finger>,
    gestures: VecDeque<String>,
    /// Target taps so far in this round (the targets are `measure.targets`).
    target_note: Option<(String, Tone)>,
    /// The last lift's measurement, as shown.
    last_sample: String,
    clock: Box<dyn Fn() -> u64 + Send>,
    event_time: Option<u64>,
    lines: Vec<String>,
    /// Where the area was drawn last (the targets are placed in it).
    area: Rect,
}

impl Default for TouchTest {
    fn default() -> Self {
        TouchTest::with_clock(Box::new(wall_clock))
    }
}

impl TouchTest {
    /// `clock`: microseconds on the clock evdev stamps events with (tests
    /// pass their own).
    pub fn with_clock(clock: Box<dyn Fn() -> u64 + Send>) -> TouchTest {
        TouchTest {
            mode: Mode::Free,
            measure: Measure::new(),
            recognizer: GestureRecognizer::new(GestureConfig::default()),
            config: GestureConfig::default(),
            fingers: Vec::new(),
            gestures: VecDeque::new(),
            target_note: None,
            last_sample: String::new(),
            clock,
            event_time: None,
            lines: Vec::new(),
            area: Rect::new(0, 0, 0, 0),
        }
    }

    /// A different clock (previews and tests).
    pub fn set_clock(&mut self, clock: Box<dyn Fn() -> u64 + Send>) {
        self.clock = clock;
    }

    /// New thresholds for the recogniser (fingers down start again).
    pub fn set_config(&mut self, c: GestureConfig) {
        self.config = c;
        self.recognizer = GestureRecognizer::new(c);
    }

    /// The kernel's time of the event about to come (used when it is within
    /// a second of this page's clock).
    pub fn set_event_time(&mut self, t: u64) {
        self.event_time = Some(t);
    }

    fn stamp(&self) -> u64 {
        let now = (self.clock)();
        match self.event_time {
            Some(t) if t.abs_diff(now) <= 1_000_000 => t,
            _ => now,
        }
    }

    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    pub fn has_lines(&self) -> bool {
        !self.lines.is_empty()
    }

    /// Targets over the area, row by row (three rows of three).
    pub fn targets(area: Rect) -> Vec<(i32, i32)> {
        let xs = [
            area.x + TARGET_INSET,
            area.x + area.w / 2,
            area.right() - 1 - TARGET_INSET,
        ];
        let ys = [
            area.y + TARGET_INSET,
            area.y + area.h / 2,
            area.bottom() - 1 - TARGET_INSET,
        ];
        ys.iter()
            .flat_map(|&y| xs.iter().map(move |&x| (x, y)))
            .collect()
    }

    /// The next target, while Targets mode has one left.
    pub fn next_target(&self) -> Option<(i32, i32)> {
        if self.mode != Mode::Targets {
            return None;
        }
        Self::targets(self.area)
            .get(self.measure.targets.len())
            .copied()
    }

    /// True while Targets mode wants the whole page.
    pub fn wants_whole_page(&self) -> bool {
        self.mode == Mode::Targets && self.next_target().is_some()
    }

    fn set_mode(&mut self, m: Mode) {
        if m == Mode::Targets {
            self.measure.targets.clear();
            self.target_note = None;
        }
        self.mode = m;
        self.measure.asking_swipes = m == Mode::Swipes;
        let word = match m {
            Mode::Free => "free",
            Mode::Targets => "targets",
            Mode::Swipes => "swipes",
        };
        self.lines.push(format!("touch-tune-mode: {word}"));
    }

    /// Back to Free (when the page is left).
    pub fn leave(&mut self) {
        if self.mode != Mode::Free {
            self.set_mode(Mode::Free);
        }
        self.fingers.clear();
        self.recognizer = GestureRecognizer::new(self.config);
    }

    /// One finger event before the tuning (screen px), from the player, for
    /// the measurements and the targets. `area` is where the test area is on
    /// screen now.
    pub fn observe(&mut self, finger: u64, phase: Touch, x: f32, y: f32, area: Rect) {
        let t = self.stamp();
        if area.w > 0 {
            self.area = Self::body(area).1;
        }
        if let Some(f) = self.fingers.iter_mut().find(|f| f.id == finger) {
            f.raw = (x, y);
        }
        if phase == Touch::Down {
            if let Some(target) = self.next_target() {
                let d = (x - target.0 as f32).hypot(y - target.1 as f32);
                if d <= TARGET_REACH {
                    self.measure
                        .target((target.0 as f32, target.1 as f32), (x, y));
                    let n = self.measure.targets.len();
                    self.lines.push(format!(
                        "touch-target: n={n} target={},{} at={x:.0},{y:.0} miss_px={d:.1}",
                        target.0, target.1
                    ));
                    self.target_note = Some(if self.next_target().is_none() {
                        (
                            "All nine done: the suggestions are on the right.".to_string(),
                            Tone::Good,
                        )
                    } else {
                        (format!("{n} of 9: {d:.0} px from the centre"), Tone::Dim)
                    });
                } else if !self.on_bar(x as i32, y as i32) {
                    self.target_note = Some((
                        "That was not near the ring: tap its centre.".to_string(),
                        Tone::Warn,
                    ));
                }
            }
        }
        if let Some(s) = self.measure.event(finger, to_phase(phase), x, y, t) {
            if s != Sample::Other {
                let line = s.line();
                self.last_sample = line.trim_start_matches("touch-measure: ").to_string();
                self.lines.push(line);
            }
        }
    }

    fn on_bar(&self, x: i32, y: i32) -> bool {
        y < self.area.y && y >= self.area.y - BAR_H && x >= self.area.x && x < self.area.right()
    }

    /// The panel's title line and the rest: (mode bar, area).
    pub(crate) fn body(r: Rect) -> (Rect, Rect) {
        let inner = r.inset(12);
        let (bar, rest) = inner.take_top(BAR_H);
        (bar, Rect::new(rest.x, rest.y + 6, rest.w, rest.h - 6))
    }

    pub(crate) fn bar_cells(bar: Rect) -> Vec<Rect> {
        let n = MODES.len() as i32;
        let w = (bar.w - 8 * (n - 1)) / n;
        (0..n)
            .map(|i| Rect::new(bar.x + i * (w + 8), bar.y, w, bar.h))
            .collect()
    }

    /// Fire long presses that are due. True when something changed.
    pub fn tick(&mut self) -> bool {
        if self.fingers.is_empty() {
            return false;
        }
        let mut out = Vec::new();
        self.recognizer.tick((self.clock)(), &mut out);
        self.note_gestures(&out)
    }

    fn note_gestures(&mut self, out: &[Gesture]) -> bool {
        let mut changed = false;
        for g in out {
            if matches!(g, Gesture::Drag { .. } | Gesture::TwoFinger { .. }) {
                continue;
            }
            if self.gestures.len() >= GESTURES_SHOWN {
                self.gestures.pop_front();
            }
            self.gestures.push_back(describe(g));
            changed = true;
        }
        changed
    }

    /// The measurements in two lines.
    pub fn summary(&self) -> (String, String) {
        let m = &self.measure;
        let rate = mpfree_touch::tuning::quantile(m.rates.iter().copied(), 0.5);
        let swipes = m.swipes().count();
        (
            format!(
                "{} taps · {} holds · {} strokes ({} swipes) · {}",
                m.taps.len(),
                m.holds.len(),
                m.strokes.len(),
                swipes,
                match rate {
                    Some(r) => format!("{r:.0} reports/s moving"),
                    None => "report rate after a few strokes".to_string(),
                }
            ),
            if self.last_sample.is_empty() {
                "Tap, hold still, drag and swipe anywhere here.".to_string()
            } else {
                format!("Last: {}", self.last_sample)
            },
        )
    }

    /// The `touch-tune-summary:` journal line.
    pub fn summary_line(&self, now: &Tuning, size: (f32, f32)) -> String {
        let m = &self.measure;
        let rate = mpfree_touch::tuning::quantile(m.rates.iter().copied(), 0.5).unwrap_or(0.0);
        let err = match m.target_error(&Tuning::default(), size) {
            Some((mean, worst)) => format!(" target_miss_px={mean:.1}/{worst:.1}"),
            None => String::new(),
        };
        let fixed = match (m.targets.is_empty(), m.target_error(now, size)) {
            (false, Some((mean, worst))) => format!(" tuned_miss_px={mean:.1}/{worst:.1}"),
            _ => String::new(),
        };
        format!(
            "touch-tune-summary: taps={} holds={} strokes={} swipes={} rate_hz={rate:.0} targets={}{err}{fixed}",
            m.taps.len(),
            m.holds.len(),
            m.strokes.len(),
            m.swipes().count(),
            m.targets.len()
        )
    }
}

fn circle(cx: &mut PaintCx, x: i32, y: i32, radius: i32, c: Color) {
    cx.canvas.fill_round_rect(
        Rect::new(x - radius, y - radius, 2 * radius, 2 * radius),
        radius,
        c,
    );
}

/// A one-pixel line (steps of small squares).
fn line(cx: &mut PaintCx, a: (i32, i32), b: (i32, i32), w: i32, c: Color) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = dx.abs().max(dy.abs()).max(1);
    for i in 0..=n {
        let x = a.0 + dx * i / n;
        let y = a.1 + dy * i / n;
        cx.canvas
            .fill_rect(Rect::new(x - w / 2, y - w / 2, w, w), c);
    }
}

impl Widget for TouchTest {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let (bar, area) = Self::body(r);
        let active = match self.mode {
            Mode::Free => 0,
            Mode::Targets => 1,
            Mode::Swipes => 2,
        };
        for (i, (cell, name)) in Self::bar_cells(bar).into_iter().zip(MODES).enumerate() {
            let on = i == active;
            cx.canvas
                .fill_round_rect(cell, t.radius, if on { t.accent } else { t.raised });
            cx.text(
                FontId::Bold,
                t.size_body,
                cell,
                Align::Center,
                if on { t.on_accent } else { t.text },
                name,
            );
        }
        cx.canvas.fill_round_rect(area, 8, t.bg);
        // A faint grid every 80 px.
        let mut gx = area.x + 80;
        while gx < area.right() {
            cx.canvas
                .fill_rect(Rect::new(gx, area.y, 1, area.h), t.outline);
            gx += 80;
        }
        let mut gy = area.y + 80;
        while gy < area.bottom() {
            cx.canvas
                .fill_rect(Rect::new(area.x, gy, area.w, 1), t.outline);
            gy += 80;
        }
        let small = t.size_small;
        let head = Rect::new(area.x + 12, area.y + 8, area.w - 24, 24);
        match self.mode {
            Mode::Targets => {
                let targets = Self::targets(area);
                let next = self.measure.targets.len();
                for (i, &(x, y)) in targets.iter().enumerate() {
                    if i == next {
                        circle(cx, x, y, TARGET_RADIUS, t.accent);
                        circle(cx, x, y, TARGET_RADIUS - 7, t.bg);
                        circle(cx, x, y, 6, t.accent);
                    } else {
                        circle(cx, x, y, 8, t.outline);
                    }
                }
                for &((tx, ty), (ax, ay)) in &self.measure.targets {
                    let (tx, ty, ax, ay) = (tx as i32, ty as i32, ax as i32, ay as i32);
                    line(cx, (tx, ty), (ax, ay), 2, t.warn);
                    circle(cx, ax, ay, 6, t.warn);
                    circle(cx, tx, ty, 4, t.good);
                }
                let (msg, tone) = self.target_note.clone().unwrap_or((
                    "Tap the centre of each ring as it lights, as you would a button.".to_string(),
                    Tone::Accent,
                ));
                cx.text(
                    FontId::Sans,
                    t.size_body,
                    head,
                    Align::Center,
                    t.tone(tone),
                    &msg,
                );
            }
            Mode::Swipes => {
                cx.text(
                    FontId::Sans,
                    t.size_body,
                    head,
                    Align::Left,
                    t.accent,
                    "Swipe as you would to turn a page, in any direction, a few times.",
                );
            }
            Mode::Free => {}
        }
        for f in &self.fingers {
            let pts: Vec<(i32, i32)> = f.trail.iter().copied().collect();
            for w in pts.windows(2) {
                line(cx, w[0], w[1], 3, t.text_dim);
            }
            let Some(&(x, y)) = f.trail.back() else {
                continue;
            };
            circle(cx, x, y, 26, t.accent);
            circle(cx, x, y, 20, t.bg);
            circle(cx, x, y, 5, t.accent);
            let (rx, ry) = (f.raw.0.round() as i32, f.raw.1.round() as i32);
            if (rx, ry) != (x, y) {
                circle(cx, rx, ry, 4, t.warn);
            }
            let panel = MPC_ONE
                .axis_map(1280, 800)
                .map(|m| m.unmap(f.raw.0, f.raw.1))
                .ok();
            let label = match panel {
                Some((px, py)) => format!("{x}, {y}  ·  untuned {rx}, {ry}  ·  panel {px}, {py}"),
                None => format!("{x}, {y}"),
            };
            let lw = cx.measure(FontId::Sans, small, &label) + 16;
            let lx = if x + 34 + lw < area.right() {
                x + 34
            } else {
                x - 34 - lw
            };
            let lx = lx.clamp(area.x + 4, (area.right() - 4 - lw).max(area.x + 4));
            let lr = Rect::new(lx, y - 14, lw, 28);
            cx.canvas.fill_round_rect(lr, 6, t.raised);
            cx.text(FontId::Sans, small, lr, Align::Center, t.text, &label);
        }
        // The gestures and the measurements, at the bottom (not over the
        // targets).
        if self.mode == Mode::Targets {
            return;
        }
        let lines_r = Rect::new(area.x + 12, area.bottom() - 84, area.w - 24, 76);
        let g = if self.gestures.is_empty() {
            "Gestures: none yet".to_string()
        } else {
            format!(
                "Gestures: {}",
                self.gestures
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("  ·  ")
            )
        };
        let (a, b) = self.summary();
        for (i, (s, c)) in [(g, t.text), (a, t.text_dim), (b, t.text_dim)]
            .into_iter()
            .enumerate()
        {
            cx.text(
                FontId::Sans,
                small,
                Rect::new(lines_r.x, lines_r.y + i as i32 * 26, lines_r.w, 24),
                Align::Left,
                c,
                &s,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (bar, area) = Self::body(r);
        let mut out: Vec<Target> = Self::bar_cells(bar)
            .into_iter()
            .enumerate()
            .map(|(i, c)| Target::tap("mode", i, MODES[i], c))
            .collect();
        // A finger anywhere in the area leaves its trail.
        out.push(Target::drag("area", 0, "test area", area));
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let t = self.stamp();
        self.event_time = None;
        let (bar, area) = Self::body(r);
        self.area = area;
        match phase {
            Touch::Down => {
                let cell = Self::bar_cells(bar).iter().position(|c| c.contains(x, y));
                self.fingers.retain(|f| f.id != finger);
                self.fingers.push(Finger {
                    id: finger,
                    trail: VecDeque::from([(x, y)]),
                    raw: (x as f32, y as f32),
                    bar: cell,
                });
            }
            Touch::Move => {
                if let Some(f) = self.fingers.iter_mut().find(|f| f.id == finger) {
                    if f.trail.len() >= TRAIL {
                        f.trail.pop_front();
                    }
                    f.trail.push_back((x, y));
                }
            }
            Touch::Up | Touch::Cancel => {
                let i = self.fingers.iter().position(|f| f.id == finger);
                let f = i.map(|i| self.fingers.remove(i));
                let bar_hit = f
                    .and_then(|f| f.bar)
                    .filter(|&c| phase == Touch::Up && Self::bar_cells(bar)[c].contains(x, y));
                match bar_hit {
                    Some(0) => self.set_mode(Mode::Free),
                    Some(1) => self.set_mode(Mode::Targets),
                    Some(2) => self.set_mode(Mode::Swipes),
                    Some(_) => {
                        self.measure.clear();
                        self.gestures.clear();
                        self.last_sample.clear();
                        self.target_note = None;
                        self.lines.push("touch-tune-clear".to_string());
                    }
                    None => {}
                }
            }
        }
        let ev = PointerEvent {
            id: ContactId(finger),
            phase: to_phase(phase),
            x: x as f32,
            y: y as f32,
            time_us: t,
        };
        let mut out = Vec::new();
        self.recognizer.frame(&[ev], &mut out);
        self.note_gestures(&out);
        None
    }

    fn animating(&self) -> bool {
        !self.fingers.is_empty()
    }
}

// ---------------------------------------------------------------- the list

/// The actions after the values, in the list's last row.
pub const ACTIONS: [&str; 3] = ["Use suggested", "Defaults", "Save"];
const ROW_H: i32 = 38;

/// What the tuning list shows (the player fills it in).
#[derive(Debug, Clone, PartialEq)]
pub struct TuneRow {
    pub label: &'static str,
    pub now: String,
    pub suggested: String,
    /// The suggestion differs from the value now.
    pub differs: bool,
    pub help: &'static str,
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TuneList {
    pub rows: Vec<TuneRow>,
    /// A value row (0..rows) or an action (rows..rows + 3).
    pub selected: usize,
    pub status: (String, Tone),
}

impl TuneList {
    pub fn len(&self) -> usize {
        self.rows.len() + ACTIONS.len()
    }

    fn row_rect(body: Rect, i: usize) -> Rect {
        Rect::new(body.x, body.y + i as i32 * ROW_H, body.w, ROW_H - 4)
    }

    /// The - and + boxes of a row.
    fn steppers(row: Rect) -> (Rect, Rect) {
        let plus = Rect::new(row.right() - 44, row.y + 1, 44, row.h - 2);
        let minus = Rect::new(plus.x - 50, row.y + 1, 44, row.h - 2);
        (minus, plus)
    }

    fn action_rects(&self, body: Rect) -> Vec<Rect> {
        let y = body.y + self.rows.len() as i32 * ROW_H + 8;
        let n = ACTIONS.len() as i32;
        let w = (body.w - 8 * (n - 1)) / n;
        (0..n)
            .map(|i| Rect::new(body.x + i * (w + 8), y, w, 48))
            .collect()
    }

    /// Below the title and the status line.
    fn body(r: Rect) -> Rect {
        let inner = r.inset(16);
        let (_, rest) = inner.take_top(26 + 30);
        rest
    }
}

impl Widget for TuneList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let top = cx.titled_panel(r, "Tuning", "now · suggested");
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(top.x, top.y, top.w, 26),
            Align::Left,
            t.tone(self.status.1),
            &self.status.0,
        );
        let body = Self::body(r);
        for (i, row) in self.rows.iter().enumerate() {
            let rr = Self::row_rect(body, i);
            if i == self.selected {
                cx.canvas.fill_round_rect(rr, 8, t.raised);
                cx.canvas.stroke_round_rect(rr, 8, 2, t.accent);
            }
            let (minus, plus) = Self::steppers(rr);
            let text_r = Rect::new(rr.x + 10, rr.y, minus.x - rr.x - 14, rr.h);
            cx.text(
                FontId::Sans,
                t.size_body,
                text_r,
                Align::Left,
                t.text,
                row.label,
            );
            let sug_colour = if row.differs { t.accent } else { t.text_dim };
            let sug_w = 110;
            let sug = Rect::new(minus.x - 8 - sug_w, rr.y, sug_w, rr.h);
            let now = Rect::new(sug.x - 8 - sug_w, rr.y, sug_w, rr.h);
            cx.text(
                FontId::Bold,
                t.size_body,
                now,
                Align::Right,
                t.text,
                &row.now,
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                sug,
                Align::Right,
                sug_colour,
                &row.suggested,
            );
            for (b, s) in [(minus, "-"), (plus, "+")] {
                cx.canvas.fill_round_rect(b, 8, t.surface);
                cx.canvas.stroke_round_rect(b, 8, 1, t.outline);
                cx.text(FontId::Bold, t.size_body, b, Align::Center, t.text, s);
            }
        }
        for (i, (b, name)) in self.action_rects(body).into_iter().zip(ACTIONS).enumerate() {
            let sel = self.selected == self.rows.len() + i;
            cx.canvas
                .fill_round_rect(b, t.radius, if sel { t.accent } else { t.raised });
            cx.canvas.stroke_round_rect(b, t.radius, 1, t.outline);
            cx.text(
                FontId::Bold,
                t.size_body,
                b,
                Align::Center,
                if sel { t.on_accent } else { t.text },
                name,
            );
        }
        // What the selected row does, and why it is suggested.
        let y = body.y + self.rows.len() as i32 * ROW_H + 64;
        let (help, why) = match self.rows.get(self.selected) {
            Some(row) => (row.help.to_string(), format!("Suggested: {}", row.why)),
            None => match self.selected - self.rows.len() {
                0 => (
                    "Take every suggested value at once.".to_string(),
                    String::new(),
                ),
                1 => (
                    "Back to the built-in values (nothing moves touches).".to_string(),
                    String::new(),
                ),
                _ => (
                    "Keep these values for the next start (the store's touch settings)."
                        .to_string(),
                    String::new(),
                ),
            },
        };
        for (i, (s, c)) in [(help, t.text), (why, t.text_dim)].into_iter().enumerate() {
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(body.x, y + i as i32 * 26, body.w, 24),
                Align::Left,
                c,
                &s,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let body = Self::body(r);
        let mut out = Vec::new();
        for i in 0..self.rows.len() {
            let rr = Self::row_rect(body, i);
            let (minus, plus) = Self::steppers(rr);
            let label = self.rows[i].label;
            out.push(Target::tap(
                "row",
                i,
                label,
                Rect::new(rr.x, rr.y, minus.x - rr.x, rr.h),
            ));
            out.push(Target::tap("row-minus", i, format!("{label} -"), minus));
            out.push(Target::tap("row-plus", i, format!("{label} +"), plus));
        }
        for (i, b) in self.action_rects(body).into_iter().enumerate() {
            out.push(Target::tap("action", i, ACTIONS[i], b));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let body = Self::body(r);
        for i in 0..self.rows.len() {
            let rr = Self::row_rect(body, i);
            if !rr.contains(x, y) {
                continue;
            }
            let (minus, plus) = Self::steppers(rr);
            if minus.contains(x, y) {
                return Some(EventKind::Step {
                    index: i,
                    delta: -1,
                });
            }
            if plus.contains(x, y) {
                return Some(EventKind::Step { index: i, delta: 1 });
            }
            return Some(EventKind::Select(i));
        }
        self.action_rects(body)
            .iter()
            .position(|b| b.contains(x, y))
            .map(EventKind::Tab)
    }
}

/// The rows for a tuning and its suggestion.
pub fn rows(now: &Tuning, suggested: &Tuning, why: &[Reason]) -> Vec<TuneRow> {
    Param::ALL
        .iter()
        .map(|&p| {
            let (a, b) = (now.get(p), suggested.get(p));
            TuneRow {
                label: p.label(),
                now: p.show(a),
                suggested: p.show(b),
                differs: (a - b).abs() > p.step() / 2.0,
                help: p.help(),
                why: why
                    .iter()
                    .find(|r| r.param == p)
                    .map(|r| r.text.clone())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// The suggestion for the measurements so far.
pub fn suggestion(now: &Tuning, test: &TouchTest, size: (f32, f32)) -> (Tuning, Vec<Reason>) {
    suggest(now, &test.measure, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    const R: Rect = Rect {
        x: 0,
        y: 112,
        w: 1280,
        h: 662,
    };

    fn test_area() -> (TouchTest, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(1_000_000));
        let c = now.clone();
        let t = TouchTest::with_clock(Box::new(move || c.load(Ordering::SeqCst)));
        (t, now)
    }

    fn bar_tap(t: &mut TouchTest, i: usize) {
        let (bar, _) = TouchTest::body(R);
        let c = TouchTest::bar_cells(bar)[i];
        let (x, y) = (c.x + c.w / 2, c.y + c.h / 2);
        t.touch(90, Touch::Down, x, y, R);
        t.touch(90, Touch::Up, x, y, R);
    }

    #[test]
    fn nine_target_taps_fill_the_targets_then_the_page_comes_back() {
        let (mut t, now) = test_area();
        t.observe(1, Touch::Down, 10.0, 10.0, R);
        assert!(!t.wants_whole_page());
        bar_tap(&mut t, 1);
        assert_eq!(t.mode, Mode::Targets);
        assert!(t.wants_whole_page());
        let targets = TouchTest::targets(TouchTest::body(R).1);
        assert_eq!(targets.len(), 9);
        // Far from the ring: not counted.
        t.observe(2, Touch::Down, 640.0, 450.0, R);
        t.observe(2, Touch::Up, 640.0, 450.0, R);
        assert!(t.measure.targets.is_empty());
        for (i, &(x, y)) in targets.iter().enumerate() {
            now.fetch_add(500_000, Ordering::SeqCst);
            t.observe(
                10 + i as u64,
                Touch::Down,
                x as f32 + 3.0,
                y as f32 - 4.0,
                R,
            );
            t.observe(10 + i as u64, Touch::Up, x as f32 + 3.0, y as f32 - 4.0, R);
        }
        assert_eq!(t.measure.targets.len(), 9);
        assert!(!t.wants_whole_page());
        let lines = t.take_lines();
        assert!(
            lines.iter().any(|l| l.starts_with("touch-target: n=9")),
            "{lines:?}"
        );
        let (s, _) = suggestion(&Tuning::default(), &t, (1280.0, 800.0));
        assert_eq!((s.offset_x, s.offset_y), (-3.0, 4.0));
    }

    #[test]
    fn swipes_mode_marks_strokes_and_clear_forgets_them() {
        let (mut t, now) = test_area();
        bar_tap(&mut t, 2);
        assert!(t.measure.asking_swipes);
        for k in 0..20u64 {
            now.store(2_000_000 + k * 9_000, Ordering::SeqCst);
            let phase = if k == 0 { Touch::Down } else { Touch::Move };
            t.observe(5, phase, 200.0 + k as f32 * 20.0, 400.0, R);
        }
        t.observe(5, Touch::Up, 580.0, 400.0, R);
        assert_eq!(t.measure.swipes().count(), 1);
        assert!(t.summary().0.contains("1 swipes"));
        bar_tap(&mut t, 3);
        assert_eq!(t.measure.strokes.len(), 0);
        assert!(t.take_lines().contains(&"touch-tune-clear".to_string()));
    }

    #[test]
    fn the_recogniser_names_a_tap_with_the_thresholds_set() {
        let (mut t, _) = test_area();
        let (_, area) = TouchTest::body(R);
        let (x, y) = (area.x + 300, area.y + 300);
        t.touch(3, Touch::Down, x, y, R);
        t.touch(3, Touch::Up, x, y, R);
        assert!(
            t.gestures.back().is_some_and(|g| g.starts_with("tap")),
            "{:?}",
            t.gestures
        );
        // A tighter slop: the same 10 px wobble is no longer a tap.
        let c = GestureConfig {
            slop_px: 5.0,
            ..GestureConfig::default()
        };
        t.set_config(c);
        t.gestures.clear();
        t.touch(4, Touch::Down, x, y, R);
        t.touch(4, Touch::Move, x + 10, y, R);
        t.touch(4, Touch::Up, x + 10, y, R);
        assert!(
            !t.gestures.iter().any(|g| g.starts_with("tap")),
            "{:?}",
            t.gestures
        );
    }

    #[test]
    fn list_rows_step_select_and_run_actions() {
        let mut l = TuneList {
            rows: rows(&Tuning::default(), &Tuning::default(), &[]),
            ..TuneList::default()
        };
        let r = Rect::new(760, 112, 520, 662);
        let body = TuneList::body(r);
        let row = TuneList::row_rect(body, 2);
        let (minus, plus) = TuneList::steppers(row);
        let at = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = at(plus);
        assert_eq!(
            l.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Step { index: 2, delta: 1 })
        );
        let (x, y) = at(minus);
        assert_eq!(
            l.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Step {
                index: 2,
                delta: -1
            })
        );
        assert_eq!(
            l.touch(1, Touch::Down, row.x + 20, row.y + 10, r),
            Some(EventKind::Select(2))
        );
        let save = l.action_rects(body)[2];
        let (x, y) = at(save);
        assert_eq!(l.touch(1, Touch::Down, x, y, r), Some(EventKind::Tab(2)));
        assert_eq!(l.len(), Param::ALL.len() + 3);
        // The list fits its panel.
        assert!(save.bottom() + 64 <= r.bottom(), "{save:?}");
    }
}
