//! Fit to the new chords (S2.5, contract 2 of the design).
//!
//! A kept clip is never rewritten on its own. When the row's chords change,
//! the clips whose notes fitted the old chords and don't fit the new ones
//! get a mark ([`marks`]), and "Fit to the new chords" moves them
//! ([`fit_clip`]); the app writes every fitted clip of the row as one edit
//! (`mpfree_play::Takes::replace_clips`), which one UNDO takes back.
//!
//! Fit moves only the notes under chords that changed. They are read back
//! into decisions under the old chords ([`decisions_of`]: each note's role
//! over its chord and its motion) and played again under the new ones
//! ([`realize`]), so rhythm, loudness, timing and the line's shape stay,
//! and a clip you played fits as well as one mpfree made. Every other note
//! stays exactly as it was. Drums never change: on a kit, notes are sounds.
//!
//! A clip shorter than its row plays under the row's chords on every pass
//! (contract 4). One clip can only fit them all when every pass meets the
//! same new chord where a chord changed; otherwise Fit says so, and the
//! clip made as long as the row can be fitted instead.

use mpfree_context::harmony::{row_tick, Harmony, HarmonyAt, Resolver, RowHarmony};
use mpfree_context::key::Key;
use mpfree_context::PartRole;
use mpfree_seq::{Clip, Holds, Speed};
use mpfree_transport::time::Tick;

use crate::event::{Part, PartId, Source};
use crate::profile::InstrumentKind;
use crate::realize::{decisions_of, realize, Fit, PlainNote};
use crate::refit::misfits;

/// Why a clip can't be fitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitRefused {
    /// Drums, or a clip of pad hits: its notes are sounds, not pitches.
    NotPitched,
    /// Shorter than its row, it meets different new chords on different
    /// passes.
    PassesDiffer,
}

impl FitRefused {
    pub const fn words(self) -> &'static str {
        match self {
            FitRefused::NotPitched => "Drums and pad hits don't move with the chords",
            FitRefused::PassesDiffer => {
                "This clip plays under different chords each time round: make it as long as the row, then fit it"
            }
        }
    }
}

/// The row's chords, asked in a clip's own ticks on one of its passes.
struct OnPass<'a> {
    row: &'a Resolver,
    speed: Speed,
    /// One pass, in song ticks.
    pass_len: Tick,
    row_len: Tick,
    pass: u32,
}

impl Harmony for OnPass<'_> {
    fn at(&self, t: Tick) -> HarmonyAt {
        let song = self.speed.to_song(t as f64).round() as Tick;
        self.row
            .at(row_tick(self.pass, self.pass_len, song, self.row_len))
    }
}

/// How a clip meets its row: its pass in song ticks and how many passes
/// the row holds.
fn passes(clip: &Clip, row: &RowHarmony) -> (Tick, u32) {
    let pass_len = (clip.speed.to_song(clip.length as f64).round() as Tick).max(1);
    let row_len = row.length.max(pass_len);
    (pass_len, ((row_len + pass_len - 1) / pass_len) as u32)
}

fn pitched(clip: &Clip, role: PartRole) -> bool {
    role != PartRole::Drums && clip.holds == Holds::Notes
}

/// The clip's sounding notes, each with its index in `clip.notes`, in time
/// order.
fn plain_notes(clip: &Clip) -> (Vec<usize>, Vec<PlainNote>) {
    let mut v: Vec<(usize, PlainNote)> = clip
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.on)
        .map(|(i, n)| {
            (
                i,
                PlainNote {
                    at: n.tick,
                    len: n.len,
                    pitch: n.note,
                    velocity: n.velocity,
                },
            )
        })
        .collect();
    v.sort_by_key(|(_, n)| (n.at, n.pitch));
    v.into_iter().unzip()
}

/// The notes of `clip` (indices into `clip.notes`) that fitted the row's
/// `old` chords and don't fit its `new` ones on some pass: chord tones on
/// a beat that no longer are. Not empty: the clip gets a mark and is
/// offered Fit. Drums and pad hits: none.
pub fn marks(clip: &Clip, role: PartRole, old: &RowHarmony, new: &RowHarmony) -> Vec<usize> {
    if !pitched(clip, role) {
        return Vec::new();
    }
    let (index, notes) = plain_notes(clip);
    let (pass_len, n) = passes(clip, old);
    let (old_r, new_r) = (Resolver::grid(old.clone()), Resolver::grid(new.clone()));
    let mut hit: Vec<usize> = (0..n)
        .flat_map(|pass| {
            let on = |row| OnPass {
                row,
                speed: clip.speed,
                pass_len,
                row_len: old.length.max(pass_len),
                pass,
            };
            misfits(&notes, &on(&old_r), &on(&new_r), 0)
        })
        .map(|i| index[i])
        .collect();
    hit.sort_unstable();
    hit.dedup();
    hit
}

