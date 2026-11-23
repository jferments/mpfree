//! Convert to notes (step sequencer refactor E1, revised after the outside
//! review of 22 Nov): a clip that follows the pads made into a clip that
//! uses notes, playing exactly what it plays now.
//!
//! A clip of pad hits ([`Holds::Hits`]) stores which pads were hit and
//! plays whatever those pads play now, so on a synth track a new scale,
//! layout or octave changes its tune. [`Clip::convert_to_notes`] turns it
//! into a clip of notes ([`Holds::Notes`]) through the pads as they are
//! now (`pad_notes`, the same pad map playback uses, a chord pad's notes
//! root last, before the track's transpose, so the transpose still applies
//! once):
//!
//! - **Each hit becomes the notes its pad plays**, a chord pad every one,
//!   each keeping everything else about the hit: time, length, velocity,
//!   chance, condition, ratchet and its ramp, tie or slide, on or off, and
//!   its pressure (a chord's notes share the hit's run of values, which
//!   `Takes::reclaim_touch` renumbers as one).
//! - **A hit's notes are its pad's trigger group** ([`pad_group`], see
//!   [`crate::groups`]): they decide once whether they play, with the key
//!   the hit had, so its chance and condition play exactly as they did, a
//!   one-note pad's too.
//! - **Two pads that play one note never meet in it.** A note cuts the
//!   same note already sounding on its track, and a tie continues it,
//!   while two pads are two sounds; so where two pads' notes on one pitch
//!   overlap (or one ties onto the other's end), the notes couldn't play
//!   the same, and it refuses ([`Refused::Overlap`]) rather than merge or
//!   change them.
//! - **One pad hit twice on one step** is one trigger: its two hits
//!   rolled with one key, so with the same chance, condition and on they
//!   decided alike already. With different ones it refuses
//!   ([`Refused::Twice`]).
//! - **A lock on the synth's sound** stays on its step, on the row of its
//!   pad's root.
//! - **The clip as it was before its first Mutate** ([`Clip::original`])
//!   is converted the same way, so Restore never brings pad hits back.
//!   Move lanes stay as they are.
//!
//! **All or nothing.** It refuses, and lists every reason at once
//! ([`Refused`]), when the result couldn't play the same: a lock on a
//! pad's own setting (it has no note to follow), a hit or a
//! lock on a pad that plays no note now, two pads' notes meeting on one
//! pitch, one pad hit twice on one step in two ways, more notes than a
//! clip holds, or two pads that share a root with locks of one setting on
//! one step. The
//! caller makes the result one take, so one UNDO brings the clip back.

use crate::groups::pad_group;
use crate::{Clip, Cond, Holds, Lock, Note, Tick, MAX_NOTES};
use mpfree_param::OnTrack;
use std::collections::HashMap;

/// What [`Clip::convert_to_notes`] made.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    /// The clip of notes, everything else as it was.
    pub clip: Clip,
    /// The pad hits it had, and the notes it has now.
    pub hits: usize,
    pub notes: usize,
}

/// Why [`Clip::convert_to_notes`] changed nothing. `original` marks a
/// reason found only in the clip as it was before its first Mutate, which
/// Restore brings back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// It uses notes already.
    Notes,
    /// A loop's cell holds a loop.
    Loop,
    /// A lock on pad `pad`'s own setting `param` (`OnTrack::Pad`), on the
    /// step at `tick`.
    PadLock {
        pad: u8,
        param: u16,
        tick: Tick,
        original: bool,
    },
    /// A hit or a lock on pad `pad`, which plays no note now.
    NoNote { pad: u8 },
    /// Pads `a` and `b` both play `note`, and the hit of `b` on the step
    /// at `tick` starts while `a`'s sounds (or ties onto its end): as
    /// notes, one would cut or continue the other.
    Overlap { a: u8, b: u8, note: u8, tick: Tick },
    /// It would hold `notes`, more than [`MAX_NOTES`].
    Room { notes: usize },
    /// Pads `a` and `b` share a root, and their locks of one setting on
    /// the step at `tick` would land in one place.
    LocksMeet { a: u8, b: u8, tick: Tick },
    /// Pad `pad` is hit twice on the step at `tick`, with a different
    /// chance, condition or on: as notes, one pad's notes on a step are
    /// one trigger, which has one of each.
    Twice { pad: u8, tick: Tick },
}

