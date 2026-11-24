//! Continue on a drum clip (step 4 of the composing plan, the engine):
//! your beat carried on, bar by bar, as a drummer keeps a groove going.
//!
//! The new bars work from your bars in turn. Your kick and snare, the
//! backbone, stay where you put them; what moves is what a drummer moves:
//! a kick pushed a sixteenth or added before the backbeat, a ghost snare,
//! the hats opened up or thinned, a percussion hit in a gap. Variation
//! sets how much moves, Activity whether the hats and percussion get
//! busier or sparser than yours, Similarity how many bars come back as
//! you played them. Every fourth bar ends with a fill into the next
//! phrase (Minimal's is the last half beat, Groove's and Jazz-ish's the
//! last beat), and the last new bar ends as Ending asks: Home with a fill
//! and a kick on its last step, Loops back with the fill that leads into
//! your first bar, Open with its last beat left to the hats. The bar
//! before a landing that fills keeps its groove, so two fills never come
//! back to back.
//!
//! Each hit keeps the timing it was played with: a sixteenth moved keeps
//! its push or drag. Pads keep their notes (a kit's pad numbers), and only
//! pads your bars play are used, so nothing lands on a pad the kit
//! doesn't have. What lane a pad is (kick, snare, hats, other) comes from
//! the kit's names, as Add a part reads them; a pad with no name is
//! treated as percussion.
//!
//! Pure: the same ask gives the same bars; Try again's count changes the
//! dice.

use mpfree_context::chord::ChordTimeline;
use mpfree_context::groove::{Lane, STEP};
use mpfree_context::{ClipRead, ClipRef, PartRole};
use mpfree_seq::{Holds, Note};
use mpfree_transport::time::{Meter, Tick};

use crate::dice::{DiceName, Job};
use crate::event::{Part, PartId};
use crate::keep::KeepError;
use crate::phrase::Line;
use crate::plan::{BarJob, BarPlan};
use crate::request::{Continued, Making, Refused};
use crate::settings::{Ending, Settings};

/// Most new bars Continue adds to a drum clip in one go.
pub const MOST_BARS: u8 = 16;
/// A drum hit's length when it is added: a sixteenth.
const HIT: Tick = STEP;

/// A hit of the clip, in its own ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub at: Tick,
    pub len: Tick,
    pub pad: u8,
    pub velocity: u8,
}

/// Continue on a drum clip.
pub struct DrumsAsk<'a> {
    /// The clip's hits, as they sound on the grid (after its quantise).
    pub hits: &'a [Hit],
    /// The clip's length, in its ticks: carried on from its next bar line.
    pub length: Tick,
    /// The lane each pad plays, as the kit names it; None: percussion.
    pub lane_of: &'a dyn Fn(u8) -> Option<Lane>,
    pub meter: Meter,
    /// Style, Similarity, Variation, Activity, Bars and Ending.
    pub settings: &'a Settings,
    /// Try again's count: 0 is the first answer.
    pub attempt: u16,
}

/// What Continue on drums makes.
#[derive(Clone, Debug, PartialEq)]
pub struct Drummed {
    /// Every bar's plan, yours then the new ones: Yours, Repeat, Vary or
    /// Land.
    pub plan: Vec<BarPlan>,
    /// The bars (from 0, yours counted) that end with a fill.
    pub fills: Vec<usize>,
    /// The new hits as Keep writes them, in the clip's ticks.
    pub notes: Vec<Note>,
    /// The clip's new length.
    pub to: Tick,
}

/// Why nothing was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumsRefused {
    /// No clip there.
    NoClip,
    /// A clip of notes: Continue in the note editor carries a tune on.
    Notes,
    /// The clip has no hits to carry on.
    Empty,
    /// The meter has no sixteenths to count (a bar shorter than one).
    NoGrid,
    Keep(KeepError),
    /// Try again on picked notes: a beat's bars are tried again whole.
    PickBars,
}

impl DrumsRefused {
    pub fn words(&self) -> String {
        match self {
            DrumsRefused::NoClip => "The clip has gone. Nothing was written.".into(),
            DrumsRefused::Notes => {
                "This clip plays notes: Continue in the note editor carries a tune on".into()
            }
            DrumsRefused::Empty => "The clip has no hits yet: play or draw a beat first".into(),
            DrumsRefused::NoGrid => "The meter has no sixteenths to carry the beat on".into(),
            DrumsRefused::Keep(k) => k.words(),
            DrumsRefused::PickBars => {
                "On a beat, pick bars to try again: a hit can't be tried on its own".into()
            }
        }
    }
}

