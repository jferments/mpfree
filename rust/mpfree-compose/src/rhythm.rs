//! Rhythm (phase 0C): one bar's starts and lengths on the 1/16 grid, how
//! syncopated they are, and the ways a bar's rhythm is kept, varied,
//! reshaped by the style's feel, or made new. The second prototype's,
//! for any meter.

use mpfree_context::groove::STEP;
use mpfree_transport::time::Meter;

use crate::dice::Dice;
use crate::motif::GridNote;
use crate::style::{Feel, RhythmStyle};

/// A bar's grid: its length and its beat, in 1/16 steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Grid {
    pub bar: i32,
    pub beat: i32,
}

impl Grid {
    pub fn of(meter: Meter) -> Grid {
        let step = STEP.max(1);
        Grid {
            bar: (meter.bar_ticks() / step).max(1) as i32,
            beat: (meter.beat_ticks() / step).max(1) as i32,
        }
    }

    /// How strong a step is: 0 on the bar line, -1 halfway through a bar
    /// of an even number of beats (four or more), -2 on a beat, -3 on an
    /// 1/8 and -4 between. In 4/4 that is the prototype's table.
    pub fn level(self, step: i32) -> i32 {
        let s = step.rem_euclid(self.bar);
        let beats = self.bar / self.beat;
        if s == 0 {
            0
        } else if beats >= 4 && beats % 2 == 0 && s == self.bar / 2 {
            -1
        } else if s % self.beat == 0 {
            -2
        } else if s % 2 == 0 {
            -3
        } else {
            -4
        }
    }

    /// The middle of the bar, where a call hands over to its answer.
    pub fn half(self) -> i32 {
        (self.bar / 2).max(1)
    }

    pub fn on_beat(self, step: i32) -> bool {
        step.rem_euclid(self.beat) == 0
    }
}

/// One note's place in a bar's rhythm, in steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Onset {
    pub on: i32,
    pub len: i32,
}

/// Starts, each lasting until the next (the last until the bar's end).
fn from_starts(on: &[i32], grid: Grid) -> Vec<Onset> {
    (0..on.len())
        .map(|i| Onset {
            on: on[i],
            len: on.get(i + 1).copied().unwrap_or(grid.bar) - on[i],
        })
        .collect()
}

/// Longuet-Higgins and Lee's syncopation of one bar's starts, 0 to 1: a
/// note held over a stronger step than its own.
pub fn syncopation(starts: &[i32], grid: Grid) -> f64 {
    if starts.is_empty() {
        return 0.0;
    }
    let mut s = 0;
    for (i, &a) in starts.iter().enumerate() {
        let b = starts.get(i + 1).copied().unwrap_or(grid.bar).min(grid.bar);
        if let Some(strongest) = (a + 1..b).map(|p| grid.level(p)).max() {
            s += (strongest - grid.level(a)).max(0);
        }
    }
    (f64::from(s) / (starts.len() as f64 * 3.0)).min(1.0)
}

/// A bar's rhythm: each note until the next note or its own end.
pub fn rhythm_of(bar: &[GridNote], grid: Grid) -> Vec<Onset> {
    (0..bar.len())
        .map(|i| {
            let on = bar[i].on;
            let next = bar.get(i + 1).map_or(grid.bar, |x| x.on);
            let end = next.min(on + bar[i].len.max(1)).max(on + 1);
            Onset { on, len: end - on }
        })
        .collect()
}

/// Small changes to a kept rhythm: split a note, join two, nudge one off
/// the beat by a step. `amount` sets how many; busier activity splits
/// more, calmer joins more.
pub fn vary(r: &mut Vec<Onset>, amount: f64, activity: f64, grid: Grid, dice: &mut Dice) {
    let split = amount * (0.5 + (activity - 1.0).max(0.0));
    let join = amount * (0.5 + (1.0 - activity).max(0.0));
    let mut out: Vec<Onset> = Vec::with_capacity(r.len() + 4);
    let mut i = 0;
    while i < r.len() {
        let Onset { on, len } = r[i];
        if len >= 2 && len % 2 == 0 && dice.chance(split * 0.5) {
            out.push(Onset { on, len: len / 2 });
            out.push(Onset {
                on: on + len / 2,
                len: len / 2,
            });
        } else if i > 0
            && i + 1 < r.len()
            && len <= 2
            && r[i + 1].len <= 2
            && dice.chance(join * 0.4)
        {
            out.push(Onset {
                on,
                len: len + r[i + 1].len,
            });
            i += 1;
        } else if i > 0 && !grid.on_beat(on) && dice.chance(amount * 0.15) {
            let s = if dice.chance(0.5) { 1 } else { -1 };
            let after = out.last().map_or(0, |x| x.on + 1);
            let before = r.get(i + 1).map_or(grid.bar, |x| x.on);
            let moved = (on + s).clamp(after, (before - 1).max(after));
            out.push(Onset {
                on: moved,
                len: (len - (moved - on)).max(1),
            });
        } else {
            out.push(r[i]);
        }
        i += 1;
    }
    *r = out;
}

