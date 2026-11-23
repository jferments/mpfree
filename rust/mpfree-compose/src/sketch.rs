//! The phrase plan (revision 3, S1): what the new bars do as a whole,
//! before any note.
//!
//! The first engine gave every bar a job and searched for notes bar by
//! bar, hoping a whole would appear; two listening rounds heard "no
//! purpose or movement". Here the new bars are sketched as one phrase
//! first:
//!
//! - **Gestures**, one per bar, from a phrase shape: answer your idea,
//!   reach up to a first landing, hold back, set up, reach the high point,
//!   come home (or leave it open, or lead back).
//! - **Anchors**: the notes everything else serves, fixed before the rest:
//!   the first landing, the high point (one, late, above anything you
//!   played) and the last note. Each says which chords may sound under it,
//!   so the chords are led to it, and the chord then picks its exact note.
//! - **Rests** where a phrase breathes: after a landing, the beat after
//!   the high point, the end of a held-back bar.
//! - **One placed surprise** before the high point: a chord from the
//!   other mode, or a sudden silence where your idea would go on.
//!
//! Draft numbers move the high point, the landings' places and the
//! surprise, so the critic has different whole drafts to choose from.

use mpfree_context::chord::{Chord, Quality};
use mpfree_context::key::{Key, Mode};

use crate::dice::{DiceName, Job};
use crate::phrase::Phrasing;
use crate::plan::{BarJob, BarPlan};
use crate::rhythm::Grid;
use crate::settings::{Ending, Settings};
use crate::style::PlanShape;

/// What a new bar does in the phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gesture {
    /// Your idea answered: its rhythm, a step away.
    Answer,
    /// Rising to the first landing.
    Reach,
    /// Your opening twice, the second time a step higher.
    Develop,
    /// Fewer notes, a held note and a rest where your idea would go on.
    HoldBack,
    /// Climbing towards the high point; the surprise is here.
    SetUp,
    /// The high point, then a beat of silence.
    Peak,
    /// Coming down after the high point.
    Settle,
    /// The last bar, closing as the ending asks.
    Close(Ending),
}

impl Gesture {
    pub const fn words(self) -> &'static str {
        match self {
            Gesture::Answer => "answer",
            Gesture::Reach => "reach up",
            Gesture::Develop => "develop",
            Gesture::HoldBack => "hold back",
            Gesture::SetUp => "set up",
            Gesture::Peak => "high point",
            Gesture::Settle => "settle",
            Gesture::Close(Ending::Home) => "come home",
            Gesture::Close(Ending::Open) => "leave open",
            Gesture::Close(Ending::LoopsBack) => "lead back",
        }
    }

    /// How tense the bar is: its register and loudness follow.
    const fn tension(self) -> f32 {
        match self {
            Gesture::Answer => 0.35,
            Gesture::Reach => 0.55,
            Gesture::Develop => 0.5,
            Gesture::HoldBack => 0.3,
            Gesture::SetUp => 0.6,
            Gesture::Peak => 0.95,
            Gesture::Settle => 0.45,
            Gesture::Close(Ending::Home) => 0.15,
            Gesture::Close(_) => 0.4,
        }
    }

    /// How busy, against what the settings ask.
    const fn activity(self) -> f32 {
        match self {
            Gesture::Answer | Gesture::Reach => 1.0,
            Gesture::Develop => 1.1,
            Gesture::HoldBack => 0.55,
            Gesture::SetUp => 0.9,
            Gesture::Peak => 1.1,
            Gesture::Settle => 0.8,
            Gesture::Close(_) => 0.7,
        }
    }

    /// How close to your bar it aims to sound, before Similarity.
    const fn similarity(self) -> f32 {
        match self {
            Gesture::Answer => 0.7,
            Gesture::Reach => 0.55,
            Gesture::Develop => 0.55,
            Gesture::HoldBack => 0.5,
            Gesture::SetUp => 0.45,
            Gesture::Peak => 0.45,
            Gesture::Settle => 0.55,
            Gesture::Close(_) => 0.6,
        }
    }

    /// The bar job the rest of the composer reads (Try again on a bar,
    /// the playing, the plan strip).
    pub const fn job(self) -> BarJob {
        match self {
            Gesture::Answer | Gesture::Settle => BarJob::Answer,
            Gesture::Reach | Gesture::Develop | Gesture::Peak => BarJob::Develop,
            Gesture::HoldBack | Gesture::SetUp => BarJob::Vary,
            Gesture::Close(e) => BarJob::Land(e),
        }
    }
}

