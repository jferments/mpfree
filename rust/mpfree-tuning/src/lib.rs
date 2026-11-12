//! Tunings for MPFree's instruments: how a note number, whole or
//! fractional, becomes a frequency.
//!
//! Every engine used to work this out with one formula: A4 (MIDI 69) at
//! 440 Hz and 12 equal steps to the octave. A [`Tuning`] replaces that
//! formula. The host hands each instrument its tuning between blocks
//! (`mpfree_node::Node::set_tuning`) and the instrument keeps its own copy,
//! so nothing allocates on the audio thread.
//!
//! - **Equal tunings** (12-TET at any A4, 24-TET quarter tones, any number
//!   of equal steps to the octave) use the formula with their own numbers.
//!   The default, 12-TET at A4 = 440 Hz, runs exactly the operations the
//!   engines ran before, so every sound stays identical to the bit.
//! - **Table tunings** (just intonation, Pythagorean, meantone, any Scala
//!   file) give each key from MIDI -64 to 191 a pitch of its own. A note
//!   between two keys (a bend, a slide, a detune) lies between their
//!   pitches on a log scale, so a slide moves evenly from one key's pitch to
//!   the next.
//!
//! [`scala`] reads Scala `.scl` scales and `.kbm` keyboard maps, the format
//! thousands of published tunings come in. [`Builtin`] lists the tunings
//! that need no file.
#![forbid(unsafe_code)]

pub mod scala;

use std::fmt;

/// The lowest key a table tuning holds.
pub const LOW_KEY: i32 = -64;
/// Keys a table tuning holds, from [`LOW_KEY`] up (to MIDI 191).
pub const KEYS: usize = 256;

/// A table's pitch range as log2(Hz): about 0.001 Hz to 1 MHz, so no key
/// is ever infinite or zero.
const LOG2_MIN: f64 = -10.0;
const LOG2_MAX: f64 = 20.0;

/// A4's frequency when nothing else is said.
pub const A4_HZ: f32 = 440.0;

/// Why a tuning couldn't be made: a file that doesn't parse, or a map that
/// gives too few keys a pitch. The message names the line where it can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Map {
    /// `ref_hz * 2^((note - ref_note) / steps)`.
    Equal {
        ref_hz: f32,
        ref_note: f32,
        steps: f32,
    },
    /// The table, linear in log2(Hz) between keys.
    Table,
}

/// How notes become frequencies. Plain data (about 1 KB): cloning it never
/// allocates, so an instrument can take a copy on the audio thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    map: Map,
    /// log2(Hz) of keys [`LOW_KEY`] up, for a table tuning.
    log2: [f32; KEYS],
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning::standard()
    }
}

impl Tuning {
    /// 12-TET with A4 = 440 Hz: the formula every engine used before.
    pub const fn standard() -> Tuning {
        Tuning {
            map: Map::Equal {
                ref_hz: A4_HZ,
                ref_note: 69.0,
                steps: 12.0,
            },
            log2: [0.0; KEYS],
        }
    }

    /// `steps` equal steps to the octave (12 for the usual tuning, 24 for
    /// quarter tones), each key one step, with MIDI 69 at `a4_hz`. A4 out of
    /// 100 to 1000 Hz, or not a number, is clamped (440 for not a number).
    pub fn equal(steps: u16, a4_hz: f32) -> Tuning {
        let a4 = if a4_hz.is_finite() {
            a4_hz.clamp(100.0, 1000.0)
        } else {
            A4_HZ
        };
        Tuning {
            map: Map::Equal {
                ref_hz: a4,
                ref_note: 69.0,
                steps: f32::from(steps.max(1)),
            },
            log2: [0.0; KEYS],
        }
    }

    /// A table tuning from each key's frequency in Hz, asked for every key
    /// from [`LOW_KEY`] to MIDI 191. `None` is a key with no pitch of its
    /// own: it sounds between its neighbours (and past the last key with a
    /// pitch, it carries on at the slope of the two nearest). Needs at least
    /// two keys with a pitch.
    pub fn from_keys(hz: impl Fn(i32) -> Option<f64>) -> Result<Tuning, Error> {
        let mut bad = None;
        let t = Tuning::from_log2(|k| {
            let f = hz(k)?;
            if f.is_finite() && f > 0.0 {
                Some(f.log2())
            } else {
                bad.get_or_insert(k);
                None
            }
        })?;
        match bad {
            Some(k) => Err(Error(format!("key {k}: the frequency must be above 0"))),
            None => Ok(t),
        }
    }

