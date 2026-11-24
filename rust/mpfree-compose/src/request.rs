//! Continue, end to end (phase 0C): from a clip and its reading to the
//! notes Keep writes, through the plan, the chords, the phrase and the
//! playing, with Try again on one bar. Phase 1 puts it on screen.
//!
//! [`continue_notes`] works on plain inputs (your notes in the clip's
//! ticks, a reading, the settings); [`continue_clip`] reads a clip in a
//! project and returns the [`Proposal`] a preview plays and Keep writes.

use std::sync::Arc;

use mpfree_context::chord::{ChordSpan, ChordTimeline};
use mpfree_context::groove::Lane;
use mpfree_context::harmony::RowHarmony;
use mpfree_context::{MusicalContext, PartRole, ReadBudget};
use mpfree_seq::{Holds, Note, Project, Speed};
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::event::NoteDecision;
use crate::event::{Event, Part, PartId, Source};
use crate::keep::{clip_notes, continuation, KeepError};
use crate::motif::GridNote;
use crate::phrase::{
    continue_line, decisions, try_bars as line_try_bars, try_notes as line_try_notes, Line,
    Phrasing,
};
use crate::plan::{plan, BarPlan};
use crate::playing::{play, timing, Touch};
use crate::preview::Proposal;
use crate::profile::InstrumentKind;
use crate::progression::progression_keeping;
use crate::read::{read, ToRead};
use crate::realize::{realize, Fit, PlainNote};
use crate::rules::Unmet;
use crate::settings::Settings;
use crate::sketch::Sketch;
use crate::timing::written;

/// What Continue is given.
#[derive(Clone, Copy)]
pub struct Continue<'a> {
    /// The clip's notes, in its own ticks as they sound on the grid, with
    /// who made each.
    pub notes: &'a [(PlainNote, Source)],
    /// The clip's length, in its ticks. A clip that isn't whole bars is
    /// carried on from its next bar line.
    pub length: Tick,
    /// The reading: key, chords over the clip's ticks, groove, the notes
    /// played.
    pub context: &'a MusicalContext,
    pub settings: &'a Settings,
    pub role: PartRole,
    pub kind: InstrumentKind,
    pub meter: Meter,
    pub bpm: f64,
    /// The swing the clip plays with.
    pub swing: Swing,
    /// The clip loops, so its last bar leads back into its first.
    pub loops: bool,
    /// How far the row's chords reach, in the clip's ticks (contract 4):
    /// a new bar wholly before it plays the chords the row plays there
    /// (`context.chords`, the row's Harmony line heard again); past it the
    /// new bars pick their own. 0: the clip's own bars only.
    pub row_length: Tick,
}

/// What Continue makes.
#[derive(Clone, Debug, PartialEq)]
pub struct Continued {
    pub plan: Vec<BarPlan>,
    /// The chords under every bar, yours as heard and the new suggested.
    pub chords: ChordTimeline,
    pub line: Line,
    /// The new bars' decisions, played.
    pub part: Part,
    pub events: Vec<Event>,
    /// The new notes as Keep writes them, in the clip's ticks.
    pub notes: Vec<Note>,
    /// The clip's new length.
    pub to: Tick,
    /// The phrase plan the new bars follow (`None` from the first
    /// engine): Try again on picked bars works from it.
    pub sketch: Option<Sketch>,
}

/// Why Continue made nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No clip there.
    NoClip,
    /// A pitched part made of pad hits: a pad's note depends on the kit.
    PadHits,
    /// A setting that never gives way is in the way.
    Unmet(Unmet),
    /// Too many notes, or too long, for a clip.
    Keep(KeepError),
    /// Try again on notes with none of the new notes among them.
    NoNewNotes,
    /// A beat carried on made nothing ([`crate::beat`]).
    Drums(crate::beat::DrumsRefused),
}

impl Refused {
    pub fn words(&self) -> String {
        match self {
            Refused::NoClip => "The clip has gone. Nothing was written.".to_string(),
            Refused::PadHits => "This clip plays pads, so there's no tune to carry on.".to_string(),
            Refused::Unmet(u) => u.words(),
            Refused::Keep(k) => k.words(),
            Refused::NoNewNotes => {
                "Pick some of the new notes to try them again: your own never change.".to_string()
            }
            Refused::Drums(d) => d.words(),
        }
    }
}

impl From<Unmet> for Refused {
    fn from(u: Unmet) -> Refused {
        Refused::Unmet(u)
    }
}

impl From<KeepError> for Refused {
    fn from(k: KeepError) -> Refused {
        Refused::Keep(k)
    }
}

/// Your bars: the clip's length in whole bars.
pub(crate) fn your_bars(req: &Continue) -> usize {
    let bar = req.meter.bar_ticks().max(1);
    ((req.length.max(1) + bar - 1) / bar) as usize
}