/// A hit of a bar: its sixteenth from the bar's start, how far off the
/// sixteenth it was played (its push or drag), its pad, loudness, length
/// and lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stroke {
    step: i32,
    off: Tick,
    pad: u8,
    velocity: u8,
    len: Tick,
    lane: Lane,
}

/// The pads your bars play in each lane, the most used first.
struct Kit {
    pads: [Vec<u8>; 4],
}

impl Kit {
    fn read(bars: &[Vec<Stroke>]) -> Kit {
        let mut counts: [Vec<(u8, usize)>; 4] = Default::default();
        for s in bars.iter().flatten() {
            let v = &mut counts[lane_index(s.lane)];
            match v.iter_mut().find(|(p, _)| *p == s.pad) {
                Some((_, n)) => *n += 1,
                None => v.push((s.pad, 1)),
            }
        }
        Kit {
            pads: counts.map(|mut v| {
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                v.into_iter().map(|(p, _)| p).collect()
            }),
        }
    }

    /// The lane's main pad.
    fn pad(&self, lane: Lane) -> Option<u8> {
        self.pads[lane_index(lane)].first().copied()
    }

    /// A pad for a fill's toms and rolls: percussion if you played any,
    /// else the snare.
    fn fill_pads(&self) -> Vec<(u8, Lane)> {
        let mut out: Vec<(u8, Lane)> = self.pads[lane_index(Lane::Other)]
            .iter()
            .take(3)
            .map(|&p| (p, Lane::Other))
            .collect();
        if let Some(s) = self.pad(Lane::Snare) {
            out.insert(0, (s, Lane::Snare));
        }
        out
    }
}

const fn lane_index(l: Lane) -> usize {
    match l {
        Lane::Kick => 0,
        Lane::Snare => 1,
        Lane::Hats => 2,
        Lane::Other => 3,
    }
}

/// Carry the drum clip on.
pub fn continue_drums(ask: &DrumsAsk) -> Result<Drummed, DrumsRefused> {
    let bar = ask.meter.bar_ticks();
    let steps = (bar / STEP) as i32;
    if steps < 1 {
        return Err(DrumsRefused::NoGrid);
    }
    if ask.hits.is_empty() {
        return Err(DrumsRefused::Empty);
    }
    let length = ask.length.max(1);
    let yours = ((length + bar - 1) / bar).max(1) as usize;
    let mut bars: Vec<Vec<Stroke>> = vec![Vec::new(); yours];
    let total = yours as Tick * Tick::from(steps as u32);
    for h in ask.hits {
        let at = h.at.rem_euclid(length);
        // The nearest sixteenth, and how far off it the hit was played; a
        // hit pushed past the clip's end is its first sixteenth's.
        let near = (at + STEP / 2) / STEP;
        let off = at - near * STEP;
        let step_abs = near % total;
        let b = (step_abs / Tick::from(steps as u32)) as usize;
        let step = (step_abs % Tick::from(steps as u32)) as i32;
        bars[b].push(Stroke {
            step,
            off,
            pad: h.pad,
            velocity: h.velocity.max(1),
            len: h.len.max(1),
            lane: (ask.lane_of)(h.pad).unwrap_or(Lane::Other),
        });
    }
    let kit = Kit::read(&bars);
    let s = ask.settings;
    let new = usize::from(s.bars.clamp(1, MOST_BARS));
    let mut plan: Vec<BarPlan> = (0..yours).map(|b| bar_plan(BarJob::Yours, b, s)).collect();
    let mut fills = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let beat = (steps / i32::from(ask.meter.beats.max(1))).max(1);
    for k in 0..new {
        let n = yours + k;
        let source = n % yours;
        let last = k + 1 == new;
        // A phrase's fill, but never right before a landing that rolls:
        // two fills back to back stumble.
        let lands_rolling = s.ending != Ending::Open;
        let phrase_end = (n + 1).is_multiple_of(4) && !(k + 2 == new && lands_rolling);
        let mut dice = DiceName {
            seed: s.seed,
            part: 0,
            phrase: (n / 4) as u16,
            job: Job::Drums,
            bar: n as u16,
            attempt: ask.attempt,
        }
        .dice();
        // How many bars come back as you played them: Similarity, and
        // never the bar before a fill or the last.
        let repeat = !last && !phrase_end && dice.chance(f64::from(s.similarity) * 0.6);
        let job = if last {
            BarJob::Land(s.ending)
        } else if repeat {
            BarJob::Repeat
        } else {
            BarJob::Vary
        };
        plan.push(bar_plan(job, source, s));
        let mut strokes = bars[source].clone();
        if !repeat {
            vary(&mut strokes, &kit, s, steps, beat, &mut dice);
        }
        let fill = match job {
            BarJob::Land(Ending::Open) => {
                // Left open: the last beat to the hats alone.
                strokes.retain(|x| x.step < steps - beat || x.lane == Lane::Hats);
                false
            }
            BarJob::Land(_) => true,
            _ => phrase_end,
        };
        if fill {
            fills.push(n);
            fill_bar(&mut strokes, &kit, s, steps, beat, &mut dice);
            if job == BarJob::Land(Ending::Home) {
                // Home: the kick on its last step closes it.
                if let Some(k) = kit.pad(Lane::Kick) {
                    strokes.retain(|x| !(x.step == steps - 1 && x.lane == Lane::Kick));
                    strokes.push(Stroke {
                        step: steps - 1,
                        off: 0,
                        pad: k,
                        velocity: 112,
                        len: HIT,
                        lane: Lane::Kick,
                    });
                }
            }
        }
        strokes.sort_by_key(|x| (x.step, x.pad));
        strokes.dedup_by_key(|x| (x.step, x.pad));
        let o = n as Tick * bar;
        for x in strokes {
            let at = (o + Tick::from(x.step) * STEP + x.off).max(o);
            notes.push(Note::new(at, x.len, x.pad, x.velocity));
        }
    }
    let to = (yours + new) as Tick * bar;
    notes.sort_by_key(|n| (n.tick, n.note));
    if notes.iter().any(|n| n.tick >= to) {
        return Err(DrumsRefused::Keep(KeepError::ClipTooLong));
    }
    Ok(Drummed {
        plan,
        fills,
        notes,
        to,
    })
}

