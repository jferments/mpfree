//! Drafts and the small critic (revision 3, S1): Continue made music
//! first.
//!
//! Each draft follows one phrase plan ([`crate::sketch`]): the chords are
//! led to its landings ([`progression_led`]), the chord under each anchor
//! picks its exact note, and every bar's notes are filled in around the
//! anchors by the same beam search as before ([`fit`]), aiming at your
//! idea as the bar's gesture reshapes it. No note but the high point
//! climbs as high as it, the rests stay silent, and an outside note
//! appears only with a job: a half step leading into the high point or
//! the last note, where the style uses them.
//!
//! The critic ([`crate::critic`]) judges every draft whole, and the one
//! that passes most checks is offered. Drafts are played without the
//! first engine's random drift: feel comes from the clip's groove and the
//! playing rules.
//!
//! Locked bars stay exactly as they were in every draft
//! ([`draft_keeping`]); Try again on picked bars fills them again from the
//! result's plan, each keeping its anchor's note ([`try_bars_s1`]).

use mpfree_context::chord::{ChordSpan, ChordTimeline, Heard as ChordHeard};
use mpfree_context::groove::STEP;
use mpfree_transport::time::Tick;

use crate::critic::{copy_bars, judge, Heard, Landing, Verdict};
use crate::dice::{Dice, Job};
use crate::keep::clip_notes;
use crate::motif::{bar_of, motifs, transform, GridNote, Transform};
use crate::phrase::{fit, BarOut, Line, NoteAs, Phrasing, Planned};
use crate::plan::BarPlan;
use crate::progression::{progression_led, Need};
use crate::request::{
    chords_in_bar, perform_with, phrasing, splice, your_bars, Continue, Continued, Refused,
};
use crate::rhythm::{density_to, feel, rhythm_of, vary, Grid, Onset};
use crate::settings::Settings;
use crate::sketch::{borrowed, sketch, Anchor, AnchorKind, Gesture, Sketch, Surprise};
use crate::style::Feel;

/// Whole drafts made for one request of eight bars or fewer (S4: up to
/// 12, the critic picking the best and repairing it).
pub const DRAFTS: u16 = 12;
/// Drafts made for a request of more than eight bars.
pub const DRAFTS_LONG: u16 = 4;
/// The runner-up kept second differs from the best in at least this share
/// of its new notes (S4: finalists kept different).
pub const DIFFER: f32 = 0.3;
/// Fresh fillings tried on each bar while repairing.
const REPAIR_ATTEMPTS: u16 = 3;
/// At most this many repairs on the picked draft.
const REPAIRS: usize = 2;
/// Repair's dice start well past Try again's, so a repair is never one of
/// your tries.
const REPAIR_DICE: u16 = 512;

/// One draft, judged.
#[derive(Clone, Debug)]
pub struct Draft {
    pub number: u16,
    pub sketch: Sketch,
    pub made: Continued,
    pub verdict: Verdict,
}

/// An anchor with its note: where, how long, which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fixed {
    kind: AnchorKind,
    bar: usize,
    on: i32,
    len: i32,
    pos: i32,
    alt: i8,
}

/// Carry the clip on, music first: the best of the drafts.
pub fn continue_s1(req: &Continue) -> Result<Continued, Refused> {
    continue_s1_keeping(req, None)
}

/// [`continue_s1`], keeping new bars of an earlier result (see
/// [`draft_keeping`]).
pub fn continue_s1_keeping(
    req: &Continue,
    keeping: Option<(&Continued, &[usize])>,
) -> Result<Continued, Refused> {
    let mut all = drafts_keeping(req, keeping)?;
    Ok(all.remove(0).made)
}

/// Every draft for `req`, best first (most checks passed, then the
/// highest score, then the earliest).
pub fn drafts(req: &Continue) -> Result<Vec<Draft>, Refused> {
    drafts_keeping(req, None)
}

