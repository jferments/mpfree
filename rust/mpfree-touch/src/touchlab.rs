//! M15: the Touch lab. Owner-paced touch steps for the GUI, with a summary
//! per step that goes to the journal for the host-side check.
//!
//! This module is pure logic. The GUI shows [`STEPS`] one at a time (the host
//! starts each with Enter), feeds what the reader and the gesture router
//! produce into a [`StepRecorder`], and at the end of the window logs
//! [`StepSummary::journal_line`]. Every gesture is also logged as it happens
//! with [`gesture_line`], so the full decision trail is in the journal.
//!
//! Journal lines (one per line, plain ASCII, `key=value` separated by single
//! spaces, never containing spaces inside a value):
//!
//! - `touch-reader: seeded=1 slot=3 fingers=0 resyncs=0 fallbacks=0 drained=0 contacts=12 events=480`
//!   (`events` counts raw input events read, so a zero there with touches
//!   tried points below the reader, at the driver)
//! - `touch-gesture: step=swipes kind=swipe dir=left speed=1180 t_ms=4521`
//!   (kinds: tap, long, drag-start, drag-end, swipe, 2f-start, 2f-end;
//!   drag-end adds `vx`, `vy`, `cancelled`; tap and long add `x`, `y`)
//! - `touch-lab: step=swipes result=pass downs=4 taps=0 long=0 drags=4 swipes=1,1,1,1 two_finger=0 flings=0 note=ok`
//!   (`swipes` is left,right,up,down; holds add `hold_n`, `hold_hz`,
//!   `hold_std_x`, `hold_std_y`, `hold_max_dev`, all in logical px)
//!
//! [`parse_line`] reads any of them back into key/value pairs, for tests and
//! for workstation analysis of a results zip.

use crate::gesture::{Gesture, SwipeDirection};
use crate::jitter::{hold_stats, HoldStats};
use crate::mt::{ContactId, Phase};
use crate::pointer::PointerEvent;

/// What a step checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// No touches; the reader's state is what matters.
    Idle,
    Taps(u32),
    LongPresses(u32),
    /// Drags that end slowly: no swipes allowed.
    DragsWithoutSwipe(u32),
    /// One swipe in each of the four directions.
    FourSwipes,
    /// At least this many releases fast enough to fling (>= the swipe speed).
    Flings(u32),
    /// One finger held still for most of the window.
    Hold,
    /// A held pad plus a swipe elsewhere: at least one swipe, no two-finger
    /// gesture.
    PadAndSwipe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabStep {
    pub name: &'static str,
    /// Shown on screen and in the host runbook.
    pub instruction: &'static str,
    /// Window length after the 3-2-1 countdown.
    pub seconds: u32,
    pub check: Check,
}

/// The lab's steps, in order. Names are stable: the host check and the
/// runbook use them.
pub const STEPS: [LabStep; 9] = [
    LabStep {
        name: "reader",
        seconds: 3,
        check: Check::Idle,
        instruction: "Don't touch the screen.",
    },
    LabStep {
        name: "taps",
        seconds: 10,
        check: Check::Taps(5),
        instruction: "Tap each of the 5 targets once, one at a time.",
    },
    LabStep {
        name: "long_press",
        seconds: 8,
        check: Check::LongPresses(2),
        instruction: "Press and hold the target for 2 seconds, then lift. Do it twice.",
    },
    LabStep {
        name: "drag_stop",
        seconds: 10,
        check: Check::DragsWithoutSwipe(2),
        instruction: "Drag slowly across the screen, stop for a second, then lift. Do it twice.",
    },
    LabStep {
        name: "swipes",
        seconds: 12,
        check: Check::FourSwipes,
        instruction: "Swipe quickly once each way: left, right, up, down.",
    },
    LabStep {
        name: "fling",
        seconds: 10,
        check: Check::Flings(2),
        instruction: "Flick the list up and down a few times.",
    },
    LabStep {
        name: "hold_centre",
        seconds: 12,
        check: Check::Hold,
        instruction: "Hold one finger still on the centre dot for 10 seconds, then lift.",
    },
    LabStep {
        name: "hold_corner",
        seconds: 12,
        check: Check::Hold,
        instruction: "Hold one finger still on the top-left dot for 10 seconds, then lift.",
    },
    LabStep {
        name: "pad_and_swipe",
        seconds: 10,
        check: Check::PadAndSwipe,
        instruction: "Hold a screen pad with one hand and swipe the page with the other.",
    },
];