/// Continue on the drum clip in `on`'s slot: its hits as they sound on
/// the grid (after its quantise, in its own ticks), each pad's lane as the
/// kit names it, carried on; with the proposal a preview plays and Keep
/// writes, bound to the clip at its revision. The row's other clips are
/// not read: a beat is carried on from itself.
pub fn continue_drum_clip(
    on: &crate::request::OnClip,
    settings: &Settings,
    attempt: u16,
) -> Result<(crate::preview::Proposal, Drummed), DrumsRefused> {
    let clip = on
        .project
        .clip(on.track, on.slot)
        .ok_or(DrumsRefused::NoClip)?;
    if clip.holds != Holds::Hits {
        return Err(DrumsRefused::Notes);
    }
    let length = clip.length.max(1);
    let hits: Vec<Hit> = clip
        .notes
        .iter()
        .filter(|n| n.on)
        .map(|n| Hit {
            at: clip
                .quantise
                .map_or(n.tick as f64, |q| q.apply(n.tick as f64))
                .round() as Tick,
            len: n.len,
            pad: n.note,
            velocity: n.velocity,
        })
        .collect();
    let track = on.track;
    let lane_of = |pad: u8| (on.lane_of)(track, pad);
    let made = continue_drums(&DrumsAsk {
        hits: &hits,
        length,
        lane_of: &lane_of,
        meter: on.meter,
        settings,
        attempt,
    })?;
    let edit =
        crate::keep::continuation(clip, made.notes.clone(), made.to).map_err(DrumsRefused::Keep)?;
    Ok((
        crate::preview::Proposal {
            edit,
            read: vec![ClipRead {
                clip: ClipRef(clip.id.0),
                revision: crate::bind::revision(clip),
                role: PartRole::Drums,
            }],
        },
        made,
    ))
}

