//! Turning decisions into notes under a harmony, and back.
//!
//! [`realize`] is deterministic: the same decisions under the same
//! harmony, key and instrument always give the same notes, with no dice.
//! That is what makes re-fitting safe (contract 2 of the design): after a
//! chord change the same decisions are realised again, so rhythm,
//! loudness, timing and the line's shape stay, and only the notes that
//! depend on the chords move.
//!
//! [`decisions_of`] reads decisions back out of plain notes (a clip
//! someone kept or wrote), so "Fit to the new chords" works on any clip,
//! not only on one the composer made.

use mpfree_context::chord::{Chord, Tone};
use mpfree_context::groove::Lane;
use mpfree_context::harmony::Harmony;
use mpfree_context::key::Key;
use mpfree_transport::time::Tick;

use crate::event::{Event, Motion, NoteDecision, Part, Role, Source};
use crate::profile::Profile;
use crate::timing::Timing;

const ALL_PCS: u16 = 0x0FFF;

/// What a part is fitted to.
#[derive(Clone, Copy)]
pub struct Fit<'a> {
    pub harmony: &'a dyn Harmony,
    /// The key, or `None` for no clear key.
    pub key: Option<Key>,
    /// Every pitch class played (bit 0 = C): what notes keep to when
    /// there's no clear key or chord.
    pub played_pcs: u16,
    /// Where the part's tick 0 falls in the harmony's ticks.
    pub origin: Tick,
}

impl Fit<'_> {
    fn chord_at(&self, at: Tick) -> Option<Chord> {
        self.harmony.at(self.origin + at).answer.chord()
    }

    /// The notes a line keeps to: the key's, else the notes played, else
    /// all twelve.
    fn base(&self) -> u16 {
        match self.key {
            Some(k) => k.pcs(),
            None if self.played_pcs & ALL_PCS != 0 => self.played_pcs & ALL_PCS,
            None => ALL_PCS,
        }
    }

    /// What a chord-tone role falls back to with no clear chord: the notes
    /// played if any, so nothing is invented.
    fn fallback(&self) -> u16 {
        if self.played_pcs & ALL_PCS != 0 {
            self.played_pcs & ALL_PCS
        } else {
            self.base()
        }
    }
}

fn pc_bit(pitch: i32) -> u16 {
    1 << pitch.rem_euclid(12)
}

/// A drum lane's pad note when a drum decision names none (General MIDI).
pub fn lane_note(lane: Lane) -> u8 {
    match lane {
        Lane::Kick => 36,
        Lane::Snare => 38,
        Lane::Hats => 42,
        Lane::Other => 39,
    }
}

fn anchor_mask(role: Role, chord: Option<Chord>, fit: &Fit) -> u16 {
    match (role, chord) {
        (Role::Chord(t), Some(c)) => pc_bit(i32::from(c.tone(t))),
        (Role::Colour, Some(c)) => {
            let colours = pc_bit(i32::from(c.root) + 2) | pc_bit(i32::from(c.root) + 9);
            let in_key = colours & fit.base();
            if in_key != 0 {
                in_key
            } else {
                colours
            }
        }
        (Role::Scale, c) => {
            let base = fit.base();
            let off = base & !c.map_or(0, |c| c.pcs());
            if off != 0 {
                off
            } else {
                base
            }
        }
        _ => fit.fallback(),
    }
}

/// The pitch in range whose pitch class is in `mask` that best follows the
/// motion from `prev` (or is nearest the register for a first note).
fn choose(mask: u16, prev: Option<i32>, m: Motion, profile: &Profile, register: u8) -> Option<i32> {
    if mask & ALL_PCS == 0 {
        return None;
    }
    let (lo, hi) = (i32::from(profile.low), i32::from(profile.high));
    let want = match prev {
        Some(q) => q + i32::from(m.dir) * i32::from(m.size),
        None => i32::from(register),
    };
    let mut best: Option<(i32, i32, i32)> = None;
    for p in lo..=hi {
        if mask & pc_bit(p) == 0 {
            continue;
        }
        let mut cost = (p - want).abs() * 2;
        if let Some(q) = prev {
            if m.dir != 0 && (p - q).signum() != i32::from(m.dir) {
                cost += 48;
            }
        }
        let key = (cost, prev.map_or(0, |q| (p - q).abs()), p);
        if best.is_none_or(|b| key < b) {
            best = Some(key);
        }
    }
    best.map(|b| b.2)
}