/// [`drafts`], keeping new bars of an earlier result.
pub fn drafts_keeping(
    req: &Continue,
    keeping: Option<(&Continued, &[usize])>,
) -> Result<Vec<Draft>, Refused> {
    let n = usize::from(req.settings.bars.max(1));
    let count = if n > 8 { DRAFTS_LONG } else { DRAFTS };
    let mut out: Vec<Draft> = Vec::new();
    let mut first_err: Option<Refused> = None;
    for d in 0..count {
        match draft_keeping(req, d, keeping) {
            Ok(x) => out.push(x),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    if out.is_empty() {
        return Err(first_err.unwrap_or(Refused::NoClip));
    }
    out.sort_by(|a, b| {
        b.verdict
            .passed()
            .cmp(&a.verdict.passed())
            .then(b.verdict.score().total_cmp(&a.verdict.score()))
            .then(a.number.cmp(&b.number))
    });
    let yours = your_bars(req);
    let from = yours as i32 * phrasing(req, &ChordTimeline::default()).grid.bar;
    finalists(&mut out, from);
    let kept = kept_bars(keeping, yours, yours + n);
    if let Some(best) = out.first_mut() {
        repair(req, best, &kept);
    }
    Ok(out)
}

/// How far two results' new notes (from step `from`) differ: the share of
/// the larger set of notes, by place and pitch, the other doesn't have.
pub fn difference(a: &Continued, b: &Continued, from: i32) -> f32 {
    let notes = |c: &Continued| -> Vec<(i32, i32, i8)> {
        let mut v: Vec<(i32, i32, i8)> = c
            .line
            .notes
            .iter()
            .filter(|m| m.on >= from)
            .map(|m| (m.on, m.pos, m.alt))
            .collect();
        v.sort_unstable();
        v
    };
    let (x, y) = (notes(a), notes(b));
    let most = x.len().max(y.len());
    if most == 0 {
        return 0.0;
    }
    let shared = x.iter().filter(|n| y.binary_search(n).is_ok()).count();
    1.0 - shared as f32 / most as f32
}

/// The best draft stays first; the best of the rest that differs from it
/// by [`DIFFER`] or more comes second, so a runner-up is a real choice.
fn finalists(out: &mut Vec<Draft>, from: i32) {
    if out.len() < 3 {
        return;
    }
    if let Some(j) =
        (1..out.len()).find(|&j| difference(&out[0].made, &out[j].made, from) >= DIFFER)
    {
        if j != 1 {
            let d = out.remove(j);
            out.insert(1, d);
        }
    }
}

/// The critic's verdict on a result made from `sk` (a draft, or a draft
/// with bars made again): its anchors' notes as the landings.
pub(crate) fn verdict_for(req: &Continue, made: &Continued, sk: &Sketch) -> Verdict {
    let s = req.settings;
    let ph = phrasing(req, &made.chords);
    let landings: Vec<Landing> = sk
        .anchors
        .iter()
        .filter_map(|a| {
            let m = made.line.notes.iter().find(|m| m.on == a.on)?;
            Some(Landing {
                on: a.on,
                pos: m.pos,
                alt: m.alt,
            })
        })
        .collect();
    judge(&Heard {
        notes: &made.line.notes,
        your_bars: your_bars(req),
        bars: your_bars(req) + usize::from(s.bars.max(1)),
        grid: ph.grid,
        key: ph.key,
        chords: &made.chords,
        ending: s.ending,
        copy_bars: copy_bars(&s.style),
        landings: &landings,
        rests: if s.keeps.rhythm { &[] } else { &sk.rests },
    })
}

/// A check's name as the critic's words say it was put right.
fn fixed_words(check: &str) -> &'static str {
    match check {
        "goes somewhere" => "it goes somewhere",
        "lands" => "it lands",
        "breathes" => "it breathes",
        "yours" => "your idea is heard",
        "not too repetitive" => "it no longer repeats",
        _ => "it flows",
    }
}

/// Repair the picked draft in place (S4): where a check fails, each new
/// bar but those you locked is made again a few ways around its anchor
/// ([`try_bars_s1`]), and the one that puts most right is kept, so long as
/// every check that passed still passes. At most [`REPAIRS`] repairs, each
/// said in words on the result ([`Continued::critic`]).
fn repair(req: &Continue, d: &mut Draft, kept: &[usize]) {
    let yours = your_bars(req);
    let total = yours + usize::from(req.settings.bars.max(1));
    for _ in 0..REPAIRS {
        let was: Vec<bool> = d.verdict.checks().iter().map(|(_, c)| c.pass).collect();
        if was.iter().all(|&p| p) {
            return;
        }
        let mut found: Option<(usize, Continued, Verdict)> = None;
        for b in (yours..total).filter(|b| !kept.contains(b)) {
            for k in 0..REPAIR_ATTEMPTS {
                let dice = REPAIR_DICE + (b as u16) * REPAIR_ATTEMPTS + k;
                let Ok(c) = try_bars_s1(req, &d.made, &d.sketch, &[b], dice) else {
                    continue;
                };
                let v = verdict_for(req, &c, &d.sketch);
                let now: Vec<bool> = v.checks().iter().map(|(_, c)| c.pass).collect();
                let keeps = was.iter().zip(&now).all(|(&a, &b)| !a || b);
                let best = found.as_ref().map_or(&d.verdict, |f| &f.2);
                let more = v.passed() > best.passed()
                    || (found.is_some() && v.passed() == best.passed() && v.score() > best.score());
                if keeps && v.passed() > d.verdict.passed() && more {
                    found = Some((b, c, v));
                }
            }
        }
        let Some((b, mut c, v)) = found else {
            return;
        };
        let put_right: Vec<&str> = d
            .verdict
            .checks()
            .iter()
            .zip(v.checks().iter())
            .filter(|((_, a), (_, b))| !a.pass && b.pass)
            .map(|((name, _), _)| fixed_words(name))
            .collect();
        c.critic = d.made.critic.clone();
        c.critic.push(format!(
            "bar {} made again so {}",
            b + 1,
            put_right.join(" and ")
        ));
        d.made = c;
        d.verdict = v;
    }
}

/// The ceiling for the notes of every bar but the high point's: a step
/// under it, as long as that leaves room.
fn with_ceiling<'a>(ph: &Phrasing<'a>, top: Option<u8>) -> Phrasing<'a> {
    let mut p = ph.clone();
    if let Some(t) = top {
        if t >= p.profile.low.saturating_add(7) {
            p.profile.high = p.profile.high.min(t);
        }
    }
    p
}

/// The note for an anchor: among the notes in range (under `top`, and
/// from `floor` up when that leaves any), the one nearest its aim that
/// sounds in the chord under it, preferring its scale degrees.
fn resolve(
    ph: &Phrasing,
    settings: &Settings,
    a: &Anchor,
    top: Option<u8>,
    floor: Option<i32>,
) -> Option<(i32, i8)> {
    let p = with_ceiling(ph, top);
    let (lo, hi) = p.pitches(settings.range)?;
    let floor = floor.filter(|&f| ph.key.position(hi).0 >= f);
    let tick = Tick::from(a.on) * STEP;
    let chord = ph.chords.span_at(tick).and_then(|s| s.answer.chord());
    let played = ph.played_pcs & 0x0FFF;
    (lo..=hi)
        .map(|pitch| ph.key.position(pitch))
        .filter(|&(pos, alt)| alt == 0 && floor.is_none_or(|f| pos >= f))
        .map(|(pos, alt)| {
            let pitch = ph.key.pitch_at(pos, alt);
            let degree = pos.rem_euclid(7);
            let rank = a
                .notes
                .iter()
                .position(|&d| d == degree)
                .map_or(2.0, |r| r as f64 * 0.6);
            let off_chord = chord.is_some_and(|c| c.tone_of(i32::from(pitch)).is_none());
            let unplayed = !ph.clear && played != 0 && played & (1 << (pitch % 12)) == 0;
            let cost = f64::from((pos - a.aim).abs())
                + rank
                + if off_chord { 6.0 } else { 0.0 }
                + if unplayed { 10.0 } else { 0.0 };
            (cost, pos, alt)
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, pos, alt)| (pos, alt))
}

/// Notes with their target: the last note of `tune` starting at or
/// before each onset.
fn aimed(rhythm: &[Onset], tune: &[GridNote]) -> Vec<Planned> {
    rhythm
        .iter()
        .map(|r| Planned {
            on: r.on,
            len: r.len,
            target: tune
                .iter()
                .rev()
                .find(|m| m.on <= r.on)
                .or(tune.first())
                .copied(),
        })
        .collect()
}

