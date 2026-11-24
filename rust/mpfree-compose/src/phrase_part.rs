//! A phrase as Add a part's part (P2): Add a part plays a phrase over the
//! row's chords, its rhythm and shape, following each chord's root, in
//! key.
//!
//! The phrase loops over the row as the phrase player loops it. Each of
//! its notes is moved from its home note to the root of the chord it
//! sounds over, by steps of the row's key, as the player moves a phrase
//! In key from the note you hold: a bass line written from C2 plays from
//! each chord's root, chord stabs written as 1, 3, 5 become each chord of
//! the key. A note written outside its scale keeps its half step. The
//! root sits in the part's own register (a bass around G2, chords and a
//! second line around middle C), the octave nearest the last root, so the
//! part moves by the short way round.
//!
//! What it makes is judged by Add a part's checks for that part and
//! relation, as a part made by the composer is, and is kept the same way:
//! one take.

use mpfree_arp::Phrase;
use mpfree_context::groove::STEP;
use mpfree_context::PartRole;
use mpfree_transport::time::Tick;

use crate::cells::{lengths, Stroke};
use crate::event::{Event, Role, Source};
use crate::parts::{chord_near, judge, AddPart, Added, Refused};
use crate::timing::Timing;

/// A phrase's notes at or under this velocity are ghosts.
const GHOST: u8 = 48;

/// The middle of each part's register, where a chord's root is placed.
fn middle(part: PartRole) -> i32 {
    match part {
        PartRole::Bass => 43,
        PartRole::Melody => 67,
        _ => 60,
    }
}

/// `phrase` played as `req`'s part over its row. Refused for drums (a
/// phrase is pitched) and for a row with no chords, as Add a part is.
pub fn phrase_part(req: &AddPart, phrase: &Phrase) -> Result<Added, Refused> {
    let row = req.row;
    if req.part == PartRole::Drums {
        return Err(Refused::PhraseNotDrums);
    }
    if row.chords.spans().is_empty() {
        return Err(Refused::NoChords);
    }
    let key = row.key;
    let len = Tick::from(phrase.ticks().max(1));
    let mid = middle(req.part);
    let mut events: Vec<Event> = Vec::new();
    let mut last_root: Option<i32> = None;
    let mut round: Tick = 0;
    while round < row.length && !phrase.is_empty() {
        for n in phrase.notes() {
            let at = round + Tick::from(n.start);
            if at >= row.length {
                break;
            }
            let step = (at / STEP) as i32;
            let Some(chord) = chord_near(row, step) else {
                continue;
            };
            // The root in the part's register, the octave nearest the root
            // before (the middle, the first time).
            let near = last_root.unwrap_or(mid);
            let pc = i32::from(chord.root % 12);
            let root = (near - 6..=near + 6)
                .filter(|p| p.rem_euclid(12) == pc)
                .min_by_key(|p| ((p - near).abs(), (p - mid).abs()))
                .unwrap_or(mid);
            // Keep within an octave of the middle, so it never drifts.
            let root = if root > mid + 7 {
                root - 12
            } else if root < mid - 7 {
                root + 12
            } else {
                root
            };
            last_root = Some(root);
            let (pos, root_alt) = key.position(root.clamp(0, 127) as u8);
            let pitch =
                i32::from(key.pitch_at(pos + i32::from(n.steps), root_alt)) + i32::from(n.alt);
            let ghost = n.velocity <= GHOST;
            let end = (at + Tick::from(n.len)).min(row.length);
            events.push(Event {
                at,
                len: (end - at).max(1),
                pitch: pitch.clamp(0, 127) as u8,
                velocity: n.velocity,
                role: if ghost {
                    Role::Ghost
                } else {
                    match chord.tone_of(pitch) {
                        Some(t) => Role::Chord(t),
                        None => Role::Scale,
                    }
                },
                source: Source::Mpfree,
                timing: Timing::default(),
            });
        }
        round += len;
    }
    events.sort_by_key(|e| (e.at, e.pitch));
    let mut strokes: Vec<Stroke> = Vec::new();
    for e in &events {
        let at = (e.at / STEP) as i32;
        if strokes.last().is_some_and(|s| s.at == at) {
            continue;
        }
        strokes.push(Stroke {
            at,
            len: 1,
            ghost: e.role == Role::Ghost,
            pushed: false,
        });
    }
    lengths(&mut strokes, row.steps);
    let verdict = judge(req, &strokes, &events);
    Ok(Added {
        part: req.part,
        relation: req.relation,
        strokes,
        changes: Vec::new(),
        events,
        verdict,
        draft: 0,
    })
}

#[cfg(test)]
mod tests_support {
    pub(super) use super::*;
    pub(super) use crate::parts::Relation;
    use crate::profile::InstrumentKind;
    pub(super) use crate::row::{Row, ToHear};
    use crate::settings::Settings;
    use mpfree_context::chord::{Chord, ChordTimeline, Heard, Quality};
    use mpfree_context::key::{Candidate, Key, KeyReading, Mode};
    use mpfree_transport::time::{Meter, Swing};

    pub(super) const BAR: Tick = 4 * 960;

