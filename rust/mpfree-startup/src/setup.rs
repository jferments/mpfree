//! The first start of a stick the MPFree USB creator made: while its format
//! step runs, the startup screen stays up with the pad player's own loading
//! card over its last frame, saying what is happening and how far it is.
//!
//! The step (`tools/mpc-one/stick/setup-format.sh`) makes the stick's data
//! partition before the pad player can start, and reports in a `key=value`
//! file (`/run/mpfree-setup/format-status`): `state` (`checking`, `running`,
//! `done`, `none` when there is nothing to make, `failed`), `done` (bytes
//! written to the partition so far, from the block layer) and `total` (what
//! the step expects to write). The stick's splash unit names that file in
//! `MPFREE_SETUP_STATUS` (a drop-in the creator adds); other sticks have none
//! and the startup screen ends as before.
//!
//! The bar moves with the bytes written, between reports at the rate they
//! show, never backwards and never to the end before the step says done.

use std::time::Duration;

/// The environment variable naming the format step's status file.
pub const STATUS_ENV: &str = "MPFREE_SETUP_STATUS";
/// How long to wait for the step to say whether it has anything to do (it
/// waits up to 30 s for the partition itself).
pub const ANSWER: Duration = Duration::from_secs(45);
/// How long to wait in all (the step's own limit is 300 s).
pub const LIMIT: Duration = Duration::from_secs(330);
/// No card for a short check: the pad player's own card follows anyway.
const CHECK_QUIET: f32 = 3.0;

/// The step's report.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub state: String,
    pub done: u64,
    pub total: u64,
}

