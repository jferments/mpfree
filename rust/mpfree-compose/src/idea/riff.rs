//! A riff: the rhythm of the style's cells, and a tune of its own.
//!
//! One period (a bar, or two) is a tune chosen whole along a shape, every
//! note of it at once by the cheapest path, so the way from its last note
//! back round to its first is weighed with the rest. A tune that came out
//! on one or two notes is made again along the other shapes. Every later
//! bar plays the tune's note for its place, moved in key with its chord's
//! root, as the phrase player itself moves a phrase. Then the plan: a
//! landing leaves the home note for another of its chord, the close takes
//! the chord note nearest the first note, what follows the close walks
//! back into the first note, and the high point is lifted above
//! everything else. One bar has no room for a plan: it is all tune.
//!
//! A riff's notes end where the plan rests, so the rest is heard, and
//! one bar, which has no plan, ends an 1/8 before it comes round.

use mpfree_arp::phrase::PLAIN_VELOCITY;
use mpfree_context::chord::Chord;
use mpfree_context::groove::STEP;
use mpfree_context::key::Key;
use mpfree_context::PartRole;
use mpfree_transport::time::{Swing, Tick};

use super::{dice, moves_off, pushed_ticks, Draft, IdeaVerdict, Made, NewIdea};
use crate::cells::{lengths, Stroke};
use crate::critic::{copy_bars, moves, repeats, Check, Heard as CriticHeard};
use crate::dice::{Dice, Job};
use crate::event::{Event, Role, Source};
use crate::motif::{line as grid_line, GridNote};
use crate::parts::{best_path_cost, chord_near, rhythm, riff_bars, unit_of, AddPart, Relation};
use crate::profile::InstrumentKind;
use crate::realize::PlainNote;
use crate::row::{Mark, Row};
use crate::settings::{Ending, Settings};
use crate::sketch::AnchorKind;
use crate::style::Style;
use crate::timing::Timing;

/// Where a riff sits.
pub(super) const RIFF_LOW: i32 = 55;
const RIFF_HIGH: i32 = 81;
const RIFF_CENTRE: i32 = 66;

/// What a place off the shape costs a note of the tune. Beside it the
/// style's taste in moves ([`TASTE`]) only chooses between ways along the
/// shape: at half this pull and half as much again of the taste, a style
/// that likes to say a note again (Minimal) made riffs of one note.
const SHAPE_PULL: f64 = 0.45;
/// The weight of the style's taste in moves.
const TASTE: f64 = 0.4;
/// What saying a note again costs after a note of an 1/8 or more, and
/// after one of a beat or more: quick notes may hammer one pitch, and
/// slow ones on one pitch are a tune standing still.
const AGAIN_AFTER_EIGHTH: f64 = 0.7;
const AGAIN_AFTER_BEAT: f64 = 1.2;
/// How much of its steps a note of a riff sounds: the style's gate, kept
/// between these so that a style played legato still leaves a riff its
/// gaps.
pub(super) const GATE_LOW: f64 = 0.4;
pub(super) const GATE_HIGH: f64 = 0.8;
/// A note on one of the plan's marks is held.
const GATE_MARKED: f64 = 0.95;
/// Shapes and reaches a tune is made along before the most varied of
/// them is kept.
const REACHES: usize = 3;

/// The shape a riff's tune follows through its period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// Up to the high point and back.
    Arch,
    /// Climbing to its last note.
    Rise,
    /// Low and high by turns, the high ones climbing.
    Zigzag,
    /// One note, then another from the high point on.
    Plateau,
}

impl Shape {
    const ALL: [Shape; 4] = [Shape::Arch, Shape::Rise, Shape::Zigzag, Shape::Plateau];

    /// Where note `k` of `n` aims, in scale places above the first, with
    /// the high point at note `peak` and `reach` places up.
    fn aim(self, k: usize, n: usize, peak: usize, reach: i32) -> i32 {
        let (k, n, peak) = (k as i32, n as i32, (peak as i32).max(1));
        match self {
            Shape::Arch if k <= peak => reach * k / peak,
            Shape::Arch => {
                let left = (n - 1 - peak).max(1);
                1 + (reach - 1) * (n - 1 - k) / left
            }
            Shape::Rise => reach * k / (n - 1).max(1),
            Shape::Zigzag if k % 2 == 0 => 0,
            Shape::Zigzag => (reach - 1 + (k >= peak) as i32).max(1),
            Shape::Plateau if k < peak => 0,
            Shape::Plateau if k == n - 1 && n - 1 > peak => 1,
            Shape::Plateau => reach,
        }
    }
}

/// The scale place of a pitch in `key`.
fn place(key: Key, p: i32) -> i32 {
    key.position(p.clamp(0, 127) as u8).0
}

/// The places from one chord's root up or down to another's, the short
/// way round.
fn root_shift(key: Key, from: Chord, to: Chord) -> i32 {
    let d = (place(key, 60 + i32::from(to.root % 12)) - place(key, 60 + i32::from(from.root % 12)))
        .rem_euclid(7);
    if d > 3 {
        d - 7
    } else {
        d
    }
}

/// `p`, or the chord note nearest it when it is no more than a whole step
/// away.
fn onto_chord(chord: Option<Chord>, p: i32) -> i32 {
    let Some(c) = chord else {
        return p;
    };
    if c.tone_of(p).is_some() {
        return p;
    }
    [1, -1, 2, -2]
        .into_iter()
        .map(|d| p + d)
        .find(|&q| c.tone_of(q).is_some())
        .unwrap_or(p)
}

/// The plan's marks a riff keeps to. One bar has none: its only mark is
/// a close at the half bar, and a close with its walk back would leave
/// half a bar of tune.
pub(super) fn marks_of(row: &Row) -> &[Mark] {
    if row.bars < 2 {
        &[]
    } else {
        &row.plan.marks
    }
}

