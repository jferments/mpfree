//! Chords and the chord timeline.
//!
//! A timeline is a list of spans, each with its own start and length, so
//! a chord can last a beat or the whole clip (drones, slow ambient
//! harmony). A span can also say "no clear chord": then nothing is
//! invented over it (contract 5 of the design).

use mpfree_transport::time::Tick;

use crate::key::pc_name;

/// The chord shapes a reading weighs: 12 qualities on 12 roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    Major,
    Minor,
    Diminished,
    Sus4,
    Dominant7,
    Major7,
    Minor7,
    HalfDiminished7,
    Diminished7,
    Minor6,
    Major6,
    Power,
}

impl Quality {
    pub const ALL: [Quality; 12] = [
        Quality::Major,
        Quality::Minor,
        Quality::Diminished,
        Quality::Sus4,
        Quality::Dominant7,
        Quality::Major7,
        Quality::Minor7,
        Quality::HalfDiminished7,
        Quality::Diminished7,
        Quality::Minor6,
        Quality::Major6,
        Quality::Power,
    ];

    /// Semitones above the root, root first, then third (or the sus 4th),
    /// fifth and any seventh or sixth.
    pub const fn intervals(self) -> &'static [u8] {
        match self {
            Quality::Major => &[0, 4, 7],
            Quality::Minor => &[0, 3, 7],
            Quality::Diminished => &[0, 3, 6],
            Quality::Sus4 => &[0, 5, 7],
            Quality::Dominant7 => &[0, 4, 7, 10],
            Quality::Major7 => &[0, 4, 7, 11],
            Quality::Minor7 => &[0, 3, 7, 10],
            Quality::HalfDiminished7 => &[0, 3, 6, 10],
            Quality::Diminished7 => &[0, 3, 6, 9],
            Quality::Minor6 => &[0, 3, 7, 9],
            Quality::Major6 => &[0, 4, 7, 9],
            Quality::Power => &[0, 7],
        }
    }

    /// The word a saved project writes for it: its suffix, with "maj"
    /// for a major triad (whose suffix is empty).
    pub const fn token(self) -> &'static str {
        match self {
            Quality::Major => "maj",
            q => q.suffix(),
        }
    }

    /// The quality a saved word names.
    pub fn from_token(word: &str) -> Option<Quality> {
        Quality::ALL.into_iter().find(|q| q.token() == word)
    }

    pub const fn suffix(self) -> &'static str {
        match self {
            Quality::Major => "",
            Quality::Minor => "m",
            Quality::Diminished => "dim",
            Quality::Sus4 => "sus4",
            Quality::Dominant7 => "7",
            Quality::Major7 => "maj7",
            Quality::Minor7 => "m7",
            Quality::HalfDiminished7 => "m7b5",
            Quality::Diminished7 => "dim7",
            Quality::Minor6 => "m6",
            Quality::Major6 => "6",
            Quality::Power => "5",
        }
    }
}

/// Which note of the chord a part asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Root,
    Third,
    Fifth,
    /// The seventh or sixth; on a triad, the root an octave up.
    Seventh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// Pitch class of the root, 0 = C.
    pub root: u8,
    pub quality: Quality,
}

impl Chord {
    pub const fn new(root: u8, quality: Quality) -> Chord {
        Chord {
            root: root % 12,
            quality,
        }
    }

    /// "Cm7", "Bbmaj7", "F#m7b5".
    pub fn name(self) -> String {
        format!("{}{}", pc_name(i32::from(self.root)), self.quality.suffix())
    }

    /// The chord's pitch classes as a mask over C.
    pub fn pcs(self) -> u16 {
        self.quality
            .intervals()
            .iter()
            .fold(0u16, |m, &i| m | 1 << ((self.root + i) % 12))
    }

    /// Whether a pitch (any octave) is a chord tone.
    pub fn has(self, pitch: i32) -> bool {
        self.pcs() & (1 << pitch.rem_euclid(12)) != 0
    }