/// Into the profile's range by octaves.
fn fold(p: i32, profile: &Profile) -> i32 {
    let (lo, hi) = (i32::from(profile.low), i32::from(profile.high));
    let mut p = p;
    while p < lo {
        p += 12;
    }
    while p > hi {
        p -= 12;
    }
    p.clamp(lo, hi)
}

/// Among `cands`, the one nearest `want`, preferring notes in `base`.
fn nearest_in(cands: impl Iterator<Item = i32>, want: i32, base: u16) -> Option<i32> {
    cands
        .map(|p| {
            let off_key = if base & pc_bit(p) != 0 { 0 } else { 1 };
            ((p - want).abs() * 2 + off_key, p)
        })
        .min()
        .map(|(_, p)| p)
}

/// The notes a part's decisions make under `fit`, in decision order.
pub fn realize(part: &Part, fit: &Fit) -> Vec<Event> {
    let profile = part.kind.profile();
    let ds = &part.decisions;
    let n = ds.len();
    let mut pitch: Vec<Option<i32>> = vec![None; n];

    // Anchors and fixed notes, left to right.
    let mut prev: Option<i32> = None;
    for (i, d) in ds.iter().enumerate() {
        let p = match (d.fixed, d.role) {
            (Some(f), _) => Some(i32::from(f)),
            (None, Role::Drum(lane)) => Some(i32::from(lane_note(lane))),
            (None, r) if r.anchors() => {
                let mask = anchor_mask(r, fit.chord_at(d.at), fit);
                choose(mask, prev, d.motion, &profile, part.register)
            }
            _ => None,
        };
        if p.is_some() {
            pitch[i] = p;
            prev = p;
        }
    }

    // Approaches, right to left: a half step into the next note, from
    // below when moving up into it (the decision's motion), else above.
    for i in (0..n).rev() {
        if pitch[i].is_none() && ds[i].role == Role::Approach {
            if let Some(next) = pitch[i + 1..].iter().flatten().next() {
                let from = if ds[i].motion.dir < 0 { 1 } else { -1 };
                pitch[i] = Some(fold(next + from, &profile));
            }
        }
    }

    // Passing, neighbour and ghost notes, left to right.
    let base = fit.base();
    for i in 0..n {
        if pitch[i].is_some() {
            continue;
        }
        let d = &ds[i];
        let before = pitch[..i].iter().rev().flatten().next().copied();
        let after = pitch[i + 1..].iter().flatten().next().copied();
        let want = before.map(|q| q + i32::from(d.motion.dir) * i32::from(d.motion.size));
        let p = match (d.role, before, after) {
            (Role::Ghost, Some(q), _) | (Role::Ghost, None, Some(q)) => Some(q),
            (Role::Passing, Some(a), Some(b)) if (b - a).abs() >= 2 => {
                let (lo, hi) = (a.min(b) + 1, a.max(b) - 1);
                nearest_in(lo..=hi, want.unwrap_or((a + b) / 2), base)
            }
            (_, Some(q), _) => {
                // A neighbour: a step or two away, in its direction.
                let dir = if d.motion.dir < 0 { -1 } else { 1 };
                nearest_in((1..=2).map(|s| q + dir * s), want.unwrap_or(q + dir), base)
            }
            (_, None, Some(q)) => Some(q),
            (_, None, None) => Some(i32::from(part.register)),
        };
        pitch[i] = p.map(|p| fold(p, &profile));
    }

    ds.iter()
        .enumerate()
        .map(|(i, d)| {
            let next = ds[i + 1..].iter().map(|e| e.at).find(|&t| t > d.at);
            let p = pitch[i].unwrap_or_else(|| i32::from(part.register));
            Event {
                at: d.at,
                len: profile.length(d.at, d.len, next),
                pitch: p.clamp(0, 127) as u8,
                velocity: d.velocity,
                role: d.role,
                source: d.source,
                timing: d.timing,
            }
        })
        .collect()
}

