//! Add a part (revision 3, S2): a bass, chords, a second line or drums
//! that play with the rest of a row, by a role.
//!
//! The row is heard as one picture first ([`crate::row`]): its parts, its
//! drums, the tune's plan. The new part's rhythm is a riff of cells that
//! fits what its role wants from the row, brought back changed where the
//! plan asks ([`crate::cells`]); then its notes are chosen for the whole
//! part at once, so a bass lands where the tune lands and moves against
//! it, and chords sit between the bass and the tune with the least
//! movement.
//!
//! The four roles (on screen "Role") are weightings, not rules:
//!
//! - **Support** plays with the beat and under the tune: a bass mostly on
//!   the kick, chords on the kick and the landings, a line in 3rds and
//!   6ths under the tune's longer notes.
//! - **Answer** replies in the tune's gaps.
//! - **Interlock** keeps its own rhythm, between the kick, the snare and
//!   the tune's notes.
//! - **Stay out** plays about one note per chord, held.
//!
//! Each part is made as a few whole drafts, judged with the row on
//! separate checks ([`PartVerdict`]), and the best is offered. A chord
//! change re-fits a part ([`refit`]): the same rhythm, notes chosen again.

use mpfree_context::chord::{Chord, Tone};
use mpfree_context::groove::{Lane, STEP};
use mpfree_context::PartRole;
use mpfree_transport::time::{Swing, Tick};

use crate::cells::{
    develop, ghosts, lay, lengths, library, riff, Aim, CellPart, Change, Develop, Stroke,
};
use crate::critic::Check;
use crate::dice::{Dice, DiceName, Job};
use crate::event::{Event, Role, Source};
use crate::profile::{InstrumentKind, HIT};
use crate::row::Row;
use crate::settings::Settings;
use crate::sketch::AnchorKind;
use crate::style::{BassLine, Comping, DrumMap, Feel, Style};
use crate::timing::Timing;

/// How a new part plays with the row (shown as "Role").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relation {
    Support,
    Answer,
    Interlock,
    StayOut,
}

impl Relation {
    pub const ALL: [Relation; 4] = [
        Relation::Support,
        Relation::Answer,
        Relation::Interlock,
        Relation::StayOut,
    ];

    pub const fn words(self) -> &'static str {
        match self {
            Relation::Support => "Support",
            Relation::Answer => "Answer",
            Relation::Interlock => "Interlock",
            Relation::StayOut => "Stay out",
        }
    }

    /// What it does, in a line.
    pub const fn says(self) -> &'static str {
        match self {
            Relation::Support => "Plays with the beat and under the tune",
            Relation::Answer => "Replies in the tune's gaps",
            Relation::Interlock => "Its own rhythm, between the others",
            Relation::StayOut => "Few notes, room for the others",
        }
    }
}

/// One Add a part request.
#[derive(Clone, Copy)]
pub struct AddPart<'a> {
    pub row: &'a Row,
    /// What to add: Bass, Chords, Melody (a second line) or Drums.
    pub part: PartRole,
    pub relation: Relation,
    pub settings: &'a Settings,
    /// What plays it.
    pub kind: InstrumentKind,
    /// For drums, the kit's pad for each drum (kick, snare, hats, the
    /// rest): only drums the kit names are played.
    pub pads: [Option<u8>; 4],
    /// The swing the new clip plays with.
    pub swing: Swing,
}

/// Why no part was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A pitched part needs chords: the row has none, heard or picked.
    NoChords,
    /// Drums need pads the kit names as drums.
    NoDrumPads,
    /// Answer replies to a tune, and the row has none.
    NothingToAnswer,
}

impl Refused {
    pub const fn words(self) -> &'static str {
        match self {
            Refused::NoChords => "No chords to play over: pick chords for the row, or keep the suggested ones",
            Refused::NoDrumPads => "The kit has no pads named as drums (kick, snare, hats)",
            Refused::NothingToAnswer => "Answer replies to a tune, and this row has none yet: add the tune first, or pick another role",
        }
    }
}

/// A part made for the row.
#[derive(Clone, Debug, PartialEq)]
pub struct Added {
    pub part: PartRole,
    pub relation: Relation,
    /// Its rhythm, which a chord change keeps (empty for drums).
    pub strokes: Vec<Stroke>,
    /// The named changes the riff came back with, by bar.
    pub changes: Vec<(usize, Change)>,
    /// Its notes, in row ticks, with the groove's timing.
    pub events: Vec<Event>,
    pub verdict: PartVerdict,
    pub draft: u16,
}

/// Whole drafts made for each part; the best is offered.
pub const DRAFTS: u16 = 4;

/// Every check's answer for one part with the row.
#[derive(Clone, Debug, PartialEq)]
pub struct PartVerdict {
    /// The role can be heard.
    pub together: Check,
    /// No half step against a note the row holds on a strong beat; no
    /// octaves or fifths in parallel with the tune.
    pub clash: Check,
    /// In its range, and under the tune as its kind should be.
    pub room: Check,
    /// On the plan's landings, on chord notes.
    pub lands: Check,
    /// The riff returns, changed now and then, never stuck.
    pub repeats: Check,
    /// No wild leaps.
    pub flows: Check,
}

