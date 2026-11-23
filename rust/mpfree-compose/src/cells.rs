//! Rhythm cells with named changes (revision 3, S2): a part's rhythm
//! built the way players build riffs, from one-beat cells, then brought
//! back changed where the row's plan asks.
//!
//! - A **cell** is one beat's starts on the 1/16 grid (the beat, its "e",
//!   "and" and "a"). Each style keeps a library of cells per kind of part
//!   (bass, comping, lines) with how much it likes each.
//! - A **riff** is one or two bars of cells, sampled to fit what the row
//!   wants at each step (a bass on Support wants the kick) and the style's
//!   density and syncopation, then laid over the whole row.
//! - **Named changes** bring it back different: keep, shift, push, split,
//!   join, drop, ghost, echo, mirror. [`develop`] makes them where the
//!   plan asks (a push into a landing, more notes at the high point, a
//!   fill or a held note at the end, a change every fourth bar), so the
//!   riff returns, never identical for long and never unrelated.
//! - **Ghost notes**: quiet, short repeats on the 1/16 before an accent,
//!   never on a kick or the backbeat ([`ghosts`]).

use crate::dice::Dice;
use crate::rhythm::{syncopation, Grid};
use crate::sketch::AnchorKind;
use crate::style::{BassLine, Comping, Feel, Style};

/// What kind of part a cell library is for (drums use the style's step
/// maps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CellPart {
    Bass,
    Comping,
    Line,
    /// A bass in two: the beat and the third beat held, a pickup now and
    /// then (Jazz-ish's bass on Support), whatever the style.
    TwoFeel,
    /// Chords on the off-beats (any style's comping on Interlock).
    Offbeats,
}

const ONE: u8 = 0b0001;
const E: u8 = 0b0010;
const AND: u8 = 0b0100;
const A: u8 = 0b1000;

/// Roots and fifths with the kick, the "a" leading into the next beat.
const SIXTEENTH_BASS: [(u8, f32); 10] = [
    (ONE, 3.0),
    (ONE | A, 2.5),
    (ONE | AND, 2.0),
    (ONE | AND | A, 1.5),
    (0, 0.8),
    (A, 0.8),
    (ONE | E | A, 0.8),
    (AND, 0.8),
    (ONE | E, 0.6),
    (ONE | E | AND | A, 0.2),
];
/// On the off-beat, as a house bass plays.
const OFFBEAT_BASS: [(u8, f32); 5] = [
    (AND, 4.0),
    (ONE | AND, 1.2),
    (ONE, 1.2),
    (0, 0.8),
    (AND | A, 0.4),
];
/// Quarter notes, now and then a swung pair.
const WALKING_BASS: [(u8, f32); 3] = [(ONE, 6.0), (ONE | AND, 0.8), (0, 0.2)];
/// A held chord: on the beat, or nothing.
const PAD: [(u8, f32); 2] = [(ONE, 2.0), (0, 4.0)];
/// Short chords, mostly off the beat.
const STABS: [(u8, f32); 6] = [
    (0, 3.0),
    (AND, 2.0),
    (ONE, 1.5),
    (A, 0.8),
    (ONE | A, 0.6),
    (AND | A, 0.5),
];
/// The beat and the "and" before the next: comping.
const CHARLESTON: [(u8, f32); 4] = [(0, 3.0), (ONE, 2.0), (AND, 2.0), (ONE | AND, 0.4)];
const PLAIN_LINE: [(u8, f32); 4] = [(ONE, 3.0), (ONE | AND, 2.0), (0, 1.5), (AND, 1.0)];
const PUSHED_LINE: [(u8, f32); 8] = [
    (ONE | AND, 2.0),
    (ONE, 1.5),
    (ONE | A, 1.5),
    (ONE | E | AND, 0.8),
    (AND, 1.0),
    (0, 1.2),
    (ONE | AND | A, 1.0),
    (A, 0.6),
];
const SWUNG_LINE: [(u8, f32); 4] = [(ONE | AND, 2.5), (ONE, 2.0), (AND, 1.5), (0, 1.2)];
/// Half notes with pickups.
const TWO_FEEL: [(u8, f32); 4] = [(ONE, 4.0), (0, 2.5), (ONE | A, 0.6), (AND, 0.6)];
/// The "and" of the beat, as house stabs and comping play.
const OFFBEATS: [(u8, f32); 4] = [(AND, 4.0), (0, 1.2), (AND | A, 0.5), (E | AND, 0.3)];

