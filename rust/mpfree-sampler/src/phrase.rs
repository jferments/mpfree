//! Chop to a phrase: a chop's slices as hits in time, in their order and
//! where they fell in the sound, so a sequence can replay the loop intact
//! at any tempo (each slice plays as recorded; only the gaps between them
//! move) with every hit free to be moved, muted, retuned or swapped.
//!
//! The phrase is in beats of the sound's own tempo. Turning it into a
//! clip of the time engine (ticks, notes, a track's slot) is the caller's.

use crate::model::{ChopSet, ModelError};

/// One slice's hit: slice `slice` of the chop (counted from the first one
/// in the phrase), `beat` beats after the phrase starts, lasting `beats`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub slice: usize,
    pub beat: f64,
    pub beats: f64,
}

impl Hit {
    /// Where the hit starts at `ppq` ticks a beat.
    pub fn tick(&self, ppq: u32) -> u64 {
        (self.beat * f64::from(ppq)).round() as u64
    }

    /// How long it lasts at `ppq` ticks a beat, at least one tick.
    pub fn ticks(&self, ppq: u32) -> u64 {
        ((self.beats * f64::from(ppq)).round() as u64).max(1)
    }
}

/// A chop's slices as hits, and how long the phrase is in whole beats.
#[derive(Clone, Debug, PartialEq)]
pub struct Phrase {
    pub hits: Vec<Hit>,
    pub beats: u32,
}

/// Within this much of a whole beat, a phrase's length is that beat: a
/// loop cut a few frames short or long is still its bars.
const BEAT_SLACK: f64 = 0.05;

/// Slices `first..first + count` of `c` as a phrase, at `frames_per_beat`
/// (the sound's tempo). Its length is the slices' span in beats, rounded
/// to the nearest whole beat when it is that close, else up.
pub fn phrase(
    c: &ChopSet,
    first: usize,
    count: usize,
    frames_per_beat: f64,
) -> Result<Phrase, ModelError> {
    if count == 0 || first + count > c.slices() {
        return Err(ModelError::BadSlice);
    }
    if !frames_per_beat.is_finite() || frames_per_beat < 1.0 {
        return Err(ModelError::BadSlice);
    }
    let bounds = |k: usize| c.bounds(k).ok_or(ModelError::BadSlice);
    let origin = bounds(first)?.0 as f64;
    let beats = |f: u64| (f as f64 - origin) / frames_per_beat;
    let hits = (first..first + count)
        .map(|k| {
            let (a, b) = bounds(k)?;
            Ok(Hit {
                slice: k - first,
                beat: beats(a),
                beats: (b - a) as f64 / frames_per_beat,
            })
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    let span = beats(bounds(first + count - 1)?.1);
    let whole = if (span - span.round()).abs() < BEAT_SLACK {
        span.round()
    } else {
        span.ceil()
    };
    Ok(Phrase {
        hits,
        beats: whole.max(1.0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn source(frames: usize) -> Source {
        Source::new("loop", 44_100, 1, vec![0.1; frames]).unwrap()
    }

    #[test]
    fn a_grid_of_eighths_is_a_hit_on_every_eighth() {
        // Two bars at 120 BPM: 22050 frames a beat.
        let s = source(8 * 22_050);
        let c = ChopSet::grid("loop", &s, 16).unwrap();
        let p = phrase(&c, 0, 16, 22_050.0).unwrap();
        assert_eq!(p.beats, 8);
        for (k, h) in p.hits.iter().enumerate() {
            assert!((h.beat - k as f64 * 0.5).abs() < 1e-9, "{h:?}");
            assert!((h.beats - 0.5).abs() < 1e-9);
            assert_eq!(h.slice, k);
        }
        assert_eq!((p.hits[3].tick(96), p.hits[3].ticks(96)), (144, 48));
    }

    #[test]
    fn a_phrase_starts_at_its_first_slice_and_lasts_whole_beats() {
        let s = source(8 * 22_050 - 300);
        let c = ChopSet::grid("loop", &s, 8).unwrap();
        // Slices 3 to 6: four beats, less a few frames.
        let p = phrase(&c, 2, 4, 22_050.0).unwrap();
        assert_eq!(p.hits[0].beat, 0.0);
        assert_eq!(p.hits.len(), 4);
        assert_eq!(p.beats, 4, "a few frames short is still 4 beats");
        // Half a beat over rounds up to the next beat.
        let p = phrase(&c, 0, 8, 22_050.0 * 7.5 / 8.0).unwrap();
        assert_eq!(p.beats, 9);
        assert!(phrase(&c, 6, 3, 22_050.0).is_err());
        assert!(phrase(&c, 0, 0, 22_050.0).is_err());
        assert!(phrase(&c, 0, 2, 0.0).is_err());
    }
}
