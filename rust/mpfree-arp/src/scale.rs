//! Scales: keep generated notes in key.
//!
//! The engine applies a scale last, after octaves and degree offsets, so an
//! offset can never land out of key. Leave it off for drum pads, whose
//! "notes" are pad slots, not pitches.

const fn mask(intervals: &[u8]) -> u16 {
    let mut m = 0u16;
    let mut i = 0;
    while i < intervals.len() {
        m |= 1 << intervals[i];
        i += 1;
    }
    m
}

/// A scale: a root (0 = C … 11 = B) and which of the 12 semitones above it
/// belong to it (bit N = N semitones above the root).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    pub root: u8,
    pub mask: u16,
}

pub const CHROMATIC: u16 = 0x0FFF;
pub const MAJOR: u16 = mask(&[0, 2, 4, 5, 7, 9, 11]);
pub const MINOR: u16 = mask(&[0, 2, 3, 5, 7, 8, 10]);
pub const HARMONIC_MINOR: u16 = mask(&[0, 2, 3, 5, 7, 8, 11]);
pub const DORIAN: u16 = mask(&[0, 2, 3, 5, 7, 9, 10]);
pub const PHRYGIAN: u16 = mask(&[0, 1, 3, 5, 7, 8, 10]);
pub const MIXOLYDIAN: u16 = mask(&[0, 2, 4, 5, 7, 9, 10]);
pub const PENTATONIC_MAJOR: u16 = mask(&[0, 2, 4, 7, 9]);
pub const PENTATONIC_MINOR: u16 = mask(&[0, 3, 5, 7, 10]);
pub const BLUES: u16 = mask(&[0, 3, 5, 6, 7, 10]);

/// Scales by name, for lists on the screen.
pub const SCALES: [(&str, u16); 10] = [
    ("Chromatic", CHROMATIC),
    ("Major", MAJOR),
    ("Minor", MINOR),
    ("Harmonic minor", HARMONIC_MINOR),
    ("Dorian", DORIAN),
    ("Phrygian", PHRYGIAN),
    ("Mixolydian", MIXOLYDIAN),
    ("Major pentatonic", PENTATONIC_MAJOR),
    ("Minor pentatonic", PENTATONIC_MINOR),
    ("Blues", BLUES),
];

impl Scale {
    /// A scale; an empty mask is treated as chromatic.
    pub const fn new(root: u8, mask: u16) -> Self {
        let mask = mask & CHROMATIC;
        Self { root: root % 12, mask: if mask == 0 { CHROMATIC } else { mask | 1 } }
    }

    pub fn contains(&self, note: i16) -> bool {
        let pc = (note - i16::from(self.root)).rem_euclid(12);
        self.mask & (1 << pc) != 0
    }

    /// The nearest scale note at or below `note`.
    pub fn snap(&self, note: i16) -> i16 {
        let mut n = note;
        while !self.contains(n) {
            n -= 1;
        }
        n
    }

    /// Snaps `note` into the scale, then moves it `degrees` scale notes up
    /// (or down, if negative).
    pub fn step(&self, note: i16, degrees: i16) -> i16 {
        let mut n = self.snap(note);
        let mut left = degrees;
        while left > 0 {
            n += 1;
            while !self.contains(n) {
                n += 1;
            }
            left -= 1;
        }
        while left < 0 {
            n -= 1;
            while !self.contains(n) {
                n -= 1;
            }
            left += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_and_steps_through_scales() {
        let c_major = Scale::new(0, MAJOR);
        assert!(c_major.contains(60) && !c_major.contains(61));
        assert_eq!(c_major.snap(61), 60);
        assert_eq!(c_major.step(60, 2), 64);
        assert_eq!(c_major.step(60, 7), 72);
        assert_eq!(c_major.step(61, 1), 62, "C# snaps to C, then one up");
        assert_eq!(c_major.step(60, -1), 59);
        let a_minor_pent = Scale::new(9, PENTATONIC_MINOR);
        assert_eq!(a_minor_pent.step(57, 1), 60);
        assert_eq!(a_minor_pent.snap(59), 57);
        assert_eq!(Scale::new(14, 0), Scale { root: 2, mask: CHROMATIC });
        for (name, m) in SCALES {
            assert!(m & 1 == 1, "{name} has its root");
        }
    }
}
