//! M15 Touch lab: step judging and journal lines, on synthetic input.

use mpfree_touch::gesture::{GestureConfig, GestureRecognizer};
use mpfree_touch::mt::{ContactId, Phase};
use mpfree_touch::pointer::PointerEvent;
use mpfree_touch::touchlab::{parse_line, reader_line, step, StepRecorder, STEPS};

const DT: u64 = 8_800;

fn ev(id: u64, phase: Phase, x: f32, y: f32, t: u64) -> PointerEvent {
    PointerEvent {
        id: ContactId(id),
        phase,
        x,
        y,
        time_us: t,
    }
}

/// Drive a recorder and a recognizer with pointer frames (one event each).
struct Rig {
    rec: StepRecorder,
    g: GestureRecognizer,
    lines: Vec<String>,
}

impl Rig {
    fn new(name: &str) -> Self {
        let cfg = GestureConfig::default();
        Self {
            rec: StepRecorder::new(step(name).unwrap(), cfg.swipe_min_speed),
            g: GestureRecognizer::new(cfg),
            lines: Vec::new(),
        }
    }
    fn feed(&mut self, e: PointerEvent) {
        self.rec.pointer(&e);
        let mut out = Vec::new();
        self.g.frame(&[e], &mut out);
        for gest in &out {
            self.lines.extend(self.rec.gesture(gest, e.time_us));
        }
    }
    fn tick(&mut self, t: u64) {
        let mut out = Vec::new();
        self.g.tick(t, &mut out);
        for gest in &out {
            self.lines.extend(self.rec.gesture(gest, t));
        }
    }
    /// A stroke of `n` reports moving (dx, dy) each, starting at t0.
    #[allow(clippy::too_many_arguments)]
    fn stroke(
        &mut self,
        id: u64,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        n: u64,
        t0: u64,
        pause: u64,
    ) -> u64 {
        self.feed(ev(id, Phase::Down, x, y, t0));
        for i in 1..=n {
            self.feed(ev(
                id,
                Phase::Move,
                x + dx * i as f32,
                y + dy * i as f32,
                t0 + i * DT,
            ));
        }
        let end = t0 + n * DT + pause + DT;
        self.feed(ev(id, Phase::Up, x + dx * n as f32, y + dy * n as f32, end));
        end
    }
}

#[test]
fn step_names_are_unique_and_windows_short() {
    for (i, s) in STEPS.iter().enumerate() {
        assert!(STEPS[i + 1..].iter().all(|o| o.name != s.name));
        assert!((3..=15).contains(&s.seconds), "{}", s.name);
        assert!(!s.name.contains(' ') && s.instruction.is_ascii());
    }
}

#[test]
fn idle_passes_untouched_and_fails_touched() {
    let r = Rig::new("reader");
    assert!(r.rec.finish().pass);
    let mut r = Rig::new("reader");
    r.stroke(1, 10.0, 10.0, 0.0, 0.0, 1, 0, 0);
    let s = r.rec.finish();
    assert!(!s.pass);
    assert_eq!(s.note, "touched");
}

#[test]
fn five_taps_pass() {
    let mut r = Rig::new("taps");
    let mut t = 0;
    for k in 0..5 {
        t = r.stroke(
            k + 1,
            100.0 + 200.0 * k as f32,
            400.0,
            0.3,
            0.0,
            3,
            t + 400_000,
            0,
        );
    }
    let s = r.rec.finish();
    assert!(s.pass, "{s:?}");
    assert_eq!((s.taps, s.drags, s.downs), (5, 0, 5));
    assert_eq!(r.lines.iter().filter(|l| l.contains("kind=tap")).count(), 5);
}

#[test]
fn long_presses_come_from_tick() {
    let mut r = Rig::new("long_press");
    for k in 0..2u64 {
        let t0 = k * 3_000_000;
        r.feed(ev(k + 1, Phase::Down, 640.0, 400.0, t0));
        r.tick(t0 + 600_000);
        r.feed(ev(k + 1, Phase::Up, 640.0, 400.0, t0 + 2_000_000));
    }
    assert!(r.rec.finish().pass);
}

