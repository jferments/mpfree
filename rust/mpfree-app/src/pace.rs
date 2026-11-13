//! When the screen thread ticks the pages, and how long it sleeps between.
//!
//! The pages tick at a steady rate whether or not anything redraws, and each
//! tick's time step is the time since the tick before, so the pages' clocks
//! keep time: a told line's 4 s, a long press's 0.5 s, the top bar's chips
//! once a second. Until 13 Nov the step was the time since the last frame
//! drawn, and while nothing redrew the loop ticked on every 4 ms pass with a
//! step that grew to 0.1 s, so those clocks ran about 23 times fast (one idle
//! second on the build workstation: 233 ticks moved them 22.8 s).
//!
//! Between ticks the thread sleeps until a message, the touchscreen's
//! doorbell ([`crate::touch_bell`]) or the next tick, instead of waking every
//! 4 ms to look.

use std::time::{Duration, Instant};

/// From the end of one frame, or from a tick that drew nothing, to the next
/// tick: about 30 frames a second, fewer while frames take long to draw (as
/// before).
pub const FRAME: Duration = Duration::from_millis(33);
/// Without the doorbell, the screen thread reads the touchscreen this often.
pub const TOUCH_POLL: Duration = Duration::from_millis(4);
/// The longest step one tick gives the pages, so a stalled thread does not
/// jump them on.
const MAX_STEP: f32 = 0.1;

/// The screen thread's ticks.
#[derive(Debug)]
pub struct Pace {
    last: Instant,
    next: Instant,
}

impl Pace {
    /// The first tick is due at once.
    pub fn new(now: Instant) -> Pace {
        Pace {
            last: now,
            next: now,
        }
    }

    /// How long to sleep until the next tick (zero when one is due).
    pub fn wait(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    /// The time step for the pages when a tick is due (or `anyway`, for the
    /// last one before the thread stops); else None.
    pub fn tick(&mut self, now: Instant, anyway: bool) -> Option<f32> {
        if now < self.next && !anyway {
            return None;
        }
        let step = now
            .saturating_duration_since(self.last)
            .as_secs_f32()
            .min(MAX_STEP);
        self.last = now;
        self.next = now + FRAME;
        Some(step)
    }

    /// A frame finished drawing at `end`: the next tick is a frame after it.
    pub fn drawn(&mut self, end: Instant) {
        self.next = end + FRAME;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Runs the screen loop's pacing for `secs` on a made-up clock: sleep
    /// for `wait`, tick, and draw for `draw` when it is not zero. Returns
    /// the ticks and the seconds they gave the pages.
    fn run(secs: u64, draw: Duration) -> (u32, f32) {
        let t0 = Instant::now();
        let end = t0 + Duration::from_secs(secs);
        let mut p = Pace::new(t0);
        let mut now = t0;
        let (mut ticks, mut ticked) = (0, 0.0);
        while now < end {
            now += p.wait(now);
            // A message wakes the thread early now and then: no tick yet.
            assert_eq!(p.tick(now - MS, false), None);
            let step = p.tick(now, false).expect("a tick is due after the wait");
            ticks += 1;
            ticked += step;
            if !draw.is_zero() {
                now += draw;
                p.drawn(now);
            }
        }
        (ticks, ticked)
    }

    #[test]
    fn the_pages_clock_keeps_time_whether_or_not_anything_draws() {
        // Nothing draws: a tick every 33 ms (the last at 3.003 s), and the
        // ticks give the pages 3.003 s.
        let (ticks, ticked) = run(3, Duration::ZERO);
        assert_eq!(ticks, 92);
        assert!((ticked - 3.003).abs() < 0.001, "{ticked}");
        // Every frame takes 10 ms to draw: a tick every 43 ms, as before
        // (the last at 3.01 s).
        let (ticks, ticked) = run(3, 10 * MS);
        assert_eq!(ticks, 71);
        assert!((ticked - 3.01).abs() < 0.001, "{ticked}");
    }

    #[test]
    fn a_stalled_thread_moves_the_pages_on_a_tenth_of_a_second_at_most() {
        let t0 = Instant::now();
        let mut p = Pace::new(t0);
        assert_eq!(p.tick(t0, false), Some(0.0), "the first tick is at once");
        assert_eq!(p.wait(t0), FRAME);
        assert_eq!(p.wait(t0 + 40 * MS), Duration::ZERO);
        assert_eq!(p.tick(t0 + 2000 * MS, false), Some(0.1));
        // The last tick, on Quit, comes whenever it is asked for.
        assert!(p.tick(t0 + 2001 * MS, true).is_some());
    }
}
