//! The timing contract (contract 3 of the design).
//!
//! Every event mpfree makes has its time built in one order:
//!
//! 1. its musical position, in ticks on the clip's grid;
//! 2. its groove offset: the drummer's residual timing at that step
//!    ([`GrooveTemplate`], already free of swing);
//! 3. its playing offset: small expressive shifts from the playing rules;
//!    2 and 3 together stay within ±[`LIMIT_MS`];
//! 4. playback's own quantise, speed and swing, as the play view applies
//!    them to every clip.
//!
//! Steps 1 to 3 are written into the clip ([`written`]); step 4 is
//! playback's ([`heard`]). The preview and Keep both play written notes
//! through playback, so what the preview plays is what Keep writes.
//! [`heard`] follows `mpfree-play` (`ClipView::note_view`, then the
//! player's run collection) so tests here can check where a note lands;
//! phase 0B checks it against the real play view.

use mpfree_context::groove::{unswing, GrooveTemplate, Lane, STEP};
use mpfree_seq::{Quantise, Speed};
use mpfree_transport::time::{Swing, Tick, PPQ};

/// How far the groove and playing offsets together may move a note.
pub const LIMIT_MS: f64 = 30.0;

/// Ticks in `ms` milliseconds at `bpm`.
pub fn ms_to_ticks(ms: f64, bpm: f64) -> f64 {
    ms * bpm * f64::from(PPQ) / 60_000.0
}

/// An event's offsets from its musical position.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Timing {
    /// The groove's residual, as a share of a 1/16 step.
    pub groove: f32,
    /// The playing rules' shift, in ticks.
    pub play: f32,
}

impl Timing {
    /// The groove at a step for a note following `lane`, against the swing
    /// playback will apply. A template read against another swing is
    /// moved onto this one first, so the drummer's notes keep landing
    /// where they were played.
    pub fn from_groove(
        groove: &GrooveTemplate,
        lane: Lane,
        step: usize,
        playback: Swing,
    ) -> Timing {
        let cell = if groove.swing == playback {
            groove.cell(lane, step)
        } else {
            groove.against(playback).cell(lane, step)
        };
        Timing {
            groove: cell.offset,
            play: 0.0,
        }
    }

    /// The two offsets together, in ticks, within ±[`LIMIT_MS`].
    pub fn offset_ticks(self, bpm: f64) -> f64 {
        let limit = ms_to_ticks(LIMIT_MS, bpm);
        (f64::from(self.groove) * STEP as f64 + f64::from(self.play)).clamp(-limit, limit)
    }
}

/// The tick a note is written at in a clip of `clip_len` ticks: its
/// musical position plus its offsets, rounded to a tick. A note pushed
/// before the clip's start is written at its end, where playback plays it
/// just before the next pass, as quantise does with a note at the end.
pub fn written(at: Tick, timing: Timing, bpm: f64, clip_len: Tick) -> Tick {
    let t = at + timing.offset_ticks(bpm).round() as Tick;
    t.rem_euclid(clip_len.max(1))
}

/// How a clip plays: its quantise, speed and swing, and the song tick its
/// pass starts on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub quantise: Option<Quantise>,
    pub speed: Speed,
    /// The clip's swing, else its track's, else the transport's.
    pub swing: Swing,
    /// One pass in the clip's own ticks.
    pub length: Tick,
    /// The song tick the pass starts on.
    pub start: Tick,
}

impl Playback {
    pub fn plain(length: Tick) -> Playback {
        Playback {
            quantise: None,
            speed: Speed::NORMAL,
            swing: Swing::default(),
            length,
            start: 0,
        }
    }
}

/// The song tick a written note is heard at: quantise, then speed, then
/// swing, in the order the play view and player apply them.
pub fn heard(written: Tick, pb: &Playback) -> f64 {
    let at = pb
        .quantise
        .map_or(written as f64, |q| q.apply(written as f64));
    let local = at.rem_euclid(pb.length.max(1) as f64);
    let straight = pb.start as f64 + pb.speed.to_song(local);
    pb.swing.apply(straight)
}