/// The real stroke that plays the plan's mark at `at`: on it, or pushed
/// in before it.
fn stroke_at(strokes: &[Stroke], at: i32, unit: i32) -> Option<usize> {
    strokes
        .iter()
        .position(|s| !s.ghost && (s.at == at || (s.pushed && s.at + unit == at)))
}

/// The mark stroke `s` plays, if any.
fn mark_of(marks: &[Mark], s: Stroke, unit: i32) -> Option<AnchorKind> {
    marks
        .iter()
        .find(|m| m.at == s.at || (s.pushed && m.at == s.at + unit))
        .map(|m| m.kind)
}

/// Whether `n` notes with `different` pitches among them, the commonest
/// played `most` times, are a tune: three different notes (all of them
/// different when there are fewer), and none of them more than two notes
/// in three.
fn varied(n: usize, different: usize, most: usize) -> bool {
    different >= n.min(3) && (n < 3 || most * 3 <= n * 2)
}

/// How much of a tune `notes` are, to tell the better of two: whether
/// they are one at all ([`varied`]), then how many different notes they
/// have, then how seldom the commonest sounds.
fn how_varied(notes: &[i32]) -> (bool, usize, std::cmp::Reverse<usize>) {
    let (different, most) = spread(notes);
    (
        varied(notes.len(), different, most),
        different,
        std::cmp::Reverse(most),
    )
}

/// How many different pitches `notes` has, and how often the commonest
/// sounds.
fn spread(notes: &[i32]) -> (usize, usize) {
    let mut sorted = notes.to_vec();
    sorted.sort_unstable();
    let most = sorted
        .chunk_by(|a, b| a == b)
        .map(<[i32]>::len)
        .max()
        .unwrap_or(0);
    sorted.dedup();
    (sorted.len(), most)
}

/// One draft of a riff.
pub(super) fn riff(req: &NewIdea, row: &Row, settings: &Settings, swung: bool, d: u16) -> Made {
    let band = AddPart {
        row,
        part: PartRole::Melody,
        relation: Relation::Support,
        settings,
        kind: InstrumentKind::OneNote,
        pads: [None; 4],
        swing: Swing::default(),
    };
    let (mut strokes, _) = rhythm(&band, d, &mut dice(req, Job::Cells, d));
    let period = riff_bars(&band, d);
    let marks = marks_of(row);
    let unit = unit_of(&req.style);
    // A part under a tune only plays more at the plan's high point. A
    // riff is the tune, so it has a note there, as a part has one on
    // every landing.
    if let Some(m) = marks.iter().find(|m| m.kind == AnchorKind::Peak) {
        if stroke_at(&strokes, m.at, unit).is_none() {
            strokes.push(Stroke::at(m.at));
            lengths(&mut strokes, row.steps);
        }
    }
    let (pitches, accent) = riff_tune(
        &band,
        &strokes,
        period,
        marks,
        &mut dice(req, Job::Pitch, d),
    );
    let st = &req.style;
    let gate = f64::from(st.playing.gate).clamp(GATE_LOW, GATE_HIGH);
    let eighth = (row.grid.beat / 2).max(1);
    let last = strokes.iter().rposition(|s| !s.ghost);
    let mut events = Vec::new();
    for (i, s) in strokes.iter().enumerate() {
        let Some(p) = pitches[i] else {
            continue;
        };
        let held = if mark_of(marks, *s, unit).is_some() {
            GATE_MARKED
        } else {
            gate
        };
        // A stroke lasts until the next one, through a rest of the plan's
        // when one comes first: the note ends where that rest starts.
        let mut until = row
            .plan
            .rests
            .iter()
            .map(|&(from, _)| from)
            .filter(|&from| s.at < from && from < s.at + s.len)
            .min()
            .unwrap_or(s.at + s.len);
        // One bar has no plan to rest in: its last note, when it is
        // longer than an 1/8, ends an 1/8 before the riff comes round.
        if marks.is_empty() && Some(i) == last && s.len > eighth {
            until = until.min(s.at + s.len - eighth);
        }
        events.push(Event {
            at: row.tick(s.at),
            len: ((f64::from(until - s.at) * held * STEP as f64) as Tick).max(STEP / 3),
            pitch: p.clamp(0, 127) as u8,
            velocity: PLAIN_VELOCITY,
            role: match chord_near(row, s.at).and_then(|c| c.tone_of(p)) {
                Some(t) => Role::Chord(t),
                None => Role::Scale,
            },
            source: Source::Mpfree,
            timing: Timing::default(),
        });
    }
    let mut accents = pushed_ticks(&strokes);
    accents.extend(accent.map(|i| row.tick(strokes[i].at)));
    Draft {
        strokes,
        events,
        accents,
    }
    .heard(row, swung, |strokes, heard| {
        judge_riff(row, st, marks, strokes, heard)
    })
}

/// What a tune is made from.
struct TuneOf<'a> {
    band: &'a AddPart<'a>,
    strokes: &'a [Stroke],
    marks: &'a [Mark],
    /// The strokes that play the tune, in order.
    tune: &'a [usize],
    /// The period's length in steps.
    per: i32,
    /// The pitch classes the tune keeps to off its chords.
    pool: u16,
}

