//! One harmony question for every place (contract 4 of the design):
//! "what is the harmony at this point?", answered the same way for the
//! live tools, the clip grid and the song.
//!
//! The answer comes from the first source that applies:
//!
//! 1. the chord the musician holds (live tools, while held);
//! 2. in song mode, the chords of the song section a block plays in;
//! 3. on the clip grid, the row's Harmony line;
//! 4. suggested chords, for a row with nothing to hear;
//! 5. otherwise no clear chord: nothing is invented.
//!
//! A row's chords run for the row's length (its set length, else its
//! longest clip, as `mpfree_seq::scene_length` works), and a shorter clip
//! loops against them: [`row_tick`] says where in the row a clip's note
//! falls on a given pass.

use mpfree_transport::time::Tick;

use crate::chord::{ChordAnswer, ChordTimeline};

/// Where an answer came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HarmonySource {
    Held,
    SongSection,
    RowLine,
    Suggested,
    None,
}

/// The answer to "what is the harmony here?", with where it came from and
/// the stretch of time it holds for (in the asker's ticks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HarmonyAt {
    pub answer: ChordAnswer,
    pub source: HarmonySource,
    pub from: Tick,
    pub until: Tick,
}

impl HarmonyAt {
    pub fn none(t: Tick) -> HarmonyAt {
        HarmonyAt {
            answer: ChordAnswer::NoClearChord,
            source: HarmonySource::None,
            from: t,
            until: Tick::MAX,
        }
    }
}

/// Anything that can answer the harmony question at a tick.
pub trait Harmony {
    fn at(&self, t: Tick) -> HarmonyAt;
}

impl Harmony for ChordTimeline {
    /// A timeline alone, read as the asker's own ticks (a clip's heard
    /// chords).
    fn at(&self, t: Tick) -> HarmonyAt {
        match self.span_at(t) {
            Some(s) => HarmonyAt {
                answer: s.answer,
                source: HarmonySource::RowLine,
                from: s.start,
                until: s.end(),
            },
            None => HarmonyAt::none(t),
        }
    }
}

/// A row's chords: its Harmony line over the row's length.
#[derive(Clone, Debug, PartialEq)]
pub struct RowHarmony {
    pub line: ChordTimeline,
    /// The row's length in song ticks (`scene_length`). The line repeats
    /// over it; a line shorter than the row loops.
    pub length: Tick,
    /// The line was suggested for a row with nothing to hear.
    pub suggested: bool,
}

/// A song section: its chords from a song tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub start: Tick,
    pub line: ChordTimeline,
}

/// Where the asker is playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playing {
    /// Clips launched from the clip grid: ticks are row ticks.
    Grid,
    /// Song mode: ticks are song ticks.
    Song,
}

/// The sources one part can ask, in priority order.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolver {
    pub playing: Playing,
    /// The chord held now (live tools), if any.
    pub held: Option<ChordAnswer>,
    pub row: Option<RowHarmony>,
    /// Song sections in song order.
    pub sections: Vec<Section>,
}

impl Resolver {
    pub fn grid(row: RowHarmony) -> Resolver {
        Resolver {
            playing: Playing::Grid,
            held: None,
            row: Some(row),
            sections: Vec::new(),
        }
    }

    pub fn song(sections: Vec<Section>) -> Resolver {
        Resolver {
            playing: Playing::Song,
            held: None,
            row: None,
            sections,
        }
    }

    pub fn nothing() -> Resolver {
        Resolver {
            playing: Playing::Grid,
            held: None,
            row: None,
            sections: Vec::new(),
        }
    }

    fn section_at(&self, t: Tick) -> Option<HarmonyAt> {
        let s = self
            .sections
            .iter()
            .rev()
            .find(|s| s.start <= t && t < s.start + s.line.len())?;
        let span = s.line.span_at(t - s.start)?;
        Some(HarmonyAt {
            answer: span.answer,
            source: HarmonySource::SongSection,
            from: s.start + span.start,
            until: s.start + span.end(),
        })
    }

    fn row_at(&self, t: Tick) -> Option<HarmonyAt> {
        let r = self.row.as_ref()?;
        let len = r.line.len();
        if len <= 0 || r.length <= 0 {
            return None;
        }
        let in_row = t.rem_euclid(r.length);
        let in_line = in_row.rem_euclid(len);
        let span = r.line.span_at(in_line)?;
        let base = t - in_line;
        Some(HarmonyAt {
            answer: span.answer,
            source: if r.suggested {
                HarmonySource::Suggested
            } else {
                HarmonySource::RowLine
            },
            from: base + span.start,
            until: base + span.end(),
        })
    }
}

