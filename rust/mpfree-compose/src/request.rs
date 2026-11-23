//! Continue, end to end (phase 0C): from a clip and its reading to the
//! notes Keep writes, through the plan, the chords, the phrase and the
//! playing, with Try again on one bar. Phase 1 puts it on screen.
//!
//! [`continue_notes`] works on plain inputs (your notes in the clip's
//! ticks, a reading, the settings); [`continue_clip`] reads a clip in a
//! project and returns the [`Proposal`] a preview plays and Keep writes.

use mpfree_context::chord::{ChordSpan, ChordTimeline};
use mpfree_context::groove::Lane;
use mpfree_context::{MusicalContext, PartRole, ReadBudget};
use mpfree_seq::{Holds, Note, Project, Speed};
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::event::{Event, Part, PartId, Source};
use crate::keep::{clip_notes, continuation, KeepError};
use crate::phrase::{continue_line, decisions, try_again, Line, Phrasing};
use crate::plan::{plan, BarPlan};
use crate::playing::{play, timing, Touch};
use crate::preview::Proposal;
use crate::profile::InstrumentKind;
use crate::progression::progression;
use crate::read::{read, ToRead};
use crate::realize::{realize, Fit, PlainNote};
use crate::rules::Unmet;
use crate::settings::Settings;

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
}

impl Refused {
    pub fn words(&self) -> String {
        match self {
            Refused::NoClip => "The clip has gone. Nothing was written.".to_string(),
            Refused::PadHits => "This clip plays pads, so there's no tune to carry on.".to_string(),
            Refused::Unmet(u) => u.words(),
            Refused::Keep(k) => k.words(),
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
    let s = req.settings;
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
        &s.style,
        req.meter,
        s.keeps.accents,
        &touch(req),
    );
    timing(
        &mut part,
        req.context.groove.as_ref(),
        &s.style,
        req.swing,
        s.seed,
        req.bpm,
    );
    let events = realize(&part, &fit);
    (part, events)
}

/// Carry the clip on by the settings' bars.
pub fn continue_notes(req: &Continue) -> Result<Continued, Refused> {
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
    let chords = progression(
        &req.context.chords,
        key,
        &s.style.harmony,
        &plan,
        s.variation,
        s.seed,
        0,
        req.meter,
    );
    let ph = phrasing(req, &chords);
    let line = continue_line(&ph, &plan, s, &[], &[])?;
    let (part, events) = perform(req, &ph, &plan, &chords, &line);
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
    })
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
    let ph = phrasing(req, &before.chords);
    let line = try_again(&ph, &before.plan, req.settings, &before.line, bar, attempt)?;
    let (mut part, mut events) = perform(req, &ph, &before.plan, &before.chords, &line);
    let bt = req.meter.bar_ticks().max(1);
    let inside = |at: Tick| at.div_euclid(bt) as usize == bar;
    let mut ds: Vec<_> = before
        .part
        .decisions
        .iter()
        .filter(|d| !inside(d.at))
        .copied()
        .chain(part.decisions.iter().filter(|d| inside(d.at)).copied())
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
    part.decisions = ds;
    events = ev;
    let notes = clip_notes(&events, req.bpm, before.to)?;
    Ok(Continued {
        plan: before.plan.clone(),
        chords: before.chords.clone(),
        line,
        part,
        events,
        notes,
        to: before.to,
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
}

/// Continue on a clip: read it and the others, carry it on, and the
/// proposal a preview plays and Keep writes (bound to every clip read,
/// at its revision).
pub fn continue_clip(
    on: &OnClip,
    settings: &Settings,
    kind: InstrumentKind,
) -> Result<(Proposal, Continued), Refused> {
    let clip = on.project.clip(on.track, on.slot).ok_or(Refused::NoClip)?;
    if clip.holds != Holds::Notes {
        return Err(Refused::PadHits);
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
    let (mut ctx, _) = read(
        on.project,
        &clips,
        on.lane_of,
        on.transport_swing,
        on.meter,
        on.budget,
    );
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
    };
    let made = continue_notes(&req)?;
    let edit = continuation(clip, made.notes.clone(), made.to)?;
    Ok((
        Proposal {
            edit,
            read: ctx.read.clone(),
        },
        made,
    ))
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
        let free = continue_notes(&ask(&notes, &ctx, &s)).unwrap();
        assert!(free.part.decisions.iter().any(|d| d.timing.play != 0.0));
        let on_grid = free.notes.iter().filter(|n| n.tick % STEP == 0).count();
        assert!(on_grid < free.notes.len(), "the drift moves notes");
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
