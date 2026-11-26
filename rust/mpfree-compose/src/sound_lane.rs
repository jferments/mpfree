//! Continue's sound moves in the clip (composing tools S3, second part):
//! the sound follows the new phrases on Continue's own lane, one lane per
//! knob a move turns ([`By::Continue`]), never your patch.
//!
//! - The composer makes the moves at Subtle and at Bold
//!   ([`crate::job::Done::sound`]); the Sound row picks which are heard,
//!   so changing it never makes a new try.
//! - Each move is a step on its knob's lane a little before its note
//!   starts ([`AHEAD`]), at the knob's value for the move
//!   ([`crate::Safe::value`]), held until the next new note: the
//!   lane ends there unless that note has a move of its own, or at the
//!   moved note's end when no new note follows, so a looping clip comes
//!   round to your bars on your knob.
//! - Brightness and Length on the track's instrument, and Space on the
//!   track's first send (its mixer strip, [`crate::space_control`]).
//! - A knob with a lane of your own is left alone (your moves win), and so
//!   is an instrument knob the clip's instrument locks forbid
//!   ([`Clip::inst_refuses`]).
//! - The values are the knob's own, written against where it was when the
//!   try was made (`base`); what plays hears them from the knob where it
//!   is now ([`mpfree_seq::mpfree_moves::Lane::follow_knob`]).
//! - Over a lane of Continue's already there (an earlier Done), the new
//!   bars are written afresh and the bars before them kept.

use crate::sound::{Control, SafeControls, SoundMove};
use mpfree_seq::mpfree_moves::{By, Kind, Map, Point, Steps, Target};
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::Clip;
use mpfree_transport::time::Tick;

/// Clip ticks a move goes in ahead of its note, so the knob is there
/// when the note starts.
pub const AHEAD: Tick = 2;

/// A move's note is the earliest new note starting this close to its
/// step (half a 1/16): the playing moves a note a few ms either way.
const NEAR: Tick = mpfree_context::groove::STEP / 2;

/// The controls a lane of Continue's moves, in the order their lanes go
/// in.
const CONTROLS: [Control; 3] = [Control::Brightness, Control::Length, Control::Space];

/// The lane of Continue's for the knob `id` of the track's instrument.
pub fn target_of(id: u16) -> Target {
    Target::Track(RelAddr::new(OnTrack::Inst, id))
}

/// The lane of Continue's for `control`'s knob `id`: Space's is on the
/// track's mixer strip (its send), the rest on its instrument.
pub fn lane_of(control: Control, id: u16) -> Target {
    match control {
        Control::Space => Target::Track(RelAddr::new(OnTrack::Strip, id)),
        _ => target_of(id),
    }
}