pub fn step(name: &str) -> Option<&'static LabStep> {
    STEPS.iter().find(|s| s.name == name)
}

/// Collects one step's touches and gestures.
#[derive(Debug, Clone)]
pub struct StepRecorder {
    step: &'static LabStep,
    start_us: Option<u64>,
    downs: u32,
    taps: u32,
    long: u32,
    drags: u32,
    swipes: [u32; 4],
    two_finger: u32,
    flings: u32,
    fling_speed: f32,
    /// Down/Move reports per contact, for holds.
    tracks: Vec<(ContactId, Vec<PointerEvent>)>,
}

impl StepRecorder {
    /// `fling_speed` should be the swipe speed of the gesture config in use.
    pub fn new(step: &'static LabStep, fling_speed: f32) -> Self {
        Self {
            step,
            start_us: None,
            downs: 0,
            taps: 0,
            long: 0,
            drags: 0,
            swipes: [0; 4],
            two_finger: 0,
            flings: 0,
            fling_speed,
            tracks: Vec::new(),
        }
    }

    pub fn step(&self) -> &'static LabStep {
        self.step
    }

    pub fn pointer(&mut self, ev: &PointerEvent) {
        self.start_us.get_or_insert(ev.time_us);
        match ev.phase {
            Phase::Down => {
                self.downs += 1;
                self.tracks.push((ev.id, vec![*ev]));
            }
            Phase::Move => {
                if let Some((_, v)) = self.tracks.iter_mut().find(|(id, _)| *id == ev.id) {
                    v.push(*ev);
                }
            }
            Phase::Up | Phase::Cancel => {}
        }
    }

    /// Record a gesture; `now_us` is the event clock. Returns the journal line
    /// to log for it (`None` for continuous updates: Drag, TwoFinger).
    pub fn gesture(&mut self, g: &Gesture, now_us: u64) -> Option<String> {
        let t_ms = now_us.saturating_sub(*self.start_us.get_or_insert(now_us)) / 1000;
        let head = format!("touch-gesture: step={}", self.step.name);
        let line = match *g {
            Gesture::Tap { x, y, .. } => {
                self.taps += 1;
                format!("{head} kind=tap x={x:.0} y={y:.0} t_ms={t_ms}")
            }
            Gesture::LongPress { x, y, .. } => {
                self.long += 1;
                format!("{head} kind=long x={x:.0} y={y:.0} t_ms={t_ms}")
            }
            Gesture::DragStart { .. } => {
                self.drags += 1;
                format!("{head} kind=drag-start t_ms={t_ms}")
            }
            Gesture::Drag { .. } => return None,
            Gesture::DragEnd {
                vx, vy, cancelled, ..
            } => {
                if !cancelled && vx.hypot(vy) >= self.fling_speed {
                    self.flings += 1;
                }
                format!(
                    "{head} kind=drag-end vx={vx:.0} vy={vy:.0} cancelled={} t_ms={t_ms}",
                    u8::from(cancelled)
                )
            }
            Gesture::Swipe {
                direction, speed, ..
            } => {
                let (i, name) = match direction {
                    SwipeDirection::Left => (0, "left"),
                    SwipeDirection::Right => (1, "right"),
                    SwipeDirection::Up => (2, "up"),
                    SwipeDirection::Down => (3, "down"),
                };
                self.swipes[i] += 1;
                format!("{head} kind=swipe dir={name} speed={speed:.0} t_ms={t_ms}")
            }
            Gesture::TwoFingerStart { .. } => {
                self.two_finger += 1;
                format!("{head} kind=2f-start t_ms={t_ms}")
            }
            Gesture::TwoFinger { .. } => return None,
            Gesture::TwoFingerEnd { cancelled } => format!(
                "{head} kind=2f-end cancelled={} t_ms={t_ms}",
                u8::from(cancelled)
            ),
        };
        Some(line)
    }

    /// End the window and judge the step.
    pub fn finish(&self) -> StepSummary {
        let hold = self
            .tracks
            .iter()
            .max_by_key(|(_, v)| {
                let first = v.first().map_or(0, |e| e.time_us);
                let last = v.last().map_or(0, |e| e.time_us);
                last - first
            })
            .and_then(|(_, v)| hold_stats(v));
        let total_swipes: u32 = self.swipes.iter().sum();
        let (pass, note) = match self.step.check {
            Check::Idle => (
                self.downs == 0,
                if self.downs == 0 { "ok" } else { "touched" },
            ),
            Check::Taps(n) => judge(self.taps >= n && self.drags == 0, "ok", "taps_or_drags"),
            Check::LongPresses(n) => judge(self.long >= n, "ok", "too_few_long_presses"),
            Check::DragsWithoutSwipe(n) => judge(
                self.drags >= n && total_swipes == 0,
                "ok",
                "swipe_or_too_few_drags",
            ),
            Check::FourSwipes => judge(
                self.swipes.iter().all(|&c| c >= 1),
                "ok",
                "missing_direction",
            ),
            Check::Flings(n) => judge(self.flings >= n, "ok", "too_few_flings"),
            Check::Hold => {
                let long_enough = hold.is_some_and(|h| h.duration_us >= 5_000_000);
                judge(long_enough, "ok", "no_hold_of_5s")
            }
            Check::PadAndSwipe => judge(
                total_swipes >= 1 && self.two_finger == 0,
                "ok",
                "no_swipe_or_paired",
            ),
        };
        StepSummary {
            step: self.step.name,
            pass,
            note,
            downs: self.downs,
            taps: self.taps,
            long: self.long,
            drags: self.drags,
            swipes: self.swipes,
            two_finger: self.two_finger,
            flings: self.flings,
            hold: if self.step.check == Check::Hold {
                hold
            } else {
                None
            },
        }
    }
}

