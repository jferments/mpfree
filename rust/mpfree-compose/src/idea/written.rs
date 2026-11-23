//! An idea as a phrase writes notes: on its grid, as long as its steps
//! with the last one short, plain or held, and plain, accented or a
//! ghost. [`Idea::phrase`] is the one place an idea becomes the arp's
//! [`Phrase`].

use mpfree_arp::phrase::{ACCENT_VELOCITY, GATE_SHORT, GATE_WRITTEN, PLAIN_VELOCITY};
use mpfree_arp::{Phrase, PhraseKind, Rate, Written};
use mpfree_context::groove::STEP;
use mpfree_context::key::Key;
use mpfree_transport::time::Tick;

use super::{Idea, GHOST_VELOCITY, METER};
use crate::event::{Event, Role};
use crate::row::Row;

/// The notes as a phrase writes them, and the grid they are written on:
/// 1/8 when it swings or every note starts on one, else 1/16. A note off
/// the grid (a ghost on the 1/16 before a swung beat) is left out. Each
/// note lasts its whole steps with the last one plain, short when it was
/// made shorter than half a step, held when it was made to reach the next
/// note (the last one, to the phrase's end); it is plain, accented where
/// `accents` says, or a ghost.
pub(super) fn written(
    mut events: Vec<Event>,
    accents: &[Tick],
    swung: bool,
    row: &Row,
) -> (Vec<Event>, Rate) {
    let eighth = 2 * STEP;
    if swung {
        events.retain(|e| e.at % eighth == 0);
    }
    let on_eighths = events.iter().all(|e| e.at % eighth == 0);
    let (grid, unit) = if swung || on_eighths {
        (Rate::Eighth, eighth)
    } else {
        (Rate::Sixteenth, STEP)
    };
    events.sort_by_key(|e| (e.at, e.pitch));
    let mut starts: Vec<Tick> = events.iter().map(|e| e.at).collect();
    starts.dedup();
    for e in &mut events {
        // To the next note, or for the last one to the phrase's end: a
        // phrase's text cannot say a note that hangs over its end, so one
        // written longer came back shorter once it was kept.
        let next = starts
            .iter()
            .copied()
            .find(|&t| t > e.at)
            .unwrap_or(row.length);
        let gap = (next - e.at).max(unit);
        let ghost = e.role == Role::Ghost;
        e.len = if ghost || e.len * 2 < unit {
            unit * Tick::from(GATE_SHORT) / 100
        } else if e.len * 10 >= gap * 9 {
            gap
        } else {
            let steps = ((e.len + unit / 5 + unit / 2) / unit).clamp(1, gap / unit);
            (steps - 1) * unit + unit * Tick::from(GATE_WRITTEN) / 100
        };
        e.velocity = if ghost {
            GHOST_VELOCITY
        } else if accents.contains(&e.at) {
            ACCENT_VELOCITY
        } else {
            PLAIN_VELOCITY
        };
    }
    (events, grid)
}

/// The home note an idea is written from. Chords are shapes over the
/// held note, so theirs is the home note under the first chord's lowest
/// note. Any other kind's is the home note it sounds longest, in the
/// octave it sounds it (the lower of two as long), so the held note plays
/// where the phrase's weight is; one that never plays the home note is
/// written from the one under its lowest note.
pub(super) fn home_of(kind: PhraseKind, events: &[Event], key: Key) -> u8 {
    let tonic = i32::from(key.tonic % 12);
    let under = |p: i32| (p - (p - tonic).rem_euclid(12)).clamp(0, 127) as u8;
    let Some(first) = events.first() else {
        return under(60);
    };
    if kind == PhraseKind::Chords {
        let low = events
            .iter()
            .filter(|e| e.at == first.at)
            .map(|e| i32::from(e.pitch))
            .min()
            .unwrap_or(60);
        return under(low);
    }
    let mut held = [0i64; 128];
    for e in events {
        if i32::from(e.pitch).rem_euclid(12) == tonic {
            held[usize::from(e.pitch)] += e.len;
        }
    }
    let most = held.iter().copied().max().unwrap_or(0);
    if most > 0 {
        if let Some(p) = (0..128).find(|&p| held[p] == most) {
            return p as u8;
        }
    }
    let low = events
        .iter()
        .map(|e| i32::from(e.pitch))
        .min()
        .unwrap_or(60);
    under(low)
}

impl Idea {
    /// The idea as the arp's phrase: its notes from its home note. It is
    /// written for either scale when every note is the home note, its
    /// 2nd, 4th or 5th, which major and minor share.
    pub fn phrase(&self) -> Phrase {
        let notes: Vec<(u32, u32, u8, u8)> = self
            .events
            .iter()
            .map(|e| (e.at.max(0) as u32, e.len.max(1) as u32, e.pitch, e.velocity))
            .collect();
        let shared = notes.iter().all(|&(_, _, p, _)| {
            matches!(
                (i32::from(p) - i32::from(self.home)).rem_euclid(12),
                0 | 2 | 5 | 7
            )
        });
        let written = if shared {
            Written::Either
        } else if self.key.mode.minorish() {
            Written::Minor
        } else {
            Written::Major
        };
        let mut p = Phrase::from_notes(&notes, self.home, written, self.grid, METER.beats);
        p.kind = self.kind;
        p.bars = self.bars;
        p.swung = self.swung;
        p
    }
}

#[cfg(test)]
mod tests {
    use super::super::{hear, key_of};
    use super::*;
    use mpfree_context::chord::{ChordTimeline, Tone};
    use mpfree_context::key::Mode;

    use crate::event::Source;
    use crate::timing::Timing;