/// The rounded tick the player schedules a written note on.
pub fn heard_tick(written: Tick, pb: &Playback) -> Tick {
    heard(written, pb).round() as Tick
}

/// Something about a clip's playback that changes what the groove does,
/// said before Keep rather than discovered after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingNote {
    /// The clip's quantise pulls notes towards its grid, groove and all.
    QuantisePullsGroove { strength: u8 },
}

impl TimingNote {
    pub fn words(self) -> String {
        match self {
            TimingNote::QuantisePullsGroove { strength: 100 } => {
                "This clip's quantise puts the new notes on the grid, so they lose the groove".to_string()
            }
            TimingNote::QuantisePullsGroove { strength } => format!(
                "This clip's quantise ({strength} %) pulls the new notes towards the grid, so they lose some of the groove"
            ),
        }
    }
}

/// What to say about a clip's playback before writing grooved notes.
pub fn check(pb: &Playback) -> Option<TimingNote> {
    match pb.quantise {
        Some(q) if q.strength > 0 => Some(TimingNote::QuantisePullsGroove {
            strength: q.strength,
        }),
        _ => None,
    }
}

/// A style's slow shared drift is used only where the style asks for it
/// and no played groove was read: the drummer's feel wins.
pub fn drift_on(style_drift_ms: f32, groove: Option<&GrooveTemplate>) -> bool {
    style_drift_ms > 0.0 && !groove.is_some_and(|g| g.played)
}

/// Where a step's swung line falls, unswung: for tests and the reading.
pub fn straight_of(heard: f64, swing: Swing) -> f64 {
    unswing(swing, heard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_stay_within_thirty_ms() {
        let t = Timing {
            groove: 0.5,
            play: 40.0,
        };
        let limit = ms_to_ticks(30.0, 92.0);
        assert!((t.offset_ticks(92.0) - limit).abs() < 1e-9);
        let early = Timing {
            groove: -0.5,
            play: 0.0,
        };
        assert!((early.offset_ticks(92.0) + limit).abs() < 1e-9);
    }

    #[test]
    fn a_note_pushed_before_the_start_is_written_at_the_end() {
        let early = Timing {
            groove: -0.05,
            play: 0.0,
        };
        let len = 4 * PPQ as Tick;
        assert_eq!(written(0, early, 120.0, len), len - 12);
        // Heard just before the next pass starts.
        let pb = Playback::plain(len);
        assert_eq!(heard_tick(len - 12, &pb), len - 12);
    }

    #[test]
    fn playback_applies_quantise_then_speed_then_swing() {
        let pb = Playback {
            quantise: Quantise::new(240, 50),
            speed: Speed::new(1, 2).unwrap(),
            swing: Swing::new(240, 66).unwrap(),
            length: 3840,
            start: 3840,
        };
        // 250 quantised half way to 240 is 245; at half speed 490 song
        // ticks; from 3840 that's 4330, in the pair 4320..4800 on its
        // straight half, swung by 66/50.
        let expect = 4320.0 + 10.0 * (240.0 * 66.0 / 50.0) / 240.0;
        assert!((heard(250, &pb) - expect).abs() < 1e-9);
        assert_eq!(
            check(&pb),
            Some(TimingNote::QuantisePullsGroove { strength: 50 })
        );
        assert_eq!(check(&Playback::plain(3840)), None);
        assert!(TimingNote::QuantisePullsGroove { strength: 50 }
            .words()
            .contains("50 %"));
    }

    #[test]
    fn drift_only_where_asked_and_never_over_a_played_groove() {
        let mut g = GrooveTemplate::straight(16, Swing::default());
        assert!(drift_on(4.0, None));
        assert!(drift_on(4.0, Some(&g)));
        assert!(!drift_on(0.0, None), "Minimal has none");
        g.played = true;
        assert!(!drift_on(4.0, Some(&g)));
    }
}