pub(crate) fn phrasing<'a>(req: &Continue<'a>, chords: &'a ChordTimeline) -> Phrasing<'a> {
    let ctx = req.context;
    let mut ph = Phrasing::new(
        &ctx.key,
        ctx.played_pcs,
        chords,
        &req.settings.style,
        req.notes,
        your_bars(req),
        req.meter,
    );
    ph.profile = req.kind.profile();
    ph.loops = req.loops;
    ph
}

/// The line's new bars as a played part, and its notes as Keep writes
/// them.
/// Your touch, from the notes you played (not mpfree's or a pattern's).
pub(crate) fn touch(req: &Continue) -> Touch {
    Touch::of(
        req.notes
            .iter()
            .filter(|(_, s)| s.learnable())
            .map(|(n, _)| n),
    )
}

pub(crate) fn perform(
    req: &Continue,
    ph: &Phrasing,
    plan: &[BarPlan],
    chords: &ChordTimeline,
    line: &Line,
) -> (Part, Vec<Event>) {
    perform_with(req, ph, plan, chords, line, true)
}

/// [`perform`], with the style's slow drift (`drift`) or without it (the
/// music-first engine, whose feel comes from the groove and the playing
/// rules).
pub(crate) fn perform_with(
    req: &Continue,
    ph: &Phrasing,
    plan: &[BarPlan],
    chords: &ChordTimeline,
    line: &Line,
    drift: bool,
) -> (Part, Vec<Event>) {
    let s = req.settings;
    let mut style = s.style;
    if !drift {
        style.drift_ms = 0.0;
    }
    let (register, ds) = decisions(ph, line, your_bars(req));
    let mut part = Part {
        id: PartId(0),
        role: req.role,
        kind: req.kind,
        register,
        decisions: ds,
    };
    let fit = Fit {
        harmony: chords,
        key: ph.clear.then_some(ph.key),
        played_pcs: ph.played_pcs,
        origin: 0,
    };
    let heard = realize(&part, &fit);
    play(
        &mut part,
        &heard,
        plan,
        chords,
        &style,
        req.meter,
        s.keeps.accents,
        &touch(req),
    );
    timing(
        &mut part,
        req.context.groove.as_ref(),
        &style,
        req.swing,
        s.seed,
        req.bpm,
    );
    let events = realize(&part, &fit);
    (part, events)
}

/// Carry the clip on by the settings' bars, music first: the new bars
/// planned as one phrase, the best of a few drafts
/// ([`crate::draft::continue_s1`]).
pub fn continue_notes(req: &Continue) -> Result<Continued, Refused> {
    continue_notes_keeping(req, None)
}

/// [`continue_notes`], keeping new bars of an earlier result: with
/// `Some((before, bars))`, each of `bars` that is still a new bar plays
/// exactly as it did in `before` (its notes, its chords, how it was
/// played), and the bars around it lead into and out of it. A locked bar
/// past the new length goes.
pub fn continue_notes_keeping(
    req: &Continue,
    keeping: Option<(&Continued, &[usize])>,
) -> Result<Continued, Refused> {
    crate::draft::continue_s1_keeping(req, keeping)
}

/// The first engine (phase 0C to S0): each bar given a job and its notes
/// searched bar by bar. Kept as a baseline the listening tests and the
/// bench play beside the music-first engine.
pub fn first_engine(req: &Continue) -> Result<Continued, Refused> {
    first_engine_keeping(req, None)
}

/// [`first_engine`], keeping new bars of an earlier result, as
/// [`continue_notes_keeping`] does.
pub fn first_engine_keeping(
    req: &Continue,
    keeping: Option<(&Continued, &[usize])>,
) -> Result<Continued, Refused> {
    let s = req.settings;
    let bar = req.meter.bar_ticks().max(1);
    let yours = your_bars(req);
    let total = yours + usize::from(s.bars.max(1));
    let beats = f32::from(req.meter.beats.max(1));
    let density = req.notes.len() as f32 / (yours as f32 * beats);
    let plan = plan(s, yours, total, density);
    // Keep Chords: the new bars play yours again, in order.
    let key = if s.keeps.chords {
        None
    } else {
        req.context.key.key()
    };
    let kept: Vec<usize> = keeping.map_or_else(Vec::new, |(before, bars)| {
        let mut k: Vec<usize> = bars
            .iter()
            .copied()
            .filter(|&b| b >= yours && b < total && b < before.plan.len())
            .collect();
        k.sort_unstable();
        k.dedup();
        k
    });
    let locked_chords: Vec<(usize, Vec<ChordSpan>)> = match keeping {
        Some((before, _)) => kept
            .iter()
            .map(|&b| (b, chords_in_bar(&before.chords, b, bar)))
            .collect(),
        None => Vec::new(),
    };
    let chords = progression_keeping(
        &req.context.chords,
        key,
        &s.style.harmony,
        &plan,
        s.variation,
        s.seed,
        0,
        req.meter,
        &locked_chords,
    );
    let ph = phrasing(req, &chords);
    let locks: Vec<Option<Vec<GridNote>>> = match keeping {
        Some((before, _)) => (0..total)
            .map(|b| kept.contains(&b).then(|| before.line.bar(b, ph.grid)))
            .collect(),
        None => Vec::new(),
    };
    let line = continue_line(&ph, &plan, s, &locks, &[])?;
    let (mut part, mut events) = perform(req, &ph, &plan, &chords, &line);
    if let Some((before, _)) = keeping {
        // The locked bars as they were played, exactly: the fresh
        // decisions and events everywhere else.
        let free = |at: Tick| !kept.contains(&(at.div_euclid(bar) as usize));
        (part.decisions, events) = splice(before, &part, &events, &free);
    }
    let to = total as Tick * bar;
    let notes = clip_notes(&events, req.bpm, to)?;
    Ok(Continued {
        plan,
        chords,
        line,
        part,
        events,
        notes,
        to,
        sketch: None,
    })
}

