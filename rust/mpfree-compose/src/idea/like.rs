//! More like this (P2): another phrase from one you have, any phrase,
//! MPFree's own, one made to order or one made from your notes.
//!
//! Three ways keep something of it and change the rest:
//!
//! - **Its rhythm, new notes** ([`Rework::NewNotes`]): the phrase is read
//!   back into strokes on the 1/16 grid (where each note starts, how long
//!   until the next, which are ghosts and which accented), and the maker
//!   of its kind picks notes for those strokes over a loop of chords from
//!   home, as New idea does: a bass and chords by Add a part's note
//!   choice, a riff by its tune along a shape, a line by its figure.
//! - **Its notes, a new rhythm** ([`Rework::NewRhythm`]): the phrase's
//!   notes in order (a chord as one) are played on a rhythm of its kind
//!   made afresh, round again from the first when the rhythm has more
//!   strokes than the phrase has notes; a ghost repeats the note before
//!   it, and the last stroke plays the phrase's last note, so it comes
//!   round as it did. The notes keep their pitches, so the backing is the
//!   home chord alone, the chord a phrase is held on.
//! - **Developed** ([`develop`]), with no band at all: moved a step up
//!   the scale, turned upside down about the home note, or its first half
//!   played twice.
//!
//! The two reworks are made as four drafts and judged by the checks of
//! the phrase's kind, as New idea is; the best is offered, and Try again
//! ([`next_rework`]) takes the next attempt not offered before. A phrase
//! the composer cannot read (not 4/4, a triplet grid, longer than four
//! bars, notes off the 1/16 grid) can still be developed.

use mpfree_arp::phrase::ACCENT_VELOCITY;
use mpfree_arp::{Phrase, PhraseKind, PhraseNote, Rate, Written};
use mpfree_context::chord::{Chord, ChordAnswer, ChordTimeline, Heard as ChordHeard};
use mpfree_context::groove::STEP;
use mpfree_context::key::Key;
use mpfree_transport::time::Tick;

use super::line::{judge_line, line_on, pulse};
use super::riff::{judge_riff, marks_of, riff_on};
use super::{
    asked, best, dice, judge_part, key_of, pushed_ticks, Backing, Draft, Given, Idea, Made,
    NewIdea, GHOST_VELOCITY, METER, TRIES,
};
use crate::cells::{lengths, Stroke};
use crate::dice::Job;
use crate::event::{Event, Role, Source};
use crate::parts::{self, chord_near, rhythm, AddPart, Relation};
use crate::profile::InstrumentKind;
use crate::progression::diatonic;
use crate::row::Row;
use crate::settings::Settings;
use crate::timing::Timing;
use mpfree_context::PartRole;
use mpfree_transport::time::Swing;

/// What of a phrase More like this keeps, when it is made again over a
/// band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rework {
    /// Its rhythm, with new notes.
    NewNotes,
    /// Its notes, on a new rhythm.
    NewRhythm,
}

/// How a phrase is developed, with no band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Develop {
    /// Every note a step higher in its scale.
    Up,
    /// Turned upside down about the home note: a step up becomes a step
    /// down. A chord keeps its shape over its lowest note.
    Upside,
    /// Its first half, played twice; the last note stays its own, so it
    /// comes round as it did.
    Halves,
}

impl Develop {
    pub const ALL: [Develop; 3] = [Develop::Up, Develop::Upside, Develop::Halves];
}

/// What the composer reads of a phrase: where each stroke is and which
/// notes it plays.
struct Read {
    bars: u8,
    minor: bool,
    swung: bool,
    strokes: Vec<Stroke>,
    /// Ticks of the accented strokes.
    accents: Vec<Tick>,
    /// The notes of each stroke that isn't a ghost, as semitones from
    /// the home note, lowest first.
    groups: Vec<Vec<i32>>,
}