/// The cells `style` plays on `part`, with how much it likes each.
pub fn library(style: &Style, part: CellPart) -> &'static [(u8, f32)] {
    match part {
        CellPart::Bass => match style.bass.line {
            BassLine::Offbeat => &OFFBEAT_BASS,
            BassLine::Sixteenths => &SIXTEENTH_BASS,
            BassLine::Walking => &WALKING_BASS,
        },
        CellPart::Comping => match style.bass.comping {
            Comping::Pad => &PAD,
            Comping::Stabs => &STABS,
            Comping::Charleston => &CHARLESTON,
        },
        CellPart::TwoFeel => &TWO_FEEL,
        CellPart::Offbeats => &OFFBEATS,
        CellPart::Line => match style.rhythm.feel {
            Feel::Simplify => &PLAIN_LINE,
            Feel::Push => &PUSHED_LINE,
            Feel::Swing8 => &SWUNG_LINE,
        },
    }
}

/// The steps a cell starts on, within a beat of `beat` steps. On a beat of
/// two steps (6/8's eighths) only the beat and its "and" remain.
pub fn starts_of(cell: u8, beat: i32) -> impl Iterator<Item = i32> {
    (0..4)
        .filter(move |k| cell >> k & 1 == 1)
        .filter_map(move |k| {
            if beat % 4 == 0 {
                Some(k * beat / 4)
            } else if beat % 2 == 0 {
                (k % 2 == 0).then_some(k / 2 * beat / 2)
            } else {
                (k == 0).then_some(0)
            }
        })
}

/// One stroke of a part's rhythm over the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stroke {
    /// Steps from the row's start.
    pub at: i32,
    /// Steps until the next stroke (or the end of a rest).
    pub len: i32,
    /// A ghost note: quiet and short.
    pub ghost: bool,
    /// Started early, tied over the beat it belongs to.
    pub pushed: bool,
}

impl Stroke {
    pub const fn at(at: i32) -> Stroke {
        Stroke {
            at,
            len: 1,
            ghost: false,
            pushed: false,
        }
    }
}

/// What a riff aims at, besides the row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    /// Notes per beat.
    pub density: f32,
    /// 0 to 1, as [`syncopation`] measures it.
    pub syncopation: f32,
}

