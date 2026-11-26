//! The looper's loops in the Clips grid, on audio tracks
//! (docs/dev/design/loops-on-audio-tracks.md).
//!
//! A loop cell is a clip on an audio track whose [`Clip::loop_clip`] names
//! one of the looper's loops by its lasting [`LoopId`], with no notes, as
//! long as its loop. The looper keeps the audio, the layers and their
//! undo; the grid keeps where each loop sits, so a loop launches, stops
//! and sits in scenes as any clip does. A loop recorded with no cell lands
//! on an audio track ([`place_audio`]); its cells follow its length
//! ([`set_audio_length`]) and go when it is cleared ([`remove_audio`]).

use mpfree_transport::time::Tick;

use crate::{Clip, ClipId, LoopId, Project, Track, MAX_CLIP_TICKS, MAX_TRACKS, MIN_CLIP_TICKS};

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
    /// It keeps time with the looper's free clock (Sync: Free): its cell
    /// on an audio track is a free cell (the audio tracks rethink's phase
    /// 4e.4).
    pub free: bool,
}

/// Loop `id`'s cells on audio tracks (phase 4: [`Clip::loop_clip`]), as
/// (track, slot).
pub fn audio_cells(p: &Project, id: LoopId) -> impl Iterator<Item = (usize, usize)> + '_ {
    p.tracks.iter().enumerate().flat_map(move |(t, track)| {
        track
            .slots
            .iter()
            .filter(move |(_, c)| c.loop_id() == Some(id))
            .map(move |(s, _)| (t, s))
    })
}

/// Loop `id`'s cells on audio tracks take its new length, and so do the
/// song's own clips of it (a cell made unique, or deleted while the song plays it: phase
/// 4e.3), so its blocks' passes follow the loop. The blocks themselves
/// never move or change length. How many changed.
pub fn set_audio_length(p: &mut Project, id: LoopId, ticks: Tick) -> usize {
    let len = length(ticks);
    let mut changed = 0;
    for c in p.tracks.iter_mut().flat_map(|t| t.slots.clips_mut()) {
        if c.loop_id() == Some(id) && c.length != len {
            c.length = len;
            changed += 1;
        }
    }
    let own: Vec<ClipId> = p
        .arrangement
        .clips
        .iter()
        .filter(|c| c.loop_id() == Some(id) && c.length != len)
        .map(|c| c.id)
        .collect();
    for k in own {
        if let Some(c) = p.arrangement.clips.get_mut(k) {
            c.length = len;
            changed += 1;
        }
    }
    changed
}

/// Loop `id`'s cells on audio tracks empty (it was cleared). How many
/// went.
pub fn remove_audio(p: &mut Project, id: LoopId) -> usize {
    p.tracks
        .iter_mut()
        .map(|t| t.slots.empty_where(|c| c.loop_id() == Some(id)))
        .sum()
}