    /// A table tuning with every key's pitch in log2(Hz): what
    /// [`Tuning::from_log2`] gives when every key has one, without its
    /// allocation.
    fn from_every_log2(mut pitch: impl FnMut(i32) -> f64) -> Tuning {
        let mut log2 = [0.0f32; KEYS];
        for (i, o) in log2.iter_mut().enumerate() {
            let p = pitch(LOW_KEY + i as i32);
            *o = if p.is_finite() {
                p.clamp(LOG2_MIN, LOG2_MAX) as f32
            } else {
                0.0
            };
        }
        Tuning {
            map: Map::Table,
            log2,
        }
    }

    /// [`Tuning::from_keys`] in log2(Hz), which can't overflow.
    pub(crate) fn from_log2(mut pitch: impl FnMut(i32) -> Option<f64>) -> Result<Tuning, Error> {
        let mut l = [0.0f64; KEYS];
        let mut known = Vec::with_capacity(KEYS);
        for (i, slot) in l.iter_mut().enumerate() {
            if let Some(p) = pitch(LOW_KEY + i as i32).filter(|p| p.is_finite()) {
                *slot = p.clamp(LOG2_MIN, LOG2_MAX);
                known.push(i);
            }
        }
        if known.len() < 2 {
            return Err(Error(
                "a tuning needs at least two keys with a pitch".to_string(),
            ));
        }
        for w in known.windows(2) {
            let (a, b) = (w[0], w[1]);
            for i in a + 1..b {
                l[i] = l[a] + (l[b] - l[a]) * (i - a) as f64 / (b - a) as f64;
            }
        }
        let (a, b) = (known[0], known[1]);
        let slope = (l[b] - l[a]) / (b - a) as f64;
        for i in 0..a {
            l[i] = (l[a] - slope * (a - i) as f64).clamp(LOG2_MIN, LOG2_MAX);
        }
        let (a, b) = (known[known.len() - 2], known[known.len() - 1]);
        let slope = (l[b] - l[a]) / (b - a) as f64;
        for i in b + 1..KEYS {
            l[i] = (l[b] + slope * (i - b) as f64).clamp(LOG2_MIN, LOG2_MAX);
        }
        let mut log2 = [0.0f32; KEYS];
        for (o, v) in log2.iter_mut().zip(l) {
            *o = v as f32;
        }
        Ok(Tuning {
            map: Map::Table,
            log2,
        })
    }

    /// The frequency in Hz of a note: a MIDI note number, fractional for
    /// bends, slides and detune. Always finite and above 0 for a finite
    /// note.
    #[inline]
    pub fn hz(&self, note: f32) -> f32 {
        match self.map {
            // The same operations as the old `440.0 * exp2((note - 69.0) / 12.0)`.
            Map::Equal {
                ref_hz,
                ref_note,
                steps,
            } => ref_hz * ((note - ref_note) / steps).exp2(),
            Map::Table => self.table_hz(note),
        }
    }

    fn table_hz(&self, note: f32) -> f32 {
        // `max` takes 0 over a NaN, so not a number plays the lowest key.
        let x = (note - LOW_KEY as f32).max(0.0).min((KEYS - 1) as f32);
        let i = (x as usize).min(KEYS - 2);
        let f = x - i as f32;
        let a = self.log2[i];
        (a + (self.log2[i + 1] - a) * f).exp2()
    }

    /// Whether this is 12-TET at A4 = 440 Hz, the tuning every sound was
    /// made in.
    pub fn is_standard(&self) -> bool {
        *self == Tuning::standard()
    }

    /// Whether this is an equal tuning (any steps, any A4).
    pub fn is_equal(&self) -> bool {
        matches!(self.map, Map::Equal { .. })
    }