pub fn parse(text: &str) -> Option<Report> {
    let mut r = Report {
        state: String::new(),
        done: 0,
        total: 0,
    };
    for line in text.lines() {
        match line.split_once('=') {
            Some(("state", v)) => r.state = v.trim().to_string(),
            Some(("done", v)) => r.done = v.trim().parse().unwrap_or(0),
            Some(("total", v)) => r.total = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    (!r.state.is_empty()).then_some(r)
}

impl Report {
    /// The step has finished, or had nothing to do: the startup screen ends.
    pub fn finished(&self) -> bool {
        matches!(self.state.as_str(), "done" | "none" | "failed")
    }
}

/// Bytes written so far as the bar shows them: between reports it goes on
/// at the average rate since the first bytes, at most two seconds past the
/// last report, below 99 % until the step is done, never backwards.
#[derive(Debug, Default)]
pub struct Smooth {
    first: Option<(f32, f64)>,
    done: f64,
    at: f32,
    rate: f64,
    shown: f64,
}

impl Smooth {
    pub fn new() -> Smooth {
        Smooth::default()
    }

    /// A report of `done` bytes at `t` seconds.
    pub fn report(&mut self, t: f32, done: u64) {
        let done = done as f64;
        if done <= self.done {
            // Before the first bytes, the rate counts from the last report of none.
            if self.first.is_none() {
                self.at = t;
            }
            return;
        }
        match self.first {
            None => self.first = Some((self.at, self.done)),
            Some((t0, d0)) if t > t0 + 0.5 => self.rate = (done - d0) / f64::from(t - t0),
            Some(_) => {}
        }
        self.done = done;
        self.at = t;
    }

    /// Where the bar is at `t` seconds, 0 to 1, of `total` bytes.
    pub fn value(&mut self, t: f32, total: u64) -> f32 {
        if total == 0 {
            return 0.0;
        }
        let total = total as f64;
        let ahead = (self.rate * f64::from((t - self.at).clamp(0.0, 2.0))).max(0.0);
        let est = (self.done + ahead).min(total * 0.99);
        self.shown = self.shown.max(est);
        (self.shown / total) as f32
    }

    /// Bytes per second, once known.
    pub fn rate(&self) -> Option<f64> {
        (self.rate > 0.0).then_some(self.rate)
    }

    /// Seconds left of `total` bytes, once the rate is known.
    pub fn left(&self, total: u64) -> Option<f64> {
        self.rate()
            .map(|r| ((total as f64 - self.shown).max(0.0)) / r)
    }
}

/// "about 25 s left" or "about 3 min left": rounded up, to 5 s or a minute.
pub fn time_left(secs: f64) -> String {
    if secs < 55.0 {
        format!("about {} s left", ((secs / 5.0).ceil() as u64 * 5).max(5))
    } else {
        format!("about {} min left", (secs / 60.0).ceil() as u64)
    }
}

fn mb(bytes: f64) -> String {
    format!("{:.0}", bytes / 1e6)
}

/// What the card says at `t` seconds since the startup screen began to
/// wait: (heading, line under it, bar), or None for no card yet.
pub fn card(report: Option<&Report>, smooth: &mut Smooth, t: f32) -> Option<(String, String, f32)> {
    match report {
        Some(r) if r.state == "running" => {
            smooth.report(t, r.done);
            let value = smooth.value(t, r.total);
            let mut detail = format!(
                "Making room for your sounds and settings   ·   {} of {} MB",
                mb(value as f64 * r.total as f64),
                mb(r.total as f64)
            );
            if let Some(left) = smooth.left(r.total) {
                detail.push_str("   ·   ");
                detail.push_str(&time_left(left));
            }
            Some((
                "Preparing the stick for its first start".into(),
                detail,
                value,
            ))
        }
        Some(r) if r.state == "done" => Some((
            "Preparing the stick for its first start".into(),
            "Ready".into(),
            1.0,
        )),
        _ if t >= CHECK_QUIET => Some(("Starting MPFree".into(), "Checking the stick".into(), 0.0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_steps_report() {
        let r = parse("state=running\ndone=1048576\ntotal=75497472\nmessage=Preparing\n").unwrap();
        assert_eq!(
            r,
            Report {
                state: "running".into(),
                done: 1 << 20,
                total: 72 << 20
            }
        );
        assert!(!r.finished());
        for s in ["done", "none", "failed"] {
            assert!(parse(&format!("state={s}\n")).unwrap().finished(), "{s}");
        }
        assert_eq!(parse(""), None);
        assert_eq!(parse("done=5\n"), None, "no state, no report");
        assert_eq!(parse("state=running\ndone=x\n").unwrap().done, 0);
    }

    #[test]
    fn the_bar_moves_between_reports_and_never_back() {
        let mut s = Smooth::new();
        let total = 100_000_000;
        s.report(0.0, 0);
        assert_eq!(s.value(0.0, total), 0.0);
        s.report(1.0, 10_000_000);
        s.report(2.0, 20_000_000);
        // 10 MB/s since the first bytes: half a second later it shows 25 MB.
        assert!((s.value(2.5, total) - 0.25).abs() < 1e-4);
        // Never more than 2 s past the last report.
        assert!((s.value(9.0, total) - 0.40).abs() < 1e-4);
        // A slow report does not take the bar back.
        s.report(10.0, 30_000_000);
        assert!(s.value(10.0, total) >= 0.40);
        // Below 99 % until the step says done.
        s.report(11.0, total);
        assert!((s.value(11.0, total) - 0.99).abs() < 1e-4);
        assert_eq!(Smooth::new().value(1.0, 0), 0.0, "no total, no bar");
    }

    #[test]
    fn says_how_long_is_left() {
        assert_eq!(time_left(0.2), "about 5 s left");
        assert_eq!(time_left(12.0), "about 15 s left");
        assert_eq!(time_left(54.0), "about 55 s left");
        assert_eq!(time_left(61.0), "about 2 min left");
        assert_eq!(time_left(600.0), "about 10 min left");
        let mut s = Smooth::new();
        assert_eq!(s.left(100), None, "not before the rate is known");
        s.report(0.0, 0);
        s.report(1.0, 10);
        s.report(2.0, 20);
        s.value(2.0, 100);
        assert!((s.left(100).unwrap() - 8.0).abs() < 1e-6);
    }

    #[test]
    fn the_card_says_what_is_happening() {
        let mut s = Smooth::new();
        assert_eq!(card(None, &mut s, 1.0), None, "a short check shows no card");
        let (head, detail, value) = card(None, &mut s, 4.0).unwrap();
        assert_eq!(
            (head.as_str(), detail.as_str(), value),
            ("Starting MPFree", "Checking the stick", 0.0)
        );
        let running = |done| Report {
            state: "running".into(),
            done,
            total: 72_000_000,
        };
        card(Some(&running(0)), &mut s, 5.0);
        card(Some(&running(7_200_000)), &mut s, 6.0);
        let (head, detail, value) = card(Some(&running(14_400_000)), &mut s, 7.0).unwrap();
        assert_eq!(head, "Preparing the stick for its first start");
        assert_eq!(
            detail,
            "Making room for your sounds and settings   ·   14 of 72 MB   ·   about 10 s left"
        );
        assert!((value - 0.2).abs() < 1e-4);
        let done = Report {
            state: "done".into(),
            done: 72_000_000,
            total: 72_000_000,
        };
        assert_eq!(card(Some(&done), &mut s, 9.0).unwrap().2, 1.0);
    }
}
