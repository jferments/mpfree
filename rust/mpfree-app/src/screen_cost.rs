//! What the screen thread spends, per frame and per page: painting the
//! changed widgets into the canvas, copying them to the panel, and how many
//! native rows that copy wrote. The software screen thread took 18-40 % of
//! a core in live tests 2 and 3; these are the numbers to beat, for the
//! software path and for the GPU's (docs/dev/design/profiling.md, "The
//! screen's cost").
//!
//! The panel is mounted turned, so a changed rectangle's width on screen is
//! the number of native rows copied: a strip across the whole screen (the
//! top bar) rewrites nearly every row of the panel, however thin it is.
//!
//! - Every second: the Performance page's "Screen" row ([`ScreenCost::second`]).
//! - Every 30 s: a `status: screen (30 s): ...` line for the log
//!   ([`ScreenCost::status`]), with the page that cost the most.
//!
//! The screen thread only: nothing here touches the audio thread.

use std::time::{Duration, Instant};

use crate::app::View;

/// How often the 30 s line goes out.
pub const STATUS_EVERY: Duration = Duration::from_secs(30);
const SECOND: Duration = Duration::from_secs(1);

/// One stretch of frames: the last second, or the last 30 s.
#[derive(Clone, Debug, Default)]
struct Window {
    frames: u64,
    paint: Duration,
    paint_max: Duration,
    copy: Duration,
    copy_max: Duration,
    rows: u64,
    /// The slowest frame, paint and copy together.
    worst: Duration,
    wakes: u64,
}

impl Window {
    fn add(&mut self, paint: Duration, copy: Duration, rows: usize) {
        self.frames += 1;
        self.paint += paint;
        self.copy += copy;
        self.paint_max = self.paint_max.max(paint);
        self.copy_max = self.copy_max.max(copy);
        self.rows += rows as u64;
        self.worst = self.worst.max(paint + copy);
    }

    fn mean_ms(&self, d: Duration) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            d.as_secs_f64() * 1e3 / self.frames as f64
        }
    }
}

/// The screen thread's counts.
#[derive(Debug)]
pub struct ScreenCost {
    second: (Instant, Window),
    long: (Instant, Window),
    /// Paint and copy time per page in the 30 s window.
    by_view: Vec<(View, Duration)>,
}

impl ScreenCost {
    pub fn new(now: Instant) -> ScreenCost {
        ScreenCost {
            second: (now, Window::default()),
            long: (now, Window::default()),
            by_view: Vec::with_capacity(8),
        }
    }

    /// One pass of the screen loop (it wakes for messages, touch and the
    /// next frame).
    pub fn wake(&mut self) {
        self.second.1.wakes += 1;
        self.long.1.wakes += 1;
    }

    /// One frame drawn on `view`: its paint and copy times and the native
    /// rows the copy wrote.
    pub fn frame(&mut self, view: View, paint: Duration, copy: Duration, rows: usize) {
        self.second.1.add(paint, copy, rows);
        self.long.1.add(paint, copy, rows);
        match self.by_view.iter_mut().find(|(v, _)| *v == view) {
            Some((_, d)) => *d += paint + copy,
            None => self.by_view.push((view, paint + copy)),
        }
    }

    /// The Performance page's "Screen" row, once a second; else None.
    pub fn second(&mut self, now: Instant) -> Option<String> {
        let (start, w) = &self.second;
        let took = now.saturating_duration_since(*start);
        if took < SECOND {
            return None;
        }
        let fps = w.frames as f64 / took.as_secs_f64();
        let text = format!(
            "{fps:.0} fps · paint {:.1} ms · copy {:.1} ms · slowest {:.1} ms",
            w.mean_ms(w.paint),
            w.mean_ms(w.copy),
            w.worst.as_secs_f64() * 1e3
        );
        self.second = (now, Window::default());
        Some(text)
    }

    /// The `status: screen (30 s): ...` line, every [`STATUS_EVERY`]; else
    /// None.
    pub fn status(&mut self, now: Instant) -> Option<String> {
        let (start, w) = &self.long;
        let took = now.saturating_duration_since(*start);
        if took < STATUS_EVERY {
            return None;
        }
        let secs = took.as_secs_f64();
        let busy = (w.paint + w.copy).as_secs_f64();
        let mut line = format!(
            "status: screen (30 s): {} frames ({:.1} a second), {:.1} % of a core drawing; paint \
             mean {:.2} ms, max {:.1} ms; copy mean {:.2} ms, max {:.1} ms, {} native rows a \
             frame; {:.0} wakeups a second",
            w.frames,
            w.frames as f64 / secs,
            busy / secs * 100.0,
            w.mean_ms(w.paint),
            w.paint_max.as_secs_f64() * 1e3,
            w.mean_ms(w.copy),
            w.copy_max.as_secs_f64() * 1e3,
            w.rows.checked_div(w.frames).unwrap_or(0),
            w.wakes as f64 / secs
        );
        if let Some((view, d)) = self.by_view.iter().max_by_key(|(_, d)| *d) {
            line.push_str(&format!(
                "; most on {} ({:.1} s)",
                view.name(),
                d.as_secs_f64()
            ));
        }
        self.long = (now, Window::default());
        self.by_view.clear();
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_second_row_and_the_status_line_say_what_the_frames_cost() {
        let t = Instant::now();
        let mut c = ScreenCost::new(t);
        for _ in 0..100 {
            c.wake();
        }
        for _ in 0..10 {
            c.frame(View::Play, 2 * MS, 4 * MS, 1256);
        }
        c.frame(View::Pads, 8 * MS, 12 * MS, 853);
        assert_eq!(c.second(t + 500 * MS), None, "not a second yet");
        let row = c.second(t + SECOND).unwrap();
        assert_eq!(row, "11 fps · paint 2.5 ms · copy 4.7 ms · slowest 20.0 ms");
        assert_eq!(c.second(t + SECOND + MS), None, "once a second");
        assert_eq!(c.status(t + SECOND), None, "not 30 s yet");
        let line = c.status(t + STATUS_EVERY).unwrap();
        assert!(
            line.starts_with(
                "status: screen (30 s): 11 frames (0.4 a second), 0.3 % of a core drawing; \
                 paint mean 2.55 ms, max 8.0 ms; copy mean 4.73 ms, max 12.0 ms, 1219 native \
                 rows a frame; 3 wakeups a second"
            ),
            "{line}"
        );
        assert!(line.ends_with("; most on Play (0.1 s)"), "{line}");
        // The next window starts empty.
        let line = c.status(t + 2 * STATUS_EVERY).unwrap();
        assert!(
            line.starts_with("status: screen (30 s): 0 frames"),
            "{line}"
        );
        assert!(!line.contains("most on"), "{line}");
    }
}
