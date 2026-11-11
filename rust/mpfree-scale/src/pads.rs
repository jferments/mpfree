//! The 16 pads laid out from a scale, for a synth on the pads (owner,
//! 2026-11-11, docs/project/scales.md):
//!
//! - **Chromatic** (the default): pad 1 is C3 and each pad a semitone up, as
//!   in round 6. With a scale picked, the pads keep their notes and only
//!   their colour changes: root, in the scale, outside it.
//! - **In key**: only scale notes. Pad 1 is the root in octave 3; the rows
//!   run in sequence (16 scale notes) or a fourth apart (each row starts
//!   three scale notes above the row below, so a chord has one shape).
//! - **Chord pads**: a chord from the scale on each pad. The bottom two rows
//!   are the triads on each scale note (I to VII, then I an octave up), the
//!   top two rows the sevenths on the same notes. A chord's notes are given
//!   root last, so a mono synth that plays the last note plays the root.
//!
//! [`PadLayout::toggled`] is the one quick switch between Chromatic and In
//! key. Pad 0 is pad 1, bottom left; pads count left to right, then up.
//! [`layout`] allocates nothing; [`pad_name`] makes the tile's text.

use crate::{note_name, Scale, NOTE_NAMES};

/// Pads on the grid.
pub const PADS: usize = 16;
/// The note pad 1 plays at octave 0: C3 (60, middle C, is C4).
pub const BASE_NOTE: i16 = 48;
/// The most notes one pad plays (a seventh chord).
pub const MAX_PAD_NOTES: usize = 4;

/// How the pads are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadLayout {
    #[default]
    Chromatic,
    InKey,
    Chords,
}

impl PadLayout {
    pub const ALL: [PadLayout; 3] = [PadLayout::Chromatic, PadLayout::InKey, PadLayout::Chords];

    pub fn label(self) -> &'static str {
        match self {
            PadLayout::Chromatic => "Chromatic",
            PadLayout::InKey => "In key",
            PadLayout::Chords => "Chord pads",
        }
    }

    /// The quick switch (owner, 2026-11-11): Chromatic to In key, and any
    /// other layout back to Chromatic.
    pub fn toggled(self) -> PadLayout {
        match self {
            PadLayout::Chromatic => PadLayout::InKey,
            _ => PadLayout::Chromatic,
        }
    }
}

/// How the In key layout fills its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InKeyRows {
    /// The 16 pads climb the scale, left to right, then up.
    #[default]
    Sequence,
    /// Each row starts three scale notes above the row below.
    Fourths,
}

impl InKeyRows {
    pub const ALL: [InKeyRows; 2] = [InKeyRows::Sequence, InKeyRows::Fourths];

    pub fn label(self) -> &'static str {
        match self {
            InKeyRows::Sequence => "In sequence",
            InKeyRows::Fourths => "A fourth apart",
        }
    }
}

/// Everything the pads' notes depend on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadSetup {
    pub layout: PadLayout,
    pub rows: InKeyRows,
    /// The pads' own scale (or the project key); None is Off, and every
    /// layout is then plain Chromatic.
    pub scale: Option<Scale>,
    /// Octaves up (or down) from pad 1 = C3, or from the root in octave 3.
    pub octave: i8,
}

/// The pad banks' layouts out of the box, as (layout, octaves above the
/// pads' octave), A to H (owner, 2026-11-11, to discuss again later): A
/// Chromatic, B In key, C Chord pads, D In key an octave up; E to H start
/// as copies of A to D.
pub const DEFAULT_BANKS: [(PadLayout, i8); 8] = [
    (PadLayout::Chromatic, 0),
    (PadLayout::InKey, 0),
    (PadLayout::Chords, 0),
    (PadLayout::InKey, 1),
    (PadLayout::Chromatic, 0),
    (PadLayout::InKey, 0),
    (PadLayout::Chords, 0),
    (PadLayout::InKey, 1),
];

/// What a pad is, for its colour on the screen and on the hardware pad.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadRole {
    /// No scale: nothing to show.
    #[default]
    Plain,
    /// The root (Chord pads: the home chord, I).
    Root,
    /// Another note, or chord, of the scale.
    InScale,
    /// A note outside the scale (Chromatic only).
    OutOfScale,
}

