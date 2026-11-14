//! The Touch lab page: the M15 steps on screen, paced by the owner on the MPC
//! itself (Start, a 3-2-1 countdown, the window, STOP, the result, Next), so
//! the host side only reads the journal.
//!
//! It opens from Prefs > Touch screen ("Touch lab", reachable with SHIFT + MENU and the
//! wheel, so it can be opened even if touch is dead) and closes with its Exit
//! button or any page change from the app. While it is open, touches go only
//! to the lab, never to the pad player.
//!
//! Every touch in the work area during a window goes to a `StepRecorder` and
//! a `GestureRouter` (in the pad-and-swipe step, fingers that start on the
//! pad are kept out of the router, as a pad widget would be). Each gesture and
//! each step's summary is queued as a journal line (`touch-gesture:`,
//! `touch-lab:`) for the screen thread to print.

use mpfree_touch::gesture::{Gesture, GestureConfig, SwipeDirection};
use mpfree_touch::mt::{ContactId, Phase};
use mpfree_touch::pointer::PointerEvent;
use mpfree_touch::route::GestureRouter;
use mpfree_touch::touchlab::{Check, StepRecorder, StepSummary, STEPS};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Touch, Widget};
use std::time::{SystemTime, UNIX_EPOCH};

const COUNTDOWN_US: u64 = 3_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Area,
    Pad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Button {
    Start,
    Skip,
    Redo,
    Next,
    Again,
    Exit,
}

impl Button {
    fn label(self) -> &'static str {
        match self {
            Button::Start => "Start",
            Button::Skip => "Skip",
            Button::Redo => "Redo",
            Button::Next => "Next",
            Button::Again => "Again",
            Button::Exit => "Exit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Ready,
    Countdown(u64),
    Window(u64),
    Result,
    Done,
}

/// What a finger is doing, from its Down to its Up.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Finger {
    Button(Button),
    Area(Owner),
    Ignored,
}

pub struct TouchLabView {
    clock: Box<dyn Fn() -> u64 + Send>,
    config: GestureConfig,
    step: usize,
    stage: Stage,
    recorder: StepRecorder,
    router: GestureRouter<Owner>,
    fingers: Vec<(u64, Finger)>,
    /// Live finger positions in the area, for drawing.
    dots: Vec<(u64, i32, i32)>,
    results: Vec<Option<StepSummary>>,
    last_gesture: String,
    reader: String,
    lines: Vec<String>,
    exit: bool,
    /// The kernel's time for the next finger event, if the caller has it.
    event_time: Option<u64>,
}

