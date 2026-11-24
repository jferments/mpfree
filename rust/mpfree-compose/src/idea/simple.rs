//! The simple way of making each kind of phrase: what a musician gets
//! with no composer, for the blind listen an idea is heard in beside it
//! (docs/dev/design/compose.md, "New ideas").
//!
//! - **A bass** is the chord's root on every kick, held to the next.
//! - **Chords** are a block chord on every bar line and every change,
//!   held to the next.
//! - **A riff** is notes of the key picked by chance, on a rhythm picked
//!   by chance in the style's feel, about as many notes as the composer's
//!   riff of the same request plays.
//! - **A line** is the chord's notes going up, one on every step, as the
//!   arp's Up plays a held chord.
//!
//! The first two are the simple way Add a part's listen plays (S2); a
//! riff's is the one Continue's styles were heard beside
//! ([`crate::baseline::Simpler::Random`]). Each is made over the same
//! band as the idea it is heard beside ([`Backing`]), written by the same
//! hand and judged by the same checks, so the two differ only in how
//! their notes were chosen, and the checks can be seen to tell them
//! apart or not.

use mpfree_arp::phrase::PLAIN_VELOCITY;
use mpfree_arp::PhraseKind;
use mpfree_context::groove::{Lane, STEP};
use mpfree_transport::time::Tick;

use super::line::judge_line;
use super::riff::{judge_riff, marks_of, GATE_HIGH, GATE_LOW, RIFF_LOW};
use super::{asked, dice, judge_part, new_idea, Backing, Draft, Idea, Made, NewIdea};
use crate::cells::{lengths, Stroke};
use crate::dice::Job;
use crate::event::{Event, Role, Source};
use crate::parts::{chord_near, unit_of};
use crate::rhythm::{fresh, Grid};
use crate::row::Row;
use crate::timing::Timing;

/// The draft number the simple way's dice are named by: none of the
/// composer's own drafts.
const SIMPLE: u16 = 0x51;

/// The lowest root of a simple way's block chord: G3, so that the home
/// chord stands on middle C, where the composer's chords lie.
const CHORDS_LOW: i32 = 55;

/// A note held to the next one.
const HELD: f64 = 1.0;
/// A note of the arp's own length: four fifths of its step.
const PLAIN: f64 = 0.8;

/// The simple way of making what `req` asks for: the phrase an idea of
/// the same request is heard beside.
pub fn simple_idea(req: &NewIdea) -> Idea {
    let over = Backing::of(req);
    let made = match req.kind {
        PhraseKind::Bass => bass(req, &over),
        PhraseKind::Chords => chords(req, &over),
        PhraseKind::Riff => riff(req, &over),
        PhraseKind::Line => line(req, &over),
    };
    over.idea(req.kind, made)
}

/// A note of `steps` steps at `gate` of its length.
fn note(row: &Row, at: i32, steps: i32, gate: f64, pitch: i32) -> Event {
    Event {
        at: row.tick(at),
        len: ((f64::from(steps) * gate * STEP as f64) as Tick).max(STEP / 3),
        pitch: pitch.clamp(0, 127) as u8,
        velocity: PLAIN_VELOCITY,
        role: match chord_near(row, at).and_then(|c| c.tone_of(pitch)) {
            Some(t) => Role::Chord(t),
            None => Role::Scale,
        },
        source: Source::Mpfree,
        timing: Timing::default(),
    }
}

/// Strokes at the steps of the row `on` picks, of the grid the style
/// writes on (every 1/16, or every 1/8 where it swings), each as long as
/// the way to the next. Nothing picked is one stroke on the first step.
fn strokes(req: &NewIdea, row: &Row, on: impl Fn(i32) -> bool) -> Vec<Stroke> {
    let unit = unit_of(&req.style);
    let mut v: Vec<Stroke> = (0..row.steps)
        .filter(|&s| s % unit == 0 && on(s))
        .map(Stroke::at)
        .collect();
    if v.is_empty() {
        v.push(Stroke::at(0));
    }
    lengths(&mut v, row.steps);
    v
}