/// `clip` fitted to the row's `new` chords: the notes under chords that
/// changed from `old` moved, every other note as it was. `kind` is what
/// plays it; `key` and `played_pcs` are the row's reading (what notes keep
/// to with no clear chord). The clip keeps its id, length and settings.
pub fn fit_clip(
    clip: &Clip,
    role: PartRole,
    kind: InstrumentKind,
    old: &RowHarmony,
    new: &RowHarmony,
    key: Option<Key>,
    played_pcs: u16,
) -> Result<Clip, FitRefused> {
    if !pitched(clip, role) {
        return Err(FitRefused::NotPitched);
    }
    let (index, notes) = plain_notes(clip);
    let (pass_len, n) = passes(clip, old);
    let row_len = old.length.max(pass_len);
    let (old_r, new_r) = (Resolver::grid(old.clone()), Resolver::grid(new.clone()));
    let on = |row, pass| OnPass {
        row,
        speed: clip.speed,
        pass_len,
        row_len,
        pass,
    };
    // Which notes move: under a chord that changed, the same change on
    // every pass.
    let mut moves = vec![false; notes.len()];
    for (k, note) in notes.iter().enumerate() {
        let pairs: Vec<_> = (0..n)
            .map(|p| {
                (
                    on(&old_r, p).at(note.at).answer,
                    on(&new_r, p).at(note.at).answer,
                )
            })
            .collect();
        if pairs.iter().all(|(o, w)| o == w) {
            continue;
        }
        if pairs.iter().any(|pr| *pr != pairs[0]) {
            return Err(FitRefused::PassesDiffer);
        }
        moves[k] = true;
    }
    if !moves.contains(&true) {
        return Ok(clip.clone());
    }
    let fit = |h| Fit {
        harmony: h,
        key,
        played_pcs,
        origin: 0,
    };
    let (old_h, new_h) = (on(&old_r, 0), on(&new_r, 0));
    let (register, decisions) = decisions_of(&notes, &fit(&old_h), Source::You);
    let part = Part {
        id: PartId(0),
        role,
        kind,
        register,
        decisions,
    };
    let moved = realize(&part, &fit(&new_h));
    let mut out = clip.clone();
    for (k, e) in moved.iter().enumerate() {
        if moves.get(k) == Some(&true) {
            out.notes[index[k]].note = e.pitch;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_context::chord::{Chord, ChordTimeline, Heard, Quality};
    use mpfree_context::key::Mode;
    use mpfree_seq::Note;
    use mpfree_transport::time::PPQ;

    const BEAT: Tick = PPQ as Tick;
    const BAR: Tick = 4 * BEAT;

    fn row(chords: &[Chord], length: Tick) -> RowHarmony {
        let line = ChordTimeline::of(
            &chords.iter().map(|&c| (c, BAR)).collect::<Vec<_>>(),
            Heard::Heard,
        );
        RowHarmony {
            line,
            length,
            suggested: false,
        }
    }

    fn four() -> [Chord; 4] {
        [
            Chord::new(0, Quality::Minor7),
            Chord::new(5, Quality::Minor7),
            Chord::new(3, Quality::Major7),
            Chord::new(7, Quality::Minor7),
        ]
    }

    /// A bass someone played over Cm7 Fm7 Ebmaj7 Gm7: root, fifth, a
    /// passing note and an approach in each bar.
    fn played_bass() -> Clip {
        let mut c = Clip::new("bass", 4 * BAR);
        c.holds = Holds::Notes;
        let bars: [[u8; 4]; 4] = [
            [36, 43, 41, 40],
            [41, 48, 46, 38],
            [39, 46, 44, 42],
            [43, 50, 48, 37],
        ];
        c.notes = bars
            .iter()
            .enumerate()
            .flat_map(|(b, ns)| {
                ns.iter().enumerate().map(move |(i, &n)| {
                    Note::new(
                        b as Tick * BAR + i as Tick * BEAT,
                        BEAT - 20,
                        n,
                        90 + i as u8,
                    )
                })
            })
            .collect();
        c
    }

    fn c_minor() -> Option<Key> {
        Some(Key::new(0, Mode::Minor))
    }

    /// Bar 3's Ebmaj7 becomes Abmaj7: the bass is marked in bar 3 only,
    /// and Fit moves only bar 3's notes, onto Abmaj7, with the same
    /// rhythm, lengths and loudness.
    #[test]
    fn fit_moves_only_the_notes_under_the_changed_chord() {
        let old = row(&four(), 4 * BAR);
        let mut chords = four();
        chords[2] = Chord::new(8, Quality::Major7);
        let new = row(&chords, 4 * BAR);
        let clip = played_bass();
        let marked = marks(&clip, PartRole::Bass, &old, &new);
        assert!(!marked.is_empty());
        assert!(marked
            .iter()
            .all(|&i| (2 * BAR..3 * BAR).contains(&clip.notes[i].tick)));
        let fitted = fit_clip(
            &clip,
            PartRole::Bass,
            InstrumentKind::Bass,
            &old,
            &new,
            c_minor(),
            0,
        )
        .unwrap();
        assert_eq!(fitted.notes.len(), clip.notes.len());
        for (a, b) in clip.notes.iter().zip(&fitted.notes) {
            assert_eq!((a.tick, a.len, a.velocity), (b.tick, b.len, b.velocity));
            if !(2 * BAR..3 * BAR).contains(&a.tick) {
                assert_eq!(a.note, b.note, "outside the change nothing moves");
            }
        }
        let bar3: Vec<u8> = fitted.notes[8..12].iter().map(|n| n.note).collect();
        let ab = chords[2];
        assert!(
            ab.has(i32::from(bar3[0])) && ab.has(i32::from(bar3[1])),
            "{bar3:?}"
        );
        assert!(marks(&fitted, PartRole::Bass, &new, &new).is_empty());
        // Nothing changed: nothing to mark, the clip as it was.
        assert!(marks(&clip, PartRole::Bass, &old, &old).is_empty());
        assert_eq!(
            fit_clip(
                &clip,
                PartRole::Bass,
                InstrumentKind::Bass,
                &old,
                &old,
                c_minor(),
                0
            ),
            Ok(clip.clone())
        );
    }

    /// A one-bar riff in a two-bar row loops under both chords; changing
    /// one of them leaves passes that disagree, which one clip can't fit.
    /// Drums never move.
    #[test]
    fn a_short_clip_under_different_chords_and_drums_are_left() {
        let mut riff = Clip::new("riff", BAR);
        riff.holds = Holds::Notes;
        riff.notes = vec![
            Note::new(0, BEAT, 60, 100),
            Note::new(2 * BEAT, BEAT, 63, 100),
        ];
        let cm = Chord::new(0, Quality::Minor7);
        let old = row(&[cm, cm], 2 * BAR);
        let bb7 = Chord::new(10, Quality::Dominant7);
        let new = row(&[cm, bb7], 2 * BAR);
        assert!(
            !marks(&riff, PartRole::Melody, &old, &new).is_empty(),
            "marked on pass 2"
        );
        assert_eq!(
            fit_clip(
                &riff,
                PartRole::Melody,
                InstrumentKind::OneNote,
                &old,
                &new,
                c_minor(),
                0
            ),
            Err(FitRefused::PassesDiffer)
        );
        // The same change on every pass: fitted.
        let both = row(&[bb7], 2 * BAR);
        let fitted = fit_clip(
            &riff,
            PartRole::Melody,
            InstrumentKind::OneNote,
            &old,
            &both,
            c_minor(),
            0,
        )
        .unwrap();
        assert!(
            fitted.notes.iter().all(|n| bb7.has(i32::from(n.note))),
            "{:?}",
            fitted.notes
        );
        let mut beat = riff.clone();
        beat.holds = Holds::Hits;
        assert!(marks(&beat, PartRole::Melody, &old, &new).is_empty());
        assert_eq!(
            fit_clip(
                &riff,
                PartRole::Drums,
                InstrumentKind::Percussive,
                &old,
                &both,
                None,
                0
            ),
            Err(FitRefused::NotPitched)
        );
    }
}