impl TouchLabView {
    /// The clock is wall-clock time, the clock evdev stamps events with unless a
    /// program changes it (EVIOCSCLOCKID, which nothing here sends), so finger
    /// events can carry the kernel's own times (`set_event_time`).
    pub fn new() -> TouchLabView {
        TouchLabView::with_clock(Box::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_micros() as u64)
                .unwrap_or(0)
        }))
    }

    /// `clock`: microseconds on any steady clock (tests pass their own).
    pub fn with_clock(clock: Box<dyn Fn() -> u64 + Send>) -> TouchLabView {
        let config = GestureConfig::default();
        TouchLabView {
            clock,
            config,
            step: 0,
            stage: Stage::Ready,
            recorder: StepRecorder::new(&STEPS[0], config.swipe_min_speed),
            router: GestureRouter::new(),
            fingers: Vec::new(),
            dots: Vec::new(),
            results: vec![None; STEPS.len()],
            last_gesture: String::new(),
            reader: "touch-reader: (no reader line yet)".to_string(),
            lines: Vec::new(),
            exit: false,
            event_time: None,
        }
    }

    /// The gesture thresholds the lab's steps use (from the next step).
    pub fn set_config(&mut self, c: GestureConfig) {
        self.config = c;
    }

    /// Start over at the first step (each time the lab is opened).
    pub fn reset(&mut self) {
        self.go_to(0);
        self.results = vec![None; STEPS.len()];
        self.exit = false;
        self.lines.push("touch-lab-open: steps=9".to_string());
    }

    /// The latest `touch-reader:` line, shown at the top of the page.
    pub fn set_reader(&mut self, line: &str) {
        self.reader = line.to_string();
    }

    /// Journal lines queued since the last call.
    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    /// True once Exit was pressed (cleared by `reset`).
    pub fn wants_exit(&self) -> bool {
        self.exit
    }

    /// True if journal lines are waiting for `take_lines`.
    pub fn has_lines(&self) -> bool {
        !self.lines.is_empty()
    }

    /// The kernel's timestamp (microseconds, wall clock) of the finger event
    /// about to be passed to `touch`. Used when it is within a second of this
    /// page's clock; otherwise the page's clock is used.
    pub fn set_event_time(&mut self, time_us: u64) {
        self.event_time = Some(time_us);
    }

    /// Leave the lab: a step in progress is closed and logged, then
    /// `touch-lab-exit`. Again does nothing until `reset`.
    pub fn leave(&mut self) {
        if self.exit {
            return;
        }
        if let Stage::Window(_) = self.stage {
            self.close_window();
        }
        self.stage = match self.stage {
            Stage::Countdown(_) => Stage::Ready,
            s => s,
        };
        self.lines.push("touch-lab-exit".to_string());
        self.exit = true;
    }

    /// Advance the countdown and window, and fire due long presses. Call every
    /// frame. Returns true when something on screen changed.
    pub fn tick(&mut self) -> bool {
        let now = (self.clock)();
        match self.stage {
            Stage::Countdown(t0) if now >= t0 + COUNTDOWN_US => {
                self.open_window(now);
                true
            }
            Stage::Countdown(_) => true,
            Stage::Window(t0) => {
                let mut out = Vec::new();
                self.router.tick(now, &mut out);
                self.record_gestures(&out, now);
                if now >= t0 + u64::from(STEPS[self.step].seconds) * 1_000_000 {
                    self.close_window();
                }
                true
            }
            _ => false,
        }
    }

    fn go_to(&mut self, step: usize) {
        self.step = step.min(STEPS.len() - 1);
        self.stage = if step >= STEPS.len() {
            Stage::Done
        } else {
            Stage::Ready
        };
        self.recorder = StepRecorder::new(&STEPS[self.step], self.config.swipe_min_speed);
        self.last_gesture.clear();
    }

    fn open_window(&mut self, now: u64) {
        let step = &STEPS[self.step];
        self.recorder = StepRecorder::new(step, self.config.swipe_min_speed);
        self.router.clear();
        self.router.register(Owner::Area, self.config);
        // Fingers already down keep out of the window: their Down was missed.
        for (_, f) in &mut self.fingers {
            if matches!(f, Finger::Area(_)) {
                *f = Finger::Ignored;
            }
        }
        self.last_gesture.clear();
        self.lines.push(format!(
            "touch-lab-step: step={} seconds={}",
            step.name, step.seconds
        ));
        self.stage = Stage::Window(now);
    }

    fn close_window(&mut self) {
        let summary = self.recorder.finish();
        self.lines.push(summary.journal_line());
        self.results[self.step] = Some(summary);
        self.router.clear();
        for (_, f) in &mut self.fingers {
            if matches!(f, Finger::Area(_)) {
                *f = Finger::Ignored;
            }
        }
        self.dots.clear();
        self.stage = Stage::Result;
    }

    fn record_gestures(&mut self, gestures: &[(Owner, Gesture)], now: u64) {
        for (_, g) in gestures {
            if let Some(line) = self.recorder.gesture(g, now) {
                self.last_gesture = describe(g);
                self.lines.push(line);
            }
        }
    }

    fn press(&mut self, b: Button) {
        let now = (self.clock)();
        match b {
            Button::Start | Button::Redo => self.stage = Stage::Countdown(now),
            Button::Skip | Button::Next => self.go_to(self.step + 1),
            Button::Again => self.reset(),
            Button::Exit => self.leave(),
        }
        if self.step + 1 >= STEPS.len()
            && matches!(b, Button::Next | Button::Skip)
            && self.stage == Stage::Ready
            && self.results[self.step].is_some()
        {
            self.stage = Stage::Done;
        }
    }

    // ------------------------------------------------------------ layout

    fn regions(r: Rect) -> (Rect, Rect, Rect, Rect) {
        let (top, rest) = r.take_top(96);
        let (bottom, rest) = rest.take_bottom(100);
        let (instr, area) = rest.take_top(56);
        (top, instr, area.inset(12), bottom)
    }

    fn buttons(&self, r: Rect) -> Vec<(Button, Rect)> {
        let (top, _, _, bottom) = Self::regions(r);
        let mut v = vec![(
            Button::Exit,
            Rect::new(top.right() - 150, top.y + 14, 130, 48),
        )];
        let row: &[Button] = match self.stage {
            Stage::Ready => &[Button::Start, Button::Skip],
            Stage::Result => &[Button::Redo, Button::Next],
            Stage::Done => &[Button::Again],
            Stage::Countdown(_) | Stage::Window(_) => &[],
        };
        let w = 220;
        let total = row.len() as i32 * w + (row.len() as i32 - 1).max(0) * 24;
        let mut x = bottom.x + (bottom.w - total) / 2;
        for &b in row {
            v.push((b, Rect::new(x, bottom.y + 20, w, 64)));
            x += w + 24;
        }
        v
    }

    /// Where the pad sits in the pad-and-swipe step.
    fn pad_rect(area: Rect) -> Rect {
        Rect::new(area.x + 40, area.y + area.h / 2 - 140, 280, 280)
    }

    /// Tap targets, dots and the like for the current step, as circles
    /// (centre x, centre y, radius).
    fn targets(&self, area: Rect) -> Vec<(i32, i32, i32)> {
        let (cx, cy) = (area.x + area.w / 2, area.y + area.h / 2);
        match STEPS[self.step].name {
            "taps" => (0..5)
                .map(|i| {
                    (
                        area.x + area.w * (2 * i + 1) / 10,
                        cy + if i % 2 == 0 { -80 } else { 80 },
                        40,
                    )
                })
                .collect(),
            "long_press" => vec![(cx, cy, 70)],
            "hold_centre" => vec![(cx, cy, 16)],
            "hold_corner" => vec![(area.x + 60, area.y + 60, 16)],
            _ => Vec::new(),
        }
    }
}