impl TuneOf<'_> {
    /// The tune along `shape`, reaching `reach` places up: each first
    /// note it might start on, the whole tune from it, and the cheapest
    /// of them, since only with the whole tune is the way back from its
    /// last note to its first, which it takes every time round, known.
    fn along(&self, shape: Shape, reach: i32, dice: &mut Dice) -> Vec<i32> {
        let (row, strokes, tune) = (self.band.row, self.strokes, self.tune);
        let g = row.grid;
        let key = row.key;
        let st = &self.band.settings.style;
        let unit = unit_of(st);
        let n = tune.len();
        // The tune's own high point: the plan's when it falls in the
        // tune, else the note nearest five eighths through.
        let peak_k = tune
            .iter()
            .position(|&i| mark_of(self.marks, strokes[i], unit) == Some(AnchorKind::Peak))
            .unwrap_or_else(|| {
                (0..n)
                    .min_by_key(|&k| (strokes[tune[k]].at - self.per * 5 / 8).abs())
                    .unwrap_or(0)
            })
            .max(usize::from(n > 1));
        let noise = 0.12 * (0.3 + f64::from(self.band.settings.variation));
        let first_chord = chord_near(row, strokes[tune[0]].at);
        let top_odds = st.melody.intervals.iter().copied().fold(0.01f32, f32::max);
        let move_cost = |q: i32, p: i32| -> f64 {
            let places = (place(key, p) - place(key, q)).unsigned_abs() as usize;
            let odds = st.melody.intervals[places.min(7)].max(0.01);
            let mut c = TASTE * f64::from(top_odds / odds).ln();
            if places > 7 {
                c += 3.0;
            }
            if (p - q).abs() == 6 {
                c += 0.8;
            }
            c
        };
        // What it costs note `k` to say the note before it again.
        let again = |k: usize| -> f64 {
            let len = strokes[tune[k - 1]].len;
            if len >= g.beat {
                AGAIN_AFTER_BEAT
            } else if len * 2 >= g.beat {
                AGAIN_AFTER_EIGHTH
            } else {
                0.0
            }
        };
        let firsts: Vec<i32> = (RIFF_LOW..=RIFF_CENTRE + 2)
            .filter(|&p| first_chord.is_none_or(|c| c.tone_of(p).is_some()))
            .collect();
        let mut chosen: Option<(f64, Vec<i32>)> = None;
        for &first in &firsts {
            let start_cost = match first_chord.map(|c| (first - i32::from(c.root)).rem_euclid(12)) {
                Some(0) => 0.0,
                Some(7) => 0.2,
                Some(3 | 4) => 0.3,
                _ => 0.8,
            } + dice.gumbel() * noise;
            let home = place(key, first);
            let cands: Vec<Vec<(i32, (), f64)>> = (0..n)
                .map(|k| {
                    if k == 0 {
                        return vec![(first, (), start_cost)];
                    }
                    let s = strokes[tune[k]];
                    let chord = chord_near(row, s.at);
                    let strong = g.level(s.at) >= -2 || s.pushed;
                    let marked = mark_of(self.marks, s, unit).is_some();
                    let aim = home + shape.aim(k, n, peak_k, reach);
                    let mut v = Vec::new();
                    for p in RIFF_LOW..=RIFF_HIGH {
                        let on_chord = chord.is_some_and(|c| c.tone_of(p).is_some());
                        if !on_chord && self.pool >> p.rem_euclid(12) & 1 == 0 {
                            continue;
                        }
                        let mut c = match (on_chord, strong) {
                            (true, true) => 0.0,
                            (true, false) => 0.1,
                            (false, true) => f64::from(st.melody.off_chord[0]),
                            (false, false) => f64::from(st.melody.off_chord[1]),
                        };
                        if marked && !on_chord {
                            c += 2.0;
                        }
                        c += SHAPE_PULL * f64::from((place(key, p) - aim).abs());
                        if k == n - 1 {
                            // The way back round to the first note.
                            c += 0.5 * move_cost(p, first);
                            if !on_chord {
                                c += 0.8;
                            }
                        }
                        c += dice.gumbel() * noise;
                        v.push((p, (), c));
                    }
                    if v.is_empty() {
                        v.push((first, (), 5.0));
                    }
                    v
                })
                .collect();
            let (path, cost) = best_path_cost(&cands, |k, qi, pi| {
                let (q, p) = (cands[k - 1][qi].0, cands[k][pi].0);
                move_cost(q, p) + if q == p { again(k) } else { 0.0 }
            });
            if chosen.as_ref().is_none_or(|(c, _)| cost < *c) {
                chosen = Some((cost, (0..n).map(|k| cands[k][path[k]].0).collect()));
            }
        }
        chosen.map(|c| c.1).unwrap_or_else(|| vec![RIFF_CENTRE; n])
    }
}

