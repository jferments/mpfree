//! A live shift of a track's notes: the sequencer's live transpose
//! (sequencer roadmap S4, Jay's card of 17 Nov: "16 Levels gets a
//! Transpose kind"; docs/dev/design/make-perform.md).
//!
//! - A [`Shift`] moves a note `by` semitones, or with a key `by` notes of
//!   that scale, so a line stays in key. A note outside the key moves with
//!   the key note below it, keeping its distance above it.
//! - [`Held`] remembers what each note that started became, so the note's
//!   end and its pressure go where it went even when the shift changes
//!   while it sounds: a note already sounding keeps its pitch.
//!
//! Notes are MIDI note numbers. Nothing here allocates.

use crate::Scale;

/// The furthest a shift goes either way: two octaves of semitones, or 24
/// notes of a scale (over three octaves of a seven-note one).
pub const MAX_SHIFT: i8 = 24;

/// A shift of every note a track plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shift {
    /// Semitones, or with `key` notes of its scale; 0 plays as written.
    /// Kept within [`MAX_SHIFT`] either way.
    pub by: i8,
    /// Move by this scale's notes (going up, both ways); None moves by
    /// semitones.
    pub key: Option<Scale>,
}

impl Shift {
    /// Plays as written.
    pub const NONE: Shift = Shift { by: 0, key: None };

    /// By semitones.
    pub const fn semitones(by: i8) -> Shift {
        Shift { by, key: None }
    }

    /// By notes of `key`.
    pub const fn in_key(by: i8, key: Scale) -> Shift {
        Shift { by, key: Some(key) }
    }

    /// It moves nothing.
    pub fn is_none(&self) -> bool {
        self.by == 0
    }

    /// `note` shifted, as a note number; None when it falls off the
    /// keyboard (below 0 or above 127), so the caller leaves it out.
    pub fn apply(&self, note: u8) -> Option<u8> {
        let n = i16::from(note);
        let by = i16::from(self.by.clamp(-MAX_SHIFT, MAX_SHIFT));
        let out = match self.key {
            _ if by == 0 => n,
            None => n + by,
            Some(k) => {
                // The notes going up, both ways: a transposition doesn't
                // depend on the way it goes.
                let key = Scale::new(k.root, k.mask);
                let below = key.snap(n);
                key.walk(below, by) + (n - below)
            }
        };
        u8::try_from(out).ok().filter(|&o| o <= 127)
    }
}

/// What each held note became under the shift it started with, for the
/// paths that end a note by its number (a synth played by note, MIDI
/// Out). A note started again while it sounds (an arp's notes overlapping)
/// plays where it went the first time and is counted, so each of its ends
/// ends one start. 256 bytes; a panic or a cut clears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    to: [u8; 128],
    /// The starts of each note still sounding.
    n: [u8; 128],
}

impl Default for Held {
    fn default() -> Self {
        Held::new()
    }
}

impl Held {
    pub const fn new() -> Held {
        Held {
            to: [0; 128],
            n: [0; 128],
        }
    }

    /// `note` starts under `shift`: the note to play, or None when it
    /// falls off the keyboard. Played again while it still sounds, it
    /// plays where it went the first time.
    pub fn on(&mut self, note: u8, shift: &Shift) -> Option<u8> {
        let i = usize::from(note);
        let n = self.n.get_mut(i)?;
        if *n > 0 {
            *n = n.saturating_add(1);
            return Some(self.to[i]);
        }
        let to = shift.apply(note)?;
        *n = 1;
        self.to[i] = to;
        Some(to)
    }

    /// One start of `note` ends: the note it played, or None if it played
    /// nothing (or every start ended already).
    pub fn off(&mut self, note: u8) -> Option<u8> {
        let i = usize::from(note);
        let n = self.n.get_mut(i)?;
        if *n == 0 {
            return None;
        }
        *n -= 1;
        Some(self.to[i])
    }

    /// The note a held `note` plays, for its pressure.
    pub fn sounding(&self, note: u8) -> Option<u8> {
        let i = usize::from(note);
        (self.n.get(i).copied().unwrap_or(0) > 0).then(|| self.to[i])
    }

    /// Nothing held (a panic, or every note cut).
    pub fn clear(&mut self) {
        self.n = [0; 128];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAJOR, MINOR};

    #[test]
    fn a_shift_moves_by_semitones_or_by_notes_of_its_key() {
        // E2 (40) up a fourth and down a tone; as written at 0.
        assert_eq!(Shift::semitones(5).apply(40), Some(45));
        assert_eq!(Shift::semitones(-2).apply(40), Some(38));
        assert_eq!(Shift::NONE.apply(40), Some(40));
        assert!(Shift::NONE.is_none());
        // Off the keyboard: left out.
        assert_eq!(Shift::semitones(7).apply(125), None);
        assert_eq!(Shift::semitones(-8).apply(3), None);
        // Kept within two octaves.
        assert_eq!(Shift::semitones(100).apply(40), Some(64));
        // C major, two notes up: C to E, E to G, B to D an octave up.
        let c = Scale::new(0, MAJOR);
        assert_eq!(Shift::in_key(2, c).apply(60), Some(64));
        assert_eq!(Shift::in_key(2, c).apply(64), Some(67));
        assert_eq!(Shift::in_key(2, c).apply(71), Some(74));
        // Seven notes of a seven-note scale: an octave either way.
        assert_eq!(Shift::in_key(7, c).apply(62), Some(74));
        assert_eq!(Shift::in_key(-7, c).apply(62), Some(50));
        // Out of key, F# moves with F: F# up one is G#.
        assert_eq!(Shift::in_key(1, c).apply(66), Some(68));
        // A minor, down one from A is G, from C is B.
        let a = Scale::new(9, MINOR);
        assert_eq!(Shift::in_key(-1, a).apply(57), Some(55));
        assert_eq!(Shift::in_key(-1, a).apply(60), Some(59));
        // The key's notes going up, both ways: melodic minor down one from
        // A is G#, as it is on the way up.
        let mm = Scale::with_down(9, crate::MELODIC_MINOR, MINOR);
        assert_eq!(Shift::in_key(-1, mm).apply(57), Some(56));
    }

    #[test]
    fn a_held_note_ends_where_it_went_whatever_the_shift_is_now() {
        let mut h = Held::new();
        let up = Shift::semitones(5);
        assert_eq!(h.on(40, &up), Some(45));
        assert_eq!(h.sounding(40), Some(45));
        // The shift changes while it sounds: its end still ends 45, and
        // played again before it ends it goes there too.
        let down = Shift::semitones(-2);
        assert_eq!(h.on(40, &down), Some(45));
        assert_eq!(h.off(40), Some(45));
        assert_eq!(h.off(40), Some(45), "each start ends");
        assert_eq!(h.off(40), None, "ended");
        assert_eq!(h.sounding(40), None);
        assert_eq!(h.on(40, &down), Some(38));
        // Off the keyboard: nothing plays, nothing to end.
        assert_eq!(h.on(126, &up), None);
        assert_eq!(h.off(126), None);
        h.clear();
        assert_eq!(h.sounding(40), None);
        assert_eq!(h.off(40), None);
    }
}
