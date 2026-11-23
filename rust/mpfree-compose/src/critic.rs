//! The critic (revision 3, S1): a whole result judged before it is
//! offered, on separate checks rather than one score.
//!
//! Each check reads the notes only, so it runs on the MPC as well as in
//! the desktop tests, and says in words what it found:
//!
//! - **Does it go somewhere?** A clear high point, reached late rather
//!   than in the first bar, above where the new bars began.
//! - **Does it land?** The landing notes arrive on time, on chord notes
//!   (the planned ones when there is a plan), and the last one closes the
//!   way the ending asks.
//! - **Does it breathe?** Rests where the phrases end; not every beat
//!   struck.
//! - **Is it yours?** Your opening can be heard in the new bars, changed
//!   rather than copied or lost.
//! - **Does it repeat too much?** No stretch copied bar for bar for
//!   longer than the style allows.
//! - **Does it flow?** Few wide leaps, each turning back, none wider
//!   than a sixth, and no trill of two notes back and forth.
//!
//! "Does it fit the row?" needs the row's other parts, which Continue is
//! not given yet; until then it isn't asked.

use mpfree_context::chord::ChordTimeline;
use mpfree_context::groove::STEP;
use mpfree_context::key::{Key, Mode};
use mpfree_seq::Note;
use mpfree_transport::time::Tick;

use crate::motif::{bar_of, line, motifs, similarity, transform, GridNote, Transform};
use crate::plan::shapes;
use crate::realize::PlainNote;
use crate::request::{your_bars, Continue};
use crate::rhythm::Grid;
use crate::settings::Ending;
use crate::style::{PlanShape, Style};

/// A note the plan says must sound: where (in steps from your first
/// bar's start) and which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landing {
    pub on: i32,
    pub pos: i32,
    pub alt: i8,
}

/// What the critic is given: the whole line, yours then the new bars.
#[derive(Clone, Copy, Debug)]
pub struct Heard<'a> {
    /// Every note, in steps from the start of your first bar.
    pub notes: &'a [GridNote],
    pub your_bars: usize,
    /// Bars in all, yours and new.
    pub bars: usize,
    pub grid: Grid,
    pub key: Key,
    /// The chords under every bar, in ticks.
    pub chords: &'a ChordTimeline,
    pub ending: Ending,
    /// The most bars in a row that may copy earlier bars exactly.
    pub copy_bars: usize,
    /// The plan's landing notes; empty with no plan.
    pub landings: &'a [Landing],
    /// The plan's rests, as `(from, to)` in steps; empty with no plan.
    pub rests: &'a [(i32, i32)],
}

/// One check's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub pass: bool,
    /// 0 (missed by far) to 1 (clearly met), to break ties.
    pub score: f32,
    /// What it found, in words.
    pub words: String,
}

impl Check {
    pub(crate) fn new(pass: bool, score: f32, words: String) -> Check {
        Check {
            pass,
            score: score.clamp(0.0, 1.0),
            words,
        }
    }
}

/// Every check's answer for one result.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub somewhere: Check,
    pub lands: Check,
    pub breathes: Check,
    pub yours: Check,
    pub repeats: Check,
    pub flows: Check,
}

