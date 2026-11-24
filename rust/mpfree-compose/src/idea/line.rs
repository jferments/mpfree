//! A line: a pulse that moves. One beat's rhythm on every beat, and a
//! figure of chord notes that starts again in every bar and follows each
//! chord, a note of the chord higher from the plan's landing and from its
//! high point to the end of their bars. A figure rises or falls, and
//! starts on the chord's root, third or fifth.

use mpfree_arp::phrase::PLAIN_VELOCITY;
use mpfree_context::chord::{Chord, Tone};
use mpfree_context::groove::STEP;
use mpfree_transport::time::Tick;

use super::{dice, moves_off, Draft, Given, IdeaVerdict, Made, NewIdea};
use crate::cells::{lengths, starts_of, Stroke};
use crate::critic::{copy_bars, judge, Check, Heard as CriticHeard};
use crate::dice::{Dice, Job};
use crate::event::{Event, Role, Source};
use crate::motif::line as grid_line;
use crate::parts::chord_near;
use crate::realize::PlainNote;
use crate::row::Row;
use crate::settings::{Ending, Settings};
use crate::sketch::AnchorKind;
use crate::style::{Feel, Style};
use crate::timing::Timing;

/// Where a line sits.
const LINE_LOW: i32 = 55;
const LINE_HIGH: i32 = 86;

/// The chord's notes going up from `base`: note `i` of that ladder (the
/// base is note 0; below it for a negative `i`). `triad` leaves the
/// seventh off the ladder, so three places up is the octave whatever the
/// chord.
fn ladder(chord: Chord, base: i32, i: i32, triad: bool) -> i32 {
    let on = |p: i32| match chord.tone_of(p) {
        Some(Tone::Seventh) => !triad,
        t => t.is_some(),
    };
    let mut p = base;
    let step = if i < 0 { -1 } else { 1 };
    let mut left = i.abs();
    while left > 0 {
        p += step;
        if on(p) {
            left -= 1;
        }
        if !(0..=127).contains(&p) {
            break;
        }
    }
    p
}

/// The figure a line plays.
struct Figure {
    /// Its notes, as places on a chord's ladder.
    places: Vec<i32>,
    /// The ladder is the triad's: the wide figure and the octaves are
    /// drawn for three notes to the octave, and on a seventh chord's four
    /// they came out as root and seventh by turns.
    triad: bool,
    /// The note of the first chord it is built on: its root (0), third
    /// (1) or fifth (2).
    from: i32,
}

impl Figure {
    /// A figure of `len` notes. One in three falls. One in two starts off
    /// the root, on the third or (more often) the fifth; the octaves keep
    /// to the root, which is what they are for. Without the two, a
    /// one-bar line over the home chord had some ten figures in a swung
    /// style, and Try again ran out of new ones.
    fn new(len: usize, dice: &mut Dice) -> Figure {
        let kind = if len == 3 {
            dice.below(2)
        } else {
            2 + dice.pick(&[2.0, 2.0, 1.5, 1.5]).unwrap_or(0)
        };
        let mut places = rising(kind, len);
        if dice.chance(1.0 / 3.0) {
            fall(&mut places);
        }
        let from = match dice.pick(&[3.0, 1.0, 2.0]) {
            Some(i) if kind != OCTAVES => i as i32,
            _ => 0,
        };
        Figure {
            places,
            triad: matches!(kind, WIDE | OCTAVES),
            from,
        }
    }

    fn top(&self) -> i32 {
        self.places.iter().copied().max().unwrap_or(0)
    }
}

/// The figures drawn for three notes to the octave.
const WIDE: usize = 1;
const OCTAVES: usize = 5;

/// Figure `kind` over `len` notes, rising from place 0.
fn rising(kind: usize, len: usize) -> Vec<i32> {
    let n = len as i32;
    (0..n)
        .map(|k| match kind {
            // Three notes rolling: up the chord, or wide.
            0 => k,
            WIDE => 2 * k,
            // Up and back.
            2 => {
                let top = n / 2;
                if k <= top {
                    k
                } else {
                    n - k
                }
            }
            // One note held to, the others climbing round it.
            3 => {
                if k % 2 == 0 {
                    0
                } else {
                    let j = (k + 1) / 2;
                    let top = (n / 4 + 1).max(2);
                    if j <= top {
                        j
                    } else {
                        2 * top - j
                    }
                }
            }
            // In thirds.
            4 => k / 2 + 2 * (k % 2),
            // Low and an octave up by turns, a fifth now and then.
            _ => match k % 4 {
                1 => 3,
                3 if k % 8 == 7 => 2,
                3 => 3,
                _ => 0,
            },
        })
        .collect()
}