/// A riff's notes, one per stroke (none for a ghost), and the stroke
/// that takes its accent: the plan's high point, or in one bar the top
/// of the tune.
fn riff_tune(
    band: &AddPart,
    strokes: &[Stroke],
    period: usize,
    marks: &[Mark],
    dice: &mut Dice,
) -> (Vec<Option<i32>>, Option<usize>) {
    let row = band.row;
    let g = row.grid;
    let key = row.key;
    let st = &band.settings.style;
    let unit = unit_of(st);
    let real: Vec<usize> = (0..strokes.len()).filter(|&i| !strokes[i].ghost).collect();
    let mut pitch: Vec<Option<i32>> = vec![None; strokes.len()];
    if real.is_empty() {
        return (pitch, None);
    }
    let playing = |kind: AnchorKind| {
        marks
            .iter()
            .filter(|m| m.kind == kind)
            .find_map(|m| stroke_at(strokes, m.at, unit))
    };
    // The tune: the period's notes before the close. The close and what
    // follows it are set from the first note once the tune is known, so a
    // tune that ran through them would be choosing notes nobody hears.
    let per = (period as i32 * g.bar).min(row.steps);
    let in_period = |i: &usize| strokes[*i].at < per;
    let mut close = playing(AnchorKind::Close);
    let mut tune: Vec<usize> = real
        .iter()
        .copied()
        .filter(in_period)
        .filter(|&i| close.is_none_or(|c| strokes[i].at < strokes[c].at))
        .collect();
    if tune.is_empty() {
        // Nothing before the close: the period's notes are all the tune
        // there is, and nothing closes.
        close = None;
        tune = real.iter().copied().filter(in_period).collect();
        if tune.is_empty() {
            tune = real.clone();
        }
    }

    // The notes the tune keeps to: the key's, or for a style that keeps
    // to a few, the first chord's and what it takes to make up the number.
    let first_chord = chord_near(row, strokes[tune[0]].at);
    let pool: u16 = {
        let want = usize::from(st.melody.pitch_set);
        let mut pcs = first_chord.map_or(0, |c| c.pcs());
        if want == 0 {
            key.pcs()
        } else {
            let mut spare: Vec<u8> = (0..12u8)
                .filter(|&pc| key.has(i32::from(pc)) && pcs >> pc & 1 == 0)
                .collect();
            while (pcs.count_ones() as usize) < want && !spare.is_empty() {
                pcs |= 1 << spare.remove(dice.below(spare.len()));
            }
            pcs
        }
    };

    // Along the dice's own shape and reach first; while what comes out is
    // no tune, along the others, and the most varied of them all is kept.
    let of = TuneOf {
        band,
        strokes,
        marks,
        tune: &tune,
        per,
        pool,
    };
    let shape0 = dice.pick(&[3.0, 1.5, 2.0, 1.5]).unwrap_or(0);
    let reach0 = dice.below(REACHES);
    let mut notes: Vec<i32> = Vec::new();
    'made: for ds in 0..Shape::ALL.len() {
        for dr in 0..REACHES {
            let shape = Shape::ALL[(shape0 + ds) % Shape::ALL.len()];
            let reach = 3 + ((reach0 + dr) % REACHES) as i32;
            let made = of.along(shape, reach, dice);
            if notes.is_empty() || how_varied(&made) > how_varied(&notes) {
                notes = made;
            }
            if how_varied(&notes).0 {
                break 'made;
            }
        }
    }
    for (k, &i) in tune.iter().enumerate() {
        pitch[i] = Some(notes[k]);
    }
    let first_note = notes[0];

    // What follows the close, in order.
    let after: Vec<usize> = close.map_or_else(Vec::new, |c| {
        real.iter()
            .copied()
            .filter(|&j| strokes[j].at > strokes[c].at)
            .collect()
    });

    // Every later bar, up to the close: the tune's note for its place, on
    // its chord's root.
    let later: Vec<usize> = real
        .iter()
        .copied()
        .filter(|i| !tune.contains(i) && Some(*i) != close && !after.contains(i))
        .collect();
    let mut b = 0;
    while b * g.bar < row.steps {
        let (from, to) = (b * g.bar, (b + 1) * g.bar);
        b += 1;
        let bar: Vec<usize> = later
            .iter()
            .copied()
            .filter(|&i| (from..to).contains(&strokes[i].at))
            .collect();
        if bar.is_empty() {
            continue;
        }
        let mut made: Vec<i32> = bar
            .iter()
            .map(|&i| {
                let s = strokes[i];
                let o = s.at.rem_euclid(per);
                let o = if s.pushed { o + unit } else { o };
                let k = tune.iter().rposition(|&j| strokes[j].at <= o).unwrap_or(0);
                let then = chord_near(row, strokes[tune[k]].at);
                let now = chord_near(row, s.at);
                let shift = match (then, now) {
                    (Some(a), Some(b)) => root_shift(key, a, b),
                    _ => 0,
                };
                let (pos, alt) = key.position(notes[k].clamp(0, 127) as u8);
                let p = i32::from(key.pitch_at(pos + shift, alt));
                if g.level(s.at) >= -2 || s.pushed {
                    onto_chord(now, p)
                } else {
                    p
                }
            })
            .collect();
        // The whole bar in the octave that keeps it nearest the tune's
        // own middle, of the octaves that keep it in range: moved by its
        // root the short way round alone, a bar under a high tune landed
        // a fourth below it, and the high point then leapt out of the bar
        // by more than an octave.
        let mean = |v: &[i32]| v.iter().sum::<i32>() as f64 / v.len().max(1) as f64;
        let (middle, here) = (mean(&notes), mean(&made));
        let fits = |o: i32| {
            made.iter()
                .all(|&p| (RIFF_LOW - 2..=RIFF_HIGH + 2).contains(&(p + o)))
        };
        let octave = [0, 12, -12]
            .into_iter()
            .filter(|&o| fits(o))
            .min_by(|&a, &b| {
                (here + f64::from(a) - middle)
                    .abs()
                    .total_cmp(&(here + f64::from(b) - middle).abs())
            })
            .unwrap_or_else(|| {
                if made.iter().any(|&p| p > RIFF_HIGH + 2) {
                    -12
                } else if made.iter().any(|&p| p < RIFF_LOW - 2) {
                    12
                } else {
                    0
                }
            });
        made.iter_mut().for_each(|p| *p += octave);
        for (&i, &p) in bar.iter().zip(&made) {
            pitch[i] = Some(p);
        }
    }

    // A landing is open, wanting more: on its chord, off the home note
    // when the chord has another near.
    for m in marks.iter().filter(|m| m.kind == AnchorKind::Landing) {
        let Some(i) = stroke_at(strokes, m.at, unit) else {
            continue;
        };
        let Some(p) = pitch[i] else {
            continue;
        };
        let chord = chord_near(row, m.at);
        let q = onto_chord(chord, p);
        pitch[i] = Some(if q.rem_euclid(12) == i32::from(key.tonic % 12) {
            [2, -2, 3, -3, 4, -4]
                .into_iter()
                .map(|d| q + d)
                .find(|&x| chord.is_some_and(|c| c.tone_of(x).is_some()))
                .unwrap_or(q)
        } else {
            q
        });
    }

    // The close: the chord note nearest the first note (the lower of two
    // as near). What follows it walks back into the first note.
    if let Some(at_close) = close {
        let s = strokes[at_close];
        let chord = chord_near(row, s.at + if s.pushed { unit } else { 0 });
        let near = (RIFF_LOW..=RIFF_HIGH)
            .filter(|&q| chord.is_none_or(|c| c.tone_of(q).is_some()))
            .min_by_key(|&q| ((q - first_note).abs(), q))
            .unwrap_or(first_note);
        pitch[at_close] = Some(near);
        let to = place(key, first_note);
        // Three places off, two, then one: from the close's side of the
        // first note, and from below when the close is the first note.
        // A close right beside the first note is already as near as a
        // walk ends, so the walk comes from the other side and the two
        // enclose the first note.
        let dir = match place(key, near) - to {
            0 => -1,
            off @ (-1 | 1) => -off,
            off => off.signum(),
        };
        let walk: Vec<i32> = (0..after.len())
            .map(|k| {
                let left = (after.len() - k) as i32;
                i32::from(key.pitch_at(to + dir * left.min(3), 0))
            })
            .collect();
        let mut before = near;
        for (k, &j) in after.iter().enumerate() {
            let p = walk[k];
            // Its chord's own note when that is a half step off, the one
            // towards the first note before the other (the leading note
            // over a dominant), unless the note before it, the one after
            // it or the first note is that note already.
            let own = chord_near(row, strokes[j].at)
                .filter(|c| c.tone_of(p).is_none())
                .and_then(|c| {
                    let towards = (first_note - p).signum();
                    [towards, -towards].into_iter().map(|d| p + d).find(|&q| {
                        c.tone_of(q).is_some()
                            && q != first_note
                            && q != before
                            && walk.get(k + 1) != Some(&q)
                    })
                });
            before = own.unwrap_or(p);
            pitch[j] = Some(before);
        }
    }

    // The high point, above everything else.
    let mut accent: Option<usize> = None;
    if let Some(i) = playing(AnchorKind::Peak) {
        if let Some(p) = pitch[i] {
            let chord = chord_near(
                row,
                strokes[i].at + if strokes[i].pushed { unit } else { 0 },
            );
            let top = real
                .iter()
                .filter(|&&j| j != i)
                .filter_map(|&j| pitch[j])
                .max()
                .unwrap_or(p);
            if p <= top {
                let lifted = (top + 1..=RIFF_HIGH + 3)
                    .find(|&q| chord.is_none_or(|c| c.tone_of(q).is_some()))
                    .unwrap_or(top);
                pitch[i] = Some(lifted);
            }
            accent = Some(i);
        }
    } else if marks.is_empty() {
        // One bar: the first of the tune's highest notes, when the tune
        // rose to it.
        let top = notes.iter().copied().max().unwrap_or(first_note);
        if top > first_note {
            accent = notes.iter().position(|&p| p == top).map(|k| tune[k]);
        }
    }
    (pitch, accent)
}