/// The chords under bar `b` of `t`, cut to the bar.
pub(crate) fn chords_in_bar(t: &ChordTimeline, b: usize, bar: Tick) -> Vec<ChordSpan> {
    let (from, to) = (b as Tick * bar, (b as Tick + 1) * bar);
    t.spans()
        .iter()
        .filter(|s| s.start < to && s.end() > from)
        .map(|s| {
            let a = s.start.max(from);
            ChordSpan {
                start: a,
                len: s.end().min(to) - a,
                ..*s
            }
        })
        .collect()
}

/// `fresh`'s decisions and events where `inside` says, `before`'s
/// everywhere else, in time order.
pub(crate) fn splice(
    before: &Continued,
    fresh: &Part,
    events: &[Event],
    inside: &dyn Fn(Tick) -> bool,
) -> (Vec<NoteDecision>, Vec<Event>) {
    let mut ds: Vec<NoteDecision> = before
        .part
        .decisions
        .iter()
        .filter(|d| !inside(d.at))
        .copied()
        .chain(fresh.decisions.iter().filter(|d| inside(d.at)).copied())
        .collect();
    ds.sort_by_key(|d| d.at);
    let mut ev: Vec<Event> = before
        .events
        .iter()
        .filter(|e| !inside(e.at))
        .copied()
        .chain(events.iter().filter(|e| inside(e.at)).copied())
        .collect();
    ev.sort_by_key(|e| e.at);
    (ds, ev)
}

/// Try one new bar again: the bar is made afresh, leading in from the bar
/// before and out into the bar after (or back into bar 1), and every
/// other note stays exactly as it was, played as it was.
pub fn try_bar(
    req: &Continue,
    before: &Continued,
    bar: usize,
    attempt: u16,
) -> Result<Continued, Refused> {
    try_bars(req, before, &[bar], attempt)
}

/// Try the new bars `bars` again (from 0, your first bar), together:
/// each made afresh between the bars that stay, every other note exactly
/// as it was, played as it was. A music-first result's bars are filled
/// again from its phrase plan, each keeping its anchor's note.
pub fn try_bars(
    req: &Continue,
    before: &Continued,
    bars: &[usize],
    attempt: u16,
) -> Result<Continued, Refused> {
    if let Some(sk) = &before.sketch {
        return crate::draft::try_bars_s1(req, before, sk, bars, attempt);
    }
    let ph = phrasing(req, &before.chords);
    let line = line_try_bars(&ph, &before.plan, req.settings, &before.line, bars, attempt)?;
    let (mut part, events) = perform(req, &ph, &before.plan, &before.chords, &line);
    let bt = req.meter.bar_ticks().max(1);
    let inside = |at: Tick| bars.contains(&(at.div_euclid(bt) as usize));
    let (ds, events) = splice(before, &part, &events, &inside);
    part.decisions = ds;
    let notes = clip_notes(&events, req.bpm, before.to)?;
    Ok(Continued {
        plan: before.plan.clone(),
        chords: before.chords.clone(),
        line,
        part,
        events,
        notes,
        to: before.to,
        sketch: before.sketch.clone(),
    })
}