/// The same figure falling: its places from the top down, so it sits
/// where its rising twin does and a line that falls needs no room of its
/// own below.
fn fall(places: &mut [i32]) {
    let top = places.iter().copied().max().unwrap_or(0);
    places.iter_mut().for_each(|p| *p = top - *p);
}

/// One draft of a line.
pub(super) fn line(req: &NewIdea, row: &Row, settings: &Settings, swung: bool, d: u16) -> Made {
    line_on(req, row, settings, swung, d, None)
}

/// A line's pulse: one beat's rhythm on every beat, all the 1/16s or some
/// of them, as busy as asked; on swung 1/8s, the beat and its "and".
pub(super) fn pulse(req: &NewIdea, row: &Row, settings: &Settings, dice: &mut Dice) -> Vec<Stroke> {
    let g = row.grid;
    let swing = req.style.rhythm.feel == Feel::Swing8;
    let cell: u8 = if swing || g.beat != 4 {
        // Sparse swings in quarter notes.
        if settings.activity < 0.3 {
            0b0001
        } else {
            0b0101
        }
    } else {
        // 3.25 notes a beat at the middle of Busy, from 2.5 at Sparse
        // (0.2) to 4 at Busy (0.8).
        let want = 3.25 + 2.5 * (f64::from(settings.activity) - 0.5);
        let cells = [0b0101u8, 0b1101, 0b1011, 0b0111, 0b1111];
        let weights: Vec<f64> = cells
            .iter()
            .map(|c| (-1.4 * (f64::from(c.count_ones()) - want).abs()).exp())
            .collect();
        cells[dice.pick(&weights).unwrap_or(4)]
    };
    let mut strokes: Vec<Stroke> = (0..row.steps / g.beat.max(1))
        .flat_map(|b| starts_of(cell, g.beat).map(move |s| Stroke::at(b * g.beat + s)))
        .collect();
    lengths(&mut strokes, row.steps);
    strokes
}

/// Draft `d` of a line, on `given`'s rhythm when there is one (More like
/// this: its rhythm with new notes), else on a pulse of its own.
pub(super) fn line_on(
    req: &NewIdea,
    row: &Row,
    settings: &Settings,
    swung: bool,
    d: u16,
    given: Option<&Given>,
) -> Made {
    let g = row.grid;
    let st = &req.style;
    let mut dice = dice(req, Job::Pitch, d);
    let strokes = match given {
        Some(gv) => gv.strokes.clone(),
        None => pulse(req, row, settings, &mut dice),
    };
    let per_bar = strokes.iter().filter(|s| s.at < g.bar).count().max(1);
    let len = {
        let lens: &[usize] = if per_bar >= 12 {
            &[3, 4, 6, 8]
        } else {
            &[3, 4, 8]
        };
        lens[dice.pick(&[1.0, 2.0, 1.2, 1.6][..lens.len()]).unwrap_or(1)]
    };
    let fig = Figure::new(len, &mut dice);
    let top = fig.top();
    let peak_mark = row.plan.peak().map(|m| m.at);
    let landing = row
        .plan
        .marks
        .iter()
        .find(|m| m.kind == AnchorKind::Landing)
        .map(|m| m.at);
    let first_chord = chord_near(row, 0);
    let mut base = first_chord.map_or(60, |c| {
        let on = [Tone::Root, Tone::Third, Tone::Fifth][fig.from.clamp(0, 2) as usize];
        let in_octave = |t: Tone| (LINE_LOW + 1..=LINE_LOW + 12).find(|&p| c.tone_of(p) == Some(t));
        in_octave(on).or(in_octave(Tone::Root)).unwrap_or(60)
    });
    let mut last_chord = first_chord;
    let mut events: Vec<Event> = Vec::new();
    let mut peak_at: Option<Tick> = None;
    let mut k = 0usize;
    for s in &strokes {
        if s.at % g.bar == 0 {
            // The figure starts again in every bar.
            k = 0;
        }
        let chord = chord_near(row, s.at);
        if chord != last_chord {
            // Onto the new chord by the nearest of its notes, its root
            // when that is as near.
            if let Some(c) = chord {
                base = (base - 6..=base + 6)
                    .filter(|&p| c.tone_of(p).is_some())
                    .min_by_key(|&p| {
                        (
                            (p - base).abs(),
                            p.rem_euclid(12) != i32::from(c.root % 12),
                            p,
                        )
                    })
                    .unwrap_or(base);
            }
            last_chord = chord;
        }
        // From the high point to the end of its bar, a note of the chord
        // higher; the high point itself two, and a place above the
        // figure's top at the least, so it is the highest note whichever
        // note of the figure falls there. From the landing to the end of
        // its bar, one. Four bars alike are then two alike, one that
        // opens and one that peaks.
        let from = |m: Option<i32>| m.is_some_and(|m| s.at >= m && s.at / g.bar == m / g.bar);
        let at = fig.places[k % fig.places.len()];
        let place = if peak_mark == Some(s.at) {
            (at + 2).max(top + 1)
        } else if from(peak_mark) || from(landing) {
            at + 1
        } else {
            at
        };
        let peak = peak_mark == Some(s.at);
        let mut p = match chord {
            Some(c) => ladder(c, base, place, fig.triad),
            None => base,
        };
        while p > LINE_HIGH {
            p -= 12;
        }
        if peak {
            peak_at = Some(row.tick(s.at));
        }
        events.push(Event {
            at: row.tick(s.at),
            len: (Tick::from(s.len) * STEP * 4 / 5).max(STEP / 3),
            pitch: p.clamp(0, 127) as u8,
            velocity: PLAIN_VELOCITY,
            role: match chord.and_then(|c| c.tone_of(p)) {
                Some(t) => Role::Chord(t),
                None => Role::Scale,
            },
            source: Source::Mpfree,
            timing: Timing::default(),
        });
        k += 1;
    }
    // Round the loop: a last note more than a sixth from the first comes
    // to the chord note nearest it.
    if let (Some(first), Some(last)) = (events.first().copied(), events.last_mut()) {
        let (a, b) = (i32::from(first.pitch), i32::from(last.pitch));
        if (a - b).abs() > 9 {
            let c = chord_near(row, row.steps - 1);
            let near = (a - 9..=a + 9)
                .filter(|&p| p != a && c.is_none_or(|c| c.tone_of(p).is_some()))
                .min_by_key(|&p| ((p - a).abs(), p));
            if let Some(p) = near {
                last.pitch = p.clamp(0, 127) as u8;
            }
        }
    }
    let accents = match given {
        Some(gv) => gv.accents.clone(),
        None => peak_at.into_iter().collect(),
    };
    Draft {
        strokes,
        events,
        accents,
    }
    .heard(row, swung, |_, heard| judge_line(row, st, heard))
}

