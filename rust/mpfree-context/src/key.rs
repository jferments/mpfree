//! Keys and modes, and a reading of the key said in words.
//!
//! A reading keeps its candidates' odds for ranking, but the screen never
//! shows them: until they have been checked against labelled melodies a
//! percentage would look more exact than it is (contract 5 of the design).
//! [`KeyReading::words`] is what a musician reads.

use mpfree_scale::Scale;

/// Pitch-class names as musicians spell keys and chords: flats for the
/// black keys apart from F#.
pub const PC_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];

/// The name of a pitch class (any integer, taken modulo 12).
pub fn pc_name(pc: i32) -> &'static str {
    PC_NAMES[pc.rem_euclid(12) as usize]
}

/// The six modes a reading weighs. With 12 tonics that is 72 keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Major,
    Minor,
    Dorian,
    Mixolydian,
    Phrygian,
    Lydian,
}

impl Mode {
    pub const ALL: [Mode; 6] = [
        Mode::Major,
        Mode::Minor,
        Mode::Dorian,
        Mode::Mixolydian,
        Mode::Phrygian,
        Mode::Lydian,
    ];

    /// The mode's notes as a 12-bit mask over the tonic, the same layout
    /// as `mpfree_scale`'s masks.
    pub const fn mask(self) -> u16 {
        match self {
            Mode::Major => mpfree_scale::MAJOR,
            Mode::Minor => mpfree_scale::MINOR,
            Mode::Dorian => mpfree_scale::DORIAN,
            Mode::Mixolydian => mpfree_scale::MIXOLYDIAN,
            Mode::Phrygian => mpfree_scale::PHRYGIAN,
            Mode::Lydian => mpfree_scale::LYDIAN,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Mode::Major => "major",
            Mode::Minor => "minor",
            Mode::Dorian => "Dorian",
            Mode::Mixolydian => "Mixolydian",
            Mode::Phrygian => "Phrygian",
            Mode::Lydian => "Lydian",
        }
    }

    /// A minor third above the tonic.
    pub const fn minorish(self) -> bool {
        self.mask() & (1 << 3) != 0
    }
}

/// A key: a tonic pitch class (0 = C) and a mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub tonic: u8,
    pub mode: Mode,
}

impl Key {
    pub const fn new(tonic: u8, mode: Mode) -> Key {
        Key {
            tonic: tonic % 12,
            mode,
        }
    }

    /// "C minor", "D Dorian".
    pub fn name(self) -> String {
        format!("{} {}", pc_name(i32::from(self.tonic)), self.mode.name())
    }

    /// The key's notes as an `mpfree_scale` scale, for snapping and
    /// stepping notes in key.
    pub const fn scale(self) -> Scale {
        Scale::new(self.tonic, self.mode.mask())
    }

    /// Whether a pitch (any octave) is in the key.
    pub fn has(self, pitch: i32) -> bool {
        let pc = (pitch - i32::from(self.tonic)).rem_euclid(12);
        self.mode.mask() & (1 << pc) != 0
    }

    /// The key's pitch classes as a mask over C (bit 0 = C).
    pub fn pcs(self) -> u16 {
        let m = u32::from(self.mode.mask());
        let t = u32::from(self.tonic);
        (((m << t) | (m >> (12 - t))) & 0x0FFF) as u16
    }
}

/// How sure a reading is, in the words the screen uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sureness {
    /// Nothing stands out: an answer in its own right. New notes then stay
    /// among the notes that were played.
    NoClearKey,
    NotSureYet,
    FairlySure,
    Sure,
}

impl Sureness {
    pub const fn words(self) -> &'static str {
        match self {
            Sureness::NoClearKey => "no clear key",
            Sureness::NotSureYet => "not sure yet",
            Sureness::FairlySure => "fairly sure",
            Sureness::Sure => "sure",
        }
    }
}

/// A candidate key with its odds (0 to 1), used inside for ranking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub key: Key,
    pub odds: f32,
}

/// The provisional lines between the words, on the best candidate's odds.
/// Phase 0C calibrates them against labelled melodies; until then they
/// only choose words, never a number on screen.
pub const FAIRLY_SURE: f32 = 0.45;
pub const SURE: f32 = 0.70;
/// Below this, or with the best two this close, there is no clear key.
pub const NO_CLEAR_KEY: f32 = 0.12;
pub const TIE: f32 = 0.02;
/// Fewer different notes than this never read better than "not sure yet":
/// three notes fit too many keys.
pub const MIN_NOTES_FOR_SURE: u8 = 4;

/// What the listener made of the key: up to three candidates, best first,
/// and how many different pitch classes it heard.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct KeyReading {
    pub candidates: [Option<Candidate>; 3],
    pub distinct_pcs: u8,
}

impl KeyReading {
    /// A reading from candidates in any order; the best three are kept.
    pub fn new(cands: &[Candidate], distinct_pcs: u8) -> KeyReading {
        let mut sorted: [Option<Candidate>; 3] = [None; 3];
        for c in cands {
            if !c.odds.is_finite() || c.odds <= 0.0 {
                continue;
            }
            let mut c = *c;
            for slot in sorted.iter_mut() {
                match slot {
                    Some(s) if s.odds >= c.odds => {}
                    _ => {
                        let out = slot.replace(c);
                        match out {
                            Some(o) => c = o,
                            None => break,
                        }
                    }
                }
            }
        }
        KeyReading {
            candidates: sorted,
            distinct_pcs,
        }
    }