impl PartVerdict {
    pub fn checks(&self) -> [(&'static str, &Check); 6] {
        [
            ("plays together", &self.together),
            ("no clash", &self.clash),
            ("room", &self.room),
            ("lands with the row", &self.lands),
            ("returns, not stuck", &self.repeats),
            ("flows", &self.flows),
        ]
    }

    pub fn passed(&self) -> usize {
        self.checks().iter().filter(|(_, c)| c.pass).count()
    }

    pub fn score(&self) -> f32 {
        self.checks().iter().map(|(_, c)| c.score).sum()
    }

    pub fn words(&self) -> String {
        self.checks()
            .iter()
            .map(|(name, c)| format!("{} {name}: {}", if c.pass { "+" } else { "-" }, c.words))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

const fn part_number(p: PartRole) -> u16 {
    match p {
        PartRole::Melody => 0x21,
        PartRole::Bass => 0x22,
        PartRole::Chords => 0x23,
        PartRole::Drums => 0x24,
    }
}

fn dice(req: &AddPart, job: Job, draft: u16, attempt: u16) -> Dice {
    DiceName {
        seed: req.settings.seed,
        part: part_number(req.part),
        phrase: draft,
        job,
        bar: req.relation as u16,
        attempt,
    }
    .dice()
}

/// Add a part: the best of [`DRAFTS`] whole drafts.
pub fn add_part(req: &AddPart) -> Result<Added, Refused> {
    add_part_again(req, 0)
}

/// Try again on the whole part: `attempt` 0 is the first answer; each
/// other attempt gives other riffs and notes.
pub fn add_part_again(req: &AddPart, attempt: u16) -> Result<Added, Refused> {
    if req.relation == Relation::Answer && req.row.lead.is_none() {
        return Err(Refused::NothingToAnswer);
    }
    match req.part {
        PartRole::Drums if req.pads.iter().all(Option::is_none) => return Err(Refused::NoDrumPads),
        PartRole::Drums => {}
        _ => {
            if !req
                .row
                .chords
                .spans()
                .iter()
                .any(|s| s.answer.chord().is_some())
            {
                return Err(Refused::NoChords);
            }
        }
    }
    let mut best: Option<Added> = None;
    for d in 0..DRAFTS {
        let a = draft(req, d, attempt);
        let better = best.as_ref().is_none_or(|b| {
            (a.verdict.passed(), a.verdict.score()) > (b.verdict.passed(), b.verdict.score())
        });
        if better {
            best = Some(a);
        }
    }
    Ok(best.expect("at least one draft"))
}

/// The same part under the row's chords as they are now: the same
/// rhythm, its notes chosen again. Drums never change.
pub fn refit(req: &AddPart, before: &Added) -> Added {
    if req.part == PartRole::Drums {
        return before.clone();
    }
    let mut pd = dice(req, Job::AddPart, before.draft, 0);
    let events = sound(req, &before.strokes, &mut pd);
    let verdict = judge(req, &before.strokes, &events);
    Added {
        events,
        verdict,
        ..before.clone()
    }
}

/// Draft `d` of a part: its riff, its notes, and what its checks found.
pub(crate) fn draft(req: &AddPart, d: u16, attempt: u16) -> Added {
    let mut cd = dice(req, Job::Cells, d, attempt);
    let (strokes, changes) = if req.part == PartRole::Drums {
        (Vec::new(), Vec::new())
    } else {
        rhythm(req, d, &mut cd)
    };
    let mut pd = dice(req, Job::AddPart, d, attempt);
    let events = sound(req, &strokes, &mut pd);
    let verdict = judge(req, &strokes, &events);
    Added {
        part: req.part,
        relation: req.relation,
        strokes,
        changes,
        events,
        verdict,
        draft: d,
    }
}

/// Draft `d` of a part on a rhythm given (More like this: a phrase's
/// rhythm with new notes): its notes and what its checks found.
pub(crate) fn draft_on(req: &AddPart, strokes: Vec<Stroke>, d: u16, attempt: u16) -> Added {
    let mut pd = dice(req, Job::AddPart, d, attempt);
    let events = sound(req, &strokes, &mut pd);
    let verdict = judge(req, &strokes, &events);
    Added {
        part: req.part,
        relation: req.relation,
        strokes,
        changes: Vec::new(),
        events,
        verdict,
        draft: d,
    }
}

// ---------------------------------------------------------------- rhythm

/// The cells a part draws from: the style's, except a walking bass plays
/// in two on Support and answers like a line, and held pad chords play
/// the off-beats on Interlock and Answer.
fn cell_part(req: &AddPart) -> CellPart {
    let st = &req.settings.style;
    match (req.part, req.relation) {
        (PartRole::Bass, Relation::Support) if st.bass.line == BassLine::Walking => {
            CellPart::TwoFeel
        }
        (PartRole::Bass, Relation::Answer) if st.bass.line == BassLine::Walking => CellPart::Line,
        (PartRole::Bass, _) => CellPart::Bass,
        (PartRole::Chords, Relation::Interlock | Relation::Answer)
            if st.bass.comping == Comping::Pad =>
        {
            CellPart::Offbeats
        }
        (PartRole::Chords, _) => CellPart::Comping,
        _ => CellPart::Line,
    }
}

/// A walking bass on Interlock walks: a note on every beat.
fn walks(req: &AddPart) -> bool {
    req.part == PartRole::Bass
        && req.relation == Relation::Interlock
        && req.settings.style.bass.line == BassLine::Walking
}

/// A gap in the tune at `s`: it rests, or holds a note that started half
/// a beat or more before, and starts nothing there (the tails of its held
/// notes, where an answer fits even in a tune that never rests).
fn tune_gap(row: &Row, s: i32) -> bool {
    if row.lead.is_none() {
        return row.plan.resting(s);
    }
    if row.lead_starts(s) {
        return false;
    }
    match row.lead_at(s) {
        None => true,
        Some(n) => row.tick(s) - n.at >= Tick::from((row.grid.beat / 2).max(1)) * STEP,
    }
}

/// How much the row wants the new part to start a note at each step.
fn desire(req: &AddPart) -> Vec<f32> {
    let row = req.row;
    let g = row.grid;
    let drums = row.has_drums();
    (0..row.steps)
        .map(|s| {
            let kick = f32::from(row.drum(Lane::Kick, s)) / 127.0;
            let snare = f32::from(row.drum(Lane::Snare, s)) / 127.0;
            let backbeat = snare > 0.5 && g.on_beat(s) && s % g.bar != 0;
            let strong = (g.level(s) + 4) as f32 / 4.0;
            let change = row.chord_starts(s);
            let mark = row.plan.mark_at(s).is_some();
            let lead_on = row.lead_starts(s);
            let sounding = row.lead_at(s).is_some();
            let gap = tune_gap(row, s);
            let off8 = g.level(s) == -3;
            let b = |x: bool| if x { 1.0 } else { 0.0 };
            let beatless = if drums { 0.0 } else { 0.6 * strong };
            -0.4 + match (req.part, req.relation) {
                (PartRole::Bass, Relation::Support) => {
                    1.4 * kick + 1.2 * b(change) + 0.8 * b(mark) + 0.4 * strong - 0.9 * b(backbeat)
                        + beatless
                }
                (PartRole::Bass, Relation::Answer) => {
                    1.2 * b(gap) - 0.9 * b(lead_on) - 0.3 * b(sounding)
                        + 0.6 * b(change)
                        + 0.2 * kick
                        - 0.6 * b(backbeat)
                }
                (PartRole::Bass, _) if walks(req) => 1.2 * b(g.on_beat(s)) + 0.4 * b(change) - 0.3,
                (PartRole::Bass, _) => {
                    0.9 * b(kick == 0.0 && snare == 0.0)
                        - 0.2 * b(lead_on)
                        - 0.5 * kick * b(s % g.bar != 0)
                        - 1.0 * snare
                        + 0.6 * b(change)
                        + 0.3 * b(off8)
                }
                (PartRole::Chords, Relation::Support) => {
                    0.8 * b(change) + 0.7 * b(mark) + 0.5 * kick + 0.3 * strong + 0.5 * beatless
                }
                (PartRole::Chords, Relation::Answer) => {
                    1.3 * b(gap) - 1.0 * b(sounding && !gap) + 0.3 * b(change)
                }
                (PartRole::Chords, _) => {
                    0.8 * b(off8) - 0.5 * snare - 0.4 * kick - 0.4 * b(lead_on) + 0.3 * b(change)
                }
                (_, Relation::Support) if row.lead.is_none() => {
                    1.0 * kick + 0.8 * b(mark) + 0.5 * b(change) + 0.3 * strong + beatless
                }
                (_, Relation::Support) => {
                    let long = row
                        .lead_at(s)
                        .is_some_and(|n| n.len >= 2 * STEP && row.tick(s) - n.at < STEP / 2);
                    1.2 * b(long) + 1.0 * b(mark) + 0.3 * b(change)
                }
                (_, Relation::Answer) => 1.3 * b(gap) - 1.2 * b(sounding && !gap) + 0.2 * strong,
                _ => 0.8 * b(!lead_on) - 0.8 * b(lead_on) + 0.2 * b(off8),
            }
        })
        .collect()
}

fn aim(req: &AddPart) -> Aim {
    let st = &req.settings.style;
    let act = 0.5 + req.settings.activity;
    let rel = match req.relation {
        Relation::Support | Relation::Interlock => 1.0,
        Relation::Answer => 0.6,
        Relation::StayOut => 0.3,
    };
    let base = match req.part {
        PartRole::Bass => match (st.bass.line, req.relation) {
            (BassLine::Sixteenths, Relation::Interlock) => 1.2,
            (BassLine::Sixteenths, _) => 1.0,
            (BassLine::Walking, Relation::Support) => 0.55,
            (BassLine::Offbeat, Relation::Support) => 0.9,
            _ => 1.0,
        },
        PartRole::Chords => match st.bass.comping {
            Comping::Pad => 0.25,
            Comping::Stabs => 0.5,
            Comping::Charleston => 0.45,
        },
        _ => st.rhythm.density * 0.5,
    };
    let sync = match req.part {
        PartRole::Bass => 0.7,
        PartRole::Chords => 1.0,
        _ => 0.8,
    };
    let mut density = base * act * rel;
    if req.relation == Relation::Answer && req.row.lead.is_some() {
        // An answer has only the tune's gaps to play in.
        let row = req.row;
        let gaps = (0..row.steps).filter(|&s| tune_gap(row, s)).count() as f32;
        let beats = (row.steps / row.grid.beat.max(1)).max(1) as f32;
        density = density.min(0.1 + 0.5 * gaps / beats);
    }
    Aim {
        density,
        syncopation: st.rhythm.syncopation * sync,
    }
}

fn blocked(row: &Row) -> Vec<bool> {
    (0..row.steps)
        .map(|s| {
            row.drum(Lane::Kick, s) > 0 || (row.drum(Lane::Snare, s) > 60 && row.grid.on_beat(s))
        })
        .collect()
}

pub(crate) fn unit_of(st: &Style) -> i32 {
    if st.rhythm.feel == Feel::Swing8 {
        2
    } else {
        1
    }
}

/// The bars of cells draft `d`'s riff is sampled over before it is laid
/// over the row: the whole row for an answer, else one bar, or two for
/// every other draft of a row that has them.
pub(crate) fn riff_bars(req: &AddPart, d: u16) -> usize {
    if req.relation == Relation::Answer {
        req.row.bars
    } else if req.row.bars >= 2 && d % 2 == 1 {
        2
    } else {
        1
    }
}

pub(crate) fn rhythm(
    req: &AddPart,
    d: u16,
    dice: &mut Dice,
) -> (Vec<Stroke>, Vec<(usize, Change)>) {
    let row = req.row;
    let g = row.grid;
    let steps = row.steps;
    let st = &req.settings.style;
    if req.relation == Relation::StayOut {
        // About one note per chord, held: on each chord's first step.
        let mut v: Vec<Stroke> = (0..steps)
            .filter(|&s| row.chord_starts(s))
            .map(Stroke::at)
            .collect();
        if v.is_empty() {
            v.push(Stroke::at(0));
        }
        lengths(&mut v, steps);
        return (v, Vec::new());
    }
    let lib = library(st, cell_part(req));
    let riff_bars = riff_bars(req, d);
    let wander = 0.2 + 0.8 * f64::from(req.settings.variation);
    let cells = riff(lib, g, riff_bars, &desire(req), aim(req), wander, dice);
    let mut v = lay(&cells, g, steps);
    let marks: Vec<(AnchorKind, i32)> = row.plan.marks.iter().map(|m| (m.kind, m.at)).collect();
    let other: Vec<bool> = (0..steps).map(|s| row.lead_starts(s)).collect();
    let blocked = blocked(row);
    let push = match st.rhythm.feel {
        Feel::Push => 0.35,
        Feel::Swing8 => 0.3,
        Feel::Simplify => 0.1,
    };
    let changes = develop(
        &mut v,
        &Develop {
            grid: g,
            bars: row.bars,
            marks: &marks,
            lands: req.relation == Relation::Support,
            push,
            feel: st.rhythm.feel,
            unit: unit_of(st),
            other: &other,
            blocked: &blocked,
        },
        dice,
    );
    // Over a beat alone, every part breathes where the plan does.
    if !row.plan.from_tune {
        v.retain(|s| !row.plan.resting(s.at));
        lengths(&mut v, steps);
    }
    // Never nothing: a part that found no place plays each chord once.
    if v.iter().all(|s| s.ghost) {
        v = (0..steps)
            .filter(|&s| row.chord_starts(s))
            .map(Stroke::at)
            .collect();
        if v.is_empty() {
            v.push(Stroke::at(0));
        }
        lengths(&mut v, steps);
    }
    if req.part == PartRole::Bass && matches!(req.relation, Relation::Support | Relation::Interlock)
    {
        let chance = f64::from(st.playing.ghosts) * (0.25 + 0.5 * f64::from(req.settings.activity));
        ghosts(&mut v, &blocked, chance, steps, g, dice);
    }
    (v, changes)
}

// ----------------------------------------------------------------- notes

fn sound(req: &AddPart, strokes: &[Stroke], dice: &mut Dice) -> Vec<Event> {
    let mut v = match req.part {
        PartRole::Bass => bass(req, strokes, dice),
        PartRole::Chords => chords(req, strokes, dice),
        PartRole::Melody => line(req, strokes, dice),
        PartRole::Drums => drums(req, dice),
    };
    v.sort_by_key(|e| (e.at, e.pitch));
    v
}

/// The chord at `step`, or the nearest before it, or the first.
pub(crate) fn chord_near(row: &Row, step: i32) -> Option<Chord> {
    let t = row.tick(step.rem_euclid(row.steps));
    let spans = row.chords.spans();
    spans
        .iter()
        .rev()
        .filter(|s| s.start <= t)
        .find_map(|s| s.answer.chord())
        .or_else(|| spans.iter().find_map(|s| s.answer.chord()))
}

/// Two notes rub when the upper is a half step above the lower, in any
/// octave: a minor 2nd or 9th. A major 7th (a tune's 9th over the chord's
/// 3rd, say) is left alone.
pub(crate) fn rubs(a: i32, b: i32) -> bool {
    (a - b).abs() % 12 == 1
}

fn pc_dist(a: i32, b: i32) -> i32 {
    let d = (a - b).rem_euclid(12);
    d.min(12 - d)
}

fn timing(req: &AddPart, lane: Lane, step: i32) -> Timing {
    match &req.row.groove {
        Some(g) => Timing::from_groove(g, lane, step.rem_euclid(req.row.steps) as usize, req.swing),
        None => Timing::default(),
    }
}

/// A stroke's loudness: the style's level and accent at its step, the
/// bar's energy, quieter for a ghost.
fn velocity(req: &AddPart, s: &Stroke, offset: i32) -> u8 {
    let st = &req.settings.style;
    let p = &st.playing;
    let g = req.row.grid;
    let step = s.at.rem_euclid(g.bar) as usize;
    let accent = i32::from(p.accent[step % p.accent.len()]);
    let bar = (s.at / g.bar) as usize;
    let energy = req.row.plan.energy_of(bar);
    let mut v = i32::from(p.velocity) + accent / 2 + offset + ((energy - 0.6) * 16.0) as i32;
    if s.pushed {
        v += 4;
    }
    if s.ghost {
        v = (v as f32 * 0.38) as i32;
    }
    v.clamp(18, 127) as u8
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BassNote {
    Chord(Tone),
    Approach,
    Passing,
}

/// The bass: every real note chosen together, root and fifth on the
/// strong beats and on each chord's first note, an approach a half step
/// into a change, the register rising with the bar's energy and the high
/// point, octaves and fifths in parallel with the tune avoided, and a
/// move against the tune into its landings.
fn bass(req: &AddPart, strokes: &[Stroke], dice: &mut Dice) -> Vec<Event> {
    let row = req.row;
    let g = row.grid;
    let st = &req.settings.style;
    let prof = req.kind.profile();
    let walking = st.bass.line == BassLine::Walking;
    let lo = i32::from(prof.low).max(28);
    let top = i32::from(prof.high).min(55);
    let real: Vec<usize> = (0..strokes.len()).filter(|&i| !strokes[i].ghost).collect();
    if real.is_empty() {
        return Vec::new();
    }
    let peak_bar = row.plan.peak().map(|m| m.at / g.bar);
    let noise = 0.12 * (0.3 + f64::from(req.settings.variation));

    struct Spot {
        step: i32,
        chord: Option<Chord>,
        strong: bool,
        starts: bool,
        next_change: Option<Chord>,
        mark: bool,
        lead: Option<u8>,
        held: Vec<u8>,
        centre: i32,
    }
    let spots: Vec<Spot> = real
        .iter()
        .enumerate()
        .map(|(k, &i)| {
            let s = strokes[i];
            let chord = chord_near(row, s.at);
            let prev_chord = k
                .checked_sub(1)
                .and_then(|j| chord_near(row, strokes[real[j]].at));
            let starts = row.chord_starts(s.at) || prev_chord != chord || k == 0;
            // What the next note leads into: a new chord, or the next
            // bar's first beat (so a long chord still gets its pickups).
            let next_change = real.get(k + 1).and_then(|&j| {
                let at = strokes[j].at;
                let c = chord_near(row, at);
                (row.chord_starts(at) || c != chord || (at % g.bar == 0 && at != s.at))
                    .then_some(c)
                    .flatten()
            });
            let mark_step = if s.pushed { s.at + unit_of(st) } else { s.at };
            let bar = s.at / g.bar;
            let energy = row.plan.energy_of(bar as usize);
            let centre =
                35 + (7.0 * energy).round() as i32 + if peak_bar == Some(bar) { 5 } else { 0 };
            let t = row.tick(s.at);
            Spot {
                step: s.at,
                chord,
                strong: g.level(s.at) >= -2 || s.pushed,
                starts,
                next_change,
                mark: row.plan.mark_at(mark_step).is_some(),
                lead: row.lead_at(s.at).map(|n| n.pitch),
                held: row.sounding(t).collect(),
                centre,
            }
        })
        .collect();

    // Candidates per spot.
    let cands: Vec<Vec<(i32, BassNote, f64)>> = spots
        .iter()
        .map(|sp| {
            let hi = if peak_bar == Some(sp.step / g.bar) {
                top
            } else {
                top - 4
            };
            let mut v = Vec::new();
            for p in lo..=hi {
                let kind = match sp.chord.and_then(|c| c.tone_of(p)) {
                    Some(t) => Some(BassNote::Chord(t)),
                    // Into the next chord by a step: a half step outside
                    // the key only when the bass walks; in the key, a half
                    // or a whole step.
                    None => match sp.next_change {
                        Some(nc)
                            if matches!(pc_dist(p, i32::from(nc.root)), 1 | 2)
                                && (row.key.has(p)
                                    || (walking && pc_dist(p, i32::from(nc.root)) == 1)) =>
                        {
                            Some(BassNote::Approach)
                        }
                        _ if walking && row.key.has(p) => Some(BassNote::Passing),
                        _ => None,
                    },
                };
                if let Some(k) = kind {
                    let role = match k {
                        BassNote::Chord(Tone::Root) => 0.0,
                        BassNote::Chord(Tone::Fifth) => {
                            if sp.starts {
                                1.5
                            } else if sp.strong {
                                0.25
                            } else {
                                0.15
                            }
                        }
                        BassNote::Chord(Tone::Third) => {
                            if sp.starts {
                                2.0
                            } else if sp.strong {
                                1.0
                            } else {
                                0.7
                            }
                        }
                        BassNote::Chord(Tone::Seventh) => {
                            if sp.starts {
                                3.0
                            } else if sp.strong {
                                1.6
                            } else {
                                0.6
                            }
                        }
                        BassNote::Approach => {
                            if sp.starts {
                                6.0
                            } else if sp.strong {
                                2.0
                            } else {
                                0.2
                            }
                        }
                        BassNote::Passing => {
                            if sp.starts {
                                6.0
                            } else if sp.strong {
                                2.5
                            } else {
                                0.6
                            }
                        }
                    };
                    let mut c = role + 0.06 * f64::from((p - sp.centre).abs());
                    if sp.strong
                        && !matches!(k, BassNote::Approach)
                        && sp.held.iter().any(|&h| rubs(p, i32::from(h)))
                    {
                        c += 1.5;
                    }
                    // A note outside the chord never a half step from one
                    // the others hold, in either direction.
                    if matches!(k, BassNote::Approach | BassNote::Passing)
                        && sp.held.iter().any(|&h| pc_dist(p, i32::from(h)) == 1)
                    {
                        c += 3.0;
                    }
                    c += dice.gumbel() * noise;
                    v.push((p, k, c));
                }
            }
            if v.is_empty() {
                // No chord and nothing in reach: the key's home note.
                let p = lo + (i32::from(row.key.tonic) - lo).rem_euclid(12);
                v.push((p, BassNote::Chord(Tone::Root), 3.0));
            }
            v
        })
        .collect();

    let step_cost = |q: i32, qk: BassNote, p: i32| -> f64 {
        let d = (p - q).abs();
        let mut c = match d {
            0 => {
                if walking {
                    1.0
                } else {
                    0.35
                }
            }
            1 | 2 => {
                if walking {
                    0.0
                } else {
                    0.1
                }
            }
            3 | 4 => 0.3,
            5 | 7 => 0.3,
            12 => 0.3,
            6 => 0.8,
            8..=11 => 0.9,
            _ => 3.0,
        };
        if qk == BassNote::Approach && !matches!(d, 1 | 2) {
            c += 4.0;
        }
        c
    };
    let pair_cost = |a: &Spot, q: i32, b: &Spot, p: i32| -> f64 {
        let mut c = 0.0;
        if let (Some(la), Some(lb)) = (a.lead, b.lead) {
            let (la, lb) = (i32::from(la), i32::from(lb));
            if a.strong && b.strong && p != q && la != lb {
                if (la - q).rem_euclid(12) == 0 && (lb - p).rem_euclid(12) == 0 {
                    c += 2.0;
                }
                if (la - q).rem_euclid(12) == 7 && (lb - p).rem_euclid(12) == 7 {
                    c += 1.0;
                }
            }
            if b.mark && la != lb && p != q {
                c += if (lb - la).signum() == (p - q).signum() {
                    0.4
                } else {
                    -0.2
                };
            }
        }
        c
    };
    let chosen = best_path(&cands, |k, qi, pi| {
        let (q, qk, _) = cands[k - 1][qi];
        let (p, _, _) = cands[k][pi];
        step_cost(q, qk, p) + pair_cost(&spots[k - 1], q, &spots[k], p)
    });

    let mut pitch_of = vec![0i32; strokes.len()];
    let mut kind_of = vec![BassNote::Chord(Tone::Root); strokes.len()];
    for (k, &i) in real.iter().enumerate() {
        let (p, kind, _) = cands[k][chosen[k]];
        pitch_of[i] = p;
        kind_of[i] = kind;
    }
    // On Stay out a bass holds each chord's note.
    let gate = if req.relation == Relation::StayOut {
        0.95
    } else {
        f64::from(st.playing.gate).clamp(0.3, 0.95)
    };
    let mut events = Vec::new();
    let mut last_real: Option<usize> = None;
    for (i, s) in strokes.iter().enumerate() {
        let (pitch, role) = if s.ghost {
            let from = last_real.or_else(|| real.first().copied()).unwrap_or(i);
            (pitch_of[from], Role::Ghost)
        } else {
            last_real = Some(i);
            let role = match kind_of[i] {
                BassNote::Chord(t) => Role::Chord(t),
                BassNote::Approach => Role::Approach,
                BassNote::Passing => Role::Passing,
            };
            (pitch_of[i], role)
        };
        let steps_long = if s.ghost {
            1.0
        } else {
            f64::from(s.len) * gate
        };
        let len = ((steps_long * STEP as f64) as Tick - crate::profile::DETACH).max(STEP / 3);
        events.push(Event {
            at: row.tick(s.at),
            len,
            pitch: pitch.clamp(0, 127) as u8,
            velocity: velocity(req, s, -4),
            role,
            source: Source::Mpfree,
            timing: timing(req, Lane::Kick, s.at),
        });
    }
    events
}

/// The cheapest path through candidates (each with its own cost as the
/// third field of the tuple), with `step(k, from, to)` the cost of moving
/// from candidate `from` of spot `k - 1` to candidate `to` of spot `k`.
fn best_path<T>(
    cands: &[Vec<(i32, T, f64)>],
    step: impl Fn(usize, usize, usize) -> f64,
) -> Vec<usize> {
    best_path_cost(cands, step).0
}

/// [`best_path`], with what the path costs.
pub(crate) fn best_path_cost<T>(
    cands: &[Vec<(i32, T, f64)>],
    step: impl Fn(usize, usize, usize) -> f64,
) -> (Vec<usize>, f64) {
    let n = cands.len();
    if n == 0 {
        return (Vec::new(), 0.0);
    }
    let mut cost: Vec<Vec<f64>> = vec![cands[0].iter().map(|c| c.2).collect()];
    let mut back: Vec<Vec<usize>> = vec![vec![0; cands[0].len()]];
    for k in 1..n {
        let mut ck = Vec::with_capacity(cands[k].len());
        let mut bk = Vec::with_capacity(cands[k].len());
        for (pi, c) in cands[k].iter().enumerate() {
            let (bi, bc) = (0..cands[k - 1].len())
                .map(|qi| (qi, cost[k - 1][qi] + step(k, qi, pi)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .expect("a candidate");
            ck.push(bc + c.2);
            bk.push(bi);
        }
        cost.push(ck);
        back.push(bk);
    }
    let mut path = vec![0; n];
    path[n - 1] = (0..cands[n - 1].len())
        .min_by(|&a, &b| cost[n - 1][a].total_cmp(&cost[n - 1][b]))
        .expect("a candidate");
    let total = cost[n - 1][path[n - 1]];
    for k in (1..n).rev() {
        path[k - 1] = back[k][path[k]];
    }
    (path, total)
}

/// Close-position voicings of `c` with every note in `[lo, hi]`: all
/// inversions, and for a chord with a seventh, without its fifth and
/// without its root.
fn voicings(c: Chord, lo: u8, hi: u8) -> Vec<Vec<u8>> {
    let tones: Vec<u8> = c
        .quality
        .intervals()
        .iter()
        .map(|i| (c.root + i) % 12)
        .collect();
    let mut sets = vec![tones.clone()];
    if tones.len() == 4 {
        sets.push(vec![tones[0], tones[1], tones[3]]);
        sets.push(vec![tones[1], tones[2], tones[3]]);
    }
    let mut out = Vec::new();
    for set in &sets {
        for r in 0..set.len() {
            let order: Vec<u8> = (0..set.len()).map(|j| set[(r + j) % set.len()]).collect();
            for base in lo..=hi {
                if base % 12 != order[0] {
                    continue;
                }
                let mut v = vec![base];
                for &pc in &order[1..] {
                    let last = *v.last().expect("a note");
                    let up = (pc + 12 - last % 12) % 12;
                    v.push(last + if up == 0 { 12 } else { up });
                }
                if *v.last().expect("a note") <= hi {
                    out.push(v);
                }
            }
        }
    }
    out
}

fn movement(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len().min(b.len());
    let moved: i32 = (0..n)
        .map(|i| (i32::from(a[i]) - i32::from(b[i])).abs())
        .sum();
    f64::from(moved) + 2.0 * (a.len().max(b.len()) - n) as f64
}

/// Chords: one voicing per chord, chosen for the whole part with the
/// least movement, under the tune (its top note at least a 3rd below the
/// tune's lowest note there), above the bass, and with nothing a half
/// step from a note the tune holds. Each stroke plays it: held to the
/// next (pads), short (stabs) or a dotted 1/8 (comping).
fn chords(req: &AddPart, strokes: &[Stroke], dice: &mut Dice) -> Vec<Event> {
    let row = req.row;
    let st = &req.settings.style;
    let (vlo, vhi) = st.bass.voicing;
    let prof = req.kind.profile();
    let noise = 0.15 * (0.3 + f64::from(req.settings.variation));
    // Group the real strokes by the chord span they sound in.
    let mut groups: Vec<(Tick, Tick, Chord, Vec<usize>)> = Vec::new();
    for (i, s) in strokes.iter().enumerate() {
        if s.ghost {
            continue;
        }
        let t = row.tick(s.at);
        let Some(c) = chord_near(row, s.at) else {
            continue;
        };
        let (from, until) = row
            .chords
            .span_at(t)
            .map_or((t, t + Tick::from(s.len) * STEP), |sp| (sp.start, sp.end()));
        match groups.last_mut() {
            Some(gr) if gr.0 == from && gr.2 == c => gr.3.push(i),
            _ => groups.push((from, until, c, vec![i])),
        }
    }
    if groups.is_empty() {
        return Vec::new();
    }
    let jazz = st.bass.comping == Comping::Charleston;
    let cands: Vec<Vec<(i32, Vec<u8>, f64)>> = groups
        .iter()
        .map(|(from, until, c, idx)| {
            let first = row.tick(strokes[idx[0]].at);
            let lead_low = row
                .lead
                .as_ref()
                .and_then(|l| l.lowest(first.min(*from), *until));
            let lead_notes: Vec<u8> = row
                .lead
                .as_ref()
                .map(|l| {
                    l.sounding_in(first, *until)
                        .filter(|n| n.len >= 2 * STEP)
                        .map(|n| n.pitch)
                        .collect()
                })
                .unwrap_or_default();
            let bass_top = row.span_of(PartRole::Bass, *from, *until).map(|(_, h)| h);
            // Under the tune: the style's range, moved down below the
            // tune's lowest note when it reaches into it.
            let (lo, hi) = match lead_low {
                Some(l) => {
                    let hi = (i32::from(l) - 2)
                        .min(i32::from(vhi))
                        .max(i32::from(prof.low) + 10);
                    let lo = i32::from(vlo).min(hi - 14).max(i32::from(prof.low));
                    (lo as u8, hi as u8)
                }
                None => (vlo, vhi),
            };
            let centre = f64::from(u16::from(lo) + u16::from(hi)) / 2.0;
            let mut v: Vec<(i32, Vec<u8>, f64)> = voicings(*c, lo, hi)
                .into_iter()
                .map(|vc| {
                    let top = i32::from(*vc.last().expect("a note"));
                    let bottom = i32::from(vc[0]);
                    let mut cost = 0.0;
                    if let Some(l) = lead_low {
                        let over = top - (i32::from(l) - 3);
                        if over > 0 {
                            cost += f64::from(over);
                        }
                    }
                    for &n in &lead_notes {
                        cost += 1.5
                            * vc.iter()
                                .filter(|&&x| rubs(i32::from(x), i32::from(n)))
                                .count() as f64;
                    }
                    if let Some(b) = bass_top {
                        let under = i32::from(b) + 5 - bottom;
                        if under > 0 {
                            cost += 0.5 * f64::from(under);
                        }
                    }
                    let mean = vc.iter().map(|&x| f64::from(x)).sum::<f64>() / vc.len() as f64;
                    cost += 0.08 * (mean - centre).abs();
                    let rootless = !vc.iter().any(|&x| x % 12 == c.root);
                    if rootless {
                        cost += if jazz { -0.3 } else { 0.5 };
                    }
                    cost += dice.gumbel() * noise;
                    (0, vc, cost)
                })
                .collect();
            if v.is_empty() {
                let base = lo + (c.root + 12 - lo % 12) % 12;
                v.push((0, vec![base, base + 4, base + 7], 5.0));
            }
            v
        })
        .collect();
    let chosen = best_path(&cands, |k, qi, pi| {
        0.2 * movement(&cands[k - 1][qi].1, &cands[k][pi].1)
    });

    let mut events = Vec::new();
    for (gi, (_, until, _, idx)) in groups.iter().enumerate() {
        let vc = &cands[gi][chosen[gi]].1;
        for &i in idx {
            let s = &strokes[i];
            let at = row.tick(s.at);
            let to_next = Tick::from(s.len) * STEP;
            let len = match (req.relation, st.bass.comping) {
                (Relation::StayOut, _) | (_, Comping::Pad) => {
                    to_next.min(*until - at).max(STEP) - STEP / 4
                }
                (_, Comping::Stabs) => (2 * STEP).min(to_next) * 4 / 5,
                (_, Comping::Charleston) => (3 * STEP).min(to_next) - STEP / 4,
            };
            for &p in vc {
                events.push(Event {
                    at,
                    len: len.max(STEP / 3),
                    pitch: p,
                    velocity: velocity(req, s, -14),
                    role: Role::Chord(
                        chord_near(row, s.at)
                            .and_then(|c| c.tone_of(i32::from(p)))
                            .unwrap_or(Tone::Root),
                    ),
                    source: Source::Mpfree,
                    timing: timing(req, Lane::Hats, s.at),
                });
            }
        }
    }
    events
}

/// A second line, under the tune: chord notes on the strong beats, steps
/// between, 3rds and 6ths below the tune's notes, no unison or octave
/// with it on a strong beat, no fifths or octaves in parallel. On Answer
/// it echoes the tune's last notes in its gaps.
fn line(req: &AddPart, strokes: &[Stroke], dice: &mut Dice) -> Vec<Event> {
    let row = req.row;
    let g = row.grid;
    let st = &req.settings.style;
    let prof = req.kind.profile();
    let real: Vec<usize> = (0..strokes.len()).filter(|&i| !strokes[i].ghost).collect();
    if real.is_empty() {
        return Vec::new();
    }
    let median = row.lead.as_ref().map_or(67, |l| {
        let mut p: Vec<u8> = l.line.iter().map(|n| n.pitch).collect();
        p.sort_unstable();
        i32::from(p[p.len() / 2])
    });
    let (blo, bhi) = match req.relation {
        Relation::Answer => (median - 9, median + 2),
        Relation::StayOut => (median - 12, median - 4),
        _ => (median - 12, median - 3),
    };
    let blo = blo.max(i32::from(prof.low));
    let bhi = bhi.min(i32::from(prof.high)).max(blo + 7);
    let noise = 0.12 * (0.3 + f64::from(req.settings.variation));
    // The tune's notes before each of its rests, for Answer to echo.
    let echo_target = |s: i32| -> Option<i32> {
        let lead = row.lead.as_ref()?;
        let &(from, _) = row.plan.rests.iter().find(|&&(a, b)| a <= s && s < b)?;
        let before: Vec<i32> = lead
            .line
            .iter()
            .filter(|n| n.at < row.tick(from))
            .map(|n| i32::from(n.pitch))
            .collect();
        let tail = &before[before.len().saturating_sub(4)..];
        if tail.is_empty() {
            return None;
        }
        let j = ((s - from) / 2).max(0) as usize % tail.len();
        let p = tail[j] - 12;
        Some(if p < blo { p + 7 } else { p })
    };
    struct Spot {
        strong: bool,
        lead: Option<i32>,
        held: Vec<u8>,
        target: Option<i32>,
    }
    let spots: Vec<Spot> = real
        .iter()
        .map(|&i| {
            let s = strokes[i].at;
            let t = row.tick(s);
            Spot {
                strong: g.level(s) >= -2,
                lead: row.lead_at(s).map(|n| i32::from(n.pitch)),
                held: row.sounding(t).collect(),
                target: if req.relation == Relation::Answer {
                    echo_target(s)
                } else {
                    None
                },
            }
        })
        .collect();
    let cands: Vec<Vec<(i32, Role, f64)>> = real
        .iter()
        .zip(&spots)
        .map(|(&i, sp)| {
            let s = strokes[i].at;
            let chord = chord_near(row, s);
            let mut v = Vec::new();
            for p in blo..=bhi {
                let tone = chord.and_then(|c| c.tone_of(p));
                if tone.is_none() && !row.key.has(p) {
                    continue;
                }
                let mut c = match (tone.is_some(), sp.strong) {
                    (true, true) => 0.0,
                    (false, true) => 1.0,
                    (true, false) => 0.2,
                    (false, false) => 0.3,
                };
                if let Some(l) = sp.lead {
                    let below = l - p;
                    c += match below.rem_euclid(12) {
                        3 | 4 | 8 | 9 => {
                            if req.relation == Relation::Support {
                                -0.8
                            } else {
                                -0.4
                            }
                        }
                        0 if sp.strong => 2.5,
                        1 | 2 | 10 | 11 if sp.strong => 1.5,
                        _ => 0.0,
                    };
                    if below < 0 {
                        c += 1.5;
                    }
                }
                if sp.strong && sp.held.iter().any(|&h| rubs(p, i32::from(h))) {
                    c += 1.5;
                }
                if let Some(t) = sp.target {
                    c += 0.15 * f64::from((p - t).abs());
                }
                c += 0.03 * f64::from((p - (blo + bhi) / 2).abs());
                c += dice.gumbel() * noise;
                let role = match tone {
                    Some(t) => Role::Chord(t),
                    None => Role::Scale,
                };
                v.push((p, role, c));
            }
            if v.is_empty() {
                v.push((blo, Role::Scale, 5.0));
            }
            v
        })
        .collect();
    let chosen = best_path(&cands, |k, qi, pi| {
        let q = cands[k - 1][qi].0;
        let p = cands[k][pi].0;
        let d = (p - q).abs();
        let mut c = match d {
            0 => 0.4,
            1 | 2 => 0.0,
            3 | 4 => 0.2,
            5..=7 => 0.6,
            8 | 9 => 1.2,
            12 => 1.5,
            _ => 4.0,
        };
        let (a, b) = (&spots[k - 1], &spots[k]);
        if let (Some(la), Some(lb)) = (a.lead, b.lead) {
            if a.strong && b.strong && p != q && la != lb {
                if (la - q).rem_euclid(12) == 0 && (lb - p).rem_euclid(12) == 0 {
                    c += 1.5;
                }
                if (la - q).rem_euclid(12) == 7 && (lb - p).rem_euclid(12) == 7 {
                    c += 1.5;
                }
            }
        }
        c
    });
    let gate = f64::from(st.playing.gate).clamp(0.4, 0.95);
    real.iter()
        .enumerate()
        .map(|(k, &i)| {
            let s = &strokes[i];
            let (p, role, _) = cands[k][chosen[k]];
            let held = if req.relation == Relation::StayOut {
                1.0
            } else {
                gate
            };
            Event {
                at: row.tick(s.at),
                len: ((f64::from(s.len) * held * STEP as f64) as Tick).max(STEP / 3),
                pitch: p as u8,
                velocity: velocity(req, s, -10),
                role,
                source: Source::Mpfree,
                timing: timing(req, Lane::Hats, s.at),
            }
        })
        .collect()
}

/// One bar of the style's beat, by drum: (step, drum, velocity, a tom
/// or cymbal). `fill` ends the bar with a fill.
fn beat_map(
    map: DrumMap,
    bar_steps: i32,
    beats: i32,
    bar: usize,
    fill: bool,
) -> Vec<(i32, Lane, u8)> {
    let mut h: Vec<(i32, Lane, u8)> = Vec::new();
    match (map, bar_steps) {
        (DrumMap::FourOnTheFloor, 16) => {
            for (k, v) in [(0, 112), (4, 100), (8, 108), (12, 100)] {
                h.push((k, Lane::Kick, v));
            }
            h.push((4, Lane::Snare, 100));
            h.push((12, Lane::Snare, 104));
            for st in 0..16 {
                if fill && st >= 12 {
                    continue;
                }
                if st % 4 == 2 {
                    h.push((st, Lane::Hats, if st == 14 { 86 } else { 76 }));
                } else if st % 2 == 1 {
                    h.push((st, Lane::Hats, if st % 4 == 3 { 52 } else { 40 }));
                }
            }
            if fill {
                for (i, st) in (12..16).enumerate() {
                    h.push((st, Lane::Snare, 58 + 14 * i as u8));
                }
            }
        }
        (DrumMap::BoomBap, 16) => {
            let a = bar.is_multiple_of(2);
            let kicks: &[(i32, u8)] = if a {
                &[(0, 112), (6, 92), (10, 102)]
            } else {
                &[(0, 112), (3, 78), (8, 104), (14, 86)]
            };
            for &(k, v) in kicks {
                if !(fill && k >= 12) {
                    h.push((k, Lane::Kick, v));
                }
            }
            h.push((4, Lane::Snare, 110));
            for st in 0..16 {
                if fill && st >= 12 {
                    continue;
                }
                let v = match st % 4 {
                    0 => 88,
                    2 => 70,
                    _ => 44,
                };
                h.push((st, Lane::Hats, v));
            }
            if fill {
                h.push((12, Lane::Snare, 112));
                h.push((13, Lane::Other, 84));
                h.push((14, Lane::Other, 92));
                h.push((15, Lane::Other, 100));
            } else {
                h.push((12, Lane::Snare, 108));
            }
        }
        (DrumMap::Ride, 16) => {
            for (st, v) in [(0, 86), (4, 100), (6, 66), (8, 84), (12, 100), (14, 66)] {
                h.push((st, Lane::Hats, v));
            }
            for st in [0, 4, 8, 12] {
                h.push((st, Lane::Kick, 34));
            }
            if fill {
                h.push((10, Lane::Snare, 78));
                h.push((14, Lane::Snare, 88));
                h.push((14, Lane::Kick, 80));
            }
        }
        (_, 12) => {
            h.push((0, Lane::Kick, 100));
            h.push((4, Lane::Snare, 58));
            h.push((8, Lane::Snare, 62));
            for st in (0..12).step_by(2) {
                if fill && st >= 8 {
                    continue;
                }
                h.push((st, Lane::Hats, if st % 4 == 0 { 54 } else { 38 }));
            }
            if fill {
                h.push((8, Lane::Snare, 66));
                h.push((10, Lane::Snare, 74));
                h.push((11, Lane::Snare, 84));
            }
        }
        _ => {
            let beat = (bar_steps / beats.max(1)).max(1);
            h.push((0, Lane::Kick, 110));
            for b in 1..beats {
                h.push((b * beat, Lane::Snare, 90));
            }
            for st in (0..bar_steps).step_by(2) {
                h.push((st, Lane::Hats, if st % beat == 0 { 80 } else { 56 }));
            }
        }
    }
    h
}

/// Drums: the style's beat, a fill in the bar before the high point and
/// at the end, then shaped by the role: Support adds kicks under the
/// tune's pushed long notes, Answer plays the plainest beat under the
/// tune and fills its gaps, Interlock adds percussion between the kick,
/// the snare and the tune, Stay out keeps only kick, backbeat and hats.
/// Only drums the kit names are played.
fn drums(req: &AddPart, dice: &mut Dice) -> Vec<Event> {
    let row = req.row;
    let g = row.grid;
    let st = &req.settings.style;
    let beats = (g.bar / g.beat).max(1);
    let peak_bar = row.plan.peak().map(|m| (m.at / g.bar) as usize);
    let mut hits: Vec<(i32, Lane, u8)> = Vec::new();
    for b in 0..row.bars {
        let last = b + 1 == row.bars;
        let fill = req.relation != Relation::StayOut
            && ((last && row.bars >= 2) || peak_bar.is_some_and(|p| p > 0 && b + 1 == p));
        let o = b as i32 * g.bar;
        let mut bar_hits = beat_map(st.bass.drums, g.bar, beats, b, fill);
        match req.relation {
            Relation::StayOut => {
                bar_hits.retain(|&(s, lane, _)| match lane {
                    Lane::Kick => s == 0 || s == g.bar / 2,
                    Lane::Snare => g.on_beat(s),
                    Lane::Hats => g.on_beat(s),
                    Lane::Other => false,
                });
            }
            Relation::Answer => {
                bar_hits.retain(|&(s, lane, _)| {
                    let at = o + s;
                    tune_gap(row, at)
                        || matches!(lane, Lane::Hats)
                        || (lane == Lane::Kick && s == 0)
                        || (lane == Lane::Snare && g.on_beat(s))
                });
                // A fill in each gap of a beat or more, and a short one in
                // the tail of each held note the tune lands on.
                for &(a, z) in &row.plan.rests {
                    if a >= o && a < o + g.bar && z - a >= g.beat {
                        let from = (z - g.beat).max(a);
                        for (i, s) in (from..z).enumerate() {
                            bar_hits.push((
                                s - o,
                                if i % 2 == 0 { Lane::Snare } else { Lane::Other },
                                64 + 10 * i as u8,
                            ));
                        }
                    }
                }
                for m in row.plan.marks_in(o, o + g.bar) {
                    let tail: Vec<i32> = (m.at + 1..(m.at + g.beat).min(o + g.bar))
                        .filter(|&s| tune_gap(row, s))
                        .collect();
                    for (i, &s) in tail.iter().enumerate() {
                        bar_hits.push((
                            s - o,
                            if i % 2 == 0 { Lane::Other } else { Lane::Snare },
                            60 + 12 * i as u8,
                        ));
                    }
                }
            }
            Relation::Interlock => {
                for s in 0..g.bar {
                    let at = o + s;
                    let free = row.drum(Lane::Kick, at) == 0
                        && row.drum(Lane::Snare, at) == 0
                        && !bar_hits.iter().any(|&(x, l, _)| x == s && l != Lane::Hats)
                        && !row.lead_starts(at)
                        && g.level(s) <= -3;
                    if free && dice.chance(0.3) {
                        bar_hits.push((s, Lane::Other, 50 + dice.below(20) as u8));
                    }
                }
            }
            Relation::Support => {
                for m in row.plan.marks_in(o, o + g.bar) {
                    if !bar_hits
                        .iter()
                        .any(|&(x, ln, _)| x == m.at - o && ln == Lane::Kick)
                    {
                        bar_hits.push((m.at - o, Lane::Kick, 96));
                    }
                }
                if let Some(l) = &row.lead {
                    for n in &l.line {
                        let s = (n.at / STEP) as i32;
                        if s / g.bar == b as i32
                            && g.level(s) == -3
                            && n.len >= 2 * STEP
                            && !bar_hits
                                .iter()
                                .any(|&(x, ln, _)| x == s - o && ln == Lane::Kick)
                        {
                            bar_hits.push((s - o, Lane::Kick, 88));
                        }
                    }
                }
            }
        }
        if !row.plan.from_tune {
            bar_hits.retain(|&(s, lane, _)| lane == Lane::Hats || !row.plan.resting(o + s));
        }
        // Over drums already in the row, a drum part adds and never
        // doubles a hit the row plays.
        bar_hits.retain(|&(s, lane, _)| row.drum(lane, o + s) == 0);
        hits.extend(bar_hits.into_iter().map(|(s, l, v)| (o + s, l, v)));
    }
    let lane_index = |l: Lane| match l {
        Lane::Kick => 0,
        Lane::Snare => 1,
        Lane::Hats => 2,
        Lane::Other => 3,
    };
    let mut events: Vec<Event> = hits
        .into_iter()
        .filter_map(|(s, lane, v)| {
            let pad = req.pads[lane_index(lane)]?;
            Some(Event {
                at: row.tick(s),
                len: HIT,
                pitch: pad,
                velocity: v,
                role: Role::Drum(lane),
                source: Source::Mpfree,
                timing: timing(req, lane, s),
            })
        })
        .collect();
    events.sort_by_key(|e| (e.at, e.pitch));
    events.dedup_by_key(|e| (e.at, e.pitch));
    events
}

// ----------------------------------------------------------------- judge

/// A part's checks, heard with the row: `events` are its notes with the
/// rhythm `strokes`.
pub(crate) fn judge(req: &AddPart, strokes: &[Stroke], events: &[Event]) -> PartVerdict {
    if req.part == PartRole::Drums {
        return judge_drums(req, events);
    }
    let row = req.row;
    let g = row.grid;
    let st = &req.settings.style;
    let real: Vec<&Stroke> = strokes.iter().filter(|s| !s.ghost).collect();
    let n = real.len().max(1) as f32;
    let share = |f: &dyn Fn(&Stroke) -> bool| real.iter().filter(|s| f(s)).count() as f32 / n;
    let unit = unit_of(st);

    let together = match req.relation {
        _ if real.is_empty() => Check::new(false, 0.0, "plays nothing".to_string()),
        Relation::Support => {
            let s = share(&|s| {
                let at = if s.pushed { s.at + unit } else { s.at };
                let beat = if row.has_drums() {
                    row.drum(Lane::Kick, at) > 0
                } else {
                    g.on_beat(at)
                };
                beat || row.chord_starts(at)
                    || row.plan.mark_at(at).is_some()
                    || (req.part != PartRole::Bass && row.lead_starts(at))
            });
            Check::new(
                s >= 0.6,
                s,
                format!("{:.0} % of its notes with the beat or the tune", s * 100.0),
            )
        }
        Relation::Answer => {
            let s = share(&|s| tune_gap(row, s.at));
            Check::new(
                s >= 0.6,
                s,
                format!("{:.0} % of its notes in the tune's gaps", s * 100.0),
            )
        }
        Relation::Interlock if walks(req) => {
            let beats = (0..row.steps).filter(|&s| g.on_beat(s)).count().max(1) as f32;
            let s = real.iter().filter(|s| g.on_beat(s.at)).count() as f32 / beats;
            Check::new(
                s >= 0.75,
                s,
                format!("walks: a note on {:.0} % of the beats", s * 100.0),
            )
        }
        Relation::Interlock => {
            // Between the others: a bass between the kicks and off the
            // snare, chords off the beat, a line between the tune's notes.
            let s = share(&|s| match req.part {
                PartRole::Bass => {
                    row.drum(Lane::Kick, s.at) == 0 && row.drum(Lane::Snare, s.at) < 60
                }
                PartRole::Chords => !g.on_beat(s.at),
                _ => !row.lead_starts(s.at),
            });
            Check::new(
                s >= 0.55,
                s,
                format!("{:.0} % of its notes between the others", s * 100.0),
            )
        }
        Relation::StayOut => {
            let chords = (0..row.steps)
                .filter(|&s| row.chord_starts(s))
                .count()
                .max(1) as f32;
            let per = real.len() as f32 / chords;
            Check::new(
                per <= 1.5,
                1.5 - per / 2.0,
                format!("{per:.1} notes per chord"),
            )
        }
    };

    // Clashes on strong beats, and parallels with the tune.
    let lead_part = row.lead.as_ref().map(|l| l.part);
    let mut clashes = 0;
    for e in events {
        let s = (e.at / STEP) as i32;
        if g.level(s) < -2 || matches!(e.role, Role::Approach | Role::Ghost | Role::Passing) {
            continue;
        }
        if row
            .sounding(e.at)
            .any(|h| rubs(i32::from(e.pitch), i32::from(h)))
        {
            clashes += 1;
        }
    }
    let mut parallels = 0;
    if req.part != PartRole::Chords && lead_part.is_some() {
        let strong: Vec<&Event> = events
            .iter()
            .filter(|e| e.role != Role::Ghost && g.level((e.at / STEP) as i32) >= -2)
            .collect();
        for w in strong.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (Some(la), Some(lb)) = (
                row.lead_at((a.at / STEP) as i32),
                row.lead_at((b.at / STEP) as i32),
            ) else {
                continue;
            };
            if la.pitch == lb.pitch || a.pitch == b.pitch {
                continue;
            }
            let ia = (i32::from(la.pitch) - i32::from(a.pitch)).rem_euclid(12);
            let ib = (i32::from(lb.pitch) - i32::from(b.pitch)).rem_euclid(12);
            if ia == ib && (ia == 0 || ia == 7) {
                parallels += 1;
            }
        }
    }
    let allowed = (events.len() / 24).max(1);
    let clash = Check::new(
        clashes <= allowed && parallels <= 1,
        1.0 - (clashes + parallels) as f32 / (allowed as f32 * 4.0),
        format!("{clashes} half-step clashes on strong beats, {parallels} parallels with the tune"),
    );

    // Room.
    let prof = req.kind.profile();
    let mut out = events
        .iter()
        .filter(|e| !prof.in_range(i32::from(e.pitch)))
        .count();
    if let Some(l) = &row.lead {
        if req.part != PartRole::Bass {
            out += events
                .iter()
                .filter(|e| {
                    let low = l.lowest(e.at, e.at + e.len.max(1));
                    low.is_some_and(|lo| e.pitch >= lo && req.relation != Relation::Answer)
                })
                .count();
        }
    }
    let allowed_out = (events.len() / 20).max(1);
    let room = Check::new(
        out <= allowed_out,
        1.0 - out as f32 / (allowed_out as f32 * 4.0),
        format!("{out} notes out of its place"),
    );

    // Lands: a note on each landing (or held through it, for chords and
    // on Stay out) that belongs to the chord there.
    let sustained = req.relation == Relation::StayOut || req.part == PartRole::Chords;
    let lands = match req.relation {
        Relation::Support | Relation::StayOut if !row.plan.marks.is_empty() => {
            let hit = row
                .plan
                .marks
                .iter()
                .filter(|m| {
                    events.iter().any(|e| {
                        let s = (e.at / STEP) as i32;
                        (s == m.at
                            || s == m.at - unit
                            || (sustained && s <= m.at && m.at < s + (e.len / STEP) as i32 + 1))
                            && chord_near(row, m.at).is_some_and(|c| c.has(i32::from(e.pitch)))
                    })
                })
                .count();
            let s = hit as f32 / row.plan.marks.len() as f32;
            Check::new(
                s >= 0.6,
                s,
                format!("on {hit} of {} landings", row.plan.marks.len()),
            )
        }
        _ => {
            let at_changes: Vec<&Event> = events
                .iter()
                .filter(|e| row.chord_starts((e.at / STEP) as i32))
                .collect();
            let good = at_changes
                .iter()
                .filter(|e| {
                    chord_near(row, (e.at / STEP) as i32).is_some_and(|c| c.has(i32::from(e.pitch)))
                })
                .count();
            let s = if at_changes.is_empty() {
                1.0
            } else {
                good as f32 / at_changes.len() as f32
            };
            Check::new(
                s >= 0.75,
                s,
                format!(
                    "{good} of {} chord changes on chord notes",
                    at_changes.len()
                ),
            )
        }
    };

    // Returns, not stuck: bars by their rhythm and notes.
    let prints: Vec<Vec<(i32, u8)>> = (0..row.bars)
        .map(|b| {
            events
                .iter()
                .filter(|e| (e.at / STEP) as i32 / g.bar == b as i32)
                .map(|e| ((e.at / STEP) as i32 % g.bar, e.pitch))
                .collect()
        })
        .collect();
    let rhythms: Vec<Vec<i32>> = prints
        .iter()
        .map(|p| p.iter().map(|x| x.0).collect())
        .collect();
    let mut run = 1;
    let mut longest = 1;
    for b in 1..prints.len() {
        if prints[b] == prints[b - 1] {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 1;
        }
    }
    let returns = row.bars < 2
        || req.relation == Relation::Answer
        || req.relation == Relation::StayOut
        || (1..rhythms.len()).any(|b| rhythms[..b].contains(&rhythms[b]));
    let stuck = row.bars >= 5 && longest > 4;
    let repeats = Check::new(
        returns && !stuck,
        if stuck {
            0.2
        } else if returns {
            1.0
        } else {
            0.5
        },
        format!(
            "{}; at most {longest} bars alike in a row",
            if returns {
                "the riff returns"
            } else {
                "no bar returns"
            }
        ),
    );

    // Flows.
    let flows = if req.part == PartRole::Chords {
        let mut moves = Vec::new();
        let mut prev: Option<Vec<u8>> = None;
        let mut i = 0;
        while i < events.len() {
            let at = events[i].at;
            let mut v: Vec<u8> = events[i..]
                .iter()
                .take_while(|e| e.at == at)
                .map(|e| e.pitch)
                .collect();
            i += v.len();
            v.sort_unstable();
            if let Some(p) = &prev {
                if p != &v {
                    moves.push(movement(p, &v) / v.len().max(1) as f64);
                }
            }
            prev = Some(v);
        }
        let mean = moves.iter().sum::<f64>() / moves.len().max(1) as f64;
        Check::new(
            mean <= 4.0,
            (1.0 - mean / 8.0) as f32,
            format!("voices move {mean:.1} semitones a change"),
        )
    } else {
        let limit = if req.part == PartRole::Bass { 12 } else { 9 };
        let notes: Vec<&Event> = events.iter().filter(|e| e.role != Role::Ghost).collect();
        let wide = notes
            .windows(2)
            .filter(|w| (i32::from(w[0].pitch) - i32::from(w[1].pitch)).abs() > limit)
            .count();
        Check::new(
            wide == 0,
            1.0 - wide as f32 / 4.0,
            format!("{wide} leaps wider than {limit} semitones"),
        )
    };
    PartVerdict {
        together,
        clash,
        room,
        lands,
        repeats,
        flows,
    }
}

/// The drum part's checks, heard with the drums already in the row: the
/// role (Support: the kick under the tune's accents and a backbeat;
/// Answer: fills in the tune's gaps; Interlock: percussion between the
/// others; Stay out: the plainest beat), no doubled hits, only named
/// pads, fills where the plan asks, and the time kept.
fn judge_drums(req: &AddPart, events: &[Event]) -> PartVerdict {
    let row = req.row;
    let g = row.grid;
    let lane_of = |e: &Event| match e.role {
        Role::Drum(l) => Some(l),
        _ => None,
    };
    let at = |e: &Event| (e.at / STEP) as i32;
    // The part and the row together.
    let hit = |lane: Lane, s: i32| {
        row.drum(lane, s) > 0
            || events
                .iter()
                .any(|e| lane_of(e) == Some(lane) && at(e) == s)
    };
    let loud_hat = |s: i32| {
        row.drum(Lane::Hats, s) >= 90
            || events
                .iter()
                .any(|e| lane_of(e) == Some(Lane::Hats) && at(e) == s && e.velocity >= 90)
    };
    let bars = row.bars.max(1);
    // A backbeat: the snare (or the ride's accent) on the beats after one.
    let backbeat_bars = (0..bars as i32)
        .filter(|&b| {
            let o = b * g.bar;
            (1..(g.bar / g.beat).max(1)).any(|k| {
                let s = o + k * g.beat;
                hit(Lane::Snare, s) || loud_hat(s)
            })
        })
        .count();
    // Fill bars: two or more snare or percussion hits in the bar's last beat.
    let is_fill = |b: usize| {
        let o = b as i32 * g.bar;
        events
            .iter()
            .filter(|e| {
                matches!(lane_of(e), Some(Lane::Snare | Lane::Other))
                    && at(e) >= o + g.bar - g.beat
                    && at(e) < o + g.bar
            })
            .count()
            >= 2
    };
    let fills: Vec<usize> = (0..bars).filter(|&b| is_fill(b)).collect();
    let peak_bar = row.plan.peak().map(|m| (m.at / g.bar) as usize);
    let wanted: Vec<usize> = (0..bars)
        .filter(|&b| (b + 1 == bars && bars >= 2) || peak_bar.is_some_and(|p| p > 0 && b + 1 == p))
        .collect();

    let together = match req.relation {
        Relation::Support => {
            let accents: Vec<i32> = match &row.lead {
                Some(l) => {
                    let mut v: Vec<i32> = row.plan.marks.iter().map(|m| m.at).collect();
                    v.extend(
                        l.line
                            .iter()
                            .filter(|n| n.len >= 2 * STEP && g.level((n.at / STEP) as i32) >= -2)
                            .map(|n| (n.at / STEP) as i32),
                    );
                    v.sort_unstable();
                    v.dedup();
                    v
                }
                None => (0..bars as i32).map(|b| b * g.bar).collect(),
            };
            let under = accents.iter().filter(|&&s| hit(Lane::Kick, s)).count() as f32
                / accents.len().max(1) as f32;
            let back = backbeat_bars as f32 / bars as f32;
            let sc = 0.5 * under + 0.5 * back;
            Check::new(
                under >= 0.5 && back >= 0.75,
                sc,
                format!(
                    "a kick under {:.0} % of {}, a backbeat in {backbeat_bars} of {bars} bars",
                    under * 100.0,
                    if row.lead.is_some() {
                        "the tune's accents"
                    } else {
                        "the bars' first beats"
                    }
                ),
            )
        }
        Relation::Answer => {
            let extra: Vec<&Event> = events
                .iter()
                .filter(|e| {
                    matches!(lane_of(e), Some(Lane::Other))
                        || (lane_of(e) == Some(Lane::Snare) && !g.on_beat(at(e)))
                })
                .collect();
            let in_gaps = extra.iter().filter(|e| tune_gap(row, at(e))).count();
            let sc = in_gaps as f32 / extra.len().max(1) as f32;
            Check::new(
                !extra.is_empty() && sc >= 0.6,
                if extra.is_empty() { 0.0 } else { sc },
                format!("{in_gaps} of {} fill hits in the tune's gaps", extra.len()),
            )
        }
        Relation::Interlock => {
            let perc: Vec<&Event> = events
                .iter()
                .filter(|e| lane_of(e) == Some(Lane::Other))
                .collect();
            let between = perc
                .iter()
                .filter(|e| {
                    row.drum(Lane::Kick, at(e)) == 0
                        && row.drum(Lane::Snare, at(e)) == 0
                        && !row.lead_starts(at(e))
                })
                .count();
            let sc = between as f32 / perc.len().max(1) as f32;
            Check::new(
                perc.len() >= bars && sc >= 0.7,
                if perc.is_empty() { 0.0 } else { sc },
                format!(
                    "{between} of {} percussion hits between the kick, the snare and the tune",
                    perc.len()
                ),
            )
        }
        Relation::StayOut => {
            let perc = events
                .iter()
                .filter(|e| lane_of(e) == Some(Lane::Other))
                .count();
            let kicks = events
                .iter()
                .filter(|e| lane_of(e) == Some(Lane::Kick))
                .count() as f32
                / bars as f32;
            Check::new(
                perc == 0 && fills.is_empty() && kicks <= 2.0,
                if perc == 0 && fills.is_empty() {
                    1.0 - kicks / 8.0
                } else {
                    0.0
                },
                format!(
                    "{kicks:.1} kicks a bar, {} fills, {perc} percussion hits",
                    fills.len()
                ),
            )
        }
    };
    let doubled = events
        .iter()
        .filter(|e| lane_of(e).is_some_and(|l| row.drum(l, at(e)) > 0))
        .count();
    let clash = Check::new(
        doubled == 0,
        1.0 - doubled as f32 / events.len().max(1) as f32,
        format!("{doubled} hits doubling the row's drums"),
    );
    let pads: Vec<u8> = req.pads.iter().flatten().copied().collect();
    let named = events.iter().all(|e| pads.contains(&e.pitch));
    let room = Check::new(
        named,
        if named { 1.0 } else { 0.0 },
        "only pads the kit names".into(),
    );
    let lands = if req.relation == Relation::StayOut {
        Check::new(fills.is_empty(), 1.0, format!("{} fills", fills.len()))
    } else {
        let got = wanted.iter().filter(|b| fills.contains(b)).count();
        let extra = fills.iter().filter(|b| !wanted.contains(b)).count();
        Check::new(
            got == wanted.len() && extra * 2 <= bars,
            got as f32 / wanted.len().max(1) as f32,
            format!(
                "fills in {got} of {} bars the plan asks for, {extra} more",
                wanted.len()
            ),
        )
    };
    let prints: Vec<Vec<(i32, u8)>> = (0..bars)
        .map(|b| {
            events
                .iter()
                .filter(|e| at(e) / g.bar == b as i32)
                .map(|e| (at(e) % g.bar, e.pitch))
                .collect()
        })
        .collect();
    let all_same = bars >= 4 && prints.windows(2).all(|w| w[0] == w[1]);
    let repeats = Check::new(
        !all_same || req.relation == Relation::StayOut,
        if all_same { 0.6 } else { 1.0 },
        if all_same {
            "the same bar throughout".into()
        } else {
            "a beat that returns, with fills".into()
        },
    );
    let time_bars = (0..bars as i32)
        .filter(|&b| {
            (0..g.bar)
                .filter(|&s| hit(Lane::Hats, b * g.bar + s) || hit(Lane::Kick, b * g.bar + s))
                .count()
                >= 4
        })
        .count();
    let flows = Check::new(
        time_bars * 4 >= bars * 3,
        time_bars as f32 / bars as f32,
        format!("keeps time in {time_bars} of {bars} bars"),
    );
    PartVerdict {
        together,
        clash,
        room,
        lands,
        repeats,
        flows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realize::PlainNote;
    use crate::row::tests::{beat, c_minor, chords_cm_fm, riff_row};
    use crate::row::{Heard, RowPart, ToHear};
    use crate::settings::Settings;
    use crate::style::{GROOVE, JAZZISH, MINIMAL};
    use mpfree_context::chord::{ChordTimeline, Heard as ChordHeard, Quality};
    use mpfree_transport::time::Meter;

    fn settings(style: Style) -> Settings {
        Settings {
            style,
            seed: 3,
            ..Settings::default()
        }
    }

    fn ask<'a>(row: &'a Row, part: PartRole, relation: Relation, s: &'a Settings) -> AddPart<'a> {
        AddPart {
            row,
            part,
            relation,
            settings: s,
            kind: match part {
                PartRole::Bass => InstrumentKind::Bass,
                PartRole::Chords => InstrumentKind::Chords,
                PartRole::Drums => InstrumentKind::Percussive,
                PartRole::Melody => InstrumentKind::OneNote,
            },
            pads: [Some(36), Some(38), Some(42), Some(45)],
            swing: Swing::default(),
        }
    }

    fn step(e: &Event) -> i32 {
        (e.at / STEP) as i32
    }

    /// A tune that rests: a held G and an Eb in bar 1, a long F in bar 2,
    /// bar 1 again, and a C to close.
    fn sparse_tune() -> Vec<PlainNote> {
        let n = |at: i64, len: i64, pitch: u8| PlainNote {
            at: at * STEP,
            len: len * STEP,
            pitch,
            velocity: 96,
        };
        vec![
            n(0, 4, 67),
            n(4, 2, 63),
            n(16, 6, 65),
            n(32, 4, 67),
            n(36, 2, 63),
            n(40, 2, 62),
            n(48, 8, 60),
        ]
    }

    fn long_riff() -> Vec<PlainNote> {
        let riff = crate::motif::tests::riff();
        let bar2 = 2 * Meter::default().bar_ticks();
        riff.iter()
            .copied()
            .chain(riff.iter().map(|n| PlainNote {
                at: n.at + bar2,
                ..*n
            }))
            .collect()
    }

    /// Four bars of Cm7 and Fm7, with a tune, a beat, or both.
    fn four_bars(tune: Option<Vec<PlainNote>>, with_beat: bool) -> Row {
        let chords = chords_cm_fm(4);
        let key = c_minor();
        let mut parts = Vec::new();
        if let Some(t) = tune {
            parts.push(RowPart {
                role: PartRole::Melody,
                heard: Heard::Notes(t),
            });
        }
        if with_beat {
            parts.push(beat(4));
        }
        Row::hear(&ToHear {
            parts: &parts,
            length: 4 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key: &key,
            played_pcs: 0,
            chords: &chords,
            groove: None,
            lead: None,
        })
    }

    /// The riff over four bars, with the beat.
    fn long_riff_row() -> (Row, ChordTimeline) {
        let chords = chords_cm_fm(4);
        let key = c_minor();
        let riff = crate::motif::tests::riff();
        let bar2 = 2 * Meter::default().bar_ticks();
        let notes: Vec<PlainNote> = riff
            .iter()
            .copied()
            .chain(riff.iter().map(|n| PlainNote {
                at: n.at + bar2,
                ..*n
            }))
            .collect();
        let parts = [
            RowPart {
                role: PartRole::Melody,
                heard: Heard::Notes(notes),
            },
            beat(4),
        ];
        let row = Row::hear(&ToHear {
            parts: &parts,
            length: 4 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key: &key,
            played_pcs: 0,
            chords: &chords,
            groove: None,
            lead: None,
        });
        (row, chords)
    }

    #[test]
    fn a_support_bass_plays_with_the_kick_and_lands_under_the_tune() {
        let (row, _) = long_riff_row();
        let s = settings(GROOVE);
        let a = add_part(&ask(&row, PartRole::Bass, Relation::Support, &s)).expect("a bass");
        assert!(a.verdict.passed() >= 5, "{}", a.verdict.words());
        // Most notes with the kick, a chord change or a landing (or a 1/16
        // before, pushed), and most kicks under a note.
        let real: Vec<&Event> = a.events.iter().filter(|e| e.role != Role::Ghost).collect();
        let near = |e: &Event, s: i32| step(e) == s || step(e) + 1 == s;
        let with = real
            .iter()
            .filter(|e| {
                (0..2).any(|d| {
                    let s = step(e) + d;
                    row.drum(Lane::Kick, s) > 0
                        || row.chord_starts(s)
                        || row.plan.mark_at(s).is_some()
                })
            })
            .count();
        assert!(with * 5 >= real.len() * 3, "{with} of {}", real.len());
        let kicks: Vec<i32> = (0..row.steps)
            .filter(|&s| row.drum(Lane::Kick, s) > 0)
            .collect();
        let covered = kicks
            .iter()
            .filter(|&&k| real.iter().any(|e| near(e, k)))
            .count();
        assert!(
            covered * 2 >= kicks.len(),
            "{covered} of {} kicks",
            kicks.len()
        );
        for e in &a.events {
            assert!((28..=55).contains(&e.pitch), "{e:?}");
        }
        // Each chord's first note is its root or fifth: F under bar 2's Fm7.
        let first_f = a
            .events
            .iter()
            .find(|e| (15..=16).contains(&step(e)) && e.role != Role::Ghost)
            .expect("a note into bar 2");
        assert!(matches!(first_f.pitch % 12, 5 | 0), "{first_f:?}");
        // On the riff's landing (the held C of bar 1), a chord note.
        assert!(
            a.events.iter().any(|e| (11..=12).contains(&step(e))
                && Chord::new(0, Quality::Minor7).has(i32::from(e.pitch))),
            "{:?}",
            a.events
                .iter()
                .map(|e| (step(e), e.pitch))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_same_request_gives_the_same_part_and_try_again_another() {
        let (row, _) = long_riff_row();
        let s = settings(GROOVE);
        let req = ask(&row, PartRole::Bass, Relation::Interlock, &s);
        let a = add_part(&req).expect("a bass");
        assert_eq!(a, add_part(&req).expect("a bass"));
        let b = add_part_again(&req, 1).expect("another");
        assert_ne!(a.events, b.events);
    }

    #[test]
    fn answer_chords_play_in_the_tunes_gaps() {
        let chords = chords_cm_fm(2);
        let key = c_minor();
        // A call of two beats, then two beats of rest, in each bar.
        let tune: Vec<PlainNote> = [
            (0, 2, 67),
            (2, 2, 70),
            (4, 4, 72),
            (16, 2, 68),
            (18, 2, 67),
            (20, 4, 65),
        ]
        .iter()
        .map(|&(s, l, p)| PlainNote {
            at: s * STEP,
            len: l * STEP,
            pitch: p,
            velocity: 90,
        })
        .collect();
        let parts = [
            RowPart {
                role: PartRole::Melody,
                heard: Heard::Notes(tune),
            },
            beat(2),
        ];
        let row = Row::hear(&ToHear {
            parts: &parts,
            length: 2 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key: &key,
            played_pcs: 0,
            chords: &chords,
            groove: None,
            lead: None,
        });
        let s = settings(GROOVE);
        let a = add_part(&ask(&row, PartRole::Chords, Relation::Answer, &s)).expect("chords");
        assert!(a.verdict.together.pass, "{}", a.verdict.words());
        assert!(!a.events.is_empty());
        // Under the tune: every chord note below its lowest note there.
        for e in &a.events {
            assert!(e.pitch < 72, "{e:?}");
        }
    }

    #[test]
    fn stay_out_plays_about_one_note_per_chord() {
        let (key, chords) = (c_minor(), chords_cm_fm(2));
        let row = riff_row(&chords, &key, true);
        let s = settings(MINIMAL);
        for part in [PartRole::Bass, PartRole::Chords, PartRole::Melody] {
            let a = add_part(&ask(&row, part, Relation::StayOut, &s)).expect("a part");
            let starts: std::collections::BTreeSet<i64> = a.events.iter().map(|e| e.at).collect();
            assert!(starts.len() <= 3, "{part:?}: {starts:?}");
            assert!(a.verdict.together.pass, "{part:?}: {}", a.verdict.words());
        }
    }

    #[test]
    fn a_chord_change_refits_the_same_rhythm() {
        let (row, _) = long_riff_row();
        let s = settings(GROOVE);
        let req = ask(&row, PartRole::Bass, Relation::Support, &s);
        let a = add_part(&req).expect("a bass");
        // Bar 2's Fm7 becomes Abmaj7.
        let bar = Meter::default().bar_ticks();
        let new_chords = ChordTimeline::of(
            &[
                (Chord::new(0, Quality::Minor7), bar),
                (Chord::new(8, Quality::Major7), bar),
                (Chord::new(0, Quality::Minor7), bar),
                (Chord::new(5, Quality::Minor7), bar),
            ],
            ChordHeard::Heard,
        );
        let mut row2 = row.clone();
        row2.chords = new_chords;
        let req2 = AddPart { row: &row2, ..req };
        let b = refit(&req2, &a);
        let times = |x: &Added| x.events.iter().map(|e| e.at).collect::<Vec<_>>();
        assert_eq!(times(&a), times(&b), "the rhythm stays");
        let into_bar2 = b
            .events
            .iter()
            .find(|e| (15..=16).contains(&step(e)) && e.role != Role::Ghost)
            .expect("bar 2's first note");
        assert!(
            Chord::new(8, Quality::Major7).has(i32::from(into_bar2.pitch)),
            "{into_bar2:?}"
        );
    }

    #[test]
    fn drums_play_only_the_pads_the_kit_names_with_fills_where_the_plan_asks() {
        let chords = chords_cm_fm(4);
        let key = c_minor();
        let riff = crate::motif::tests::riff();
        let parts = [RowPart {
            role: PartRole::Melody,
            heard: Heard::Notes(riff),
        }];
        let row = Row::hear(&ToHear {
            parts: &parts,
            length: 4 * Meter::default().bar_ticks(),
            meter: Meter::default(),
            key: &key,
            played_pcs: 0,
            chords: &chords,
            groove: None,
            lead: None,
        });
        let s = settings(GROOVE);
        let mut req = ask(&row, PartRole::Drums, Relation::Support, &s);
        req.pads = [Some(36), Some(38), None, None];
        let a = add_part(&req).expect("drums");
        assert!(a.events.iter().all(|e| e.pitch == 36 || e.pitch == 38));
        assert!(a.verdict.together.pass, "{}", a.verdict.words());
        req.pads = [None; 4];
        assert_eq!(add_part(&req), Err(Refused::NoDrumPads));
    }

    #[test]
    fn a_pitched_part_needs_chords() {
        let key = c_minor();
        let none = ChordTimeline::default();
        let row = riff_row(&none, &key, true);
        let s = settings(GROOVE);
        assert_eq!(
            add_part(&ask(&row, PartRole::Bass, Relation::Support, &s)),
            Err(Refused::NoChords)
        );
    }

    #[test]
    fn a_second_line_stays_under_the_tune_without_unisons_on_strong_beats() {
        let (row, _) = long_riff_row();
        for style in [GROOVE, MINIMAL, JAZZISH] {
            let s = settings(style);
            let a = add_part(&ask(&row, PartRole::Melody, Relation::Support, &s)).expect("a line");
            for e in &a.events {
                let st = step(e);
                if let Some(l) = row.lead_at(st) {
                    assert!(e.pitch <= l.pitch, "{}: {e:?} over {l:?}", style.name);
                    if row.grid.level(st) >= -2 {
                        assert_ne!(
                            e.pitch % 12,
                            l.pitch % 12,
                            "{}: a unison at {st}",
                            style.name
                        );
                    }
                }
            }
        }
    }

    /// On four rows (the riff with a beat, the riff alone, a tune that
    /// rests, with a beat, and a beat alone), every style, part and role
    /// plays its role and passes at least five of the six checks.
    #[test]
    fn every_role_and_part_plays_its_role_on_four_rows() {
        let rows = [
            ("riff and beat", four_bars(Some(long_riff()), true)),
            ("riff alone", four_bars(Some(long_riff()), false)),
            (
                "resting tune and beat",
                four_bars(Some(sparse_tune()), true),
            ),
            ("beat alone", four_bars(None, true)),
        ];
        let mut failed = Vec::new();
        for (name, row) in &rows {
            for style in [GROOVE, MINIMAL, JAZZISH] {
                let s = settings(style);
                for part in [
                    PartRole::Bass,
                    PartRole::Chords,
                    PartRole::Melody,
                    PartRole::Drums,
                ] {
                    for rel in Relation::ALL {
                        if rel == Relation::Answer && row.lead.is_none() {
                            assert_eq!(
                                add_part(&ask(row, part, rel, &s)),
                                Err(Refused::NothingToAnswer)
                            );
                            continue;
                        }
                        let a = add_part(&ask(row, part, rel, &s)).expect("a part");
                        if a.events.is_empty() || !a.verdict.together.pass || a.verdict.passed() < 5
                        {
                            failed.push(format!(
                                "{name}, {} {part:?} {rel:?}:\n{}",
                                style.name,
                                a.verdict.words()
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            failed.is_empty(),
            "{} failed:\n{}",
            failed.len(),
            failed.join("\n")
        );
    }
}