    fn note(at: Tick, len: Tick, pitch: u8) -> Event {
        Event {
            at,
            len,
            pitch,
            velocity: 90,
            role: Role::Chord(Tone::Root),
            source: Source::Mpfree,
            timing: Timing::default(),
        }
    }

    fn one_bar() -> Row {
        hear(key_of(true), &ChordTimeline::default(), &[], 1)
    }

    #[test]
    fn a_note_is_written_short_plain_or_held() {
        let row = one_bar();
        let made = vec![
            // Three tenths of a step: short.
            note(0, STEP * 3 / 10, 60),
            // Most of its step, with a note on the next: plain.
            note(STEP, STEP * 8 / 10, 62),
            // All the way to the next note: held.
            note(2 * STEP, 2 * STEP, 63),
            // Nearly two steps of the four to the next: two, the last plain.
            note(4 * STEP, STEP * 18 / 10, 65),
            // The last note, to the phrase's end and no further.
            note(8 * STEP, 12 * STEP, 67),
        ];
        let (w, grid) = written(made, &[], false, &row);
        assert_eq!(grid, Rate::Sixteenth);
        let lens: Vec<Tick> = w.iter().map(|e| e.len).collect();
        assert_eq!(
            lens,
            vec![
                STEP / 4,
                STEP * 4 / 5,
                2 * STEP,
                STEP + STEP * 4 / 5,
                8 * STEP
            ]
        );
        assert!(w.iter().all(|e| e.velocity == PLAIN_VELOCITY));
    }

    #[test]
    fn an_accent_and_a_ghost_are_written_as_a_phrase_has_them() {
        let row = one_bar();
        let mut ghost = note(STEP, STEP, 60);
        ghost.role = Role::Ghost;
        let made = vec![note(0, STEP, 60), ghost, note(4 * STEP, 2 * STEP, 67)];
        let (w, _) = written(made, &[4 * STEP], false, &row);
        assert_eq!(
            w.iter().map(|e| e.velocity).collect::<Vec<_>>(),
            vec![PLAIN_VELOCITY, GHOST_VELOCITY, ACCENT_VELOCITY]
        );
        // A ghost is short however long it was made.
        assert_eq!(w[1].len, STEP / 4);
    }

    #[test]
    fn notes_all_on_eighths_are_written_on_eighths() {
        let row = one_bar();
        let made = vec![note(0, STEP, 60), note(2 * STEP, STEP / 2, 62)];
        let (w, grid) = written(made, &[], false, &row);
        assert_eq!(grid, Rate::Eighth);
        // A step of an 1/8: half of one is plain, a quarter short.
        assert_eq!(w[0].len, 2 * STEP * 4 / 5);
        assert_eq!(w[1].len, 2 * STEP / 4);
    }

    #[test]
    fn a_swung_phrase_leaves_out_what_falls_between_its_eighths() {
        let row = one_bar();
        let mut ghost = note(3 * STEP, STEP / 2, 60);
        ghost.role = Role::Ghost;
        let made = vec![note(0, 2 * STEP, 60), ghost, note(4 * STEP, 2 * STEP, 63)];
        let (w, grid) = written(made, &[], true, &row);
        assert_eq!(grid, Rate::Eighth);
        assert_eq!(
            w.iter().map(|e| e.at).collect::<Vec<_>>(),
            vec![0, 4 * STEP]
        );
    }

    #[test]
    fn the_home_note_is_where_the_phrase_weighs() {
        let key = Key::new(0, Mode::Minor);
        // The home note it sounds longest, in the octave it sounds it.
        let riff = [
            note(0, STEP, 60),
            note(STEP, 4 * STEP, 72),
            note(6 * STEP, STEP, 67),
        ];
        assert_eq!(home_of(PhraseKind::Riff, &riff, key), 72);
        // Of two as long, the lower.
        let even = [note(0, STEP, 72), note(STEP, STEP, 60)];
        assert_eq!(home_of(PhraseKind::Bass, &even, key), 60);
        // It never plays the home note: the one under its lowest note.
        let off = [note(0, STEP, 63), note(STEP, STEP, 67)];
        assert_eq!(home_of(PhraseKind::Line, &off, key), 60);
        // Chords are shapes over the held note: under the first chord's
        // lowest note, whatever sounds longest later.
        let chords = [
            note(0, STEP, 63),
            note(0, STEP, 67),
            note(0, STEP, 72),
            note(4 * STEP, 8 * STEP, 48),
        ];
        assert_eq!(home_of(PhraseKind::Chords, &chords, key), 60);
        // Nothing at all: middle C's home note.
        assert_eq!(home_of(PhraseKind::Riff, &[], key), 60);
    }

    #[test]
    fn a_phrase_of_notes_major_and_minor_share_is_written_for_either() {
        use crate::idea::{new_idea, NewIdea};
        use crate::style::STYLES;
        // Octaves on the home note: every style's first one-bar lines
        // hold some. Whatever they are, the rule is the phrase's own.
        for seed in 1..=12 {
            let idea = new_idea(&NewIdea {
                kind: PhraseKind::Line,
                style: STYLES[0],
                minor: true,
                bars: 1,
                activity: 0.5,
                seed,
                attempt: 0,
            });
            let p = idea.phrase();
            let shared = p
                .notes()
                .iter()
                .all(|n| matches!(i32::from(n.semis).rem_euclid(12), 0 | 2 | 5 | 7));
            assert_eq!(p.written == Written::Either, shared, "{}", p.to_text());
            if !shared {
                assert_eq!(p.written, Written::Minor);
            }
        }
    }
}
