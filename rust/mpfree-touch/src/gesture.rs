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
//! M14 adds release velocity (`DragEnd` carries `vx`/`vy` in px/s), swipes
//! (a fast, mostly straight drag, reported after its `DragEnd`) and [`Fling`]
//! for kinetic scrolling. The intended use is one recognizer per widget that
//! wants gestures, fed only the fingers that widget owns, so a pad press and a
//! fader drag elsewhere on screen can never pair up into a pinch.
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
    /// Release velocity is measured over the moves in this window before the
    /// lift. A finger that paused longer than this before lifting has zero
    /// velocity (the panel sends nothing while a finger is still).
    pub velocity_window_us: u64,
    /// A swipe must travel at least this far (logical px)...
    pub swipe_min_px: f32,
    /// ...be moving at least this fast at release (px/s)...
    pub swipe_min_speed: f32,
    /// ...and its main axis must dominate the other by this ratio, both in
    /// distance travelled and in release velocity.
    pub swipe_axis_ratio: f32,
}

impl Default for GestureConfig {
    fn default() -> Self {
        // Starting points, not yet tuned on the device. At 113 Hz an 80 ms
        // window holds about 9 reports.
        Self {
            slop_px: 12.0,
            tap_max_us: 300_000,
            long_press_us: 500_000,
            two_finger: true,
            velocity_window_us: 80_000,
            swipe_min_px: 80.0,
            swipe_min_speed: 500.0,
            swipe_axis_ratio: 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    Tap { id: ContactId, x: f32, y: f32 },
    LongPress { id: ContactId, x: f32, y: f32 },
    DragStart { id: ContactId, x: f32, y: f32 },
    /// `dx`/`dy` are since the previous Drag (or DragStart).
    Drag { id: ContactId, x: f32, y: f32, dx: f32, dy: f32 },
    /// `vx`/`vy`: release velocity in px/s (zero when cancelled).
    DragEnd { id: ContactId, x: f32, y: f32, vx: f32, vy: f32, cancelled: bool },
    /// A fast, mostly straight drag. Always follows that contact's `DragEnd`,
    /// so a widget that only scrolls can ignore it. `speed` is px/s along the
    /// swipe's direction.
    Swipe { id: ContactId, direction: SwipeDirection, speed: f32 },
    TwoFingerStart { cx: f32, cy: f32 },
    /// Centroid pan since the previous update, and spread relative to the
    /// spread when the gesture started (pinch: <1 in, >1 out).
    TwoFinger { cx: f32, cy: f32, dx: f32, dy: f32, scale: f32 },
    TwoFingerEnd { cancelled: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwipeDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Recent positions of one contact, for release velocity.
const HISTORY: usize = 16;

#[derive(Debug, Clone, Copy)]
struct History {
    t: [u64; HISTORY],
    x: [f32; HISTORY],
    y: [f32; HISTORY],
    len: usize,
    head: usize,
}

impl History {
    fn new(t: u64, x: f32, y: f32) -> Self {
        let mut h = Self { t: [0; HISTORY], x: [0.0; HISTORY], y: [0.0; HISTORY], len: 0, head: 0 };
        h.push(t, x, y);
        h
    }

    fn push(&mut self, t: u64, x: f32, y: f32) {
        self.t[self.head] = t;
        self.x[self.head] = x;
        self.y[self.head] = y;
        self.head = (self.head + 1) % HISTORY;
        self.len = (self.len + 1).min(HISTORY);
    }

    /// The `back`-th newest sample (0 = newest).
    fn get(&self, back: usize) -> (u64, f32, f32) {
        let i = (self.head + HISTORY - 1 - back) % HISTORY;
        (self.t[i], self.x[i], self.y[i])
    }

    /// Velocity in px/s at `end_us`, from the samples within `window_us`.
    fn velocity(&self, end_us: u64, window_us: u64) -> (f32, f32) {
        let (t1, x1, y1) = self.get(0);
        if end_us.saturating_sub(t1) > window_us {
            return (0.0, 0.0); // paused before lifting
        }
        let mut oldest = None;
        for back in 1..self.len {
            let (t, x, y) = self.get(back);
            if end_us.saturating_sub(t) > window_us {
                break;
            }
            oldest = Some((t, x, y));
        }
        match oldest {
            Some((t0, x0, y0)) if t1 > t0 => {
                let dt = (t1 - t0) as f32 / 1e6;
                ((x1 - x0) / dt, (y1 - y0) / dt)
            }
            _ => (0.0, 0.0),
        }
    }
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
    history: History,
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
            history: History::new(ev.time_us, ev.x, ev.y),
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
        t.history.push(ev.time_us, ev.x, ev.y);
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
                let (vx, vy) = if cancelled {
                    (0.0, 0.0)
                } else {
                    t.history.velocity(ev.time_us, self.config.velocity_window_us)
                };
                out.push(Gesture::DragEnd { id: t.id, x: ev.x, y: ev.y, vx, vy, cancelled });
                if !cancelled {
                    if let Some((direction, speed)) =
                        swipe(&self.config, ev.x - t.start_x, ev.y - t.start_y, vx, vy)
                    {
                        out.push(Gesture::Swipe { id: t.id, direction, speed });
                    }
                }
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

/// Classify a finished drag (displacement `dx, dy`, release velocity
/// `vx, vy`) as a swipe, or not.
fn swipe(c: &GestureConfig, dx: f32, dy: f32, vx: f32, vy: f32) -> Option<(SwipeDirection, f32)> {
    let horizontal = dx.abs() >= dy.abs();
    let (d, d_off, v, v_off) = if horizontal { (dx, dy, vx, vy) } else { (dy, dx, vy, vx) };
    let same_way = d * v > 0.0;
    let straight = d.abs() >= c.swipe_axis_ratio * d_off.abs() && v.abs() >= c.swipe_axis_ratio * v_off.abs();
    if !same_way || !straight || d.abs() < c.swipe_min_px || v.abs() < c.swipe_min_speed {
        return None;
    }
    let direction = match (horizontal, d > 0.0) {
        (true, true) => SwipeDirection::Right,
        (true, false) => SwipeDirection::Left,
        (false, true) => SwipeDirection::Down,
        (false, false) => SwipeDirection::Up,
    };
    Some((direction, v.abs()))
}

/// Kinetic scrolling along one axis after a release: the content keeps moving
/// at the release velocity and slows down exponentially.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fling {
    velocity: f32,
    /// Fraction of velocity lost per second, as an exponential rate (1/s).
    decay: f32,
    stop_speed: f32,
}

impl Fling {
    /// Typical list feel: velocity halves about every 0.25 s, and the fling
    /// stops below 20 px/s. Untuned.
    pub fn new(velocity: f32) -> Self {
        Self::with(velocity, std::f32::consts::LN_2 / 0.25, 20.0)
    }

    pub fn with(velocity: f32, decay: f32, stop_speed: f32) -> Self {
        let velocity = if velocity.is_finite() && velocity.abs() >= stop_speed { velocity } else { 0.0 };
        Self { velocity, decay: decay.max(0.01), stop_speed }
    }

    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    pub fn done(&self) -> bool {
        self.velocity == 0.0
    }

    /// Advance by `dt` seconds; returns how far the content moves (px).
    pub fn step(&mut self, dt: f32) -> f32 {
        if self.done() || dt.is_nan() || dt <= 0.0 {
            return 0.0;
        }
        let keep = (-self.decay * dt).exp();
        let moved = self.velocity * (1.0 - keep) / self.decay;
        self.velocity *= keep;
        if self.velocity.abs() < self.stop_speed {
            self.velocity = 0.0;
        }
        moved
    }

    /// Total distance the fling would still travel if left alone.
    pub fn remaining(&self) -> f32 {
        self.velocity / self.decay
    }
}

fn spread(a: &Track, b: &Track) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}