/// Your bar's rhythm, varied by `share` of the bar's amount (when
/// `vary_it`), given the style's feel and made as busy as the bar asks;
/// as yours with Keep Rhythm.
fn reshaped(
    ph: &Phrasing,
    bp: &BarPlan,
    settings: &Settings,
    tune: &[GridNote],
    share: f64,
    vary_it: bool,
    dice: &mut Dice,
) -> Vec<Onset> {
    let style = ph.style;
    let grid = ph.grid;
    let aim = f64::from(bp.similarity);
    let act = f64::from(bp.activity);
    let mut r = rhythm_of(tune, grid);
    if settings.keeps.rhythm {
        return r;
    }
    let amount = (f64::from(style.development.variation) + f64::from(settings.variation) * 0.5)
        * (1.0 - aim).max(0.08)
        * 2.0;
    if vary_it {
        vary(&mut r, amount * share, act, grid, dice);
    }
    feel(&style.rhythm, &mut r, act, aim, grid, dice);
    if style.rhythm.feel != Feel::Simplify && !tune.is_empty() {
        let want = tune.len() as f64 * act;
        let (lo, hi) = (
            (want * 0.85).floor() as usize,
            (want * 1.15).ceil() as usize,
        );
        let unit = if style.rhythm.feel == Feel::Swing8 {
            2
        } else {
            1
        };
        if r.len() > hi {
            density_to(&mut r, hi, unit, grid, dice);
        } else if r.len() < lo {
            density_to(&mut r, lo, unit, grid, dice);
        }
    }
    r
}

fn note(on: i32, len: i32, pos: i32, alt: i8, velocity: u8) -> GridNote {
    GridNote {
        on,
        len,
        pos,
        alt,
        velocity,
    }
}

/// One new bar's planned notes and which are held to a note, from its
/// gesture. `anchor` is the bar's anchor, from the bar's start.
#[allow(clippy::too_many_arguments)]
fn material(
    ph: &Phrasing,
    bp: &BarPlan,
    g: Gesture,
    src: &[GridNote],
    head: &[GridNote],
    anchor: Option<Fixed>,
    next_first: Option<i32>,
    settings: &Settings,
    dice: &mut Dice,
) -> (Vec<Planned>, Vec<NoteAs>) {
    let grid = ph.grid;
    let beat = grid.beat;
    let eighth = (beat / 2).max(1);
    let velocity = src
        .first()
        .map_or(ph.style.playing.velocity, |m| m.velocity);
    let mut planned: Vec<Planned> = match (g, anchor) {
        (_, Some(a)) => {
            // Your bar's rhythm up to the anchor, your tune moved so it
            // arrives a step away (from below rising to a landing or the
            // high point, from above coming home), then the anchor.
            let rising = a.kind != AnchorKind::Close;
            let mut r: Vec<Onset> = reshaped(ph, bp, settings, src, 0.5, true, dice)
                .into_iter()
                .filter(|o| o.on < a.on)
                .collect();
            if let Some(l) = r.last_mut() {
                l.len = l.len.min(a.on - l.on).max(1);
            }
            let tune = match r.last() {
                Some(last) => {
                    let at = src
                        .iter()
                        .rev()
                        .find(|m| m.on <= last.on)
                        .or(src.first())
                        .map_or(a.pos, |m| m.pos);
                    let into = a.pos + if rising { -1 } else { 1 };
                    transform(src, Transform::Move(into - at))
                }
                None => Vec::new(),
            };
            let mut v = aimed(&r, &tune);
            v.push(Planned {
                on: a.on,
                len: a.len,
                target: Some(note(a.on, a.len, a.pos, a.alt, velocity)),
            });
            let after = a.on + a.len;
            match g {
                Gesture::Reach if after + eighth < grid.bar - eighth => {
                    // A pickup after a breath: two notes (one in a sparse
                    // style) stepping down from the landing towards where
                    // the next bar starts, in your opening's rhythm.
                    let from = after + eighth;
                    let o = head.first().map_or(0, |m| m.on);
                    let count = if ph.style.rhythm.feel == Feel::Simplify {
                        1
                    } else {
                        2
                    };
                    let lead: Vec<GridNote> = transform(head, Transform::Shift(from - o))
                        .into_iter()
                        .filter(|m| m.on < grid.bar)
                        .take(count)
                        .enumerate()
                        .map(|(k, m)| GridNote {
                            pos: a.pos - 1 - k as i32,
                            alt: 0,
                            ..m
                        })
                        .collect();
                    v.extend(lead.iter().map(|m| Planned {
                        on: m.on,
                        len: m.len.min(grid.bar - m.on).max(1),
                        target: Some(*m),
                    }));
                }
                Gesture::Peak => {
                    // A beat of silence, then the way down if there's room,
                    // towards the next bar's first note when it is known.
                    let goal = next_first.map_or(a.pos - 4, |p| p + 1).min(a.pos - 2);
                    let room = ((grid.bar - after - beat) / beat).clamp(0, 2);
                    let from = a.pos - 1;
                    for k in 0..room {
                        let at = after + beat * (k + 1);
                        let len = beat.min(grid.bar - at);
                        let pos = from + (goal - from) * (k + 1) / room.max(1);
                        v.push(Planned {
                            on: at,
                            len,
                            target: Some(note(at, len, pos, 0, velocity)),
                        });
                    }
                }
                _ => {}
            }
            v
        }
        (Gesture::Answer, None) | (Gesture::Settle, None) => {
            // The call's tune a step away, ending on another note than the
            // call did.
            let call_end = src.last().map_or(0, |m| m.pos.rem_euclid(7));
            let degree = if call_end == 0 { 4 } else { 0 };
            let tune = transform(src, Transform::Move(bp.shift));
            let near = tune.last().map_or(ph.centre, |m| m.pos);
            let below = (near - degree).rem_euclid(7);
            let end = near - below + if below > 3 { 7 } else { 0 };
            let tune = transform(&tune, Transform::EndOn(end));
            let r = reshaped(ph, bp, settings, src, 0.5, true, dice);
            aimed(&r, &tune)
        }
        (Gesture::Develop, None) => {
            // Your opening twice, the second time a step higher.
            let half = grid.half();
            let mut h: Vec<GridNote> = head.iter().filter(|m| m.on < half).copied().collect();
            if h.len() < 2 {
                h = src.iter().filter(|m| m.on < half).copied().collect();
            }
            if h.last().is_some_and(|m| m.on + m.len > half) {
                h = transform(&h, Transform::Diminish);
            }
            let o = h.first().map_or(0, |m| m.on);
            let a = transform(&h, Transform::Shift(-o));
            let b = transform(&transform(&a, Transform::Move(1)), Transform::Shift(half));
            let mut tune = a;
            tune.extend(b);
            tune.retain(|m| m.on < grid.bar);
            let r = reshaped(ph, bp, settings, &tune, 0.4, true, dice);
            aimed(&r, &tune)
        }
        (Gesture::HoldBack, None) => {
            // Your opening's first notes a step lower, the last held into
            // the bar's end; the plan's rest takes the last beat.
            let tune: Vec<GridNote> = transform(head, Transform::Move(-1))
                .into_iter()
                .filter(|m| m.on < grid.half())
                .take(3)
                .collect();
            let mut r = rhythm_of(&tune, grid);
            if let Some(l) = r.last_mut() {
                l.len = (grid.bar - beat - l.on).max(l.len);
            }
            aimed(&r, &tune)
        }
        (_, None) => {
            // A set-up: your tune climbing, its rhythm reshaped.
            let tune = transform(src, Transform::Move(bp.shift));
            let r = reshaped(ph, bp, settings, src, 0.6, true, dice);
            aimed(&r, &tune)
        }
    };
    planned.sort_by_key(|p| p.on);
    planned.dedup_by_key(|p| p.on);
    // A bar without an anchor leads into the next bar's, when that falls
    // on the downbeat: its last note a step away, from the side it is on.
    if let (None, Some(nf)) = (anchor, next_first) {
        if let Some(t) = planned.last_mut().and_then(|p| p.target.as_mut()) {
            if (t.pos - nf).abs() != 1 {
                t.pos = if t.pos > nf { nf + 1 } else { nf - 1 };
                t.alt = 0;
            }
        }
    }
    // No note runs into the next.
    for i in 0..planned.len() {
        let next = planned.get(i + 1).map_or(grid.bar, |p| p.on);
        planned[i].len = planned[i].len.min(next - planned[i].on).max(1);
    }
    let mut notes_as: Vec<NoteAs> = planned
        .iter()
        .map(|p| match anchor {
            Some(a) if p.on == a.on => NoteAs::Fixed(a.pos, a.alt),
            _ => NoteAs::Free,
        })
        .collect();
    // An outside note with a job: a half step leading into the high point
    // or the last note, in a style that uses them.
    if let Some(a) = anchor {
        let i = planned.iter().position(|p| p.on == a.on).unwrap_or(0);
        if i > 0
            && a.kind != AnchorKind::Landing
            && ph.style.melody.chromatic > 0.0
            && planned[i - 1].on + planned[i - 1].len == a.on
            && planned[i - 1].len <= eighth
            && dice.chance(0.5)
        {
            let pitch = ph.key.pitch_at(a.pos, a.alt).saturating_sub(1);
            let (pos, alt) = ph.key.position(pitch);
            notes_as[i - 1] = NoteAs::Fixed(pos, alt);
        }
    }
    (planned, notes_as)
}