/// Continue on a drum clip as Continue asks for a tune ([`Making`]): a
/// new beat; a new one with the locked bars of the one shown kept as they
/// were; or the picked bars of the one shown again, every other bar as it
/// was. Notes can't be picked on a beat. The answer is Continue's own, so
/// the preview, Keep, the tries and the bar jobs work as for a tune: it
/// has no tune and no chords.
pub fn continue_drum_clip_how(
    on: &crate::request::OnClip,
    settings: &Settings,
    kind: crate::profile::InstrumentKind,
    how: &Making,
) -> Result<(crate::preview::Proposal, Continued), Refused> {
    let (attempt, before, keep): (u16, Option<&Continued>, Vec<usize>) = match how {
        Making::New => (0, None, Vec::new()),
        Making::Keeping { before, locked } => (
            0,
            Some(before),
            locked.iter().map(|&b| usize::from(b)).collect(),
        ),
        Making::Bars {
            before,
            bars,
            attempt,
        } => {
            let again: Vec<usize> = bars.iter().map(|&b| usize::from(b)).collect();
            let keep = (0..before.plan.len())
                .filter(|b| !again.contains(b))
                .collect();
            (*attempt, Some(before), keep)
        }
        Making::Notes { .. } => return Err(Refused::Drums(DrumsRefused::PickBars)),
    };
    let (mut proposal, mut made) =
        continue_drum_clip(on, settings, attempt).map_err(Refused::Drums)?;
    if let Some(before) = before {
        let bar = on.meter.bar_ticks().max(1);
        let in_bar = |n: &Note, b: usize| n.tick / bar == b as Tick;
        let yours = made.plan.iter().filter(|p| p.job == BarJob::Yours).count();
        let kept: Vec<usize> = keep
            .into_iter()
            .filter(|&b| b >= yours && b < made.plan.len() && b < before.plan.len())
            .collect();
        if !kept.is_empty() {
            made.notes.retain(|n| !kept.iter().any(|&b| in_bar(n, b)));
            made.notes.extend(
                before
                    .notes
                    .iter()
                    .filter(|n| kept.iter().any(|&b| in_bar(n, b)) && n.tick < made.to)
                    .copied(),
            );
            made.notes.sort_by_key(|n| (n.tick, n.note));
            for &b in &kept {
                made.plan[b] = before.plan[b];
            }
            let clip = on.project.clip(on.track, on.slot).ok_or(Refused::NoClip)?;
            proposal.edit = crate::keep::continuation(clip, made.notes.clone(), made.to)?;
        }
    }
    Ok((proposal, made.continued(kind)))
}

impl Drummed {
    /// As Continue's answer: the bars and the notes, no tune, no chords.
    pub fn continued(self, kind: crate::profile::InstrumentKind) -> Continued {
        Continued {
            line: Line {
                notes: Vec::new(),
                similarity: vec![None; self.plan.len()],
                gave_way: Vec::new(),
                tried: 0,
            },
            plan: self.plan,
            chords: ChordTimeline::default(),
            part: Part {
                id: PartId(0),
                role: PartRole::Drums,
                kind,
                register: 36,
                decisions: Vec::new(),
            },
            events: Vec::new(),
            notes: self.notes,
            to: self.to,
            sketch: None,
            critic: Vec::new(),
        }
    }
}

fn bar_plan(job: BarJob, source: usize, s: &Settings) -> BarPlan {
    BarPlan {
        job,
        source,
        shift: 0,
        tension: match job {
            BarJob::Land(_) => 0.6,
            BarJob::Vary => 0.35,
            _ => 0.2,
        },
        activity: s.activity,
        similarity: match job {
            BarJob::Yours => 1.0,
            BarJob::Repeat => 1.0,
            _ => s.similarity,
        },
    }
}