/// A riff judged on what a phrase writes of it:
///
/// - **has a tune:** three different notes, none more than two in three;
/// - **goes somewhere:** see [`somewhere`];
/// - **leads back:** see [`leads_back`];
/// - **breathes:** a rest of an 1/8 or more in every four bars, the one
///   round the loop's end counted, and nothing sounding in the plan's
///   rests. The critic's "not every beat struck" is not asked: a riff
///   may strike them all;
/// - **not too repetitive:** the critic's, so four bars are not one bar
///   four times;
/// - **flows:** see [`flows`].
pub(super) fn judge_riff(
    row: &Row,
    style: &Style,
    marks: &[Mark],
    strokes: &[Stroke],
    events: &[Event],
) -> IdeaVerdict {
    let g = row.grid;
    let key = row.key;
    let plain: Vec<PlainNote> = events
        .iter()
        .map(|e| PlainNote {
            at: e.at,
            len: e.len,
            pitch: e.pitch,
            velocity: e.velocity,
        })
        .collect();
    let notes: Vec<GridNote> = grid_line(&plain, key);
    let pitches: Vec<i32> = notes.iter().map(|m| i32::from(m.pitch(key))).collect();

    let (different, most) = spread(&pitches);
    let tune = Check::new(
        varied(pitches.len(), different, most),
        (different as f32 / 4.0).min(1.0),
        format!(
            "{different} different notes, the commonest {most} of its {}",
            pitches.len()
        ),
    );

    let eighth = (g.beat / 2).max(1);
    let end = row.steps;
    let mut gaps: Vec<(i32, i32)> = notes
        .windows(2)
        .map(|w| (w[0].on + w[0].len, w[1].on))
        .filter(|&(a, b)| b - a >= eighth)
        .collect();
    if let (Some(first), Some(last)) = (notes.first(), notes.last()) {
        // Round the loop: from the last note's end to the first note.
        let from = last.on + last.len;
        if first.on + end - from >= eighth {
            gaps.push((from.min(end), end));
            gaps.push(((from - end).max(0), first.on));
        }
    }
    let groups = row.bars.div_ceil(4).max(1);
    let with_one = (0..groups as i32)
        .filter(|&q| {
            let (a, b) = (q * 4 * g.bar, (q + 1) * 4 * g.bar);
            gaps.iter().any(|&(x, y)| y > x && x < b && y > a)
        })
        .count();
    let rests: &[(i32, i32)] = if marks.is_empty() {
        &[]
    } else {
        &row.plan.rests
    };
    let kept = rests
        .iter()
        .filter(|&&(a, b)| !notes.iter().any(|m| m.on < b && m.on + m.len > a))
        .count();
    let breathes = Check::new(
        with_one == groups && kept == rests.len(),
        0.6 * with_one as f32 / groups as f32
            + 0.4
                * if rests.is_empty() {
                    1.0
                } else {
                    kept as f32 / rests.len() as f32
                },
        format!(
            "{with_one} of {groups} four-bar stretches with a rest of an 1/8 or more{}",
            if rests.is_empty() {
                String::new()
            } else {
                format!(", {kept} of {} planned rests kept", rests.len())
            }
        ),
    );

    let again = repeats(&CriticHeard {
        notes: &notes,
        your_bars: 0,
        bars: row.bars,
        grid: g,
        key,
        chords: &row.chords,
        ending: Ending::LoopsBack,
        copy_bars: copy_bars(style),
        landings: &[],
        rests: &[],
    });

    IdeaVerdict {
        checks: vec![
            ("has a tune", tune),
            ("goes somewhere", somewhere(row, &notes, &pitches)),
            (
                "leads back",
                leads_back(row, marks, strokes, unit_of(style), &notes),
            ),
            ("breathes", breathes),
            ("not too repetitive", again),
            ("flows", flows(row, &notes, &pitches)),
        ],
    }
}

