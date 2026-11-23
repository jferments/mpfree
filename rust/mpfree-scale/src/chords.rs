//! Chords for the note editor (piano roll plan 2B.2): the notes of a chord
//! on a root, and the name of the chord some notes make.
//!
//! - [`chord`] gives a chord's notes, lowest first. A Triad and a Seventh
//!   take a scale's own notes when there is one, as the Chord pads do
//!   ([`crate::pads`]), so in C major a D gives D F A; with none they are
//!   major or minor. Sus2, Sus4, Power and Octave are the same shape in
//!   any key.
//! - [`ChordName::of`] names the chord some notes make, with the Chord
//!   pads' namer ([`crate::pads::chord_quality`]) and sus2, sus4 and
//!   power chords besides, and the bass after a slash when it isn't the
//!   root ("C/E"). Nothing here allocates.

use crate::pads::chord_quality;
use crate::{Scale, NOTE_NAMES};
use core::fmt;

/// A chord's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Root, third and fifth: in the scale, or major or minor.
    Triad,
    /// A triad and its seventh: in the scale, or major or minor seventh.
    Seventh,
    /// Root, the second and the fifth.
    Sus2,
    /// Root, the fourth and the fifth.
    Sus4,
    /// Root and fifth.
    Power,
    /// Root and the octave over it.
    Octave,
}

/// The most notes [`chord`] gives (a seventh chord).
pub const MAX_CHORD: usize = 4;

/// A chord's notes, lowest first; notes outside 0..=127 are left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chord {
    notes: [u8; MAX_CHORD],
    len: u8,
}

impl Chord {
    /// The notes, lowest first.
    pub fn notes(&self) -> &[u8] {
        &self.notes[..usize::from(self.len)]
    }

    fn of(notes: &[i16]) -> Chord {
        let mut c = Chord::default();
        for &n in notes.iter().take(MAX_CHORD) {
            if (0..=127).contains(&n) {
                c.notes[usize::from(c.len)] = n as u8;
                c.len += 1;
            }
        }
        c
    }
}

/// The notes of a `shape` chord on `root`. A Triad or a Seventh goes up
/// `scale`'s notes from the root (a root off the scale counts the scale
/// note above it as its first step), or with no scale is major, or minor
/// when `minor` is set.
pub fn chord(root: i16, shape: Shape, scale: Option<Scale>, minor: bool) -> Chord {
    let up = |steps: &[i16]| -> Chord {
        let mut n = [0i16; MAX_CHORD];
        for (k, s) in steps.iter().enumerate() {
            n[k] = root + s;
        }
        Chord::of(&n[..steps.len()])
    };
    match (shape, scale) {
        (Shape::Triad, Some(k)) => Chord::of(&[root, k.walk(root, 2), k.walk(root, 4)]),
        (Shape::Seventh, Some(k)) => {
            Chord::of(&[root, k.walk(root, 2), k.walk(root, 4), k.walk(root, 6)])
        }
        (Shape::Triad, None) if minor => up(&[0, 3, 7]),
        (Shape::Triad, None) => up(&[0, 4, 7]),
        (Shape::Seventh, None) if minor => up(&[0, 3, 7, 10]),
        (Shape::Seventh, None) => up(&[0, 4, 7, 11]),
        (Shape::Sus2, _) => up(&[0, 2, 7]),
        (Shape::Sus4, _) => up(&[0, 5, 7]),
        (Shape::Power, _) => up(&[0, 7]),
        (Shape::Octave, _) => up(&[0, 12]),
    }
}

/// The name of the chord some notes make: "Dm7", "Csus4", "G5", "C/E".
/// Shown with `{}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordName {
    /// The root's pitch class (0 is C).
    pub root: u8,
    /// What follows the root's name: "", "m", "7", "sus4", "5" ...
    pub suffix: &'static str,
    /// The bass's pitch class when it isn't the root.
    pub bass: Option<u8>,
}

impl ChordName {
    /// The chord `notes` make, whatever their octaves and order: two or
    /// more pitch classes that make a triad, a seventh chord, a sus2, a
    /// sus4 or a power chord. The lowest note is tried as the root first,
    /// so an inversion is named over its bass ("C/E" rather than an Em
    /// with a sharp sixth); None when nothing fits.
    pub fn of(notes: &[u8]) -> Option<ChordName> {
        let bass = notes.iter().min()? % 12;
        let set = notes.iter().fold(0u16, |m, n| m | 1 << (n % 12));
        if set.count_ones() < 2 {
            return None;
        }
        let roots = core::iter::once(bass).chain((0..12).filter(|&r| r != bass));
        for root in roots {
            if set & 1 << root == 0 {
                continue;
            }
            // The set's semitones above this root.
            let m = (set >> root | set << (12 - root)) & 0xFFF;
            if let Some(suffix) = suffix_of(m) {
                return Some(ChordName {
                    root,
                    suffix,
                    bass: (bass != root).then_some(bass),
                });
            }
        }
        None
    }
}

