//! Mutate with restore (sequencer S6, the Torso T-1's mutate and the
//! KeyStep's Mutate and Original): a variation of a clip's notes, made from
//! the clip as it was before its first Mutate. The clip keeps that
//! original beside it ([`Clip::original`], saved with it), so Restore puts
//! it back in one tap whatever was edited since, and a second Mutate
//! varies the original again, not the variation.
//!
//! A variation changes some of the notes that are on (about one in four;
//! one in two with [`Amount::More`]), each in one way, picked among those
//! that apply to it:
//!
//! - its **velocity**, 10 to 30 up or down (within 1 to 127);
//! - its **chance**: a note that always plays plays 50 or 75 % of the
//!   passes, and one with a chance always plays;
//! - its **ratchet**: a plain note plays 2 or 3 hits (each at least
//!   [`MIN_HIT`] ticks long), and a ratchet plays one;
//! - its **pitch**, on a melodic track only ([`Pitches`]): the next note
//!   up or down in its key, or among the pitches the clip already plays.
//!
//! In a clip of notes a trigger group's notes vary as one
//! ([`crate::groups`]): every velocity by the same amount, so the voicing
//! stays; the trigger's chance; or the same ratchet choice on each note's
//! own. Pitch moves leave a trigger alone.
//!
//! At least one note changes. A note tied or slid into keeps its pitch and
//! its hits, and so does the note a tie continues, so ties and slides
//! still join. A lock on a note's row (its pitch) moves with the note. The
//! rest is the original's: its length, other locks and moves; the clip's
//! settings (speed, direction and the rest) are the clip's own, as they
//! are now.
//!
//! The variation is a pure function of the original, the seed and the
//! amount: the app picks the seed, so each Mutate sounds new, and a test
//! gets the same variation every time.

use mpfree_moves::Moves;
use mpfree_transport::time::Tick;

use crate::{mix64, Clip, Join, Lock, ModelError, Note, TouchPoint, MAX_LOCKS, MAX_NOTES};

/// The shortest ratchet hit a Mutate makes, in clip ticks (a 1/128 note
/// at 960 to the quarter): shorter, the hits blur into one.
pub const MIN_HIT: Tick = 30;

/// A clip as it was before its first Mutate ([`Clip::original`]): what
/// Restore puts back and what each Mutate varies.
#[derive(Clone, Debug, PartialEq)]
pub struct Original {
    pub length: Tick,
    pub notes: Vec<Note>,
    pub locks: Vec<Lock>,
    pub moves: Moves,
    /// The Mutates made from it, 1 or more: the clip plays variation
    /// `variation` (the clip card says "variant 3").
    pub variation: u32,
}

impl Original {
    /// The clip's notes, locks, length and moves, before any Mutate.
    fn of(clip: &Clip) -> Original {
        Original {
            length: clip.length,
            notes: clip.notes.clone(),
            locks: clip.locks.clone(),
            moves: clip.moves.clone(),
            variation: 0,
        }
    }

    /// Every value in range, as the clip's own ([`Clip::check`]): its
    /// notes name pressure in the clip's `touch`, which only grows, so the
    /// original's notes keep theirs.
    pub fn check(&self, touch: &[TouchPoint]) -> Result<(), ModelError> {
        if !(crate::MIN_CLIP_TICKS..=crate::MAX_CLIP_TICKS).contains(&self.length) {
            return Err(ModelError::ClipLength);
        }
        if self.notes.len() > MAX_NOTES {
            return Err(ModelError::TooManyNotes);
        }
        if self.locks.len() > MAX_LOCKS {
            return Err(ModelError::TooManyLocks);
        }
        if let Some(i) = self.notes.iter().position(|n| !n.fits(self.length, touch)) {
            return Err(ModelError::BadNote(i));
        }
        if let Some(i) = self.locks.iter().position(|l| !l.fits(self.length)) {
            return Err(ModelError::BadLock(i));
        }
        let len = u32::try_from(self.length).map_err(|_| ModelError::ClipLength)?;
        self.moves.check(len).map_err(|_| ModelError::BadMoves)
    }
}

