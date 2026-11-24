//! How sure the looper was of a free first pass's close, four ways kept
//! apart (the owner's looper review, 3.4; the looper refactor's R1
//! Close): where the loop repeats (its end), its bars, its tempo and its
//! start. Each is told from what smart loop end decided
//! ([`Decision`]) and how the length read ([`Reading`]), so that "the
//! end is clear, the bars aren't" can be said, and a later decision can
//! weigh one without the others. For now they go in the log
//! ([`crate::SmartNote::sure`]); the screen shows them only where they
//! change what you're offered (a close call's Read as, Length's
//! question).

use crate::ends::{Decision, Verdict};
use crate::reading::{Reading, Sure};

/// How well the notes played into a free first loop must fall on its
/// beats for the tempo to count as the beat's ([`Reading::beat`]).
pub const BEAT_FITS: f64 = 0.6;

/// Where the loop comes round: the end smart loop end looked for near
/// your tap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// One end clearly ahead ([`Verdict::Moved`]): smart loop end moved
    /// to it or offers it.
    Clear,
    /// Candidates, none clearly ahead, or a bar count in doubt that
    /// doesn't agree on one ([`Verdict::NotClear`]).
    Possible,
    /// No note near the tap, too many to follow, or no answer in time:
    /// nothing to go on.
    Unmatched,
}

impl Boundary {
    pub fn of(v: Verdict) -> Boundary {
        match v {
            Verdict::Moved => Boundary::Clear,
            Verdict::NotClear => Boundary::Possible,
            Verdict::NoMatch | Verdict::Lost | Verdict::Late => Boundary::Unmatched,
        }
    }
}

/// The loop's bar count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bars {
    /// Length said it, and it was kept.
    Yours,
    /// One count well ahead of the rest ([`Sure::Clear`]).
    Clear,
    /// Two close ([`Sure::Ambiguous`]): Read as offers the other.
    Ambiguous,
}

impl Bars {
    pub fn of(r: &Reading) -> Bars {
        if r.said.is_some() && !r.overruled() {
            return Bars::Yours;
        }
        match r.sure {
            Sure::Clear => Bars::Clear,
            Sure::Ambiguous => Bars::Ambiguous,
        }
    }
}

/// What the loop's tempo rests on (it is always the tempo that makes its
/// bars exactly its length).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tempo {
    /// Length's bars.
    FromYourBars,
    /// The beat you played into the loop: it fits these bars well
    /// ([`BEAT_FITS`]), or picked a count that isn't a power of two.
    FromTheBeat,
    /// The length alone, or a beat that fits only loosely: a guess.
    FromTheLength,
}

impl Tempo {
    pub fn of(r: &Reading) -> Tempo {
        if r.said.is_some() && !r.overruled() {
            return Tempo::FromYourBars;
        }
        match r.beat {
            Some(_) if !r.bars.is_power_of_two() => Tempo::FromTheBeat,
            Some(f) if f >= BEAT_FITS => Tempo::FromTheBeat,
            _ => Tempo::FromTheLength,
        }
    }
}

/// Where the loop starts. A possible pickup isn't told apart yet: that
/// needs phrase evidence (the plan's P2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// Your first hit came within the reach of your press: the loop is
    /// measured from it ([`Decision::lead`]).
    FirstHit,
    /// No hit near the press (a rest you meant, or silence): the start
    /// stays on your press.
    Press,
}

impl Start {
    pub fn of(d: &Decision) -> Start {
        if d.opened {
            Start::FirstHit
        } else {
            Start::Press
        }
    }
}

/// The four, together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Confidence {
    pub boundary: Boundary,
    pub bars: Bars,
    pub tempo: Tempo,
    pub start: Start,
}

impl Confidence {
    /// From smart loop end's decision `d` and the reading `r` the loop
    /// has now.
    pub fn of(d: &Decision, r: &Reading) -> Confidence {
        Confidence {
            boundary: Boundary::of(d.verdict),
            bars: Bars::of(r),
            tempo: Tempo::of(r),
            start: Start::of(d),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ends::Candidate;

    fn reading(bars: u32, sure: Sure, said: Option<u32>, beat: Option<f64>) -> Reading {
        Reading {
            bars,
            bpm: 120.0,
            sure,
            other: None,
            said,
            beat,
        }
    }

    fn decision(verdict: Verdict, opened: bool) -> Decision {
        let tap = Candidate {
            end: 1_000,
            onset: 0.0,
            rhythm: 0.0,
            context: 0.0,
            prior: 1.0,
            score: 0.0,
        };
        Decision {
            verdict,
            end: 1_000,
            best: None,
            second: None,
            tap,
            lead: 0,
            opened,
        }
    }

    #[test]
    fn each_verdict_says_how_clear_the_end_is() {
        assert_eq!(Boundary::of(Verdict::Moved), Boundary::Clear);
        assert_eq!(Boundary::of(Verdict::NotClear), Boundary::Possible);
        for v in [Verdict::NoMatch, Verdict::Lost, Verdict::Late] {
            assert_eq!(Boundary::of(v), Boundary::Unmatched, "{v:?}");
        }
    }

    #[test]
    fn length_kept_makes_the_bars_and_the_tempo_yours() {
        let r = reading(4, Sure::Clear, Some(4), None);
        assert_eq!(
            (Bars::of(&r), Tempo::of(&r)),
            (Bars::Yours, Tempo::FromYourBars)
        );
        // Overruled (slower than 30 BPM): read as if free.
        let r = reading(8, Sure::Ambiguous, Some(4), None);
        assert_eq!(
            (Bars::of(&r), Tempo::of(&r)),
            (Bars::Ambiguous, Tempo::FromTheLength)
        );
    }

    #[test]
    fn the_tempo_is_the_beats_only_when_the_beat_fits_or_chose_the_bars() {
        let at = |bars, beat| Tempo::of(&reading(bars, Sure::Clear, None, beat));
        assert_eq!(at(2, None), Tempo::FromTheLength);
        assert_eq!(at(2, Some(0.59)), Tempo::FromTheLength);
        assert_eq!(at(2, Some(BEAT_FITS)), Tempo::FromTheBeat);
        assert_eq!(at(3, Some(0.3)), Tempo::FromTheBeat);
        assert_eq!(Bars::of(&reading(2, Sure::Clear, None, None)), Bars::Clear);
    }

    #[test]
    fn the_start_is_your_first_hit_only_when_one_came_near_the_press() {
        assert_eq!(Start::of(&decision(Verdict::Moved, true)), Start::FirstHit);
        assert_eq!(Start::of(&decision(Verdict::Moved, false)), Start::Press);
        let c = Confidence::of(
            &decision(Verdict::NotClear, true),
            &reading(2, Sure::Ambiguous, None, Some(0.9)),
        );
        assert_eq!(
            c,
            Confidence {
                boundary: Boundary::Possible,
                bars: Bars::Ambiguous,
                tempo: Tempo::FromTheBeat,
                start: Start::FirstHit,
            }
        );
    }
}