#[test]
fn slow_drags_that_stop_pass_and_a_swipe_fails_them() {
    let mut r = Rig::new("drag_stop");
    let t = r.stroke(1, 100.0, 400.0, 3.0, 0.0, 100, 0, 1_000_000);
    r.stroke(2, 100.0, 300.0, 3.0, 0.0, 100, t + 500_000, 1_000_000);
    assert!(r.rec.finish().pass, "{:?}", r.rec.finish());
    r.stroke(3, 100.0, 200.0, 12.0, 0.0, 20, 9_000_000, 0);
    let s = r.rec.finish();
    assert!(!s.pass && s.note == "swipe_or_too_few_drags");
}

#[test]
fn four_swipes_pass_and_three_fail() {
    let mut r = Rig::new("swipes");
    let mut t = 0;
    for (dx, dy) in [(-12.0, 0.0), (12.0, 0.0), (0.0, -12.0)] {
        t = r.stroke(1, 640.0, 400.0, dx, dy, 20, t + 500_000, 0);
    }
    assert_eq!(r.rec.finish().note, "missing_direction");
    r.stroke(2, 640.0, 200.0, 0.0, 12.0, 20, t + 500_000, 0);
    let s = r.rec.finish();
    assert!(s.pass);
    assert_eq!(s.swipes, [1, 1, 1, 1]);
    assert!(s.journal_line().contains(" swipes=1,1,1,1 "));
}

#[test]
fn flings_count_fast_releases() {
    let mut r = Rig::new("fling");
    let t = r.stroke(1, 600.0, 700.0, 0.0, -10.0, 20, 0, 0);
    r.stroke(2, 600.0, 100.0, 0.0, 10.0, 20, t + 300_000, 0);
    let s = r.rec.finish();
    assert!(s.pass && s.flings == 2, "{s:?}");
}

#[test]
fn a_hold_reports_jitter_in_its_line() {
    let mut r = Rig::new("hold_centre");
    r.feed(ev(1, Phase::Down, 640.0, 400.0, 0));
    for i in 1..40u64 {
        let d = if i % 2 == 0 { 1.0 } else { -1.0 };
        r.feed(ev(1, Phase::Move, 640.0 + d, 400.0, i * 250_000));
    }
    r.feed(ev(1, Phase::Up, 640.0, 400.0, 10_000_000));
    let s = r.rec.finish();
    assert!(s.pass);
    let h = s.hold.unwrap();
    assert_eq!(h.samples, 40);
    let line = s.journal_line();
    assert!(
        line.contains(" hold_n=40 ") && line.contains(" hold_hz=4.0 "),
        "{line}"
    );
    // A short hold fails.
    let mut r = Rig::new("hold_corner");
    r.stroke(1, 50.0, 50.0, 0.1, 0.0, 10, 0, 1_000_000);
    assert_eq!(r.rec.finish().note, "no_hold_of_5s");
}

#[test]
fn journal_lines_parse_back() {
    let mut r = Rig::new("swipes");
    r.stroke(1, 640.0, 400.0, -12.0, 0.0, 20, 0, 0);
    let swipe = r.lines.iter().find(|l| l.contains("kind=swipe")).unwrap();
    let journal = format!("[  123.456] mpfree-hello[201]: {swipe}");
    let (tag, kv) = parse_line(&journal).unwrap();
    assert_eq!(tag, "touch-gesture");
    assert!(kv.contains(&("step", "swipes")) && kv.contains(&("dir", "left")));
    let lab = r.rec.finish().journal_line();
    let (tag, kv) = parse_line(&lab).unwrap();
    assert_eq!(tag, "touch-lab");
    assert!(kv.contains(&("result", "fail")) && kv.contains(&("note", "missing_direction")));
    let line = reader_line(true, 3, 0, 1, 0, 5, 12, 480);
    assert_eq!(line, "touch-reader: seeded=1 slot=3 fingers=0 resyncs=1 fallbacks=0 drained=5 contacts=12 events=480");
    assert_eq!(parse_line(&line).unwrap().1.len(), 8);
    assert!(parse_line("status: hits=3").is_none());
}