/// A note as a clip holds it, without decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlainNote {
    pub at: Tick,
    pub len: Tick,
    pub pitch: u8,
    pub velocity: u8,
}

/// Read decisions back out of notes heard under `fit`'s harmony: each
/// note's role over its chord and its motion from the note before. The
/// first note's pitch becomes the register. Realising the result under
/// the same harmony gives the same notes for anything diatonic.
pub fn decisions_of(notes: &[PlainNote], fit: &Fit, source: Source) -> (u8, Vec<NoteDecision>) {
    let base = fit.base();
    let register = notes.first().map_or(60, |n| n.pitch);
    let motion_from = |p: i32, from: Option<i32>| match from {
        Some(q) => Motion {
            dir: (p - q).signum() as i8,
            size: (p - q).unsigned_abs().min(24) as u8,
        },
        None => Motion::NEAREST,
    };
    let mut last_anchor: Option<i32> = None;
    let ds = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let p = i32::from(n.pitch);
            let prev = i.checked_sub(1).map(|j| i32::from(notes[j].pitch));
            let next = notes.get(i + 1).map(|m| i32::from(m.pitch));
            let role = match fit.chord_at(n.at).and_then(|c| c.tone_of(p)) {
                Some(t) => Role::Chord(t),
                None => match (prev, next) {
                    (_, Some(x)) if (x - p).abs() == 1 && base & pc_bit(p) == 0 => Role::Approach,
                    (Some(a), Some(b)) if (a.min(b) < p && p < a.max(b)) && (b - a).abs() <= 4 => {
                        Role::Passing
                    }
                    (Some(q), _) if (1..=2).contains(&(p - q).abs()) => Role::Neighbour,
                    (_, Some(x)) if (x - p).abs() == 1 => Role::Approach,
                    _ => Role::Scale,
                },
            };
            let motion = if role == Role::Approach {
                // Into the next note: from below is up.
                Motion {
                    dir: next.map_or(1, |x| (x - p).signum() as i8),
                    size: 1,
                }
            } else if role.anchors() {
                let m = motion_from(p, last_anchor);
                last_anchor = Some(p);
                m
            } else {
                motion_from(p, prev)
            };
            NoteDecision {
                at: n.at,
                len: n.len,
                velocity: n.velocity,
                role,
                motion,
                motif: None,
                fixed: None,
                lane: Lane::Kick,
                timing: Timing::default(),
                source,
            }
        })
        .collect();
    (register, ds)
}

/// Whether a pitch is a chord tone of the harmony at `at`.
pub fn is_chord_tone(fit: &Fit, at: Tick, pitch: u8) -> bool {
    fit.chord_at(at).is_some_and(|c| c.has(i32::from(pitch)))
}

