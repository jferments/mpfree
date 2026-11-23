//! Instrument profiles (contract 10 of the design): the playing adapts to
//! what plays the part. Five kinds to start; Add a part takes the kind
//! from the instrument it names, Improvise from the track's instrument.

use mpfree_transport::time::{Tick, PPQ};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstrumentKind {
    /// A mono lead: one note at a time; legato may glide.
    OneNote,
    /// Keys or a poly synth playing a line: overlap allowed.
    Chordal,
    /// A sampled or synth bass: notes kept apart so it doesn't muddy.
    Bass,
    /// Pads and stabs: voicings in the middle range.
    Chords,
    /// A kit or a one-shot: hits only, note length means nothing.
    Percussive,
}

/// How notes join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Articulation {
    Legato,
    Detached,
    Sustained,
    Hit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    pub kind: InstrumentKind,
    /// Notes at once.
    pub polyphony: u8,
    /// Lowest and highest note it plays well.
    pub low: u8,
    pub high: u8,
    pub articulation: Articulation,
    /// Overlapping notes are wanted.
    pub overlap: bool,
    /// How much loudness changes the sound (0 to 1): how far the playing
    /// rules may move velocity.
    pub velocity_response: f32,
    /// Slides and bends mean something.
    pub slides: bool,
    /// Note length changes the sound.
    pub length_matters: bool,
}

/// The gap a bass or one-note line leaves before its next note.
pub const DETACH: Tick = (PPQ / 32) as Tick;
/// The length a hit is written with.
pub const HIT: Tick = (PPQ / 8) as Tick;

impl InstrumentKind {
    pub const ALL: [InstrumentKind; 5] = [
        InstrumentKind::OneNote,
        InstrumentKind::Chordal,
        InstrumentKind::Bass,
        InstrumentKind::Chords,
        InstrumentKind::Percussive,
    ];

    pub const fn profile(self) -> Profile {
        match self {
            InstrumentKind::OneNote => Profile {
                kind: self,
                polyphony: 1,
                low: 48,
                high: 88,
                articulation: Articulation::Legato,
                overlap: false,
                velocity_response: 0.6,
                slides: true,
                length_matters: true,
            },
            InstrumentKind::Chordal => Profile {
                kind: self,
                polyphony: 8,
                low: 40,
                high: 96,
                articulation: Articulation::Sustained,
                overlap: true,
                velocity_response: 1.0,
                slides: false,
                length_matters: true,
            },
            InstrumentKind::Bass => Profile {
                kind: self,
                polyphony: 1,
                low: 28,
                high: 55,
                articulation: Articulation::Detached,
                overlap: false,
                velocity_response: 0.7,
                slides: true,
                length_matters: true,
            },
            InstrumentKind::Chords => Profile {
                kind: self,
                polyphony: 6,
                low: 48,
                high: 79,
                articulation: Articulation::Sustained,
                overlap: true,
                velocity_response: 0.8,
                slides: false,
                length_matters: true,
            },
            InstrumentKind::Percussive => Profile {
                kind: self,
                polyphony: 16,
                low: 0,
                high: 127,
                articulation: Articulation::Hit,
                overlap: true,
                velocity_response: 1.0,
                slides: false,
                length_matters: false,
            },
        }
    }
}

impl Profile {
    pub fn in_range(&self, pitch: i32) -> bool {
        (i32::from(self.low)..=i32::from(self.high)).contains(&pitch)
    }

    /// The middle of its range: where a part with no other aim starts.
    pub fn centre(&self) -> u8 {
        ((u16::from(self.low) + u16::from(self.high)) / 2) as u8
    }

    /// A note's length on this instrument, given where the next note
    /// starts: one-note and bass lines end before the next note, hits get
    /// a fixed short length.
    pub fn length(&self, at: Tick, len: Tick, next: Option<Tick>) -> Tick {
        match self.articulation {
            Articulation::Hit => HIT,
            _ if !self.overlap => match next {
                Some(n) if n > at => {
                    let gap = if self.articulation == Articulation::Legato {
                        0
                    } else {
                        DETACH
                    };
                    len.min(n - at - gap).max(1)
                }
                _ => len.max(1),
            },
            _ => len.max(1),
        }
    }
}

/// The kind of instrument a line of `notes` sounds written for, when
/// nothing says what plays it (a plug-in, MIDI out): Chordal when at
/// least a third of its notes sound with another, Bass when its middle
/// note is under C3 (48, where [`InstrumentKind::Bass`]'s range sits),
/// else OneNote. Notes switched off are left out.
pub fn kind_of_notes(notes: &[mpfree_seq::Note]) -> InstrumentKind {
    let mut on: Vec<&mpfree_seq::Note> = notes.iter().filter(|n| n.on).collect();
    if on.is_empty() {
        return InstrumentKind::OneNote;
    }
    on.sort_by_key(|n| (n.tick, n.note));
    let together = on
        .iter()
        .enumerate()
        .filter(|&(i, n)| {
            on.iter()
                .enumerate()
                .any(|(j, m)| j != i && m.tick <= n.tick && n.tick < m.tick + m.len.max(1))
        })
        .count();
    if together * 3 >= on.len() {
        return InstrumentKind::Chordal;
    }
    let mut pitches: Vec<u8> = on.iter().map(|n| n.note).collect();
    pitches.sort_unstable();
    match pitches[pitches.len() / 2] < 48 {
        true => InstrumentKind::Bass,
        false => InstrumentKind::OneNote,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_says_what_it_was_written_for() {
        let note = |tick: Tick, note: u8, len: Tick| mpfree_seq::Note::new(tick, len, note, 100);
        let lead: Vec<_> = (0..8)
            .map(|k| note(k * 240, 67 + (k % 3) as u8, 200))
            .collect();
        assert_eq!(kind_of_notes(&lead), InstrumentKind::OneNote);
        let bass: Vec<_> = (0..8)
            .map(|k| note(k * 240, 36 + (k % 2) as u8 * 7, 200))
            .collect();
        assert_eq!(kind_of_notes(&bass), InstrumentKind::Bass);
        let mut keys = lead.clone();
        keys.extend((0..4).map(|k| note(k * 480, 60, 400)));
        assert_eq!(kind_of_notes(&keys), InstrumentKind::Chordal);
        assert_eq!(kind_of_notes(&[]), InstrumentKind::OneNote);
    }

    #[test]
    fn five_kinds_with_sane_ranges() {
        for k in InstrumentKind::ALL {
            let p = k.profile();
            assert_eq!(p.kind, k);
            assert!(p.low < p.high && p.polyphony >= 1);
            assert!(p.in_range(i32::from(p.centre())));
        }
        assert!(InstrumentKind::Bass.profile().high < InstrumentKind::OneNote.profile().low + 12);
    }

    #[test]
    fn lengths_follow_the_instrument() {
        let bass = InstrumentKind::Bass.profile();
        assert_eq!(bass.length(0, 960, Some(480)), 480 - DETACH);
        assert_eq!(bass.length(0, 240, Some(480)), 240);
        let lead = InstrumentKind::OneNote.profile();
        assert_eq!(lead.length(0, 960, Some(480)), 480, "legato: no gap");
        let pad = InstrumentKind::Chords.profile();
        assert_eq!(pad.length(0, 960, Some(480)), 960, "pads may overlap");
        let kit = InstrumentKind::Percussive.profile();
        assert_eq!(kit.length(0, 960, None), HIT);
    }
}