/// The phrase as the composer reads it, or None when it cannot: not
/// 4/4, on a triplet grid, longer than four bars, with no notes, or with
/// notes off the 1/16 grid.
fn read(of: &Phrase, req: &NewIdea) -> Option<Read> {
    let triplet = matches!(
        of.unit,
        Rate::EighthTriplet | Rate::SixteenthTriplet | Rate::ThirtySecondTriplet
    );
    if of.beats != 4 || triplet || of.bars > 4 || of.is_empty() {
        return None;
    }
    // One, two or four bars, as New idea makes them; three are read as
    // four, a rest at the end.
    let bars = match of.bars {
        1 => 1,
        2 => 2,
        _ => 4,
    };
    let step = STEP as u32;
    if of.notes().iter().any(|n| n.start % step != 0) {
        return None;
    }
    let steps = i32::from(bars) * METER.bar_ticks() as i32 / STEP as i32;
    let mut strokes: Vec<Stroke> = Vec::new();
    let mut accents = Vec::new();
    let mut groups: Vec<Vec<i32>> = Vec::new();
    let notes = of.notes();
    let mut i = 0;
    while i < notes.len() {
        let start = notes[i].start;
        let together: Vec<&PhraseNote> =
            notes[i..].iter().take_while(|n| n.start == start).collect();
        i += together.len();
        let ghost = together.iter().all(|n| n.velocity <= GHOST_VELOCITY);
        let at = (start / step) as i32;
        if at >= steps {
            continue;
        }
        if together.iter().any(|n| n.velocity >= ACCENT_VELOCITY) {
            accents.push(Tick::from(start));
        }
        strokes.push(Stroke {
            at,
            len: 1,
            ghost,
            // An accent off the beat reads as a push.
            pushed: !ghost && at % 4 != 0 && together.iter().any(|n| n.velocity >= ACCENT_VELOCITY),
        });
        if !ghost {
            let mut g: Vec<i32> = together.iter().map(|n| i32::from(n.semis)).collect();
            g.sort_unstable();
            g.dedup();
            groups.push(g);
        }
    }
    if groups.is_empty() {
        return None;
    }
    lengths(&mut strokes, steps);
    let minor = match of.written {
        Written::Minor => true,
        Written::Major => false,
        Written::Either => req.minor,
    };
    Some(Read {
        bars,
        minor,
        swung: of.swung,
        strokes,
        accents,
        groups,
    })
}

/// Whether More like this can make `of` again keeping its rhythm or its
/// notes ([`rework`]); a phrase it cannot read can still be developed.
pub fn reworkable(of: &Phrase) -> bool {
    let req = NewIdea {
        kind: of.kind,
        style: crate::style::MINIMAL,
        minor: false,
        bars: of.bars,
        activity: 0.5,
        seed: 0,
        attempt: 0,
    };
    read(of, &req).is_some()
}

/// The request a rework of `of` makes: its kind, length and scale, the
/// rest as asked.
fn request_of(of: &Phrase, r: &Read, req: &NewIdea) -> NewIdea {
    NewIdea {
        kind: of.kind,
        bars: r.bars,
        minor: r.minor,
        ..*req
    }
}

/// More like this: `of` made again keeping its rhythm or its notes, in
/// `req`'s style, as busy as asked, with `req`'s seed and attempt (its
/// kind, length and scale are the phrase's). None when the composer
/// cannot read the phrase ([`read`]).
pub fn rework(of: &Phrase, how: Rework, req: &NewIdea) -> Option<Idea> {
    let r = read(of, req)?;
    let req = request_of(of, &r, req);
    let made = match how {
        Rework::NewNotes => {
            let over = Backing::of(&req);
            let given = Given {
                strokes: r.strokes.clone(),
                accents: r.accents.clone(),
            };
            let swung = r.swung;
            let made = best(|d| new_notes(&req, &over.row, &over.settings, swung, d, &given));
            over.idea(req.kind, made)
        }
        Rework::NewRhythm => {
            let over = home_backing(&req);
            let swung = over.swung;
            // The best of the drafts, each with the chords its notes spell.
            let mut chosen: Option<(Made, ChordTimeline, Row)> = None;
            for d in 0..super::DRAFTS {
                let (m, chords, row) =
                    new_rhythm(&req, &over.row, &over.settings, swung, d, &r.groups);
                let better = chosen.as_ref().is_none_or(|(b, _, _)| {
                    (m.verdict.passed(), m.verdict.score())
                        > (b.verdict.passed(), b.verdict.score())
                });
                if better {
                    chosen = Some((m, chords, row));
                }
            }
            let (made, chords, row) = chosen.expect("at least one draft");
            Backing {
                chords,
                row,
                ..over
            }
            .idea(req.kind, made)
        }
    };
    Some(made)
}