/// A new audio track's name: "Audio 1", "Audio 2" ... the first number
/// no track has taken ("+ Track"'s Audio track, and a loop landing on a
/// new audio track: [`place_audio`]).
pub fn audio_name(p: &Project) -> String {
    let taken = |name: &str| p.tracks.iter().any(|t| t.name == name);
    (1..)
        .map(|k| format!("Audio {k}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

/// Where a loop lands on an audio track ([`place_audio`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landed {
    pub track: usize,
    pub slot: usize,
    /// On an audio track made for it.
    pub new_track: bool,
}

/// Where a loop recorded with no cell lands (the audio tracks rethink's
/// phase 4e.4: from the Looper tab or with
/// Capture, no cell armed for it), `id` its lasting id: in row `row` of
/// `armed` (the audio track armed by its own arm) if that cell is empty,
/// whatever the track plays; else in row `row` of the first other audio
/// track with nothing playing or cued (`busy`) whose cell there is empty;
/// else in the first empty cell of such a track; else on a new audio track
/// at the end, in row `row`, while `room` (an engine track is free) and
/// the project has fewer than [`MAX_TRACKS`] tracks. Its cell, a loop clip
/// named `name` of `ticks`, is put there; None, and nothing changed, when
/// there is nowhere or `row` is past the grid.
#[allow(clippy::too_many_arguments)]
pub fn place_audio(
    p: &mut Project,
    id: LoopId,
    name: &str,
    ticks: Tick,
    row: usize,
    armed: Option<usize>,
    busy: impl Fn(usize) -> bool,
    room: bool,
) -> Option<Landed> {
    if row >= crate::ROWS || id.0 == 0 {
        return None;
    }
    let audio = |t: usize| p.tracks.get(t).is_some_and(Track::is_audio);
    let free = |t: usize, s: usize| audio(t) && p.tracks[t].slots[s].is_none();
    let other = |t: usize| audio(t) && Some(t) != armed && !busy(t);
    let at = armed
        .filter(|&t| free(t, row))
        .map(|t| (t, row))
        .or_else(|| {
            (0..p.tracks.len())
                .find(|&t| other(t) && free(t, row))
                .map(|t| (t, row))
        })
        .or_else(|| {
            (0..p.tracks.len())
                .filter(|&t| other(t))
                .find_map(|t| p.tracks[t].slots.first_free().map(|s| (t, s)))
        });
    let (track, slot, new_track) = match at {
        Some((t, s)) => (t, s, false),
        None if room && p.tracks.len() < MAX_TRACKS => {
            let mut track = Track::new_audio(&audio_name(p), 0);
            p.give_track_id(&mut track);
            p.tracks.push(track);
            (p.tracks.len() - 1, row, true)
        }
        None => return None,
    };
    let clip = Clip::new_audio_loop(id, name, ticks);
    if p.put_new_clip(track, slot, clip).is_err() {
        if new_track {
            p.tracks.pop();
        }
        return None;
    }
    Some(Landed {
        track,
        slot,
        new_track,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Tick = 3840;

    fn project() -> Project {
        let mut p = Project::new();
        for i in 0..2 {
            p.tracks.push(Track::new(&format!("Track {}", i + 1), i));
        }
        p
    }

    /// Phase 4e.3: a loop made shorter on an audio track: its cells and
    /// the song's own clips of it take the new length, so its blocks'
    /// passes come sooner, and its blocks stay where they were, as long.
    /// Another loop's clips are left alone.
    #[test]
    fn a_loops_cells_and_the_songs_own_clips_of_it_follow_its_length_and_its_blocks_stay() {
        use crate::arrange::Placement;
        let mut p = project();
        let mut a = Track::new_audio("Vocal", 2);
        p.give_track_id(&mut a);
        p.tracks.push(a);
        let (id, other) = (LoopId(0xa), LoopId(0xb));
        p.tracks[2].slots[0] = Some(Clip::new_audio_loop(id, "Vox", 4 * BAR));
        p.tracks[2].slots[1] = Some(Clip::new_audio_loop(other, "Hum", 4 * BAR));
        p.give_clip_ids();
        let mut own = Clip::new_audio_loop(id, "Vox", 4 * BAR);
        own.id = p.new_clip_id();
        let own_id = own.id;
        p.arrangement.clips.keep(own).unwrap();
        let cell = p.tracks[2].slots[0].as_ref().unwrap().id;
        let mut late = Placement::new(2, own_id, 8 * BAR, 8 * BAR);
        late.offset = BAR;
        p.arrangement.placements = vec![Placement::new(2, cell, 0, 8 * BAR), late];
        p.check().unwrap();
        let blocks = p.arrangement.placements.clone();
        assert_eq!(set_audio_length(&mut p, id, 2 * BAR), 2);
        let len = |c: Option<&Clip>| c.map(|c| c.length);
        assert_eq!(len(p.clip(2, 0)), Some(2 * BAR));
        assert_eq!(len(p.arrangement.clips.get(own_id)), Some(2 * BAR));
        assert_eq!(len(p.clip(2, 1)), Some(4 * BAR), "another loop");
        assert_eq!(p.arrangement.placements, blocks, "the blocks stay");
        assert_eq!(set_audio_length(&mut p, id, 2 * BAR), 0, "once");
        p.check().unwrap();
    }

    /// Phase 4e.4: a loop with no cell lands in the armed audio track's
    /// cell in the row playing, whatever it plays; else in that row of
    /// the first other audio track playing nothing; else in such a
    /// track's first empty cell; else on a new audio track, while an
    /// engine track is free. Never on a note track or a busy one.
    #[test]
    fn a_loop_with_no_cell_lands_on_the_armed_audio_track_then_a_free_one_then_a_new_one() {
        let mut p = project();
        for name in ["Vocal", "Guitar"] {
            let mut t = Track::new_audio(name, 2);
            p.give_track_id(&mut t);
            p.tracks.push(t);
        }
        let (vocal, guitar) = (2, 3);
        let idle = |_: usize| false;
        let at = |l: Option<Landed>| l.map(|l| (l.track, l.slot, l.new_track));
        // Armed: its cell in the row playing, even while it plays.
        let got = place_audio(
            &mut p,
            LoopId(1),
            "Loop 1",
            BAR,
            2,
            Some(guitar),
            |t| t == guitar,
            false,
        );
        assert_eq!(at(got), Some((guitar, 2, false)));
        let c = p.clip(guitar, 2).unwrap();
        assert_eq!(
            (c.loop_id(), c.name.as_str(), c.length),
            (Some(LoopId(1)), "Loop 1", BAR)
        );
        // Its cell taken: the first other audio track, in that row.
        let got = place_audio(
            &mut p,
            LoopId(2),
            "Loop 2",
            BAR,
            2,
            Some(guitar),
            idle,
            false,
        );
        assert_eq!(at(got), Some((vocal, 2, false)));
        // Vocal busy: Guitar is armed, so not "another"; none left in
        // row 2, so Vocal's first empty cell is skipped too (busy).
        let got = place_audio(
            &mut p,
            LoopId(3),
            "Loop 3",
            BAR,
            2,
            Some(guitar),
            |t| t == vocal,
            false,
        );
        assert_eq!(got, None, "no room for a new track");
        // Not busy: Vocal's first empty cell, row 1.
        let got = place_audio(
            &mut p,
            LoopId(3),
            "Loop 3",
            BAR,
            2,
            Some(guitar),
            idle,
            false,
        );
        assert_eq!(at(got), Some((vocal, 0, false)));
        // Every audio track busy: a new one, in the row playing.
        let tracks = p.tracks.len();
        let got = place_audio(&mut p, LoopId(4), "Loop 4", BAR, 2, None, |_| true, true);
        assert_eq!(at(got), Some((tracks, 2, true)));
        assert!(p.tracks[tracks].is_audio());
        assert_eq!(p.tracks[tracks].name, "Audio 1");
        assert_ne!(p.tracks[tracks].id, p.tracks[vocal].id);
        // No id, or past the grid: nowhere, nothing changed.
        let before = p.clone();
        assert_eq!(
            place_audio(&mut p, LoopId(0), "x", BAR, 0, None, idle, true),
            None
        );
        assert_eq!(
            place_audio(&mut p, LoopId(5), "x", BAR, crate::ROWS, None, idle, true),
            None
        );
        assert_eq!(p, before);
        p.check().unwrap();
        assert!(loops_named_on_audio_only(&p));
    }

    /// Every loop cell of `p` is on an audio track.
    fn loops_named_on_audio_only(p: &Project) -> bool {
        p.tracks
            .iter()
            .all(|t| t.is_audio() || t.slots.clips().all(|c| c.loop_id().is_none()))
    }

    #[test]
    fn a_loop_cells_length_keeps_within_a_clips() {
        assert_eq!(length(0), MIN_CLIP_TICKS);
        assert_eq!(length(2 * BAR), 2 * BAR);
        assert_eq!(length(MAX_CLIP_TICKS * 2), MAX_CLIP_TICKS);
    }
}