/// The root of the chord at `step`, in the octave from `low` up.
fn root(row: &Row, step: i32, low: i32) -> i32 {
    let pc = chord_near(row, step).map_or(0, |c| i32::from(c.root % 12));
    low + (pc - low).rem_euclid(12)
}

/// The notes of the chord at `step`, close, from its root in the octave
/// from `low` up.
fn chord_notes(row: &Row, step: i32, low: i32) -> Vec<i32> {
    let low = root(row, step, low);
    match chord_near(row, step) {
        Some(c) => c
            .quality
            .intervals()
            .iter()
            .map(|&i| low + i32::from(i))
            .collect(),
        None => vec![low],
    }
}

/// The chord's root on every kick, held to the next; on every beat where
/// the beat has no kick to follow.
fn bass(req: &NewIdea, over: &Backing) -> Made {
    let row = &over.row;
    let kicks = (0..row.steps).any(|s| row.drum(Lane::Kick, s) > 0);
    let strokes = strokes(req, row, |s| {
        if kicks {
            row.drum(Lane::Kick, s) > 0
        } else {
            row.grid.on_beat(s)
        }
    });
    let events = strokes
        .iter()
        .map(|s| note(row, s.at, s.len, HELD, root(row, s.at, 36)))
        .collect();
    let band = asked(req, row, &over.settings);
    Draft {
        strokes,
        events,
        accents: Vec::new(),
    }
    .heard(row, over.swung, |strokes, heard| {
        judge_part(&band, strokes, heard)
    })
}

/// A block chord on every bar line and every change, held to the next:
/// the chord's notes from its root up, close, around middle C.
fn chords(req: &NewIdea, over: &Backing) -> Made {
    let row = &over.row;
    let strokes = strokes(req, row, |s| s % row.grid.bar == 0 || row.chord_starts(s));
    let events = strokes
        .iter()
        .flat_map(|s| {
            chord_notes(row, s.at, CHORDS_LOW)
                .into_iter()
                .map(|p| note(row, s.at, s.len, HELD, p))
        })
        .collect();
    let band = asked(req, row, &over.settings);
    Draft {
        strokes,
        events,
        accents: Vec::new(),
    }
    .heard(row, over.swung, |strokes, heard| {
        judge_part(&band, strokes, heard)
    })
}

/// Notes of the key in the octave a riff is made from (the fourth under
/// middle C to the fifth over it), each picked by chance, on a rhythm
/// picked by chance bar by bar in the style's feel
/// (the first engine's [`fresh`], on the grid the style writes on), with
/// as many notes a bar as the composer's riff of the same request plays,
/// so that neither is kept for being the busier. They sound as long as
/// the composer's do: the style's gate.
fn riff(req: &NewIdea, over: &Backing) -> Made {
    let row = &over.row;
    let unit = unit_of(&req.style);
    let grid = Grid {
        bar: row.grid.bar / unit,
        beat: row.grid.beat / unit,
    };
    let bars = (row.steps / row.grid.bar).max(1);
    let mut theirs: Vec<Tick> = new_idea(req).events.iter().map(|e| e.at).collect();
    theirs.dedup();
    let rhythm = &req.style.rhythm;
    let mut dice = dice(req, Job::Pitch, SIMPLE);
    let mut on: Vec<i32> = Vec::new();
    for b in 0..bars {
        // The bars share the notes out evenly, the first ones taking
        // what is left over.
        let n = theirs.len() as i32 / bars + i32::from(b < theirs.len() as i32 % bars);
        let busy = f64::from(n) / (f64::from(rhythm.density) * f64::from(grid.bar / grid.beat));
        on.extend(
            fresh(rhythm, busy, &[], grid, &mut dice)
                .iter()
                .map(|o| b * row.grid.bar + o.on * unit),
        );
    }
    let strokes = strokes(req, row, |s| on.contains(&s));
    let notes: Vec<i32> = (RIFF_LOW..=RIFF_LOW + 12)
        .filter(|&p| over.key.has(p))
        .collect();
    let gate = f64::from(req.style.playing.gate).clamp(GATE_LOW, GATE_HIGH);
    let events = strokes
        .iter()
        .map(|s| note(row, s.at, s.len, gate, notes[dice.below(notes.len())]))
        .collect();
    Draft {
        strokes,
        events,
        accents: Vec::new(),
    }
    .heard(row, over.swung, |strokes, heard| {
        judge_riff(row, &req.style, marks_of(row), strokes, heard)
    })
}