/// Try again on a rework: the attempt asked, or the first after it whose
/// phrase is neither `of` nor any of `offered`, and the attempt it was.
/// None when the phrase cannot be read, or [`TRIES`] attempts in a row
/// made nothing new.
pub fn next_rework(
    of: &Phrase,
    how: Rework,
    req: &NewIdea,
    offered: &[Phrase],
) -> Option<(Idea, u16)> {
    read(of, req)?;
    (0..TRIES).find_map(|k| {
        let attempt = req.attempt.wrapping_add(k);
        let idea = rework(of, how, &NewIdea { attempt, ..*req })?;
        let p = idea.phrase();
        (p.notes() != of.notes() && !offered.contains(&p)).then_some((idea, attempt))
    })
}

/// A backing of the home chord alone, with the style's beat.
fn home_backing(req: &NewIdea) -> Backing {
    let key = key_of(req.minor);
    let settings = super::settings_of(req);
    let home = diatonic(key, 0, req.style.harmony.sevenths);
    let chords = ChordTimeline::whole(
        Tick::from(settings.bars) * METER.bar_ticks(),
        ChordAnswer::Chord(home),
        ChordHeard::Suggested,
    );
    let row = super::band(key, &chords, &settings);
    Backing {
        key,
        settings,
        chords,
        row,
        swung: req.style.rhythm.feel == crate::style::Feel::Swing8,
    }
}

/// Draft `d` of new notes on the given rhythm.
fn new_notes(
    req: &NewIdea,
    row: &Row,
    settings: &Settings,
    swung: bool,
    d: u16,
    given: &Given,
) -> Made {
    match req.kind {
        PhraseKind::Bass | PhraseKind::Chords => {
            // A rhythm given may sit with the kick or between the kicks,
            // whichever the style's part would: half the drafts hear it
            // each way, and the checks pick.
            let mut band = asked(req, row, settings);
            if d % 2 == 1 {
                band.relation = match band.relation {
                    Relation::Support => Relation::Interlock,
                    _ => Relation::Support,
                };
            }
            let added = parts::draft_on(&band, given.strokes.clone(), d, req.attempt);
            Draft {
                accents: given.accents.clone(),
                strokes: added.strokes,
                events: added.events,
            }
            .heard(row, swung, |strokes, heard| {
                judge_part(&band, strokes, heard)
            })
        }
        PhraseKind::Riff => riff_on(req, row, settings, swung, d, Some(given)),
        PhraseKind::Line => line_on(req, row, settings, swung, d, Some(given)),
    }
}