/// The chord tone a role names at `at`, if there's a chord.
pub fn tone_at(fit: &Fit, at: Tick, tone: Tone) -> Option<u8> {
    fit.chord_at(at).map(|c| c.tone(tone))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::PartId;
    use crate::profile::InstrumentKind;
    use mpfree_context::chord::{ChordTimeline, Heard, Quality};
    use mpfree_context::key::Mode;
    use mpfree_context::PartRole;
    use mpfree_transport::time::PPQ;

    const BEAT: Tick = PPQ as Tick;
    const BAR: Tick = 4 * BEAT;

    fn timeline() -> ChordTimeline {
        ChordTimeline::of(
            &[
                (Chord::new(0, Quality::Minor7), BAR),
                (Chord::new(5, Quality::Minor7), BAR),
            ],
            Heard::Heard,
        )
    }

    #[test]
    fn chord_tones_follow_the_motion() {
        let t = timeline();
        let fit = Fit {
            harmony: &t,
            key: Some(Key::new(0, Mode::Minor)),
            played_pcs: 0,
            origin: 0,
        };
        let d = |at, role, m| NoteDecision::new(at, BEAT, 100, role, m);
        let part = Part {
            id: PartId(1),
            role: PartRole::Bass,
            kind: InstrumentKind::Bass,
            register: 36,
            decisions: vec![
                d(0, Role::Chord(Tone::Root), Motion::NEAREST),
                d(BEAT, Role::Chord(Tone::Fifth), Motion::up(7)),
                d(2 * BEAT, Role::Passing, Motion::down(1)),
                d(3 * BEAT, Role::Approach, Motion::up(1)),
                d(BAR, Role::Chord(Tone::Root), Motion::down(2)),
                d(BAR + BEAT, Role::Ghost, Motion::NEAREST),
            ],
        };
        let ev = realize(&part, &fit);
        let ps: Vec<u8> = ev.iter().map(|e| e.pitch).collect();
        // C2, G2, a passing note between G2 and the approach, E2 (half
        // step under F2), F2, F2.
        assert_eq!(ps[0], 36);
        assert_eq!(ps[1], 43);
        assert_eq!(ps[3], 40);
        assert_eq!(ps[4], 41);
        assert_eq!(ps[5], 41);
        assert!(ps[2] < 43 && ps[2] > 40);
        // A bass keeps its notes apart.
        assert!(ev[0].len < BEAT);
    }

    #[test]
    fn no_clear_chord_keeps_to_the_notes_played() {
        let t = ChordTimeline::whole(BAR, mpfree_context::ChordAnswer::NoClearChord, Heard::Heard);
        let played = (1 << 2) | (1 << 5) | (1 << 9); // D F A
        let fit = Fit {
            harmony: &t,
            key: None,
            played_pcs: played,
            origin: 0,
        };
        let part = Part {
            id: PartId(1),
            role: PartRole::Melody,
            kind: InstrumentKind::OneNote,
            register: 64,
            decisions: (0..8)
                .map(|i| {
                    NoteDecision::new(
                        i * BEAT / 2,
                        BEAT / 2,
                        90,
                        if i % 2 == 0 {
                            Role::Chord(Tone::Root)
                        } else {
                            Role::Scale
                        },
                        Motion::up(3),
                    )
                })
                .collect(),
        };
        for e in realize(&part, &fit) {
            assert!(
                played & pc_bit(i32::from(e.pitch)) != 0,
                "{} not played",
                e.pitch
            );
        }
    }

    #[test]
    fn decisions_read_back_from_notes_make_the_same_notes() {
        let t = timeline();
        let fit = Fit {
            harmony: &t,
            key: Some(Key::new(0, Mode::Minor)),
            played_pcs: 0,
            origin: 0,
        };
        // C minor melody over Cm7 then Fm7.
        let pitches = [
            72, 70, 67, 68, 67, 65, 63, 65, 68, 72, 70, 68, 67, 65, 63, 60,
        ];
        let notes: Vec<PlainNote> = pitches
            .iter()
            .enumerate()
            .map(|(i, &p)| PlainNote {
                at: i as Tick * BEAT / 2,
                len: BEAT / 2,
                pitch: p,
                velocity: 96,
            })
            .collect();
        let (register, ds) = decisions_of(&notes, &fit, Source::You);
        let part = Part {
            id: PartId(0),
            role: PartRole::Melody,
            kind: InstrumentKind::Chordal,
            register,
            decisions: ds,
        };
        let again: Vec<u8> = realize(&part, &fit).iter().map(|e| e.pitch).collect();
        assert_eq!(again, pitches.to_vec());
        assert!(realize(&part, &fit).iter().all(|e| e.source == Source::You));
    }
}
