//! The bar reading: how many bars a first loop recorded in your own time
//! holds, and so the tempo it sets ([`Command::TempoFromLoop`]).
//!
//! A loop's length alone can't say how many bars were played: 3 s in 4/4
//! is 1 bar at 80 BPM, 2 bars at 160 or 4 at 320, and all three are the
//! same audio. Before 22 Nov the looper took the fewest bars (1, 2, 4 ...)
//! that put the tempo between 80 and 160 BPM, so a few milliseconds at
//! the window's edge doubled the tempo, and anyone playing under 80 always
//! got double. The reading (the smart loop end plan, step 0B) weighs the
//! evidence instead, in order of strength:
//!
//! 1. **Length said the bars.** With a bar count set, the loop is that
//!    many bars at the tempo you played: plain arithmetic, nothing read.
//!    Only a count that would put the tempo under the slowest the app
//!    allows is read as below instead, and the reading says so
//!    ([`Reading::said`]).
//! 2. **A soft preference.** Each bar count's tempo is scored by how near
//!    it is to everyday tempos, on a curve that fades out instead of
//!    stopping at an edge. Its centre is the middle of the old window, so
//!    with no other evidence the pick is the old rule's pick: only the
//!    label and the tempos past the old window change.
//! 3. **A tempo you meant.** The song's tempo before the loop counts too:
//!    strongly when it was set by hand since the project opened
//!    ([`TempoKind::Set`]), lightly when it was left from before
//!    ([`TempoKind::Earlier`]).
//!
//! The reading also says how sure it is: [`Sure::Clear`] when one bar
//! count is well ahead, [`Sure::Ambiguous`] when two are close. Ambiguous
//! still picks one, so the song can start, and names the other
//! ([`Reading::other`]) for the screen to offer.
//!
//! The beat inside the loop (pad notes, line-in hits) joins as evidence in
//! the plan's steps 1 and 3, which also allow counts other than powers of
//! two.
//!
//! [`Command::TempoFromLoop`]: crate::Command::TempoFromLoop

use mpfree_transport::time::{Tick, PPQ};

/// The slowest tempo a first loop may set: the app's slowest (the
/// MPC's range, 30 to 300 BPM).
pub const LOOP_MIN_BPM: f64 = 30.0;
/// The fastest: a free first pass shorter than a bar at this tempo (0.8 s
/// in 4/4; with Length set, that many bars) waits to close until it is
/// one.
pub const LOOP_MAX_BPM: f64 = 300.0;
/// The middle of the soft preference: the middle of the old 80 to 160
/// window (on a log scale), so a reading with no other evidence picks the
/// bar count the old rule picked.
pub const PREFERRED_BPM: f64 = 113.137_084_989_847_6;
/// How fast the preference fades: a tempo this far from
/// [`PREFERRED_BPM`] on a log scale (about half an octave) scores -0.5.
const SPREAD: f64 = 0.35;
/// How far ahead the best bar count must score for the reading to be
/// [`Sure::Clear`]. Two counts an octave apart are within it when the
/// slower is between about 73 and 87 BPM.
pub const MARGIN: f64 = 0.5;
/// How near a reading must be to a tempo you meant to count for it
/// (about 6 % on a log scale).
const HINT_WIDTH: f64 = 0.06;
/// What a tempo set by hand adds to a reading on it.
const HINT_SET: f64 = 2.0;
/// What a tempo left from before adds.
const HINT_EARLIER: f64 = 0.3;
/// The most bars a reading considers.
const MAX_BARS: u32 = 1 << 16;

/// How the song's tempo before the loop came about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TempoKind {
    /// Set by hand since the project opened (typed, tapped, turned): you
    /// meant it, so a reading on it counts strongly.
    Set,
    /// Left from before (the project's saved tempo, the default, an
    /// earlier loop's): it counts lightly.
    Earlier,
}

/// The song's tempo before the loop, as evidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoHint {
    pub bpm: f64,
    pub kind: TempoKind,
}

/// How sure a reading is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sure {
    /// One bar count is well ahead (or Length said it).
    Clear,
    /// Two are close: the reading picks one and names the other.
    Ambiguous,
}

/// What a first loop's length reads as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub bars: u32,
    /// The tempo that makes `bars` bars exactly the loop's length.
    pub bpm: f64,
    pub sure: Sure,
    /// The close runner-up, bars and tempo, when [`Sure::Ambiguous`].
    pub other: Option<(u32, f64)>,
    /// The bar count Length said. `bars` is it unless that count would
    /// be slower than [`LOOP_MIN_BPM`]: then the length was read as if
    /// free, and `bars` differs.
    pub said: Option<u32>,
}

impl Reading {
    /// Length said a count and the reading couldn't keep it.
    pub fn overruled(&self) -> bool {
        self.said.is_some_and(|n| n != self.bars)
    }
}

/// The tempo at which `bars` bars of `bar` ticks last `len` frames at
/// `rate` (the tempo counts quarter notes).
pub fn tempo_of(len: usize, rate: u32, bar: Tick, bars: u32) -> f64 {
    let quarters = bar as f64 / f64::from(PPQ);
    60.0 * f64::from(rate) * quarters * f64::from(bars) / len as f64
}

/// The soft preference and the tempo you meant, for a reading at `bpm`.
fn score(bpm: f64, hint: Option<TempoHint>) -> f64 {
    let d = (bpm / PREFERRED_BPM).ln();
    let mut s = -d * d / (2.0 * SPREAD * SPREAD);
    if let Some(h) = hint.filter(|h| h.bpm.is_finite() && h.bpm > 0.0) {
        let e = (bpm / h.bpm).ln();
        let weight = match h.kind {
            TempoKind::Set => HINT_SET,
            TempoKind::Earlier => HINT_EARLIER,
        };
        s += weight * (-e * e / (2.0 * HINT_WIDTH * HINT_WIDTH)).exp();
    }
    s
}