/// A riff of `riff_bars` bars of cells (one cell per beat) from `lib`,
/// fitting `desire` (how much the row wants a note at each of its steps,
/// folded onto the riff, since the riff repeats), the style's liking and
/// `aim`. Of 24 sampled riffs the best is kept; `wander` (from Variation)
/// lets a slightly worse one win now and then.
pub fn riff(
    lib: &[(u8, f32)],
    grid: Grid,
    riff_bars: usize,
    desire: &[f32],
    aim: Aim,
    wander: f64,
    dice: &mut Dice,
) -> Vec<u8> {
    let bpb = (grid.bar / grid.beat).max(1) as usize;
    let beats = riff_bars.max(1) * bpb;
    let rs = beats as i32 * grid.beat;
    let n = desire.len() as i32;
    let reps = (n / rs).max(1);
    let folded: Vec<f32> = (0..rs)
        .map(|k| {
            (0..reps)
                .filter_map(|r| desire.get((r * rs + k) as usize))
                .sum::<f32>()
                / reps as f32
        })
        .collect();
    let fit = |j: usize, cell: u8| -> f32 {
        starts_of(cell, grid.beat)
            .map(|s| folded[j * grid.beat as usize + s as usize])
            .sum()
    };
    let liking = |cell: u8| {
        lib.iter()
            .find(|(c, _)| *c == cell)
            .map_or(0.01, |&(_, w)| w)
    };
    let score = |cells: &[u8]| -> f64 {
        let starts: Vec<i32> = cells
            .iter()
            .enumerate()
            .flat_map(|(j, &c)| starts_of(c, grid.beat).map(move |s| j as i32 * grid.beat + s))
            .collect();
        let fits: f32 = cells.iter().enumerate().map(|(j, &c)| fit(j, c)).sum();
        let likes: f64 = cells.iter().map(|&c| f64::from(liking(c)).ln()).sum();
        let density = starts.len() as f32 / beats as f32;
        let sync = per_bar_syncopation(&starts, grid);
        // A riff of two bars or more plays in each of them.
        let empty = if starts.len() >= riff_bars {
            (0..riff_bars as i32)
                .filter(|&b| !starts.iter().any(|&s| s / grid.bar == b))
                .count()
        } else {
            0
        };
        f64::from(fits) + 0.4 * likes
            - 3.0 * f64::from((density - aim.density).abs()) * riff_bars as f64
            - 2.0 * f64::from((sync - aim.syncopation).abs())
            - 1.5 * empty as f64
    };
    let mut best: Option<(f64, Vec<u8>)> = None;
    for _ in 0..24 {
        let cells: Vec<u8> = (0..beats)
            .map(|j| {
                let w: Vec<f64> = lib
                    .iter()
                    .map(|&(c, lw)| f64::from(lw) * (f64::from(fit(j, c)) * 1.5).exp())
                    .collect();
                lib[dice.pick(&w).unwrap_or(0)].0
            })
            .collect();
        let s = score(&cells) + dice.gumbel() * wander;
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            best = Some((s, cells));
        }
    }
    best.map(|(_, c)| c).unwrap_or_default()
}

fn per_bar_syncopation(starts: &[i32], grid: Grid) -> f32 {
    let bars = starts.iter().map(|s| s / grid.bar).max().unwrap_or(0) + 1;
    let total: f64 = (0..bars)
        .map(|b| {
            let s: Vec<i32> = starts
                .iter()
                .filter(|&&s| s / grid.bar == b)
                .map(|s| s % grid.bar)
                .collect();
            syncopation(&s, grid)
        })
        .sum();
    (total / f64::from(bars.max(1))) as f32
}

/// A riff laid over `steps` steps of the row, from its start.
pub fn lay(cells: &[u8], grid: Grid, steps: i32) -> Vec<Stroke> {
    let rs = cells.len() as i32 * grid.beat;
    if rs <= 0 {
        return Vec::new();
    }
    let mut v = Vec::new();
    let mut o = 0;
    while o < steps {
        for (j, &c) in cells.iter().enumerate() {
            for s in starts_of(c, grid.beat) {
                let at = o + j as i32 * grid.beat + s;
                if at < steps {
                    v.push(Stroke::at(at));
                }
            }
        }
        o += rs;
    }
    lengths(&mut v, steps);
    v
}

/// Sort the strokes, keep one per step (a real note over a ghost) and
/// make each last until the next, the last round the loop into the first.
pub fn lengths(strokes: &mut Vec<Stroke>, steps: i32) {
    strokes.retain(|s| (0..steps).contains(&s.at));
    strokes.sort_by_key(|s| (s.at, s.ghost));
    strokes.dedup_by_key(|s| s.at);
    let n = strokes.len();
    for i in 0..n {
        let next = if i + 1 < n {
            strokes[i + 1].at
        } else {
            strokes.first().map_or(steps, |f| f.at + steps)
        };
        strokes[i].len = (next - strokes[i].at).max(1);
    }
}

/// The named changes a riff comes back with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    Keep,
    /// Every stroke a 1/16 later.
    Shift,
    /// A note on the beat started early, tied over it.
    Push,
    /// The longest note split in two.
    Split,
    /// A short off-beat note joined to the one before.
    Join,
    /// The weakest note left out, for a rest.
    Drop,
    /// A quiet repeat before an accent.
    Ghost,
    /// Another part's starts, a beat later.
    Echo,
    /// Notes where another part rests.
    Mirror,
}