/// One pad: the notes it plays (none when they fall outside 0..=127), its
/// role, and on Chord pads the scale degree its chord is built on (0 = I).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pad {
    notes: [u8; MAX_PAD_NOTES],
    len: u8,
    pub role: PadRole,
    pub degree: Option<u8>,
    /// Chord pads: a seventh rather than a triad.
    pub seventh: bool,
}

impl Pad {
    /// The notes, a chord's root last.
    pub fn notes(&self) -> &[u8] {
        &self.notes[..usize::from(self.len)]
    }

    fn of(notes: &[i16], role: PadRole) -> Pad {
        let mut p = Pad {
            role,
            ..Pad::default()
        };
        if notes.iter().all(|n| (0..=127).contains(n)) {
            for (k, &n) in notes.iter().take(MAX_PAD_NOTES).enumerate() {
                p.notes[k] = n as u8;
            }
            p.len = notes.len().min(MAX_PAD_NOTES) as u8;
        }
        p
    }
}

fn role_of(scale: &Scale, note: i16) -> PadRole {
    if scale.is_root(note) {
        PadRole::Root
    } else if scale.contains(note) {
        PadRole::InScale
    } else {
        PadRole::OutOfScale
    }
}

/// The 16 pads for `setup`.
pub fn layout(setup: &PadSetup) -> [Pad; PADS] {
    let mut pads = [Pad::default(); PADS];
    let octave = 12 * i16::from(setup.octave);
    let Some(scale) = setup.scale else {
        for (i, p) in pads.iter_mut().enumerate() {
            *p = Pad::of(&[BASE_NOTE + octave + i as i16], PadRole::Plain);
        }
        return pads;
    };
    let start = BASE_NOTE + i16::from(scale.root) + octave;
    for (i, p) in pads.iter_mut().enumerate() {
        let i = i as i16;
        *p = match setup.layout {
            PadLayout::Chromatic => {
                let note = BASE_NOTE + octave + i;
                Pad::of(&[note], role_of(&scale, note))
            }
            PadLayout::InKey => {
                let k = match setup.rows {
                    InKeyRows::Sequence => i,
                    InKeyRows::Fourths => 3 * (i / 4) + i % 4,
                };
                let note = scale.nth(start, k);
                Pad::of(&[note], role_of(&scale, note))
            }
            PadLayout::Chords => {
                let k = i % 8;
                let seventh = i >= 8;
                let root = scale.walk(start, k);
                let third = scale.walk(start, k + 2);
                let fifth = scale.walk(start, k + 4);
                let role = if scale.is_root(root) {
                    PadRole::Root
                } else {
                    PadRole::InScale
                };
                let mut pad = if seventh {
                    Pad::of(&[third, fifth, scale.walk(start, k + 6), root], role)
                } else {
                    Pad::of(&[third, fifth, root], role)
                };
                pad.degree = Some(k as u8);
                pad.seventh = seventh;
                pad
            }
        };
    }
    pads
}

const NUMERALS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

/// A stacked-thirds chord's usual name from its notes above the root
/// (third, fifth, and the seventh if any): (name suffix, numeral suffix,
/// whether the numeral is upper case). None when it has no usual name.
pub fn chord_quality(
    third: u8,
    fifth: u8,
    seventh: Option<u8>,
) -> Option<(&'static str, &'static str, bool)> {
    Some(match (third, fifth, seventh) {
        (4, 7, None) => ("", "", true),
        (3, 7, None) => ("m", "", false),
        (3, 6, None) => ("dim", "\u{b0}", false),
        (4, 8, None) => ("+", "+", true),
        (4, 7, Some(11)) => ("maj7", "maj7", true),
        (4, 7, Some(10)) => ("7", "7", true),
        (3, 7, Some(10)) => ("m7", "7", false),
        (3, 7, Some(11)) => ("mMaj7", "maj7", false),
        (3, 6, Some(10)) => ("m7b5", "\u{f8}7", false),
        (3, 6, Some(9)) => ("dim7", "\u{b0}7", false),
        (4, 8, Some(11)) => ("+maj7", "+maj7", true),
        (4, 8, Some(10)) => ("+7", "+7", true),
        _ => return None,
    })
}