/// The plan's rests in bar `b`: planned notes starting in one go (never a
/// held one), and notes before it end by its start.
fn rest_in(planned: &mut Vec<Planned>, notes_as: &mut Vec<NoteAs>, rests: &[(i32, i32)]) {
    let mut keep: Vec<(Planned, NoteAs)> = Vec::with_capacity(planned.len());
    for (p, a) in planned.iter().zip(notes_as.iter()) {
        let held = matches!(a, NoteAs::Fixed(..));
        if !held && rests.iter().any(|&(f, t)| p.on >= f && p.on < t) {
            continue;
        }
        let mut p = *p;
        if let Some(&(f, _)) = rests.iter().find(|&&(f, _)| p.on < f && p.on + p.len > f) {
            p.len = (f - p.on).max(1);
        }
        keep.push((p, *a));
    }
    *planned = keep.iter().map(|x| x.0).collect();
    *notes_as = keep.iter().map(|x| x.1).collect();
}

/// Your bars one by one (from each bar's start), and your opening: the
/// first motif from its start, within a bar (your first bar when the
/// motif is a single note).
fn yours_for(ph: &Phrasing) -> (Vec<Vec<GridNote>>, Vec<GridNote>) {
    let gb = ph.grid.bar;
    let your_bars: Vec<Vec<GridNote>> = (0..ph.your_bars.max(1))
        .map(|b| bar_of(&ph.yours, b as i32, gb))
        .collect();
    let m = motifs(&ph.yours).into_iter().next().unwrap_or_default();
    let o = m.first().map_or(0, |x| x.on);
    let mut head = transform(&m, Transform::Shift(-o));
    head.retain(|x| x.on < gb);
    if head.len() < 2 {
        head = your_bars[0].clone();
    }
    (your_bars, head)
}

/// What every new bar of a draft is filled from.
struct Frame<'a, 'b> {
    ph: &'b Phrasing<'a>,
    settings: &'b Settings,
    sketch: &'b Sketch,
    fixed: &'b [Fixed],
    /// A step under the high point: nothing but the high point reaches it.
    under: Option<u8>,
    your_bars: &'b [Vec<GridNote>],
    head: &'b [GridNote],
}

impl Frame<'_, '_> {
    /// The next bar's anchor, when it falls on that bar's downbeat.
    fn anchor_after(&self, b: usize) -> Option<i32> {
        let at = (b as i32 + 1) * self.ph.grid.bar;
        self.fixed
            .iter()
            .find(|f| f.bar == b + 1 && f.on == at)
            .map(|f| f.pos)
    }

    /// New bar `b`'s notes (from the bar's start), after `notes` (every
    /// note before it, from your first bar's start), leading into
    /// `next_first`; `None` when the plan leaves the bar silent. Free
    /// notes steer off `avoid`'s (the bar as it was, for Try again) where
    /// another note fits.
    fn bar(
        &self,
        b: usize,
        notes: &[GridNote],
        next_first: Option<i32>,
        attempt: u16,
        avoid: &[GridNote],
    ) -> Result<Option<BarOut>, Refused> {
        let (ph, s) = (self.ph, self.settings);
        let gb = ph.grid.bar;
        let off = b as i32 * gb;
        let g = self.sketch.gestures[b - ph.your_bars];
        let bp = self.sketch.plan[b];
        let mut rd = ph.dice(s, Job::Rhythm, b, attempt);
        let mut pd = ph.dice(s, Job::Pitch, b, attempt);
        let src = &self.your_bars[b % self.your_bars.len()];
        let anchor = self.fixed.iter().find(|f| f.bar == b).map(|f| Fixed {
            on: f.on - off,
            ..*f
        });
        let (mut planned, mut notes_as) =
            material(ph, &bp, g, src, self.head, anchor, next_first, s, &mut rd);
        if !s.keeps.rhythm {
            let rests: Vec<(i32, i32)> = self
                .sketch
                .rests
                .iter()
                .filter(|&&(f, t)| t > off && f < off + gb)
                .map(|&(f, t)| (f - off, t - off))
                .collect();
            rest_in(&mut planned, &mut notes_as, &rests);
        }
        if planned.is_empty() {
            return Ok(None);
        }
        for (p, a) in planned.iter().zip(notes_as.iter_mut()) {
            if *a == NoteAs::Free {
                if let Some(m) = avoid.iter().find(|m| m.on == p.on) {
                    *a = NoteAs::Change(m.pos, m.alt);
                }
            }
        }
        let before: Vec<(i32, i8)> = notes
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|m| (m.pos, m.alt))
            .collect();
        let last_on = notes.last().map(|m| m.on - off);
        // Nothing but the high point itself (held as it is) reaches it.
        let bar_ph = with_ceiling(ph, self.under);
        let out = fit(
            &bar_ph, &bp, b, &planned, &notes_as, &before, last_on, next_first, src, s, &mut pd,
        )?;
        Ok(Some(out))
    }
}

