//! Loop tracks: the looper's loops in the Clips grid
//! (docs/dev/design/loop-tracks.md).
//!
//! A loop is a clip on a Loop track: a cell whose [`Clip::looped`] names
//! one of the looper's [`LOOPS`] loops, with no notes, as long as its loop.
//! The looper keeps the audio, the layers and their undo; the grid keeps
//! where each loop sits, so a loop launches, stops and sits in scenes as
//! any clip does. One loop plays per Loop track, as one clip plays per
//! track; loops on different Loop tracks play together.
//!
//! Which loops play follows the launches ([`playing`]): a loop plays while
//! one of its cells is launched on a Loop track that isn't muted. A loop
//! in no cell plays, as every loop did before Loop tracks. A loop recorded
//! from Play's strip or the Looper tab lands on a Loop track with nothing
//! playing ([`place`]); its cells follow its length ([`set_length`]) and
//! go when it is cleared ([`remove`]).

use mpfree_transport::time::Tick;

use crate::{Clip, DestId, Project, Track, LOOPS, MAX_CLIP_TICKS, MAX_TRACKS, MIN_CLIP_TICKS};

/// A loop's cell length for a loop of `ticks`: within a clip's range.
pub fn length(ticks: Tick) -> Tick {
    ticks.clamp(MIN_CLIP_TICKS, MAX_CLIP_TICKS)
}

/// What the looper says of a loop, for the grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopNow {
    /// It has a first pass: it plays or overdubs (an empty loop, one armed
    /// or recording its first pass has none).
    pub pass: bool,
    /// Its length, once it has a first pass.
    pub ticks: Tick,
    /// The song tick it started on.
    pub origin: Tick,
}

/// What the grid wants of the loops: which play, and the bar line each
/// one's change lands on (None: at once).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopPlan {
    pub on: [bool; LOOPS],
    pub at: [Option<Tick>; LOOPS],
}

/// The loops track `t`'s cells hold, one bit a loop (bit `n` for loop
/// `n`); 0 for a note track.
pub fn on_track(p: &Project, t: usize) -> u8 {
    p.tracks.get(t).map_or(0, |t| {
        t.slots
            .iter()
            .flatten()
            .filter_map(|c| c.looped)
            .fold(0, |m, n| m | bit(n))
    })
}

fn bit(n: u8) -> u8 {
    1u8.checked_shl(u32::from(n)).unwrap_or(0)
}

/// Loop `n`'s cells, as (track, slot).
pub fn cells(p: &Project, n: u8) -> impl Iterator<Item = (usize, usize)> + '_ {
    p.tracks.iter().enumerate().flat_map(move |(t, track)| {
        track
            .slots
            .iter()
            .enumerate()
            .filter(move |(_, c)| c.as_ref().is_some_and(|c| c.looped == Some(n)))
            .map(move |(s, _)| (t, s))
    })
}

/// Whether loop `n` is in a cell.
pub fn in_cell(p: &Project, n: u8) -> bool {
    cells(p, n).next().is_some()
}

/// Which loops play: a loop in a cell while one of its cells is launched
/// (`launched` gives each launch as track and slot) on a Loop track that
/// isn't muted; a loop in no cell, as before Loop tracks. Allocates
/// nothing.
pub fn playing(p: &Project, launched: impl Iterator<Item = (usize, usize)>) -> [bool; LOOPS] {
    let mut placed = 0u8;
    for t in p.tracks.iter().filter(|t| t.is_loops()) {
        for n in t.slots.iter().flatten().filter_map(|c| c.looped) {
            placed |= bit(n);
        }
    }
    let mut on = !placed;
    for (t, s) in launched {
        let Some(track) = p.tracks.get(t).filter(|t| t.is_loops() && !t.mute) else {
            continue;
        };
        if let Some(n) = track
            .slots
            .get(s)
            .and_then(Option::as_ref)
            .and_then(|c| c.looped)
        {
            on |= bit(n);
        }
    }
    std::array::from_fn(|n| on & bit(n as u8) != 0)
}