/// A pad tile's text, as (main line, small line): the note ("A3") for the
/// Chromatic and In key layouts; on Chord pads the numeral and the chord's
/// name ("iv7", "Dm7") in a seven-note scale, otherwise the degree and the
/// chord's notes ("4", "D F A"). Empty for a pad with no notes.
pub fn pad_name(setup: &PadSetup, pad: &Pad) -> (String, String) {
    let notes = pad.notes();
    let Some(&root) = notes.last() else {
        return (String::new(), String::new());
    };
    let Some(k) = pad.degree else {
        return (note_name(i16::from(root)), String::new());
    };
    let above = |n: u8| (n + 12 - root % 12) % 12;
    let (third, fifth) = (above(notes[0]), above(notes[1]));
    let seventh = (notes.len() == 4).then(|| above(notes[2]));
    let name = NOTE_NAMES[usize::from(root % 12)];
    let seven = setup.scale.is_some_and(|s| s.len() == 7);
    match chord_quality(third, fifth, seventh) {
        Some((suffix, mark, upper)) if seven => {
            let numeral = NUMERALS[usize::from(k % 7)];
            let numeral = if upper {
                numeral.to_string()
            } else {
                numeral.to_ascii_lowercase()
            };
            (format!("{numeral}{mark}"), format!("{name}{suffix}"))
        }
        _ => {
            let mut by_pitch = [0u8; MAX_PAD_NOTES];
            by_pitch[..notes.len()].copy_from_slice(notes);
            by_pitch[..notes.len()].sort_unstable();
            let names: Vec<&str> = by_pitch[..notes.len()]
                .iter()
                .map(|&n| NOTE_NAMES[usize::from(n % 12)])
                .collect();
            (format!("{}", k + 1), names.join(" "))
        }
    }
}

/// Every pad's tile text, for the screen.
pub fn pad_names(setup: &PadSetup) -> [(String, String); PADS] {
    let pads = layout(setup);
    core::array::from_fn(|i| pad_name(setup, &pads[i]))
}