/// What a drummer moves in a bar: the backbone stays, and with Variation
/// a kick moves a sixteenth or is added before a backbeat, a ghost snare
/// comes in, a percussion hit fills a gap; with Activity the hats get
/// busier (sixteenths between your eighths) or sparser (the off-beat
/// sixteenths go); loudness breathes a little.
fn vary(
    strokes: &mut Vec<Stroke>,
    kit: &Kit,
    s: &Settings,
    steps: i32,
    beat: i32,
    dice: &mut crate::dice::Dice,
) {
    let v = f64::from(s.variation.clamp(0.0, 1.0));
    let busy = f64::from(s.activity.clamp(0.0, 1.0)) - 0.5;
    let taken =
        |st: &[Stroke], step: i32, lane: Lane| st.iter().any(|x| x.step == step && x.lane == lane);
    let backbeat: Vec<i32> = strokes
        .iter()
        .filter(|x| x.lane == Lane::Snare && x.velocity >= 70)
        .map(|x| x.step)
        .collect();
    // A kick off the bar's first step moves a sixteenth either way.
    if dice.chance(v * 0.5) {
        let kicks: Vec<usize> = (0..strokes.len())
            .filter(|&i| strokes[i].lane == Lane::Kick && strokes[i].step != 0)
            .collect();
        if !kicks.is_empty() {
            let i = kicks[dice.below(kicks.len())];
            let to = strokes[i].step + if dice.chance(0.5) { 1 } else { -1 };
            if (1..steps).contains(&to)
                && !taken(strokes, to, Lane::Kick)
                && !backbeat.contains(&to)
            {
                strokes[i].step = to;
            }
        }
    }
    // A kick on the sixteenth before a backbeat.
    if let Some(k) = kit.pad(Lane::Kick) {
        if dice.chance(v * 0.4) && !backbeat.is_empty() {
            let b = backbeat[dice.below(backbeat.len())];
            let at = b - 1;
            if at > 0 && !taken(strokes, at, Lane::Kick) {
                strokes.push(Stroke {
                    step: at,
                    off: 0,
                    pad: k,
                    velocity: 84,
                    len: HIT,
                    lane: Lane::Kick,
                });
            }
        }
    }
    // A ghost snare on an off-beat sixteenth.
    if let Some(sn) = kit.pad(Lane::Snare) {
        if dice.chance(v * 0.5) {
            let free: Vec<i32> = (0..steps)
                .filter(|st| st % 2 == 1 && !taken(strokes, *st, Lane::Snare))
                .collect();
            if !free.is_empty() {
                strokes.push(Stroke {
                    step: free[dice.below(free.len())],
                    off: 0,
                    pad: sn,
                    velocity: 40 + dice.below(16) as u8,
                    len: HIT,
                    lane: Lane::Snare,
                });
            }
        }
    }
    // Hats: busier or sparser than yours.
    if let Some(h) = kit.pad(Lane::Hats) {
        let hats: Vec<i32> = strokes
            .iter()
            .filter(|x| x.lane == Lane::Hats)
            .map(|x| x.step)
            .collect();
        if busy > 0.05 && !hats.is_empty() {
            for st in 0..steps {
                if st % 2 == 1
                    && !hats.contains(&st)
                    && hats.contains(&(st - 1))
                    && dice.chance(busy * 1.2)
                {
                    strokes.push(Stroke {
                        step: st,
                        off: 0,
                        pad: h,
                        velocity: 50 + dice.below(14) as u8,
                        len: HIT,
                        lane: Lane::Hats,
                    });
                }
            }
        } else if busy < -0.05 {
            strokes.retain(|x| {
                x.lane != Lane::Hats || x.step % beat == 0 || !dice.chance(-busy * 1.4)
            });
        }
    }
    // Percussion in a gap, on a weak sixteenth.
    if let Some(p) = kit.pad(Lane::Other) {
        if dice.chance(v * 0.4 + busy.max(0.0) * 0.4) {
            let free: Vec<i32> = (0..steps)
                .filter(|st| {
                    st % beat != 0
                        && !strokes
                            .iter()
                            .any(|x| x.step == *st && x.lane != Lane::Hats)
                })
                .collect();
            if !free.is_empty() {
                strokes.push(Stroke {
                    step: free[dice.below(free.len())],
                    off: 0,
                    pad: p,
                    velocity: 56 + dice.below(20) as u8,
                    len: HIT,
                    lane: Lane::Other,
                });
            }
        }
    }
    // Loudness breathes: a few points either way, more with Variation.
    let spread = (2.0 + v * 6.0) as i32;
    for x in strokes.iter_mut() {
        let d = dice.below((2 * spread + 1) as usize) as i32 - spread;
        x.velocity = (i32::from(x.velocity) + d).clamp(1, 127) as u8;
    }
}