/// Try the picked new notes again: `picked` are notes of the clip as it is
/// heard with `before` (where each is written, in the clip's ticks, and
/// its note). Each gets another note in its place where one fits, played
/// as it was (its timing, length and loudness); every other note stays
/// exactly as it was. Your own notes are never tried.
pub fn try_notes(
    req: &Continue,
    before: &Continued,
    picked: &[(Tick, u8)],
    attempt: u16,
) -> Result<Continued, Refused> {
    let ph = phrasing(req, &before.chords);
    let step = Tick::from(mpfree_context::groove::STEP);
    // Each pick's event, then the note of the line it plays.
    let mut events_picked: Vec<usize> = Vec::new();
    let mut line_picked: Vec<usize> = Vec::new();
    for &(tick, pitch) in picked {
        let Some(e) = before
            .events
            .iter()
            .position(|e| e.pitch == pitch && written(e.at, e.timing, req.bpm, before.to) == tick)
        else {
            continue;
        };
        let at = before.events[e].at;
        let from = ph.your_bars as i32 * ph.grid.bar;
        let Some(i) = before
            .line
            .notes
            .iter()
            .enumerate()
            .filter(|(_, m)| m.on >= from && m.pitch(ph.key) == pitch)
            .min_by_key(|(_, m)| (Tick::from(m.on) * step - at).abs())
            .map(|(i, _)| i)
        else {
            continue;
        };
        events_picked.push(e);
        line_picked.push(i);
    }
    if line_picked.is_empty() {
        return Err(Refused::NoNewNotes);
    }
    let line = line_try_notes(
        &ph,
        &before.plan,
        req.settings,
        &before.line,
        &line_picked,
        attempt,
    )?;
    // The decisions read again for the bars touched; the events as they
    // were, each picked one on its new note.
    let (mut part, events) = perform(req, &ph, &before.plan, &before.chords, &line);
    let bt = req.meter.bar_ticks().max(1);
    let touched: Vec<usize> = events_picked
        .iter()
        .map(|&e| before.events[e].at.div_euclid(bt) as usize)
        .collect();
    let inside = |at: Tick| touched.contains(&(at.div_euclid(bt) as usize));
    let (ds, _) = splice(before, &part, &events, &inside);
    part.decisions = ds;
    let mut events = before.events.clone();
    for (&e, &i) in events_picked.iter().zip(&line_picked) {
        events[e].pitch = line.notes[i].pitch(ph.key);
    }
    let notes = clip_notes(&events, req.bpm, before.to)?;
    Ok(Continued {
        plan: before.plan.clone(),
        chords: before.chords.clone(),
        line,
        part,
        events,
        notes,
        to: before.to,
        sketch: before.sketch.clone(),
    })
}

/// A chord timeline in song ticks, in a clip's own ticks at `speed`.
fn in_clip_ticks(t: &ChordTimeline, speed: Speed) -> ChordTimeline {
    if speed == Speed::NORMAL {
        return t.clone();
    }
    let at = |x: Tick| speed.to_clip(x as f64).round() as Tick;
    let spans: Vec<ChordSpan> = t
        .spans()
        .iter()
        .map(|s| ChordSpan {
            start: at(s.start),
            len: at(s.end()) - at(s.start),
            ..*s
        })
        .filter(|s| s.len > 0)
        .collect();
    ChordTimeline::new(spans).unwrap_or_default()
}

/// Where Continue is asked for: a clip in a project, and how to read it.
#[derive(Clone, Copy)]
pub struct OnClip<'a> {
    pub project: &'a Project,
    pub track: usize,
    pub slot: usize,
    pub role: PartRole,
    /// Other clips to listen to (the rest of its row).
    pub others: &'a [ToRead],
    /// The drum a pad of a drum track plays.
    pub lane_of: &'a dyn Fn(usize, u8) -> Option<Lane>,
    pub transport_swing: Swing,
    pub meter: Meter,
    pub bpm: f64,
    pub budget: ReadBudget,
    /// The row's Harmony line over its length (contract 4,
    /// `mpfree_seq::row_harmony`), if it has one: heard again with this
    /// reading, the chords you set stay.
    pub row: Option<&'a RowHarmony>,
}

/// What a Continue on a clip makes: a new result, or part of an earlier
/// one again (phase 1 step 3).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Making {
    /// A new result. With an earlier one, its `locked` new bars (from 0,
    /// your first bar) play exactly as they did, where they still fit.
    #[default]
    New,
    Keeping {
        before: Arc<Continued>,
        locked: Vec<u16>,
    },
    /// These new bars of `before` again, every other bar as it was.
    Bars {
        before: Arc<Continued>,
        bars: Vec<u16>,
        attempt: u16,
    },
    /// These notes of the clip as heard with `before` (where each is
    /// written, and its note) again, every other note as it was.
    Notes {
        before: Arc<Continued>,
        notes: Vec<(Tick, u8)>,
        attempt: u16,
    },
}

/// Continue on a clip: read it and the others, carry it on, and the
/// proposal a preview plays and Keep writes (bound to every clip read,
/// at its revision).
pub fn continue_clip(
    on: &OnClip,
    settings: &Settings,
    kind: InstrumentKind,
) -> Result<(Proposal, Continued), Refused> {
    continue_clip_read(on, settings, kind).0
}