/// A full-screen touch page the pad player opens over its own pages (the
/// Touch lab, Calibrate touch). While one is open it gets every finger.
pub trait TouchPage: Widget {
    /// Start over, each time the page is opened (queues its open line).
    fn open(&mut self);
    /// Leave: close anything in progress and queue the exit line. Again does
    /// nothing until `open`.
    fn leave(&mut self);
    /// True once the page's own Exit was pressed.
    fn wants_exit(&self) -> bool;
    fn has_lines(&self) -> bool;
    /// Journal lines queued since the last call.
    fn take_lines(&mut self) -> Vec<String>;
    /// Advance timers; call every frame while `animating`.
    fn tick(&mut self) {}
    /// The kernel's timestamp of the finger event about to come.
    fn set_event_time(&mut self, _time_us: u64) {}
}

impl TouchPage for TouchLabView {
    fn open(&mut self) {
        TouchLabView::reset(self);
    }

    fn leave(&mut self) {
        TouchLabView::leave(self);
    }

    fn wants_exit(&self) -> bool {
        TouchLabView::wants_exit(self)
    }

    fn has_lines(&self) -> bool {
        TouchLabView::has_lines(self)
    }

    fn take_lines(&mut self) -> Vec<String> {
        TouchLabView::take_lines(self)
    }

    fn tick(&mut self) {
        TouchLabView::tick(self);
    }

    fn set_event_time(&mut self, time_us: u64) {
        TouchLabView::set_event_time(self, time_us);
    }
}

impl Default for TouchLabView {
    fn default() -> Self {
        TouchLabView::new()
    }
}

