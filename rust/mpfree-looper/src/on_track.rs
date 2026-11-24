//! Loops on audio tracks (audio tracks phase 4, step 4b:
//! docs/dev/design/loops-on-audio-tracks.md, "What the looper is told").
//!
//! A loop on an audio track doesn't launch itself: the track player says,
//! a period at a time, on which frames it sounds and from which origin
//! ([`crate::Looper::play`]), and the looper renders it there and nowhere
//! else, into the loop's own buffer ([`crate::LoopOuts`]), for the track
//! player to fade and mix into the track's channel.
//!
//! - **Its place** is the song position's counted from the run's origin,
//!   `(tick - origin)` modulo the loop's ticks: each frame's tick is moved
//!   by the loop's own origin less the run's, so everything inside the
//!   loop (its layers, print, edits' crossfades, overdubs) works as it
//!   always has, and the loop's own origin stays where its first pass was
//!   recorded.
//! - **A new origin going straight on** (the same loop from another
//!   launch or block, as the track player keeps it one voice) crossfades
//!   the place over [`crate::FADE`], as a locate does.
//! - **After silence** (frames it wasn't told), it starts at its place
//!   with no crossfade from where it was: the track fades it in.
//! - **At full gain:** the track does the fades and the mute, so the
//!   loop's own launch, mute and solo are refused while it is on a track,
//!   and its gain is held at 1 while it plays.

/// Runs a loop on a track is told at most in a period: the track player's
/// changes (`MAX_AUDIO_CHANGES`, 4) and two more.
pub const MAX_PLAY_RUNS: usize = 6;

/// Where a loop on an audio track sounds in the next period: frames
/// `from..to` of it, at its place counted from song tick `origin` (the
/// tick its pass 0 starts on).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayRun {
    pub from: u32,
    pub to: u32,
    pub origin: f64,
}

/// A loop on a track, as the looper keeps it: the runs it was told for
/// the next period, and where it stopped playing last.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OnTrack {
    runs: [PlayRun; MAX_PLAY_RUNS],
    n: usize,
    /// The frame after the last one it played, and the origin it played
    /// it from.
    pub last: Option<(u64, f64)>,
}

/// The runs a loop was told for a period, taken for the period
/// ([`OnTrack::take`]).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Told {
    runs: [PlayRun; MAX_PLAY_RUNS],
    n: usize,
}

impl Told {
    pub fn runs(&self) -> &[PlayRun] {
        &self.runs[..self.n]
    }
}

impl OnTrack {
    /// The runs for the next period, in place of any told before. False
    /// (and nothing changed) if there are too many, or they are out of
    /// order, empty or overlapping.
    pub fn tell(&mut self, runs: &[PlayRun]) -> bool {
        if runs.len() > MAX_PLAY_RUNS
            || runs.iter().any(|r| r.from >= r.to || !r.origin.is_finite())
            || runs.windows(2).any(|w| w[0].to > w[1].from)
        {
            return false;
        }
        self.runs[..runs.len()].copy_from_slice(runs);
        self.n = runs.len();
        true
    }

    /// This period's runs: told afresh for the next.
    pub fn take(&mut self) -> Told {
        let t = Told {
            runs: self.runs,
            n: self.n,
        };
        self.n = 0;
        t
    }
}

/// How a cut of a run starts: as any run does (the span's first frame
/// looks for a jump), with a jump looked for (a new origin straight on),
/// or with none (after silence).
pub(crate) fn first_of(last: Option<(u64, f64)>, f: u64, origin: f64) -> Option<bool> {
    match last {
        Some((e, o)) if e == f && o == origin => None,
        Some((e, _)) if e == f => Some(true),
        _ => Some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(from: u32, to: u32, origin: f64) -> PlayRun {
        PlayRun { from, to, origin }
    }

    #[test]
    fn runs_out_of_order_overlapping_empty_or_too_many_are_refused() {
        let mut o = OnTrack::default();
        assert!(o.tell(&[run(0, 10, 0.0), run(10, 20, 5.0)]));
        assert_eq!(o.take().runs(), &[run(0, 10, 0.0), run(10, 20, 5.0)]);
        assert!(o.take().runs().is_empty(), "taken once");
        assert!(o.tell(&[run(0, 10, 0.0)]));
        assert!(!o.tell(&[run(5, 10, 0.0), run(0, 4, 0.0)]));
        assert!(!o.tell(&[run(0, 10, 0.0), run(9, 12, 0.0)]));
        assert!(!o.tell(&[run(3, 3, 0.0)]));
        assert!(!o.tell(&[run(0, 1, f64::NAN)]));
        assert!(!o.tell(&[run(0, 1, 0.0); MAX_PLAY_RUNS + 1]));
        assert_eq!(o.take().runs(), &[run(0, 10, 0.0)], "kept as told");
    }

    #[test]
    fn a_cut_looks_for_a_jump_only_where_it_should() {
        assert_eq!(first_of(Some((100, 0.0)), 100, 0.0), None);
        assert_eq!(first_of(Some((100, 0.0)), 100, 960.0), Some(true));
        assert_eq!(first_of(Some((100, 0.0)), 140, 0.0), Some(false));
        assert_eq!(first_of(None, 0, 0.0), Some(false));
    }
}