/// The name's suffix for the semitones `m` above a root (bit 0 the root).
fn suffix_of(m: u16) -> Option<&'static str> {
    let bit = |s: u8| m & 1 << s != 0;
    match m {
        0b0000_1000_0101 => return Some("sus2"),
        0b0000_1010_0001 => return Some("sus4"),
        0b0000_1000_0001 => return Some("5"),
        _ => {}
    }
    let one = |of: &[u8]| -> Option<Option<u8>> {
        let mut found = of.iter().filter(|&&s| bit(s));
        match (found.next(), found.next()) {
            (None, _) => Some(None),
            (Some(&s), None) => Some(Some(s)),
            _ => None,
        }
    };
    let third = one(&[3, 4])??;
    let fifth = one(&[6, 7, 8])??;
    let seventh = one(&[9, 10, 11])?;
    let all = 1 | 1 << third | 1 << fifth | seventh.map_or(0, |s| 1 << s);
    if all != m {
        return None;
    }
    chord_quality(third, fifth, seventh).map(|(suffix, _, _)| suffix)
}

impl fmt::Display for ChordName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            NOTE_NAMES[usize::from(self.root % 12)],
            self.suffix
        )?;
        match self.bass {
            Some(b) => write!(f, "/{}", NOTE_NAMES[usize::from(b % 12)]),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DORIAN, MAJOR};

    /// A triad and a seventh go up the scale, as the Chord pads do; with
    /// no scale they are major or minor; the other shapes are the same
    /// in any key; notes past 127 are left out.
    #[test]
    fn a_chords_notes_come_from_its_shape_and_the_scale() {
        let c_major = Some(Scale::new(0, MAJOR));
        let notes = |root, shape, scale, minor| chord(root, shape, scale, minor).notes().to_vec();
        assert_eq!(
            notes(62, Shape::Triad, c_major, false),
            [62, 65, 69],
            "D F A"
        );
        assert_eq!(
            notes(67, Shape::Seventh, c_major, false),
            [67, 71, 74, 77],
            "G7"
        );
        assert_eq!(notes(60, Shape::Triad, None, false), [60, 64, 67]);
        assert_eq!(notes(60, Shape::Triad, None, true), [60, 63, 67]);
        assert_eq!(notes(60, Shape::Seventh, None, false), [60, 64, 67, 71]);
        assert_eq!(notes(60, Shape::Seventh, None, true), [60, 63, 67, 70]);
        assert_eq!(notes(60, Shape::Sus2, c_major, false), [60, 62, 67]);
        assert_eq!(notes(60, Shape::Sus4, None, true), [60, 65, 67]);
        assert_eq!(notes(60, Shape::Power, None, false), [60, 67]);
        assert_eq!(notes(60, Shape::Octave, None, false), [60, 72]);
        // A root off the scale counts the scale note above it first: C#
        // in C major gives C# E G.
        assert_eq!(notes(61, Shape::Triad, c_major, false), [61, 64, 67]);
        // D Dorian's own triad on D is minor.
        let d_dorian = Some(Scale::new(2, DORIAN));
        assert_eq!(notes(62, Shape::Triad, d_dorian, false), [62, 65, 69]);
        assert_eq!(notes(121, Shape::Triad, None, false), [121, 125]);
    }

    /// Names: triads and sevenths as the Chord pads name them, sus and
    /// power chords, the bass after a slash, any octave and order;
    /// nothing for one pitch or a cluster.
    #[test]
    fn notes_are_named_as_the_chord_they_make() {
        let name = |n: &[u8]| ChordName::of(n).map(|c| c.to_string());
        assert_eq!(name(&[62, 65, 69]).as_deref(), Some("Dm"));
        assert_eq!(name(&[62, 65, 69, 72]).as_deref(), Some("Dm7"));
        assert_eq!(name(&[67, 71, 74, 77]).as_deref(), Some("G7"));
        assert_eq!(name(&[60, 64, 67, 71]).as_deref(), Some("Cmaj7"));
        assert_eq!(name(&[71, 74, 77]).as_deref(), Some("Bdim"));
        assert_eq!(name(&[71, 74, 77, 81]).as_deref(), Some("Bm7b5"));
        assert_eq!(name(&[60, 62, 67]).as_deref(), Some("Csus2"));
        assert_eq!(name(&[60, 65, 67]).as_deref(), Some("Csus4"));
        assert_eq!(name(&[43, 50]).as_deref(), Some("G5"));
        assert_eq!(
            name(&[64, 67, 72]).as_deref(),
            Some("C/E"),
            "first inversion"
        );
        assert_eq!(name(&[55, 72, 76]).as_deref(), Some("C/G"));
        assert_eq!(
            name(&[72, 60, 79, 64]).as_deref(),
            Some("C"),
            "any order, doubled"
        );
        assert_eq!(name(&[61, 65, 68]).as_deref(), Some("C#"));
        assert_eq!(name(&[60, 72]), None, "an octave");
        assert_eq!(name(&[60]), None);
        assert_eq!(name(&[]), None);
        assert_eq!(name(&[60, 61, 62]), None);
        assert_eq!(
            name(&[60, 64, 67, 69]).as_deref(),
            Some("Am7/C"),
            "C6 is named as Am7 over C"
        );
    }
}
