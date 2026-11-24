//! Bridge between two clips (revision 3, S5): new bars that carry one clip
//! on and lead into another.
//!
//! A bridge is Continue on the first clip with two changes. Its second
//! half grows from the next clip's bars instead of the first clip's, so it
//! starts like the one and ends like the other; and its last note leads
//! into the next clip's first note, over a chord that leads into the next
//! clip's first chord (its five, else its seven or four), the way a clip
//! that loops leads back into its own opening. The critic judges it as a
//! loop leading back, to the next clip's first note.
//!
//! It works in the first clip's key: the next clip's notes are read in it.

use mpfree_context::chord::ChordTimeline;
use mpfree_context::groove::STEP;
use mpfree_context::MusicalContext;
use mpfree_transport::time::Tick;

use crate::draft::{drafts_with, Draft, Into};
use crate::event::Source;
use crate::motif::{bar_of, line};
use crate::realize::PlainNote;
use crate::request::{phrasing, Continue, Continued, Refused};
use crate::settings::{Ending, Settings};

/// What a Bridge is given: the first clip's request (its settings say how
/// many bars, the style and the rest; its Ending is not asked) and the
/// clip it leads into.
#[derive(Clone, Copy)]
pub struct Bridge<'a> {
    pub from: Continue<'a>,
    /// The next clip's notes, in its own ticks.
    pub to: &'a [(PlainNote, Source)],
    /// The next clip's length, in its ticks.
    pub to_length: Tick,
    /// The next clip's reading (its chords).
    pub to_context: &'a MusicalContext,
}

/// Every draft of the bridge, best first.
pub fn bridge_drafts(b: &Bridge) -> Result<Vec<Draft>, Refused> {
    let settings = Settings {
        ending: Ending::LoopsBack,
        ..*b.from.settings
    };
    let req = Continue {
        settings: &settings,
        loops: false,
        ..b.from
    };
    let into = into(&req, b)?;
    drafts_with(&req, None, Some(&into))
}

/// The bridge the critic picks.
pub fn bridge(b: &Bridge) -> Result<Continued, Refused> {
    Ok(bridge_drafts(b)?.remove(0).made)
}