    /// How far a key sits from its 12-TET pitch at A4 = 440 Hz, in cents
    /// (for a tuning page or a pad label).
    pub fn cents_off(&self, key: i32) -> f32 {
        let std = (key as f32 - 69.0) / 12.0 + A4_HZ.log2();
        (self.hz(key as f32).log2() - std) * 1200.0
    }
}

/// The tunings that need no file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    /// 12 equal steps: the usual tuning.
    Equal12,
    /// 5-limit just intonation, 12 notes on the root: pure thirds and
    /// fifths in the root's key, out of tune in far keys.
    Just,
    /// Pythagorean: every fifth pure (3/2), thirds wide.
    Pythagorean,
    /// Quarter-comma meantone, E-flat to G-sharp: pure major thirds, the
    /// tuning of much Renaissance and early Baroque keyboard music.
    Meantone,
    /// 24 equal steps (quarter tones), each key a quarter tone, A4 on
    /// MIDI 69.
    Equal24,
}

impl Builtin {
    pub const ALL: [Builtin; 5] = [
        Builtin::Equal12,
        Builtin::Just,
        Builtin::Pythagorean,
        Builtin::Meantone,
        Builtin::Equal24,
    ];

    /// The name a menu shows (at most 20 characters).
    pub fn name(self) -> &'static str {
        match self {
            Builtin::Equal12 => "12-TET",
            Builtin::Just => "Just (5-limit)",
            Builtin::Pythagorean => "Pythagorean",
            Builtin::Meantone => "Meantone 1/4 comma",
            Builtin::Equal24 => "24-TET quarter tones",
        }
    }

    /// Whether the root matters (it does for every table tuning).
    pub fn has_root(self) -> bool {
        !matches!(self, Builtin::Equal12 | Builtin::Equal24)
    }

    /// The 12 degrees above 1/1 in cents (the last is the octave), for a
    /// table tuning; `None` for an equal one. Plain data: no allocation.
    pub fn degrees(self) -> Option<[f64; 12]> {
        let ratios: &[(u64, u64); 12] = match self {
            Builtin::Equal12 | Builtin::Equal24 => return None,
            Builtin::Meantone => {
                // Fifths of 5^(1/4) from E-flat (-3) to G-sharp (8), folded
                // into the octave and sorted by pitch class.
                let fifth = 300.0 * 5f64.log2();
                let mut cents = [0.0f64; 12];
                for k in -3i32..=8 {
                    let pc = (7 * k).rem_euclid(12) as usize;
                    cents[pc] = (f64::from(k) * fifth).rem_euclid(1200.0);
                }
                let mut degrees = [1200.0f64; 12];
                degrees[..11].copy_from_slice(&cents[1..]);
                return Some(degrees);
            }
            Builtin::Just => &[
                (16, 15),
                (9, 8),
                (6, 5),
                (5, 4),
                (4, 3),
                (45, 32),
                (3, 2),
                (8, 5),
                (5, 3),
                (9, 5),
                (15, 8),
                (2, 1),
            ],
            Builtin::Pythagorean => &[
                (256, 243),
                (9, 8),
                (32, 27),
                (81, 64),
                (4, 3),
                (729, 512),
                (3, 2),
                (128, 81),
                (27, 16),
                (16, 9),
                (243, 128),
                (2, 1),
            ],
        };
        Some(ratios.map(|(a, b)| 1200.0 * (a as f64 / b as f64).log2()))
    }

    /// The scale above 1/1, for a table tuning (as a Scala scale).
    pub fn scale(self) -> Option<scala::Scale> {
        let description = match self {
            Builtin::Meantone => "Quarter-comma meantone, Eb to G#".to_string(),
            b => b.name().to_string(),
        };
        Some(scala::Scale {
            description,
            cents: self.degrees()?.to_vec(),
        })
    }

    /// The tuning with its 1/1 on pitch class `root` (0 = C to 11 = B,
    /// taken modulo 12) and the root at its 12-TET pitch for `a4_hz`, so
    /// the root sounds where it always did and the rest moves around it.
    /// Equal tunings ignore the root. Never allocates, so the audio thread
    /// may call it; the result is the same, to the bit, as the scale and a
    /// linear keyboard map through [`scala::tuning`].
    pub fn tuning(self, root: u8, a4_hz: f32) -> Tuning {
        let Some(degrees) = self.degrees() else {
            return Tuning::equal(if self == Builtin::Equal24 { 24 } else { 12 }, a4_hz);
        };
        let key = 60 + i32::from(root % 12);
        Tuning::linear(&degrees, key, root_hz(key, a4_hz))
    }
}