/// What song mode plays of the loops (loop-tracks.md part 5): the blocks
/// on Loop tracks the song plays (`free(t)`: no clip launched there and
/// not muted, as for a track's notes). Over `spans`, the period's tick
/// ranges in order (an empty range: the transport stopped there), each
/// loop is on if a block of it covers the song once the period is over,
/// and `at` is the tick in the period a block of it starts or ends on, if
/// one does, so the looper turns it there. Allocates nothing.
pub fn song_plan(
    p: &Project,
    spans: impl Iterator<Item = std::ops::Range<Tick>> + Clone,
    free: impl Fn(usize) -> bool,
) -> LoopPlan {
    let mut plan = LoopPlan::default();
    let Some(last) = spans.clone().last() else {
        return plan;
    };
    let now = last.end;
    let rolling = !last.is_empty();
    for b in &p.arrangement.placements {
        let Some(n) = b.clip(&p.tracks).and_then(|c| c.looped) else {
            continue;
        };
        let Some(i) = Some(usize::from(n)).filter(|&i| i < LOOPS) else {
            continue;
        };
        if !p.tracks.get(b.track).is_some_and(Track::is_loops) || !free(b.track) {
            continue;
        }
        // Rolling, the period played up to `now`: a block ending there is
        // still on (its end is the next period's to turn). Stopped at
        // `now`, the block that starts there plays when PLAY does.
        let covers = if rolling {
            b.start < now && now <= b.end()
        } else {
            b.start <= now && now < b.end()
        };
        plan.on[i] |= covers;
        for r in spans.clone() {
            if r.contains(&b.start) {
                plan.at[i] = Some(b.start);
            } else if r.contains(&b.end()) && plan.at[i].is_none() {
                plan.at[i] = Some(b.end());
            }
        }
    }
    plan
}

/// Loop `n`'s cells take its new length. How many changed.
pub fn set_length(p: &mut Project, n: u8, ticks: Tick) -> usize {
    let len = length(ticks);
    let mut changed = 0;
    for c in p
        .tracks
        .iter_mut()
        .flat_map(|t| t.slots.iter_mut().flatten())
    {
        if c.looped == Some(n) && c.length != len {
            c.length = len;
            changed += 1;
        }
    }
    changed
}

/// Loop `n`'s cells empty (it was cleared). How many went.
pub fn remove(p: &mut Project, n: u8) -> usize {
    let mut gone = 0;
    for s in p.tracks.iter_mut().flat_map(|t| t.slots.iter_mut()) {
        if s.as_ref().is_some_and(|c| c.looped == Some(n)) {
            *s = None;
            gone += 1;
        }
    }
    gone
}

