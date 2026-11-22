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