impl Clip {
    /// This clip of pad hits as a clip of notes, each hit the notes
    /// `pad_notes` gives for its pad (root last, as `mpfree_play::PadRows`
    /// keeps them; none for a pad that plays nothing); see the
    /// [module](self). Every reason it can't is listed, the clip's first,
    /// each once.
    pub fn convert_to_notes<'a>(
        &self,
        pad_notes: impl Fn(u8) -> &'a [u8],
    ) -> Result<Converted, Vec<Refused>> {
        if self.looped.is_some() {
            return Err(vec![Refused::Loop]);
        }
        if self.holds == Holds::Notes {
            return Err(vec![Refused::Notes]);
        }
        let mut why = Vec::new();
        overlaps(&self.notes, &pad_notes, self.length, &mut why);
        let notes = notes_of(&self.notes, &pad_notes, &mut why);
        let locks = locks_of(&self.locks, &pad_notes, false, &mut why);
        let original = self.original.as_ref().map(|o| {
            overlaps(&o.notes, &pad_notes, o.length, &mut why);
            (
                notes_of(&o.notes, &pad_notes, &mut why),
                locks_of(&o.locks, &pad_notes, true, &mut why),
            )
        });
        let most = notes
            .len()
            .max(original.as_ref().map_or(0, |(n, _)| n.len()));
        if most > MAX_NOTES {
            why.push(Refused::Room { notes: most });
        }
        if !why.is_empty() {
            let mut once = Vec::with_capacity(why.len());
            for r in why {
                if !once.contains(&r) {
                    once.push(r);
                }
            }
            return Err(once);
        }
        let mut clip = self.clone();
        if let (Some(o), Some((n, l))) = (&mut clip.original, original) {
            o.notes = n;
            o.locks = l;
        }
        let made = notes.len();
        clip.notes = notes;
        clip.locks = locks;
        clip.holds = Holds::Notes;
        Ok(Converted {
            clip,
            hits: self.notes.len(),
            notes: made,
        })
    }
}

/// The notes `hits` play through `pad_notes`, every one kept; the reasons
/// they can't go into `why`.
fn notes_of<'a>(
    hits: &[Note],
    pad_notes: &impl Fn(u8) -> &'a [u8],
    why: &mut Vec<Refused>,
) -> Vec<Note> {
    let mut out: Vec<Note> = Vec::with_capacity(hits.len());
    // Each pad's first hit on each tick, for a second one there.
    let mut first: HashMap<(Tick, u8), (u8, Cond, bool)> = HashMap::new();
    for h in hits {
        let shared = (h.chance, h.cond, h.on);
        if *first.entry((h.tick, h.note)).or_insert(shared) != shared {
            why.push(Refused::Twice {
                pad: h.note,
                tick: h.tick,
            });
        }
        let pitches = pad_notes(h.note);
        if pitches.is_empty() {
            why.push(Refused::NoNote { pad: h.note });
            continue;
        }
        // Far past the room: stop growing (the count still refuses).
        if out.len() <= MAX_NOTES {
            // The pad's trigger group: its notes decide as its hit did,
            // with the same key, so its chance and condition play the same.
            let group = pad_group(h.note);
            out.extend(pitches.iter().map(|&p| Note {
                note: p,
                group,
                ..*h
            }));
        } else {
            out.extend(std::iter::repeat_n(*h, pitches.len()));
        }
    }
    out
}