/// `clip` with Continue's lanes for `moves` over its new bars (from
/// `new_from`, in clip ticks) on the knobs `safe` names; `may(id)` says
/// whether instrument knob `id` may take a lane on this clip. The lanes
/// that went in, in order.
pub fn put_sound(
    clip: &mut Clip,
    moves: &[SoundMove],
    safe: &SafeControls,
    new_from: Tick,
    may: impl Fn(u16) -> bool,
) -> Vec<Target> {
    let len = clip.length.max(1);
    let mut onsets: Vec<(Tick, Tick)> = clip
        .notes
        .iter()
        .filter(|n| n.on && n.tick >= new_from - NEAR && n.tick < len)
        .map(|n| (n.tick, n.len.max(1)))
        .collect();
    onsets.sort_unstable();
    // One onset a tick, with its longest note.
    onsets.dedup_by(|b, a| {
        let same = a.0 == b.0;
        if same {
            a.1 = a.1.max(b.1);
        }
        same
    });
    let mut out = Vec::new();
    for control in CONTROLS {
        let Some(k) = safe.get(control) else {
            continue;
        };
        let target = lane_of(control, k.id);
        let inst = control != Control::Space;
        if clip.moves.by(target) == Some(By::Hand) || (inst && !may(k.id)) {
            continue;
        }
        let map = Map {
            min: k.min,
            max: k.max,
            log: k.log && k.min > 0.0,
            steps: Steps::None,
        };
        let mine: Vec<&SoundMove> = moves
            .iter()
            .filter(|m| m.control == control && m.at >= new_from && m.at < len)
            .collect();
        let mut points: Vec<Point> = Vec::new();
        let mut put = |p: Point| {
            // One point a tick, in order: a later point on or before the
            // last takes its place (an end where the next move starts).
            while points.last().is_some_and(|q| q.tick >= p.tick) {
                points.pop();
            }
            points.push(p);
        };
        for (j, m) in mine.iter().enumerate() {
            let near = |t: Tick| (t - m.at).abs() <= NEAR;
            let note = onsets.iter().copied().find(|&(o, _)| near(o));
            let start = note.map_or(m.at, |(o, _)| o.min(m.at));
            let at = (start - AHEAD).clamp(0, len - 2);
            put(Point::step(at as u32, map.unit(k.value(m.amount))));
            // The knob goes back at the next new note, unless that one
            // has a move of its own; else at this note's end.
            let next = onsets.iter().copied().find(|&(o, _)| o > m.at + NEAR);
            let moved_next = next
                .is_some_and(|(nx, _)| mine.get(j + 1).is_some_and(|q| (q.at - nx).abs() <= NEAR));
            if moved_next {
                continue;
            }
            let end = match (next, note) {
                (Some((nx, _)), _) => nx - AHEAD,
                (None, Some((o, l))) => o + l,
                (None, None) => m.at + mpfree_context::groove::STEP,
            };
            put(Point::end(end.clamp(at + 1, len - 1) as u32));
        }
        if points.is_empty() {
            continue;
        }
        if write_lane(clip, target, map, points, k.base, new_from) {
            out.push(target);
        }
    }
    out
}