fn judge(ok: bool, yes: &'static str, no: &'static str) -> (bool, &'static str) {
    (ok, if ok { yes } else { no })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepSummary {
    pub step: &'static str,
    pub pass: bool,
    pub note: &'static str,
    pub downs: u32,
    pub taps: u32,
    pub long: u32,
    pub drags: u32,
    /// Left, right, up, down.
    pub swipes: [u32; 4],
    pub two_finger: u32,
    pub flings: u32,
    pub hold: Option<HoldStats>,
}

impl StepSummary {
    pub fn journal_line(&self) -> String {
        let s = self.swipes;
        let mut line = format!(
            "touch-lab: step={} result={} downs={} taps={} long={} drags={} swipes={},{},{},{} two_finger={} flings={} note={}",
            self.step,
            if self.pass { "pass" } else { "fail" },
            self.downs, self.taps, self.long, self.drags, s[0], s[1], s[2], s[3],
            self.two_finger, self.flings, self.note,
        );
        if let Some(h) = self.hold {
            line.push_str(&format!(
                " hold_n={} hold_hz={:.1} hold_std_x={:.2} hold_std_y={:.2} hold_max_dev={:.2}",
                h.samples, h.rate_hz, h.std_x, h.std_y, h.max_dev
            ));
        }
        line
    }
}

/// The reader's state, as a journal line.
#[allow(clippy::too_many_arguments)]
pub fn reader_line(
    seeded: bool,
    slot: i32,
    fingers: u32,
    resyncs: u64,
    fallbacks: u64,
    drained: u64,
    contacts: u64,
    events: u64,
) -> String {
    format!(
        "touch-reader: seeded={} slot={slot} fingers={fingers} resyncs={resyncs} fallbacks={fallbacks} drained={drained} contacts={contacts} events={events}",
        u8::from(seeded)
    )
}

/// Split a journal message (`touch-…: k=v k=v`) into its tag and pairs.
/// Leading text before the tag (a timestamp, unit name) is skipped.
pub fn parse_line(line: &str) -> Option<(&str, Vec<(&str, &str)>)> {
    let start = ["touch-lab:", "touch-gesture:", "touch-reader:"]
        .iter()
        .filter_map(|tag| line.find(tag).map(|at| (at, *tag)))
        .min_by_key(|(at, _)| *at)?;
    let rest = &line[start.0 + start.1.len()..];
    let pairs = rest
        .split_whitespace()
        .filter_map(|kv| kv.split_once('='))
        .collect();
    Some((start.1.trim_end_matches(':'), pairs))
}
