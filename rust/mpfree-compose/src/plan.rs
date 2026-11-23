//! The plan: what each bar is for (phase 0C). Before any note, each bar
//! gets a job, a tension aim, an activity and a similarity aim.
//!
//! Jobs are yours, repeat (your bar again), loop, vary, sequence (your bar
//! moved in the scale), answer, develop, contrast, return and land (home,
//! open, or leading back to the start). The style's pattern sets the jobs:
//! Minimal loops, Groove calls and answers, Jazz-ish builds sentences.
//! Its tension curve, picked by the seed from the style's own, sets the
//! rise and fall: one climax about three quarters through, small waves, or
//! a slow build. Sixteen bars or more get a contrasting phrase and a
//! return. The numbers are the second prototype's.

use crate::dice::{DiceName, Job};
use crate::settings::{Ending, Settings};
use crate::style::{PlanShape, Style};

/// What a bar is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BarJob {
    /// One of the bars you played.
    Yours,
    /// Your bar again.
    Repeat,
    /// Your bar again, as a riff does.
    Loop,
    /// Your bar with small changes.
    Vary,
    /// Your bar moved up or down the scale.
    Sequence,
    /// A reply to the bar before.
    Answer,
    /// Your idea taken somewhere.
    Develop,
    /// Something else for a while.
    Contrast,
    /// Back to your first bar after a contrast.
    Return,
    /// The phrase closes: home, open, or leading back.
    Land(Ending),
}

impl BarJob {
    /// In words, for the plan strip.
    pub const fn words(self) -> &'static str {
        match self {
            BarJob::Yours => "yours",
            BarJob::Repeat => "repeat",
            BarJob::Loop => "loop",
            BarJob::Vary => "vary",
            BarJob::Sequence => "sequence",
            BarJob::Answer => "answer",
            BarJob::Develop => "develop",
            BarJob::Contrast => "contrast",
            BarJob::Return => "return",
            BarJob::Land(Ending::Home) => "land home",
            BarJob::Land(Ending::Open) => "leave open",
            BarJob::Land(Ending::LoopsBack) => "lead back",
        }
    }

    /// How close to your bar a bar with this job sits, before Similarity.
    const fn similarity(self) -> f32 {
        match self {
            BarJob::Yours => 1.0,
            BarJob::Repeat => 0.9,
            BarJob::Loop => 0.92,
            BarJob::Return => 0.85,
            BarJob::Sequence => 0.75,
            BarJob::Vary => 0.65,
            BarJob::Answer => 0.6,
            BarJob::Land(_) => 0.45,
            BarJob::Develop => 0.5,
            BarJob::Contrast => 0.25,
        }
    }

    /// Whether the bar plays your bar's chords as they were. A loop bar
    /// plays your riff over the style's own chords (Minimal's Cm | % | Bb
    /// | % | Cm), so it doesn't.
    pub const fn copies_chords(self) -> bool {
        matches!(self, BarJob::Repeat | BarJob::Return)
    }
}

/// One bar's plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarPlan {
    pub job: BarJob,
    /// Which of your bars it works from.
    pub source: usize,
    /// Scale steps it moves your bar (a sequence).
    pub shift: i32,
    /// 0 (settled) to 1 (most tense): register, leaps, off-chord notes on
    /// strong beats and loudness follow it.
    pub tension: f32,
    /// Notes per beat as a share of yours.
    pub activity: f32,
    /// How close to your bar it aims to be (0 to 1).
    pub similarity: f32,
}

impl PlanShape {
    /// The shapes that set which jobs bars get; the others are tension
    /// curves.
    pub const fn is_pattern(self) -> bool {
        matches!(
            self,
            PlanShape::Loop | PlanShape::CallAndAnswer | PlanShape::Sentence
        )
    }
}

/// The style's pattern (its first pattern shape) and the tension curve
/// the seed picks from its curves (one climax when it lists none).
pub fn shapes(style: &Style, seed: u64) -> (PlanShape, PlanShape) {
    let plans = style.development.plans;
    let pattern = plans
        .iter()
        .copied()
        .find(|p| p.is_pattern())
        .unwrap_or(PlanShape::Sentence);
    let curves: Vec<PlanShape> = plans.iter().copied().filter(|p| !p.is_pattern()).collect();
    let curve = if curves.is_empty() {
        PlanShape::OneClimax
    } else {
        let mut d = DiceName {
            seed,
            part: 0,
            phrase: 0,
            job: Job::Plan,
            bar: 0,
            attempt: 0,
        }
        .dice();
        curves[d.below(curves.len())]
    };
    (pattern, curve)
}