/// Draft `d` of the phrase's notes on a new rhythm of its kind, with
/// the chords its notes spell where they now fall and the band over
/// those chords, which it is judged against.
fn new_rhythm(
    req: &NewIdea,
    row: &Row,
    settings: &Settings,
    swung: bool,
    d: u16,
    groups: &[Vec<i32>],
) -> (Made, ChordTimeline, Row) {
    let home = i32::from(super::written::home_of(req.kind, row.key));
    let strokes = match req.kind {
        PhraseKind::Bass | PhraseKind::Chords => {
            let band = asked(req, row, settings);
            rhythm(&band, d, &mut dice(req, Job::Cells, d)).0
        }
        PhraseKind::Riff => {
            let band = AddPart {
                row,
                part: PartRole::Melody,
                relation: Relation::Support,
                settings,
                kind: InstrumentKind::OneNote,
                pads: [None; 4],
                swing: Swing::default(),
            };
            rhythm(&band, d, &mut dice(req, Job::Cells, d)).0
        }
        PhraseKind::Line => pulse(req, row, settings, &mut dice(req, Job::Cells, d)),
    };
    let real: Vec<usize> = (0..strokes.len()).filter(|&i| !strokes[i].ghost).collect();
    let last = real.last().copied();
    let mut events: Vec<Event> = Vec::new();
    let mut before: Option<i32> = None;
    let mut k = 0usize;
    // A riff rests where the plan rests, as a riff made to order does: a
    // stroke in a rest is not played, and a note ends where one starts.
    let rests: &[(i32, i32)] = if req.kind == PhraseKind::Riff {
        &row.plan.rests
    } else {
        &[]
    };
    for (i, s) in strokes.iter().enumerate() {
        if rests.iter().any(|&(from, to)| from <= s.at && s.at < to) {
            continue;
        }
        let mut until = rests
            .iter()
            .map(|&(from, _)| from)
            .filter(|&from| s.at < from && from < s.at + s.len)
            .min()
            .unwrap_or(s.at + s.len);
        // A riff with no plan to rest in (one bar) breathes before it
        // comes round: its last note, longer than an 1/8, ends an 1/8
        // early.
        let eighth = (row.grid.beat / 2).max(1);
        if req.kind == PhraseKind::Riff
            && marks_of(row).is_empty()
            && Some(i) == last
            && s.len > eighth
        {
            until = until.min(s.at + s.len - eighth);
        }
        let at = row.tick(s.at);
        let len = (Tick::from(until - s.at) * STEP * 4 / 5).max(STEP / 3);
        let chord = chord_near(row, s.at);
        let role_of = |p: i32| match chord.and_then(|c| c.tone_of(p)) {
            Some(t) => Role::Chord(t),
            None => Role::Scale,
        };
        if s.ghost {
            // A ghost repeats the note before it (the phrase's first, when
            // it comes first).
            let p = before.unwrap_or(home + groups[0].last().copied().unwrap_or(0));
            events.push(note(at, len, p, Role::Ghost));
            continue;
        }
        let g = if Some(i) == last {
            &groups[groups.len() - 1]
        } else {
            &groups[k % groups.len()]
        };
        k += 1;
        for &semis in g {
            let p = home + semis;
            events.push(note(at, len, p, role_of(p)));
        }
        before = g.last().map(|&s| home + s);
    }
    let accents = pushed_ticks(&strokes);
    let st = &req.style;
    let chords = spelled(row.key, &events, settings.bars, st.harmony.sevenths);
    let row = super::band(row.key, &chords, settings);
    // Each note's part in the chord it now sounds over.
    for e in events.iter_mut().filter(|e| e.role != Role::Ghost) {
        let step = (e.at / STEP) as i32;
        e.role = match chord_near(&row, step).and_then(|c| c.tone_of(i32::from(e.pitch))) {
            Some(t) => Role::Chord(t),
            None => Role::Scale,
        };
    }
    let row_ref = &row;
    let made = Draft {
        strokes,
        events,
        accents,
    }
    .heard(row_ref, swung, |strokes, heard| match req.kind {
        PhraseKind::Bass | PhraseKind::Chords => {
            judge_part(&asked(req, row_ref, settings), strokes, heard)
        }
        PhraseKind::Riff => judge_riff(row_ref, st, marks_of(row_ref), strokes, heard),
        PhraseKind::Line => judge_line(row_ref, st, heard),
    });
    (made, chords, row)
}

/// The chords `events` spell, a chord of the key to each bar: the one
/// whose notes they sound longest (a downbeat counting double), the home
/// chord where nothing is played or two are as good.
fn spelled(key: Key, events: &[Event], bars: u8, sevenths: bool) -> ChordTimeline {
    let bar = METER.bar_ticks();
    let mut out: Vec<(Chord, Tick)> = Vec::new();
    for b in 0..Tick::from(bars) {
        let (from, to) = (b * bar, (b + 1) * bar);
        let best = (0..7)
            .map(|deg| diatonic(key, deg, sevenths))
            .enumerate()
            .max_by(|(i, x), (j, y)| {
                let score = |c: &Chord| -> i64 {
                    events
                        .iter()
                        .filter(|e| e.role != Role::Ghost && from <= e.at && e.at < to)
                        .map(|e| {
                            let w = e.len.max(1) * if e.at == from { 2 } else { 1 };
                            if c.tone_of(i32::from(e.pitch)).is_some() {
                                w
                            } else {
                                -w / 2
                            }
                        })
                        .sum()
                };
                // Ties to the home chord, then the lower degree.
                (score(x), std::cmp::Reverse(*i)).cmp(&(score(y), std::cmp::Reverse(*j)))
            })
            .map(|(_, c)| c)
            .unwrap_or_else(|| diatonic(key, 0, sevenths));
        match out.last_mut() {
            Some((c, len)) if *c == best => *len += bar,
            _ => out.push((best, bar)),
        }
    }
    ChordTimeline::of(&out, ChordHeard::Suggested)
}

fn note(at: Tick, len: Tick, pitch: i32, role: Role) -> Event {
    Event {
        at,
        len,
        pitch: pitch.clamp(0, 127) as u8,
        velocity: mpfree_arp::phrase::PLAIN_VELOCITY,
        role,
        source: Source::Mpfree,
        timing: Timing::default(),
    }
}

// ------------------------------------------------------------- developed