/// A line judged on what a phrase writes of it: it keeps its pulse, plays
/// the chords, has a shape (three different notes), goes somewhere (a
/// high point late; in one bar, off its first note), stays in reach,
/// comes back changed, and joins round the loop.
pub(super) fn judge_line(row: &Row, style: &Style, events: &[Event]) -> IdeaVerdict {
    let g = row.grid;
    let n = events.len().max(1) as f32;
    let beats = (row.steps / g.beat.max(1)).max(1);
    let struck = (0..beats)
        .filter(|&b| {
            let (a, z) = (row.tick(b * g.beat), row.tick((b + 1) * g.beat));
            events.iter().any(|e| a <= e.at && e.at < z)
        })
        .count() as f32
        / beats as f32;
    let pulse = Check::new(
        struck >= 0.95,
        struck,
        format!("{:.0} % of beats struck", struck * 100.0),
    );
    let on = events
        .iter()
        .filter(|e| matches!(e.role, Role::Chord(_)))
        .count() as f32
        / n;
    let chords = Check::new(
        on >= 0.85,
        on,
        format!("{:.0} % of its notes on the chord", on * 100.0),
    );
    let mut different: Vec<u8> = events.iter().map(|e| e.pitch).collect();
    different.sort_unstable();
    different.dedup();
    let shape = Check::new(
        different.len() >= 3,
        (different.len() as f32 / 4.0).min(1.0),
        format!("{} different notes", different.len()),
    );
    let (lo, hi) = events
        .iter()
        .fold((127u8, 0u8), |(l, h), e| (l.min(e.pitch), h.max(e.pitch)));
    let span = i32::from(hi) - i32::from(lo);
    let reach = Check::new(
        (4..=24).contains(&span),
        if (4..=24).contains(&span) { 1.0 } else { 0.3 },
        format!("{span} semitones from its lowest note to its highest"),
    );
    let join = match (events.first(), events.last()) {
        (Some(a), Some(b)) => (i32::from(a.pitch) - i32::from(b.pitch)).abs(),
        _ => 0,
    };
    let round = Check::new(
        join <= 9,
        1.0 - join as f32 / 24.0,
        format!("{join} semitones from its last note round to its first"),
    );
    let plain: Vec<PlainNote> = events
        .iter()
        .map(|e| PlainNote {
            at: e.at,
            len: e.len,
            pitch: e.pitch,
            velocity: e.velocity,
        })
        .collect();
    let notes = grid_line(&plain, row.key);
    let v = judge(&CriticHeard {
        notes: &notes,
        your_bars: 0,
        bars: row.bars,
        grid: g,
        key: row.key,
        chords: &row.chords,
        ending: Ending::LoopsBack,
        copy_bars: copy_bars(style),
        landings: &[],
        rests: &[],
        lead_to: None,
    });
    // The critic's high point, late and above the opening, is the plan's
    // lift heard. One bar has no plan and may fall from its first note.
    let somewhere = if row.bars < 2 {
        let pitches: Vec<i32> = events.iter().map(|e| i32::from(e.pitch)).collect();
        moves_off(&pitches)
    } else {
        v.somewhere
    };
    IdeaVerdict {
        checks: vec![
            ("keeps its pulse", pulse),
            ("plays the chords", chords),
            ("has a shape", shape),
            ("goes somewhere", somewhere),
            ("in reach", reach),
            ("not too repetitive", v.repeats),
            ("joins round", round),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::DiceName;
    use mpfree_context::chord::Quality;

    fn dice_of(seed: u64) -> Dice {
        DiceName {
            seed,
            part: 0x34,
            phrase: 0,
            job: Job::Pitch,
            bar: 0,
            attempt: 0,
        }
        .dice()
    }

    #[test]
    fn three_places_up_a_triads_ladder_is_the_octave_whatever_the_chord() {
        let cm7 = Chord::new(0, Quality::Minor7);
        // The chord's own ladder has four notes to the octave.
        assert_eq!(ladder(cm7, 60, 1, false), 63);
        assert_eq!(ladder(cm7, 60, 3, false), 70);
        assert_eq!(ladder(cm7, 60, 4, false), 72);
        assert_eq!(ladder(cm7, 60, -1, false), 58);
        // The triad's has three: the seventh is stepped over.
        assert_eq!(ladder(cm7, 60, 2, true), 67);
        assert_eq!(ladder(cm7, 60, 3, true), 72);
        assert_eq!(ladder(cm7, 60, -1, true), 55);
        // A triad's ladder is the same either way.
        let c = Chord::new(0, Quality::Major);
        assert_eq!(ladder(c, 64, 3, false), ladder(c, 64, 3, true));
        assert_eq!(ladder(c, 64, 3, true), 76);
        assert_eq!(ladder(c, 60, 0, true), 60);
    }

    #[test]
    fn figures_rise_and_fall_and_start_on_any_note_of_the_chord() {
        let (mut rising, mut falling) = (0, 0);
        let mut from = [0usize; 3];
        for seed in 0..600u64 {
            let len = [3usize, 4, 6, 8][(seed % 4) as usize];
            let f = Figure::new(len, &mut dice_of(seed));
            assert_eq!(f.places.len(), len);
            // From the ladder's place 0 up to its top, both of them played.
            assert_eq!(f.places.iter().copied().min(), Some(0));
            assert!(f.top() >= 2, "{:?}", f.places);
            assert!((0..=2).contains(&f.from));
            if f.triad && len != 3 {
                // The octaves keep to the root.
                assert_eq!(f.from, 0);
                assert!(f.places.iter().all(|p| [0, 1, 2, 3].contains(p)));
            }
            if f.places[0] == f.top() {
                falling += 1;
            } else if f.places[0] == 0 {
                rising += 1;
            }
            from[f.from as usize] += 1;
        }
        // About one in three falls, and about half start on the root.
        assert!((120..=280).contains(&falling), "{falling} of 600 fall");
        assert!(rising > falling);
        assert!(from[0] > from[2] && from[2] > from[1], "{from:?}");
        assert!(from[1] > 30);
    }

    #[test]
    fn the_figures_are_these() {
        assert_eq!(rising(0, 3), [0, 1, 2]);
        assert_eq!(rising(WIDE, 3), [0, 2, 4]);
        assert_eq!(rising(2, 4), [0, 1, 2, 1]);
        assert_eq!(rising(2, 8), [0, 1, 2, 3, 4, 3, 2, 1]);
        assert_eq!(rising(3, 8), [0, 1, 0, 2, 0, 3, 0, 2]);
        assert_eq!(rising(4, 6), [0, 2, 1, 3, 2, 4]);
        assert_eq!(rising(OCTAVES, 8), [0, 3, 0, 3, 0, 3, 0, 2]);
    }

    #[test]
    fn a_falling_figure_is_its_rising_twin_from_the_top_down() {
        let mut up = rising(0, 3);
        fall(&mut up);
        assert_eq!(up, [2, 1, 0]);
        let mut held = rising(3, 8);
        fall(&mut held);
        // The note held to is now the top one, the others falling under it.
        assert_eq!(held, [3, 2, 3, 1, 3, 0, 3, 1]);
        // The same notes, so the same room.
        for kind in 0..=OCTAVES {
            for len in [3, 4, 6, 8] {
                let rise = rising(kind, len);
                let mut fell = rise.clone();
                fall(&mut fell);
                assert_eq!(rise.iter().max(), fell.iter().max());
                assert_eq!(fell.iter().min(), Some(&0));
            }
        }
    }
}