/// The chord's notes going up from its root, one on every step, starting
/// again on every bar line and every change.
fn line(req: &NewIdea, over: &Backing) -> Made {
    let row = &over.row;
    let strokes = strokes(req, row, |_| true);
    let mut events = Vec::new();
    let mut k = 0usize;
    for s in &strokes {
        if s.at % row.grid.bar == 0 || row.chord_starts(s.at) {
            k = 0;
        }
        let notes = chord_notes(row, s.at, 60);
        events.push(note(row, s.at, s.len, PLAIN, notes[k % notes.len()]));
        k += 1;
    }
    Draft {
        strokes,
        events,
        accents: Vec::new(),
    }
    .heard(row, over.swung, |_, heard| {
        judge_line(row, &req.style, heard)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Feel, STYLES};
    use mpfree_arp::Phrase;

    /// Every style, scale and length of `kind`, sparse, middling and
    /// busy, `seeds` of each.
    fn requests(kind: PhraseKind, seeds: u64) -> Vec<NewIdea> {
        let mut v = Vec::new();
        for style in STYLES {
            for minor in [true, false] {
                for bars in [1u8, 2, 4] {
                    for activity in [0.2f32, 0.5, 0.8] {
                        for seed in 1..=seeds {
                            v.push(NewIdea {
                                kind,
                                style,
                                minor,
                                bars,
                                activity,
                                seed,
                                attempt: 0,
                            });
                        }
                    }
                }
            }
        }
        v
    }

    /// The steps the notes of `idea` start on, and the notes on each.
    fn by_step(idea: &Idea) -> Vec<(i32, Vec<i32>)> {
        let mut v: Vec<(i32, Vec<i32>)> = Vec::new();
        for e in &idea.events {
            let step = (e.at / STEP) as i32;
            match v.last_mut() {
                Some((s, notes)) if *s == step => notes.push(i32::from(e.pitch)),
                _ => v.push((step, vec![i32::from(e.pitch)])),
            }
        }
        v
    }

    #[test]
    fn the_simple_way_is_a_phrase_that_reads_back_from_its_text() {
        // The listening tool plays `Idea::phrase` of both, so the simple
        // way has to be a phrase as whole as the idea beside it.
        for kind in PhraseKind::ALL {
            for req in requests(kind, 1) {
                let idea = simple_idea(&req);
                assert_eq!(idea, simple_idea(&req), "{req:?}");
                let p = idea.phrase();
                assert!(!p.is_empty(), "{req:?}");
                assert_eq!((p.kind, p.bars), (req.kind, req.bars), "{req:?}");
                assert_eq!(p.swung, req.style.rhythm.feel == Feel::Swing8);
                assert_eq!(Phrase::parse(&p.to_text()), Ok(p), "{}", p.to_text());
            }
        }
    }

    #[test]
    fn the_simple_way_is_made_over_the_idea_s_own_chords() {
        // One band for both: the same loop of chords, key, length and
        // swing, so a pair differs only in its notes.
        for kind in PhraseKind::ALL {
            for req in requests(kind, 1).into_iter().step_by(5) {
                let (a, b) = (new_idea(&req), simple_idea(&req));
                assert_eq!(a.chords, b.chords, "{req:?}");
                assert_eq!((a.key, a.bars, a.swung), (b.key, b.bars, b.swung));
            }
        }
    }

    #[test]
    fn a_simple_bass_is_the_chord_s_root_on_every_kick() {
        for req in requests(PhraseKind::Bass, 1) {
            let row = Backing::of(&req).row;
            let unit = unit_of(&req.style);
            let idea = simple_idea(&req);
            let kicks: Vec<i32> = (0..row.steps)
                .filter(|&s| s % unit == 0 && row.drum(Lane::Kick, s) > 0)
                .collect();
            assert!(!kicks.is_empty(), "{req:?}: every style's beat has a kick");
            let played = by_step(&idea);
            assert_eq!(
                played.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
                kicks,
                "{req:?}"
            );
            for (step, notes) in played {
                let chord = chord_near(&row, step).expect("a chord");
                assert_eq!(notes.len(), 1);
                assert_eq!(notes[0].rem_euclid(12), i32::from(chord.root), "{req:?}");
                assert!((36..48).contains(&notes[0]));
            }
        }
    }

    #[test]
    fn simple_chords_are_a_block_chord_on_every_bar_line_and_change() {
        let mut changes = 0;
        for req in requests(PhraseKind::Chords, 3) {
            let row = Backing::of(&req).row;
            let unit = unit_of(&req.style);
            let idea = simple_idea(&req);
            let on: Vec<i32> = (0..row.steps)
                .filter(|&s| s % unit == 0 && (s % row.grid.bar == 0 || row.chord_starts(s)))
                .collect();
            changes += on.iter().filter(|&&s| s % row.grid.bar != 0).count();
            let played = by_step(&idea);
            assert_eq!(
                played.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
                on,
                "{req:?}"
            );
            for (i, (step, notes)) in played.iter().enumerate() {
                let chord = chord_near(&row, *step).expect("a chord");
                // The chord whole, from its root up, close.
                assert_eq!(notes.len(), chord.quality.intervals().len(), "{req:?}");
                assert_eq!(notes[0].rem_euclid(12), i32::from(chord.root));
                assert!((CHORDS_LOW..CHORDS_LOW + 12).contains(&notes[0]));
                assert!(notes.iter().all(|&p| chord.has(p) && p - notes[0] < 12));
                // Held to the next chord, the last one to the phrase's end.
                let until = played.get(i + 1).map_or(row.steps, |(s, _)| *s);
                let len = Tick::from(until - step) * STEP;
                assert!(
                    idea.events
                        .iter()
                        .filter(|e| e.at == row.tick(*step))
                        .all(|e| e.len == len),
                    "{req:?}"
                );
            }
        }
        // Some loops change chord inside a bar.
        assert!(changes > 0);
    }

    #[test]
    fn a_simple_riff_is_notes_of_the_key_about_as_many_as_the_composer_s() {
        let (mut ours, mut theirs) = (0usize, 0usize);
        for req in requests(PhraseKind::Riff, 2) {
            let over = Backing::of(&req);
            let idea = simple_idea(&req);
            let played = by_step(&idea);
            assert_eq!(played[0].0, 0, "{req:?}: a rhythm stands on the downbeat");
            for (_, notes) in &played {
                assert_eq!(notes.len(), 1);
                assert!(over.key.has(notes[0]), "{req:?}");
                assert!((RIFF_LOW..=RIFF_LOW + 12).contains(&notes[0]), "{req:?}");
            }
            ours += played.len();
            theirs += by_step(&new_idea(&req)).len();
        }
        // A bar plays two notes at least and now and then rests a beat,
        // so the two are as busy as each other all told, not riff by
        // riff.
        let share = ours as f64 / theirs as f64;
        assert!((0.85..=1.1).contains(&share), "{ours} notes to {theirs}");
    }

    #[test]
    fn a_riff_s_checks_tell_notes_picked_by_chance_from_a_riff() {
        // Every one of the composer's riffs of these requests passes
        // every check (`ideas_pass_their_checks`); of the simple way's,
        // fewer than one in three does.
        let reqs = requests(PhraseKind::Riff, 2);
        let pass = reqs.iter().filter(|r| simple_idea(r).verdict.all()).count();
        assert!(pass * 3 < reqs.len(), "{pass} of {}", reqs.len());
    }

    #[test]
    fn a_simple_line_is_the_chord_s_notes_going_up_on_every_step() {
        for req in requests(PhraseKind::Line, 1) {
            let row = Backing::of(&req).row;
            let unit = unit_of(&req.style);
            let idea = simple_idea(&req);
            let played = by_step(&idea);
            assert_eq!(
                played.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
                (0..row.steps).filter(|s| s % unit == 0).collect::<Vec<_>>(),
                "{req:?}"
            );
            let mut k = 0;
            for (step, notes) in &played {
                let chord = chord_near(&row, *step).expect("a chord");
                if step % row.grid.bar == 0 || row.chord_starts(*step) {
                    k = 0;
                }
                let ivs = chord.quality.intervals();
                let low = 60 + (i32::from(chord.root) - 60).rem_euclid(12);
                assert_eq!(notes[..], [low + i32::from(ivs[k % ivs.len()])], "{req:?}");
                k += 1;
            }
        }
    }

    /// The simple way beside the composer's, to read: how many notes a
    /// bar each plays, how many pass every check, and (with `IDEA_SHOW=1`)
    /// the first of each as a phrase. `cargo test --release -p
    /// mpfree-compose --lib print_the_simple_way_beside -- --ignored
    /// --nocapture`
    #[test]
    #[ignore]
    fn print_the_simple_way_beside() {
        let show = std::env::var("IDEA_SHOW").is_ok();
        for kind in PhraseKind::ALL {
            for style in STYLES {
                for activity in [0.2f32, 0.5, 0.8] {
                    let (mut n, mut ns, mut pass, mut pass_s, mut of, mut bars_all) =
                        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
                    for bars in [1u8, 2, 4] {
                        for minor in [true, false] {
                            for seed in 1..=10u64 {
                                let req = NewIdea {
                                    kind,
                                    style,
                                    minor,
                                    bars,
                                    activity,
                                    seed,
                                    attempt: 0,
                                };
                                let (a, b) = (new_idea(&req), simple_idea(&req));
                                let starts = |i: &Idea| {
                                    let mut t: Vec<Tick> = i.events.iter().map(|e| e.at).collect();
                                    t.dedup();
                                    t.len()
                                };
                                n += starts(&a);
                                ns += starts(&b);
                                pass += usize::from(a.verdict.all());
                                pass_s += usize::from(b.verdict.all());
                                of += 1;
                                bars_all += usize::from(bars);
                                if show && seed == 1 && minor && activity == 0.5 {
                                    println!(
                                        "{} {} {bars} bars [{}]\n  composer {}/{}: {}\n  simple   {}/{}: {}\n    {}",
                                        style.name,
                                        kind.label(),
                                        a.chords.names(),
                                        a.verdict.passed(),
                                        a.verdict.checks.len(),
                                        a.phrase().to_text(),
                                        b.verdict.passed(),
                                        b.verdict.checks.len(),
                                        b.phrase().to_text(),
                                        b.verdict.words(),
                                    );
                                }
                            }
                        }
                    }
                    println!(
                        "{} {} at {activity}: {:.1} starts a bar, the simple way {:.1}; {pass} of {of} pass every check, the simple way {pass_s}",
                        kind.label(),
                        style.name,
                        n as f64 / bars_all as f64,
                        ns as f64 / bars_all as f64,
                    );
                }
            }
        }
    }
}