/// The new bars of `keeping` to keep: those still new bars, in order.
fn kept_bars(keeping: Option<(&Continued, &[usize])>, yours: usize, total: usize) -> Vec<usize> {
    keeping.map_or_else(Vec::new, |(before, bars)| {
        let mut k: Vec<usize> = bars
            .iter()
            .copied()
            .filter(|&b| b >= yours && b < total && b < before.plan.len())
            .collect();
        k.sort_unstable();
        k.dedup();
        k
    })
}

/// Draft number `d`.
pub fn draft(req: &Continue, d: u16) -> Result<Draft, Refused> {
    draft_keeping(req, d, None)
}

/// Draft number `d`, keeping new bars of an earlier result: with
/// `Some((before, bars))`, each of `bars` that is still a new bar plays
/// exactly as it did (its notes, its chords, how it was played), its
/// anchor's note stays, and the bars around it lead into and out of it.
pub fn draft_keeping(
    req: &Continue,
    d: u16,
    keeping: Option<(&Continued, &[usize])>,
) -> Result<Draft, Refused> {
    let s = req.settings;
    let yours = your_bars(req);
    let total = yours + usize::from(s.bars.max(1));
    let bar_ticks = req.meter.bar_ticks().max(1);
    let key = if s.keeps.chords {
        None
    } else {
        req.context.key.key()
    };
    let empty = ChordTimeline::default();
    let ph0 = phrasing(req, &empty);
    let grid: Grid = ph0.grid;
    let gb = grid.bar;
    let sk = sketch(&ph0, s, total, d);
    let kept = kept_bars(keeping, yours, total);
    let is_kept = |b: usize| kept.contains(&b);
    let old = keeping.map(|(before, _)| before);
    // New bars inside the row play its chords (contract 4): those with a
    // clear chord, or one you set, are locked like kept bars.
    let row_bars: Vec<usize> = (yours..total)
        .filter(|&b| !is_kept(b) && (b as Tick + 1) * bar_ticks <= req.row_length)
        .filter(|&b| {
            chords_in_bar(&req.context.chords, b, bar_ticks)
                .iter()
                .any(|s| s.answer.chord().is_some() || s.heard == ChordHeard::Set)
        })
        .collect();
    let fixed = |b: usize| is_kept(b) || row_bars.contains(&b);
    // A kept bar's notes, from your first bar's start.
    let kept_notes = |b: usize| -> Vec<GridNote> {
        old.map_or_else(Vec::new, |o| {
            o.line
                .notes
                .iter()
                .filter(|m| m.on.div_euclid(gb) == b as i32)
                .copied()
                .collect()
        })
    };

    // The chords, led to the anchors and the surprise; a kept bar's
    // chords stay as they were.
    let mut needs: Vec<Need> = sk
        .anchors
        .iter()
        .filter(|a| !fixed(a.bar))
        .map(|a| {
            let at = Tick::from(a.on) * STEP;
            if a.kind == AnchorKind::Peak {
                Need::Holding {
                    at,
                    pc: ph0.key.pitch_at(a.aim, 0) % 12,
                    degrees: a.chords.clone(),
                }
            } else {
                Need::Degrees {
                    at,
                    degrees: a.chords.clone(),
                }
            }
        })
        .collect();
    if let (Some(Surprise::Borrowed { bar }), Some(k)) = (sk.surprise, key) {
        if !fixed(bar) {
            let at = (bar as i32 * gb + grid.half()) as Tick * STEP;
            needs.push(Need::Chord {
                at,
                chord: borrowed(k),
            });
        }
    }
    let mut locked_chords: Vec<(usize, Vec<ChordSpan>)> = match old {
        Some(o) => kept
            .iter()
            .map(|&b| (b, chords_in_bar(&o.chords, b, bar_ticks)))
            .collect(),
        None => Vec::new(),
    };
    locked_chords.extend(
        row_bars
            .iter()
            .map(|&b| (b, chords_in_bar(&req.context.chords, b, bar_ticks))),
    );
    let chords = progression_led(
        &req.context.chords,
        key,
        &s.style.harmony,
        &sk.plan,
        s.variation,
        s.seed,
        d,
        req.meter,
        &locked_chords,
        &needs,
    );
    let mut ph = phrasing(req, &chords);
    ph.music_first = true;

    // The anchors' notes: the high point first (above any kept note),
    // then the rest under it. A kept bar's anchor keeps the note it has.
    let kept_note = |a: &Anchor| -> Option<(i32, i8)> {
        let bar = kept_notes(a.bar);
        bar.iter()
            .find(|m| m.on == a.on)
            .or_else(|| {
                (a.kind == AnchorKind::Peak)
                    .then(|| bar.iter().max_by_key(|m| ph.key.pitch_at(m.pos, m.alt)))
                    .flatten()
            })
            .map(|m| (m.pos, m.alt))
    };
    let kept_top = kept
        .iter()
        .flat_map(|&b| kept_notes(b))
        .map(|m| m.pos)
        .max();
    let peak = sk
        .anchors
        .iter()
        .find(|a| a.kind == AnchorKind::Peak)
        .and_then(|a| {
            if is_kept(a.bar) {
                kept_note(a)
            } else {
                resolve(&ph, s, a, None, kept_top.map(|t| t + 1))
                    .or_else(|| resolve(&ph, s, a, None, None))
            }
            .map(|(pos, alt)| (a, pos, alt))
        });
    let peak_pitch = peak.map(|(_, pos, alt)| ph.key.pitch_at(pos, alt));
    let under = peak_pitch.map(|p| p.saturating_sub(1));
    // Each landing on the way up sits above the one before (the first
    // above your opening), so the reaches climb to the high point.
    let mut floor = ph.yours.first().map(|m| m.pos + 1);
    let mut fixed: Vec<Fixed> = Vec::new();
    for a in &sk.anchors {
        let got = match a.kind {
            _ if is_kept(a.bar) => kept_note(a),
            AnchorKind::Peak => peak.map(|(_, pos, alt)| (pos, alt)),
            AnchorKind::Landing if a.bar < peak.map_or(usize::MAX, |p| p.0.bar) => {
                resolve(&ph, s, a, under, floor)
            }
            _ => resolve(&ph, s, a, under, None),
        };
        if let Some((pos, alt)) = got {
            if a.kind == AnchorKind::Landing && a.bar < peak.map_or(usize::MAX, |p| p.0.bar) {
                floor = Some(pos + 1);
            }
            fixed.push(Fixed {
                kind: a.kind,
                bar: a.bar,
                on: a.on,
                len: a.len,
                pos,
                alt,
            });
        }
    }

    // The notes, bar by bar; a kept bar as it was.
    let (your_bars, head) = yours_for(&ph);
    let frame = Frame {
        ph: &ph,
        settings: s,
        sketch: &sk,
        fixed: &fixed,
        under,
        your_bars: &your_bars,
        head: &head,
    };
    let mut notes: Vec<GridNote> = ph
        .yours
        .iter()
        .filter(|m| m.on < yours as i32 * gb)
        .copied()
        .collect();
    let mut sims = vec![None; yours];
    let mut gave_way = Vec::new();
    let mut tried = 0;
    for b in yours..total {
        let off = b as i32 * gb;
        if is_kept(b) {
            notes.extend(kept_notes(b));
            sims.push(None);
            continue;
        }
        let next_first = frame
            .anchor_after(b)
            .or_else(|| {
                is_kept(b + 1)
                    .then(|| {
                        kept_notes(b + 1)
                            .first()
                            .filter(|m| m.on == off + gb)
                            .map(|m| m.pos)
                    })
                    .flatten()
            })
            .or_else(|| {
                (b + 1 == total && req.loops)
                    .then(|| ph.yours.first().map(|m| m.pos))
                    .flatten()
            });
        match frame.bar(b, &notes, next_first, d, &[])? {
            Some(out) => {
                tried += out.tried;
                gave_way.extend(out.gave_way.iter().map(|&r| (b as u16 + 1, r)));
                sims.push(Some(out.similarity));
                notes.extend(out.notes.iter().map(|m| GridNote {
                    on: m.on + off,
                    ..*m
                }));
            }
            None => sims.push(Some(0.0)),
        }
    }
    let line = Line {
        notes,
        similarity: sims,
        gave_way,
        tried,
    };
    let (mut part, mut events) = perform_with(req, &ph, &sk.plan, &chords, &line, false, Some(&sk));
    if let Some(o) = old {
        // The kept bars as they were played, exactly.
        let free = |at: Tick| !is_kept(at.div_euclid(bar_ticks) as usize);
        (part.decisions, events) = splice(o, &part, &events, &free);
    }
    let to = total as Tick * bar_ticks;
    let clip = clip_notes(&events, req.bpm, to)?;
    let landings: Vec<Landing> = fixed
        .iter()
        .map(|f| Landing {
            on: f.on,
            pos: f.pos,
            alt: f.alt,
        })
        .collect();
    let verdict = judge(&Heard {
        notes: &line.notes,
        your_bars: yours,
        bars: total,
        grid,
        key: ph.key,
        chords: &chords,
        ending: s.ending,
        copy_bars: copy_bars(&s.style),
        landings: &landings,
        rests: if s.keeps.rhythm { &[] } else { &sk.rests },
    });
    Ok(Draft {
        number: d,
        made: Continued {
            plan: sk.plan.clone(),
            chords,
            line,
            part,
            events,
            notes: clip,
            to,
            sketch: Some(sk.clone()),
            critic: Vec::new(),
        },
        sketch: sk,
        verdict,
    })
}