    /// The pitch class of a chord tone. A power chord has no third (its
    /// fifth stands in) and a triad no seventh (its root stands in).
    pub fn tone(self, t: Tone) -> u8 {
        let iv = self.quality.intervals();
        let i = match t {
            Tone::Root => 0,
            Tone::Third if iv.len() >= 3 => iv[1],
            Tone::Third => iv[iv.len() - 1],
            Tone::Fifth => iv[if iv.len() >= 3 { 2 } else { iv.len() - 1 }],
            Tone::Seventh if iv.len() >= 4 => iv[3],
            Tone::Seventh => 0,
        };
        (self.root + i) % 12
    }

    /// Which chord tone a pitch is, if any.
    pub fn tone_of(self, pitch: i32) -> Option<Tone> {
        let iv = ((pitch - i32::from(self.root)).rem_euclid(12)) as u8;
        let ivs = self.quality.intervals();
        let at = ivs.iter().position(|&i| i == iv)?;
        Some(match (at, ivs.len()) {
            (0, _) => Tone::Root,
            (1, n) if n >= 3 => Tone::Third,
            (1, _) => Tone::Fifth,
            (2, _) => Tone::Fifth,
            _ => Tone::Seventh,
        })
    }
}

/// What the listener heard over a span: a chord, or no clear chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChordAnswer {
    Chord(Chord),
    NoClearChord,
}

impl ChordAnswer {
    pub fn chord(self) -> Option<Chord> {
        match self {
            ChordAnswer::Chord(c) => Some(c),
            ChordAnswer::NoClearChord => None,
        }
    }

    pub fn name(self) -> String {
        match self {
            ChordAnswer::Chord(c) => c.name(),
            ChordAnswer::NoClearChord => "no clear chord".to_string(),
        }
    }
}

/// Where a span's chord came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Heard {
    /// Read from notes someone played or wrote.
    Heard,
    /// Made up by mpfree for a row with nothing to hear: shown dashed
    /// until the musician keeps or changes it.
    Suggested,
    /// Set by the musician.
    Set,
}

impl Heard {
    /// The word a saved project writes for it.
    pub const fn token(self) -> &'static str {
        match self {
            Heard::Heard => "heard",
            Heard::Suggested => "suggested",
            Heard::Set => "set",
        }
    }

    pub fn from_token(word: &str) -> Option<Heard> {
        [Heard::Heard, Heard::Suggested, Heard::Set]
            .into_iter()
            .find(|h| h.token() == word)
    }
}

/// One chord on the timeline: its own start and length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChordSpan {
    pub start: Tick,
    pub len: Tick,
    pub answer: ChordAnswer,
    pub heard: Heard,
    /// How sure the reading is (0 to 1), for ranking only.
    pub sureness: f32,
    pub runner_up: Option<Chord>,
}

impl ChordSpan {
    pub fn new(start: Tick, len: Tick, answer: ChordAnswer, heard: Heard) -> ChordSpan {
        ChordSpan {
            start,
            len,
            answer,
            heard,
            sureness: 1.0,
            runner_up: None,
        }
    }

    pub fn end(&self) -> Tick {
        self.start + self.len
    }