/// Key `key`'s 12-TET pitch in Hz for A4 at `a4_hz` (clamped as
/// [`Tuning::equal`] clamps it): where a tuning on that root puts it.
pub fn root_hz(key: i32, a4_hz: f32) -> f64 {
    f64::from(Tuning::equal(12, a4_hz).hz(key as f32))
}

impl Tuning {
    /// Each key the next degree of `cents` (degrees above 1/1, the last
    /// the period) from `key`, which plays 1/1 at `hz`: Scala's degree
    /// arithmetic (`Scale::degree_cents`) with every key given a pitch, so
    /// it never allocates.
    pub(crate) fn linear(cents: &[f64], key: i32, hz: f64) -> Tuning {
        let base = hz.log2();
        let n = cents.len().max(1) as i64;
        let period = cents.last().copied().unwrap_or(1200.0);
        Tuning::from_every_log2(|k| {
            let off = i64::from(k) - i64::from(key);
            let i = off.rem_euclid(n) as usize;
            let within = if i == 0 { 0.0 } else { cents[i - 1] };
            let c = off.div_euclid(n) as f64 * period + within;
            base + c / 1200.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Within 0.02 cents.
    fn close(a: f32, b: f32) -> bool {
        (a / b - 1.0).abs() < 1e-5
    }

    fn old_formula(note: f32) -> f32 {
        440.0 * ((note - 69.0) / 12.0).exp2()
    }

    #[test]
    fn the_standard_tuning_is_the_old_formula_to_the_bit() {
        let t = Tuning::standard();
        assert!(t.is_standard() && t.is_equal());
        assert!(Tuning::default().is_standard());
        assert!(Tuning::equal(12, 440.0).is_standard());
        assert!(Builtin::Equal12.tuning(5, 440.0).is_standard());
        let mut n = -200.0f32;
        while n < 300.0 {
            assert_eq!(t.hz(n).to_bits(), old_formula(n).to_bits(), "note {n}");
            n += 0.013;
        }
        for n in [0.0, 60.0, 69.0, 127.0, -0.0, 1e-7, 60.5, f32::MAX, f32::MIN] {
            assert_eq!(t.hz(n).to_bits(), old_formula(n).to_bits(), "note {n}");
        }
    }

    #[test]
    fn equal_tunings_step_evenly() {
        let q = Tuning::equal(24, 440.0);
        assert!(!q.is_standard());
        assert_eq!(q.hz(69.0), 440.0);
        assert!((q.hz(93.0) - 880.0).abs() < 1e-3);
        assert!((q.hz(70.0) / 440.0 - 2f32.powf(1.0 / 24.0)).abs() < 1e-6);
        let a432 = Tuning::equal(12, 432.0);
        assert_eq!(a432.hz(69.0), 432.0);
        assert!((a432.cents_off(60) - 1200.0 * (432f32 / 440.0).log2()).abs() < 0.01);
        // Out of range A4 is clamped, not a number falls back to 440.
        assert_eq!(Tuning::equal(12, 5.0).hz(69.0), 100.0);
        assert!(Tuning::equal(12, f32::NAN).is_standard());
        assert_eq!(Tuning::equal(0, 440.0).hz(70.0), 880.0);
    }

    #[test]
    fn built_in_tunings_match_their_scala_scales_to_the_bit() {
        for b in Builtin::ALL {
            for root in 0..12u8 {
                for a4 in [415.0, 432.0, 440.0, 442.5, 466.0] {
                    let got = b.tuning(root, a4);
                    let want = match b.scale() {
                        None => continue,
                        Some(scale) => {
                            let key = 60 + i32::from(root);
                            let ref_hz = f64::from(Tuning::equal(12, a4).hz(key as f32));
                            let map = scala::KeyMap::linear(key, key, ref_hz);
                            scala::tuning(&scale, &map).unwrap()
                        }
                    };
                    assert_eq!(got, want, "{} on {root} at {a4}", b.name());
                }
            }
        }
        assert_eq!(Builtin::Equal24.tuning(3, 432.0), Tuning::equal(24, 432.0));
        assert_eq!(Builtin::Equal12.tuning(3, 440.0), Tuning::standard());
    }

    #[test]
    fn just_intonation_is_pure_on_its_root() {
        let c = Builtin::Just.tuning(0, 440.0);
        let root = c.hz(60.0);
        assert!((root - old_formula(60.0)).abs() < 1e-3);
        for (key, ratio) in [
            (64, 1.25f32),
            (67, 1.5),
            (65, 4.0 / 3.0),
            (72, 2.0),
            (48, 0.5),
        ] {
            let r = c.hz(key as f32) / root;
            assert!(close(r, ratio), "key {key}: {r} vs {ratio}");
        }
        // On A, A4 stays at 440 and E5 is a pure fifth above it.
        let a = Builtin::Just.tuning(9, 440.0);
        assert!((a.hz(69.0) - 440.0).abs() < 1e-3);
        assert!((a.hz(76.0) - 660.0).abs() < 2e-3);
        // A4 = 432 moves the root with it.
        let a432 = Builtin::Just.tuning(9, 432.0);
        assert!((a432.hz(69.0) - 432.0).abs() < 1e-3);
        assert!(!c.is_equal() && !c.is_standard());
    }

    #[test]
    fn pythagorean_fifths_and_meantone_thirds_are_pure() {
        let p = Builtin::Pythagorean.tuning(0, 440.0);
        assert!(close(p.hz(67.0) / p.hz(60.0), 1.5));
        assert!(close(p.hz(64.0) / p.hz(60.0), 81.0 / 64.0));
        let m = Builtin::Meantone.tuning(0, 440.0);
        assert!(close(m.hz(64.0) / m.hz(60.0), 1.25));
        assert!(close(m.hz(68.0) / m.hz(64.0), 1.25));
        // Its fifth is a quarter comma narrow: 696.58 cents.
        let fifth = 1200.0 * (m.hz(67.0) / m.hz(60.0)).log2();
        assert!((fifth - 696.578).abs() < 0.01, "{fifth}");
        for b in Builtin::ALL {
            assert!(b.name().len() <= 20, "{}", b.name());
            assert_eq!(b.scale().is_some(), b.has_root());
        }
    }

    #[test]
    fn a_note_between_keys_lies_between_their_pitches() {
        let t = Builtin::Just.tuning(0, 440.0);
        let (a, b) = (t.hz(60.0), t.hz(61.0));
        let mid = t.hz(60.5);
        assert!((mid - (a * b).sqrt()).abs() < 1e-3, "{mid}");
        let mut last = 0.0;
        let mut n = 59.0f32;
        while n < 62.0 {
            let f = t.hz(n);
            assert!(f > last, "rises evenly at {n}");
            last = f;
            n += 0.01;
        }
    }

    #[test]
    fn every_note_gives_a_usable_frequency() {
        for t in Builtin::ALL.map(|b| b.tuning(3, 440.0)) {
            for n in [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                -1e9,
                1e9,
                -64.0,
                191.0,
                191.5,
            ] {
                let f = t.hz(n);
                if t.is_equal() && !n.is_finite() {
                    continue;
                }
                if t.is_equal() && n.abs() > 1e6 {
                    continue;
                }
                assert!(f.is_finite() && f > 0.0, "{n} -> {f}");
            }
        }
    }

    #[test]
    fn keys_without_a_pitch_lie_between_their_neighbours() {
        let t = Tuning::from_keys(|k| match k {
            60 => Some(200.0),
            62 => Some(800.0),
            _ => None,
        })
        .unwrap();
        assert!((t.hz(61.0) - 400.0).abs() < 1e-3);
        // Past the ends the slope carries on: two octaves per two keys.
        assert!((t.hz(64.0) - 3200.0).abs() < 0.1);
        assert!((t.hz(58.0) - 50.0).abs() < 1e-3);
        assert!(Tuning::from_keys(|k| (k == 60).then_some(261.0)).is_err());
        assert!(Tuning::from_keys(|k| Some(if k == 70 { -1.0 } else { 100.0 })).is_err());
    }
}