impl Change {
    pub const fn words(self) -> &'static str {
        match self {
            Change::Keep => "keep",
            Change::Shift => "shift",
            Change::Push => "push",
            Change::Split => "split",
            Change::Join => "join",
            Change::Drop => "drop",
            Change::Ghost => "ghost",
            Change::Echo => "echo",
            Change::Mirror => "mirror",
        }
    }
}

/// Where a change may reach: steps `[from, to)` of the row, on the grid
/// `grid`, in units of `unit` steps (1 on 1/16s, 2 on swung 1/8s).
#[derive(Clone, Copy, Debug)]
pub struct Span<'a> {
    pub from: i32,
    pub to: i32,
    pub grid: Grid,
    pub unit: i32,
    /// Per row step: another part starts a note there (for echo and
    /// mirror).
    pub other: &'a [bool],
    /// Per row step: no ghost may go there (a kick, the backbeat).
    pub blocked: &'a [bool],
}

impl Span<'_> {
    fn has(&self, step: i32) -> bool {
        self.from <= step && step < self.to
    }

    fn flag(v: &[bool], step: i32) -> bool {
        v.get(step as usize).copied().unwrap_or(false)
    }
}

/// Apply `change` within `span`. Whether anything changed.
pub fn apply(strokes: &mut Vec<Stroke>, change: Change, span: &Span, dice: &mut Dice) -> bool {
    let steps = strokes
        .iter()
        .map(|s| s.at + s.len)
        .max()
        .unwrap_or(span.to)
        .max(span.to);
    let inside: Vec<usize> = (0..strokes.len())
        .filter(|&i| span.has(strokes[i].at))
        .collect();
    let taken = |v: &[Stroke], s: i32| v.iter().any(|x| x.at == s);
    let grid = span.grid;
    let changed = match change {
        Change::Keep => false,
        Change::Shift => {
            let mut moved = false;
            for &i in inside.iter().rev() {
                let to = strokes[i].at + 1;
                if span.has(to) && !taken(strokes, to) {
                    strokes[i].at = to;
                    moved = true;
                }
            }
            moved
        }
        Change::Push => {
            // The latest note on a beat (not the span's first step) moves
            // a unit earlier, if that step is free.
            let pick = inside.iter().rev().copied().find(|&i| {
                let at = strokes[i].at;
                grid.on_beat(at) && at - span.unit >= span.from && !taken(strokes, at - span.unit)
            });
            match pick {
                Some(i) => push_stroke(strokes, i, span.unit),
                None => false,
            }
        }
        Change::Split => {
            let pick = inside
                .iter()
                .copied()
                .filter(|&i| strokes[i].len >= 2 && !strokes[i].ghost)
                .max_by_key(|&i| (strokes[i].len, std::cmp::Reverse(strokes[i].at)));
            match pick {
                Some(i) => {
                    let s = strokes[i];
                    let half = if s.len >= 4 { 2 } else { 1 }.max(span.unit);
                    let mid = (s.at + s.len / 2) / half * half;
                    let mid = if mid <= s.at { s.at + half } else { mid };
                    if span.has(mid) && mid < s.at + s.len && !taken(strokes, mid) {
                        strokes.push(Stroke::at(mid));
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        }
        Change::Join => {
            let pick = inside
                .iter()
                .copied()
                .filter(|&i| i > 0 && !grid.on_beat(strokes[i].at))
                .min_by_key(|&i| (strokes[i - 1].len, strokes[i].at));
            match pick {
                Some(i) => {
                    strokes.remove(i);
                    true
                }
                None => false,
            }
        }
        Change::Drop => {
            let pick = inside
                .iter()
                .copied()
                .filter(|&i| strokes[i].at % grid.bar != 0)
                .min_by_key(|&i| (grid.level(strokes[i].at), dice.below(4)));
            match pick {
                Some(i) => {
                    strokes.remove(i);
                    true
                }
                None => false,
            }
        }
        Change::Ghost => {
            let spots: Vec<i32> = inside
                .iter()
                .map(|&i| strokes[i])
                .filter(|s| !s.ghost && grid.on_beat(s.at))
                .map(|s| s.at - 1)
                .filter(|&g| span.has(g) && !taken(strokes, g) && !Span::flag(span.blocked, g))
                .collect();
            if spots.is_empty() {
                false
            } else {
                let g = spots[dice.below(spots.len())];
                strokes.push(Stroke {
                    ghost: true,
                    ..Stroke::at(g)
                });
                true
            }
        }
        Change::Echo => {
            let mut added = false;
            for s in span.from..span.to {
                let to = s + grid.beat;
                if Span::flag(span.other, s) && span.has(to) && !taken(strokes, to) {
                    strokes.push(Stroke::at(to));
                    added = true;
                }
            }
            added
        }
        Change::Mirror => {
            // Notes where the other part leaves a gap, on the unit's grid:
            // the first free unit after each of its notes.
            let mut added = false;
            let u = span.unit.max(1);
            let mut s = span.from.div_euclid(u) * u;
            while s < span.to {
                let gap = !Span::flag(span.other, s) && Span::flag(span.other, s - u);
                if gap && span.has(s) && !taken(strokes, s) {
                    strokes.push(Stroke::at(s));
                    added = true;
                }
                s += u;
            }
            added
        }
    };
    if changed {
        lengths(strokes, steps);
    }
    changed
}

fn push_stroke(strokes: &mut [Stroke], i: usize, unit: i32) -> bool {
    let s = &mut strokes[i];
    s.at -= unit;
    s.pushed = true;
    true
}

/// Push the stroke at `step` a unit earlier, if there is one and the
/// step before is free.
pub fn push_at(strokes: &mut Vec<Stroke>, step: i32, unit: i32, steps: i32) -> bool {
    let free = !strokes.iter().any(|s| s.at == step - unit);
    match strokes.iter().position(|s| s.at == step) {
        Some(i) if free && step - unit >= 0 => {
            push_stroke(strokes, i, unit);
            lengths(strokes, steps);
            true
        }
        _ => false,
    }
}

/// Ghost notes: a quiet repeat on the 1/16 before a note on the beat,
/// with chance `chance` each, never on a blocked step (a kick or the
/// backbeat) or where a note already starts.
pub fn ghosts(
    strokes: &mut Vec<Stroke>,
    blocked: &[bool],
    chance: f64,
    steps: i32,
    grid: Grid,
    dice: &mut Dice,
) -> usize {
    let spots: Vec<i32> = strokes
        .iter()
        .filter(|s| !s.ghost && grid.on_beat(s.at) && s.at > 0)
        .map(|s| s.at - 1)
        .collect();
    let mut added = 0;
    for g in spots {
        let free = !strokes.iter().any(|s| s.at == g);
        if free && !blocked.get(g as usize).copied().unwrap_or(false) && dice.chance(chance) {
            strokes.push(Stroke {
                ghost: true,
                ..Stroke::at(g)
            });
            added += 1;
        }
    }
    if added > 0 {
        lengths(strokes, steps);
    }
    added
}

/// Where the row's plan asks a laid riff to change, and how a style
/// plays those places.
#[derive(Clone, Copy, Debug)]
pub struct Develop<'a> {
    pub grid: Grid,
    pub bars: usize,
    /// The plan's marks: (kind, step).
    pub marks: &'a [(AnchorKind, i32)],
    /// The part lands with the tune: a note on each landing.
    pub lands: bool,
    /// How often a note on a landing is pushed in early.
    pub push: f64,
    /// How the style ends: a fill (Groove), a held note (Minimal) or an
    /// anticipation into the first bar (Jazz-ish).
    pub feel: Feel,
    pub unit: i32,
    pub other: &'a [bool],
    pub blocked: &'a [bool],
}

/// Bring a laid riff back changed where the plan asks: a note on (or
/// pushed into) each landing, more at the high point, the end as the style
/// ends, and one change every fourth bar. The changes made, by bar.
pub fn develop(strokes: &mut Vec<Stroke>, d: &Develop, dice: &mut Dice) -> Vec<(usize, Change)> {
    let steps = d.bars as i32 * d.grid.bar;
    let mut made = Vec::new();
    let span_of = |b: usize| Span {
        from: b as i32 * d.grid.bar,
        to: (b as i32 + 1) * d.grid.bar,
        grid: d.grid,
        unit: d.unit,
        other: d.other,
        blocked: d.blocked,
    };
    let mut changed_bars = vec![false; d.bars];
    for &(kind, at) in d.marks {
        let b = (at / d.grid.bar) as usize;
        if b >= d.bars {
            continue;
        }
        match kind {
            AnchorKind::Landing | AnchorKind::Close => {
                if d.lands && !strokes.iter().any(|s| s.at == at) {
                    strokes.push(Stroke::at(at));
                    lengths(strokes, steps);
                }
                if d.grid.on_beat(at)
                    && at > 0
                    && dice.chance(d.push)
                    && push_at(strokes, at, d.unit, steps)
                {
                    made.push((b, Change::Push));
                    changed_bars[b] = true;
                }
            }
            AnchorKind::Peak => {
                if dice.chance(0.7) && apply(strokes, Change::Split, &span_of(b), dice) {
                    made.push((b, Change::Split));
                    changed_bars[b] = true;
                }
            }
        }
    }
    // The end, as the style ends.
    if d.bars >= 2 {
        let last = d.bars - 1;
        let s = span_of(last);
        let end_beat = Span {
            from: s.to - d.grid.beat,
            ..s
        };
        let change = match d.feel {
            Feel::Push => Change::Split,
            Feel::Simplify => Change::Drop,
            Feel::Swing8 => Change::Push,
        };
        let done = match change {
            Change::Split => {
                let a = apply(strokes, Change::Split, &end_beat, dice);
                let b = apply(strokes, Change::Split, &end_beat, dice);
                a || b
            }
            _ => apply(strokes, change, &s, dice),
        };
        if done {
            made.push((last, change));
            changed_bars[last] = true;
        }
    }
    // Every fourth bar, one change, where nothing changed yet.
    for b in (3..d.bars.saturating_sub(1)).step_by(4) {
        if changed_bars[b] {
            continue;
        }
        let options: [(Change, f64); 4] = match d.feel {
            Feel::Push => [
                (Change::Split, 2.0),
                (Change::Ghost, 1.5),
                (Change::Drop, 1.0),
                (Change::Shift, 0.3),
            ],
            Feel::Simplify => [
                (Change::Drop, 2.0),
                (Change::Split, 1.0),
                (Change::Ghost, 0.2),
                (Change::Shift, 0.2),
            ],
            Feel::Swing8 => [
                (Change::Push, 1.5),
                (Change::Split, 1.5),
                (Change::Drop, 1.0),
                (Change::Ghost, 0.3),
            ],
        };
        let w: Vec<f64> = options.iter().map(|o| o.1).collect();
        let c = options[dice.pick(&w).unwrap_or(0)].0;
        if apply(strokes, c, &span_of(b), dice) {
            made.push((b, c));
        }
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::{DiceName, Job};
    use crate::style::{GROOVE, JAZZISH, MINIMAL};
    use mpfree_transport::time::Meter;

    fn dice(n: u16) -> Dice {
        DiceName {
            seed: 7,
            part: 1,
            phrase: 0,
            job: Job::Cells,
            bar: n,
            attempt: 0,
        }
        .dice()
    }

    fn four() -> Grid {
        Grid::of(Meter::default())
    }

    fn starts(v: &[Stroke]) -> Vec<i32> {
        v.iter().map(|s| s.at).collect()
    }

    #[test]
    fn a_cell_keeps_its_eighths_on_a_beat_of_two_steps() {
        assert_eq!(
            starts_of(ONE | AND | A, 4).collect::<Vec<_>>(),
            vec![0, 2, 3]
        );
        assert_eq!(starts_of(ONE | AND | A, 2).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(starts_of(E, 2).count(), 0);
    }

    #[test]
    fn a_riff_follows_what_the_row_wants_and_rolls_the_same_twice() {
        // Two bars that want notes on a boom-bap kick: 1, the "and" of 2,
        // the "and" of 3.
        let mut desire = vec![-0.3f32; 32];
        for b in 0..2 {
            for k in [0, 6, 10] {
                desire[b * 16 + k] = 2.0;
            }
        }
        let aim = Aim {
            density: 0.9,
            syncopation: 0.5,
        };
        let lib = library(&GROOVE, CellPart::Bass);
        let r = riff(lib, four(), 1, &desire, aim, 0.3, &mut dice(1));
        assert_eq!(r, riff(lib, four(), 1, &desire, aim, 0.3, &mut dice(1)));
        let laid = lay(&r, four(), 32);
        let on_kicks = [0, 6, 10]
            .iter()
            .filter(|k| laid.iter().any(|s| s.at == **k))
            .count();
        assert!(on_kicks >= 2, "{r:?} -> {:?}", starts(&laid));
        // The riff repeats in the second bar.
        let first: Vec<i32> = laid.iter().filter(|s| s.at < 16).map(|s| s.at).collect();
        let second: Vec<i32> = laid
            .iter()
            .filter(|s| s.at >= 16)
            .map(|s| s.at - 16)
            .collect();
        assert_eq!(first, second);
    }

    #[test]
    fn walking_and_pads_keep_to_their_cells() {
        let desire = vec![0.0f32; 16];
        let aim = Aim {
            density: 1.0,
            syncopation: 0.0,
        };
        let w = riff(
            library(&JAZZISH, CellPart::Bass),
            four(),
            1,
            &desire,
            aim,
            0.0,
            &mut dice(2),
        );
        assert!(
            w.iter().all(|&c| c & (E | A) == 0),
            "swung eighths only: {w:?}"
        );
        let p = riff(
            library(&MINIMAL, CellPart::Comping),
            four(),
            1,
            &desire,
            aim,
            0.0,
            &mut dice(3),
        );
        assert!(p.iter().all(|&c| c == ONE || c == 0), "{p:?}");
    }

    fn span<'a>(from: i32, to: i32, other: &'a [bool], blocked: &'a [bool]) -> Span<'a> {
        Span {
            from,
            to,
            grid: four(),
            unit: 1,
            other,
            blocked,
        }
    }

    #[test]
    fn each_named_change_does_what_it_says() {
        let none = vec![false; 16];
        let base = || {
            let mut v = vec![Stroke::at(0), Stroke::at(4), Stroke::at(8), Stroke::at(10)];
            lengths(&mut v, 16);
            v
        };
        let mut v = base();
        assert!(apply(
            &mut v,
            Change::Push,
            &span(0, 16, &none, &none),
            &mut dice(4)
        ));
        assert_eq!(
            starts(&v),
            vec![0, 4, 7, 10],
            "the last note on a beat, a 1/16 early"
        );
        assert!(v.iter().any(|s| s.pushed && s.at == 7));

        let mut v = base();
        assert!(apply(
            &mut v,
            Change::Split,
            &span(0, 16, &none, &none),
            &mut dice(4)
        ));
        assert_eq!(starts(&v), vec![0, 4, 8, 10, 12], "the longest note split");

        let mut v = base();
        assert!(apply(
            &mut v,
            Change::Drop,
            &span(0, 16, &none, &none),
            &mut dice(4)
        ));
        assert_eq!(starts(&v), vec![0, 4, 8], "the weakest note goes");

        let mut v = base();
        assert!(apply(
            &mut v,
            Change::Join,
            &span(0, 16, &none, &none),
            &mut dice(4)
        ));
        assert_eq!(starts(&v), vec![0, 4, 8]);

        let mut blocked = vec![false; 16];
        blocked[3] = true;
        let mut v = base();
        assert!(apply(
            &mut v,
            Change::Ghost,
            &span(0, 16, &none, &blocked),
            &mut dice(4)
        ));
        let g: Vec<i32> = v.iter().filter(|s| s.ghost).map(|s| s.at).collect();
        assert_eq!(g, vec![7], "before the beat at 8; 3 is blocked");

        let mut other = vec![false; 16];
        other[0] = true;
        other[1] = true;
        let mut v = vec![Stroke::at(8)];
        assert!(apply(
            &mut v,
            Change::Echo,
            &span(0, 16, &other, &none),
            &mut dice(4)
        ));
        assert_eq!(starts(&v), vec![4, 5, 8], "its starts a beat later");

        let mut v = vec![];
        assert!(apply(
            &mut v,
            Change::Mirror,
            &span(0, 8, &other, &none),
            &mut dice(4)
        ));
        assert_eq!(starts(&v), vec![2], "the first gap after the other's notes");

        let mut v = vec![Stroke::at(0), Stroke::at(5)];
        lengths(&mut v, 16);
        assert!(apply(
            &mut v,
            Change::Shift,
            &span(0, 16, &none, &none),
            &mut dice(4)
        ));
        assert_eq!(starts(&v), vec![1, 6]);
        assert!(!apply(
            &mut v,
            Change::Keep,
            &span(0, 16, &none, &none),
            &mut dice(4)
        ));
    }

    #[test]
    fn ghosts_go_before_accents_and_never_on_a_kick_or_the_backbeat() {
        let mut blocked = vec![false; 32];
        for k in [3, 11, 19, 27] {
            blocked[k] = true;
        }
        let mut v = lay(&[ONE, ONE, ONE, ONE], four(), 32);
        let n = ghosts(&mut v, &blocked, 1.0, 32, four(), &mut dice(5));
        assert!(n > 0);
        for s in v.iter().filter(|s| s.ghost) {
            assert!(!blocked[s.at as usize], "{s:?}");
            assert_eq!(s.at % 4, 3, "on the 1/16 before a beat: {s:?}");
        }
    }

    #[test]
    fn the_riff_comes_back_changed_where_the_plan_asks() {
        let none = vec![false; 128];
        let r = vec![ONE | A, ONE, ONE | AND, AND];
        let mut v = lay(&r, four(), 128);
        let plain = v.clone();
        let marks = [
            (AnchorKind::Landing, 3 * 16),
            (AnchorKind::Peak, 5 * 16 + 8),
            (AnchorKind::Close, 7 * 16 + 8),
        ];
        let d = Develop {
            grid: four(),
            bars: 8,
            marks: &marks,
            lands: true,
            push: 1.0,
            feel: Feel::Push,
            unit: 1,
            other: &none,
            blocked: &none,
        };
        let made = develop(&mut v, &d, &mut dice(6));
        assert!(
            made.iter().any(|&(b, c)| b == 3 && c == Change::Push),
            "{made:?}"
        );
        assert!(
            made.iter().any(|&(b, _)| b == 7),
            "the end changes: {made:?}"
        );
        let bar = |v: &[Stroke], b: i32| -> Vec<i32> {
            v.iter()
                .filter(|s| s.at / 16 == b)
                .map(|s| s.at - b * 16)
                .collect()
        };
        // It returns: some bars as laid, and never all of them.
        let same = (0..8).filter(|&b| bar(&v, b) == bar(&plain, b)).count();
        assert!((3..8).contains(&same), "{same} bars unchanged");
        assert!(
            v.iter()
                .any(|s| s.at == 7 * 16 + 8 || (s.at == 7 * 16 + 7 && s.pushed)),
            "a note on the close, or pushed into it"
        );
        assert!(v.iter().all(|s| (0..128).contains(&s.at)));
    }
}