pub(crate) fn describe(g: &Gesture) -> String {
    match *g {
        Gesture::Tap { x, y, .. } => format!("tap at {x:.0}, {y:.0}"),
        Gesture::LongPress { .. } => "long press".to_string(),
        Gesture::DragStart { .. } => "drag".to_string(),
        Gesture::DragEnd { vx, vy, .. } => format!("lift at {:.0} px/s", vx.hypot(vy)),
        Gesture::Swipe {
            direction, speed, ..
        } => {
            let d = match direction {
                SwipeDirection::Left => "left",
                SwipeDirection::Right => "right",
                SwipeDirection::Up => "up",
                SwipeDirection::Down => "down",
            };
            format!("swipe {d} at {speed:.0} px/s")
        }
        Gesture::TwoFingerStart { .. } => "two fingers".to_string(),
        _ => String::new(),
    }
}

fn circle(cx: &mut PaintCx, x: i32, y: i32, radius: i32, c: Color) {
    cx.canvas.fill_round_rect(
        Rect::new(x - radius, y - radius, 2 * radius, 2 * radius),
        radius,
        c,
    );
}

impl Widget for TouchLabView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (top, instr, area, _) = Self::regions(r);
        let step = &STEPS[self.step];
        // Top: title, step, reader line.
        let (line1, line2) = top.inset_xy(20, 10).take_top(48);
        cx.text(
            FontId::Bold,
            t.size_title,
            line1,
            Align::Left,
            t.text,
            "Touch lab",
        );
        let label = match self.stage {
            Stage::Done => "All steps done".to_string(),
            _ => format!("Step {} of {}: {}", self.step + 1, STEPS.len(), step.name),
        };
        cx.text(
            FontId::Sans,
            t.size_body,
            line1,
            Align::Center,
            t.text,
            &label,
        );
        cx.text(
            FontId::Mono,
            t.size_small,
            line2,
            Align::Left,
            t.text_dim,
            &self.reader,
        );
        // Instruction or status.
        cx.panel(instr);
        let now = (self.clock)();
        let (msg, color) = match self.stage {
            Stage::Ready => (
                format!("{}  Press Start when ready.", step.instruction),
                t.text,
            ),
            Stage::Countdown(t0) => {
                let left = (t0 + COUNTDOWN_US).saturating_sub(now) / 1_000_000 + 1;
                (format!("Get ready: {left}"), t.warn)
            }
            Stage::Window(t0) => {
                let end = t0 + u64::from(step.seconds) * 1_000_000;
                let left = end.saturating_sub(now) / 1_000_000 + 1;
                (format!("GO ({left} s): {}", step.instruction), t.accent)
            }
            Stage::Result => match &self.results[self.step] {
                Some(s) if s.pass => (format!("PASS. {}", counts(s)), t.good),
                Some(s) => (
                    format!(
                        "Not as expected ({}). {}",
                        s.note.replace('_', " "),
                        counts(s)
                    ),
                    t.warn,
                ),
                None => (String::new(), t.text),
            },
            Stage::Done => {
                let pass = self.results.iter().flatten().filter(|s| s.pass).count();
                let run = self.results.iter().flatten().count();
                (
                    format!("{pass} of {run} steps as expected. Results are in the system log."),
                    t.text,
                )
            }
        };
        cx.text(
            FontId::Sans,
            t.size_body,
            instr.inset_xy(16, 0),
            Align::Left,
            color,
            &msg,
        );
        // Work area.
        cx.canvas.stroke_round_rect(area, t.radius, 2, t.outline);
        if self.stage == Stage::Done {
            let mut row = area.inset(24);
            for (s, result) in STEPS.iter().zip(&self.results) {
                let (line, rest) = row.take_top(40);
                let (text, c) = match result {
                    Some(r) if r.pass => (format!("{:<14} pass", s.name), t.good),
                    Some(r) => (
                        format!("{:<14} {}", s.name, r.note.replace('_', " ")),
                        t.warn,
                    ),
                    None => (format!("{:<14} skipped", s.name), t.text_dim),
                };
                cx.text(FontId::Mono, t.size_body, line, Align::Left, c, &text);
                row = rest;
            }
        } else {
            if step.check == Check::PadAndSwipe {
                let pad = Self::pad_rect(area);
                cx.canvas.fill_round_rect(pad, t.radius, t.raised);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    pad,
                    Align::Center,
                    t.text,
                    "Hold this pad",
                );
                let right = Rect::new(
                    pad.right() + 40,
                    area.y,
                    area.right() - pad.right() - 40,
                    area.h,
                );
                cx.text(
                    FontId::Sans,
                    t.size_body,
                    right,
                    Align::Center,
                    t.text_dim,
                    "Swipe here",
                );
            }
            if step.check == Check::Flings(2) {
                let mut y = area.y + 20;
                while y + 50 < area.bottom() {
                    cx.canvas.fill_round_rect(
                        Rect::new(area.x + area.w / 4, y, area.w / 2, 44),
                        8,
                        t.surface,
                    );
                    y += 56;
                }
            }
            for (x, y, radius) in self.targets(area) {
                circle(cx, x, y, radius, t.accent);
            }
            for &(_, x, y) in &self.dots {
                circle(cx, x, y, 22, t.good);
            }
            if !self.last_gesture.is_empty() {
                let (_, foot) = area.take_bottom(40);
                cx.text(
                    FontId::Mono,
                    t.size_body,
                    foot.inset_xy(16, 0),
                    Align::Left,
                    t.text_dim,
                    &self.last_gesture,
                );
            }
        }
        for (b, br) in self.buttons(r) {
            let fill = if b == Button::Exit {
                t.surface
            } else {
                t.raised
            };
            cx.canvas.fill_round_rect(br, t.radius, fill);
            cx.canvas.stroke_round_rect(br, t.radius, 2, t.outline);
            cx.text(
                FontId::Bold,
                t.size_body,
                br,
                Align::Center,
                t.text,
                b.label(),
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<mpfree_ui::Target> {
        use mpfree_ui::Target;
        let mut out: Vec<Target> = self
            .buttons(r)
            .into_iter()
            .enumerate()
            .map(|(i, (b, br))| Target::tap("button", i, b.label(), br))
            .collect();
        if matches!(self.stage, Stage::Window(_)) {
            let (_, _, area, _) = Self::regions(r);
            out.push(Target::drag("area", 0, "touch area", area));
        }
        out
    }

    fn touch(
        &mut self,
        finger: u64,
        phase: Touch,
        x: i32,
        y: i32,
        r: Rect,
    ) -> Option<mpfree_ui::EventKind> {
        let clock = (self.clock)();
        let now = match self.event_time.take() {
            Some(t) if t.abs_diff(clock) < 1_000_000 => t,
            _ => clock,
        };
        let (_, _, area, _) = Self::regions(r);
        if phase == Touch::Down {
            let what = if let Some((b, _)) = self
                .buttons(r)
                .into_iter()
                .find(|(_, br)| br.contains(x, y))
            {
                Finger::Button(b)
            } else if matches!(self.stage, Stage::Window(_)) && area.contains(x, y) {
                let pad = STEPS[self.step].check == Check::PadAndSwipe
                    && Self::pad_rect(area).contains(x, y);
                Finger::Area(if pad { Owner::Pad } else { Owner::Area })
            } else {
                Finger::Ignored
            };
            self.fingers.retain(|(f, _)| *f != finger);
            self.fingers.push((finger, what));
        }
        let what = self
            .fingers
            .iter()
            .find(|(f, _)| *f == finger)
            .map(|(_, w)| *w)?;
        if matches!(phase, Touch::Up | Touch::Cancel) {
            self.fingers.retain(|(f, _)| *f != finger);
        }
        match what {
            Finger::Button(b) => {
                if phase == Touch::Up
                    && self
                        .buttons(r)
                        .iter()
                        .any(|(bb, br)| *bb == b && br.contains(x, y))
                {
                    self.press(b);
                }
            }
            Finger::Area(owner) => {
                let ev = PointerEvent {
                    id: ContactId(finger),
                    phase: match phase {
                        Touch::Down => Phase::Down,
                        Touch::Move => Phase::Move,
                        Touch::Up => Phase::Up,
                        Touch::Cancel => Phase::Cancel,
                    },
                    x: x as f32,
                    y: y as f32,
                    time_us: now,
                };
                self.recorder.pointer(&ev);
                let mut out = Vec::new();
                self.router.frame(&[(owner, ev)], &mut out);
                self.record_gestures(&out, now);
                self.dots.retain(|(f, _, _)| *f != finger);
                if matches!(phase, Touch::Down | Touch::Move) {
                    self.dots.push((finger, x, y));
                }
            }
            Finger::Ignored => {}
        }
        None
    }

    fn animating(&self) -> bool {
        matches!(self.stage, Stage::Countdown(_) | Stage::Window(_))
    }
}

fn counts(s: &StepSummary) -> String {
    let w = s.swipes;
    format!(
        "taps {}, long {}, drags {}, swipes L{} R{} U{} D{}, flings {}",
        s.taps, s.long, s.drags, w[0], w[1], w[2], w[3], s.flings
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    const SCREEN: Rect = Rect::new(0, 0, 1280, 800);

    fn lab() -> (TouchLabView, Arc<AtomicU64>) {
        let t = Arc::new(AtomicU64::new(1_000_000));
        let c = t.clone();
        let mut lab = TouchLabView::with_clock(Box::new(move || c.load(Ordering::SeqCst)));
        lab.reset();
        (lab, t)
    }

    fn tap_button(lab: &mut TouchLabView, b: Button, finger: u64) {
        let (_, r) = *lab
            .buttons(SCREEN)
            .iter()
            .find(|(bb, _)| *bb == b)
            .expect("button shown");
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        lab.touch(finger, Touch::Down, x, y, SCREEN);
        lab.touch(finger, Touch::Up, x, y, SCREEN);
    }

    fn advance(lab: &mut TouchLabView, t: &AtomicU64, us: u64) {
        t.fetch_add(us, Ordering::SeqCst);
        lab.tick();
    }

    #[test]
    fn a_step_runs_from_start_to_result_and_logs_its_line() {
        let (mut lab, t) = lab();
        assert_eq!(lab.take_lines(), vec!["touch-lab-open: steps=9"]);
        // Step 1, reader: nothing touched, so it passes.
        tap_button(&mut lab, Button::Start, 1);
        assert!(matches!(lab.stage, Stage::Countdown(_)));
        advance(&mut lab, &t, 3_100_000);
        assert!(matches!(lab.stage, Stage::Window(_)));
        advance(&mut lab, &t, 3_100_000);
        assert_eq!(lab.stage, Stage::Result);
        let lines = lab.take_lines();
        assert_eq!(lines[0], "touch-lab-step: step=reader seconds=3");
        assert!(
            lines[1].starts_with("touch-lab: step=reader result=pass "),
            "{lines:?}"
        );
        // The Start tap itself was not recorded.
        assert_eq!(lab.results[0].unwrap().downs, 0);
        tap_button(&mut lab, Button::Next, 2);
        assert_eq!((lab.step, lab.stage), (1, Stage::Ready));
    }

    #[test]
    fn taps_in_the_window_are_recorded_and_gestures_logged() {
        let (mut lab, t) = lab();
        tap_button(&mut lab, Button::Skip, 1);
        tap_button(&mut lab, Button::Start, 2);
        advance(&mut lab, &t, 3_100_000);
        let (_, _, area, _) = TouchLabView::regions(SCREEN);
        for (i, (x, y, _)) in lab.targets(area).into_iter().enumerate() {
            let f = 10 + i as u64;
            lab.touch(f, Touch::Down, x, y, SCREEN);
            t.fetch_add(80_000, Ordering::SeqCst);
            lab.touch(f, Touch::Up, x, y, SCREEN);
            t.fetch_add(300_000, Ordering::SeqCst);
        }
        advance(&mut lab, &t, 10_000_000);
        let s = lab.results[1].unwrap();
        assert!(s.pass, "{s:?}");
        let lines = lab.take_lines();
        assert_eq!(lines.iter().filter(|l| l.contains("kind=tap")).count(), 5);
    }

    #[test]
    fn a_held_pad_stays_out_of_the_swipe() {
        let (mut lab, t) = lab();
        for _ in 0..8 {
            tap_button(&mut lab, Button::Skip, 1);
        }
        assert_eq!(STEPS[lab.step].name, "pad_and_swipe");
        tap_button(&mut lab, Button::Start, 2);
        advance(&mut lab, &t, 3_100_000);
        let (_, _, area, _) = TouchLabView::regions(SCREEN);
        let pad = TouchLabView::pad_rect(area);
        lab.touch(20, Touch::Down, pad.x + 50, pad.y + 50, SCREEN);
        let (mut x, y) = (area.right() - 100, area.y + area.h / 2);
        lab.touch(21, Touch::Down, x, y, SCREEN);
        for _ in 0..20 {
            t.fetch_add(8_800, Ordering::SeqCst);
            x -= 15;
            lab.touch(21, Touch::Move, x, y, SCREEN);
        }
        t.fetch_add(8_800, Ordering::SeqCst);
        lab.touch(21, Touch::Up, x, y, SCREEN);
        advance(&mut lab, &t, 10_000_000);
        let s = lab.results[8].unwrap();
        assert!(s.pass && s.two_finger == 0 && s.swipes[0] == 1, "{s:?}");
    }

    #[test]
    fn targets_sit_in_the_work_area_and_buttons_outside_it() {
        let (mut lab, _) = lab();
        let (top, instr, area, bottom) = TouchLabView::regions(SCREEN);
        assert!(
            top.bottom() <= instr.y && instr.bottom() <= area.y && area.bottom() <= bottom.y,
            "{area:?}"
        );
        assert!(area.w > 1200 && area.h > 500, "{area:?}");
        for (step, s) in STEPS.iter().enumerate() {
            lab.go_to(step);
            for (x, y, r) in lab.targets(area) {
                assert!(
                    area.contains(x - r, y - r) && area.contains(x + r, y + r),
                    "{} {x},{y}",
                    s.name
                );
            }
            for (b, br) in lab.buttons(SCREEN) {
                assert!(
                    br.intersect(area).is_empty()
                        && SCREEN.contains(br.right() - 1, br.bottom() - 1),
                    "{b:?}"
                );
            }
        }
        assert!(area.contains(
            TouchLabView::pad_rect(area).right(),
            TouchLabView::pad_rect(area).bottom()
        ));
    }

    #[test]
    fn exit_closes_an_open_window_and_says_so() {
        let (mut lab, t) = lab();
        tap_button(&mut lab, Button::Start, 1);
        advance(&mut lab, &t, 3_100_000);
        tap_button(&mut lab, Button::Exit, 2);
        assert!(lab.wants_exit());
        let lines = lab.take_lines();
        assert!(lines
            .iter()
            .any(|l| l.starts_with("touch-lab: step=reader")));
        assert_eq!(lines.last().unwrap(), "touch-lab-exit");
        lab.reset();
        assert!(!lab.wants_exit());
    }

    #[test]
    fn every_stage_draws() {
        let (mut lab, t) = lab();
        let mut canvas = mpfree_ui::SoftCanvas::new(1280, 800);
        let mut fonts = mpfree_ui::Fonts::embedded();
        let theme = mpfree_ui::Theme::dark();
        let mut draw = |lab: &TouchLabView| {
            let mut cx = PaintCx {
                canvas: &mut canvas,
                theme: &theme,
                fonts: &mut fonts,
            };
            lab.paint(&mut cx, SCREEN);
        };
        for step in 0..STEPS.len() {
            draw(&lab);
            tap_button(&mut lab, Button::Start, 1);
            draw(&lab);
            advance(&mut lab, &t, 3_100_000);
            draw(&lab);
            advance(&mut lab, &t, 20_000_000);
            draw(&lab);
            assert_eq!(lab.stage, Stage::Result, "step {step}");
            tap_button(&mut lab, Button::Next, 2);
        }
        assert_eq!(lab.stage, Stage::Done);
        draw(&lab);
    }
}
