//! M11: toolkit-neutral gesture recognition over [`PointerEvent`]s.
//!
//! Deliberately conservative so it never steals simultaneous independent
//! touches from a music UI:
//! - A contact that has started dragging or long-pressing is never pulled
//!   into a two-finger gesture.
//! - A two-finger gesture starts only when exactly two contacts are down and
//!   both are still undecided (inside tap slop, before long-press).
//! - Cancelled contacts never produce Tap; open drags/two-finger gestures end
//!   with `cancelled: true`.
//!
//! "Scroll" is not a separate recognizer: a scrollable widget interprets
//! `Drag` (one finger) or `TwoFinger` pan deltas as scrolling.
//!
//! Time comes only from event timestamps and [`GestureRecognizer::tick`], so
//! behaviour is deterministic under test. Input is one decoder frame at a
//! time ([`GestureRecognizer::frame`]).

use crate::mt::{ContactId, Phase};
use crate::pointer::PointerEvent;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureConfig {
    /// Movement (logical px) a contact may make and still count as a tap.
    pub slop_px: f32,
    /// Longest press that still counts as a tap.
    pub tap_max_us: u64,
    /// Hold time before a stationary contact becomes a long-press.
    pub long_press_us: u64,
    pub two_finger: bool,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self { slop_px: 12.0, tap_max_us: 300_000, long_press_us: 500_000, two_finger: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    Tap { id: ContactId, x: f32, y: f32 },
    LongPress { id: ContactId, x: f32, y: f32 },
    DragStart { id: ContactId, x: f32, y: f32 },
    /// `dx`/`dy` are since the previous Drag (or DragStart).
    Drag { id: ContactId, x: f32, y: f32, dx: f32, dy: f32 },
    DragEnd { id: ContactId, x: f32, y: f32, cancelled: bool },
    TwoFingerStart { cx: f32, cy: f32 },
    /// Centroid pan since the previous update, and spread relative to the
    /// spread when the gesture started (pinch: <1 in, >1 out).
    TwoFinger { cx: f32, cy: f32, dx: f32, dy: f32, scale: f32 },
    TwoFingerEnd { cancelled: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Pending,
    LongPressed,
    Dragging,
    TwoFinger,
    /// Partner of a finished two-finger gesture; ignored until it lifts.
    Spent,
}

#[derive(Debug, Clone, Copy)]
struct Track {
    id: ContactId,
    state: State,
    start_x: f32,
    start_y: f32,
    x: f32,
    y: f32,
    down_us: u64,
}

#[derive(Debug, Clone, Copy)]
struct Pair {
    a: ContactId,
    b: ContactId,
    start_spread: f32,
    cx: f32,
    cy: f32,
}

#[derive(Debug, Clone)]
pub struct GestureRecognizer {
    config: GestureConfig,
    tracks: Vec<Track>,
    pair: Option<Pair>,
}

impl GestureRecognizer {
    pub fn new(config: GestureConfig) -> Self {
        Self { config, tracks: Vec::with_capacity(crate::mt::MAX_SLOTS), pair: None }
    }

    /// Fire due long-presses. Call from the UI loop; events also call it.
    pub fn tick(&mut self, now_us: u64, out: &mut Vec<Gesture>) {
        for t in &mut self.tracks {
            if t.state == State::Pending
                && now_us.saturating_sub(t.down_us) >= self.config.long_press_us
            {
                t.state = State::LongPressed;
                out.push(Gesture::LongPress { id: t.id, x: t.x, y: t.y });
            }
        }
    }

    /// Feed one committed decoder frame (all of its events, in order). The
    /// frame is the unit of coherence: when both fingers of a two-finger
    /// gesture move in one frame, exactly one `TwoFinger` update is emitted.
    pub fn frame(&mut self, events: &[PointerEvent], out: &mut Vec<Gesture>) {
        let Some(first) = events.first() else { return };
        self.tick(first.time_us, out);
        let mut pair_moved = false;
        for ev in events {
            match ev.phase {
                Phase::Down => self.down(ev, out),
                Phase::Move => pair_moved |= self.moved(ev, out),
                Phase::Up | Phase::Cancel => self.ended(ev, ev.phase == Phase::Cancel, out),
            }
        }
        if pair_moved {
            self.emit_two_finger(out);
        }
    }

    fn index(&self, id: ContactId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    fn down(&mut self, ev: &PointerEvent, out: &mut Vec<Gesture>) {
        if self.index(ev.id).is_some() {
            return;
        }
        self.tracks.push(Track {
            id: ev.id,
            state: State::Pending,
            start_x: ev.x,
            start_y: ev.y,
            x: ev.x,
            y: ev.y,
            down_us: ev.time_us,
        });
        if self.config.two_finger
            && self.pair.is_none()
            && self.tracks.len() == 2
            && self.tracks.iter().all(|t| t.state == State::Pending)
        {
            let (a, b) = (self.tracks[0], self.tracks[1]);
            let (cx, cy) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
            self.pair = Some(Pair { a: a.id, b: b.id, start_spread: spread(&a, &b), cx, cy });
            for t in &mut self.tracks {
                t.state = State::TwoFinger;
            }
            out.push(Gesture::TwoFingerStart { cx, cy });
        }
    }

    /// Returns true when a two-finger participant moved.
    fn moved(&mut self, ev: &PointerEvent, out: &mut Vec<Gesture>) -> bool {
        let Some(i) = self.index(ev.id) else { return false };
        let slop = self.config.slop_px;
        let t = &mut self.tracks[i];
        let (px, py) = (t.x, t.y);
        t.x = ev.x;
        t.y = ev.y;
        match t.state {
            State::Pending | State::LongPressed => {
                if (t.x - t.start_x).hypot(t.y - t.start_y) > slop {
                    t.state = State::Dragging;
                    out.push(Gesture::DragStart { id: t.id, x: t.start_x, y: t.start_y });
                    out.push(Gesture::Drag {
                        id: t.id,
                        x: t.x,
                        y: t.y,
                        dx: t.x - t.start_x,
                        dy: t.y - t.start_y,
                    });
                }
            }
            State::Dragging => out.push(Gesture::Drag {
                id: t.id,
                x: t.x,
                y: t.y,
                dx: t.x - px,
                dy: t.y - py,
            }),
            State::TwoFinger => return true,
            State::Spent => {}
        }
        false
    }

    fn emit_two_finger(&mut self, out: &mut Vec<Gesture>) {
        let Some(pair) = self.pair.as_mut() else { return };
        let find = |id| self.tracks.iter().find(|t| t.id == id).copied();
        let (Some(a), Some(b)) = (find(pair.a), find(pair.b)) else { return };
        let (cx, cy) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let scale = if pair.start_spread > 0.0 { spread(&a, &b) / pair.start_spread } else { 1.0 };
        out.push(Gesture::TwoFinger { cx, cy, dx: cx - pair.cx, dy: cy - pair.cy, scale });
        pair.cx = cx;
        pair.cy = cy;
    }

    fn ended(&mut self, ev: &PointerEvent, cancelled: bool, out: &mut Vec<Gesture>) {
        let Some(i) = self.index(ev.id) else { return };
        let t = self.tracks.remove(i);
        match t.state {
            State::Pending => {
                if !cancelled && ev.time_us.saturating_sub(t.down_us) <= self.config.tap_max_us {
                    out.push(Gesture::Tap { id: t.id, x: t.x, y: t.y });
                }
            }
            State::Dragging => {
                out.push(Gesture::DragEnd { id: t.id, x: ev.x, y: ev.y, cancelled })
            }
            State::TwoFinger => {
                self.pair = None;
                for other in &mut self.tracks {
                    if other.state == State::TwoFinger {
                        other.state = State::Spent;
                    }
                }
                out.push(Gesture::TwoFingerEnd { cancelled });
            }
            State::LongPressed | State::Spent => {}
        }
    }
}

fn spread(a: &Track, b: &Track) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}