    pub fn contains(&self, t: Tick) -> bool {
        (self.start..self.end()).contains(&t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineError {
    /// Spans must start where the one before ends, from tick 0.
    Gap,
    /// A span of no length.
    Empty,
}

/// Chords in time over a length, from tick 0, with no gaps. A span may
/// cover the whole length.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ChordTimeline {
    spans: Vec<ChordSpan>,
}

impl ChordTimeline {
    pub fn new(spans: Vec<ChordSpan>) -> Result<ChordTimeline, TimelineError> {
        let mut at = 0;
        for s in &spans {
            if s.len <= 0 {
                return Err(TimelineError::Empty);
            }
            if s.start != at {
                return Err(TimelineError::Gap);
            }
            at = s.end();
        }
        Ok(ChordTimeline { spans })
    }

    /// One answer over the whole length: a drone, or no clear chord.
    pub fn whole(len: Tick, answer: ChordAnswer, heard: Heard) -> ChordTimeline {
        ChordTimeline {
            spans: vec![ChordSpan::new(0, len.max(1), answer, heard)],
        }
    }

    /// Chords set one after another, each `len` ticks: a convenience for
    /// tests and for a Harmony line typed in.
    pub fn of(chords: &[(Chord, Tick)], heard: Heard) -> ChordTimeline {
        let mut at = 0;
        let spans = chords
            .iter()
            .map(|&(c, len)| {
                let s = ChordSpan::new(at, len, ChordAnswer::Chord(c), heard);
                at += len;
                s
            })
            .collect();
        ChordTimeline { spans }
    }

    pub fn spans(&self) -> &[ChordSpan] {
        &self.spans
    }

    pub fn len(&self) -> Tick {
        self.spans.last().map_or(0, |s| s.end())
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// The span at a tick of the timeline, if inside it.
    pub fn span_at(&self, t: Tick) -> Option<&ChordSpan> {
        let i = self.spans.partition_point(|s| s.end() <= t);
        self.spans.get(i).filter(|s| s.contains(t))
    }

    /// Replace the answer of the span at `t` (a chord the musician set).
    pub fn set_at(&mut self, t: Tick, answer: ChordAnswer) -> bool {
        let i = self.spans.partition_point(|s| s.end() <= t);
        match self.spans.get_mut(i) {
            Some(s) if s.contains(t) => {
                s.answer = answer;
                s.heard = Heard::Set;
                true
            }
            _ => false,
        }
    }

    /// Only what the line says (each span's chord, length and where it
    /// came from), without a reading's sureness and runner-up: the line
    /// as a row keeps it.
    pub fn plain(&self) -> ChordTimeline {
        ChordTimeline {
            spans: self
                .spans
                .iter()
                .map(|s| ChordSpan::new(s.start, s.len, s.answer, s.heard))
                .collect(),
        }
    }

    /// Cut to `to` ticks (the spans past it go, the last one is cut).
    pub fn cut(&self, to: Tick) -> ChordTimeline {
        ChordTimeline {
            spans: self
                .spans
                .iter()
                .filter(|s| s.start < to)
                .map(|s| ChordSpan {
                    len: s.len.min(to - s.start),
                    ..*s
                })
                .collect(),
        }
    }

    /// Heard again (contract 4's Harmony line): `heard` is a new reading
    /// of the row. Chords the musician set stay where they are; elsewhere
    /// a chord heard now wins; where nothing clear is heard a suggested
    /// chord stays, and a chord heard before goes (its notes are gone).
    /// The line is as long as the new reading, or as before when nothing
    /// was read; a line set shorter than the reading isn't stretched:
    /// past its end the reading speaks.
    pub fn rehear(&self, heard: &ChordTimeline) -> ChordTimeline {
        let len = if heard.is_empty() {
            self.len()
        } else {
            heard.len()
        };
        let mut cuts: Vec<Tick> = self
            .spans
            .iter()
            .chain(&heard.spans)
            .flat_map(|s| [s.start, s.end()])
            .filter(|&t| t < len)
            .chain([0, len])
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut spans: Vec<ChordSpan> = Vec::new();
        // Which span each piece came from (line 0 the old, 1 the heard,
        // 2 none, and its start), so only pieces of one span join again.
        let mut last_from: Option<(u8, Tick)> = None;
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let old = self.span_at(a);
            let new = heard.span_at(a);
            let (pick, from) = match (old, new) {
                (Some(o), _) if o.heard == Heard::Set => (*o, (0, o.start)),
                (_, Some(n)) if n.answer.chord().is_some() => (
                    ChordSpan {
                        heard: Heard::Heard,
                        ..*n
                    },
                    (1, n.start),
                ),
                (Some(o), _) if o.heard == Heard::Suggested => (*o, (0, o.start)),
                _ => (
                    ChordSpan::new(a, b - a, ChordAnswer::NoClearChord, Heard::Heard),
                    (2, 0),
                ),
            };
            let piece = ChordSpan {
                start: a,
                len: b - a,
                ..pick
            };
            match spans.last_mut() {
                Some(l) if last_from == Some(from) && l.end() == a => l.len += piece.len,
                _ => spans.push(piece),
            }
            last_from = Some(from);
        }
        ChordTimeline { spans }.plain()
    }

    /// Names, one per span, "Cm7 Fm7 Ebmaj7 Gm7".
    pub fn names(&self) -> String {
        self.spans
            .iter()
            .map(|s| s.answer.name())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_transport::time::PPQ;

    const BAR: Tick = 4 * PPQ as Tick;

    #[test]
    fn chord_tones_and_names() {
        let c = Chord::new(0, Quality::Minor7);
        assert_eq!(c.name(), "Cm7");
        assert_eq!(c.tone(Tone::Third), 3);
        assert_eq!(c.tone(Tone::Seventh), 10);
        assert_eq!(c.tone_of(67), Some(Tone::Fifth));
        assert_eq!(c.tone_of(62), None);
        assert_eq!(Chord::new(10, Quality::Major7).name(), "Bbmaj7");
        assert_eq!(Chord::new(6, Quality::HalfDiminished7).name(), "F#m7b5");
        // A power chord's third is its fifth; a triad's seventh its root.
        assert_eq!(Chord::new(0, Quality::Power).tone(Tone::Third), 7);
        assert_eq!(Chord::new(0, Quality::Major).tone(Tone::Seventh), 0);
        assert_eq!(Chord::new(0, Quality::Power).tone_of(55), Some(Tone::Fifth));
    }

    #[test]
    fn a_chord_can_last_the_whole_clip() {
        let len = 64 * BAR;
        let drone = ChordTimeline::whole(
            len,
            ChordAnswer::Chord(Chord::new(0, Quality::Power)),
            Heard::Heard,
        );
        assert_eq!(drone.len(), len);
        assert_eq!(drone.span_at(len - 1).unwrap().answer.name(), "C5");
        assert!(drone.span_at(len).is_none());
    }

    #[test]
    fn no_clear_chord_is_an_answer() {
        let t = ChordTimeline::whole(BAR, ChordAnswer::NoClearChord, Heard::Heard);
        assert_eq!(t.span_at(0).unwrap().answer.chord(), None);
        assert_eq!(t.names(), "no clear chord");
    }

    fn cm7() -> Chord {
        Chord::new(0, Quality::Minor7)
    }
    fn fm7() -> Chord {
        Chord::new(5, Quality::Minor7)
    }
    fn ab() -> Chord {
        Chord::new(8, Quality::Major7)
    }

    #[test]
    fn saved_words_name_every_quality_and_source_once() {
        for q in Quality::ALL {
            assert_eq!(Quality::from_token(q.token()), Some(q));
            assert!(!q.token().is_empty() && !q.token().contains(' '));
        }
        for h in [Heard::Heard, Heard::Suggested, Heard::Set] {
            assert_eq!(Heard::from_token(h.token()), Some(h));
        }
        assert_eq!(Quality::from_token(""), None);
        assert_eq!(Heard::from_token("yours"), None);
    }

    /// Contract 4's line heard again: yours stays, a chord heard now wins
    /// elsewhere, a suggested chord stays only where nothing is heard,
    /// and a chord heard before goes once nothing plays it.
    #[test]
    fn hearing_again_keeps_your_chords_and_follows_the_rest() {
        let mut line = ChordTimeline::of(&[(cm7(), BAR), (fm7(), BAR)], Heard::Heard);
        assert!(line.set_at(BAR, ChordAnswer::Chord(ab())));
        // The row now plays Cm7 Cm7 Fm7 Fm7 over four bars.
        let heard = ChordTimeline::of(&[(cm7(), 2 * BAR), (fm7(), 2 * BAR)], Heard::Heard);
        let again = line.rehear(&heard);
        assert_eq!(again.len(), 4 * BAR);
        assert_eq!(again.names(), "Cm7 Abmaj7 Fm7");
        let s = again.spans();
        assert_eq!((s[0].start, s[0].len, s[0].heard), (0, BAR, Heard::Heard));
        assert_eq!((s[1].start, s[1].len, s[1].heard), (BAR, BAR, Heard::Set));
        assert_eq!((s[2].start, s[2].len), (2 * BAR, 2 * BAR));

        // A suggested line under a beat: nothing heard keeps it whole.
        let suggested = ChordTimeline::of(&[(cm7(), BAR), (fm7(), BAR)], Heard::Suggested);
        let nothing = ChordTimeline::whole(2 * BAR, ChordAnswer::NoClearChord, Heard::Heard);
        assert_eq!(suggested.rehear(&nothing), suggested);
        assert_eq!(suggested.rehear(&ChordTimeline::default()), suggested);
        // A tune joins: its chords win where they are clear.
        let tune = ChordTimeline::new(vec![
            ChordSpan::new(0, BAR, ChordAnswer::Chord(ab()), Heard::Heard),
            ChordSpan::new(BAR, BAR, ChordAnswer::NoClearChord, Heard::Heard),
        ])
        .unwrap();
        let joined = suggested.rehear(&tune);
        assert_eq!(joined.names(), "Abmaj7 Fm7");
        assert_eq!(joined.spans()[1].heard, Heard::Suggested);

        // Heard chords whose notes went: no clear chord, nothing invented.
        let heard_only = ChordTimeline::of(&[(cm7(), BAR)], Heard::Heard);
        let gone = heard_only.rehear(&ChordTimeline::whole(
            BAR,
            ChordAnswer::NoClearChord,
            Heard::Heard,
        ));
        assert_eq!(gone.names(), "no clear chord");

        // Two spans of one chord heard apart stay apart.
        let twice = ChordTimeline::of(&[(cm7(), BAR), (cm7(), BAR)], Heard::Heard);
        assert_eq!(ChordTimeline::default().rehear(&twice).spans().len(), 2);
        // A reading's sureness isn't kept with the row.
        let mut sure = twice.clone();
        sure.spans[0].sureness = 0.4;
        sure.spans[0].runner_up = Some(fm7());
        assert_eq!(ChordTimeline::default().rehear(&sure), twice);
        assert_eq!(sure.plain(), twice);
        assert_eq!(twice.cut(BAR + 10).len(), BAR + 10);
        assert_eq!(twice.cut(BAR).spans().len(), 1);
    }

    #[test]
    fn spans_join_with_no_gaps() {
        let cm = ChordAnswer::Chord(Chord::new(0, Quality::Minor));
        assert_eq!(
            ChordTimeline::new(vec![
                ChordSpan::new(0, BAR, cm, Heard::Heard),
                ChordSpan::new(BAR + 1, BAR, cm, Heard::Heard)
            ]),
            Err(TimelineError::Gap)
        );
        assert_eq!(
            ChordTimeline::new(vec![ChordSpan::new(0, 0, cm, Heard::Heard)]),
            Err(TimelineError::Empty)
        );
        let mut t = ChordTimeline::of(
            &[
                (Chord::new(0, Quality::Minor7), BAR / 2),
                (Chord::new(5, Quality::Dominant7), BAR / 2),
            ],
            Heard::Heard,
        );
        assert_eq!(t.names(), "Cm7 F7");
        assert_eq!(t.span_at(BAR / 2).unwrap().answer.name(), "F7");
        assert!(t.set_at(BAR / 2, ChordAnswer::Chord(Chord::new(8, Quality::Major7))));
        assert_eq!(t.span_at(BAR - 1).unwrap().heard, Heard::Set);
        assert_eq!(t.names(), "Cm7 Abmaj7");
    }
}