/// How much a Mutate changes: Mutate about one note in four, SHIFT +
/// Mutate about one in two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Amount {
    #[default]
    Little,
    More,
}

/// Where a Mutate may move a note's pitch to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pitches<'a> {
    /// Nowhere: the notes are pads (a drum kit, a sampler's pads), so a
    /// pitch is a sound.
    Pads,
    /// The next pitch up or down among those the clip already plays, so
    /// it stays in whatever key the clip is in.
    Own,
    /// The next of these notes up or down (a scale's notes, low to high).
    Key(&'a [u8]),
}

/// The variation's dice: splitmix64.
struct Dice(u64);

impl Dice {
    /// 0 to `n - 1`.
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        (mix64(self.0) % n.max(1) as u64) as usize
    }
}

/// The ways a note can vary.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Velocity,
    Chance,
    Ratchet,
    Pitch,
}

impl Clip {
    /// The variation Mutate makes of this clip, from `seed` (see the
    /// [module docs](crate::mutate)), keeping the clip as it was before
    /// its first Mutate beside it; None for a loop's cell, or a clip whose
    /// original has no note that is on.
    pub fn mutated(&self, seed: u64, amount: Amount, pitches: Pitches) -> Option<Clip> {
        if self.looped.is_some() {
            return None;
        }
        let base = match &self.original {
            Some(o) => (**o).clone(),
            None => Original::of(self),
        };
        let on: Vec<usize> = (0..base.notes.len())
            .filter(|&i| base.notes[i].on)
            .collect();
        if on.is_empty() {
            return None;
        }
        let mut own: Vec<u8> = base.notes.iter().map(|n| n.note).collect();
        own.sort_unstable();
        own.dedup();
        let keys = match pitches {
            Pitches::Pads => None,
            Pitches::Own => Some(&own[..]),
            Pitches::Key(k) => Some(k),
        };
        let mut dice = Dice(mix64(seed));
        let mut notes = base.notes.clone();
        let mut locks = base.locks.clone();
        let odds = match amount {
            Amount::Little => 4,
            Amount::More => 2,
        };
        // What varies as one: a note, or a trigger group's notes (a clip
        // of notes), in the order their first notes come.
        let mut units: Vec<Vec<usize>> = Vec::with_capacity(on.len());
        for &i in &on {
            let n = base.notes[i];
            let grouped = self.holds == crate::Holds::Notes && n.group != 0;
            let with = grouped
                .then(|| {
                    units.iter().position(|u| {
                        let f = base.notes[u[0]];
                        (f.tick, f.group) == (n.tick, n.group)
                    })
                })
                .flatten();
            match with {
                Some(u) => units[u].push(i),
                None => units.push(vec![i]),
            }
        }
        let mut changed = false;
        for u in &units {
            if dice.below(odds) == 0 {
                vary_unit(u, &mut notes, &mut locks, base.length, keys, &mut dice);
                changed = true;
            }
        }
        if !changed {
            // At least one: a velocity can always move.
            let u = &units[dice.below(units.len())];
            vary_unit(u, &mut notes, &mut locks, base.length, keys, &mut dice);
        }
        let mut out = self.clone();
        out.length = base.length;
        out.notes = notes;
        out.locks = locks;
        out.moves = base.moves.clone();
        out.original = Some(Box::new(Original {
            variation: base.variation.saturating_add(1),
            ..base
        }));
        Some(out)
    }

    /// The clip Restore makes: its original back (its notes, locks, length
    /// and moves as they were before its first Mutate), the clip's
    /// settings as they are, and no original kept. None without one.
    pub fn restored(&self) -> Option<Clip> {
        let o = self.original.as_ref()?;
        let mut c = self.clone();
        c.length = o.length;
        c.notes = o.notes.clone();
        c.locks = o.locks.clone();
        c.moves = o.moves.clone();
        c.original = None;
        Some(c)
    }

    /// The variation it plays ([`Original::variation`]); None when it
    /// keeps no original (it was never mutated, or was restored).
    pub fn variation(&self) -> Option<u32> {
        self.original.as_ref().map(|o| o.variation)
    }
}