/// The phrase shape for `n` new bars closing with `close`. With
/// `early_peak` the high point comes a bar sooner and a settling bar
/// follows it. More than eight bars are an opening stretch that ends open
/// with no high point, then eight bars with it.
pub fn gestures(n: usize, close: Ending, early_peak: bool) -> Vec<Gesture> {
    use Gesture::*;
    let c = Close(close);
    if n > 8 {
        let mut v: Vec<Gesture> = gestures(n - 8, Ending::Open, false)
            .into_iter()
            .map(|g| if g == Peak { Develop } else { g })
            .collect();
        v.extend(gestures(8, close, early_peak));
        return v;
    }
    let late: &[Gesture] = match n {
        0 => &[],
        1 => &[c],
        2 => &[Peak, c],
        3 => &[Answer, Peak, c],
        4 => &[Answer, Reach, Peak, c],
        5 => &[Answer, Reach, SetUp, Peak, c],
        6 => &[Answer, Reach, HoldBack, SetUp, Peak, c],
        7 => &[Answer, Reach, Develop, HoldBack, SetUp, Peak, c],
        _ => &[Answer, Reach, Develop, Reach, HoldBack, SetUp, Peak, c],
    };
    let mut v = late.to_vec();
    if early_peak && n >= 5 {
        // ... SetUp, Peak, Close becomes ... SetUp, Peak, Settle, Close:
        // the bar before the set-up goes.
        let at = v.iter().position(|&g| g == SetUp).unwrap_or(0);
        if at > 0 {
            v.remove(at - 1);
            let p = v.len() - 1;
            v.insert(p, Settle);
        }
    }
    v
}

/// What an anchor is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnchorKind {
    /// The end of a phrase inside the new bars: open, wanting more.
    Landing,
    /// The high point.
    Peak,
    /// The last note.
    Close,
}

/// A note the plan fixes before any other: where it sounds, how long,
/// about where in the scale, and the chords that may sound under it. The
/// chord picks the exact note.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub kind: AnchorKind,
    pub bar: usize,
    /// Steps from the start of your first bar.
    pub on: i32,
    pub len: i32,
    /// The scale place it aims at.
    pub aim: i32,
    /// Scale degrees (0 = home) the note prefers, best first.
    pub notes: Vec<i32>,
    /// Chord degrees that may sound under it, best first.
    pub chords: Vec<u8>,
}

/// The one surprise a draft places before its high point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surprise {
    /// A chord from the other mode under the set-up bar's second half.
    Borrowed { bar: usize },
    /// Silence from the middle of the set-up bar, where your idea would
    /// go on.
    Silence { bar: usize },
}

impl Surprise {
    pub fn bar(self) -> usize {
        match self {
            Surprise::Borrowed { bar } | Surprise::Silence { bar } => bar,
        }
    }
}

/// The chord a borrowed surprise plays in `key`: iv in a major key (the
/// minor four), IV in minor and Phrygian, and bVI in Dorian, whose IV is
/// already major.
pub fn borrowed(key: Key) -> Chord {
    let t = key.tonic;
    match key.mode {
        Mode::Major | Mode::Lydian | Mode::Mixolydian => Chord::new((t + 5) % 12, Quality::Minor),
        Mode::Minor | Mode::Phrygian => Chord::new((t + 5) % 12, Quality::Major),
        Mode::Dorian => Chord::new((t + 8) % 12, Quality::Major),
    }
}

/// The phrase plan for one draft.
#[derive(Clone, Debug, PartialEq)]
pub struct Sketch {
    /// One per new bar.
    pub gestures: Vec<Gesture>,
    /// One per bar, yours first: what the rest of the composer reads.
    pub plan: Vec<BarPlan>,
    pub anchors: Vec<Anchor>,
    /// Where the line rests, as `(from, to)` in steps from your first bar.
    pub rests: Vec<(i32, i32)>,
    pub surprise: Option<Surprise>,
}

/// Where the middle of a bar falls for a landing: the half bar in an even
/// meter, the second beat in 3/4.
fn middle(grid: Grid) -> i32 {
    let beats = (grid.bar / grid.beat).max(1);
    (beats / 2).max(1) * grid.beat
}

