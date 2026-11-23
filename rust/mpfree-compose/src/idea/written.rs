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

/// The home note a bass is written from, in C: C2.
const HOME_BASS: u8 = 36;
/// The home note a riff, chords and a line are written from, in C:
/// middle C.
const HOME: u8 = 60;

/// The home note an idea is written from: the one its kind is made
/// around, C2 for a bass and middle C for the rest. The composer chooses
/// each kind's register itself (a bass around G2, a riff and chords
/// around middle C, a line a little over it), so the home note is known
/// and is not read off the notes: every idea of a kind sits in one place
/// from the held note, where MPFree's own phrases of that kind sit from
/// theirs, and Try again never comes back an octave away.
///
/// Two rules that read it off the notes were tried first, and measured
/// (docs/dev/design/compose.md, "The home note"): the home note sounded
/// longest hung riffs and lines an octave under the held note, and the
/// highest one a phrase dips no more than a fourth under threw a bass an
/// octave up for one low note.
pub(super) fn home_of(kind: PhraseKind, key: Key) -> u8 {
    let c = if kind == PhraseKind::Bass {
        HOME_BASS
    } else {
        HOME
    };
    c + key.tonic % 12
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
    fn the_home_note_is_the_one_a_kind_is_made_around() {
        let c = Key::new(0, Mode::Minor);
        assert_eq!(home_of(PhraseKind::Bass, c), 36);
        for kind in [PhraseKind::Riff, PhraseKind::Chords, PhraseKind::Line] {
            assert_eq!(home_of(kind, c), 60);
        }
        // In another key, that key's home note.
        assert_eq!(home_of(PhraseKind::Bass, Key::new(9, Mode::Minor)), 45);
    }

    #[test]
    fn ideas_sit_where_mpfree_s_own_phrases_sit_from_the_held_note() {
        use crate::idea::{new_idea, NewIdea};
        use crate::style::STYLES;
        // MPFree's own lie from a fourth under the home note (a bass
        // more often than the rest) to two octaves over it, their middle
        // from just under it to a little over an octave above. An idea
        // lies in the same stretch, give or take a third, whatever its
        // style, scale or length, and the middles of all of a kind's
        // within an octave and a half of each other: none is an octave
        // away from where the last one was.
        for kind in PhraseKind::ALL {
            let mut middles: Vec<f64> = Vec::new();
            for style in STYLES {
                for minor in [true, false] {
                    for bars in [1u8, 2, 4] {
                        for seed in 1..=4u64 {
                            let idea = new_idea(&NewIdea {
                                kind,
                                style,
                                minor,
                                bars,
                                activity: 0.5,
                                seed,
                                attempt: 0,
                            });
                            let from: Vec<i32> = idea
                                .events
                                .iter()
                                .map(|e| i32::from(e.pitch) - i32::from(idea.home))
                                .collect();
                            let (low, high) = (
                                from.iter().copied().min().unwrap_or(0),
                                from.iter().copied().max().unwrap_or(0),
                            );
                            assert!(
                                (-9..=28).contains(&low) && (-9..=28).contains(&high),
                                "{kind:?} {} {bars} bars seed {seed}: {low} to {high} from its home note",
                                style.name
                            );
                            middles
                                .push(from.iter().sum::<i32>() as f64 / from.len().max(1) as f64);
                        }
                    }
                }
            }
            let (low, high) = middles
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, h), &m| (l.min(m), h.max(m)));
            assert!(
                low >= -4.0 && high <= 18.0 && high - low <= 18.0,
                "{kind:?}: middles from {low:.1} to {high:.1}"
            );
        }
    }

    /// Where phrases sit from their home note, in semitones: the lowest
    /// note, the middle (each note counted once) and the highest, for
    /// MPFree's own phrases and for ideas, by kind. `cargo test -p
    /// mpfree-compose --lib print_where_phrases_sit -- --ignored
    /// --nocapture`
    #[test]
    #[ignore]
    fn print_where_phrases_sit() {
        use crate::idea::{new_idea, simple_idea, NewIdea};
        use crate::style::STYLES;
        use mpfree_arp::Library;
        let sits = |p: &Phrase| {
            let semis: Vec<i32> = p.notes().iter().map(|n| i32::from(n.semis)).collect();
            let n = semis.len().max(1) as f64;
            (
                semis.iter().copied().min().unwrap_or(0),
                semis.iter().sum::<i32>() as f64 / n,
                semis.iter().copied().max().unwrap_or(0),
            )
        };
        let library = Library::built_in();
        for kind in PhraseKind::ALL {
            let mut rows: Vec<(i32, f64, i32)> = library
                .phrases()
                .filter(|(_, p)| p.kind == kind)
                .map(|(_, p)| sits(p))
                .collect();
            let show = |what: &str, rows: &mut Vec<(i32, f64, i32)>| {
                rows.sort_by(|a, b| a.1.total_cmp(&b.1));
                let n = rows.len().max(1);
                let mean = rows.iter().map(|r| r.1).sum::<f64>() / n as f64;
                let under = rows.iter().filter(|r| r.1 < 0.0).count();
                println!(
                    "{} {what}: {} phrases, middle {mean:.1} on average ({:.1} to {:.1}), {under} with the middle under the home note; lowest {} to {}, highest {} to {}",
                    kind.label(),
                    rows.len(),
                    rows.first().map_or(0.0, |r| r.1),
                    rows.last().map_or(0.0, |r| r.1),
                    rows.iter().map(|r| r.0).min().unwrap_or(0),
                    rows.iter().map(|r| r.0).max().unwrap_or(0),
                    rows.iter().map(|r| r.2).min().unwrap_or(0),
                    rows.iter().map(|r| r.2).max().unwrap_or(0),
                );
            };
            show("MPFree's own", &mut rows);
            for (what, make) in [
                ("ideas", new_idea as fn(&NewIdea) -> crate::idea::Idea),
                ("the simple way", simple_idea),
            ] {
                let mut rows = Vec::new();
                for style in STYLES {
                    for minor in [true, false] {
                        for bars in [1u8, 2, 4] {
                            for seed in 1..=10u64 {
                                let idea = make(&NewIdea {
                                    kind,
                                    style,
                                    minor,
                                    bars,
                                    activity: 0.5,
                                    seed,
                                    attempt: 0,
                                });
                                rows.push(sits(&idea.phrase()));
                            }
                        }
                    }
                }
                show(what, &mut rows);
                // Where they lie as made, before a home note is chosen.
                let mut abs: Vec<(i32, f64, i32)> = Vec::new();
                for style in STYLES {
                    for minor in [true, false] {
                        for bars in [1u8, 2, 4] {
                            for seed in 1..=10u64 {
                                let idea = make(&NewIdea {
                                    kind,
                                    style,
                                    minor,
                                    bars,
                                    activity: 0.5,
                                    seed,
                                    attempt: 0,
                                });
                                let ps: Vec<i32> =
                                    idea.events.iter().map(|e| i32::from(e.pitch)).collect();
                                abs.push((
                                    *ps.iter().min().unwrap(),
                                    ps.iter().sum::<i32>() as f64 / ps.len() as f64,
                                    *ps.iter().max().unwrap(),
                                ));
                            }
                        }
                    }
                }
                abs.sort_by(|a, b| a.1.total_cmp(&b.1));
                let q = |f: f64| abs[((abs.len() - 1) as f64 * f) as usize];
                println!(
                    "    as made: lowest {} to {}, highest {} to {}; middle {:.1} at the lowest, {:.1} at a quarter, {:.1} at the half, {:.1} at three quarters, {:.1} at the highest",
                    abs.iter().map(|r| r.0).min().unwrap(),
                    abs.iter().map(|r| r.0).max().unwrap(),
                    abs.iter().map(|r| r.2).min().unwrap(),
                    abs.iter().map(|r| r.2).max().unwrap(),
                    q(0.0).1, q(0.25).1, q(0.5).1, q(0.75).1, q(1.0).1
                );
            }
        }
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