/// The reading of a first pass of `len` frames at `rate`, with bars of
/// `bar` ticks: the bars Length `said` (None: free), and the song's tempo
/// before (`hint`). None while the pass is too short to close: shorter
/// than one bar at [`LOOP_MAX_BPM`], or than the `said` bars at it.
pub fn read_bars(
    len: usize,
    rate: u32,
    bar: Tick,
    said: Option<u32>,
    hint: Option<TempoHint>,
) -> Option<Reading> {
    let fewest = said.filter(|&n| n > 0).unwrap_or(1);
    let fastest = tempo_of(len, rate, bar, fewest);
    // (A NaN or an infinity, for no frames or no bar, is refused here.)
    if !(fastest.is_finite() && fastest > 0.0 && fastest <= LOOP_MAX_BPM) {
        return None;
    }
    let said = said.filter(|&n| n > 0);
    if let Some(n) = said {
        if fastest >= LOOP_MIN_BPM {
            return Some(Reading {
                bars: n,
                bpm: fastest,
                sure: Sure::Clear,
                other: None,
                said,
            });
        }
    }
    // Free, or a said count too slow: every power of two in the app's
    // tempos, scored. Ties go to fewer bars, as the old window's did.
    let mut best: Option<(u32, f64, f64)> = None;
    let mut second: Option<(u32, f64, f64)> = None;
    let mut bars = 1u32;
    while bars <= MAX_BARS {
        let bpm = tempo_of(len, rate, bar, bars);
        if bpm > LOOP_MAX_BPM {
            break;
        }
        if bpm >= LOOP_MIN_BPM {
            let s = score(bpm, hint);
            match best {
                Some((_, _, b)) if s <= b + 1e-12 => {
                    if second.is_none_or(|(_, _, c)| s > c + 1e-12) {
                        second = Some((bars, bpm, s));
                    }
                }
                _ => {
                    second = best;
                    best = Some((bars, bpm, s));
                }
            }
        }
        bars *= 2;
    }
    let (bars, bpm, s) = best?;
    let close = second.filter(|&(_, _, c)| s - c < MARGIN);
    Some(Reading {
        bars,
        bpm,
        sure: if close.is_some() {
            Sure::Ambiguous
        } else {
            Sure::Clear
        },
        other: close.map(|(b, t, _)| (b, t)),
        said,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 22_050;
    const BAR: Tick = 4 * PPQ as Tick;

    /// The frames of `bars` bars of 4/4 at `bpm`.
    fn frames(bars: u32, bpm: f64) -> usize {
        (f64::from(bars) * 4.0 * 60.0 * f64::from(RATE) / bpm).round() as usize
    }

    #[test]
    fn the_centre_is_the_middle_of_the_old_window() {
        assert!((PREFERRED_BPM - (80.0f64 * 160.0).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn an_everyday_tempo_reads_clear() {
        let r = read_bars(frames(1, 96.0), RATE, BAR, None, None).unwrap();
        assert_eq!((r.bars, r.sure, r.other), (1, Sure::Clear, None));
        assert!((r.bpm - 96.0).abs() < 0.01, "{}", r.bpm);
    }

    #[test]
    fn near_the_old_edge_it_says_it_is_a_close_call() {
        // 1 bar at 79 or 2 at 158: the old rule said 158 without a word.
        let r = read_bars(frames(1, 79.0), RATE, BAR, None, None).unwrap();
        assert_eq!((r.bars, r.sure), (2, Sure::Ambiguous));
        let (b, t) = r.other.unwrap();
        assert_eq!(b, 1);
        assert!((t - 79.0).abs() < 0.01);
    }

    #[test]
    fn a_tempo_set_by_hand_settles_a_close_call() {
        let hint = TempoHint {
            bpm: 80.0,
            kind: TempoKind::Set,
        };
        let r = read_bars(frames(1, 79.0), RATE, BAR, None, Some(hint)).unwrap();
        assert_eq!((r.bars, r.sure), (1, Sure::Clear));
        // Left from before, it only leans.
        let hint = TempoHint {
            bpm: 80.0,
            kind: TempoKind::Earlier,
        };
        let r = read_bars(frames(1, 79.0), RATE, BAR, None, Some(hint)).unwrap();
        assert_eq!(r.sure, Sure::Ambiguous);
    }

    #[test]
    fn length_says_the_bars_and_the_tempo_is_arithmetic() {
        let r = read_bars(frames(2, 70.0), RATE, BAR, Some(2), None).unwrap();
        assert_eq!((r.bars, r.sure, r.said), (2, Sure::Clear, Some(2)));
        assert!((r.bpm - 70.0).abs() < 0.01);
        assert!(!r.overruled());
        // Too short for 2 bars at 300: it waits.
        assert_eq!(read_bars(frames(1, 300.0), RATE, BAR, Some(2), None), None);
        // 1 bar under 30 BPM: read as if free, and it says so.
        let r = read_bars(frames(1, 25.0), RATE, BAR, Some(1), None).unwrap();
        assert!(r.overruled());
        assert_eq!(r.bars, 4);
        assert!((r.bpm - 100.0).abs() < 0.01);
    }

    #[test]
    fn the_shortest_loop_is_a_bar_at_the_fastest_tempo() {
        assert!(read_bars(frames(1, LOOP_MAX_BPM), RATE, BAR, None, None).is_some());
        assert_eq!(
            read_bars(frames(1, LOOP_MAX_BPM) - 2, RATE, BAR, None, None),
            None
        );
        assert_eq!(read_bars(0, RATE, BAR, None, None), None);
        assert_eq!(read_bars(44_100, RATE, 0, None, None), None);
    }
}
