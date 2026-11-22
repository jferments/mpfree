//! What Keep writes: events as clip notes.
//!
//! Keep is one transaction in the app (phase 1, through the same path as
//! step edits); this is the part that turns events into the notes it
//! writes, all or nothing. The notes carry the groove and playing offsets
//! ([`crate::timing::written`]) and nothing of playback's swing, so a
//! kept clip plays exactly as its preview did.

use mpfree_seq::{Note, MAX_CLIP_TICKS, MAX_NOTES};
use mpfree_transport::time::Tick;

use crate::event::Event;
use crate::timing::written;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepError {
    /// More notes than a clip holds.
    TooManyNotes { notes: usize },
    /// Longer than a clip can be.
    ClipTooLong,
}

impl KeepError {
    pub fn words(self) -> String {
        match self {
            KeepError::TooManyNotes { notes } => {
                format!("Too many notes to keep ({notes}; a clip holds {MAX_NOTES})")
            }
            KeepError::ClipTooLong => "Too long to keep as one clip".to_string(),
        }
    }
}

/// The clip notes for `events` in a clip of `clip_len` ticks at `bpm`,
/// sorted by tick and pitch, or why they can't be kept. Nothing is
/// written unless all of them can be.
pub fn clip_notes(events: &[Event], bpm: f64, clip_len: Tick) -> Result<Vec<Note>, KeepError> {
    if clip_len > MAX_CLIP_TICKS {
        return Err(KeepError::ClipTooLong);
    }
    if events.len() > MAX_NOTES {
        return Err(KeepError::TooManyNotes {
            notes: events.len(),
        });
    }
    let mut notes: Vec<Note> = events
        .iter()
        .map(|e| {
            Note::new(
                written(e.at, e.timing, bpm, clip_len),
                e.len.max(1),
                e.pitch,
                e.velocity.clamp(1, 127),
            )
        })
        .collect();
    notes.sort_by_key(|n| (n.tick, n.note));
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Role, Source};
    use crate::timing::Timing;
    use mpfree_context::chord::Tone;

    fn ev(at: Tick, pitch: u8) -> Event {
        Event {
            at,
            len: 240,
            pitch,
            velocity: 0,
            role: Role::Chord(Tone::Root),
            source: Source::Mpfree,
            timing: Timing::default(),
        }
    }

    #[test]
    fn all_or_nothing_inside_the_clip_limits() {
        let notes = clip_notes(&[ev(480, 60), ev(0, 64), ev(0, 60)], 120.0, 3840).unwrap();
        let order: Vec<(Tick, u8)> = notes.iter().map(|n| (n.tick, n.note)).collect();
        assert_eq!(order, vec![(0, 60), (0, 64), (480, 60)]);
        assert_eq!(notes[0].velocity, 1, "velocity 0 is kept as 1");
        assert_eq!(
            clip_notes(&[], 120.0, MAX_CLIP_TICKS + 1),
            Err(KeepError::ClipTooLong)
        );
        let many = vec![ev(0, 60); MAX_NOTES + 1];
        assert_eq!(
            clip_notes(&many, 120.0, 3840),
            Err(KeepError::TooManyNotes {
                notes: MAX_NOTES + 1
            })
        );
    }
}