/// The tension curve at bar `b` of `total`, before the jobs' own lifts.
fn curve(shape: PlanShape, b: usize, total: usize) -> f32 {
    let x = (b as f32 + 0.5) / total as f32;
    match shape {
        PlanShape::Waves => {
            // A small rise over each four bars, a gentle lift overall, and
            // a fall in the last bar.
            let pp = (b % 4) as f32 / 3.0;
            let fall = if b + 1 == total { -0.2 } else { 0.0 };
            0.25 + 0.35 * pp + 0.15 * x + fall
        }
        PlanShape::SlowBuild => 0.15 + 0.7 * x,
        // One climax about three quarters through.
        _ => 0.2 + 0.6 * if x < 0.75 { x / 0.75 } else { (1.0 - x) / 0.25 },
    }
}

/// How busy phrase `ph` is against the others, 0.85 to 1.15, by the seed.
fn phrase_level(seed: u64, ph: usize) -> f32 {
    let mut d = DiceName {
        seed,
        part: 0,
        phrase: ph as u16,
        job: Job::Plan,
        bar: 0,
        attempt: 1,
    }
    .dice();
    0.85 + 0.3 * d.unit() as f32
}

/// The plan for `total` bars, the first `yours` of them yours. `density`
/// is your notes per beat, which decides how a contrast thins or fills.
pub fn plan(settings: &Settings, yours: usize, total: usize, density: f32) -> Vec<BarPlan> {
    let (pattern, shape) = shapes(&settings.style, settings.seed);
    let yours = yours.max(1);
    let contrast = (total >= 16).then(|| total / 4 - 2);
    let act = 0.5 + settings.activity * 1.1;
    let mut v = Vec::with_capacity(total);
    for b in 0..total {
        let pp = b % 4;
        let ph = b / 4;
        let mut tension = curve(shape, b, total);
        let mut activity = act;
        let source = b % yours;
        let (job, shift) = if b < yours {
            (BarJob::Yours, 0)
        } else if b + 1 == total {
            (BarJob::Land(settings.ending), 0)
        } else if Some(ph) == contrast {
            tension += 0.15;
            activity *= if density > 1.5 { 0.6 } else { 1.4 };
            match pp {
                0 | 1 => (BarJob::Contrast, 0),
                2 => (BarJob::Develop, 1),
                _ => (BarJob::Land(Ending::Open), 0),
            }
        } else {
            match pattern {
                // Your bar round again, but never more than twice running:
                // the third bar moves it up or down the scale and the
                // fourth varies it or leaves it open.
                PlanShape::Loop => match pp {
                    0 if contrast.is_some_and(|c| ph == c + 1) => (BarJob::Return, 0),
                    0 => (BarJob::Loop, 0),
                    1 if ph % 2 == 1 => (BarJob::Vary, 0),
                    1 => (BarJob::Loop, 0),
                    2 => (BarJob::Sequence, if ph % 2 == 0 { 1 } else { -1 }),
                    _ if ph % 2 == 0 => (BarJob::Land(Ending::Open), 0),
                    _ => (BarJob::Vary, 0),
                },
                PlanShape::CallAndAnswer => match pp {
                    0 => (
                        if ph == 0 {
                            BarJob::Repeat
                        } else {
                            BarJob::Return
                        },
                        0,
                    ),
                    1 => (BarJob::Answer, 0),
                    2 => (BarJob::Vary, if ph % 2 == 0 { 0 } else { 1 }),
                    _ => (
                        BarJob::Land(if ph % 2 == 0 {
                            Ending::Open
                        } else {
                            Ending::Home
                        }),
                        0,
                    ),
                },
                _ => match pp {
                    0 if ph == 0 => (BarJob::Repeat, 0),
                    0 if contrast.is_some_and(|c| ph == c + 1) => (BarJob::Return, 0),
                    0 => (BarJob::Sequence, if ph % 2 == 1 { 1 } else { -1 }),
                    1 => (BarJob::Vary, 0),
                    2 => {
                        activity *= 1.2;
                        (BarJob::Develop, 1)
                    }
                    _ => (
                        BarJob::Land(if ph % 2 == 0 {
                            Ending::Open
                        } else {
                            Ending::Home
                        }),
                        0,
                    ),
                },
            }
        };
        match job {
            BarJob::Land(Ending::Home) => tension -= 0.15,
            BarJob::Land(_) | BarJob::Develop => tension += 0.05,
            _ => {}
        }
        if matches!(job, BarJob::Land(_)) {
            activity *= 0.8;
        }
        // Busier and sparser stretches: a phrase's second bar a little
        // sparser and its third busier, every bar busier as the tension
        // rises, and each phrase a little busier or calmer by the seed.
        // The listening tests' first round heard every bar as busy as the
        // last.
        let lift = [1.0, 0.9, 1.15, 1.0][pp];
        activity *= lift * (0.8 + 0.4 * tension.clamp(0.0, 1.0)) * phrase_level(settings.seed, ph);
        // The tense bars that rework your bar lift it up the scale (up
        // to a fourth at the peak), so the line climbs to a high point and
        // comes back for the returns and landings, which stay where you
        // played.
        let shift = if matches!(job, BarJob::Vary | BarJob::Answer | BarJob::Contrast) {
            shift + ((tension - 0.4) * 5.0).round().clamp(-1.0, 3.0) as i32
        } else {
            shift
        };
        // A return's source is your first bar.
        let source = if job == BarJob::Return { 0 } else { source };
        let similarity = (job.similarity() + (settings.similarity - 0.5) * 0.6).clamp(0.05, 0.98);
        v.push(BarPlan {
            job,
            source,
            shift,
            tension: tension.clamp(0.0, 1.0),
            activity,
            similarity,
        });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Style, GROOVE, JAZZISH, MINIMAL};

    fn settings(style: Style) -> Settings {
        Settings {
            style,
            similarity: 0.6,
            ..Settings::default()
        }
    }

    /// The prototype's 16 bars in Jazz-ish: a sentence with one climax.
    fn sixteen() -> Vec<BarPlan> {
        let mut s = settings(JAZZISH);
        // A seed whose curve is the climax.
        s.seed = (0..64)
            .find(|&seed| shapes(&JAZZISH, seed).1 == PlanShape::OneClimax)
            .unwrap();
        plan(&s, 2, 16, 1.0)
    }

    #[test]
    fn sixteen_bars_contrast_peak_return_and_land_as_the_prototype_did() {
        let p = sixteen();
        let jobs: Vec<&str> = p.iter().map(|b| b.job.words()).collect();
        assert_eq!(
            jobs,
            vec![
                "yours",
                "yours",
                "develop",
                "leave open",
                "sequence",
                "vary",
                "develop",
                "land home",
                "contrast",
                "contrast",
                "develop",
                "leave open",
                "return",
                "vary",
                "develop",
                "land home"
            ]
        );
        // Bars 9 and 10 contrast at about 0.3, bar 12 peaks at 0.97, bar
        // 13 returns aiming at 0.91, bar 16 lands home at 0.13.
        assert!((p[8].similarity - 0.31).abs() < 0.01);
        let peak = p
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.tension.total_cmp(&b.1.tension))
            .unwrap();
        assert_eq!(peak.0, 11);
        assert!((peak.1.tension - 0.975).abs() < 0.01);
        assert!((p[12].similarity - 0.91).abs() < 0.01);
        assert_eq!(p[12].source, 0);
        assert!((p[15].tension - 0.125).abs() < 0.01);
    }

    #[test]
    fn a_loop_goes_round_at_most_twice_and_tense_bars_climb() {
        for seed in 0..16 {
            let s = Settings {
                seed,
                ..settings(MINIMAL)
            };
            let p = plan(&s, 2, 34, 1.0);
            for w in p.windows(3) {
                assert!(
                    !w.iter().all(|b| b.job == BarJob::Loop),
                    "seed {seed}: three loops running"
                );
            }
        }
        for style in [MINIMAL, GROOVE, JAZZISH] {
            let s = settings(style);
            let p = plan(&s, 2, 18, 1.0);
            let reworked =
                |b: &&BarPlan| matches!(b.job, BarJob::Vary | BarJob::Answer | BarJob::Contrast);
            // The tensest reworked bar sits higher than the calmest.
            let hi = p
                .iter()
                .filter(reworked)
                .max_by(|a, b| a.tension.total_cmp(&b.tension));
            let lo = p
                .iter()
                .filter(reworked)
                .min_by(|a, b| a.tension.total_cmp(&b.tension));
            if let (Some(hi), Some(lo)) = (hi, lo) {
                assert!(
                    hi.shift > lo.shift || hi.tension - lo.tension < 0.2,
                    "{}",
                    style.name
                );
                assert!(hi.shift >= 1, "{}: {hi:?}", style.name);
            }
        }
    }

    #[test]
    fn each_style_has_its_own_pattern() {
        let words = |s: Style| -> Vec<&'static str> {
            plan(&settings(s), 2, 8, 1.0)
                .iter()
                .map(|b| b.job.words())
                .collect()
        };
        assert_eq!(
            words(MINIMAL),
            vec![
                "yours",
                "yours",
                "sequence",
                "leave open",
                "loop",
                "vary",
                "sequence",
                "land home"
            ]
        );
        assert_eq!(
            words(GROOVE),
            vec![
                "yours",
                "yours",
                "vary",
                "leave open",
                "return",
                "answer",
                "vary",
                "land home"
            ]
        );
        assert_eq!(
            words(JAZZISH),
            vec![
                "yours",
                "yours",
                "develop",
                "leave open",
                "sequence",
                "vary",
                "develop",
                "land home"
            ]
        );
    }

    #[test]
    fn the_seed_picks_the_curve_from_the_styles_own() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0..64 {
            let (pattern, curve) = shapes(&MINIMAL, seed);
            assert_eq!(pattern, PlanShape::Loop);
            assert!(MINIMAL.development.plans.contains(&curve));
            seen.insert(curve);
        }
        assert_eq!(seen.len(), 2, "both of Minimal's curves come up");
        // A slow build rises to the end.
        let mut s = settings(MINIMAL);
        s.seed = (0..64)
            .find(|&seed| shapes(&MINIMAL, seed).1 == PlanShape::SlowBuild)
            .unwrap();
        s.ending = Ending::Open;
        let p = plan(&s, 1, 8, 1.0);
        assert!(p[6].tension > p[2].tension);
    }

    #[test]
    fn busier_and_sparser_stretches() {
        let mut s = settings(JAZZISH);
        s.seed = 4;
        let p = plan(&s, 2, 16, 1.0);
        // A phrase's third bar is busier than its second.
        for ph in 1..4 {
            let b = ph * 4;
            if p[b + 1].job != BarJob::Contrast && p[b + 2].job != BarJob::Contrast {
                assert!(p[b + 2].activity > p[b + 1].activity, "{ph}: {p:?}");
            }
        }
        // Not every bar as busy as the last.
        let acts: Vec<f32> = p[2..].iter().map(|b| b.activity).collect();
        let lo = acts.iter().copied().fold(f32::MAX, f32::min);
        let hi = acts.iter().copied().fold(0.0, f32::max);
        assert!(hi > 1.4 * lo, "{acts:?}");
        // The phrases' levels come from the seed, and stay inside 15 %.
        assert_eq!(phrase_level(4, 2), phrase_level(4, 2));
        assert!((0..64).all(|ph| (0.85..=1.15).contains(&phrase_level(9, ph))));
    }

    #[test]
    fn similarity_moves_every_aim_and_activity_scales() {
        let mut s = settings(GROOVE);
        s.similarity = 1.0;
        let close = plan(&s, 2, 8, 1.0);
        s.similarity = 0.0;
        let free = plan(&s, 2, 8, 1.0);
        for (c, f) in close.iter().zip(&free).skip(2) {
            assert!(c.similarity > f.similarity);
        }
        s.activity = 1.0;
        let busy = plan(&s, 2, 8, 1.0);
        s.activity = 0.0;
        let sparse = plan(&s, 2, 8, 1.0);
        assert!(busy[4].activity > 2.0 * sparse[4].activity);
    }
}