/// The notes of `unit` changed: one note as [`vary`] does; a trigger
/// group's notes as one, in one of the ways that apply to all of them:
/// every velocity by the same amount (so the voicing stays), the
/// trigger's chance, or the same ratchet choice on each note's own. Pitch
/// moves leave a trigger alone, as two of its notes could land on one
/// pitch.
fn vary_unit(
    unit: &[usize],
    notes: &mut [Note],
    locks: &mut [Lock],
    length: Tick,
    keys: Option<&[u8]>,
    dice: &mut Dice,
) {
    if let [i] = unit {
        return vary(*i, notes, locks, length, keys, dice);
    }
    let joined = |n: &Note| {
        n.join != Join::Attack
            || notes.iter().any(|m| {
                let d = (m.tick - n.tick).rem_euclid(length.max(1));
                m.join == Join::Tie && m.note == n.note && d > 0 && d <= n.len
            })
    };
    let members: Vec<Note> = unit.iter().map(|&i| notes[i]).collect();
    let mut ways = vec![Way::Velocity, Way::Chance];
    if members
        .iter()
        .all(|n| !joined(n) && (n.ratchet > 1 || n.len / 2 >= MIN_HIT))
    {
        ways.push(Way::Ratchet);
    }
    let first = members[0];
    match ways[dice.below(ways.len())] {
        Way::Velocity => {
            let by = 10 + dice.below(21) as i32;
            let v = i32::from(first.velocity);
            let (a, b) = if dice.below(2) == 0 {
                (by, -by)
            } else {
                (-by, by)
            };
            let d = if (v + a).clamp(1, 127) != v { a } else { b };
            for &i in unit {
                notes[i].velocity = (i32::from(notes[i].velocity) + d).clamp(1, 127) as u8;
            }
        }
        Way::Chance => {
            let chance = if first.chance < 100 {
                100
            } else {
                [50, 75][dice.below(2)]
            };
            for &i in unit {
                notes[i].chance = chance;
            }
        }
        Way::Ratchet => {
            let three = members.iter().all(|n| n.len / 3 >= MIN_HIT) && dice.below(2) == 0;
            for &i in unit {
                notes[i].ratchet = if notes[i].ratchet > 1 {
                    1
                } else if three {
                    3
                } else {
                    2
                };
            }
        }
        Way::Pitch => {}
    }
}