/// A fill over the bar's last beat (Minimal: its last half beat): what
/// was there goes but the kick on the beat, and the snare and your
/// percussion roll in sixteenths, louder to the end.
fn fill_bar(
    strokes: &mut Vec<Stroke>,
    kit: &Kit,
    s: &Settings,
    steps: i32,
    beat: i32,
    dice: &mut crate::dice::Dice,
) {
    let pads = kit.fill_pads();
    if pads.is_empty() {
        return;
    }
    let span = if s.style.name == crate::style::MINIMAL.name {
        (beat / 2).max(1)
    } else {
        beat
    };
    let from = steps - span;
    strokes.retain(|x| x.step < from || (x.step == from && x.lane == Lane::Kick));
    let n = span as usize;
    let mut k = dice.below(pads.len());
    for i in 0..n {
        let (pad, lane) = pads[k % pads.len()];
        // Down the toms (or on the snare), a step at a time.
        if pads.len() > 1 && dice.chance(0.6) {
            k += 1;
        }
        let rise = if n > 1 { 40 * i / (n - 1) } else { 40 };
        strokes.push(Stroke {
            step: from + i as i32,
            off: 0,
            pad,
            velocity: (72 + rise) as u8,
            len: HIT,
            lane,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{GROOVE, MINIMAL};

    const BAR: Tick = 16 * STEP;
    const FOUR: Meter = Meter { beats: 4, unit: 4 };
    const KICK: u8 = 0;
    const SNARE: u8 = 1;
    const HAT: u8 = 2;
    const TOM: u8 = 3;

    fn lanes(pad: u8) -> Option<Lane> {
        match pad {
            KICK => Some(Lane::Kick),
            SNARE => Some(Lane::Snare),
            HAT => Some(Lane::Hats),
            _ => None,
        }
    }

    /// Two bars: kick on 1 and the "and" of 3, snare on 2 and 4, eighth
    /// hats, a tom at the end of bar 2; the hats played a little late.
    fn beat() -> Vec<Hit> {
        let mut v = Vec::new();
        for b in 0..2 {
            let o = b * BAR;
            for st in [0, 10] {
                v.push(Hit {
                    at: o + st * STEP,
                    len: HIT,
                    pad: KICK,
                    velocity: 110,
                });
            }
            for st in [4, 12] {
                v.push(Hit {
                    at: o + st * STEP,
                    len: HIT,
                    pad: SNARE,
                    velocity: 100,
                });
            }
            for st in (0..16).step_by(2) {
                v.push(Hit {
                    at: o + st * STEP + 12,
                    len: HIT,
                    pad: HAT,
                    velocity: 80,
                });
            }
        }
        v.push(Hit {
            at: BAR + 15 * STEP,
            len: HIT,
            pad: TOM,
            velocity: 90,
        });
        v
    }

    fn settings(bars: u8) -> Settings {
        Settings {
            style: GROOVE,
            bars,
            ..Settings::default()
        }
    }

    fn ask<'a>(hits: &'a [Hit], s: &'a Settings, attempt: u16) -> DrumsAsk<'a> {
        DrumsAsk {
            hits,
            length: 2 * BAR,
            lane_of: &lanes,
            meter: FOUR,
            settings: s,
            attempt,
        }
    }

    fn bar_of(d: &Drummed, b: usize) -> Vec<&Note> {
        let o = b as Tick * BAR;
        d.notes
            .iter()
            .filter(|n| (o..o + BAR).contains(&n.tick))
            .collect()
    }

    /// Six bars on two: every new bar keeps the backbone (kick on 1, snare
    /// on 2 and 4), plays only your pads, keeps the hats' lateness; bar 4
    /// ends with a fill and the last lands home with its kick on the last
    /// step; the clip grows to eight bars.
    #[test]
    fn your_beat_carries_on_with_its_backbone_fills_and_a_landing() {
        let hits = beat();
        let s = settings(6);
        let d = continue_drums(&ask(&hits, &s, 0)).unwrap();
        assert_eq!(d.to, 8 * BAR);
        assert_eq!(d.plan.len(), 8);
        assert!(d.plan[..2].iter().all(|p| p.job == BarJob::Yours));
        assert_eq!(d.plan[7].job, BarJob::Land(Ending::Home));
        assert_eq!(d.fills, [3, 7]);
        assert!(d
            .notes
            .iter()
            .all(|n| n.tick >= 2 * BAR && n.tick < 8 * BAR));
        assert!(d
            .notes
            .iter()
            .all(|n| [KICK, SNARE, HAT, TOM].contains(&n.note)));
        for b in 2..8 {
            let notes = bar_of(&d, b);
            let o = b as Tick * BAR;
            let on = |pad: u8, st: Tick| {
                notes
                    .iter()
                    .any(|n| n.note == pad && n.tick == o + st * STEP)
            };
            assert!(on(KICK, 0), "bar {b}: the kick on 1");
            assert!(on(SNARE, 4), "bar {b}: the snare on 2");
            if !d.fills.contains(&b) {
                assert!(on(SNARE, 12), "bar {b}: the snare on 4");
                assert!(
                    notes
                        .iter()
                        .filter(|n| n.note == HAT)
                        .all(|n| (n.tick - o) % STEP == 12 || (n.tick - o) % (2 * STEP) != 0),
                    "bar {b}: your hats keep their lateness"
                );
            }
        }
        let last = bar_of(&d, 7);
        assert!(last
            .iter()
            .any(|n| n.note == KICK && n.tick == 7 * BAR + 15 * STEP));
        // The fill: the last beat rolls in sixteenths, louder to the end.
        let fill: Vec<&Note> = bar_of(&d, 3)
            .into_iter()
            .filter(|n| n.tick >= 3 * BAR + 12 * STEP && n.note != KICK)
            .collect();
        assert_eq!(fill.len(), 4);
        assert!(fill.windows(2).all(|w| w[0].velocity <= w[1].velocity));
        assert!(fill.iter().all(|n| n.note == SNARE || n.note == TOM));
    }

    /// The same ask gives the same bars; Try again gives others; Similarity
    /// all the way up brings more bars back as you played them, and Minimal's
    /// fill is half a beat.
    #[test]
    fn try_again_varies_and_similarity_brings_your_bars_back() {
        let hits = beat();
        let s = settings(8);
        let a = continue_drums(&ask(&hits, &s, 0)).unwrap();
        assert_eq!(a, continue_drums(&ask(&hits, &s, 0)).unwrap());
        let b = continue_drums(&ask(&hits, &s, 1)).unwrap();
        assert_ne!(a.notes, b.notes);
        let repeats = |sim: f32| {
            (0..20u16)
                .map(|t| {
                    let s = Settings {
                        similarity: sim,
                        ..settings(8)
                    };
                    continue_drums(&ask(&hits, &s, t))
                        .unwrap()
                        .plan
                        .iter()
                        .filter(|p| p.job == BarJob::Repeat)
                        .count()
                })
                .sum::<usize>()
        };
        assert!(
            repeats(1.0) > repeats(0.0) + 20,
            "{} {}",
            repeats(1.0),
            repeats(0.0)
        );
        let minimal = Settings {
            style: MINIMAL,
            ..settings(2)
        };
        let d = continue_drums(&ask(&hits, &minimal, 0)).unwrap();
        let fill = bar_of(&d, 3)
            .into_iter()
            .filter(|n| n.tick >= 3 * BAR + 14 * STEP && n.note != KICK)
            .count();
        assert_eq!(fill, 2, "half a beat");
        assert!(bar_of(&d, 3)
            .iter()
            .any(|n| n.note == HAT && n.tick == 3 * BAR + 12 * STEP + 12));
    }

    /// Activity up adds hats between your eighths, down thins them to the
    /// beats; Open leaves the last beat to the hats; Loops back ends on a
    /// fill; an empty clip is refused.
    #[test]
    fn activity_shapes_the_hats_and_the_ending_closes_it() {
        let hits = beat();
        let hats = |act: f32| {
            let s = Settings {
                activity: act,
                variation: 0.0,
                similarity: 0.0,
                ..settings(4)
            };
            continue_drums(&ask(&hits, &s, 0))
                .unwrap()
                .notes
                .iter()
                .filter(|n| n.note == HAT)
                .count()
        };
        assert!(hats(1.0) > hats(0.5), "{} {}", hats(1.0), hats(0.5));
        assert!(hats(0.0) < hats(0.5), "{} {}", hats(0.0), hats(0.5));
        let open = Settings {
            ending: Ending::Open,
            ..settings(2)
        };
        let d = continue_drums(&ask(&hits, &open, 0)).unwrap();
        assert!(bar_of(&d, 3)
            .iter()
            .filter(|n| n.tick >= 3 * BAR + 12 * STEP)
            .all(|n| n.note == HAT));
        assert!(!d.fills.contains(&3));
        let back = Settings {
            ending: Ending::LoopsBack,
            ..settings(2)
        };
        let d = continue_drums(&ask(&hits, &back, 0)).unwrap();
        assert_eq!(d.plan[3].job, BarJob::Land(Ending::LoopsBack));
        assert!(d.fills.contains(&3));
        // Three new bars land on bar 5: bar 4 keeps its groove, so the
        // fills don't come back to back; left open, bar 4 has its fill.
        let d = continue_drums(&ask(&hits, &settings(3), 0)).unwrap();
        assert_eq!(d.fills, [4]);
        let open = Settings {
            ending: Ending::Open,
            ..settings(3)
        };
        assert_eq!(continue_drums(&ask(&hits, &open, 0)).unwrap().fills, [3]);
        assert_eq!(
            continue_drums(&ask(&[], &settings(2), 0)).unwrap_err(),
            DrumsRefused::Empty
        );
    }

    /// On a clip: its hits read after its quantise, its pads' lanes from
    /// the kit, the proposal adds the new bars and grows the clip, bound to
    /// the clip; a clip of notes is refused with where to go instead.
    #[test]
    fn a_drum_clip_is_carried_on_as_a_proposal() {
        use mpfree_seq::{Clip, Project, Track};
        let mut project = Project::new();
        let mut t = Track::new("Drums", mpfree_seq::DestId::default());
        let mut c = Clip::new("Beat", 2 * BAR);
        c.holds = Holds::Hits;
        c.notes = beat()
            .iter()
            .map(|h| Note::new(h.at, h.len, h.pad, h.velocity))
            .collect();
        t.slots[0] = Some(c);
        project.tracks.push(t);
        project.give_clip_ids();
        let lane_of = |_t: usize, pad: u8| lanes(pad);
        let on = |project| crate::request::OnClip {
            project,
            track: 0,
            slot: 0,
            role: PartRole::Drums,
            others: &[],
            lane_of: &lane_of,
            transport_swing: Default::default(),
            meter: FOUR,
            bpm: 120.0,
            budget: Default::default(),
            row: None,
        };
        let s = settings(2);
        let (p, d) = continue_drum_clip(&on(&project), &s, 0).unwrap();
        assert_eq!(p.edit.length, Some((2 * BAR, 4 * BAR)));
        assert_eq!(p.edit.add, d.notes);
        assert_eq!(p.read.len(), 1);
        assert_eq!(p.read[0].role, PartRole::Drums);
        let mut notes = project.clone();
        notes.tracks[0].slots[0].as_mut().unwrap().holds = Holds::Notes;
        let why = continue_drum_clip(&on(&notes), &s, 0).unwrap_err();
        assert_eq!(why, DrumsRefused::Notes);
        assert!(why.words().contains("note editor"));
    }

    /// Continue's own path carries a drum clip on: the same answer as a
    /// tune's, with no tune and no chords; picked bars come again with the
    /// others as they were, locked bars stay through a new try, and picked
    /// notes are refused with what to pick instead.
    #[test]
    fn continue_on_a_drum_clip_answers_as_for_a_tune() {
        use crate::request::continue_clip_how;
        use mpfree_seq::{Clip, Project, Track};
        use std::sync::Arc;
        let mut project = Project::new();
        let mut t = Track::new("Drums", mpfree_seq::DestId::default());
        let mut c = Clip::new("Beat", 2 * BAR);
        c.holds = Holds::Hits;
        c.notes = beat()
            .iter()
            .map(|h| Note::new(h.at, h.len, h.pad, h.velocity))
            .collect();
        t.slots[0] = Some(c);
        project.tracks.push(t);
        project.give_clip_ids();
        let lane_of = |_t: usize, pad: u8| lanes(pad);
        let on = crate::request::OnClip {
            project: &project,
            track: 0,
            slot: 0,
            role: PartRole::Drums,
            others: &[],
            lane_of: &lane_of,
            transport_swing: Default::default(),
            meter: FOUR,
            bpm: 120.0,
            budget: Default::default(),
            row: None,
        };
        let kind = crate::profile::InstrumentKind::Percussive;
        let s = settings(4);
        let (p, made) = continue_clip_how(&on, &s, kind, &Making::New).0.unwrap();
        assert_eq!(made.to, 6 * BAR);
        assert_eq!(p.edit.add, made.notes);
        assert_eq!(made.part.role, PartRole::Drums);
        assert!(made.chords.spans().is_empty() && made.sketch.is_none());
        assert_eq!(made.plan[5].job, BarJob::Land(Ending::Home));
        let before = Arc::new(made);
        let bar_notes = |m: &Continued, b: Tick| -> Vec<Note> {
            m.notes
                .iter()
                .filter(|n| n.tick / BAR == b)
                .copied()
                .collect()
        };
        // Bar 3 again: it changes, bars 2, 4 and 5 stay as they were.
        let again = Making::Bars {
            before: before.clone(),
            bars: vec![3],
            attempt: 4,
        };
        let (p, m) = continue_clip_how(&on, &s, kind, &again).0.unwrap();
        assert_eq!(p.edit.add, m.notes);
        assert_ne!(bar_notes(&m, 3), bar_notes(&before, 3));
        for b in [2, 4, 5] {
            assert_eq!(bar_notes(&m, b), bar_notes(&before, b), "bar {b}");
        }
        // A new try with bar 2 locked: bar 2 stays, the rest come new.
        let s2 = Settings {
            seed: 99,
            ..settings(4)
        };
        let keeping = Making::Keeping {
            before: before.clone(),
            locked: vec![2],
        };
        let (_, m) = continue_clip_how(&on, &s2, kind, &keeping).0.unwrap();
        assert_eq!(bar_notes(&m, 2), bar_notes(&before, 2));
        assert_ne!(m.notes, before.notes);
        let notes = Making::Notes {
            before,
            notes: vec![(2 * BAR, KICK)],
            attempt: 1,
        };
        let why = continue_clip_how(&on, &s, kind, &notes).0.unwrap_err();
        assert_eq!(why, Refused::Drums(DrumsRefused::PickBars));
        assert!(why.words().contains("pick bars"));
    }
}