/// A new Loop track's name: "Loops A", "Loops B" ... the first letter no
/// track has taken; past Z, "Loops 27" and on.
pub fn new_name(p: &Project) -> String {
    let taken = |name: &str| p.tracks.iter().any(|t| t.name == name);
    (b'A'..=b'Z')
        .map(|c| format!("Loops {}", c as char))
        .chain((27..).map(|k| format!("Loops {k}")))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

/// Where a loop recorded from Play's strip or the Looper tab lands: in
/// row `row` of the first Loop track with nothing playing or cued
/// (`busy`) whose cell in that row is empty, or else on a new Loop track
/// at the end. Its cell is put there; None (and nothing changed) when
/// the project has [`MAX_TRACKS`] tracks or `row` is past the grid.
pub fn place(
    p: &mut Project,
    n: u8,
    ticks: Tick,
    row: usize,
    busy: impl Fn(usize) -> bool,
) -> Option<(usize, usize)> {
    if row >= crate::SLOTS || usize::from(n) >= LOOPS {
        return None;
    }
    let free = p
        .tracks
        .iter()
        .enumerate()
        .position(|(t, track)| track.is_loops() && !busy(t) && track.slots[row].is_none());
    let t = match free {
        Some(t) => t,
        None if p.tracks.len() < MAX_TRACKS => {
            let name = new_name(p);
            // The first destination no track has, as a new note track
            // gets (`Takes::add_track`), though nothing plays into it.
            let dest = (0..=MAX_TRACKS as DestId)
                .find(|d| p.tracks.iter().all(|t| t.dest != *d))
                .unwrap_or(p.tracks.len() as DestId);
            p.tracks.push(Track::new_loops(&name, dest));
            p.tracks.len() - 1
        }
        None => return None,
    };
    p.tracks[t].slots[row] = Some(Clip::new_loop(n, ticks));
    Some((t, row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LoopInput, ModelError, Note, TrackKind, SLOTS};

    const BAR: Tick = 3840;

    fn project() -> Project {
        let mut p = Project::new();
        for i in 0..2 {
            p.tracks.push(Track::new(&format!("Track {}", i + 1), i));
        }
        p
    }

    /// Song mode: a Loop track's blocks turn their loops on for their
    /// length, on their own ticks; a launch on the track, or its mute,
    /// keeps the song off it (loop-tracks.md part 5).
    #[test]
    fn the_songs_blocks_turn_their_loops_on_and_off_on_their_ticks() {
        use crate::arrange::Placement;
        let mut p = project();
        let mut a = Track::new_loops("Loops A", 2);
        a.slots[0] = Some(Clip::new_loop(0, 2 * BAR));
        a.slots[1] = Some(Clip::new_loop(1, BAR));
        p.tracks.push(a);
        // Loop 1 from bar 2 for two bars, then loop 2 for a bar.
        p.arrangement.placements = vec![
            Placement::linked(2, 0, BAR, 2 * BAR),
            Placement::linked(2, 1, 3 * BAR, BAR),
        ];
        let free = |_| true;
        let at = |from: Tick, to: Tick| song_plan(&p, std::iter::once(from..to), free);
        // Bar 1: nothing yet.
        assert_eq!(at(0, 100), LoopPlan::default());
        // The period over bar 2's line: loop 1 on, from that tick.
        let on = at(BAR - 50, BAR + 50);
        assert_eq!((on.on[0], on.at[0]), (true, Some(BAR)));
        // Inside the block: on, with no tick to turn on.
        let mid = at(2 * BAR, 2 * BAR + 100);
        assert_eq!((mid.on[0], mid.at[0], mid.on[1]), (true, None, false));
        // A period ending on the block's end: still on, its end the next
        // period's.
        assert!(at(3 * BAR - 100, 3 * BAR).on[0]);
        // Over bar 4's line: loop 1 off and loop 2 on, both from it.
        let swap = at(3 * BAR - 50, 3 * BAR + 50);
        assert_eq!((swap.on[0], swap.at[0]), (false, Some(3 * BAR)));
        assert_eq!((swap.on[1], swap.at[1]), (true, Some(3 * BAR)));
        // Stopped at bar 2's line: the block there plays when PLAY does.
        assert!(at(BAR, BAR).on[0]);
        assert!(!at(3 * BAR, 3 * BAR).on[0]);
        // Two spans (a jump back to bar 1): the last one says what plays.
        let jumped = song_plan(&p, [2 * BAR..2 * BAR + 50, 0..50].into_iter(), free);
        assert!(!jumped.on[0]);
        // A launch on the track (or its mute) keeps the song off it.
        assert!(!song_plan(&p, std::iter::once(2 * BAR..2 * BAR + 9), |t| t != 2).on[0]);
    }

    #[test]
    fn a_loop_cell_is_a_clip_on_a_loop_track_with_no_notes() {
        let mut p = project();
        let c = Clip::new_loop(1, 2 * BAR);
        assert_eq!(
            (c.name.as_str(), c.length, c.looped),
            ("Loop 2", 2 * BAR, Some(1))
        );
        assert_eq!(c.check(), Ok(()));
        // A loop past the looper's, or a cell with a note, doesn't hold.
        assert_eq!(Clip::new_loop(8, BAR).check(), Err(ModelError::BadLoop));
        let mut noted = Clip::new_loop(0, BAR);
        noted.notes.push(Note::new(0, 240, 36, 100));
        assert_eq!(noted.check(), Err(ModelError::BadLoop));
        // A cell sits on a Loop track only, and a Loop track holds cells only.
        p.tracks[0].slots[0] = Some(Clip::new_loop(0, BAR));
        assert_eq!(p.check(), Err(ModelError::BadLoop));
        p.tracks[0].slots[0] = None;
        p.tracks.push(Track::new_loops("Loops A", 2));
        p.tracks[2].slots[0] = Some(Clip::new("Beat", BAR));
        assert_eq!(p.check(), Err(ModelError::BadLoop));
        p.tracks[2].slots[0] = Some(Clip::new_loop(0, BAR));
        assert_eq!(p.check(), Ok(()));
        assert_eq!(p.tracks[2].kind, TrackKind::Loops(LoopInput::Pads));
        assert_eq!(
            p.tracks[2].loops().map(LoopInput::name),
            Some("Pads + synth")
        );
        assert!(!p.tracks[0].is_loops());
        // Lengths keep within a clip's.
        assert_eq!(length(0), MIN_CLIP_TICKS);
        assert_eq!(length(MAX_CLIP_TICKS * 2), MAX_CLIP_TICKS);
    }

    #[test]
    fn a_recorded_loop_lands_on_a_loop_track_with_nothing_playing() {
        let mut p = project();
        // No Loop track yet: one is added, Loops A, at the end.
        assert_eq!(place(&mut p, 0, 2 * BAR, 0, |_| false), Some((2, 0)));
        assert_eq!(p.tracks[2].name, "Loops A");
        assert_eq!(p.tracks[2].dest, 2);
        // Loops A plays loop 1, so loop 2 goes on a new Loops B.
        let busy = |t: usize| t == 2;
        assert_eq!(place(&mut p, 1, BAR, 0, busy), Some((3, 0)));
        assert_eq!(p.tracks[3].name, "Loops B");
        // In the row asked for, on the first free Loop track.
        assert_eq!(place(&mut p, 2, BAR, 4, |_| false), Some((2, 4)));
        // A track whose cell in that row is taken is passed over.
        assert_eq!(place(&mut p, 3, BAR, 4, |_| false), Some((3, 4)));
        assert_eq!(p.check(), Ok(()));
        let all: Vec<_> = (0..4).map(|n| cells(&p, n).collect::<Vec<_>>()).collect();
        assert_eq!(
            all,
            [vec![(2, 0)], vec![(3, 0)], vec![(2, 4)], vec![(3, 4)]]
        );
        assert_eq!(on_track(&p, 2), 0b0101);
        assert_eq!(on_track(&p, 0), 0);
        // Names go on past letters taken.
        p.tracks[0].name = "Loops C".into();
        assert_eq!(new_name(&p), "Loops D");
        // Past the grid's rows, or with the project full, it lands nowhere.
        assert_eq!(place(&mut p, 4, BAR, SLOTS, |_| false), None);
        while p.tracks.len() < MAX_TRACKS {
            p.tracks.push(Track::new("x", 0));
        }
        assert_eq!(place(&mut p, 4, BAR, 0, |_| true), None);
        assert!(!in_cell(&p, 4));
    }

    #[test]
    fn a_loop_plays_while_its_cell_is_launched() {
        let mut p = project();
        place(&mut p, 0, BAR, 0, |_| false);
        place(&mut p, 1, BAR, 1, |_| false);
        // Loops A holds loops 1 and 2; loop 3 is in no cell.
        let want = |p: &Project, l: &[(usize, usize)]| playing(p, l.iter().copied());
        let none = want(&p, &[]);
        assert_eq!(&none[..3], &[false, false, true]);
        assert!(none[3..].iter().all(|&on| on), "loops in no cell play");
        let first = want(&p, &[(2, 0)]);
        assert_eq!(&first[..3], &[true, false, true]);
        let second = want(&p, &[(2, 1)]);
        assert_eq!(&second[..2], &[false, true]);
        // A note track's launch has nothing to do with loops.
        assert_eq!(want(&p, &[(0, 0)])[..2], [false, false]);
        // A muted Loop track's loop is off.
        p.tracks[2].mute = true;
        assert_eq!(want(&p, &[(2, 0)])[..2], [false, false]);
        p.tracks[2].mute = false;
        // The same loop in two rows plays from either.
        p.tracks[2].slots[5] = Some(Clip::new_loop(0, BAR));
        assert!(want(&p, &[(2, 5)])[0]);
        // Its length follows the loop's; clearing it empties its cells.
        assert_eq!(set_length(&mut p, 0, 2 * BAR), 2);
        assert_eq!(set_length(&mut p, 0, 2 * BAR), 0);
        assert_eq!(
            p.tracks[2].slots[5].as_ref().map(|c| c.length),
            Some(2 * BAR)
        );
        assert_eq!(remove(&mut p, 0), 2);
        assert!(!in_cell(&p, 0) && in_cell(&p, 1));
        assert!(want(&p, &[])[0], "a loop in no cell plays");
    }
}