/// Chord degrees holding scale degree `note`, in the order wanted under a
/// high point: the four and two chords first, home last.
fn chords_holding(note: i32) -> Vec<u8> {
    [3u8, 1, 5, 4, 0, 2, 6]
        .into_iter()
        .filter(|&d| matches!((note - i32::from(d)).rem_euclid(7), 0 | 2 | 4))
        .collect()
}

/// Sketch `total` bars, the first `ph.your_bars` yours, for draft `draft`.
pub fn sketch(ph: &Phrasing, settings: &Settings, total: usize, draft: u16) -> Sketch {
    let yours = ph.your_bars.min(total);
    let n = total - yours;
    let grid = ph.grid;
    let mut dice = DiceName {
        seed: settings.seed,
        part: ph.part,
        phrase: draft,
        job: Job::Sketch,
        bar: 0,
        attempt: 0,
    }
    .dice();
    let gestures = gestures(n, settings.ending, draft % 2 == 1);
    let pattern = crate::plan::shapes(ph.style, settings.seed).0;
    let act = 0.5 + settings.activity * 1.1;

    // The plan the rest of the composer reads.
    let mut plan: Vec<BarPlan> = (0..yours)
        .map(|b| BarPlan {
            job: BarJob::Yours,
            source: b,
            shift: 0,
            tension: 0.3,
            activity: 1.0,
            similarity: 1.0,
        })
        .collect();
    for (i, &g) in gestures.iter().enumerate() {
        let shift = match g {
            Gesture::Answer => 1,
            Gesture::SetUp => 2,
            Gesture::Settle => -1,
            _ => 0,
        };
        plan.push(BarPlan {
            job: g.job(),
            source: (yours + i) % yours.max(1),
            shift,
            tension: g.tension(),
            activity: g.activity() * act,
            similarity: (g.similarity() + (settings.similarity - 0.5) * 0.6).clamp(0.05, 0.98),
        });
    }

    // Registers, in scale places.
    let (lo, hi) = ph
        .pitches(settings.range)
        .map(|(l, h)| (ph.key.position(l).0, ph.key.position(h).0))
        .unwrap_or((ph.centre - 7, ph.centre + 7));
    let top_yours = ph
        .yours
        .iter()
        .map(|m| m.pos)
        .max()
        .unwrap_or(ph.centre + 2);
    let first_yours = ph.yours.first().map_or(ph.centre, |m| m.pos);
    let peak_aim = (top_yours + 1)
        .max(ph.centre + 4 + i32::from(dice.chance(0.4)))
        .min(hi)
        .max(lo);
    let home = {
        let h = ph.centre - ph.centre.rem_euclid(7);
        if ph.centre - h > 4 {
            h + 7
        } else {
            h
        }
    };

    let mid = middle(grid);
    let beat = grid.beat;
    let eighth = (beat / 2).max(1);
    let mut anchors: Vec<Anchor> = Vec::new();
    let mut rests: Vec<(i32, i32)> = Vec::new();
    let downbeat_landings = dice.chance(0.5);
    let mut first_landing = true;
    for (i, &g) in gestures.iter().enumerate() {
        let b = yours + i;
        let start = b as i32 * grid.bar;
        let end = start + grid.bar;
        match g {
            Gesture::Reach => {
                // Open: on the five chord, the note its root or its 5th; or
                // now and then on the four or six chord.
                let (notes, chords) = if dice.chance(0.75) {
                    (vec![4, 1], vec![4])
                } else {
                    (vec![2, 0], vec![5, 3])
                };
                let aim = (ph.centre + 2 + i32::from(!first_landing)).min(peak_aim - 1);
                let (on, len) = if downbeat_landings {
                    (start, mid)
                } else {
                    (start + mid, grid.bar - mid - eighth)
                };
                anchors.push(Anchor {
                    kind: AnchorKind::Landing,
                    bar: b,
                    on,
                    len: len.max(1),
                    aim,
                    notes,
                    chords,
                });
                rests.push((on + len.max(1), (on + len.max(1) + eighth).min(end)));
                first_landing = false;
            }
            Gesture::Peak => {
                // On the middle, or a beat in; in a groove now and then an
                // 1/8 early, pushing into it. A beat of silence after it.
                let at = if pattern == PlanShape::CallAndAnswer && dice.chance(0.4) {
                    mid - eighth
                } else if dice.chance(0.5) {
                    mid
                } else {
                    beat
                };
                let len = beat.min(grid.bar - at - beat).max(eighth);
                let on = start + at;
                anchors.push(Anchor {
                    kind: AnchorKind::Peak,
                    bar: b,
                    on,
                    len,
                    aim: peak_aim,
                    notes: vec![peak_aim.rem_euclid(7)],
                    chords: chords_holding(peak_aim.rem_euclid(7)),
                });
                rests.push((on + len, (on + len + beat).min(end)));
            }
            Gesture::HoldBack => {
                rests.push((end - beat, end));
            }
            Gesture::Close(e) => {
                let (aim, notes, chords) = match e {
                    Ending::Home => (home, vec![0, 2, 4], vec![0]),
                    Ending::Open => (ph.centre + 1, vec![4, 1], vec![4]),
                    Ending::LoopsBack => (
                        first_yours,
                        vec![first_yours.rem_euclid(7), 4],
                        vec![4, 6, 3],
                    ),
                };
                let on = if n == 1 || dice.chance(0.5) {
                    start + mid
                } else {
                    start
                };
                // Short of the very end when it leads back, so the first bar
                // comes in fresh.
                let len = if e == Ending::LoopsBack {
                    (end - eighth - on).max(1)
                } else {
                    end - on
                };
                anchors.push(Anchor {
                    kind: if b + 1 == total {
                        AnchorKind::Close
                    } else {
                        AnchorKind::Landing
                    },
                    bar: b,
                    on,
                    len,
                    aim: aim.clamp(lo, hi),
                    notes,
                    chords,
                });
                if b + 1 < total {
                    rests.push((end - eighth, end));
                }
            }
            _ => {}
        }
    }

    // One placed surprise, in the set-up bar.
    let surprise = gestures
        .iter()
        .rposition(|&g| g == Gesture::SetUp)
        .map(|i| yours + i)
        .map(|bar| {
            let borrowed_odds = match pattern {
                PlanShape::Sentence => 0.7,
                PlanShape::Loop => 0.3,
                _ => 0.5,
            };
            let borrowed_odds = if draft.is_multiple_of(2) {
                borrowed_odds
            } else {
                1.0 - borrowed_odds
            };
            // A borrowed chord only where chords change every bar or
            // sooner, so it colours the set-up alone.
            let short_chords = i32::from(ph.style.harmony.window_steps) <= grid.bar;
            if ph.clear && short_chords && dice.chance(borrowed_odds) {
                Surprise::Borrowed { bar }
            } else {
                Surprise::Silence { bar }
            }
        });
    if let Some(Surprise::Silence { bar }) = surprise {
        let start = bar as i32 * grid.bar;
        rests.push((start + mid, start + grid.bar));
    }
    Sketch {
        gestures,
        plan,
        anchors,
        rests,
        surprise,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_length_ends_with_its_close_and_has_one_late_high_point() {
        for n in 3..=16 {
            for early in [false, true] {
                let g = gestures(n, Ending::Home, early);
                assert_eq!(g.len(), n, "{n} bars");
                assert_eq!(g.last(), Some(&Gesture::Close(Ending::Home)));
                let peaks: Vec<usize> = (0..n).filter(|&i| g[i] == Gesture::Peak).collect();
                assert_eq!(peaks.len(), 1, "{n} bars, early {early}: {g:?}");
                let at = (peaks[0] as f64 + 0.5) / n as f64;
                assert!(
                    (0.4..0.95).contains(&at),
                    "{n} bars: the high point at {at}"
                );
            }
        }
    }

    #[test]
    fn six_new_bars_follow_the_design_example() {
        use Gesture::*;
        assert_eq!(
            gestures(6, Ending::Home, false),
            vec![Answer, Reach, HoldBack, SetUp, Peak, Close(Ending::Home)]
        );
        assert_eq!(
            gestures(6, Ending::Home, true),
            vec![Answer, Reach, SetUp, Peak, Settle, Close(Ending::Home)]
        );
    }

    #[test]
    fn the_borrowed_chord_is_outside_the_key() {
        for mode in Mode::ALL {
            let key = Key::new(2, mode);
            let c = borrowed(key);
            let outside = c
                .quality
                .intervals()
                .iter()
                .any(|&i| !key.has(i32::from(c.root) + i32::from(i)));
            assert!(outside, "{mode:?}: {}", c.name());
        }
    }
}