/// Continue's lane on `target`: `points` over the new bars, any lane of
/// Continue's there kept before them (its points from the first new one
/// on taken out), on and against `base`. False when the clip has no room
/// for it.
fn write_lane(
    clip: &mut Clip,
    target: Target,
    map: Map,
    points: Vec<Point>,
    base: f32,
    new_from: Tick,
) -> bool {
    let first = points.first().map_or(0, |p| p.tick);
    let cut = first.min(u32::try_from(new_from.max(0)).unwrap_or(u32::MAX));
    // The bars kept from before are heard from the knob where it is now
    // (`base`), as the new ones are written.
    let kept = clip.moves.lane(target).map(|old| {
        let mut old = old.clone();
        old.follow_knob(base);
        old
    });
    let mut all: Vec<Point> = match &kept {
        Some(old) => old
            .points
            .iter()
            .filter(|p| p.tick < cut)
            .map(|p| Point {
                // The knob's range is the same on the same instrument;
                // written in the new map all the same.
                u: if p.kind == Kind::End {
                    p.u
                } else {
                    map.unit(old.map.value(p.u))
                },
                ..*p
            })
            .collect(),
        None => Vec::new(),
    };
    all.extend(points);
    let len = u32::try_from(clip.length.max(1)).unwrap_or(u32::MAX);
    let old = clip.moves.remove(target);
    let fits = clip
        .moves
        .reserve(target, map, all.len())
        .and_then(|_| clip.moves.restore(target, map, 0, len, &all))
        .is_ok();
    if !fits {
        if let Some(l) = old {
            let _ = clip.moves.put_back(l);
        } else {
            clip.moves.remove(target);
        }
        return false;
    }
    clip.moves.set_by(target, By::Continue { on: true, base });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::{Safe, Swing};
    use mpfree_seq::Note;

    const BAR: Tick = 4 * 960;

    fn cutoff() -> Safe {
        Safe {
            id: 7,
            swing: Swing::Octaves(1.0),
            base: 1000.0,
            min: 20.0,
            max: 20_000.0,
            log: true,
        }
    }

    fn release() -> Safe {
        Safe {
            id: 9,
            swing: Swing::Times(3.0),
            base: 200.0,
            min: 1.0,
            max: 5000.0,
            log: true,
        }
    }

    /// A clip of 2 bars, yours the first, quarter notes in the second.
    fn clip() -> Clip {
        let mut c = Clip::new("riff", 2 * BAR);
        for k in 0..4 {
            c.notes.push(Note::new(k * 960, 480, 60, 100));
        }
        for k in 0..4 {
            c.notes
                .push(Note::new(BAR + k * 960, 480, 62 + k as u8, 100));
        }
        c
    }

    fn mv(control: Control, at: Tick, amount: f32) -> SoundMove {
        SoundMove {
            control,
            at,
            amount,
        }
    }

    fn safe() -> SafeControls {
        SafeControls {
            brightness: Some(cutoff()),
            length: Some(release()),
            space: None,
        }
    }

    #[test]
    fn each_move_steps_ahead_of_its_note_and_ends_at_the_next_without_one() {
        let mut c = clip();
        let moves = [
            mv(Control::Brightness, BAR, 0.5),
            mv(Control::Brightness, BAR + 960, 1.0),
            // None on the third note: the knob goes back there.
            mv(Control::Brightness, BAR + 3 * 960, -0.5),
        ];
        let got = put_sound(&mut c, &moves, &safe(), BAR, |_| true);
        assert_eq!(got, vec![target_of(7)]);
        let lane = c.moves.lane(target_of(7)).expect("a lane on the cutoff");
        assert_eq!(
            lane.by,
            By::Continue {
                on: true,
                base: 1000.0
            }
        );
        let k = cutoff();
        let ticks: Vec<(u32, Kind)> = lane.points.iter().map(|p| (p.tick, p.kind)).collect();
        let b = BAR as u32;
        assert_eq!(
            ticks,
            vec![
                (b - 2, Kind::Step),
                (b + 958, Kind::Step),
                (b + 2 * 960 - 2, Kind::End),
                (b + 3 * 960 - 2, Kind::Step),
                // The last note's end: the loop comes round on your knob.
                (b + 3 * 960 + 480, Kind::End),
            ]
        );
        let v = lane.map.value(lane.points[1].u);
        assert!((v - k.value(1.0)).abs() / v < 1e-3, "{v}");
        assert!((v - 2000.0).abs() < 2.0, "an octave up: {v}");
        // Your bars are left alone: free from the clip's start to the
        // first move.
        assert_eq!(lane.state_at(0.0), None);
        assert_eq!(lane.state_at(f64::from(b - 3)), None);
    }

    #[test]
    fn a_lane_of_your_own_on_the_knob_wins() {
        let mut c = clip();
        let (target, map) = (target_of(7), Map::linear(20.0, 20_000.0));
        c.moves
            .put(target, map, &[(10.0, 0.3, Kind::Step)], 2 * BAR as u32)
            .unwrap();
        let before = c.moves.lane(target).cloned();
        let moves = [
            mv(Control::Brightness, BAR, 0.5),
            mv(Control::Length, BAR + 3 * 960, 0.6),
        ];
        let got = put_sound(&mut c, &moves, &safe(), BAR, |_| true);
        assert_eq!(got, vec![target_of(9)], "the release only");
        assert_eq!(c.moves.lane(target).cloned(), before);
    }

    #[test]
    fn a_knob_the_clip_may_not_lock_gets_no_lane() {
        let mut c = clip();
        let moves = [mv(Control::Brightness, BAR, 0.5)];
        assert!(put_sound(&mut c, &moves, &safe(), BAR, |id| id != 7).is_empty());
        assert!(c.moves.is_empty());
    }

    #[test]
    fn continue_again_keeps_the_bars_before_and_writes_the_new_ones() {
        let mut c = clip();
        let first = [mv(Control::Brightness, BAR, 0.5)];
        put_sound(&mut c, &first, &safe(), BAR, |_| true);
        // Two more bars, carried on: the old lane stays before them.
        let mut longer = c.clone();
        longer.length = 4 * BAR;
        for k in 0..4 {
            longer
                .notes
                .push(Note::new(2 * BAR + k * 960, 480, 65, 100));
        }
        let mut b = cutoff();
        b.base = 1500.0;
        let s = SafeControls {
            brightness: Some(b),
            ..SafeControls::default()
        };
        let second = [mv(Control::Brightness, 2 * BAR + 960, -1.0)];
        put_sound(&mut longer, &second, &s, 2 * BAR, |_| true);
        let lane = longer.moves.lane(target_of(7)).unwrap();
        let ticks: Vec<u32> = lane.points.iter().map(|p| p.tick).collect();
        let bar = BAR as u32;
        assert_eq!(
            ticks,
            vec![bar - 2, bar + 958, 2 * bar + 958, 2 * bar + 2 * 960 - 2]
        );
        assert_eq!(
            lane.by,
            By::Continue {
                on: true,
                base: 1500.0
            }
        );
        // The bars before go with the knob, now at 1500: the same ratio
        // to it as they had to the knob they were written against.
        let old = c.moves.lane(target_of(7)).unwrap();
        let ratio = |l: &mpfree_seq::mpfree_moves::Lane| {
            let By::Continue { base, .. } = l.by else {
                panic!("Continue's");
            };
            l.map.value(l.points[0].u) / base
        };
        assert!(
            (ratio(lane) - ratio(old)).abs() < 1e-3,
            "{} {}",
            ratio(lane),
            ratio(old)
        );
    }

    #[test]
    fn no_moves_no_lane() {
        let mut c = clip();
        assert!(put_sound(&mut c, &[], &safe(), BAR, |_| true).is_empty());
        let none = SafeControls::default();
        let moves = [mv(Control::Brightness, BAR, 0.5)];
        assert!(put_sound(&mut c, &moves, &none, BAR, |_| true).is_empty());
        // Space has no send here.
        let space = [mv(Control::Space, BAR, 0.5)];
        assert!(put_sound(&mut c, &space, &safe(), BAR, |_| true).is_empty());
        assert!(c.moves.is_empty());
    }

    /// Space throws the note into the echo on the track's first send (its
    /// mixer strip, not the instrument): up to three times its own level
    /// at a full move, never past the send's top, and an instrument lock
    /// refused there doesn't stop it. A send that is off has no Space.
    #[test]
    fn space_moves_the_tracks_first_send() {
        use crate::sound::{space_control, SPACE_DB};
        let send = 6;
        let mut c = clip();
        let s = SafeControls {
            space: space_control(send, -12.0, -80.0, 6.0),
            ..safe()
        };
        let moves = [mv(Control::Space, BAR + 3 * 960, 1.0)];
        let got = put_sound(&mut c, &moves, &s, BAR, |_| false);
        let strip = lane_of(Control::Space, send);
        assert_eq!(got, vec![strip]);
        assert_eq!(strip, Target::Track(RelAddr::new(OnTrack::Strip, send)));
        let lane = c.moves.lane(strip).unwrap();
        let top = lane.map.value(lane.points[0].u);
        assert!((top - (-12.0 + SPACE_DB)).abs() < 1e-3, "{top}");
        assert_eq!(
            lane.by,
            By::Continue {
                on: true,
                base: -12.0
            }
        );
        // Near the top: held there.
        let high = space_control(send, 2.0, -80.0, 6.0).unwrap();
        assert_eq!(high.value(1.0), 6.0);
        // Off, or at the top already: no Space.
        assert_eq!(space_control(send, -80.0, -80.0, 6.0), None);
        assert_eq!(space_control(send, 6.0, -80.0, 6.0), None);
    }
}
