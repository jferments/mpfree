//! The simpler ways (phase 0C): each job done plainly, to be played
//! beside mpfree's result, unlabelled, in the listening tests the design
//! sets (the second review's list). Where mpfree doesn't clearly win, that
//! stage is reworked or dropped for the simpler way: the extra complexity
//! has to earn its place.
//!
//! | Job | Played beside |
//! |---|---|
//! | Continue a melody | your phrase repeated ([`Simpler::Repeat`]), or moved in the scale ([`Simpler::Moved`]) |
//! | Styles | random notes from the style's scale and rhythm ([`Simpler::Random`]) |
//! | Groove | the grid, or a fixed swing ([`Feel`]) |
//! | 16 bars | a four-bar motif repeated with small random changes ([`motif_repeated`]) |
//! | Try again on a bar | the same bar made without looking at the bar after it ([`try_bar_one_sided`]) |
//!
//! A bass part beside root notes on the kicks comes with Add a part
//! (phase 2). Every way returns the new notes as Keep would write them, in
//! the clip's ticks, so a listening page can play either in its place.

use mpfree_context::groove::STEP;
use mpfree_seq::Note;
use mpfree_transport::time::Tick;

use crate::dice::{Dice, DiceName, Job};
use crate::motif::GridNote;
use crate::phrase::try_again;
use crate::request::{perform, phrasing, touch, your_bars, Continue, Continued, Refused};
use crate::rhythm::fresh;
use crate::timing::ms_to_ticks;

/// A plain way to carry a clip on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Simpler {
    /// Your bars again, exactly as played, to fill the new bars.
    Repeat,
    /// Your bars again as played, a scale step up, then a step down, by
    /// turns.
    Moved,
    /// Random notes from the key's scale around your tune, in rhythms made
    /// from the style's density and syncopation, played with the style's
    /// accents on your touch (as mpfree's own notes are).
    Random,
}

impl Simpler {
    pub const fn words(self) -> &'static str {
        match self {
            Simpler::Repeat => "your phrase repeated",
            Simpler::Moved => "your phrase moved in the scale",
            Simpler::Random => "random notes from the style's scale and rhythm",
        }
    }
}

fn dice(seed: u64, job: Job, bar: usize) -> Dice {
    DiceName {
        seed,
        part: 0x5_1e,
        phrase: 0,
        job,
        bar: bar as u16,
        attempt: 0,
    }
    .dice()
}

/// Grid notes, `from` steps on, as clip notes with the style's gate.
fn notes_of(line: &[GridNote], key: mpfree_context::key::Key, gate: f64) -> Vec<Note> {
    line.iter()
        .map(|m| {
            let p = m.plain(key);
            Note::new(
                p.at,
                ((p.len as f64 * gate) as Tick).max(STEP / 3),
                p.pitch,
                p.velocity.clamp(1, 127),
            )
        })
        .collect()
}

/// The new bars made the plain way, as clip notes.
pub fn simpler(req: &Continue, way: Simpler) -> Vec<Note> {
    let s = req.settings;
    let empty = mpfree_context::chord::ChordTimeline::default();
    let ph = phrasing(req, &empty);
    let grid = ph.grid;
    let yours = your_bars(req);
    let total = yours + usize::from(s.bars.max(1));
    let bar = req.meter.bar_ticks().max(1);
    let mut out: Vec<Note> = Vec::new();
    let mut line: Vec<GridNote> = Vec::new();
    let touch = touch(req);
    for b in yours..total {
        let k = b - yours;
        let from = b % yours;
        match way {
            Simpler::Repeat | Simpler::Moved => {
                // Your notes as you played them, timing, lengths and loudness.
                let shift = match way {
                    Simpler::Moved if (k / yours).is_multiple_of(2) => 1,
                    Simpler::Moved => -1,
                    _ => 0,
                };
                for (n, _) in req
                    .notes
                    .iter()
                    .filter(|(n, _)| n.at.div_euclid(bar) as usize == from)
                {
                    let (pos, alt) = ph.key.position(n.pitch);
                    let pitch = if shift == 0 {
                        n.pitch
                    } else {
                        ph.key.pitch_at(pos + shift, alt)
                    };
                    out.push(Note::new(
                        n.at + (b - from) as Tick * bar,
                        n.len.max(1),
                        pitch,
                        n.velocity.clamp(1, 127),
                    ));
                }
            }
            Simpler::Random => {
                let off = b as i32 * grid.bar;
                let mut d = dice(s.seed, Job::Variation, b);
                let r = fresh(&s.style.rhythm, 1.0, &[], grid, &mut d);
                let range = s.range.steps();
                let p = &s.style.playing;
                for o in r {
                    let pos = ph.centre - range + d.below((2 * range + 1) as usize) as i32;
                    let accent = p.accent[(o.on as usize) % p.accent.len()];
                    line.push(GridNote {
                        on: o.on + off,
                        len: o.len,
                        pos,
                        alt: 0,
                        velocity: (i32::from(p.velocity) + i32::from(accent)).clamp(20, 127) as u8,
                    });
                }
            }
        }
    }
    // The random notes at the style's level and accents, leaned toward
    // your touch as mpfree's own are.
    let mut loud: Vec<f64> = line.iter().map(|m| f64::from(m.velocity)).collect();
    touch.lean(&mut loud);
    for (m, v) in line.iter_mut().zip(loud) {
        m.velocity = v.round().clamp(20.0, 127.0) as u8;
    }
    out.extend(notes_of(&line, ph.key, touch.gate(&s.style.playing)));
    out.sort_by_key(|n| (n.tick, n.note));
    out
}