    /// The reading when nothing has been heard.
    pub fn nothing() -> KeyReading {
        KeyReading::default()
    }

    pub fn best(&self) -> Option<Key> {
        self.candidates[0].map(|c| c.key)
    }

    pub fn runner_up(&self) -> Option<Key> {
        self.candidates[1].map(|c| c.key)
    }

    /// The key to compose in, or `None` for no clear key.
    pub fn key(&self) -> Option<Key> {
        match self.sureness() {
            Sureness::NoClearKey => None,
            _ => self.best(),
        }
    }

    pub fn sureness(&self) -> Sureness {
        let Some(best) = self.candidates[0] else {
            return Sureness::NoClearKey;
        };
        let second = self.candidates[1].map_or(0.0, |c| c.odds);
        if best.odds < NO_CLEAR_KEY || best.odds - second < TIE {
            Sureness::NoClearKey
        } else if self.distinct_pcs < MIN_NOTES_FOR_SURE || best.odds < FAIRLY_SURE {
            Sureness::NotSureYet
        } else if best.odds < SURE {
            Sureness::FairlySure
        } else {
            Sureness::Sure
        }
    }

    /// What the screen says: "Likely D minor, also possible D Dorian, not
    /// sure yet", "C minor, fairly sure", "C minor", or "No clear key".
    /// Never a percentage.
    pub fn words(&self) -> String {
        match (self.sureness(), self.best()) {
            (Sureness::NoClearKey, _) | (_, None) => "No clear key".to_string(),
            (Sureness::NotSureYet, Some(k)) => match self.runner_up() {
                Some(r) => format!(
                    "Likely {}, also possible {}, not sure yet",
                    k.name(),
                    r.name()
                ),
                None => format!("Likely {}, not sure yet", k.name()),
            },
            (Sureness::FairlySure, Some(k)) => format!("{}, fairly sure", k.name()),
            (Sureness::Sure, Some(k)) => k.name(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(t: u8, m: Mode, odds: f32) -> Candidate {
        Candidate {
            key: Key::new(t, m),
            odds,
        }
    }

    #[test]
    fn seventy_two_keys_and_their_notes() {
        assert_eq!(Mode::ALL.len() * 12, 72);
        let cm = Key::new(0, Mode::Minor);
        assert!(cm.has(63) && cm.has(70) && !cm.has(64));
        assert_eq!(Key::new(2, Mode::Dorian).name(), "D Dorian");
        assert_eq!(Key::new(10, Mode::Major).name(), "Bb major");
        // D Dorian has the same notes as C major.
        assert_eq!(
            Key::new(2, Mode::Dorian).pcs(),
            Key::new(0, Mode::Major).pcs()
        );
        assert!(Key::new(2, Mode::Dorian).scale().contains(71));
        assert!(Mode::Dorian.minorish() && !Mode::Mixolydian.minorish());
    }

    #[test]
    fn readings_are_said_in_words_never_numbers() {
        // The prototype's D F A: the odds stay inside.
        let r = KeyReading::new(
            &[
                c(9, Mode::Minor, 0.14),
                c(2, Mode::Minor, 0.21),
                c(2, Mode::Dorian, 0.11),
            ],
            3,
        );
        assert_eq!(r.best(), Some(Key::new(2, Mode::Minor)));
        assert_eq!(r.sureness(), Sureness::NotSureYet);
        assert_eq!(
            r.words(),
            "Likely D minor, also possible A minor, not sure yet"
        );
        assert!(!r.words().contains('%'));

        let riff = KeyReading::new(&[c(0, Mode::Minor, 0.47), c(0, Mode::Dorian, 0.17)], 6);
        assert_eq!(riff.words(), "C minor, fairly sure");
        let sure = KeyReading::new(&[c(0, Mode::Minor, 0.8)], 7);
        assert_eq!(sure.words(), "C minor");
    }

    #[test]
    fn three_notes_never_read_as_sure() {
        let r = KeyReading::new(&[c(7, Mode::Major, 0.9)], 3);
        assert_eq!(r.sureness(), Sureness::NotSureYet);
    }

    #[test]
    fn no_clear_key_is_an_answer() {
        assert_eq!(KeyReading::nothing().words(), "No clear key");
        assert_eq!(KeyReading::nothing().key(), None);
        // A tie at the top.
        let tie = KeyReading::new(&[c(0, Mode::Minor, 0.30), c(0, Mode::Major, 0.295)], 5);
        assert_eq!(tie.sureness(), Sureness::NoClearKey);
        assert_eq!(tie.key(), None);
        let flat = KeyReading::new(&[c(0, Mode::Minor, 0.05)], 8);
        assert_eq!(flat.words(), "No clear key");
    }

    #[test]
    fn candidates_are_kept_best_first() {
        let r = KeyReading::new(
            &[
                c(0, Mode::Major, 0.1),
                c(1, Mode::Major, 0.4),
                c(2, Mode::Major, 0.2),
                c(3, Mode::Major, 0.3),
                c(4, Mode::Major, f32::NAN),
            ],
            7,
        );
        let tonics: Vec<u8> = r.candidates.iter().flatten().map(|c| c.key.tonic).collect();
        assert_eq!(tonics, vec![1, 3, 2]);
    }
}