/// Semitones from the home note to each step of the scale `written`
/// counts in (either counts from the major scale, as degrees are named).
fn scale_of(written: Written) -> [i32; 7] {
    match written {
        Written::Minor => [0, 2, 3, 5, 7, 8, 10],
        Written::Major | Written::Either => [0, 2, 4, 5, 7, 9, 11],
    }
}

/// Semitones from home of `steps` steps of the scale and `alt` half steps.
fn semis_of(scale: &[i32; 7], steps: i32, alt: i32) -> i32 {
    12 * steps.div_euclid(7) + scale[steps.rem_euclid(7) as usize] + alt
}

/// Notes further than this from home are out of a phrase's reach.
const REACH: i32 = 48;

/// `of` developed as `how` asks, or None when that changes nothing (a
/// phrase of one note turned upside down, halves already alike) or would
/// take a note out of reach.
pub fn develop(of: &Phrase, how: Develop) -> Option<Phrase> {
    let scale = scale_of(of.written);
    let notes = of.notes();
    if notes.is_empty() {
        return None;
    }
    let mut out: Vec<(u32, u32, i32, u8)> = Vec::new();
    match how {
        Develop::Up => {
            for n in notes {
                let semis = semis_of(&scale, i32::from(n.steps) + 1, i32::from(n.alt));
                out.push((n.start, n.len, semis, n.velocity));
            }
        }
        Develop::Upside => {
            let mut i = 0;
            while i < notes.len() {
                let start = notes[i].start;
                let group: Vec<&PhraseNote> =
                    notes[i..].iter().take_while(|n| n.start == start).collect();
                i += group.len();
                // The lowest note mirrored about home; the others keep
                // their steps over it.
                let low = group
                    .iter()
                    .min_by_key(|n| n.semis)
                    .copied()
                    .copied()
                    .unwrap_or_default();
                let mirrored = -i32::from(low.steps);
                for n in &group {
                    let steps = mirrored + i32::from(n.steps) - i32::from(low.steps);
                    let alt = if n.semis == low.semis {
                        -i32::from(n.alt)
                    } else {
                        i32::from(n.alt)
                    };
                    out.push((n.start, n.len, semis_of(&scale, steps, alt), n.velocity));
                }
            }
        }
        Develop::Halves => {
            let half = of.ticks() / 2;
            let first: Vec<&PhraseNote> = notes.iter().filter(|n| n.start < half).collect();
            if first.is_empty() {
                return None;
            }
            // The phrase's last start, whose notes close the second half.
            let last_start = notes.iter().map(|n| n.start).max().unwrap_or(0);
            let closing: Vec<&PhraseNote> =
                notes.iter().filter(|n| n.start == last_start).collect();
            for n in &first {
                let len = n.len.min(half - n.start).max(1);
                out.push((n.start, len, i32::from(n.semis), n.velocity));
            }
            let copy_last = first.iter().map(|n| n.start).max().unwrap_or(0) + half;
            for n in &first {
                let start = n.start + half;
                let len = n.len.min(of.ticks() - start).max(1);
                if start == copy_last && last_start >= half {
                    continue;
                }
                out.push((start, len, i32::from(n.semis), n.velocity));
            }
            if last_start >= half {
                for n in &closing {
                    let start = copy_last;
                    let len = n.len.min(of.ticks() - start).max(1);
                    out.push((start, len, i32::from(n.semis), n.velocity));
                }
            }
        }
    }
    if out.iter().any(|&(_, _, s, _)| s.abs() > REACH) {
        return None;
    }
    let home: i32 = 60;
    let played: Vec<(u32, u32, u8, u8)> = out
        .iter()
        .map(|&(start, len, s, v)| (start, len, (home + s) as u8, v))
        .collect();
    let mut p = Phrase::from_notes(&played, home as u8, of.written, of.unit, of.beats);
    p.kind = of.kind;
    p.bars = of.bars;
    p.swung = of.swung;
    (p.notes() != of.notes()).then_some(p)
}