/// A four-bar motif repeated with small random changes: the first four
/// bars of `made` (yours and its first new bars), then again and again to
/// its end, each note now and then moved a scale step, held over or left
/// out. Beside mpfree's long form, which develops, contrasts and returns.
pub fn motif_repeated(req: &Continue, made: &Continued) -> Vec<Note> {
    let s = req.settings;
    let ph = phrasing(req, &made.chords);
    let grid = ph.grid;
    let yours = your_bars(req);
    let total = made.plan.len();
    let motif: Vec<Vec<GridNote>> = (0..4).map(|b| made.line.bar(b, grid)).collect();
    let mut line: Vec<GridNote> = Vec::new();
    for b in yours..total {
        let mut d = dice(s.seed, Job::Variation, b);
        let off = b as i32 * grid.bar;
        if b < 4 {
            // Its own first bars stay as made.
            line.extend(motif[b].iter().map(|m| GridNote {
                on: m.on + off,
                ..*m
            }));
            continue;
        }
        let src = &motif[b % 4];
        let mut i = 0;
        while i < src.len() {
            let mut m = src[i];
            m.on += off;
            let roll = d.unit();
            if roll < 0.12 {
                m.pos += if d.chance(0.5) { 1 } else { -1 };
                m.alt = 0;
            } else if roll < 0.18 && i + 1 < src.len() {
                // Held over the next note.
                m.len += src[i + 1].len;
                i += 1;
            } else if roll < 0.22 && i > 0 {
                i += 1;
                continue;
            }
            line.push(m);
            i += 1;
        }
    }
    notes_of(&line, ph.key, touch(req).gate(&s.style.playing))
}

/// The plain timings for the groove test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Feel {
    /// Every note on its grid line; with the clip's own swing, that is the
    /// fixed-swing way.
    Grid,
    /// Ordinary humanize: separate random timing for every note, this
    /// spread in ms.
    Humanize { ms: f32 },
}

/// mpfree's notes with their groove and drift taken off, and the plain
/// feel put on instead.
pub fn regroove(made: &Continued, feel: Feel, bpm: f64, seed: u64) -> Vec<Note> {
    let mut d = dice(seed, Job::Groove, 0);
    let mut normal = || {
        let u = d.unit().clamp(1e-12, 1.0);
        let w = d.unit();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * w).cos()
    };
    let len = made.to.max(1);
    let mut out: Vec<Note> = made
        .events
        .iter()
        .map(|e| {
            let shift = match feel {
                Feel::Grid => 0.0,
                Feel::Humanize { ms } => normal() * ms_to_ticks(f64::from(ms), bpm),
            };
            let at = (e.at as f64 + shift).round() as Tick;
            Note::new(
                at.rem_euclid(len),
                e.len.max(1),
                e.pitch,
                e.velocity.clamp(1, 127),
            )
        })
        .collect();
    out.sort_by_key(|n| (n.tick, n.note));
    out
}

