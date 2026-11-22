//! New chords (phase 0C, the design's "Harmony"): a chord timeline over
//! every bar, yours as heard and the new ones from the style's own chord
//! moves.
//!
//! Minimal mostly stays and moves by step; Groove vamps; Jazz-ish goes
//! round the circle of fifths with a borrowed V7 in minor. The bars you
//! played keep the chords heard in them, and a bar that repeats, loops or
//! returns to one of yours plays that bar's chords. A bar that lands
//! takes the ending's chord on its last beat: home is the home chord, open
//! is V, and leading back is a chord that leads into the first. Within
//! those rules the chain is sampled rather than maximised (forward
//! filtering, backward sampling), so Try again gives another good
//! progression instead of the same "best" one, and Variation sets how
//! adventurous it is. New chords are suggested (shown dashed) until the
//! musician keeps or changes them.
//!
//! With no clear key there is nothing to build chords from: the new bars
//! play your bars' chords again, in order, and nothing is invented.

use mpfree_context::chord::{Chord, ChordAnswer, ChordSpan, ChordTimeline, Heard, Quality};
use mpfree_context::groove::STEP;
use mpfree_context::key::Key;
use mpfree_transport::time::{Meter, Tick};

use crate::dice::{DiceName, Job};
use crate::plan::{BarJob, BarPlan};
use crate::settings::Ending;
use crate::style::HarmonyStyle;

/// The chord on scale degree `degree` (0 = I) of `key`: a triad, or a
/// seventh chord.
pub fn diatonic(key: Key, degree: u8, sevenths: bool) -> Chord {
    let steps = key.mode.steps();
    let d = usize::from(degree % 7);
    let root = steps[d];
    let above = |k: usize| (steps[(d + k) % 7] + 12 - root) % 12;
    let q = match (above(2), above(4), sevenths, above(6)) {
        (4, 7, false, _) => Quality::Major,
        (3, 7, false, _) => Quality::Minor,
        (3, 6, false, _) => Quality::Diminished,
        (4, 7, true, 11) => Quality::Major7,
        (4, 7, true, 10) => Quality::Dominant7,
        (3, 7, true, 10) | (3, 7, true, 11) => Quality::Minor7,
        (3, 6, true, 10) => Quality::HalfDiminished7,
        (3, 6, true, _) => Quality::Diminished7,
        _ => Quality::Major,
    };
    Chord::new((root + key.tonic) % 12, q)
}

/// The chords the style builds on in `key`, each with its degree.
fn palette(key: Key, style: &HarmonyStyle) -> Vec<(Chord, u8)> {
    let mut v: Vec<(Chord, u8)> = style
        .degrees
        .iter()
        .map(|&d| (diatonic(key, d, style.sevenths), d))
        .collect();
    if style.borrowed_v && key.mode.minorish() {
        v.retain(|c| c.1 != 4);
        let q = if style.sevenths {
            Quality::Dominant7
        } else {
            Quality::Major
        };
        v.push((Chord::new(key.tonic + 7, q), 4));
    }
    v
}

/// One stretch of the new chords: fixed (heard, or copied from your bar)
/// or free among the palette's chords.
#[derive(Clone, Debug)]
struct Slot {
    start: Tick,
    len: Tick,
    /// The chords it may be: one when fixed.
    may: Vec<ChordAnswer>,
    fixed: bool,
}

/// How likely the move from `a` to `b` is, in the style's own words: how
/// much it stays, and its root motion.
fn moves(style: &HarmonyStyle, a: ChordAnswer, b: ChordAnswer) -> f64 {
    match (a, b) {
        (ChordAnswer::Chord(a), ChordAnswer::Chord(b)) => {
            if a == b {
                f64::from(style.stay)
            } else {
                let up = usize::from((b.root + 12 - a.root) % 12);
                f64::from(style.motion[up]) + 0.01
            }
        }
        // Into or out of no clear chord: no preference.
        _ => 1.0,
    }
}