/// Try the new bars `bars` of a music-first result again, together: each
/// is filled afresh around its anchor (a landing keeps its note, the high
/// point stays the highest) between the bars that stay, leading in from
/// the bar before and out into the bar after, and every other note stays
/// exactly as it was, played as it was.
pub fn try_bars_s1(
    req: &Continue,
    before: &Continued,
    sk: &Sketch,
    bars: &[usize],
    attempt: u16,
) -> Result<Continued, Refused> {
    let s = req.settings;
    let yours = your_bars(req);
    let total = before.plan.len().min(yours + sk.gestures.len());
    let mut ph = phrasing(req, &before.chords);
    ph.music_first = true;
    let gb = ph.grid.bar;
    let mut tried_bars: Vec<usize> = bars
        .iter()
        .copied()
        .filter(|&b| b >= yours && b < total)
        .collect();
    tried_bars.sort_unstable();
    tried_bars.dedup();
    // The anchors with the notes they have now.
    let fixed: Vec<Fixed> = sk
        .anchors
        .iter()
        .filter_map(|a| {
            let m = before.line.notes.iter().find(|m| m.on == a.on)?;
            Some(Fixed {
                kind: a.kind,
                bar: a.bar,
                on: a.on,
                len: a.len,
                pos: m.pos,
                alt: m.alt,
            })
        })
        .collect();
    let under = fixed
        .iter()
        .find(|f| f.kind == AnchorKind::Peak)
        .map(|f| ph.key.pitch_at(f.pos, f.alt).saturating_sub(1));
    let (your_bars, head) = yours_for(&ph);
    let frame = Frame {
        ph: &ph,
        settings: s,
        sketch: sk,
        fixed: &fixed,
        under,
        your_bars: &your_bars,
        head: &head,
    };
    let mut notes = before.line.notes.clone();
    let mut sims = before.line.similarity.clone();
    sims.resize(total, None);
    let mut gave_way: Vec<_> = before
        .line
        .gave_way
        .iter()
        .filter(|(b, _)| !tried_bars.contains(&(usize::from(*b) - 1)))
        .copied()
        .collect();
    let mut tried = 0;
    for &b in &tried_bars {
        let off = b as i32 * gb;
        let was: Vec<GridNote> = before.line.bar(b, ph.grid);
        notes.retain(|m| m.on < off || m.on >= off + gb);
        let prior: Vec<GridNote> = notes.iter().filter(|m| m.on < off).copied().collect();
        let next_first = frame
            .anchor_after(b)
            .or_else(|| {
                (b + 1 < total && !tried_bars.contains(&(b + 1)))
                    .then(|| notes.iter().find(|m| m.on == off + gb).map(|m| m.pos))
                    .flatten()
            })
            .or_else(|| {
                (b + 1 == total && req.loops)
                    .then(|| ph.yours.first().map(|m| m.pos))
                    .flatten()
            });
        match frame.bar(b, &prior, next_first, TRY_ATTEMPTS + attempt, &was)? {
            Some(out) => {
                tried += out.tried;
                gave_way.extend(out.gave_way.iter().map(|&r| (b as u16 + 1, r)));
                sims[b] = Some(out.similarity);
                notes.extend(out.notes.iter().map(|m| GridNote {
                    on: m.on + off,
                    ..*m
                }));
            }
            None => sims[b] = Some(0.0),
        }
        notes.sort_by_key(|m| m.on);
    }
    gave_way.sort_by_key(|g| g.0);
    let line = Line {
        notes,
        similarity: sims,
        gave_way,
        tried,
    };
    let (mut part, events) = perform_with(
        req,
        &ph,
        &before.plan,
        &before.chords,
        &line,
        false,
        Some(sk),
    );
    let bt = req.meter.bar_ticks().max(1);
    let inside = |at: Tick| tried_bars.contains(&(at.div_euclid(bt) as usize));
    let (ds, events) = splice(before, &part, &events, &inside);
    part.decisions = ds;
    let clip = clip_notes(&events, req.bpm, before.to)?;
    Ok(Continued {
        plan: before.plan.clone(),
        chords: before.chords.clone(),
        line,
        part,
        events,
        notes: clip,
        to: before.to,
        sketch: Some(sk.clone()),
        critic: Vec::new(),
    })
}