/// Where the bridge leads, read from the next clip.
fn into(req: &Continue, b: &Bridge) -> Result<Into, Refused> {
    let empty = ChordTimeline::default();
    let ph = phrasing(req, &empty);
    let key = ph.key;
    let notes: Vec<PlainNote> = b.to.iter().map(|(n, _)| *n).collect();
    let tune = line(&notes, key);
    let first = tune.first().copied().ok_or(Refused::NoClip)?;
    // The chords that lead into its first chord: that chord's five, then
    // its seven and its four, as a clip leading back into its own home.
    let degree = b
        .to_context
        .chords
        .span_at(Tick::from(first.on) * STEP)
        .and_then(|s| s.answer.chord())
        .and_then(|c| {
            let (pos, alt) = key.position(60 + c.root);
            (alt == 0).then_some(pos.rem_euclid(7))
        })
        .unwrap_or(0);
    let chords = [4, 6, 3]
        .into_iter()
        .map(|d| ((degree + d) % 7) as u8)
        .collect();
    let gb = ph.grid.bar;
    let bar_ticks = req.meter.bar_ticks().max(1);
    let to_bars = ((b.to_length.max(1) + bar_ticks - 1) / bar_ticks).max(1) as i32;
    let bars = (0..to_bars)
        .map(|k| bar_of(&tune, k, gb))
        .filter(|bar| !bar.is_empty())
        .collect();
    Ok(Into {
        pos: first.pos,
        chords,
        bars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motif::{similarity, GridNote};
    use crate::profile::InstrumentKind;
    use crate::style::{GROOVE, JAZZISH, MINIMAL};
    use mpfree_context::listen::{listen, Played};
    use mpfree_context::{PartRole, ReadBudget};
    use mpfree_transport::time::{Meter, Swing, PPQ};

    const BAR: Tick = 4 * PPQ as Tick;

    fn clip(notes: &[(i64, i64, u8)]) -> Vec<(PlainNote, Source)> {
        notes
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

    /// The prototype's riff in C minor.
    fn riff() -> Vec<(PlainNote, Source)> {
        clip(&[
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
            (28, 4, 60),
        ])
    }

    /// Another idea in C minor: long notes falling from the fifth, in
    /// another rhythm.
    fn chorus() -> Vec<(PlainNote, Source)> {
        clip(&[
            (0, 6, 67),
            (6, 2, 70),
            (8, 4, 68),
            (12, 4, 67),
            (16, 6, 65),
            (22, 2, 63),
            (24, 8, 62),
        ])
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

    #[test]
    fn a_bridge_starts_like_one_clip_ends_like_the_other_and_leads_in() {
        let (a, b) = (riff(), chorus());
        let (ca, cb) = (context(&a), context(&b));
        let mut like = [[0.0f64; 2]; 2];
        let mut n = 0.0;
        for style in [MINIMAL, GROOVE, JAZZISH] {
            for seed in 1..=4 {
                let s = Settings {
                    style,
                    bars: 4,
                    seed,
                    ..Settings::default()
                };
                let from = Continue {
                    notes: &a,
                    length: 2 * BAR,
                    context: &ca,
                    settings: &s,
                    role: PartRole::Melody,
                    kind: InstrumentKind::OneNote,
                    meter: Meter::default(),
                    bpm: 120.0,
                    swing: Swing::default(),
                    loops: true,
                    row_length: 0,
                };
                let br = Bridge {
                    from,
                    to: &b,
                    to_length: 2 * BAR,
                    to_context: &cb,
                };
                let what = format!("{} seed {seed}", style.name);
                let best = bridge_drafts(&br).unwrap().remove(0);
                assert!(
                    best.verdict.passed() >= 5,
                    "{what}: {}",
                    best.verdict.words()
                );
                assert!(
                    best.verdict.lands.pass,
                    "{what}: {}",
                    best.verdict.lands.words
                );
                // The last note leads into the next clip's first.
                let ph = phrasing(&from, &best.made.chords);
                let key = ph.key;
                let gb = ph.grid.bar;
                let target = line(&b.iter().map(|x| x.0).collect::<Vec<_>>(), key)[0].pos;
                let last = best.made.line.notes.last().unwrap();
                assert!(
                    (last.pos - target).abs() <= 2 || last.pos.rem_euclid(7) == 4,
                    "{what}: ends on {} for {target}",
                    last.pos
                );
                // The first new bar is more like the first clip, the last
                // more like the next.
                let bar = |k: i32| bar_of(&best.made.line.notes, k, gb);
                let ya = line(&a.iter().map(|x| x.0).collect::<Vec<_>>(), key);
                let yb = line(&b.iter().map(|x| x.0).collect::<Vec<_>>(), key);
                let mean = |x: &[GridNote], of: &[GridNote]| {
                    (0..2)
                        .map(|k| similarity(key, x, &bar_of(of, k, gb)).total)
                        .fold(0.0, f64::max)
                };
                for (i, k) in [2, 4].into_iter().enumerate() {
                    like[i][0] += mean(&bar(k), &ya);
                    like[i][1] += mean(&bar(k), &yb);
                }
                n += 1.0;
            }
        }
        let [[first_a, first_b], [late_a, late_b]] = like.map(|r| r.map(|x| x / n));
        println!("bridge: first new bar like A {first_a:.2} B {first_b:.2}; last like A {late_a:.2} B {late_b:.2}");
        assert!(
            first_a > first_b,
            "the first new bar: like A {first_a:.2}, like B {first_b:.2}"
        );
        assert!(
            late_b > late_a,
            "the late bar: like A {late_a:.2}, like B {late_b:.2}"
        );
    }

    #[test]
    fn a_bridge_into_nothing_is_refused() {
        let a = riff();
        let ca = context(&a);
        let s = Settings::default();
        let from = Continue {
            notes: &a,
            length: 2 * BAR,
            context: &ca,
            settings: &s,
            role: PartRole::Melody,
            kind: InstrumentKind::OneNote,
            meter: Meter::default(),
            bpm: 120.0,
            swing: Swing::default(),
            loops: false,
            row_length: 0,
        };
        let empty = MusicalContext::empty();
        let br = Bridge {
            from,
            to: &[],
            to_length: 2 * BAR,
            to_context: &empty,
        };
        assert!(matches!(bridge(&br), Err(Refused::NoClip)));
    }
}
