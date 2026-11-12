//! The Q-Links' touch sensors chatter. In the live test of 12 Nov 2026
//! (main c8ef977), 264 of 288 releases were followed by a new touch within
//! 150 ms, some within 2 ms, and 260 of 292 touches lasted under 100 ms, so
//! a lit card flickered (most visibly on Synthwave), and on the Sample page
//! each false release ended a turn early (one undo step each).
//!
//! [`TouchHold`] passes a touch on at once, and a release only once the
//! finger has stayed off for [`HOLD`] seconds. A touch inside that time is
//! the same hold: it is counted as a bounce and passed on as nothing.
//! Pure logic, no I/O and no allocation (it runs on the audio thread).

/// How long a finger must stay off before the release counts, in seconds.
pub const HOLD: f64 = 0.15;

/// Four Q-Links' touches, with the chatter held back.
#[derive(Clone, Debug, Default)]
pub struct TouchHold {
    /// A finger is on the knob, as far as the rest of the app knows.
    held: [bool; 4],
    /// When the sensor last let go, while the release waits to count.
    off_since: [Option<f64>; 4],
    /// Touches passed on.
    pub touches: u64,
    /// Releases that a touch inside [`HOLD`] took back.
    pub bounces: u64,
}

impl TouchHold {
    /// A touch (`true`) or a release from knob `k` at `t` seconds. Returns a
    /// touch to pass on now; a release always waits for [`TouchHold::due`].
    pub fn event(&mut self, k: usize, touched: bool, t: f64) -> bool {
        if k >= 4 {
            return false;
        }
        if touched {
            if self.off_since[k].take().is_some() {
                self.bounces += 1;
                return false;
            }
            if self.held[k] {
                return false;
            }
            self.held[k] = true;
            self.touches += 1;
            true
        } else {
            if self.held[k] && self.off_since[k].is_none() {
                self.off_since[k] = Some(t);
            }
            false
        }
    }

    /// The knobs whose finger has now been off for [`HOLD`] seconds: their
    /// releases, to pass on now (each once).
    pub fn due(&mut self, t: f64) -> [bool; 4] {
        let mut out = [false; 4];
        for (k, o) in out.iter_mut().enumerate() {
            // (A microsecond short still counts: the times are floats.)
            if self.off_since[k].is_some_and(|since| t - since >= HOLD - 1e-6) {
                self.off_since[k] = None;
                self.held[k] = false;
                *o = true;
            }
        }
        out
    }

    /// One line for the status log.
    pub fn describe(&self) -> String {
        format!(
            "Q-Link touch: {} touches, {} bounces held (a release shorter than {} ms)",
            self.touches,
            self.bounces,
            (HOLD * 1000.0).round()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_passes_at_once_and_a_release_after_the_hold() {
        let mut h = TouchHold::default();
        assert!(h.event(0, true, 1.0));
        assert!(!h.event(0, false, 2.0));
        assert_eq!(h.due(2.0 + HOLD / 2.0), [false; 4]);
        assert_eq!(h.due(2.0 + HOLD), [true, false, false, false]);
        // Once only.
        assert_eq!(h.due(3.0), [false; 4]);
        assert_eq!((h.touches, h.bounces), (1, 0));
    }

    #[test]
    fn chatter_inside_the_hold_is_one_long_touch() {
        // The pattern from the live test: lets go and touches again within
        // 2 to 100 ms while the knob turns.
        let mut h = TouchHold::default();
        assert!(h.event(2, true, 10.0));
        let mut t = 10.0;
        for gap in [0.002, 0.003, 0.05, 0.1, 0.149] {
            t += 0.03;
            assert!(!h.event(2, false, t));
            assert_eq!(h.due(t + gap), [false; 4]);
            t += gap;
            assert!(!h.event(2, true, t));
        }
        assert_eq!((h.touches, h.bounces), (1, 5));
        assert!(!h.event(2, false, t + 0.01));
        assert_eq!(h.due(t + 0.01 + HOLD), [false, false, true, false]);
    }

    #[test]
    fn knobs_are_held_apart_and_strays_are_ignored() {
        let mut h = TouchHold::default();
        assert!(h.event(0, true, 0.0));
        assert!(h.event(3, true, 0.0));
        // A second touch report without a release in between passes nothing.
        assert!(!h.event(3, true, 0.01));
        // A release of a knob never touched waits for nothing.
        assert!(!h.event(1, false, 0.0));
        assert!(!h.event(0, false, 0.1));
        assert_eq!(h.due(0.1 + HOLD), [true, false, false, false]);
        assert!(!h.event(9, true, 0.0));
        assert!(h.describe().contains("2 touches, 0 bounces held"));
    }
}