/// A riff goes somewhere: its highest note is three semitones or more
/// above its first, first sounds late (from an 1/8 before the half of
/// the first bar in two bars, and of two fifths of the way in four, to
/// 92 % of the way through) and is struck no more than one note in four.
/// The critic measures a tune's high point against its opening bar; a
/// riff's high point is in its opening bar when it has two, so a riff is
/// measured from its first note. One bar is asked only to move off its
/// first note ([`moves_off`]).
fn somewhere(row: &Row, notes: &[GridNote], pitches: &[i32]) -> Check {
    if row.bars < 2 {
        return moves_off(pitches);
    }
    let (Some(&first), Some(&top)) = (pitches.first(), pitches.iter().max()) else {
        return Check::new(false, 0.0, "no notes".into());
    };
    let lift = top - first;
    let g = row.grid;
    let at = pitches
        .iter()
        .position(|&p| p == top)
        .map_or(0, |k| notes[k].on);
    let eighth = (g.beat / 2).max(1);
    let earliest = if row.bars <= 2 {
        g.half()
    } else {
        row.steps * 2 / 5
    } - eighth;
    let late = (earliest..=row.steps * 92 / 100).contains(&at);
    let on_top = pitches.iter().filter(|&&p| p == top).count();
    let hammered = on_top > (pitches.len() / 4).max(2);
    Check::new(
        late && lift >= 3 && !hammered,
        if late { 0.5 } else { 0.0 }
            + (lift as f32 / 7.0).clamp(0.0, 1.0) * 0.4
            + if hammered { 0.0 } else { 0.1 },
        format!(
            "highest note first in bar {} of {}, {lift} semitones above its first note{}",
            at.div_euclid(g.bar) + 1,
            row.bars,
            if hammered { ", struck often" } else { "" }
        ),
    )
}

/// A riff leads back: its last note steps into its first (within two
/// places of it), or is the key's fifth, or is a note of the chord the
/// first note comes in on, so the loop's seam is a step or a chord and
/// never an accident. Where a note plays the plan's close, it is a note
/// of its chord. (A style that ends by leaving a note out may leave the
/// close out; the riff then ends as its tune does.)
fn leads_back(
    row: &Row,
    marks: &[Mark],
    strokes: &[Stroke],
    unit: i32,
    notes: &[GridNote],
) -> Check {
    let key = row.key;
    let mut misses: Vec<&str> = Vec::new();
    let mut asked = 1;
    let close = marks
        .iter()
        .find(|m| m.kind == AnchorKind::Close)
        .and_then(|m| Some((m.at, strokes[stroke_at(strokes, m.at, unit)?].at)));
    if let Some((mark, at)) = close {
        asked += 1;
        match (notes.iter().find(|x| x.on == at), chord_near(row, mark)) {
            (None, _) => misses.push("the note at the close was not written"),
            (Some(x), Some(c)) if c.tone_of(i32::from(x.pitch(key))).is_none() => {
                misses.push("the close is off its chord");
            }
            _ => {}
        }
    }
    if let (Some(first), Some(last)) = (notes.first(), notes.last()) {
        let step = (last.pos - first.pos).abs() <= 2;
        let fifth = last.pos.rem_euclid(7) == 4 && last.alt == 0;
        let chord = chord_near(row, first.on)
            .is_some_and(|c| c.tone_of(i32::from(last.pitch(key))).is_some());
        if !step && !fifth && !chord {
            misses.push("the last note doesn't lead back");
        }
    }
    Check::new(
        misses.is_empty(),
        1.0 - misses.len() as f32 / asked as f32,
        if misses.is_empty() {
            "its last note leads back into its first".to_string()
        } else {
            misses.join(", ")
        },
    )
}