impl Verdict {
    pub fn checks(&self) -> [(&'static str, &Check); 6] {
        [
            ("goes somewhere", &self.somewhere),
            ("lands", &self.lands),
            ("breathes", &self.breathes),
            ("yours", &self.yours),
            ("not too repetitive", &self.repeats),
            ("flows", &self.flows),
        ]
    }

    /// Checks passed.
    pub fn passed(&self) -> usize {
        self.checks().iter().filter(|(_, c)| c.pass).count()
    }

    /// The scores together, to break a tie in checks passed.
    pub fn score(&self) -> f32 {
        self.checks().iter().map(|(_, c)| c.score).sum()
    }

    /// One line per check, for the listening report.
    pub fn words(&self) -> String {
        self.checks()
            .iter()
            .map(|(name, c)| format!("{} {name}: {}", if c.pass { "+" } else { "-" }, c.words))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// The most bars in a row a style lets copy earlier bars exactly: a loop
/// or a call and answer may play a bar round twice; a sentence never
/// plays one bar again whole.
pub fn copy_bars(style: &Style) -> usize {
    match shapes(style, 0).0 {
        PlanShape::Sentence => 1,
        _ => 2,
    }
}

/// Judge `new`, notes in the clip's ticks after the clip `req` carries
/// on, over `chords`: the result of any way of carrying it on (mpfree's,
/// or a simpler one), with no plan.
pub fn judge_continue(req: &Continue, new: &[Note], chords: &ChordTimeline) -> Verdict {
    let key = req.context.key.best().unwrap_or(Key::new(0, Mode::Major));
    let yours = your_bars(req);
    let bar = req.meter.bar_ticks().max(1);
    let to = new
        .iter()
        .map(|n| n.tick)
        .max()
        .map_or(yours as Tick, |t| t / bar + 1);
    let mut plain: Vec<PlainNote> = req
        .notes
        .iter()
        .filter(|(n, s)| s.learnable() && n.at < yours as Tick * bar)
        .map(|(n, _)| *n)
        .collect();
    plain.extend(new.iter().map(|n| PlainNote {
        at: n.tick,
        len: n.len,
        pitch: n.note,
        velocity: n.velocity,
    }));
    let notes = line(&plain, key);
    let bars = (yours as Tick + Tick::from(req.settings.bars)).max(to) as usize;
    judge(&Heard {
        notes: &notes,
        your_bars: yours,
        bars,
        grid: Grid::of(req.meter),
        key,
        chords,
        ending: req.settings.ending,
        copy_bars: copy_bars(&req.settings.style),
        landings: &[],
        rests: &[],
    })
}

/// Judge one result.
pub fn judge(h: &Heard) -> Verdict {
    let start = h.your_bars as i32 * h.grid.bar;
    let new: Vec<GridNote> = h.notes.iter().filter(|m| m.on >= start).copied().collect();
    Verdict {
        somewhere: somewhere(h, &new, start),
        lands: lands(h, &new),
        breathes: breathes(h, &new, start),
        yours: yours(h, start),
        repeats: repeats(h),
        flows: flows(h, start),
    }
}

fn pitch(h: &Heard, m: &GridNote) -> i32 {
    i32::from(h.key.pitch_at(m.pos, m.alt))
}

fn bar_words(h: &Heard, on: i32) -> String {
    format!(
        "bar {} of {}",
        on.div_euclid(h.grid.bar) as usize + 1 - h.your_bars,
        h.bars - h.your_bars
    )
}

/// A clear high point, reached late: the highest new note first sounds
/// between 40 % and 92 % of the way through, at least 3 semitones above
/// the new bars' opening, and not hammered (at most one note in four of
/// the new bars on it).
fn somewhere(h: &Heard, new: &[GridNote], start: i32) -> Check {
    let Some(top) = new.iter().map(|m| pitch(h, m)).max() else {
        return Check::new(false, 0.0, "no new notes".into());
    };
    let span = ((h.bars - h.your_bars) as i32 * h.grid.bar).max(1);
    let first = new
        .iter()
        .find(|m| pitch(h, m) == top)
        .map_or(start, |m| m.on);
    let at = f64::from(first - start) / f64::from(span);
    let opening: Vec<i32> = new
        .iter()
        .filter(|m| m.on < start + (span / 4).max(h.grid.bar))
        .map(|m| pitch(h, m))
        .collect();
    let open = opening.iter().sum::<i32>() as f64 / opening.len().max(1) as f64;
    let lift = f64::from(top) - open;
    let on_top = new.iter().filter(|m| pitch(h, m) == top).count();
    let hammered = on_top > (new.len() / 4).max(2);
    // Two bars or fewer have no room to wait: the second half of the
    // first bar is late enough.
    let earliest = if h.bars - h.your_bars <= 2 { 0.25 } else { 0.4 };
    let late = (earliest..=0.92).contains(&at);
    let pass = late && lift >= 3.0 && !hammered;
    let timing = if late {
        1.0
    } else {
        (1.0 - (at - 0.66).abs() * 2.0).max(0.0) as f32
    };
    let score =
        timing * 0.5 + (lift / 7.0).min(1.0) as f32 * 0.4 + if hammered { 0.0 } else { 0.1 };
    let words = format!(
        "highest note first in {}, {:.0} semitones above the opening{}",
        bar_words(h, first),
        lift,
        if hammered { ", struck often" } else { "" }
    );
    Check::new(pass, score, words)
}

/// Whether `m` is a note of the chord sounding when it starts.
fn on_chord(h: &Heard, m: &GridNote) -> Option<bool> {
    let tick = Tick::from(m.on) * STEP;
    let chord = h.chords.span_at(tick).and_then(|s| s.answer.chord())?;
    Some(chord.tone_of(pitch(h, m)).is_some())
}

/// The landings: with a plan, each planned landing note sounds at its
/// place; and the last note closes. The last note starts on a beat, is
/// held a beat or to the bar's end, is a note of its chord, and closes as
/// the ending asks: home on the home note, its 3rd or 5th; open off the
/// home note; leading back near your first note or on the 5th.
fn lands(h: &Heard, new: &[GridNote]) -> Check {
    let Some(last) = new.last().copied() else {
        return Check::new(false, 0.0, "no new notes".into());
    };
    let mut misses: Vec<String> = Vec::new();
    let planned = h.landings.len();
    for l in h.landings {
        if !new
            .iter()
            .any(|m| m.on == l.on && m.pos == l.pos && m.alt == l.alt)
        {
            misses.push(format!("the landing in {} missing", bar_words(h, l.on)));
        }
    }
    let end = h.bars as i32 * h.grid.bar;
    let bar_end = (last.on.div_euclid(h.grid.bar) + 1) * h.grid.bar;
    if last.on % h.grid.beat != 0 {
        misses.push("the last note starts off the beat".into());
    }
    if last.len < h.grid.beat && last.on + last.len < bar_end.min(end) {
        misses.push("the last note is short".into());
    }
    if on_chord(h, &last) == Some(false) {
        misses.push("the last note is off its chord".into());
    }
    let degree = last.pos.rem_euclid(7);
    let closes = match h.ending {
        Ending::Home => last.alt == 0 && matches!(degree, 0 | 2 | 4),
        Ending::Open => degree != 0 || last.alt != 0,
        Ending::LoopsBack => {
            let first = h.notes.first().map_or(last.pos, |m| m.pos);
            (last.pos - first).abs() <= 2 || degree == 4
        }
    };
    if !closes {
        misses.push(format!(
            "the last note doesn't {}",
            match h.ending {
                Ending::Home => "land home",
                Ending::Open => "leave it open",
                Ending::LoopsBack => "lead back",
            }
        ));
    }
    let checks = planned + 4;
    let failed = misses.len();
    let score = 1.0 - failed as f32 / checks as f32;
    let words = if misses.is_empty() {
        if planned > 0 {
            format!("all {planned} landings on time, and it closes")
        } else {
            "it closes on time, on its chord".into()
        }
    } else {
        misses.join(", ")
    };
    Check::new(misses.is_empty(), score, words)
}

/// Rests: at least one rest of an 1/8 or more in every four new bars
/// (where the plan put them, when there is a plan), and no more than nine
/// beats in ten struck.
fn breathes(h: &Heard, new: &[GridNote], start: i32) -> Check {
    let end = h.bars as i32 * h.grid.bar;
    let eighth = (h.grid.beat / 2).max(1);
    let mut gaps: Vec<(i32, i32)> = Vec::new();
    for w in new.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b.on - (a.on + a.len) >= eighth {
            gaps.push((a.on + a.len, b.on));
        }
    }
    // A rest before the first new note counts too.
    if let (Some(f), Some(y)) = (new.first(), h.notes.iter().rev().find(|m| m.on < start)) {
        if f.on - (y.on + y.len) >= eighth {
            gaps.push((y.on + y.len, f.on));
        }
    }
    let groups = (h.bars - h.your_bars).div_ceil(4).max(1);
    let per_group = (0..groups)
        .filter(|&g| {
            let (a, b) = (
                start + g as i32 * 4 * h.grid.bar,
                start + (g as i32 + 1) * 4 * h.grid.bar,
            );
            gaps.iter().any(|&(x, y)| x < b && y > a)
        })
        .count();
    let planned_hit = h
        .rests
        .iter()
        .filter(|&&(a, b)| !new.iter().any(|m| m.on >= a && m.on < b))
        .count();
    let beats_total = ((end - start) / h.grid.beat).max(1);
    let struck = (0..beats_total)
        .filter(|&k| {
            let (a, b) = (start + k * h.grid.beat, start + (k + 1) * h.grid.beat);
            new.iter().any(|m| m.on >= a && m.on < b)
        })
        .count();
    let share = struck as f64 / f64::from(beats_total);
    let pass = per_group == groups && planned_hit == h.rests.len() && share <= 0.9;
    let score = 0.5 * per_group as f32 / groups as f32
        + 0.3
            * if h.rests.is_empty() {
                1.0
            } else {
                planned_hit as f32 / h.rests.len() as f32
            }
        + 0.2 * if share <= 0.9 { 1.0 } else { 0.0 };
    let words = format!(
        "{} rests of an 1/8 or more, {per_group} of {groups} four-bar stretches with one, {:.0} % of beats struck{}",
        gaps.len(),
        share * 100.0,
        if h.rests.is_empty() {
            String::new()
        } else {
            format!(", {planned_hit} of {} planned rests kept", h.rests.len())
        }
    );
    Check::new(pass, score, words)
}

/// Your opening (your first motif) heard in the new bars: some new bar
/// holds a stretch alike in tune to it (0.5 or more) without being a copy
/// of one of your bars, and at most half the new bars copy yours.
fn yours(h: &Heard, start: i32) -> Check {
    let your_line: Vec<GridNote> = h.notes.iter().filter(|m| m.on < start).copied().collect();
    // Your first motif, within its first bar and at most six notes.
    let Some(mut head) = motifs(&your_line).into_iter().next() else {
        return Check::new(true, 0.5, "too few notes of yours to look for".into());
    };
    let o = head.first().map_or(0, |m| m.on);
    head.retain(|m| m.on < o + h.grid.bar);
    head.truncate(6);
    if head.len() < 2 {
        return Check::new(true, 0.5, "too few notes of yours to look for".into());
    }
    let head = transform(&head, Transform::Shift(-o));
    let your_bars: Vec<Vec<GridNote>> = (0..h.your_bars)
        .map(|b| bar_of(h.notes, b as i32, h.grid.bar))
        .collect();
    let mut heard: Vec<usize> = Vec::new();
    let mut copied = 0;
    let new_bars = h.bars - h.your_bars;
    for b in h.your_bars..h.bars {
        let bar = bar_of(h.notes, b as i32, h.grid.bar);
        if bar.is_empty() {
            continue;
        }
        if your_bars.iter().any(|y| same(y, &bar)) {
            copied += 1;
            continue;
        }
        // The best stretch of the bar's notes as long as the head.
        let n = head.len();
        let best = (0..bar.len().saturating_sub(n.saturating_sub(1)).max(1))
            .map(|i| {
                let w: Vec<GridNote> = bar.iter().skip(i).take(n).copied().collect();
                let o = w.first().map_or(0, |m| m.on);
                similarity(h.key, &transform(&w, Transform::Shift(-o)), &head).tune()
            })
            .fold(0.0, f64::max);
        if best >= 0.5 {
            heard.push(b + 1 - h.your_bars);
        }
    }
    let copy_share = copied as f32 / new_bars.max(1) as f32;
    let pass = !heard.is_empty() && copy_share <= 0.5;
    let score = 0.6 * (heard.len() as f32 / new_bars.max(1) as f32 * 3.0).min(1.0)
        + 0.4 * (1.0 - copy_share);
    let words = if heard.is_empty() {
        format!("your opening not heard in the new bars; {copied} bars copied")
    } else {
        format!(
            "your opening heard, changed, in bars {}; {copied} copied",
            heard
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Check::new(pass, score, words)
}

/// The same notes in the same places.
fn same(a: &[GridNote], b: &[GridNote]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.on == y.on && x.pos == y.pos && x.alt == y.alt)
}

/// The longest run of new bars each a copy of the bar one, two or (your
/// bars) bars before it, against what the style allows.
fn repeats(h: &Heard) -> Check {
    let bars: Vec<Vec<GridNote>> = (0..h.bars)
        .map(|b| bar_of(h.notes, b as i32, h.grid.bar))
        .collect();
    let mut longest = 0;
    let mut at = h.your_bars;
    for period in [1, 2, h.your_bars.max(1)] {
        let mut run = 0;
        for b in h.your_bars..h.bars {
            if b >= period && !bars[b].is_empty() && same(&bars[b], &bars[b - period]) {
                run += 1;
                if run > longest {
                    longest = run;
                    at = b + 1 - run;
                }
            } else {
                run = 0;
            }
        }
    }
    let pass = longest <= h.copy_bars;
    let score = if pass {
        1.0
    } else {
        (1.0 - (longest - h.copy_bars) as f32 / 4.0).max(0.0)
    };
    let words = if longest == 0 {
        "no bar copied".into()
    } else {
        format!(
            "{longest} bars in a row copied from {} on (the style allows {})",
            bar_words(h, at as i32 * h.grid.bar),
            h.copy_bars
        )
    };
    Check::new(pass, score, words)
}

/// The new bars move well: no leap wider than a sixth, at most one
/// wider than a fourth in every two new bars, none followed by another
/// leap the same way, and no trill (two notes a step apart, back and
/// forth twice in quick notes of an 1/8 or less). The note before the
/// new bars counts as where they start from.
fn flows(h: &Heard, start: i32) -> Check {
    let from = h.notes.iter().rposition(|m| m.on < start).unwrap_or(0);
    let line: Vec<(i32, i32)> = h.notes[from..]
        .iter()
        .map(|m| (pitch(h, m), m.len))
        .collect();
    let steps: Vec<i32> = line.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let mut wide = 0usize;
    let mut leaps = 0usize;
    let mut on_and_on = 0usize;
    for (i, &d) in steps.iter().enumerate() {
        if d.abs() > 9 {
            wide += 1;
        }
        if d.abs() > 5 {
            leaps += 1;
            if steps
                .get(i + 1)
                .is_some_and(|&e| e.abs() > 4 && e.signum() == d.signum())
            {
                on_and_on += 1;
            }
        }
    }
    let quick = (h.grid.beat / 2).max(1);
    let trills = line
        .windows(4)
        .filter(|w| {
            w[0].0 == w[2].0
                && w[1].0 == w[3].0
                && (1..=2).contains(&(w[0].0 - w[1].0).abs())
                && w.iter().all(|x| x.1 <= quick)
        })
        .count();
    let allowed = ((h.bars - h.your_bars) / 2).max(1);
    let extra = leaps.saturating_sub(allowed);
    let pass = wide == 0 && extra == 0 && on_and_on == 0 && trills == 0;
    let score = 1.0
        - 0.5 * wide as f32
        - 0.25 * extra as f32
        - 0.3 * on_and_on as f32
        - 0.3 * trills as f32;
    let words = format!(
        "{leaps} leaps wider than a fourth ({allowed} allowed), {on_and_on} leaping on the same \
         way, {wide} wider than a sixth, {trills} trills"
    );
    Check::new(pass, score, words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_context::chord::{Chord, ChordAnswer, Heard as Made, Quality};
    use mpfree_context::key::Mode;
    use mpfree_transport::time::{Meter, PPQ};

    const C: Key = Key::new(0, Mode::Major);

    fn n(on: i32, len: i32, pos: i32) -> GridNote {
        GridNote {
            on,
            len,
            pos,
            alt: 0,
            velocity: 90,
        }
    }

    fn c_major(bars: usize) -> ChordTimeline {
        ChordTimeline::whole(
            bars as Tick * 4 * PPQ as Tick,
            ChordAnswer::Chord(Chord::new(0, Quality::Major)),
            Made::Heard,
        )
    }

    fn heard<'a>(notes: &'a [GridNote], bars: usize, chords: &'a ChordTimeline) -> Heard<'a> {
        Heard {
            notes,
            your_bars: 2,
            bars,
            grid: Grid::of(Meter::default()),
            key: C,
            chords,
            ending: Ending::Home,
            copy_bars: 2,
            landings: &[],
            rests: &[],
        }
    }

    /// Two bars of yours: C D E G | E D C (rest).
    fn yours_two() -> Vec<GridNote> {
        vec![
            n(0, 4, 0),
            n(4, 4, 1),
            n(8, 4, 2),
            n(12, 4, 4),
            n(16, 4, 2),
            n(20, 4, 1),
            n(24, 4, 0),
        ]
    }

    #[test]
    fn a_line_that_climbs_late_lands_and_breathes_passes() {
        let mut v = yours_two();
        // Bars 3-5: the opening again, a step higher each time, with a
        // rest on the fourth beat, then the high C; bar 6 comes home.
        for (b, lift) in [(2, 1), (3, 2), (4, 3)] {
            let o = b * 16;
            v.extend([n(o, 4, lift), n(o + 4, 4, 1 + lift), n(o + 8, 4, 2 + lift)]);
        }
        v.push(n(76, 2, 7));
        v.extend([n(80, 4, 4), n(84, 4, 2), n(88, 8, 0)]);
        let chords = c_major(6);
        let verdict = judge(&heard(&v, 6, &chords));
        assert!(verdict.somewhere.pass, "{}", verdict.words());
        assert!(verdict.lands.pass, "{}", verdict.words());
        assert!(verdict.breathes.pass, "{}", verdict.words());
        assert!(verdict.yours.pass, "{}", verdict.words());
        assert!(verdict.repeats.pass, "{}", verdict.words());
        assert!(verdict.flows.pass, "{}", verdict.words());
        assert_eq!(verdict.passed(), 6);
    }

    #[test]
    fn leaps_that_go_on_and_trills_do_not_flow() {
        let mut v = yours_two();
        // C4 up to A4 and on up to E5: two leaps the same way.
        v.extend([n(32, 4, 0), n(36, 4, 5), n(40, 4, 9)]);
        let chords = c_major(4);
        let verdict = judge(&heard(&v, 4, &chords));
        assert!(!verdict.flows.pass, "{}", verdict.words());
        // E D E D: a trill.
        let mut v = yours_two();
        v.extend([
            n(32, 2, 2),
            n(34, 2, 1),
            n(36, 2, 2),
            n(38, 2, 1),
            n(40, 8, 0),
        ]);
        let verdict = judge(&heard(&v, 4, &chords));
        assert!(!verdict.flows.pass, "{}", verdict.words());
        assert!(
            verdict.flows.words.contains("1 trills"),
            "{}",
            verdict.flows.words
        );
    }

    #[test]
    fn copying_your_bars_round_and_round_fails_repeats_and_somewhere() {
        let y = yours_two();
        let mut v = y.clone();
        for k in 1..4 {
            v.extend(y.iter().map(|m| GridNote {
                on: m.on + k * 32,
                ..*m
            }));
        }
        let chords = c_major(8);
        let verdict = judge(&heard(&v, 8, &chords));
        assert!(!verdict.repeats.pass, "{}", verdict.words());
        assert!(!verdict.somewhere.pass, "{}", verdict.words());
        assert!(
            !verdict.yours.pass,
            "copies don't count as heard: {}",
            verdict.words()
        );
    }

    #[test]
    fn a_line_that_never_stops_fails_breathes() {
        let mut v = yours_two();
        for k in 0..64 {
            v.push(n(32 + k, 1, k % 5));
        }
        let chords = c_major(6);
        let verdict = judge(&heard(&v, 6, &chords));
        assert!(!verdict.breathes.pass, "{}", verdict.words());
    }

    #[test]
    fn a_planned_landing_must_sound_where_planned() {
        let mut v = yours_two();
        v.extend([n(32, 4, 2), n(36, 4, 4), n(40, 8, 4), n(48, 16, 0)]);
        let chords = c_major(4);
        let landing = [Landing {
            on: 40,
            pos: 2,
            alt: 0,
        }];
        let mut h = heard(&v, 4, &chords);
        h.landings = &landing;
        let verdict = judge(&h);
        assert!(!verdict.lands.pass, "{}", verdict.words());
        assert!(verdict.lands.words.contains("missing"));
    }
}