    pub(super) fn row(chords: &[(u8, Quality)]) -> Row {
        let key = Key::new(0, Mode::Minor);
        let reading = KeyReading::new(&[Candidate { key, odds: 0.9 }], 7);
        let timeline = ChordTimeline::of(
            &chords
                .iter()
                .map(|&(root, quality)| (Chord { root, quality }, BAR))
                .collect::<Vec<_>>(),
            Heard::Suggested,
        );
        Row::hear(&ToHear {
            parts: &[],
            length: chords.len() as Tick * BAR,
            meter: Meter { beats: 4, unit: 4 },
            key: &reading,
            played_pcs: key.pcs(),
            chords: &timeline,
            groove: None,
            lead: None,
        })
    }

    pub(super) fn ask<'a>(row: &'a Row, settings: &'a Settings, part: PartRole) -> AddPart<'a> {
        AddPart {
            row,
            part,
            relation: Relation::Support,
            settings,
            kind: match part {
                PartRole::Bass => InstrumentKind::Bass,
                PartRole::Chords => InstrumentKind::Chords,
                _ => InstrumentKind::OneNote,
            },
            pads: [None; 4],
            swing: Swing::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use crate::settings::Settings;
    use mpfree_context::chord::Quality;

    /// A one-bar bass phrase over four bars of chords plays from each
    /// chord's root, in the bass's register, looped.
    #[test]
    fn a_bass_phrase_follows_each_root() {
        let r = row(&[
            (0, Quality::Minor),
            (5, Quality::Minor),
            (7, Quality::Minor),
            (0, Quality::Minor),
        ]);
        let settings = Settings::default();
        let p = Phrase::parse("bass minor : 1 . . 1 . . 5 . 1 . . . b7- . 5- .").unwrap();
        let added = phrase_part(&ask(&r, &settings, PartRole::Bass), &p).unwrap();
        // Each bar's downbeat is its chord's root.
        let downbeats: Vec<u8> = (0..4)
            .map(|b| {
                added
                    .events
                    .iter()
                    .find(|e| e.at == b * BAR)
                    .map(|e| e.pitch % 12)
                    .unwrap()
            })
            .collect();
        assert_eq!(downbeats, vec![0, 5, 7, 0]);
        assert!(added.events.iter().all(|e| (31..=55).contains(&e.pitch)));
        assert_eq!(added.events.len(), 4 * p.len());
    }

    /// Chord stabs written as 1, b3, 5 in minor become each chord of the key.
    #[test]
    fn chord_stabs_become_each_chord_of_the_key() {
        let r = row(&[(0, Quality::Minor), (8, Quality::Major)]);
        let settings = Settings::default();
        let p = Phrase::parse("chords minor 1/8 : 1,b3,5 . . 1,b3,5 . . . .").unwrap();
        let added = phrase_part(&ask(&r, &settings, PartRole::Chords), &p).unwrap();
        let at = |t: Tick| -> Vec<u8> {
            let mut v: Vec<u8> = added
                .events
                .iter()
                .filter(|e| e.at == t)
                .map(|e| e.pitch % 12)
                .collect();
            v.sort_unstable();
            v
        };
        // C minor, then A flat major (Ab C Eb).
        assert_eq!(at(0), vec![0, 3, 7]);
        assert_eq!(at(BAR), vec![0, 3, 8]);
    }

    /// Drums and a row with no chords are refused.
    #[test]
    fn drums_and_no_chords_are_refused() {
        let r = row(&[(0, Quality::Minor)]);
        let settings = Settings::default();
        let p = Phrase::default();
        assert_eq!(
            phrase_part(&ask(&r, &settings, PartRole::Drums), &p).unwrap_err(),
            Refused::PhraseNotDrums
        );
        let empty = row(&[]);
        assert_eq!(
            phrase_part(&ask(&empty, &settings, PartRole::Bass), &p).unwrap_err(),
            Refused::NoChords
        );
    }
}

#[cfg(test)]
mod measure {
    use super::tests_support::*;
    use mpfree_arp::{Library, PhraseKind};
    use mpfree_context::chord::Quality;

    /// MPFree's 64 phrases as parts over four bars of C minor's i, iv,
    /// VI and v: how many of Add a part's checks each passes, by kind.
    /// `cargo test --release -p mpfree-compose --lib
    /// print_the_64_as_parts -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_the_64_as_parts() {
        let r = row(&[
            (0, Quality::Minor),
            (5, Quality::Minor),
            (8, Quality::Major),
            (7, Quality::Minor),
        ]);
        let settings = crate::settings::Settings::default();
        let lib = Library::built_in();
        for kind in [
            PhraseKind::Bass,
            PhraseKind::Chords,
            PhraseKind::Riff,
            PhraseKind::Line,
        ] {
            let part = match kind {
                PhraseKind::Bass => PartRole::Bass,
                PhraseKind::Chords => PartRole::Chords,
                _ => PartRole::Melody,
            };
            let mut counts = [0usize; 7];
            let mut missed = std::collections::BTreeMap::new();
            for (_, p) in lib.phrases().filter(|(_, p)| p.kind == kind) {
                let added = phrase_part(&ask(&r, &settings, part), p).unwrap();
                counts[added.verdict.passed()] += 1;
                for (name, c) in added.verdict.checks() {
                    if !c.pass {
                        *missed.entry(name).or_insert(0usize) += 1;
                    }
                }
            }
            println!("{kind:?}: checks passed (0..6) {counts:?}; missed {missed:?}");
        }
    }
}