/// Improvise's Evolve (S5): `of` with a little of it changed, for the
/// `generation`th time (1 on). A run of about a quarter of its places (a
/// place is a start, a chord counting once; at least one, never the
/// last, so it comes round as it did) takes the notes More like this's
/// new notes give those places ([`Rework::NewNotes`], which keeps the
/// rhythm), or, where it has none (a phrase the composer cannot read),
/// moves a step up or down its scale. The rhythm stays. Each generation
/// rolls its own place, so Evolve every few bars wanders a little at a
/// time, and Return goes back to the first phrase. None for a phrase of
/// one place.
pub fn evolve(of: &Phrase, req: &NewIdea, generation: u32) -> Option<Phrase> {
    let notes = of.notes();
    let mut places: Vec<u32> = notes.iter().map(|n| n.start).collect();
    places.dedup();
    if places.len() < 2 {
        return None;
    }
    let free = places.len() - 1;
    let run = ((places.len() as f64 / 4.0).round() as usize).clamp(1, free);
    let attempt = req.attempt.wrapping_add(generation as u16);
    let fresh = rework(of, Rework::NewNotes, &NewIdea { attempt, ..*req }).map(|i| i.phrase());
    let scale = scale_of(of.written);
    let mut d = dice(req, Job::Variation, generation as u16);
    let first = d.below(free - run + 1);
    let up = d.chance(0.5);
    for shift in 0..=free - run {
        let from = (first + shift) % (free - run + 1);
        let changed: Vec<u32> = places[from..from + run].to_vec();
        let mut out: Vec<(u32, u32, i32, u8)> = Vec::new();
        for &start in &places {
            let group: Vec<&PhraseNote> = notes.iter().filter(|n| n.start == start).collect();
            let theirs: Vec<&PhraseNote> = fresh
                .as_ref()
                .map(|p| p.notes().iter().filter(|n| n.start == start).collect())
                .unwrap_or_default();
            if !changed.contains(&start) {
                out.extend(
                    group
                        .iter()
                        .map(|n| (n.start, n.len, i32::from(n.semis), n.velocity)),
                );
            } else if !theirs.is_empty()
                && theirs
                    .iter()
                    .map(|n| n.semis)
                    .ne(group.iter().map(|n| n.semis))
            {
                let (len, velocity) = (group[0].len, group[0].velocity);
                out.extend(
                    theirs
                        .iter()
                        .map(|n| (start, len, i32::from(n.semis), velocity)),
                );
            } else {
                let step = if up { 1 } else { -1 };
                out.extend(group.iter().map(|n| {
                    let semis = semis_of(&scale, i32::from(n.steps) + step, i32::from(n.alt));
                    (n.start, n.len, semis, n.velocity)
                }));
            }
        }
        if out.iter().any(|&(_, _, s, _)| s.abs() > REACH) {
            continue;
        }
        let home: i32 = 60;
        let played: Vec<(u32, u32, u8, u8)> = out
            .iter()
            .map(|&(start, len, s, v)| (start, len, (home + s) as u8, v))
            .collect();
        let mut p = Phrase::from_notes(&played, home as u8, of.written, of.unit, of.beats);
        p.kind = of.kind;
        p.bars = of.bars;
        p.swung = of.swung;
        if p.notes() != of.notes() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idea::new_idea;
    use crate::style::STYLES;

    fn req(kind: PhraseKind, bars: u8, seed: u64) -> NewIdea {
        NewIdea {
            kind,
            style: STYLES[1],
            minor: true,
            bars,
            activity: 0.5,
            seed,
            attempt: 0,
        }
    }

    fn starts(p: &Phrase) -> Vec<u32> {
        let mut v: Vec<u32> = p.notes().iter().map(|n| n.start).collect();
        v.dedup();
        v
    }

    fn pitches(p: &Phrase) -> Vec<i8> {
        p.notes().iter().map(|n| n.semis).collect()
    }

    /// Evolve changes a little at a time: a run of about a quarter of the
    /// places, never the last, the rhythm kept, each generation its own.
    #[test]
    fn evolve_changes_a_little_of_the_phrase_each_time() {
        for kind in PhraseKind::ALL {
            for bars in [1u8, 2, 4] {
                let of = new_idea(&req(kind, bars, 3)).phrase();
                let places = starts(&of);
                let mut moved = Vec::new();
                for g in 1..=6 {
                    let p = evolve(&of, &req(kind, bars, 5), g).expect("evolves");
                    let what = format!("{kind:?} {bars} bars, generation {g}");
                    assert_eq!(starts(&p), places, "{what}: the rhythm");
                    let changed: Vec<u32> = places
                        .iter()
                        .copied()
                        .filter(|&s| {
                            let at = |q: &Phrase| -> Vec<i8> {
                                q.notes()
                                    .iter()
                                    .filter(|n| n.start == s)
                                    .map(|n| n.semis)
                                    .collect()
                            };
                            at(&p) != at(&of)
                        })
                        .collect();
                    assert!(!changed.is_empty(), "{what}: nothing changed");
                    let most = (places.len() as f64 / 4.0).round().max(1.0) as usize;
                    assert!(
                        changed.len() <= most,
                        "{what}: {} of {} places",
                        changed.len(),
                        places.len()
                    );
                    assert!(
                        !changed.contains(places.last().unwrap()),
                        "{what}: the last place"
                    );
                    moved.push(changed[0]);
                }
                moved.sort_unstable();
                moved.dedup();
                // Unless only one run fits before the last place.
                let run = (places.len() as f64 / 4.0).round().max(1.0) as usize;
                if places.len() - 1 > run {
                    assert!(
                        moved.len() >= 2,
                        "{kind:?} {bars} bars: always the same place"
                    );
                }
            }
        }
        // A phrase the composer can't read (triplets) moves by steps.
        let t = Phrase::parse("riff minor 1/8T : 1 b3 5 b3 1 5-").unwrap();
        let p = evolve(&t, &req(PhraseKind::Riff, 1, 5), 1).expect("evolves");
        assert_eq!(starts(&p), starts(&t));
        assert_ne!(pitches(&p), pitches(&t));
        // One place: nothing to evolve.
        let one = Phrase::parse("bass minor : 1 _ _ _").unwrap();
        assert_eq!(evolve(&one, &req(PhraseKind::Bass, 1, 5), 1), None);
    }

    /// Its rhythm, new notes: every kind keeps the phrase's starts and
    /// changes its notes.
    #[test]
    fn new_notes_keep_the_rhythm_and_change_the_notes() {
        for kind in PhraseKind::ALL {
            for bars in [1u8, 2, 4] {
                let of = new_idea(&req(kind, bars, 3)).phrase();
                // Try again's way: never the phrase itself.
                let (again, _) =
                    next_rework(&of, Rework::NewNotes, &req(kind, bars, 9), &[]).expect("read");
                let p = again.phrase();
                assert_eq!(starts(&p), starts(&of), "{kind:?} {bars}");
                assert_ne!(pitches(&p), pitches(&of), "{kind:?} {bars}");
                assert_eq!(p.kind, of.kind);
            }
        }
    }

    /// Its notes, a new rhythm: the notes come in the phrase's order and
    /// the last is its last, on another rhythm.
    #[test]
    fn new_rhythm_keeps_the_notes_in_order() {
        for kind in [PhraseKind::Bass, PhraseKind::Riff, PhraseKind::Line] {
            let of = new_idea(&req(kind, 2, 5)).phrase();
            let Some((again, _)) = next_rework(&of, Rework::NewRhythm, &req(kind, 2, 11), &[])
            else {
                panic!("{kind:?}: nothing new");
            };
            let p = again.phrase();
            assert_ne!(starts(&p), starts(&of), "{kind:?}");
            let played: Vec<i8> = of
                .notes()
                .iter()
                .filter(|n| n.velocity > GHOST_VELOCITY)
                .map(|n| n.semis)
                .collect();
            let new: Vec<i8> = p
                .notes()
                .iter()
                .filter(|n| n.velocity > GHOST_VELOCITY)
                .map(|n| n.semis)
                .collect();
            // Every note it plays is one of the phrase's, and it ends on
            // the phrase's last.
            assert!(new.iter().all(|s| played.contains(s)), "{kind:?}");
            assert_eq!(new.last(), played.last(), "{kind:?}");
        }
    }

    /// A phrase the composer cannot read: a 3/4 phrase has no rework, and
    /// can still be developed.
    #[test]
    fn a_phrase_in_three_is_developed_but_not_reworked() {
        let of = Phrase::parse("riff minor 3/4 : 1 . 3 5 . 3").unwrap();
        assert!(rework(&of, Rework::NewNotes, &req(PhraseKind::Riff, 1, 1)).is_none());
        assert!(!reworkable(&of));
        assert!(develop(&of, Develop::Up).is_some());
        let four = Phrase::parse("riff minor : 1 . 3 5 . 3 . .").unwrap();
        assert!(reworkable(&four));
    }

    /// Up a step, upside down and first half twice, on a written phrase.
    #[test]
    fn developed_three_ways() {
        let of = Phrase::parse("riff major : 1 . 2 . 3 . 5 . 6 . 5 . 3 . 2 .").unwrap();
        let up = develop(&of, Develop::Up).unwrap();
        // 1 2 3 5 6 5 3 2 a step up the major scale: 2 3 4 6 7 6 4 3.
        assert_eq!(pitches(&up), vec![2, 4, 5, 9, 11, 9, 5, 4]);
        let upside = develop(&of, Develop::Upside).unwrap();
        // Mirrored about home: 1 7- 6- 4- 3- 4- 6- 7-.
        assert_eq!(pitches(&upside), vec![0, -1, -3, -7, -8, -7, -3, -1]);
        let halves = develop(&of, Develop::Halves).unwrap();
        // 1 2 3 5, then 1 2 3 and the phrase's own last note, 2.
        assert_eq!(pitches(&halves), vec![0, 2, 4, 7, 0, 2, 4, 2]);
        assert_eq!(starts(&halves), starts(&of));
        // Halves alike already: nothing to do.
        let alike = Phrase::parse("riff major : 1 . 3 . 1 . 3 . 1 . 3 . 1 . 3 .").unwrap();
        assert!(develop(&alike, Develop::Halves).is_none());
        // One note upside down is itself.
        let one = Phrase::parse("bass minor : 1 . . . 1 . . .").unwrap();
        assert!(develop(&one, Develop::Upside).is_none());
    }

    /// Most reworks pass every check of their kind (measured over more
    /// by `print_how_often_reworks_pass`: 70 to 90 of each 90).
    #[test]
    fn most_reworks_pass_every_check() {
        for kind in PhraseKind::ALL {
            for how in [Rework::NewNotes, Rework::NewRhythm] {
                let (mut n, mut pass) = (0, 0);
                for style in STYLES {
                    for bars in [1u8, 2, 4] {
                        let of = new_idea(&NewIdea {
                            style,
                            ..req(kind, bars, 2)
                        })
                        .phrase();
                        let r = NewIdea {
                            style,
                            ..req(kind, bars, 8)
                        };
                        if let Some((idea, _)) = next_rework(&of, how, &r, &[]) {
                            n += 1;
                            pass += usize::from(idea.verdict.all());
                        }
                    }
                }
                assert!(
                    n >= 6 && pass * 4 >= n * 3,
                    "{kind:?} {how:?}: {pass} of {n}"
                );
            }
        }
    }

    /// The same request always makes the same rework.
    #[test]
    fn a_rework_is_the_same_every_time() {
        let of = new_idea(&req(PhraseKind::Chords, 2, 4)).phrase();
        let a = rework(&of, Rework::NewNotes, &req(PhraseKind::Chords, 2, 7)).unwrap();
        let b = rework(&of, Rework::NewNotes, &req(PhraseKind::Chords, 2, 7)).unwrap();
        assert_eq!(a.phrase(), b.phrase());
    }
}

#[cfg(test)]
mod measure {
    use super::*;
    use crate::idea::new_idea;
    use crate::style::STYLES;

    /// How often a rework passes every check of its kind, against the
    /// idea it was made from: `cargo test --release -p mpfree-compose
    /// --lib print_how_often_reworks_pass -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_how_often_reworks_pass() {
        for kind in PhraseKind::ALL {
            for how in [Rework::NewNotes, Rework::NewRhythm] {
                let (mut n, mut from, mut again) = (0, 0, 0);
                let mut missed = std::collections::BTreeMap::new();
                for style in STYLES {
                    for bars in [1u8, 2, 4] {
                        for seed in 0..10u64 {
                            let req = NewIdea {
                                kind,
                                style,
                                minor: seed % 2 == 0,
                                bars,
                                activity: 0.5,
                                seed,
                                attempt: 0,
                            };
                            let idea = new_idea(&req);
                            let of = idea.phrase();
                            let Some((r, _)) = next_rework(
                                &of,
                                how,
                                &NewIdea {
                                    seed: seed + 100,
                                    ..req
                                },
                                &[],
                            ) else {
                                continue;
                            };
                            n += 1;
                            from += usize::from(idea.verdict.all());
                            again += usize::from(r.verdict.all());
                            for (name, c) in &r.verdict.checks {
                                if !c.pass {
                                    *missed.entry(*name).or_insert(0usize) += 1;
                                }
                            }
                        }
                    }
                }
                println!("{kind:?} {how:?}: {again} of {n} pass every check (the ideas: {from}); missed {missed:?}");
            }
        }
    }
}