/// A new rhythm, for a contrast: steps ranked by strength mixed with the
/// style's syncopation and some chance, as many as the activity asks;
/// of 24 tries, the one nearest the style's syncopation that shares
/// fewest starts with `avoid` (your bar).
pub fn fresh(
    style: &RhythmStyle,
    activity: f64,
    avoid: &[i32],
    grid: Grid,
    dice: &mut Dice,
) -> Vec<Onset> {
    let beats = (grid.bar / grid.beat).max(1);
    let want = ((f64::from(style.density) * f64::from(beats) * activity).round() as usize)
        .clamp(2, (grid.bar as usize).saturating_sub(2).max(2));
    let sync = f64::from(style.syncopation);
    let mut best: (f64, Vec<i32>) = (f64::MAX, vec![0, grid.half()]);
    for _ in 0..24 {
        let mut ranked: Vec<(f64, i32)> = (0..grid.bar)
            .map(|s| {
                let strength = f64::from(grid.level(s) + 4) / 4.0;
                (
                    strength * (1.0 - sync) + (1.0 - strength) * sync + 0.6 * dice.unit(),
                    s,
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        // The bar's first step always sounds: a new rhythm stands on the
        // downbeat, or the whole bar sounds pushed late.
        let mut on: Vec<i32> = vec![0];
        on.extend(
            ranked
                .iter()
                .map(|x| x.1)
                .filter(|&s| s != 0)
                .take(want - 1),
        );
        // Now and then a beat left silent.
        if dice.chance(f64::from(style.rest)) {
            let beat = dice.below(beats as usize) as i32 * grid.beat;
            on.retain(|&s| s < beat || s >= beat + grid.beat || s == 0);
        }
        on.sort_unstable();
        // Unlike your bar off the beat; sharing its beats is fine.
        let shared = on
            .iter()
            .filter(|&&o| !grid.on_beat(o) && avoid.contains(&o))
            .count() as f64
            / on.len() as f64;
        // A note a 1/16 after a beat that doesn't run straight on into the
        // next 1/16 sounds like a late beat, not a syncopation (a 1/16
        // before a beat, pushing into it, is the syncopation).
        let late = on.iter().filter(|&&o| is_late(o, &on, grid)).count() as f64 / on.len() as f64;
        let score = (syncopation(&on, grid) - sync).abs() + 0.5 * shared + LATE_COST * late;
        if score < best.0 {
            best = (score, on);
        }
    }
    from_starts(&best.1, grid)
}

/// What a fresh rhythm pays for its share of late notes.
const LATE_COST: f64 = 1.2;

/// Whether a note starting at `o` sounds like a late beat: a 1/16 after
/// a beat, with nothing on the 1/16 after it.
fn is_late(o: i32, on: &[i32], grid: Grid) -> bool {
    grid.level(o) == -4 && o.rem_euclid(grid.beat) == 1 && !on.contains(&(o + 1))
}

/// Thin or fill a rhythm to about `want` notes. Thinner drops the weakest
/// notes (the weakest steps and shortest notes, never the bar's first),
/// each leaving a rest or, half the time, a longer note before it.
/// Fuller splits the longest notes in two at the strongest step inside
/// them that is a multiple of `unit` steps (2 keeps a swung line on its
/// 1/8s). The plan's activity and the Activity setting work through this.
pub fn density_to(r: &mut Vec<Onset>, want: usize, unit: i32, grid: Grid, dice: &mut Dice) {
    let unit = unit.max(1);
    let want = want.clamp(1, grid.bar.max(1) as usize);
    while r.len() > want && r.len() > 1 {
        let scores: Vec<(usize, f64)> = (1..r.len())
            .map(|i| {
                let x = r[i];
                let s = f64::from(grid.level(x.on) + 5) + 0.4 * f64::from(x.len) + dice.unit();
                (i, s)
            })
            .collect();
        let Some(&(i, _)) = scores.iter().min_by(|a, b| a.1.total_cmp(&b.1)) else {
            break;
        };
        let gone = r.remove(i);
        let p = &mut r[i - 1];
        if p.on + p.len == gone.on && dice.chance(0.5) {
            p.len += gone.len;
        }
    }
    while r.len() < want {
        let inside = |x: Onset| (x.on + 1..x.on + x.len).filter(move |p| p % unit == 0);
        let scores: Vec<(usize, f64)> = (0..r.len())
            .filter(|&i| inside(r[i]).next().is_some())
            .map(|i| (i, f64::from(r[i].len) + dice.unit()))
            .collect();
        let Some(&(i, _)) = scores.iter().max_by(|a, b| a.1.total_cmp(&b.1)) else {
            break;
        };
        let x = r[i];
        let mid = x.on + x.len / 2;
        let Some(at) = inside(x).max_by_key(|&p| (grid.level(p), -(p - mid).abs())) else {
            break;
        };
        r[i].len = at - x.on;
        r.insert(
            i + 1,
            Onset {
                on: at,
                len: x.on + x.len - at,
            },
        );
    }
}

/// The steps where a note of `r` sounds.
fn sounding(r: &[Onset], grid: Grid) -> Vec<bool> {
    let mut s = vec![false; grid.bar.max(1) as usize];
    for x in r {
        for p in x.on.max(0)..(x.on + x.len).min(grid.bar) {
            s[p as usize] = true;
        }
    }
    s
}

/// Each note of `r` stops where `was` (the rhythm it came from) had a
/// rest of an 1/8 or more: the rests you played stay rests.
fn keep_rests(r: &mut [Onset], was: &[bool]) {
    let silent = |p: i32| !was.get(p as usize).copied().unwrap_or(true);
    for x in r.iter_mut() {
        let end = x.on + x.len;
        if let Some(p) = (x.on + 1..end - 1).find(|&p| silent(p) && silent(p + 1)) {
            x.len = p - x.on;
        }
    }
}

/// The style's feel on a kept rhythm: Minimal keeps the strongest notes,
/// Groove pushes some beats a 1/16 early and chops long notes, Jazz-ish
/// moves notes to 1/8s and anticipates some beats. The further the bar
/// may stray from yours (`similarity` low), the more it changes. Rests of
/// an 1/8 or more in the rhythm stay rests.
pub fn feel(
    style: &RhythmStyle,
    r: &mut Vec<Onset>,
    activity: f64,
    similarity: f64,
    grid: Grid,
    dice: &mut Dice,
) {
    let k = (1.25 - similarity).clamp(0.1, 1.0);
    let beats = f64::from((grid.bar / grid.beat).max(1));
    let was = sounding(r, grid);
    let mut on: Vec<i32> = r.iter().map(|x| x.on).collect();
    match style.feel {
        Feel::Simplify => {
            let keep = ((f64::from(style.density)
                * beats
                * activity
                * (1.0 + (similarity - 0.5).max(0.0)))
            .round() as usize)
                .clamp(2, 10);
            if on.len() > keep {
                let mut scored: Vec<(f64, i32)> = r
                    .iter()
                    .map(|x| {
                        let first = if x.on == 0 { 10.0 } else { 0.0 };
                        (
                            f64::from(grid.level(x.on) + 5) + 0.4 * f64::from(x.len) + first,
                            x.on,
                        )
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.total_cmp(&a.0));
                on = scored.iter().take(keep).map(|x| x.1).collect();
            }
        }
        Feel::Push => {
            let mut v = Vec::with_capacity(on.len() * 2);
            for (i, &o) in on.iter().enumerate() {
                let prev = if i > 0 { on[i - 1] } else { -1 };
                if grid.on_beat(o) && o != 0 && o > prev && dice.chance(0.5 * k) {
                    v.push(o - 1);
                } else {
                    v.push(o);
                }
                // A long note chopped into a short one and a repeat (never
                // into a rest).
                let next = on.get(i + 1).copied().unwrap_or(grid.bar);
                let held = was
                    .get((o + grid.beat - 1) as usize)
                    .copied()
                    .unwrap_or(false);
                if next - o >= grid.beat && held && dice.chance(0.4 * k) {
                    v.push(o + grid.beat - 1);
                }
            }
            on = v;
        }
        Feel::Swing8 => {
            let last = (grid.bar - 2).max(0);
            let mut v: Vec<i32> = on.iter().map(|&o| ((o + 1) / 2 * 2).min(last)).collect();
            v.dedup();
            for i in 1..v.len() {
                let o = v[i];
                if o != 0 && grid.on_beat(o) && o - 2 >= v[i - 1] && dice.chance(0.4 * k) {
                    // Anticipated by a 1/8.
                    v[i] = o - 2;
                }
            }
            on = v;
        }
    }
    on.sort_unstable();
    on.dedup();
    on.retain(|&o| (0..grid.bar).contains(&o));
    *r = from_starts(&on, grid);
    keep_rests(r, &was);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::{DiceName, Job};
    use crate::style::{GROOVE, JAZZISH, MINIMAL};

    const FOUR: Grid = Grid { bar: 16, beat: 4 };

    fn dice(bar: u16) -> Dice {
        DiceName {
            seed: 7,
            part: 0,
            phrase: 0,
            job: Job::Rhythm,
            bar,
            attempt: 0,
        }
        .dice()
    }

    #[test]
    fn the_grid_in_four_and_three() {
        assert_eq!(Grid::of(Meter::default()), FOUR);
        let levels: Vec<i32> = (0..16).map(|s| FOUR.level(s)).collect();
        assert_eq!(
            levels,
            vec![0, -4, -3, -4, -2, -4, -3, -4, -1, -4, -3, -4, -2, -4, -3, -4]
        );
        let three = Grid::of(Meter::new(3, 4).unwrap());
        assert_eq!(three, Grid { bar: 12, beat: 4 });
        assert_eq!(three.level(4), -2);
        assert_eq!(three.level(6), -3);
        let six_eight = Grid::of(Meter::new(6, 8).unwrap());
        assert_eq!(six_eight, Grid { bar: 12, beat: 2 });
    }

    #[test]
    fn syncopation_is_notes_held_over_stronger_steps() {
        assert_eq!(syncopation(&[0, 4, 8, 12], FOUR), 0.0);
        assert!(syncopation(&[3, 7, 11, 15], FOUR) > 0.5);
        assert!(syncopation(&[0, 3, 6, 10], FOUR) > syncopation(&[0, 2, 4, 6], FOUR));
        assert_eq!(syncopation(&[], FOUR), 0.0);
    }

    #[test]
    fn a_rhythm_is_read_and_varied_inside_the_bar() {
        let bar = [
            (0, 2),
            (2, 1),
            (3, 1),
            (4, 2),
            (6, 2),
            (8, 3),
            (11, 1),
            (12, 4),
        ]
        .map(|(on, len)| GridNote {
            on,
            len,
            pos: 35,
            alt: 0,
            velocity: 90,
        });
        let r = rhythm_of(&bar, FOUR);
        assert_eq!(r.len(), 8);
        assert_eq!(r[7], Onset { on: 12, len: 4 });
        for b in 0..50 {
            let mut v = r.clone();
            vary(&mut v, 0.8, 1.0, FOUR, &mut dice(b));
            assert!(!v.is_empty());
            assert!(v.windows(2).all(|w| w[0].on < w[1].on), "{v:?}");
            assert!(v.iter().all(|x| x.on >= 0 && x.on < 16 && x.len >= 1));
        }
        // Nothing changes when nothing may.
        let mut same = r.clone();
        vary(&mut same, 0.0, 1.0, FOUR, &mut dice(0));
        assert_eq!(same, r);
    }

    #[test]
    fn new_rhythms_follow_the_style_and_avoid_yours() {
        let avoid = [0, 2, 3, 4, 6, 8, 11, 12];
        let mean = |style: &RhythmStyle| {
            (0..30)
                .map(|b| {
                    let r = fresh(style, 1.0, &avoid, FOUR, &mut dice(b));
                    assert!(r.len() >= 2);
                    let on: Vec<i32> = r.iter().map(|x| x.on).collect();
                    syncopation(&on, FOUR)
                })
                .sum::<f64>()
                / 30.0
        };
        // Groove's rhythms are more syncopated than Minimal's.
        assert!(mean(&GROOVE.rhythm) > mean(&MINIMAL.rhythm));
    }

    #[test]
    fn busier_and_sparser_rhythms_keep_the_downbeat_and_the_grid() {
        let r: Vec<Onset> = from_starts(&[0, 4, 6, 8, 12], FOUR);
        for b in 0..30 {
            let mut thin = r.clone();
            density_to(&mut thin, 3, 1, FOUR, &mut dice(b));
            assert_eq!(thin.len(), 3);
            assert_eq!(thin[0].on, 0, "the bar's first note stays");
            let mut full = r.clone();
            density_to(&mut full, 8, 1, FOUR, &mut dice(b));
            assert_eq!(full.len(), 8);
            assert!(full.windows(2).all(|w| w[0].on + w[0].len <= w[1].on));
            // On 1/8s only, a swung line stays on its 1/8s.
            let mut swung = r.clone();
            density_to(&mut swung, 8, 2, FOUR, &mut dice(b));
            assert!(swung.iter().all(|x| x.on % 2 == 0), "{swung:?}");
        }
    }

    #[test]
    fn a_fresh_rhythm_stands_on_the_downbeat() {
        // Your bar on every beat and 1/8: a contrast used to dodge all of
        // them and land every note a 1/16 late.
        let yours = [0, 2, 3, 4, 6, 8, 11, 12];
        for style in [MINIMAL.rhythm, GROOVE.rhythm, JAZZISH.rhythm] {
            let mut late = 0;
            for b in 0..40 {
                let r = fresh(&style, 0.7, &yours, FOUR, &mut dice(b));
                assert_eq!(r[0].on, 0, "{r:?}");
                let on: Vec<i32> = r.iter().map(|x| x.on).collect();
                late += on.iter().filter(|&&o| is_late(o, &on, FOUR)).count();
            }
            assert!(late < 10, "{late} late notes in 40 bars");
        }
    }

    #[test]
    fn the_rests_you_played_stay_rests() {
        // Two 1/8s, a beat and a half of rest, then a beat: the feel keeps
        // the rest.
        let r = vec![
            Onset { on: 0, len: 2 },
            Onset { on: 2, len: 2 },
            Onset { on: 10, len: 4 },
        ];
        for style in [&MINIMAL.rhythm, &GROOVE.rhythm, &JAZZISH.rhythm] {
            for b in 0..20 {
                let mut x = r.clone();
                feel(style, &mut x, 1.0, 0.9, FOUR, &mut dice(b));
                let sounding = sounding(&x, FOUR);
                assert!(
                    sounding[5..9].iter().all(|s| !s),
                    "{}: {x:?}",
                    style.syncopation
                );
            }
        }
    }

    #[test]
    fn each_feel_reshapes_a_rhythm() {
        let r: Vec<Onset> = from_starts(&[0, 2, 3, 4, 6, 8, 11, 12], FOUR);
        let mut minimal = r.clone();
        feel(&MINIMAL.rhythm, &mut minimal, 0.6, 0.6, FOUR, &mut dice(1));
        assert!(minimal.len() < r.len(), "Minimal keeps fewer notes");
        assert_eq!(minimal[0].on, 0, "and keeps the downbeat");
        let mut jazz = r.clone();
        feel(&JAZZISH.rhythm, &mut jazz, 1.0, 0.6, FOUR, &mut dice(1));
        assert!(jazz.iter().all(|x| x.on % 2 == 0), "Jazz-ish on 1/8s");
        let beats = from_starts(&[0, 4, 8, 12], FOUR);
        let pushed = (0..40)
            .filter(|&b| {
                let mut g = beats.clone();
                feel(&GROOVE.rhythm, &mut g, 1.0, 0.3, FOUR, &mut dice(b));
                g.iter().any(|x| x.on % 4 == 3)
            })
            .count();
        assert!(pushed > 10, "Groove pushes beats early ({pushed} of 40)");
    }
}