/// The range the pads cover, for the status chip: "C3 to D#4", or the
/// lowest and highest chord roots on Chord pads.
pub fn range_text(setup: &PadSetup) -> String {
    let pads = layout(setup);
    let lowest = |p: &Pad| p.notes().last().copied();
    match (
        pads.iter().find_map(lowest),
        pads.iter().rev().find_map(lowest),
    ) {
        (Some(a), Some(b)) => format!("{} to {}", note_name(i16::from(a)), note_name(i16::from(b))),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MINOR, PENTATONIC_MINOR};

    fn a_minor(layout: PadLayout) -> PadSetup {
        PadSetup {
            layout,
            scale: Some(Scale::new(9, MINOR)),
            ..PadSetup::default()
        }
    }

    fn mains(setup: &PadSetup) -> Vec<String> {
        pad_names(setup).iter().map(|(a, _)| a.clone()).collect()
    }

    #[test]
    fn chromatic_is_round_6_and_a_scale_only_lights_it() {
        let plain = PadSetup::default();
        let pads = layout(&plain);
        assert_eq!(
            pads.iter().map(|p| p.notes()[0]).collect::<Vec<_>>(),
            (48..64).collect::<Vec<_>>()
        );
        assert!(pads.iter().all(|p| p.role == PadRole::Plain));
        assert_eq!(range_text(&plain), "C3 to D#4");
        // A scale, but the Chromatic layout: same notes, coloured.
        let lit = a_minor(PadLayout::Chromatic);
        let pads = layout(&lit);
        assert_eq!(
            pads.iter().map(|p| p.notes()[0]).collect::<Vec<_>>(),
            (48..64).collect::<Vec<_>>()
        );
        assert_eq!(
            (pads[9].role, pads[0].role, pads[1].role),
            (PadRole::Root, PadRole::InScale, PadRole::OutOfScale)
        );
        // No scale: In key and Chord pads are plain Chromatic too.
        assert_eq!(
            layout(&PadSetup {
                layout: PadLayout::Chords,
                ..plain
            }),
            layout(&plain)
        );
    }

    #[test]
    fn in_key_climbs_the_scale_in_sequence_or_a_fourth_apart() {
        let s = a_minor(PadLayout::InKey);
        assert_eq!(
            mains(&s),
            [
                "A3", "B3", "C4", "D4", "E4", "F4", "G4", "A4", "B4", "C5", "D5", "E5", "F5", "G5",
                "A5", "B5"
            ]
        );
        let pads = layout(&s);
        assert_eq!(
            (pads[0].role, pads[7].role, pads[1].role),
            (PadRole::Root, PadRole::Root, PadRole::InScale)
        );
        let f = PadSetup {
            rows: InKeyRows::Fourths,
            ..s
        };
        assert_eq!(
            mains(&f),
            [
                "A3", "B3", "C4", "D4", "D4", "E4", "F4", "G4", "G4", "A4", "B4", "C5", "C5", "D5",
                "E5", "F5"
            ]
        );
        // A pentatonic scale covers three octaves and one note.
        let p = PadSetup {
            scale: Some(Scale::new(9, PENTATONIC_MINOR)),
            ..s
        };
        assert_eq!(
            (mains(&p)[0].as_str(), mains(&p)[15].as_str()),
            ("A3", "A6")
        );
        // Octaves move it; notes past 127 leave their pads empty.
        assert_eq!(mains(&PadSetup { octave: -1, ..s })[0], "A2");
        let high = layout(&PadSetup { octave: 5, ..s });
        assert!(!high[0].notes().is_empty() && high[15].notes().is_empty());
    }

    #[test]
    fn chord_pads_have_triads_below_and_sevenths_above_root_last() {
        let s = a_minor(PadLayout::Chords);
        let names = pad_names(&s);
        let row = |a: usize| {
            names[a..a + 4]
                .iter()
                .map(|(n, c)| format!("{n} {c}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(row(0), ["i Am", "ii\u{b0} Bdim", "III C", "iv Dm"]);
        assert_eq!(row(4), ["v Em", "VI F", "VII G", "i Am"]);
        assert_eq!(
            row(8),
            ["i7 Am7", "ii\u{f8}7 Bm7b5", "IIImaj7 Cmaj7", "iv7 Dm7"]
        );
        assert_eq!(row(12), ["v7 Em7", "VImaj7 Fmaj7", "VII7 G7", "i7 Am7"]);
        let pads = layout(&s);
        assert_eq!(
            pads[0].notes(),
            [60, 64, 57],
            "C4 E4, then the root A3 last"
        );
        assert_eq!(pads[7].notes(), [72, 76, 69], "I again, an octave up");
        assert_eq!(pads[11].notes(), [65, 69, 72, 62], "Dm7: F A C, then D");
        assert_eq!(
            (pads[0].role, pads[7].role, pads[1].role),
            (PadRole::Root, PadRole::Root, PadRole::InScale)
        );
        // C major's dominant seventh, and the harmonic minor's augmented III.
        let c = PadSetup {
            scale: Some(Scale::new(0, crate::MAJOR)),
            ..s
        };
        assert_eq!(pad_names(&c)[12], ("V7".to_string(), "G7".to_string()));
        let h = PadSetup {
            scale: Some(Scale::new(9, crate::HARMONIC_MINOR)),
            ..s
        };
        assert_eq!(pad_names(&h)[2], ("III+".to_string(), "C+".to_string()));
        // Not seven notes: the degree and the notes.
        let p = PadSetup {
            scale: Some(Scale::new(9, PENTATONIC_MINOR)),
            ..s
        };
        assert_eq!(pad_names(&p)[0], ("1".to_string(), "A D G".to_string()));
        assert_eq!(range_text(&s), "A3 to A4");
    }

    #[test]
    fn the_quick_switch_and_the_banks() {
        assert_eq!(PadLayout::Chromatic.toggled(), PadLayout::InKey);
        assert_eq!(PadLayout::InKey.toggled(), PadLayout::Chromatic);
        assert_eq!(PadLayout::Chords.toggled(), PadLayout::Chromatic);
        assert_eq!(DEFAULT_BANKS[..4], DEFAULT_BANKS[4..]);
        assert_eq!(DEFAULT_BANKS[3], (PadLayout::InKey, 1));
        assert!(PadLayout::ALL.iter().all(|l| l.label().len() <= 18));
        assert!(InKeyRows::ALL.iter().all(|r| r.label().len() <= 18));
    }
}