impl Harmony for Resolver {
    fn at(&self, t: Tick) -> HarmonyAt {
        if let Some(held) = self.held {
            return HarmonyAt {
                answer: held,
                source: HarmonySource::Held,
                from: t,
                until: Tick::MAX,
            };
        }
        let found = match self.playing {
            Playing::Song => self.section_at(t),
            Playing::Grid => self.row_at(t),
        };
        found.unwrap_or_else(|| HarmonyAt::none(t))
    }
}

/// Where in its row a clip's note falls: a clip of `clip_len` ticks on
/// pass `pass` (0 first), at its own tick `local`, in a row of `row_len`.
/// A two-bar clip in an eight-bar row meets four different stretches of
/// the row's chords.
pub fn row_tick(pass: u32, clip_len: Tick, local: Tick, row_len: Tick) -> Tick {
    (Tick::from(pass) * clip_len + local).rem_euclid(row_len.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::{Chord, Heard, Quality};
    use mpfree_transport::time::PPQ;

    const BAR: Tick = 4 * PPQ as Tick;

    fn ch(root: u8, q: Quality) -> Chord {
        Chord::new(root, q)
    }

    fn eight_bar_row() -> RowHarmony {
        RowHarmony {
            line: ChordTimeline::of(
                &[
                    (ch(0, Quality::Minor7), 2 * BAR),
                    (ch(5, Quality::Minor7), 2 * BAR),
                    (ch(3, Quality::Major7), 2 * BAR),
                    (ch(7, Quality::Minor7), 2 * BAR),
                ],
                Heard::Heard,
            ),
            length: 8 * BAR,
            suggested: false,
        }
    }

    #[test]
    fn a_held_chord_comes_first() {
        let mut r = Resolver::grid(eight_bar_row());
        r.held = Some(ChordAnswer::Chord(ch(8, Quality::Major)));
        let a = r.at(3 * BAR);
        assert_eq!(a.source, HarmonySource::Held);
        assert_eq!(a.answer.name(), "Ab");
        r.held = None;
        assert_eq!(r.at(3 * BAR).answer.name(), "Fm7");
        assert_eq!(r.at(3 * BAR).source, HarmonySource::RowLine);
    }

    #[test]
    fn a_shorter_clip_loops_against_the_rows_chords() {
        let r = Resolver::grid(eight_bar_row());
        let clip_len = 2 * BAR;
        let heard: Vec<String> = (0..4)
            .map(|pass| r.at(row_tick(pass, clip_len, 0, 8 * BAR)).answer.name())
            .collect();
        assert_eq!(heard, vec!["Cm7", "Fm7", "Ebmaj7", "Gm7"]);
        // The fifth pass is the row's start again.
        assert_eq!(
            r.at(row_tick(4, clip_len, BAR, 8 * BAR)).answer.name(),
            "Cm7"
        );
    }

    #[test]
    fn a_line_shorter_than_its_row_repeats() {
        let row = RowHarmony {
            line: ChordTimeline::of(
                &[(ch(0, Quality::Minor), BAR), (ch(10, Quality::Major), BAR)],
                Heard::Heard,
            ),
            length: 8 * BAR,
            suggested: false,
        };
        let r = Resolver::grid(row);
        assert_eq!(r.at(5 * BAR + 10).answer.name(), "Bb");
        let a = r.at(6 * BAR + 10);
        assert_eq!((a.from, a.until), (6 * BAR, 7 * BAR));
    }

    #[test]
    fn song_sections_answer_in_song_mode() {
        let verse = ChordTimeline::of(&[(ch(0, Quality::Minor), 4 * BAR)], Heard::Heard);
        let chorus = ChordTimeline::of(
            &[
                (ch(8, Quality::Major), 2 * BAR),
                (ch(10, Quality::Major), 2 * BAR),
            ],
            Heard::Set,
        );
        let r = Resolver::song(vec![
            Section {
                start: 0,
                line: verse,
            },
            Section {
                start: 4 * BAR,
                line: chorus,
            },
        ]);
        assert_eq!(r.at(BAR).answer.name(), "Cm");
        let a = r.at(6 * BAR + 1);
        assert_eq!(a.answer.name(), "Bb");
        assert_eq!(a.source, HarmonySource::SongSection);
        assert_eq!((a.from, a.until), (6 * BAR, 8 * BAR));
        // Past the last section: nothing is invented.
        assert_eq!(r.at(9 * BAR).source, HarmonySource::None);
    }

    #[test]
    fn suggested_chords_say_so_and_nothing_means_no_clear_chord() {
        let mut row = eight_bar_row();
        row.suggested = true;
        assert_eq!(Resolver::grid(row).at(0).source, HarmonySource::Suggested);
        let none = Resolver::nothing().at(123);
        assert_eq!(none.answer, ChordAnswer::NoClearChord);
        assert_eq!(none.source, HarmonySource::None);
    }
}