/// Try again's dice start past the drafts', so a tried bar is never the
/// same as another draft's.
const TRY_ATTEMPTS: u16 = 64;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Source;
    use crate::profile::InstrumentKind;
    use crate::style::{Style, GROOVE, JAZZISH, MINIMAL};
    use crate::PlainNote;
    use mpfree_context::listen::{listen, Played};
    use mpfree_context::{MusicalContext, PartRole, ReadBudget};
    use mpfree_transport::time::{Meter, Swing, PPQ};

    const BAR: Tick = 4 * PPQ as Tick;

    /// The prototype's two-bar riff in C minor.
    fn riff() -> Vec<(PlainNote, Source)> {
        [
            (0, 2, 60),
            (2, 1, 63),
            (3, 1, 67),
            (4, 2, 65),
            (6, 2, 63),
            (8, 3, 62),
            (11, 1, 63),
            (12, 4, 60),
            (16, 2, 67),
            (18, 1, 68),
            (19, 1, 67),
            (20, 2, 65),
            (22, 2, 63),
            (24, 2, 62),
            (26, 2, 63),
            (28, 4, 65),
        ]
        .iter()
        .map(|&(on, len, pitch)| {
            (
                PlainNote {
                    at: on * STEP,
                    len: len * STEP,
                    pitch,
                    velocity: 90,
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

    fn settings(style: Style, bars: u8, seed: u64) -> Settings {
        Settings {
            style,
            bars,
            seed,
            ..Settings::default()
        }
    }

    #[test]
    fn every_draft_plays_its_plan_the_same_way_twice() {
        let notes = riff();
        let ctx = context(&notes);
        for style in [MINIMAL, GROOVE, JAZZISH] {
            for seed in [1, 2, 3] {
                let s = settings(style, 8, seed);
                let req = ask(&notes, &ctx, &s);
                let all = drafts(&req).unwrap();
                assert_eq!(all.len(), usize::from(DRAFTS));
                for d in all.iter().filter(|d| d.made.critic.is_empty()) {
                    let again = draft(&req, d.number).unwrap();
                    assert_eq!(
                        again.made, d.made,
                        "{} seed {seed} draft {}",
                        style.name, d.number
                    );
                    let made = &d.made;
                    assert_eq!(made.to, 10 * BAR);
                    assert_eq!(made.notes.len(), made.events.len());
                    assert!(made
                        .notes
                        .iter()
                        .all(|n| (2 * BAR - 60..10 * BAR).contains(&n.tick)));
                    assert!(made.events.iter().all(|e| e.source == Source::Mpfree));
                    assert!(made.events.iter().all(|e| (20..=127).contains(&e.velocity)));
                }
                // Best first: no later draft passes more checks.
                assert!(all
                    .windows(2)
                    .all(|w| w[0].verdict.passed() >= w[1].verdict.passed()));
            }
        }
    }

    #[test]
    fn the_landings_climb_to_one_high_point_and_the_last_note_comes_home() {
        let notes = riff();
        let ctx = context(&notes);
        for style in [MINIMAL, GROOVE, JAZZISH] {
            for seed in 1..=4 {
                let s = settings(style, 8, seed);
                let req = ask(&notes, &ctx, &s);
                for d in drafts(&req).unwrap() {
                    let what = format!("{} seed {seed} draft {}", style.name, d.number);
                    let line = &d.made.line.notes;
                    let ph = phrasing(&req, &d.made.chords);
                    let key = ph.key;
                    let pitch = |on: i32| {
                        line.iter()
                            .find(|m| m.on == on)
                            .map(|m| key.pitch_at(m.pos, m.alt))
                    };
                    let peak = d
                        .sketch
                        .anchors
                        .iter()
                        .find(|a| a.kind == AnchorKind::Peak)
                        .expect("a high point");
                    let top = pitch(peak.on).unwrap_or_else(|| panic!("{what}: high point sounds"));
                    let start = 2 * ph.grid.bar;
                    let highest = line
                        .iter()
                        .filter(|m| m.on >= start)
                        .map(|m| key.pitch_at(m.pos, m.alt))
                        .max()
                        .unwrap();
                    assert_eq!(top, highest, "{what}: nothing above the high point");
                    let first = line
                        .iter()
                        .filter(|m| m.on >= start)
                        .position(|m| key.pitch_at(m.pos, m.alt) == top)
                        .unwrap();
                    assert_eq!(
                        line.iter()
                            .filter(|m| m.on >= start)
                            .nth(first)
                            .map(|m| m.on),
                        Some(peak.on),
                        "{what}: the high point is first heard on its anchor"
                    );
                    let landings: Vec<u8> = d
                        .sketch
                        .anchors
                        .iter()
                        .filter(|a| a.kind == AnchorKind::Landing && a.bar < peak.bar)
                        .filter_map(|a| pitch(a.on))
                        .collect();
                    assert!(
                        landings.windows(2).all(|w| w[0] < w[1]),
                        "{what}: the landings climb, {landings:?}"
                    );
                    assert!(d.verdict.lands.pass, "{what}: {}", d.verdict.words());
                }
            }
        }
    }

    #[test]
    fn short_and_long_requests_are_carried_on_too() {
        let notes = riff();
        let ctx = context(&notes);
        for bars in [1, 2, 4, 16] {
            let s = settings(GROOVE, bars, 5);
            let req = ask(&notes, &ctx, &s);
            let all = drafts(&req).unwrap();
            assert_eq!(
                all.len(),
                usize::from(if bars > 8 { DRAFTS_LONG } else { DRAFTS }),
                "{bars} bars"
            );
            let made = continue_s1(&req).unwrap();
            assert_eq!(made, all[0].made);
            assert_eq!(made.to, (2 + Tick::from(bars)) * BAR);
            assert!(!made.notes.is_empty(), "{bars} bars");
        }
    }

    /// S4: the second draft is a real choice, and a draft that fails a
    /// check is repaired in place: a bar made again, the check put right,
    /// nothing that passed broken, your locked bars untouched, and the
    /// repair said in words.
    #[test]
    fn the_runner_up_differs_and_a_weak_draft_is_repaired() {
        let notes = riff();
        let ctx = context(&notes);
        let from = 2 * 16;
        let (mut tried, mut repaired) = (0, 0);
        for style in [MINIMAL, GROOVE, JAZZISH] {
            for seed in 1..=6 {
                let s = settings(style, 8, seed);
                let req = ask(&notes, &ctx, &s);
                let all = drafts(&req).unwrap();
                if (1..all.len()).any(|j| difference(&all[0].made, &all[j].made, from) >= DIFFER) {
                    assert!(
                        difference(&all[0].made, &all[1].made, from) >= DIFFER,
                        "{} seed {seed}: the runner-up is a real choice",
                        style.name
                    );
                }
                for d in all.iter().filter(|d| d.verdict.passed() < 6) {
                    tried += 1;
                    let mut fixed = d.clone();
                    let locked = [7];
                    repair(&req, &mut fixed, &locked);
                    if fixed.made.critic.is_empty() {
                        assert_eq!(fixed.made, d.made, "no repair: nothing changes");
                        continue;
                    }
                    repaired += 1;
                    assert!(fixed.verdict.passed() > d.verdict.passed());
                    for ((name, a), (_, b)) in
                        d.verdict.checks().iter().zip(fixed.verdict.checks().iter())
                    {
                        assert!(!a.pass || b.pass, "{name} passed and still does");
                    }
                    assert_eq!(fixed.verdict, verdict_for(&req, &fixed.made, &fixed.sketch));
                    let bar = |b: usize| -> Vec<(i32, i32, i8)> {
                        let r = (b as i32 * 16)..((b as i32 + 1) * 16);
                        let pick = |c: &Continued| {
                            c.line
                                .notes
                                .iter()
                                .filter(|m| r.contains(&m.on))
                                .map(|m| (m.on, m.pos, m.alt))
                                .collect::<Vec<_>>()
                        };
                        assert_eq!(pick(&fixed.made), pick(&d.made), "bar {} is locked", b + 1);
                        pick(&d.made)
                    };
                    let _ = bar(7);
                    for w in &fixed.made.critic {
                        assert!(
                            w.starts_with("bar ") && w.contains(" made again so "),
                            "{w}"
                        );
                    }
                }
            }
        }
        assert!(tried > 0, "some draft fails a check");
        assert!(repaired > 0, "a repair was found ({tried} weak drafts)");
    }

    /// Events and chords inside bar `b`.
    fn in_bar(c: &Continued, b: usize) -> (Vec<crate::Event>, Vec<ChordSpan>) {
        let ev = c
            .events
            .iter()
            .filter(|e| e.at.div_euclid(BAR) as usize == b)
            .copied()
            .collect();
        (ev, chords_in_bar(&c.chords, b, BAR))
    }

    #[test]
    fn locked_bars_play_as_they_were_and_the_rest_is_made_again() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(GROOVE, 8, 3);
        let before = crate::continue_notes(&ask(&notes, &ctx, &s)).unwrap();
        assert!(before.sketch.is_some());
        let s2 = settings(GROOVE, 8, 4);
        let locked = [4, 7];
        let after =
            crate::continue_notes_keeping(&ask(&notes, &ctx, &s2), Some((&before, &locked)))
                .unwrap();
        for b in locked {
            assert_eq!(in_bar(&after, b), in_bar(&before, b), "bar {b} kept");
        }
        let changed = (2..10)
            .filter(|b| !locked.contains(b))
            .any(|b| in_bar(&after, b).0 != in_bar(&before, b).0);
        assert!(changed, "the other bars are made again");
    }

    #[test]
    fn try_again_on_a_bar_keeps_its_landing_and_every_other_note() {
        let notes = riff();
        let ctx = context(&notes);
        for style in [MINIMAL, GROOVE, JAZZISH] {
            let s = settings(style, 8, 2);
            let req = ask(&notes, &ctx, &s);
            let before = crate::continue_notes(&req).unwrap();
            let sk = before.sketch.clone().unwrap();
            let landing = sk
                .anchors
                .iter()
                .find(|a| a.kind == AnchorKind::Landing)
                .unwrap();
            let held = before
                .line
                .notes
                .iter()
                .find(|m| m.on == landing.on)
                .copied();
            let mut differs = false;
            for attempt in 1..=4 {
                let after = crate::try_bars(&req, &before, &[landing.bar], attempt).unwrap();
                assert_eq!(after.to, before.to);
                assert_eq!(after.chords, before.chords);
                for b in (2..10).filter(|&b| b != landing.bar) {
                    assert_eq!(
                        in_bar(&after, b).0,
                        in_bar(&before, b).0,
                        "{} bar {b}",
                        style.name
                    );
                }
                let now = after
                    .line
                    .notes
                    .iter()
                    .find(|m| m.on == landing.on)
                    .copied();
                assert_eq!(
                    now.map(|m| (m.pos, m.alt)),
                    held.map(|m| (m.pos, m.alt)),
                    "{}: the landing keeps its note",
                    style.name
                );
                differs |= in_bar(&after, landing.bar).0 != in_bar(&before, landing.bar).0;
            }
            assert!(differs, "{}: Try again changes the bar", style.name);
        }
    }
}