/// The chord timeline over every planned bar: yours as `heard` (over your
/// bars, from tick 0), and the new bars' chords. `key` is the key to build
/// in, or `None` for no clear key. `attempt` is Try again's count.
#[allow(clippy::too_many_arguments)]
pub fn progression(
    heard: &ChordTimeline,
    key: Option<Key>,
    style: &HarmonyStyle,
    plan: &[BarPlan],
    variation: f32,
    seed: u64,
    attempt: u16,
    meter: Meter,
) -> ChordTimeline {
    let bar = meter.bar_ticks().max(1);
    let yours = plan.iter().take_while(|b| b.job == BarJob::Yours).count();
    let yours_len = yours as Tick * bar;
    let total = plan.len() as Tick * bar;
    // Your bars, as heard (cut or padded with no clear chord to their
    // length).
    let mut spans: Vec<ChordSpan> = heard
        .spans()
        .iter()
        .filter(|s| s.start < yours_len)
        .map(|s| ChordSpan {
            len: s.len.min(yours_len - s.start),
            ..*s
        })
        .collect();
    let heard_end = spans.last().map_or(0, |s| s.end());
    if heard_end < yours_len {
        spans.push(ChordSpan::new(
            heard_end,
            yours_len - heard_end,
            ChordAnswer::NoClearChord,
            Heard::Heard,
        ));
    }
    let your_chord_at = |t: Tick| -> ChordAnswer {
        spans
            .iter()
            .find(|s| s.contains(t))
            .map_or(ChordAnswer::NoClearChord, |s| s.answer)
    };
    // Your bar's chords as fixed slots at `at`.
    let copy = |source: usize, at: Tick| -> Vec<Slot> {
        let from = source as Tick * bar;
        spans
            .iter()
            .filter(|s| s.start < from + bar && s.end() > from)
            .map(|s| {
                let a = s.start.max(from);
                let e = s.end().min(from + bar);
                Slot {
                    start: at + a - from,
                    len: e - a,
                    may: vec![s.answer],
                    fixed: true,
                }
            })
            .collect()
    };

    // The new bars as slots.
    let window = (Tick::from(style.window_steps) * STEP).max(STEP);
    let mut slots: Vec<Slot> = Vec::new();
    let pal = key.map(|k| palette(k, style)).unwrap_or_default();
    let mut b = yours;
    while b < plan.len() {
        let at = b as Tick * bar;
        let bp = plan[b];
        if key.is_none() || bp.job.copies_chords() {
            slots.extend(copy(
                if key.is_none() {
                    b % yours.max(1)
                } else {
                    bp.source
                },
                at,
            ));
            b += 1;
            continue;
        }
        // A run of free bars, cut into windows from its start.
        let run_end = (b..plan.len())
            .find(|&i| plan[i].job.copies_chords())
            .unwrap_or(plan.len());
        let end = run_end as Tick * bar;
        let mut t = at;
        while t < end {
            let len = window.min(end - t);
            slots.push(Slot {
                start: t,
                len,
                may: pal.iter().map(|c| ChordAnswer::Chord(c.0)).collect(),
                fixed: false,
            });
            t += len;
        }
        b = run_end;
    }
    // Each landing bar's last beat: the ending's chord.
    if let Some(key) = key {
        for (i, bp) in plan.iter().enumerate() {
            let BarJob::Land(ending) = bp.job else {
                continue;
            };
            let last = (i as Tick + 1) * bar - 1;
            let Some(slot) = slots
                .iter_mut()
                .find(|s| !s.fixed && (s.start..s.start + s.len).contains(&last))
            else {
                continue;
            };
            let allowed: Vec<u8> = match ending {
                Ending::Home => vec![0],
                Ending::Open => vec![4],
                Ending::LoopsBack => vec![4, 6, 3],
            };
            slot.may = pal
                .iter()
                .filter(|c| allowed.contains(&c.1))
                .map(|c| ChordAnswer::Chord(c.0))
                .collect();
            if slot.may.is_empty() {
                // The style has no such chord: the plain one.
                let d = allowed[0];
                slot.may = vec![ChordAnswer::Chord(diatonic(key, d, style.sevenths))];
            }
        }
    }

    // Forward filtering over the slots, from your last chord, then
    // backward sampling.
    let sharp = 0.59 / (0.35 + 0.6 * f64::from(variation.clamp(0.0, 1.0)));
    let w = |a: ChordAnswer, b: ChordAnswer| moves(style, a, b).powf(sharp);
    let start = your_chord_at(yours_len - 1);
    let mut fw: Vec<Vec<f64>> = Vec::with_capacity(slots.len());
    for (i, s) in slots.iter().enumerate() {
        let row: Vec<f64> = s
            .may
            .iter()
            .map(|&c| {
                if i == 0 {
                    w(start, c)
                } else {
                    slots[i - 1]
                        .may
                        .iter()
                        .zip(&fw[i - 1])
                        .map(|(&p, &f)| f * w(p, c))
                        .sum()
                }
            })
            .collect();
        let z: f64 = row.iter().sum();
        fw.push(if z > 0.0 {
            row.iter().map(|x| x / z).collect()
        } else {
            vec![1.0 / row.len().max(1) as f64; row.len()]
        });
    }
    let mut dice = DiceName {
        seed,
        part: 0,
        phrase: 0,
        job: Job::Harmony,
        bar: 0,
        attempt,
    }
    .dice();
    let mut chosen = vec![0usize; slots.len()];
    for i in (0..slots.len()).rev() {
        let weights: Vec<f64> = if i + 1 == slots.len() {
            fw[i].clone()
        } else {
            let next = slots[i + 1].may[chosen[i + 1]];
            slots[i]
                .may
                .iter()
                .zip(&fw[i])
                .map(|(&c, &f)| f * w(c, next))
                .collect()
        };
        chosen[i] = dice.pick(&weights).unwrap_or(0);
    }
    for (s, &c) in slots.iter().zip(&chosen) {
        let answer = s.may[c];
        let made = if s.fixed {
            Heard::Heard
        } else {
            Heard::Suggested
        };
        match spans.last_mut() {
            Some(prev) if prev.answer == answer && prev.heard == made && prev.end() == s.start => {
                prev.len += s.len;
            }
            _ => spans.push(ChordSpan::new(s.start, s.len, answer, made)),
        }
    }
    let end = spans.last().map_or(0, |s| s.end());
    if end < total {
        spans.push(ChordSpan::new(
            end,
            total - end,
            ChordAnswer::NoClearChord,
            Heard::Heard,
        ));
    }
    ChordTimeline::new(spans).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::plan;
    use crate::settings::Settings;
    use crate::style::{Style, GROOVE, JAZZISH, MINIMAL};
    use mpfree_context::key::Mode;
    use mpfree_transport::time::PPQ;

    const BAR: Tick = 4 * PPQ as Tick;
    const CM: Key = Key::new(0, Mode::Minor);

    fn two_bars_of_cm() -> ChordTimeline {
        ChordTimeline::whole(
            2 * BAR,
            ChordAnswer::Chord(Chord::new(0, Quality::Minor)),
            Heard::Heard,
        )
    }

    fn make(style: Style, bars: usize, seed: u64, attempt: u16) -> (Vec<BarPlan>, ChordTimeline) {
        let s = Settings {
            style,
            seed,
            ..Settings::default()
        };
        let p = plan(&s, 2, bars, 1.0);
        let tl = progression(
            &two_bars_of_cm(),
            Some(CM),
            &style.harmony,
            &p,
            0.4,
            seed,
            attempt,
            Meter::default(),
        );
        (p, tl)
    }

    #[test]
    fn diatonic_chords_in_c_minor() {
        let names: Vec<String> = (0..7).map(|d| diatonic(CM, d, false).name()).collect();
        assert_eq!(names, vec!["Cm", "Ddim", "Eb", "Fm", "Gm", "Ab", "Bb"]);
        let sevenths: Vec<String> = (0..7).map(|d| diatonic(CM, d, true).name()).collect();
        assert_eq!(
            sevenths,
            vec!["Cm7", "Dm7b5", "Ebmaj7", "Fm7", "Gm7", "Abmaj7", "Bb7"]
        );
        assert_eq!(diatonic(Key::new(2, Mode::Dorian), 3, true).name(), "G7");
    }

    #[test]
    fn yours_stay_as_heard_the_new_are_suggested_and_the_end_lands() {
        for style in [MINIMAL, GROOVE, JAZZISH] {
            for seed in 0..20 {
                let (_, tl) = make(style, 8, seed, 0);
                assert_eq!(tl.len(), 8 * BAR, "{}", style.name);
                let first = tl.span_at(0).unwrap();
                assert_eq!(first.heard, Heard::Heard);
                assert_eq!(first.answer.name(), "Cm");
                assert!(first.end() >= 2 * BAR);
                // Every chord is from the style's palette or heard.
                let pal = palette(CM, &style.harmony);
                for s in tl.spans().iter().filter(|s| s.heard == Heard::Suggested) {
                    let c = s.answer.chord().unwrap();
                    assert!(pal.iter().any(|p| p.0 == c), "{} {}", style.name, c.name());
                }
                // Lands home: the last beat is on the home chord.
                let last = tl.span_at(8 * BAR - 1).unwrap().answer.chord().unwrap();
                assert_eq!(last.root, 0, "{} seed {seed}: {}", style.name, tl.names());
            }
        }
    }

    #[test]
    fn repeats_and_returns_play_your_bars_chords() {
        // A riff with two chords a bar.
        let heard = ChordTimeline::of(
            &[
                (Chord::new(0, Quality::Minor7), BAR / 2),
                (Chord::new(5, Quality::Minor7), BAR / 2),
                (Chord::new(3, Quality::Major7), BAR),
            ],
            Heard::Heard,
        );
        let s = Settings {
            style: GROOVE,
            ..Settings::default()
        };
        let p = plan(&s, 2, 8, 1.0);
        assert_eq!(p[4].job, BarJob::Return);
        for seed in 0..10 {
            let tl = progression(
                &heard,
                Some(CM),
                &GROOVE.harmony,
                &p,
                0.4,
                seed,
                0,
                Meter::default(),
            );
            for t in [0, BAR / 2] {
                let s = tl.span_at(4 * BAR + t).unwrap();
                assert_eq!(s.answer, heard.span_at(t).unwrap().answer);
                assert_eq!(s.heard, Heard::Heard, "copied, not made up");
            }
        }
    }

    #[test]
    fn try_again_gives_another_progression_and_the_same_try_the_same() {
        let (_, a) = make(JAZZISH, 8, 5, 0);
        let (_, again) = make(JAZZISH, 8, 5, 0);
        assert_eq!(a, again);
        let differs = (1..8).any(|t| make(JAZZISH, 8, 5, t).1.names() != a.names());
        assert!(differs);
    }

    #[test]
    fn variation_makes_it_more_adventurous() {
        let distinct = |variation: f32| -> usize {
            let mut seen = std::collections::HashSet::new();
            for seed in 0..40 {
                let s = Settings {
                    style: GROOVE,
                    seed,
                    ..Settings::default()
                };
                let p = plan(&s, 2, 8, 1.0);
                let tl = progression(
                    &two_bars_of_cm(),
                    Some(CM),
                    &GROOVE.harmony,
                    &p,
                    variation,
                    seed,
                    0,
                    Meter::default(),
                );
                seen.insert(tl.names());
            }
            seen.len()
        };
        assert!(distinct(1.0) > distinct(0.0));
    }

    #[test]
    fn open_and_loop_back_endings() {
        let s = Settings {
            style: GROOVE,
            ending: Ending::Open,
            ..Settings::default()
        };
        let p = plan(&s, 2, 4, 1.0);
        let tl = progression(
            &two_bars_of_cm(),
            Some(CM),
            &GROOVE.harmony,
            &p,
            0.4,
            1,
            0,
            Meter::default(),
        );
        assert_eq!(
            tl.span_at(4 * BAR - 1)
                .unwrap()
                .answer
                .chord()
                .unwrap()
                .root,
            7
        );
        let s = Settings {
            ending: Ending::LoopsBack,
            ..s
        };
        let p = plan(&s, 2, 4, 1.0);
        let tl = progression(
            &two_bars_of_cm(),
            Some(CM),
            &GROOVE.harmony,
            &p,
            0.4,
            1,
            0,
            Meter::default(),
        );
        let root = tl
            .span_at(4 * BAR - 1)
            .unwrap()
            .answer
            .chord()
            .unwrap()
            .root;
        assert!([7, 10, 5].contains(&root), "{}", tl.names());
    }

    #[test]
    fn no_clear_key_invents_nothing() {
        let heard = ChordTimeline::of(
            &[
                (Chord::new(0, Quality::Power), BAR),
                (Chord::new(3, Quality::Power), BAR),
            ],
            Heard::Heard,
        );
        let s = Settings::default();
        let p = plan(&s, 2, 6, 1.0);
        let tl = progression(
            &heard,
            None,
            &MINIMAL.harmony,
            &p,
            0.4,
            1,
            0,
            Meter::default(),
        );
        assert_eq!(tl.names(), "C5 Eb5 C5 Eb5 C5 Eb5");
        assert!(tl.spans().iter().all(|s| s.heard == Heard::Heard));
    }
}