/// A riff flows: no leap wider than an octave, at most one wider than a
/// sixth in every two bars (the way up to the high point), no more than
/// two wider than a fourth in any bar (there and back), none followed by
/// another leap the same way, and no trill. The move from the last note
/// round to the first counts, and a move across a rest of a beat or more
/// does not.
///
/// The critic's limit for a tune, one leap wider than a fourth in every
/// two bars, is not a riff's: a riff comes round, so a leap in its tune
/// is in every bar, and up a fifth to an accent and back is what many
/// riffs are.
fn flows(row: &Row, notes: &[GridNote], pitches: &[i32]) -> Check {
    let g = row.grid;
    let mut line: Vec<(i32, i32, i32)> = notes
        .iter()
        .zip(pitches)
        .map(|(m, &p)| (m.on, p, m.len))
        .collect();
    if let Some(&(on, p, len)) = line.first() {
        line.push((on + row.steps, p, len));
    }
    let m = moves(&line, row.bars, g, Some(g.beat));
    let most = m.leaps.iter().copied().max().unwrap_or(0);
    let wide_allowed = (row.bars / 2).max(1);
    let pass =
        m.wild == 0 && m.wide <= wide_allowed && most <= 2 && m.on_and_on == 0 && m.trills == 0;
    let score = 1.0
        - 0.5 * m.wild as f32
        - 0.25 * m.wide.saturating_sub(wide_allowed) as f32
        - 0.25 * most.saturating_sub(2) as f32
        - 0.3 * m.on_and_on as f32
        - 0.3 * m.trills as f32;
    Check::new(
        pass,
        score,
        format!(
            "at most {most} leaps wider than a fourth in a bar (2 allowed), {} wider than a sixth \
             ({wide_allowed} allowed), {} wider than an octave, {} leaping on the same way, {} \
             trills",
            m.wide, m.wild, m.on_and_on, m.trills
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{band, chord_loop, key_of, settings_of};
    use super::*;
    use crate::style::STYLES;
    use mpfree_arp::PhraseKind;
    use mpfree_context::chord::{ChordAnswer, ChordTimeline, Heard as ChordHeard, Quality};
    use mpfree_transport::time::Meter;

    /// A Groove riff request.
    fn request(bars: u8, minor: bool, seed: u64) -> NewIdea {
        NewIdea {
            kind: PhraseKind::Riff,
            style: STYLES[1],
            minor,
            bars,
            activity: 0.5,
            seed,
            attempt: 0,
        }
    }

    /// The band `req` is made for, over its own loop of chords or over
    /// the home chord alone.
    fn row_of(req: &NewIdea, home_only: bool) -> (Row, Settings) {
        let key = key_of(req.minor);
        let settings = settings_of(req);
        let chords = if home_only {
            let quality = if req.minor {
                Quality::Minor
            } else {
                Quality::Major
            };
            ChordTimeline::whole(
                Tick::from(settings.bars) * Meter { beats: 4, unit: 4 }.bar_ticks(),
                ChordAnswer::Chord(Chord::new(0, quality)),
                ChordHeard::Suggested,
            )
        } else {
            chord_loop(req, key, &settings)
        };
        (band(key, &chords, &settings), settings)
    }

    fn part<'a>(row: &'a Row, settings: &'a Settings) -> AddPart<'a> {
        AddPart {
            row,
            part: PartRole::Melody,
            relation: Relation::Support,
            settings,
            kind: InstrumentKind::OneNote,
            pads: [None; 4],
            swing: Swing::default(),
        }
    }

    fn strokes_at(at: &[i32], steps: i32) -> Vec<Stroke> {
        let mut v: Vec<Stroke> = at.iter().map(|&a| Stroke::at(a)).collect();
        lengths(&mut v, steps);
        v
    }

    fn note(on: i32, len: i32, pos: i32) -> GridNote {
        GridNote {
            on,
            len,
            pos,
            alt: 0,
            velocity: 100,
        }
    }

    fn pitches_of(row: &Row, notes: &[GridNote]) -> Vec<i32> {
        notes.iter().map(|m| i32::from(m.pitch(row.key))).collect()
    }

    #[test]
    fn a_tune_is_three_notes_and_none_of_them_two_times_in_three() {
        assert!(varied(6, 3, 4));
        assert!(!varied(6, 3, 5));
        assert!(!varied(8, 2, 4));
        // Fewer than three notes: all of them different.
        assert!(varied(2, 2, 1));
        assert!(!varied(2, 1, 2));
        assert_eq!(spread(&[60, 63, 60, 67, 60]), (3, 3));
    }

    #[test]
    fn the_shapes_aim_where_they_say() {
        // Eight notes, the high point at the sixth, four places up.
        let aims = |s: Shape| (0..8).map(|k| s.aim(k, 8, 5, 4)).collect::<Vec<_>>();
        assert_eq!(aims(Shape::Arch), [0, 0, 1, 2, 3, 4, 2, 1]);
        assert_eq!(aims(Shape::Rise), [0, 0, 1, 1, 2, 2, 3, 4]);
        assert_eq!(aims(Shape::Zigzag), [0, 3, 0, 3, 0, 4, 0, 4]);
        assert_eq!(aims(Shape::Plateau), [0, 0, 0, 0, 0, 4, 4, 1]);
    }

    #[test]
    fn one_bar_has_no_plan() {
        let (one, _) = row_of(&request(1, true, 1), false);
        assert!(marks_of(&one).is_empty());
        assert!(!one.plan.marks.is_empty());
        let (two, _) = row_of(&request(2, true, 1), false);
        assert_eq!(marks_of(&two).len(), two.plan.marks.len());
        assert!(marks_of(&two).iter().any(|m| m.kind == AnchorKind::Peak));
    }

    /// Two bars with the high point on beat 3 of the first and the close
    /// on beat 3 of the second: four notes of tune, three that bring it
    /// back in bar 2, the close, and three that walk back.
    const TWO_BARS: [i32; 11] = [0, 3, 6, 8, 16, 19, 22, 24, 26, 28, 30];

    #[test]
    fn the_close_and_the_walk_back_lead_into_the_first_note() {
        for seed in 1..=40 {
            for minor in [true, false] {
                let req = request(2, minor, seed);
                let (row, settings) = row_of(&req, false);
                let strokes = strokes_at(&TWO_BARS, row.steps);
                let (pitch, _) = riff_tune(
                    &part(&row, &settings),
                    &strokes,
                    1,
                    marks_of(&row),
                    &mut dice(&req, Job::Pitch, 0),
                );
                let p: Vec<i32> = pitch.iter().map(|p| p.expect("a note")).collect();
                let first = p[0];
                let why = format!("seed {seed} minor {minor}: {p:?}");
                // The close: the note of its chord nearest the first note.
                let chord = chord_near(&row, 24).expect("a chord");
                assert!(chord.tone_of(p[7]).is_some(), "{why}");
                let nearer = (RIFF_LOW..=RIFF_HIGH)
                    .filter(|&q| chord.tone_of(q).is_some())
                    .any(|q| (q - first).abs() < (p[7] - first).abs());
                assert!(!nearer, "{why}");
                // The walk: never the first note itself, no note said
                // twice running, and each as near the first note as the
                // one before it or nearer, the last a step away.
                let walk = &p[7..];
                assert!(walk.windows(2).all(|w| w[0] != w[1]), "{why}");
                assert!(walk[1..].iter().all(|&q| q != first), "{why}");
                let off: Vec<i32> = walk[1..].iter().map(|q| (q - first).abs()).collect();
                assert!(off.windows(2).all(|w| w[0] >= w[1]), "{why}");
                assert!(off[2] <= 3, "{why}");
            }
        }
    }

    #[test]
    fn the_high_point_is_above_everything_else_and_takes_the_accent() {
        for seed in 1..=40 {
            let req = request(2, seed % 2 == 0, seed);
            let (row, settings) = row_of(&req, false);
            let strokes = strokes_at(&TWO_BARS, row.steps);
            let (pitch, accent) = riff_tune(
                &part(&row, &settings),
                &strokes,
                1,
                marks_of(&row),
                &mut dice(&req, Job::Pitch, 0),
            );
            assert_eq!(accent, Some(3));
            let top = pitch[3].expect("a note");
            for (i, p) in pitch.iter().enumerate() {
                assert!(i == 3 || p.expect("a note") < top, "seed {seed}: {pitch:?}");
            }
        }
    }

    #[test]
    fn later_bars_bring_the_tune_back() {
        // Over one chord the tune comes back as it was: the notes of bar
        // 2 before its landing are the tune's, a note that was off the
        // chord on a beat moved onto it.
        for seed in 1..=20 {
            let req = request(4, true, seed);
            let (row, settings) = row_of(&req, true);
            let at: Vec<i32> = (0..4)
                .flat_map(|b| [0, 3, 6, 10].map(|s| b * 16 + s))
                .collect();
            let strokes = strokes_at(&at, row.steps);
            let (pitch, _) = riff_tune(
                &part(&row, &settings),
                &strokes,
                1,
                marks_of(&row),
                &mut dice(&req, Job::Pitch, 0),
            );
            let p: Vec<i32> = pitch.iter().map(|p| p.expect("a note")).collect();
            // Steps 16, 19 and 22 of bar 2 against 0, 3 and 6 of bar 1.
            assert_eq!(p[4], p[0], "seed {seed}: {p:?}");
            assert_eq!(p[5], p[1], "seed {seed}: {p:?}");
            assert_eq!(p[6], p[2], "seed {seed}: {p:?}");
        }
    }

    #[test]
    fn a_ghost_has_no_note_and_a_riff_of_ghosts_no_tune() {
        let req = request(1, true, 1);
        let (row, settings) = row_of(&req, false);
        let mut strokes = strokes_at(&[0, 2, 4, 8], row.steps);
        strokes[1].ghost = true;
        let band = part(&row, &settings);
        let (pitch, _) = riff_tune(&band, &strokes, 1, &[], &mut dice(&req, Job::Pitch, 0));
        assert!(pitch[1].is_none());
        assert_eq!(pitch.iter().flatten().count(), 3);
        strokes.iter_mut().for_each(|s| s.ghost = true);
        let (pitch, accent) = riff_tune(&band, &strokes, 1, &[], &mut dice(&req, Job::Pitch, 0));
        assert!(pitch.iter().all(Option::is_none));
        assert_eq!(accent, None);
    }

    #[test]
    fn a_riff_may_leap_in_every_bar_where_a_tune_may_not() {
        let (row, _) = row_of(&request(4, true, 1), true);
        // Up a fifth to an accent and back, in each of four bars.
        let riff: Vec<GridNote> = (0..4)
            .flat_map(|b| {
                [
                    note(b * 16, 3, 0),
                    note(b * 16 + 4, 3, 4),
                    note(b * 16 + 8, 3, 0),
                ]
            })
            .collect();
        let c = flows(&row, &riff, &pitches_of(&row, &riff));
        assert!(c.pass, "{}", c.words);
        // Three leaps in a bar is one too many.
        let mut busy = riff.clone();
        busy.insert(3, note(11, 1, 4));
        busy.insert(4, note(12, 3, 0));
        let c = flows(&row, &busy, &pitches_of(&row, &busy));
        assert!(!c.pass, "{}", c.words);
        // A leap wider than an octave never passes.
        let wild = [note(0, 3, 0), note(4, 3, 8), note(8, 3, 7)];
        assert!(!flows(&row, &wild, &pitches_of(&row, &wild)).pass);
        // What follows a beat's rest is a new start, not a leap.
        let breath = [note(0, 3, 0), note(3, 1, 1), note(8, 4, 8), note(12, 4, 7)];
        let c = flows(&row, &breath, &pitches_of(&row, &breath));
        assert!(c.pass, "{}", c.words);
    }

    #[test]
    fn a_riff_goes_somewhere_from_its_first_note() {
        let (row, _) = row_of(&request(2, true, 1), true);
        // The top a fourth above the first note, on beat 3 of bar 1: a
        // tune's critic would call that its opening bar.
        let late = [note(0, 2, 0), note(4, 2, 1), note(8, 4, 3), note(16, 4, 0)];
        let c = somewhere(&row, &late, &pitches_of(&row, &late));
        assert!(c.pass, "{}", c.words);
        // The same top on the first beat goes nowhere.
        let early = [note(0, 2, 3), note(4, 2, 1), note(8, 4, 0), note(16, 4, 0)];
        assert!(!somewhere(&row, &early, &pitches_of(&row, &early)).pass);
        // A step above the first note is not a high point.
        let flat = [note(0, 2, 0), note(4, 2, 1), note(8, 4, 1), note(16, 4, 0)];
        assert!(!somewhere(&row, &flat, &pitches_of(&row, &flat)).pass);
    }

    #[test]
    fn a_riff_leads_back_by_a_step_the_fifth_or_its_first_chord() {
        let (row, _) = row_of(&request(2, true, 1), true);
        let strokes = strokes_at(&[0, 8, 24, 28], row.steps);
        let unit = unit_of(&STYLES[1]);
        let ends_on = |pos: i32| {
            let notes = [
                note(0, 4, 0),
                note(8, 4, 2),
                note(24, 4, 0),
                note(28, 2, pos),
            ];
            leads_back(&row, marks_of(&row), &strokes, unit, &notes)
        };
        // A step under the first note, the key's fifth, a note of the
        // home chord the first note comes in on.
        assert!(ends_on(-1).pass);
        assert!(ends_on(4).pass);
        assert!(ends_on(-5).pass);
        // The sixth above is none of them.
        let c = ends_on(5);
        assert!(!c.pass, "{}", c.words);
        // The close off its chord.
        let notes = [
            note(0, 4, 0),
            note(8, 4, 2),
            note(24, 4, 1),
            note(28, 2, -1),
        ];
        assert!(!leads_back(&row, marks_of(&row), &strokes, unit, &notes).pass);
    }
}