/// Try one bar again without looking at the bar after it: the simpler way
/// for the boundary test, otherwise the same as [`crate::try_bar`].
pub fn try_bar_one_sided(
    req: &Continue,
    before: &Continued,
    bar: usize,
    attempt: u16,
) -> Result<Continued, Refused> {
    let mut ph = phrasing(req, &before.chords);
    ph.two_sided = false;
    ph.loops = false;
    let line = try_again(&ph, &before.plan, req.settings, &before.line, bar, attempt)?;
    let (part, events) = perform(req, &ph, &before.plan, &before.chords, &line);
    let bt = req.meter.bar_ticks().max(1);
    let inside = |at: Tick| at.div_euclid(bt) as usize == bar;
    let mut ev: Vec<_> = before
        .events
        .iter()
        .filter(|e| !inside(e.at))
        .copied()
        .chain(events.iter().filter(|e| inside(e.at)).copied())
        .collect();
    ev.sort_by_key(|e| e.at);
    let notes = crate::keep::clip_notes(&ev, req.bpm, before.to)?;
    Ok(Continued {
        line,
        part,
        events: ev,
        notes,
        ..before.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Source;
    use crate::motif::{bar_of, line, similarity};
    use crate::profile::InstrumentKind;
    use crate::realize::PlainNote;
    use crate::request::continue_notes;
    use crate::settings::Settings;
    use crate::style::{Style, GROOVE, JAZZISH, MINIMAL};
    use mpfree_context::key::{Key, Mode};
    use mpfree_context::listen::{listen, Played};
    use mpfree_context::{MusicalContext, PartRole, ReadBudget};
    use mpfree_transport::time::{Meter, Swing, PPQ};

    const BAR: Tick = 4 * PPQ as Tick;
    const CM: Key = Key::new(0, Mode::Minor);

    fn riff() -> Vec<(PlainNote, Source)> {
        [
            (0, 2, 60, 100),
            (2, 1, 63, 88),
            (3, 1, 67, 92),
            (4, 2, 65, 96),
            (6, 2, 63, 86),
            (8, 3, 62, 94),
            (11, 1, 63, 80),
            (12, 4, 60, 90),
            (16, 2, 67, 100),
            (18, 1, 68, 86),
            (19, 1, 67, 84),
            (20, 2, 65, 94),
            (22, 2, 63, 84),
            (24, 2, 62, 92),
            (26, 2, 63, 84),
            (28, 4, 65, 88),
        ]
        .iter()
        .map(|&(on, len, pitch, velocity)| {
            (
                PlainNote {
                    at: on * STEP,
                    len: len * STEP,
                    pitch,
                    velocity,
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
            cut_short: false,
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

    /// Chord tones on the beat, against the chords the result was made
    /// over.
    fn on_chords(notes: &[Note], made: &Continued) -> f64 {
        let beats: Vec<&Note> = notes.iter().filter(|n| n.tick % PPQ as Tick == 0).collect();
        let hits = beats
            .iter()
            .filter(|n| {
                made.chords
                    .span_at(n.tick)
                    .and_then(|s| s.answer.chord())
                    .is_some_and(|c| c.has(i32::from(n.note)))
            })
            .count();
        hits as f64 / beats.len().max(1) as f64
    }

    #[test]
    fn the_plain_ways_fill_the_same_bars() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(GROOVE, 4, 3);
        let req = ask(&notes, &ctx, &s);
        let repeat = simpler(&req, Simpler::Repeat);
        // Your two bars, twice, in bars 3 to 6.
        assert_eq!(repeat.len(), 32);
        assert!(repeat.iter().all(|n| (2 * BAR..6 * BAR).contains(&n.tick)));
        assert_eq!(repeat[0].note, 60);
        assert_eq!(repeat[16].tick, 4 * BAR);
        // Exactly as played: timing, lengths and loudness.
        assert!(repeat[..16].iter().zip(&notes).all(|(r, (n, _))| {
            r.tick == n.at + 2 * BAR && r.len == n.len && r.velocity == n.velocity
        }));
        let moved = simpler(&req, Simpler::Moved);
        assert_eq!(moved.len(), 32);
        assert_eq!(moved[0].note, 62, "a step up in C minor");
        assert_eq!(moved[16].note, 58, "then a step down");
        let random = simpler(&req, Simpler::Random);
        assert!(random.len() >= 8);
        assert!(random.iter().all(|n| (2 * BAR..6 * BAR).contains(&n.tick)));
        assert_eq!(random, simpler(&req, Simpler::Random), "repeatable");
        for w in [Simpler::Repeat, Simpler::Moved, Simpler::Random] {
            assert!(!w.words().is_empty());
        }
    }

    #[test]
    fn mpfree_keeps_closer_to_the_chords_and_your_tune_than_random_notes() {
        let notes = riff();
        let ctx = context(&notes);
        let mut ours = (0.0, 0.0);
        let mut plain = (0.0, 0.0);
        let yours = line(&notes.iter().map(|x| x.0).collect::<Vec<_>>(), CM);
        for style in [MINIMAL, GROOVE, JAZZISH] {
            for seed in 0..10 {
                let s = settings(style, 4, seed);
                let req = ask(&notes, &ctx, &s);
                let made = continue_notes(&req).unwrap();
                let random = simpler(&req, Simpler::Random);
                // On the grid, so the drift doesn't hide notes from the count.
                let on_grid = regroove(&made, Feel::Grid, 120.0, 0);
                ours.0 += on_chords(&on_grid, &made);
                plain.0 += on_chords(&random, &made);
                let key = CM;
                let as_line = |v: &[Note]| -> Vec<GridNote> {
                    line(
                        &v.iter()
                            .map(|n| PlainNote {
                                at: n.tick,
                                len: n.len,
                                pitch: n.note,
                                velocity: n.velocity,
                            })
                            .collect::<Vec<_>>(),
                        key,
                    )
                };
                let first = bar_of(&yours, 0, 16);
                ours.1 += similarity(key, &bar_of(&as_line(&made.notes), 2, 16), &first).tune();
                plain.1 += similarity(key, &bar_of(&as_line(&random), 2, 16), &first).tune();
            }
        }
        assert!(
            ours.0 > plain.0,
            "chord tones on the beat {ours:?} {plain:?}"
        );
        assert!(ours.1 > plain.1, "like your tune {ours:?} {plain:?}");
    }

    #[test]
    fn the_long_form_moves_where_the_repeated_motif_stays() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(JAZZISH, 14, 16);
        let req = ask(&notes, &ctx, &s);
        let made = continue_notes(&req).unwrap();
        let plain = motif_repeated(&req, &made);
        assert!(plain.iter().all(|n| (2 * BAR..16 * BAR).contains(&n.tick)));
        // Bars 3 and 4 are the same in both: the motif is mpfree's own.
        let ours_34: Vec<&Note> = made
            .notes
            .iter()
            .filter(|n| n.tick < 4 * BAR - 60)
            .collect();
        let plain_34: Vec<&Note> = plain.iter().filter(|n| n.tick < 4 * BAR).collect();
        assert_eq!(ours_34.len(), plain_34.len());
        // mpfree's phrase changes what its bars do; the motif just repeats.
        let jobs: std::collections::BTreeSet<String> = made.plan[2..]
            .iter()
            .map(|p| format!("{:?}", p.job))
            .collect();
        assert!(jobs.len() >= 3, "{jobs:?}");
    }

    #[test]
    fn the_plain_feels_and_the_one_sided_try() {
        let notes = riff();
        let ctx = context(&notes);
        let s = settings(GROOVE, 4, 2);
        let req = ask(&notes, &ctx, &s);
        let made = continue_notes(&req).unwrap();
        let grid = regroove(&made, Feel::Grid, 120.0, 1);
        assert!(grid.iter().all(|n| n.tick % STEP == 0), "on the grid");
        assert_eq!(grid.len(), made.notes.len());
        let loose = regroove(&made, Feel::Humanize { ms: 12.0 }, 120.0, 1);
        assert!(loose.iter().filter(|n| n.tick % STEP != 0).count() > loose.len() / 2);
        let one = try_bar_one_sided(&req, &made, 4, 1).unwrap();
        assert_eq!(one.to, made.to);
        let outside = |v: &[Note]| -> Vec<Note> {
            v.iter()
                .copied()
                .filter(|n| !(4 * BAR - 60..5 * BAR - 60).contains(&n.tick))
                .collect()
        };
        assert_eq!(outside(&one.notes), outside(&made.notes));
    }
}