/// Where two pads' hits that play one pitch meet in it, into `why`: one
/// starting while the other sounds, on a clip `length` ticks long that
/// loops (a note past the end sounds on from the start), or tying or
/// sliding onto its end. Hits that are off never sound. Per pitch, sorted,
/// so a full clip is quick.
fn overlaps<'a>(
    hits: &[Note],
    pad_notes: &impl Fn(u8) -> &'a [u8],
    length: Tick,
    why: &mut Vec<Refused>,
) {
    use crate::Join;
    // (pitch, start, end, pad, joins), one a note each hit plays.
    let mut spans: Vec<(u8, Tick, Tick, u8, bool)> = Vec::with_capacity(hits.len());
    for h in hits.iter().filter(|h| h.on) {
        let joins = matches!(h.join, Join::Tie | Join::Slide);
        for &p in pad_notes(h.note) {
            spans.push((p, h.tick, h.tick + h.len.max(1), h.note, joins));
        }
    }
    spans.sort_unstable();
    // `b` starts no earlier than `a`: they meet if `b` starts while `a`
    // sounds or, joining, as `a` ends.
    let meets = |(s, e): (Tick, Tick), t: Tick, joins: bool| (s..e).contains(&t) || joins && t == e;
    // Each pair of pads once, on each pitch.
    let mut seen: Vec<(u8, u8, u8)> = Vec::new();
    for (i, a) in spans.iter().enumerate() {
        for b in spans[i + 1..].iter().take_while(|b| b.0 == a.0) {
            let (pads, note) = ((a.3.min(b.3), a.3.max(b.3)), a.0);
            if a.3 == b.3 || seen.contains(&(pads.0, pads.1, note)) {
                continue;
            }
            // On this pass, or the next: a note past the clip's end goes
            // on from its start.
            let back = |(_, s, e, _, _): &(u8, Tick, Tick, u8, bool)| (s - length, e - length);
            let met = meets((a.1, a.2), b.1, b.4)
                || length > 0 && a.2 > length && meets(back(a), b.1, b.4)
                || length > 0 && b.2 > length && meets(back(b), a.1, a.4);
            if met {
                seen.push((pads.0, pads.1, note));
                why.push(Refused::Overlap {
                    a: pads.0,
                    b: pads.1,
                    note,
                    tick: b.1,
                });
            }
        }
    }
}