/// Note `i` changed in one of the ways that apply to it.
fn vary(
    i: usize,
    notes: &mut [Note],
    locks: &mut [Lock],
    length: Tick,
    keys: Option<&[u8]>,
    dice: &mut Dice,
) {
    let n = notes[i];
    // A tie or slide joins it to the note before, or a tie continues it:
    // its pitch and hits stay so the join still holds.
    let joined = n.join != Join::Attack
        || notes.iter().any(|m| {
            let d = (m.tick - n.tick).rem_euclid(length.max(1));
            m.join == Join::Tie && m.note == n.note && d > 0 && d <= n.len
        });
    let pitch = keys.filter(|_| !joined).and_then(|k| {
        let up = k.iter().copied().find(|&p| p > n.note);
        let down = k.iter().rev().copied().find(|&p| p < n.note);
        let free = |p: u8| !notes.iter().any(|m| m.tick == n.tick && m.note == p);
        let (a, b) = if dice.below(2) == 0 {
            (up, down)
        } else {
            (down, up)
        };
        a.filter(|&p| free(p)).or(b.filter(|&p| free(p)))
    });
    let mut ways = vec![Way::Velocity, Way::Chance];
    if !joined && (n.ratchet > 1 || n.len / 2 >= MIN_HIT) {
        ways.push(Way::Ratchet);
    }
    if pitch.is_some() {
        ways.push(Way::Pitch);
    }
    let m = &mut notes[i];
    match ways[dice.below(ways.len())] {
        Way::Velocity => {
            let by = 10 + dice.below(21) as i32;
            let v = i32::from(n.velocity);
            let (a, b) = if dice.below(2) == 0 {
                (by, -by)
            } else {
                (-by, by)
            };
            let up = (v + a).clamp(1, 127);
            m.velocity = if up != v { up } else { (v + b).clamp(1, 127) } as u8;
        }
        Way::Chance => {
            m.chance = if n.chance < 100 {
                100
            } else {
                [50, 75][dice.below(2)]
            };
        }
        Way::Ratchet => {
            m.ratchet = if n.ratchet > 1 {
                1
            } else if n.len / 3 >= MIN_HIT && dice.below(2) == 0 {
                3
            } else {
                2
            };
        }
        Way::Pitch => {
            let to = pitch.expect("a pitch to go to");
            m.note = to;
            for l in locks.iter_mut() {
                if l.tick == n.tick && l.sound == Some(n.note) {
                    l.sound = Some(to);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_param::{OnTrack, RelAddr};

    /// A bar of 1/16 hats, velocities 80 to 95, one locked.
    fn hats() -> Clip {
        let mut c = Clip::new("hats", 3840);
        for s in 0..16 {
            c.notes.push(Note::new(s * 240, 120, 42, 80 + s as u8));
        }
        c.locks.push(Lock::new(
            480,
            Some(42),
            RelAddr::new(OnTrack::Inst, 2),
            0.5,
        ));
        c
    }

    fn differ(a: &Clip, b: &Clip) -> usize {
        a.notes.iter().zip(&b.notes).filter(|(x, y)| x != y).count()
    }

    /// Mutate keeps the clip as it was, varies some notes and nothing
    /// else, the same for the same seed; Restore puts it all back.
    #[test]
    fn mutate_varies_some_notes_and_restore_puts_them_back() {
        let c = hats();
        let v = c.mutated(7, Amount::Little, Pitches::Pads).unwrap();
        assert_eq!(v.variation(), Some(1));
        assert_eq!(v.notes.len(), 16);
        let changed = differ(&c, &v);
        assert!((1..16).contains(&changed), "{changed}");
        // Ticks and lengths stay, and on drums the pad does too.
        for (a, b) in c.notes.iter().zip(&v.notes) {
            assert_eq!((a.tick, a.len, a.note), (b.tick, b.len, b.note));
        }
        assert_eq!(v.locks, c.locks);
        assert_eq!(v.check(), Ok(()));
        assert_eq!(c.mutated(7, Amount::Little, Pitches::Pads), Some(v.clone()));
        assert_ne!(c.mutated(8, Amount::Little, Pitches::Pads), Some(v.clone()));
        assert_eq!(v.restored(), Some(c.clone()));
        assert_eq!(c.restored(), None);
        assert_eq!(c.variation(), None);
    }

    /// A bar of chords in a clip of notes: each step a three-note trigger
    /// (a pad's), voiced 90, 70 and 50 loud, and a lone note.
    fn chords() -> Clip {
        let mut c = Clip::new("chords", 3840);
        c.holds = crate::Holds::Notes;
        for s in 0..8 {
            for (pitch, velocity) in [(60, 90), (64, 70), (67, 50)] {
                c.notes.push(Note {
                    group: 2,
                    ..Note::new(s * 480, 240, pitch, velocity)
                });
            }
            c.notes.push(Note::new(s * 480 + 240, 120, 72, 80));
        }
        c
    }

    /// Trigger groups: Mutate changes a trigger as one, whatever the
    /// seed: one chance on all its notes, every velocity moved by the
    /// same amount (the voicing kept), the same ratchet choice, and its
    /// pitches left alone on a melodic track; the same seed, the same
    /// variation.
    #[test]
    fn a_trigger_varies_as_one_and_keeps_its_voicing() {
        let c = chords();
        let key = [60, 62, 64, 65, 67, 69, 71, 72];
        let mut varied = 0;
        for seed in 0..100 {
            let v = c.mutated(seed, Amount::More, Pitches::Key(&key)).unwrap();
            assert_eq!(v.check(), Ok(()), "seed {seed}");
            assert_eq!(
                c.mutated(seed, Amount::More, Pitches::Key(&key)),
                Some(v.clone())
            );
            for s in 0..8 {
                let was: Vec<&Note> = c.notes.iter().filter(|n| n.tick == s * 480).collect();
                let now: Vec<&Note> = v.notes.iter().filter(|n| n.tick == s * 480).collect();
                assert_eq!(now.len(), 3, "seed {seed}");
                let pitches: Vec<u8> = now.iter().map(|n| n.note).collect();
                assert_eq!(
                    pitches,
                    vec![60, 64, 67],
                    "seed {seed}: pitch moves skip it"
                );
                let by: Vec<i32> = was
                    .iter()
                    .zip(&now)
                    .map(|(a, b)| i32::from(b.velocity) - i32::from(a.velocity))
                    .collect();
                assert!(by.iter().all(|&d| d == by[0]), "seed {seed}: {by:?}");
                let ratchets: Vec<u8> = now.iter().map(|n| n.ratchet).collect();
                assert!(ratchets.iter().all(|&r| r == ratchets[0]), "seed {seed}");
                varied += usize::from(was.iter().zip(&now).any(|(a, b)| a != b));
            }
        }
        assert!(varied > 100, "{varied}");
    }

    /// More varies more notes, over many seeds.
    #[test]
    fn more_varies_more() {
        let c = hats();
        let count = |amount| -> usize {
            (0..200)
                .map(|seed| differ(&c, &c.mutated(seed, amount, Pitches::Pads).unwrap()))
                .sum()
        };
        let (little, more) = (count(Amount::Little), count(Amount::More));
        // About 4 and 8 of 16 a time.
        assert!((600..1000).contains(&little), "{little}");
        assert!((1300..1900).contains(&more), "{more}");
    }

    /// A second Mutate varies the original, not the variation, whatever
    /// was edited since; Restore brings back the original's length and
    /// moves too, and keeps the clip's settings as they are now.
    #[test]
    fn a_second_mutate_varies_the_original() {
        let c = hats();
        let mut v = c.mutated(1, Amount::More, Pitches::Pads).unwrap();
        v.notes.truncate(3);
        v.length = 960;
        v.locks.clear();
        v.direction = crate::Direction::Reverse;
        let w = v.mutated(2, Amount::More, Pitches::Pads).unwrap();
        assert_eq!(w.variation(), Some(2));
        assert_eq!(w.notes.len(), 16);
        assert_eq!(w.length, 3840);
        assert_eq!(w.locks, c.locks);
        assert_eq!(w.direction, crate::Direction::Reverse);
        assert_eq!(w.original.as_ref().unwrap().notes, c.notes);
        let r = w.restored().unwrap();
        assert_eq!(r.notes, c.notes);
        assert_eq!(r.length, 3840);
        assert_eq!(r.direction, crate::Direction::Reverse);
        assert_eq!(r.original, None);
    }

    /// Each way stays in range: velocities 1 to 127, chance 50 or 75 or
    /// back to 100, ratchets of 2 or 3 hits no shorter than MIN_HIT, or
    /// back to 1.
    #[test]
    fn every_way_stays_in_range() {
        let mut c = Clip::new("edges", 3840);
        c.notes = vec![
            Note::new(0, 240, 36, 1),
            Note::new(240, 240, 36, 127),
            Note {
                chance: 60,
                ..Note::new(480, 240, 36, 64)
            },
            Note {
                ratchet: 4,
                ..Note::new(720, 240, 36, 64)
            },
            // Too short for a ratchet.
            Note::new(960, 50, 36, 64),
        ];
        for seed in 0..300 {
            let v = c.mutated(seed, Amount::More, Pitches::Pads).unwrap();
            assert_eq!(v.check(), Ok(()));
            for (a, b) in c.notes.iter().zip(&v.notes) {
                assert!(b.chance == a.chance || [50, 75, 100].contains(&b.chance));
                if b.ratchet != a.ratchet {
                    assert!(if a.ratchet > 1 {
                        b.ratchet == 1
                    } else {
                        b.len / Tick::from(b.ratchet) >= MIN_HIT
                    });
                }
                let dv = (i32::from(a.velocity) - i32::from(b.velocity)).abs();
                assert!(dv == 0 || (1..=30).contains(&dv), "{a:?} {b:?}");
            }
            assert_eq!(v.notes[4].ratchet, 1, "too short to ratchet");
        }
    }

    /// On a melodic track a note moves to the next pitch of its key, or of
    /// the clip's own; a lock on its row goes with it, and a tie still
    /// joins: neither the tied note nor the one it continues moves.
    #[test]
    fn a_melodic_note_moves_in_key_and_ties_hold() {
        let mut c = Clip::new("bass", 3840);
        c.notes = vec![
            Note::new(0, 240, 48, 100),
            // Tied: 55 at 240, held through 480.
            Note::new(240, 240, 55, 100),
            Note {
                join: Join::Tie,
                ..Note::new(480, 240, 55, 100)
            },
            Note::new(720, 240, 60, 100),
        ];
        c.locks
            .push(Lock::new(0, Some(48), RelAddr::new(OnTrack::Inst, 1), 0.2));
        let c_minor: Vec<u8> = (0..128u8)
            .filter(|n| [0, 2, 3, 5, 7, 8, 10].contains(&(n % 12)))
            .collect();
        let mut moved = 0;
        for seed in 0..300 {
            for pitches in [Pitches::Own, Pitches::Key(&c_minor)] {
                let v = c.mutated(seed, Amount::More, pitches).unwrap();
                assert_eq!(v.check(), Ok(()));
                assert_eq!(
                    (v.notes[1].note, v.notes[2].note),
                    (55, 55),
                    "the tie holds"
                );
                assert_eq!((v.notes[1].ratchet, v.notes[2].ratchet), (1, 1));
                let n = v.notes[0].note;
                match pitches {
                    Pitches::Own => assert!([48, 55].contains(&n), "{n}"),
                    _ => assert!([46, 48, 50].contains(&n), "{n}"),
                }
                assert_eq!(v.locks[0].sound, Some(n), "the lock goes with it");
                if n != 48 {
                    moved += 1;
                }
                // The top note can go down only, among the clip's own.
                if pitches == Pitches::Own {
                    assert!([55, 60].contains(&v.notes[3].note));
                }
            }
        }
        assert!(moved > 0);
        // A drum track never moves a pad.
        for seed in 0..100 {
            let v = c.mutated(seed, Amount::More, Pitches::Pads).unwrap();
            let pads: Vec<u8> = v.notes.iter().map(|n| n.note).collect();
            assert_eq!(pads, [48, 55, 55, 60]);
        }
    }

    /// A step that is off stays as it is; with nothing on, or on a loop's
    /// cell, there is nothing to mutate.
    #[test]
    fn steps_off_stay_and_an_empty_clip_has_nothing_to_mutate() {
        let mut c = hats();
        for n in c.notes.iter_mut().skip(1) {
            n.on = false;
        }
        for seed in 0..50 {
            let v = c.mutated(seed, Amount::More, Pitches::Pads).unwrap();
            assert_eq!(v.notes[1..], c.notes[1..]);
            assert_ne!(v.notes[0], c.notes[0]);
        }
        c.notes[0].on = false;
        assert_eq!(c.mutated(1, Amount::Little, Pitches::Pads), None);
        assert_eq!(
            Clip::new("empty", 960).mutated(1, Amount::Little, Pitches::Own),
            None
        );
        assert_eq!(
            Clip::new_loop(0, 3840).mutated(1, Amount::Little, Pitches::Own),
            None
        );
    }

    /// A clip's check covers its original.
    #[test]
    fn the_clip_check_covers_its_original() {
        let mut v = hats().mutated(3, Amount::Little, Pitches::Pads).unwrap();
        assert_eq!(v.check(), Ok(()));
        v.original.as_mut().unwrap().notes[0].velocity = 0;
        assert_eq!(v.check(), Err(ModelError::BadNote(0)));
        let mut v = hats().mutated(3, Amount::Little, Pitches::Pads).unwrap();
        v.original.as_mut().unwrap().length = 240;
        assert!(v.check().is_err());
        let mut l = Clip::new_loop(0, 3840);
        l.original = hats()
            .mutated(3, Amount::Little, Pitches::Pads)
            .unwrap()
            .original;
        assert_eq!(l.check(), Err(ModelError::BadLoop));
    }
}