/// [`continue_clip`], with how many clips the reading left out (gone, or
/// past its budget), which the app counts.
pub fn continue_clip_read(
    on: &OnClip,
    settings: &Settings,
    kind: InstrumentKind,
) -> (Result<(Proposal, Continued), Refused>, usize) {
    continue_clip_how(on, settings, kind, &Making::New)
}

/// [`continue_clip_read`], making what `how` asks for.
pub fn continue_clip_how(
    on: &OnClip,
    settings: &Settings,
    kind: InstrumentKind,
    how: &Making,
) -> (Result<(Proposal, Continued), Refused>, usize) {
    let Some(clip) = on.project.clip(on.track, on.slot) else {
        return (Err(Refused::NoClip), 0);
    };
    if clip.holds == Holds::Hits && on.role == PartRole::Drums {
        // A beat is carried on from itself: the row isn't read.
        return (
            crate::beat::continue_drum_clip_how(on, settings, kind, how),
            0,
        );
    }
    if clip.holds != Holds::Notes {
        return (Err(Refused::PadHits), 0);
    }
    let mut clips = vec![ToRead {
        track: on.track,
        slot: on.slot,
        role: on.role,
    }];
    clips.extend(
        on.others
            .iter()
            .filter(|r| (r.track, r.slot) != (on.track, on.slot)),
    );
    let (mut ctx, left) = read(
        on.project,
        &clips,
        on.lane_of,
        on.transport_swing,
        on.meter,
        on.budget,
    );
    // The row's chords (contract 4): its line heard again with this
    // reading, over the row's length; without a line, the reading. The
    // reading hears every clip looped over the longest, so it reaches as
    // far as the row plays.
    let heard_len = ctx.chords.len();
    if let Some(r) = on.row {
        ctx.chords = r.line.rehear(&ctx.chords);
    }
    let row_len = on.row.map_or(heard_len, |r| r.length).min(ctx.chords.len());
    let row_length = clip.speed.to_clip(row_len as f64).round() as Tick;
    ctx.chords = in_clip_ticks(&ctx.chords, clip.speed);
    let length = clip.length.max(1);
    let notes: Vec<(PlainNote, Source)> = clip
        .notes
        .iter()
        .filter(|n| n.on)
        .map(|n| {
            let at = clip
                .quantise
                .map_or(n.tick as f64, |q| q.apply(n.tick as f64))
                .round() as Tick;
            (
                PlainNote {
                    at: at.rem_euclid(length),
                    len: n.len,
                    pitch: n.note,
                    velocity: n.velocity,
                },
                Source::You,
            )
        })
        .collect();
    let swing = clip
        .swing
        .or(on.project.tracks[on.track].swing)
        .unwrap_or(on.transport_swing);
    let req = Continue {
        notes: &notes,
        length,
        context: &ctx,
        settings,
        role: on.role,
        kind,
        meter: on.meter,
        bpm: on.bpm,
        swing,
        loops: !clip.once,
        row_length,
    };
    let as_bars = |v: &[u16]| v.iter().map(|&b| usize::from(b)).collect::<Vec<usize>>();
    let made = match how {
        Making::New => continue_notes(&req),
        Making::Keeping { before, locked } => {
            continue_notes_keeping(&req, Some((before, &as_bars(locked))))
        }
        Making::Bars {
            before,
            bars,
            attempt,
        } => try_bars(&req, before, &as_bars(bars), *attempt),
        Making::Notes {
            before,
            notes,
            attempt,
        } => try_notes(&req, before, notes, *attempt),
    };
    let result = made.and_then(|made| {
        let edit = continuation(clip, made.notes.clone(), made.to)?;
        Ok((
            Proposal {
                edit,
                read: ctx.read.clone(),
            },
            made,
        ))
    });
    (result, left.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rhythm::{syncopation, Grid};
    use crate::rules::{RangeSetting, Rule};
    use crate::settings::Ending;
    use crate::style::{Style, GROOVE, JAZZISH, MINIMAL};
    use mpfree_context::groove::STEP;
    use mpfree_context::listen::{listen, Played};
    use mpfree_seq::{Clip, Track};
    use mpfree_transport::time::PPQ;

    const BEAT: Tick = PPQ as Tick;
    const BAR: Tick = 4 * BEAT;

    fn riff() -> Vec<(PlainNote, Source)> {
        [
            (0, 2, 60, 100),
            (2, 1, 63, 88),
            (3, 1, 67, 92),
            (4, 2, 65, 96),
            (6, 2, 63, 86),
            (8, 3, 62, 94),
            (11, 1, 63, 80),
            (12, 4, 60, 90),
            (16, 2, 67, 100),
            (18, 1, 68, 86),
            (19, 1, 67, 84),
            (20, 2, 65, 94),
            (22, 2, 63, 84),
            (24, 2, 62, 92),
            (26, 2, 63, 84),
            (28, 4, 65, 88),
        ]
        .iter()
        .map(|&(on, len, pitch, velocity)| {
            (
                PlainNote {
                    at: on * STEP,
                    len: len * STEP,
                    pitch,
                    velocity,
                },
                Source::You,
            )
        })
        .collect()
    }

    fn context(notes: &[(PlainNote, Source)]) -> MusicalContext {
        let played: Vec<Played> = notes
            .iter()
            .map(|(n, _)| Played::new(n.at, n.len, n.pitch, n.velocity))
            .collect();
        let r = listen(&played, 2 * BAR, ReadBudget::default(), Meter::default());
        MusicalContext {
            key: r.key,
            chords: r.chords,
            groove: None,
            played_pcs: r.played_pcs,
            motifs: Vec::new(),
            read: Vec::new(),
            cut_short: r.cut_short,
        }
    }

    fn ask<'a>(
        notes: &'a [(PlainNote, Source)],
        ctx: &'a MusicalContext,
        settings: &'a Settings,
    ) -> Continue<'a> {
        Continue {
            notes,
            length: 2 * BAR,
            context: ctx,
            settings,
            role: PartRole::Melody,
            kind: InstrumentKind::OneNote,
            meter: Meter::default(),
            bpm: 120.0,
            swing: Swing::default(),
            loops: true,
            row_length: 0,
        }
    }

    fn settings(style: Style, bars: u8) -> Settings {
        Settings {
            style,
            bars,
            seed: 11,
            ..Settings::default()
        }
    }

    #[test]
    fn three_styles_carry_the_riff_on_each_in_its_own_way() {
        let notes = riff();
        let ctx = context(&notes);
        let grid = Grid::of(Meter::default());
        let mut per_bar = Vec::new();
        let mut sync = Vec::new();
        let mut on_eighths = Vec::new();
        for style in [MINIMAL, GROOVE, JAZZISH] {
            let s = settings(style, 8);
            let req = ask(&notes, &ctx, &s);
            let a = continue_notes(&req).unwrap();
            assert_eq!(a, continue_notes(&req).unwrap(), "the same request");
            assert_eq!(a.to, 10 * BAR);
            assert_eq!(a.notes.len(), a.events.len());
            assert!(a
                .notes
                .iter()
                .all(|n| (2 * BAR - 60..10 * BAR).contains(&n.tick)));
            assert!(a.events.iter().all(|e| e.source == Source::Mpfree));
            assert!(a.events.iter().all(|e| (20..=127).contains(&e.velocity)));
            per_bar.push(a.events.len() as f64 / 8.0);
            let mut sy = 0.0;
            for b in 2..10 {
                let on: Vec<i32> = a.line.bar(b, grid).iter().map(|m| m.on).collect();
                sy += syncopation(&on, grid);
            }
            sync.push(sy / 8.0);
            let eighths = a.line.notes[16..].iter().filter(|m| m.on % 2 == 0).count();
            on_eighths.push(eighths as f64 / (a.line.notes.len() - 16) as f64);
            // The phrase lands home, on the last bar's last note.
            let last = a.line.notes.last().unwrap();
            assert!(
                matches!(last.pos.rem_euclid(7), 0 | 2 | 4),
                "{}",
                style.name
            );
        }
        // Groove is the busiest and pushes off the beat; Minimal thins
        // the riff out; Jazz-ish keeps to 1/8s.
        assert!(
            per_bar[1] > per_bar[0] && per_bar[1] > per_bar[2],
            "{per_bar:?}"
        );
        assert!(per_bar[0] < 8.0, "{per_bar:?}");
        assert!(sync[1] > sync[0], "{sync:?}");
        assert!(on_eighths[2] > 0.95, "{on_eighths:?}");
    }

    #[test]
    fn the_new_notes_drift_together_or_sit_on_the_drummer() {
        let notes = riff();
        let mut ctx = context(&notes);
        let s = settings(GROOVE, 4);
        // The first engine drifts; the music-first engine plays without
        // drift, its feel from the groove and the playing rules.
        let free = first_engine(&ask(&notes, &ctx, &s)).unwrap();
        assert!(free.part.decisions.iter().any(|d| d.timing.play != 0.0));
        let on_grid = free.notes.iter().filter(|n| n.tick % STEP == 0).count();
        assert!(on_grid < free.notes.len(), "the drift moves notes");
        let plain = continue_notes(&ask(&notes, &ctx, &s)).unwrap();
        assert!(plain.part.decisions.iter().all(|d| d.timing.play == 0.0));
        // With drums read, the drummer's groove and no drift.
        ctx.groove = Some(mpfree_context::groove::GrooveTemplate::read(
            &[mpfree_context::groove::Hit {
                lane: Lane::Hats,
                heard: (BEAT / 2 + STEP / 4) as f64,
                velocity: 90,
            }],
            16,
            Swing::default(),
        ));
        let grooved = continue_notes(&ask(&notes, &ctx, &s)).unwrap();
        assert!(grooved.part.decisions.iter().all(|d| d.timing.play == 0.0));
        for (d, n) in grooved.part.decisions.iter().zip(&grooved.events) {
            assert_eq!(d.at, n.at);
            let late = (d.at % BAR) == BEAT / 2;
            assert_eq!(d.timing.groove != 0.0, late, "{d:?}");
        }
    }

    #[test]
    fn trying_a_bar_again_keeps_every_other_note_as_it_was() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(JAZZISH, 4);
        let req = ask(&notes, &ctx, &s);
        let a = continue_notes(&req).unwrap();
        let mut changed = 0;
        for attempt in 1..=3 {
            let t = try_bar(&req, &a, 3, attempt).unwrap();
            let outside = |v: &[Note]| -> Vec<Note> {
                v.iter()
                    .copied()
                    .filter(|n| !(3 * BAR - 60..4 * BAR - 60).contains(&n.tick))
                    .collect()
            };
            assert_eq!(outside(&t.notes), outside(&a.notes));
            assert_eq!(t.to, a.to);
            if t.notes != a.notes {
                changed += 1;
            }
        }
        assert!(changed >= 2, "{changed}");
    }

    /// The notes of `v` outside bars `bars` (a little early counts as the
    /// bar it leans into, as the drift can put a note there).
    fn outside_bars(v: &[Note], bars: &[usize]) -> Vec<Note> {
        v.iter()
            .copied()
            .filter(|n| {
                !bars.iter().any(|&b| {
                    let b = b as Tick;
                    (b * BAR - 60..(b + 1) * BAR - 60).contains(&n.tick)
                })
            })
            .collect()
    }

    #[test]
    fn trying_two_bars_again_keeps_every_other_bar() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(GROOVE, 4);
        let req = ask(&notes, &ctx, &s);
        let a = continue_notes(&req).unwrap();
        let mut changed = 0;
        for attempt in 1..=3 {
            let t = try_bars(&req, &a, &[2, 4], attempt).unwrap();
            assert_eq!(
                outside_bars(&t.notes, &[2, 4]),
                outside_bars(&a.notes, &[2, 4])
            );
            assert_eq!(t.chords, a.chords);
            if t.notes != a.notes {
                changed += 1;
            }
        }
        assert!(changed >= 2, "{changed}");
        // Your bars are never tried.
        let t = try_bars(&req, &a, &[0, 1], 1).unwrap();
        assert_eq!(t.notes, a.notes);
    }

    #[test]
    fn trying_picked_notes_again_moves_only_them_in_their_places() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(JAZZISH, 4);
        let req = ask(&notes, &ctx, &s);
        let a = continue_notes(&req).unwrap();
        // Two new notes in bar 4 (from 1): where they're written and what.
        let in_bar: Vec<Note> = a
            .notes
            .iter()
            .copied()
            .filter(|n| (3 * BAR..4 * BAR).contains(&n.tick))
            .collect();
        assert!(in_bar.len() >= 3, "{in_bar:?}");
        let picked: Vec<(Tick, u8)> = in_bar[1..3].iter().map(|n| (n.tick, n.note)).collect();
        let mut moved = 0;
        for attempt in 1..=3 {
            let t = try_notes(&req, &a, &picked, attempt).unwrap();
            assert_eq!(t.notes.len(), a.notes.len());
            // Every note in the same place, as long and as loud; only the
            // picked ones may sound another note.
            let place = |v: &[Note]| -> Vec<(Tick, Tick, u8)> {
                let mut p: Vec<_> = v.iter().map(|n| (n.tick, n.len, n.velocity)).collect();
                p.sort_unstable();
                p
            };
            assert_eq!(place(&t.notes), place(&a.notes));
            for n in &a.notes {
                if !picked.iter().any(|&(tick, _)| tick == n.tick) {
                    assert!(t.notes.contains(n), "{n:?} moved");
                }
            }
            if t.notes != a.notes {
                moved += 1;
            }
        }
        assert!(moved >= 2, "{moved}");
        // Your own notes can't be tried (the riff's first note), nor no
        // notes at all.
        assert_eq!(
            try_notes(&req, &a, &[], 1).unwrap_err(),
            Refused::NoNewNotes
        );
        assert_eq!(
            try_notes(&req, &a, &[(0, 60)], 1).unwrap_err(),
            Refused::NoNewNotes
        );
    }

    #[test]
    fn a_locked_bar_stays_through_a_new_try_on_everything() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(GROOVE, 4);
        let req = ask(&notes, &ctx, &s);
        let a = continue_notes(&req).unwrap();
        let in_bar = |v: &[Note], b: Tick| -> Vec<Note> {
            v.iter()
                .copied()
                .filter(|n| (b * BAR - 60..(b + 1) * BAR - 60).contains(&n.tick))
                .collect()
        };
        let mut others_changed = 0;
        for seed in 12..16 {
            let s2 = Settings { seed, ..s };
            let req2 = ask(&notes, &ctx, &s2);
            let t = continue_notes_keeping(&req2, Some((&a, &[3]))).unwrap();
            assert_eq!(in_bar(&t.notes, 3), in_bar(&a.notes, 3), "seed {seed}");
            assert_eq!(
                chords_in_bar(&t.chords, 3, BAR),
                chords_in_bar(&a.chords, 3, BAR),
                "seed {seed}"
            );
            if outside_bars(&t.notes, &[3]) != outside_bars(&a.notes, &[3]) {
                others_changed += 1;
            }
        }
        assert!(others_changed >= 3, "{others_changed}");
        // Fewer bars: a lock past the end goes.
        let s3 = Settings { bars: 1, ..s };
        let req3 = ask(&notes, &ctx, &s3);
        let t = continue_notes_keeping(&req3, Some((&a, &[4]))).unwrap();
        assert_eq!(t.to, 3 * BAR);
    }

    #[test]
    fn keep_chords_plays_your_chords_again() {
        let notes = riff();
        let ctx = context(&notes);
        let mut s = settings(JAZZISH, 4);
        s.keeps.chords = true;
        let a = continue_notes(&ask(&notes, &ctx, &s)).unwrap();
        let yours = ctx.chords.span_at(0).unwrap().answer;
        for b in 2..6 {
            assert_eq!(a.chords.span_at(b * BAR).unwrap().answer, yours);
        }
    }

    #[test]
    fn a_refusal_names_the_setting() {
        let notes = riff();
        let ctx = context(&notes);
        let mut s = settings(MINIMAL, 4);
        s.range = RangeSetting::Narrow;
        let mut req = ask(&notes, &ctx, &s);
        req.kind = InstrumentKind::Bass;
        let r = continue_notes(&req).unwrap_err();
        let Refused::Unmet(u) = r else {
            panic!("{r:?}")
        };
        assert!(matches!(u.rule, Rule::RangeSetting | Rule::InstrumentRange));
        assert!(
            r.words().starts_with("Can't fit a new bar 3"),
            "{}",
            r.words()
        );
    }

    fn lanes(_: usize, _: u8) -> Option<Lane> {
        None
    }

    fn on_clip(project: &Project, slot: usize) -> OnClip<'_> {
        OnClip {
            project,
            track: 0,
            slot,
            role: PartRole::Melody,
            others: &[],
            lane_of: &lanes,
            transport_swing: Swing::default(),
            meter: Meter::default(),
            bpm: 120.0,
            budget: ReadBudget::default(),
            row: None,
        }
    }

    #[test]
    fn a_clip_in_a_project_becomes_a_proposal_bound_to_what_was_read() {
        let mut p = Project::new();
        let mut t = Track::new("Lead", 0);
        let mut c = Clip::new("riff", 2 * BAR);
        c.holds = Holds::Notes;
        c.notes = riff()
            .iter()
            .map(|(n, _)| mpfree_seq::Note::new(n.at, n.len, n.pitch, n.velocity))
            .collect();
        t.slots[0] = Some(c);
        p.tracks.push(t);
        p.give_clip_ids();
        let on = on_clip(&p, 0);
        let s = Settings {
            ending: Ending::Home,
            ..settings(MINIMAL, 2)
        };
        let (proposal, made) = continue_clip(&on, &s, InstrumentKind::OneNote).unwrap();
        assert_eq!(proposal.edit.length, Some((2 * BAR, 4 * BAR)));
        assert_eq!(proposal.edit.add, made.notes);
        assert_eq!(proposal.read.len(), 1);
        assert_eq!(
            proposal.read[0].revision,
            crate::bind::revision(p.clip(0, 0).unwrap())
        );
        // A clip of pad hits has no tune to carry on.
        p.tracks[0].slots[0].as_mut().unwrap().holds = Holds::Hits;
        assert_eq!(
            continue_clip(&on_clip(&p, 0), &s, InstrumentKind::OneNote).unwrap_err(),
            Refused::PadHits
        );
        assert_eq!(
            continue_clip(&on_clip(&p, 1), &s, InstrumentKind::OneNote).unwrap_err(),
            Refused::NoClip
        );
    }
}