/// `locks` with each pad's on its root; the reasons they can't go into
/// `why`.
fn locks_of<'a>(
    locks: &[Lock],
    pad_notes: &impl Fn(u8) -> &'a [u8],
    original: bool,
    why: &mut Vec<Refused>,
) -> Vec<Lock> {
    let mut out: Vec<Lock> = Vec::with_capacity(locks.len());
    // Each lock's pad, beside it, for a clash's words.
    let mut from: Vec<Option<u8>> = Vec::with_capacity(locks.len());
    for l in locks {
        if let OnTrack::Pad(pad) = l.addr.on {
            why.push(Refused::PadLock {
                pad,
                param: l.addr.param,
                tick: l.tick,
                original,
            });
            continue;
        }
        let mut n = *l;
        if let Some(pad) = l.sound {
            let Some(&root) = pad_notes(pad).last() else {
                why.push(Refused::NoNote { pad });
                continue;
            };
            n.sound = Some(root);
        }
        if let Some(i) = out.iter().position(|o| o.same_place(&n)) {
            why.push(Refused::LocksMeet {
                a: from[i].unwrap_or(0),
                b: l.sound.unwrap_or(0),
                tick: l.tick,
            });
            continue;
        }
        out.push(n);
        from.push(l.sound);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cond, Join, Touch, TouchPoint};
    use mpfree_param::RelAddr;

    /// Pad 0 plays C3 (48), pad 1 a C major chord (E, G, C: root last),
    /// pad 2 C3 too, the rest nothing.
    fn pads(p: u8) -> &'static [u8] {
        match p {
            0 => &[48],
            1 => &[52, 55, 48],
            2 => &[48],
            _ => &[],
        }
    }

    fn hits() -> Clip {
        let mut c = Clip::new("riff", 3840);
        let touch = c
            .add_touch(&[
                TouchPoint { at: 10, value: 50 },
                TouchPoint { at: 90, value: 70 },
            ])
            .unwrap();
        c.add_note(Note {
            cond: Cond::Every { a: 1, b: 2 },
            ratchet: 3,
            ramp: 2,
            join: Join::Tie,
            touch,
            ..Note::new(0, 240, 0, 100)
        })
        .unwrap();
        c.add_note(Note::new(480, 120, 1, 90)).unwrap();
        c
    }

    fn inst(param: u16) -> RelAddr {
        RelAddr {
            on: OnTrack::Inst,
            param,
        }
    }

    #[test]
    fn each_hit_becomes_its_pads_notes_and_keeps_the_rest() {
        let c = hits();
        let made = c.convert_to_notes(pads).unwrap();
        assert_eq!((made.hits, made.notes), (2, 4));
        assert_eq!(made.clip.holds, Holds::Notes);
        let first = made.clip.notes[0];
        assert_eq!(
            first,
            Note {
                note: 48,
                group: 1,
                ..c.notes[0]
            },
            "everything but the number, in its pad's trigger group"
        );
        assert_ne!(first.touch, Touch::NONE);
        let chord: Vec<(Tick, u8)> = made.clip.notes[1..]
            .iter()
            .map(|n| (n.tick, n.note))
            .collect();
        assert_eq!(chord, vec![(480, 52), (480, 55), (480, 48)]);
        // The rest of the clip as it was.
        assert_eq!(made.clip.touch, c.touch);
        assert_eq!((made.clip.length, &made.clip.name), (c.length, &c.name));
    }

    #[test]
    fn two_pads_meeting_on_one_note_are_refused_and_apart_they_convert() {
        // Pads 0 and 2 both play C3: hit together, the second note would
        // cut the first.
        let mut c = Clip::new("both", 3840);
        c.add_note(Note::new(0, 100, 0, 100)).unwrap();
        c.add_note(Note::new(0, 100, 2, 80)).unwrap();
        let met = Refused::Overlap {
            a: 0,
            b: 2,
            note: 48,
            tick: 0,
        };
        assert_eq!(c.convert_to_notes(pads).unwrap_err(), vec![met.clone()]);
        // One after the other they convert, each its own note.
        c.notes[1].tick = 100;
        let made = c.convert_to_notes(pads).unwrap();
        let notes: Vec<(Tick, u8)> = made.clip.notes.iter().map(|n| (n.tick, n.note)).collect();
        assert_eq!(notes, vec![(0, 48), (100, 48)]);
        // A tie onto the other's end would continue it.
        c.notes[1].join = Join::Tie;
        assert!(matches!(
            c.convert_to_notes(pads).unwrap_err()[..],
            [Refused::Overlap { tick: 100, .. }]
        ));
        // A chord pad's root meets pad 0, and a note past the clip's end
        // meets one at its start.
        let mut c = Clip::new("wrap", 960);
        c.add_note(Note::new(900, 200, 1, 100)).unwrap();
        c.add_note(Note::new(50, 10, 0, 100)).unwrap();
        assert!(matches!(
            c.convert_to_notes(pads).unwrap_err()[..],
            [Refused::Overlap {
                a: 0,
                b: 1,
                note: 48,
                ..
            }]
        ));
        // A hit that is off never sounds.
        c.notes[0].on = false;
        assert!(c.convert_to_notes(pads).is_ok());
    }

    #[test]
    fn locks_move_to_their_pads_root_and_whole_step_locks_stay() {
        let mut c = hits();
        c.locks.push(Lock::new(480, Some(1), inst(3), 0.5));
        c.locks.push(Lock::new(0, None, inst(4), 0.25));
        let made = c.convert_to_notes(pads).unwrap();
        assert_eq!(made.clip.locks[0].sound, Some(48), "the chord's root");
        assert_eq!(made.clip.locks[1], c.locks[1]);
    }

    #[test]
    fn every_reason_it_refuses_is_listed_and_nothing_changes() {
        let mut c = hits();
        c.add_note(Note::new(960, 100, 7, 100)).unwrap();
        c.add_note(Note::new(1200, 100, 7, 100)).unwrap();
        let tune = RelAddr {
            on: OnTrack::Pad(1),
            param: 2,
        };
        c.locks.push(Lock::new(480, Some(1), tune, 0.1));
        // Pads 0 and 2 share C3: their locks of one setting meet.
        c.locks.push(Lock::new(0, Some(0), inst(3), 0.5));
        c.locks.push(Lock::new(0, Some(2), inst(3), 0.6));
        let why = c.convert_to_notes(pads).unwrap_err();
        assert_eq!(
            why,
            vec![
                Refused::NoNote { pad: 7 },
                Refused::PadLock {
                    pad: 1,
                    param: 2,
                    tick: 480,
                    original: false
                },
                Refused::LocksMeet {
                    a: 0,
                    b: 2,
                    tick: 0
                },
            ],
            "pad 7 once, though hit twice"
        );
        let mut n = Clip::new("n", 3840);
        n.holds = Holds::Notes;
        assert_eq!(n.convert_to_notes(pads).unwrap_err(), vec![Refused::Notes]);
        let mut l = Clip::new("l", 3840);
        l.looped = Some(0);
        assert_eq!(l.convert_to_notes(pads).unwrap_err(), vec![Refused::Loop]);
    }

    #[test]
    fn chord_pads_past_a_clips_room_are_refused() {
        let mut c = Clip::new("full", 3840);
        for i in 0..MAX_NOTES / 2 + 1 {
            c.add_note(Note::new((i % 3840) as Tick, 10, 1, 100))
                .unwrap();
        }
        let why = c.convert_to_notes(pads).unwrap_err();
        assert_eq!(
            why,
            vec![Refused::Room {
                notes: 3 * (MAX_NOTES / 2 + 1)
            }]
        );
    }

    /// [`hits`] with its original kept, as Mutate keeps it.
    fn mutated() -> Clip {
        let mut c = hits();
        c.original = Some(Box::new(crate::Original {
            length: c.length,
            notes: c.notes.clone(),
            locks: c.locks.clone(),
            moves: c.moves.clone(),
            variation: 1,
        }));
        c.notes[1].velocity = 40;
        c
    }

    #[test]
    fn the_original_restore_brings_back_is_converted_too() {
        let made = mutated().convert_to_notes(pads).unwrap();
        let o = made.clip.original.as_ref().expect("kept");
        let notes: Vec<u8> = o.notes.iter().map(|n| n.note).collect();
        assert_eq!(notes, vec![48, 52, 55, 48]);
        // A reason found in the original alone is named as such.
        let mut c = mutated();
        c.original.as_mut().unwrap().notes[0].note = 7;
        assert_eq!(
            c.convert_to_notes(pads).unwrap_err(),
            vec![Refused::NoNote { pad: 7 }]
        );
    }

    /// Each hit's notes are its pad's trigger group, keeping its chance
    /// and condition: a chord pad's and a one-note pad's.
    #[test]
    fn a_hits_notes_are_its_pads_trigger_group_and_keep_its_chance() {
        let mut c = hits();
        c.notes[0].chance = 50;
        c.notes[1].chance = 25;
        c.notes[1].cond = Cond::Fill;
        let made = c.convert_to_notes(pads).unwrap();
        let got: Vec<(u8, u8, u8, Cond)> = made
            .clip
            .notes
            .iter()
            .map(|n| (n.note, n.group, n.chance, n.cond))
            .collect();
        let half = Cond::Every { a: 1, b: 2 };
        assert_eq!(
            got,
            vec![
                (48, 1, 50, half),
                (52, 2, 25, Cond::Fill),
                (55, 2, 25, Cond::Fill),
                (48, 2, 25, Cond::Fill)
            ]
        );
        assert!(made.clip.groups_ok());
        assert_eq!(made.clip.check(), Ok(()));
    }

    /// One pad hit twice on one step is one trigger: alike, it converts;
    /// with two chances, it refuses.
    #[test]
    fn one_pad_hit_twice_on_a_step_converts_only_when_alike() {
        let mut c = hits();
        c.notes.push(Note {
            cond: Cond::Every { a: 1, b: 2 },
            ..Note::new(0, 120, 0, 60)
        });
        let made = c.convert_to_notes(pads).unwrap();
        assert!(made.clip.groups_ok());
        c.notes[2].chance = 50;
        assert_eq!(
            c.convert_to_notes(pads).unwrap_err(),
            vec![Refused::Twice { pad: 0, tick: 0 }]
        );
    }
}
