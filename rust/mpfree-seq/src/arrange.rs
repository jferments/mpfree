//! The arrangement (roadmap T13): clips on a timeline.
//!
//! A [`Placement`] puts a clip on a track from a song tick for a length.
//! It is **linked** to a slot of its track ([`Source::Slot`]): an edit to
//! the slot's clip shows in every placement of it, the way a pattern plays
//! wherever the song uses it. [`Arrangement::make_unique`] gives a
//! placement its own copy ([`Source::Own`]) to change on its own. A
//! placement longer than its clip loops it; `offset` starts it part way in.
//! [`Marker`]s name places in the song. A placement linked to a slot that
//! is emptied (an undone capture, say) stays and plays nothing until the
//! slot has a clip again; [`Arrangement::unlink`] keeps a copy instead.
//!
//! Scenes and the arrangement convert both ways:
//!
//! - [`scenes_to_song`]: rows one after another, each as long as
//!   [`scene_length`] (its follow's bars, else its longest clip in whole
//!   bars), as linked placements, a marker on each named after the scene.
//! - [`song_to_scenes`]: each section of the song (between the places
//!   anything starts or ends, and the markers) becomes a row in a run of
//!   empty rows, its clips copied in, chained by follow actions (Next after
//!   the section's bars; the last one stops), so launching the first row
//!   plays the song.
//!
//! Plain values checked by [`Arrangement::check`]; `mpfree-play` plays
//! them (song mode) and `mpfree-perform` records into them (T11).

use crate::{Clip, Follow, FollowAction, ModelError, Project, Track, SCENES, SLOTS};
use mpfree_transport::time::Tick;

/// Placements in a song.
pub const MAX_PLACEMENTS: usize = 4096;
/// Markers in a song.
pub const MAX_MARKERS: usize = 256;

/// Where a placement's clip comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// The clip in this slot of the placement's track.
    Slot(usize),
    /// Its own copy (made unique).
    Own(Box<Clip>),
}

/// A clip on the timeline: see the module notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub track: usize,
    /// The song tick it starts on.
    pub start: Tick,
    /// How long it plays, in song ticks.
    pub length: Tick,
    /// Where in the clip it starts, in the clip's own ticks (0: its top).
    pub offset: Tick,
    pub source: Source,
}

impl Placement {
    /// Slot `slot` of `track` from `start` for `length` song ticks.
    pub fn linked(track: usize, slot: usize, start: Tick, length: Tick) -> Placement {
        Placement {
            track,
            start,
            length,
            offset: 0,
            source: Source::Slot(slot),
        }
    }

    pub fn end(&self) -> Tick {
        self.start + self.length
    }

    pub fn contains(&self, tick: Tick) -> bool {
        (self.start..self.end()).contains(&tick)
    }

    /// The clip it plays, linked or its own.
    pub fn clip<'a>(&'a self, tracks: &'a [Track]) -> Option<&'a Clip> {
        match &self.source {
            Source::Slot(s) => tracks.get(self.track)?.slots.get(*s)?.as_ref(),
            Source::Own(c) => Some(c),
        }
    }

    /// The song tick its clip's tick 0 falls on in the pass it starts in:
    /// `start` moved back by `offset` at the clip's speed (to a whole tick).
    pub fn origin(&self, clip: &Clip) -> Tick {
        let back = clip
            .speed
            .to_song(self.offset.rem_euclid(clip.length.max(1)) as f64);
        self.start - back.round() as Tick
    }

    /// The slot it is linked to.
    pub fn slot(&self) -> Option<usize> {
        match self.source {
            Source::Slot(s) => Some(s),
            Source::Own(_) => None,
        }
    }
}

/// A named place in the song.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub at: Tick,
    pub name: String,
}

/// The song: placements sorted by track, then start (none overlapping on a
/// track), and markers sorted by tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arrangement {
    pub placements: Vec<Placement>,
    pub markers: Vec<Marker>,
}

impl Arrangement {
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// Where the last placement ends (0 for none).
    pub fn end(&self) -> Tick {
        self.placements
            .iter()
            .map(Placement::end)
            .max()
            .unwrap_or(0)
    }

    pub fn check(&self, tracks: &[Track]) -> Result<(), ModelError> {
        if self.placements.len() > MAX_PLACEMENTS || self.markers.len() > MAX_MARKERS {
            return Err(ModelError::BadPlacement);
        }
        for (i, p) in self.placements.iter().enumerate() {
            if p.length <= 0 || p.start < 0 || p.offset < 0 || p.track >= tracks.len() {
                return Err(ModelError::BadPlacement);
            }
            match &p.source {
                Source::Slot(s) if *s >= SLOTS => return Err(ModelError::BadPlacement),
                Source::Own(c) => c.check()?,
                Source::Slot(_) => {}
            }
            if let Some(prev) = i.checked_sub(1).map(|j| &self.placements[j]) {
                let order = (prev.track, prev.start) < (p.track, p.start);
                if !order || (prev.track == p.track && prev.end() > p.start) {
                    return Err(ModelError::BadPlacement);
                }
            }
        }
        if self.markers.iter().any(|m| m.at < 0)
            || self.markers.windows(2).any(|w| w[0].at > w[1].at)
        {
            return Err(ModelError::BadPlacement);
        }
        Ok(())
    }

    /// Put `p` in its place. Refused when it overlaps another placement on
    /// its track, or is empty. Returns its index.
    pub fn place(&mut self, p: Placement) -> Result<usize, ModelError> {
        if p.length <= 0 || p.start < 0 || p.offset < 0 || self.placements.len() >= MAX_PLACEMENTS {
            return Err(ModelError::BadPlacement);
        }
        let i = self
            .placements
            .partition_point(|q| (q.track, q.start) < (p.track, p.start));
        let before = i.checked_sub(1).map(|j| &self.placements[j]);
        let after = self.placements.get(i);
        if before.is_some_and(|q| q.track == p.track && q.end() > p.start)
            || after.is_some_and(|q| q.track == p.track && q.start < p.end())
        {
            return Err(ModelError::BadPlacement);
        }
        self.placements.insert(i, p);
        Ok(i)
    }

    pub fn remove(&mut self, i: usize) -> Option<Placement> {
        (i < self.placements.len()).then(|| self.placements.remove(i))
    }

    /// The placements on `track`, in order.
    pub fn on_track(&self, track: usize) -> &[Placement] {
        let a = self.placements.partition_point(|p| p.track < track);
        let b = self.placements.partition_point(|p| p.track <= track);
        &self.placements[a..b]
    }

    /// The placement playing on `track` at `tick`.
    pub fn at(&self, track: usize, tick: Tick) -> Option<usize> {
        let a = self.placements.partition_point(|p| p.track < track);
        self.placements[a..]
            .iter()
            .take_while(|p| p.track == track && p.start <= tick)
            .position(|p| p.contains(tick))
            .map(|k| a + k)
    }

    /// Give placement `i` its own copy of its clip.
    pub fn make_unique(&mut self, tracks: &[Track], i: usize) -> Result<(), ModelError> {
        let p = self.placements.get(i).ok_or(ModelError::BadPlacement)?;
        if let Source::Slot(_) = p.source {
            let clip = p.clip(tracks).ok_or(ModelError::BadPlacement)?.clone();
            self.placements[i].source = Source::Own(Box::new(clip));
        }
        Ok(())
    }

    /// Before slot `slot` of `track` is emptied: every placement linked to
    /// it gets its own copy, so the song still plays it. How many.
    pub fn unlink(&mut self, tracks: &[Track], track: usize, slot: usize) -> usize {
        let mut n = 0;
        for i in 0..self.placements.len() {
            let p = &self.placements[i];
            if p.track == track && p.slot() == Some(slot) && self.make_unique(tracks, i).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Add a marker in its place.
    pub fn mark(&mut self, at: Tick, name: &str) -> Result<(), ModelError> {
        if at < 0 || self.markers.len() >= MAX_MARKERS {
            return Err(ModelError::BadPlacement);
        }
        let i = self.markers.partition_point(|m| m.at <= at);
        self.markers.insert(
            i,
            Marker {
                at,
                name: name.to_string(),
            },
        );
        Ok(())
    }
}

/// How long row `row` plays in a song: its follow's bars, else its longest
/// clip in whole bars of `bar` ticks (one bar at least).
pub fn scene_length(project: &Project, row: usize, bar: Tick) -> Tick {
    let bar = bar.max(1);
    if let Some(f) = project.scene(row).and_then(|s| s.follow) {
        return Tick::from(f.bars) * bar;
    }
    let longest = project
        .tracks
        .iter()
        .filter_map(|t| t.slots.get(row)?.as_ref())
        .map(|c| c.speed.to_song(c.length as f64).ceil() as Tick)
        .max()
        .unwrap_or(bar);
    (longest + bar - 1) / bar * bar
}

/// Rows one after another from `at`, each for its length in song ticks,
/// as linked placements with a marker each: see the module notes.
pub fn scenes_to_song(
    project: &Project,
    rows: &[(usize, Tick)],
    at: Tick,
) -> Result<Arrangement, ModelError> {
    let mut song = Arrangement::default();
    let mut t = at;
    for &(row, length) in rows {
        if row >= SCENES || length <= 0 {
            return Err(ModelError::BadPlacement);
        }
        for (track, tr) in project.tracks.iter().enumerate() {
            if tr.slots[row].is_some() {
                song.place(Placement::linked(track, row, t, length))?;
            }
        }
        let name = project
            .scene(row)
            .map(|s| s.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Scene {}", row + 1));
        song.mark(t, &name)?;
        t += length;
    }
    Ok(song)
}

/// The song's sections: where anything starts or ends and the markers, in
/// order, merged where the same placements play on, without the silent
/// ones. Each is (start, end, the placement playing on each track).
pub fn sections(project: &Project) -> Vec<(Tick, Tick, Vec<Option<usize>>)> {
    let song = &project.arrangement;
    let mut cuts: Vec<Tick> = song
        .placements
        .iter()
        .flat_map(|p| [p.start, p.end()])
        .chain(song.markers.iter().map(|m| m.at))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let marked = |t: Tick| song.markers.iter().any(|m| m.at == t);
    let mut out: Vec<(Tick, Tick, Vec<Option<usize>>)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let playing: Vec<Option<usize>> =
            (0..project.tracks.len()).map(|t| song.at(t, a)).collect();
        if playing.iter().all(Option::is_none) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.1 == a && last.2 == playing && !marked(a) => last.1 = b,
            _ => out.push((a, b, playing)),
        }
    }
    out
}

/// Each section of the song into a row: see the module notes. Uses the
/// first run of empty rows long enough; returns the rows, or `NoRoom` (and
/// changes nothing) when there is none.
pub fn song_to_scenes(project: &mut Project, bar: Tick) -> Result<Vec<usize>, ModelError> {
    let bar = bar.max(1);
    let parts = sections(project);
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    let empty = |p: &Project, r: usize| {
        !p.row_filled(r)
            && p.scene(r)
                .is_none_or(|s| s.follow.is_none() && s.values.is_empty())
    };
    let n = parts.len();
    let first = (0..SCENES.saturating_sub(n - 1))
        .find(|&r| (r..r + n).all(|k| empty(project, k)))
        .ok_or(ModelError::NoRoom)?;
    let mut p = project.clone();
    for (k, (a, b, playing)) in parts.iter().enumerate() {
        let row = first + k;
        for (track, at) in playing.iter().enumerate() {
            let Some(i) = at else { continue };
            // A placement of an emptied slot plays nothing: nothing to copy.
            if let Some(clip) = project.arrangement.placements[*i].clip(&project.tracks) {
                p.tracks[track].slots[row] = Some(clip.clone());
            }
        }
        let bars = ((b - a + bar - 1) / bar).clamp(1, i64::from(crate::scenes::MAX_FOLLOW_BARS));
        let name = project
            .arrangement
            .markers
            .iter()
            .rev()
            .find(|m| m.at <= *a)
            .map(|m| m.name.clone())
            .unwrap_or_default();
        let scene = p.scene_mut(row).ok_or(ModelError::NoRoom)?;
        scene.name = name;
        let action = if k + 1 == n {
            FollowAction::Stop
        } else {
            FollowAction::Next
        };
        scene.follow = Some(Follow::after(bars as u16, action));
    }
    p.check()?;
    *project = p;
    Ok((first..first + n).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Tick = 3840;

    fn project() -> Project {
        let mut p = Project::new();
        for name in ["drums", "bass"] {
            let mut t = Track::new(name, 0);
            t.slots[0] = Some(Clip::new("a", BAR));
            t.slots[1] = Some(Clip::new("b", 2 * BAR));
            p.tracks.push(t);
        }
        p
    }

    #[test]
    fn placements_stay_sorted_and_never_overlap_on_a_track() {
        let p = project();
        let mut s = Arrangement::default();
        assert_eq!(s.place(Placement::linked(1, 0, BAR, BAR)), Ok(0));
        assert_eq!(s.place(Placement::linked(0, 0, 0, 2 * BAR)), Ok(0));
        assert_eq!(s.place(Placement::linked(0, 1, 2 * BAR, BAR)), Ok(1));
        assert_eq!(
            s.place(Placement::linked(0, 1, BAR, BAR)),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(s.check(&p.tracks), Ok(()));
        assert_eq!(s.on_track(0).len(), 2);
        assert_eq!(s.at(0, 2 * BAR + 5), Some(1));
        assert_eq!(s.at(1, 0), None);
        assert_eq!(s.end(), 3 * BAR);
        s.placements.swap(0, 1);
        assert_eq!(s.check(&p.tracks), Err(ModelError::BadPlacement));
    }

    #[test]
    fn a_linked_placement_follows_its_slot_until_made_unique() {
        let mut p = project();
        p.arrangement
            .place(Placement::linked(0, 1, 0, 4 * BAR))
            .unwrap();
        p.tracks[0].slots[1].as_mut().unwrap().name = "edited".into();
        let name = |p: &Project| {
            p.arrangement.placements[0]
                .clip(&p.tracks)
                .unwrap()
                .name
                .clone()
        };
        assert_eq!(name(&p), "edited");
        let tracks = p.tracks.clone();
        p.arrangement.make_unique(&tracks, 0).unwrap();
        p.tracks[0].slots[1].as_mut().unwrap().name = "again".into();
        assert_eq!(name(&p), "edited", "its own copy now");
        // Emptying a slot: its placements stay, silent, unless unlinked
        // first (they keep a copy).
        p.arrangement
            .place(Placement::linked(1, 0, 0, BAR))
            .unwrap();
        p.tracks[1].slots[0] = None;
        assert_eq!(p.check(), Ok(()));
        assert!(p.arrangement.placements[1].clip(&p.tracks).is_none());
        p.tracks[1].slots[0] = Some(Clip::new("a", BAR));
        let tracks = p.tracks.clone();
        assert_eq!(p.arrangement.unlink(&tracks, 1, 0), 1);
        p.tracks[1].slots[0] = None;
        assert_eq!(p.check(), Ok(()));
    }

    #[test]
    fn an_offset_moves_the_origin_back_at_the_clips_speed() {
        let mut c = Clip::new("c", BAR);
        c.speed = crate::Speed::new(1, 2).unwrap();
        let mut pl = Placement::linked(0, 0, 10 * BAR, BAR);
        pl.offset = 960;
        assert_eq!(pl.origin(&c), 10 * BAR - 1920, "half speed: twice as far");
    }

    #[test]
    fn scenes_become_a_song_and_the_song_becomes_scenes() {
        let mut p = project();
        p.scene_mut(1).unwrap().name = "Chorus".into();
        p.scene_mut(1).unwrap().follow = Some(Follow::after(4, FollowAction::Next));
        assert_eq!(scene_length(&p, 0, BAR), BAR);
        assert_eq!(scene_length(&p, 1, BAR), 4 * BAR, "its follow's bars");
        let song = scenes_to_song(&p, &[(0, BAR), (1, 4 * BAR), (0, BAR)], 0).unwrap();
        assert_eq!(song.placements.len(), 6);
        assert_eq!(song.end(), 6 * BAR);
        let names: Vec<_> = song
            .markers
            .iter()
            .map(|m| (m.at, m.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![(0, "Scene 1"), (BAR, "Chorus"), (5 * BAR, "Scene 1")]
        );
        p.arrangement = song;
        assert_eq!(p.check(), Ok(()));
        // Back: three sections into rows 2, 3 and 4, chained.
        assert_eq!(sections(&p).len(), 3);
        let rows = song_to_scenes(&mut p, BAR).unwrap();
        assert_eq!(rows, vec![2, 3, 4]);
        let s = |r: usize| p.scene(r).unwrap();
        assert_eq!(s(2).follow, Some(Follow::after(1, FollowAction::Next)));
        assert_eq!(s(3).follow, Some(Follow::after(4, FollowAction::Next)));
        assert_eq!(s(4).follow, Some(Follow::after(1, FollowAction::Stop)));
        assert_eq!(s(3).name, "Chorus");
        assert_eq!(p.tracks[1].slots[3].as_ref().unwrap().name, "b");
        // No room for 3 more after row 13 is taken.
        p.tracks[0].slots[13] = Some(Clip::new("x", BAR));
        p.tracks[0].slots[8] = Some(Clip::new("x", BAR));
        let before = p.clone();
        assert_eq!(song_to_scenes(&mut p, BAR), Ok(vec![5, 6, 7]));
        let mut q = before;
        q.tracks[0].slots[6] = Some(Clip::new("x", BAR));
        q.tracks[0].slots[11] = Some(Clip::new("x", BAR));
        let again = q.clone();
        assert_eq!(song_to_scenes(&mut q, BAR), Err(ModelError::NoRoom));
        assert_eq!(q, again, "nothing changed");
    }

    #[test]
    fn sections_merge_where_nothing_changes_and_split_at_markers() {
        let mut p = project();
        // Drums 0..4 bars of one placement; bass 0..2 and 2..4 bars of two.
        let s = &mut p.arrangement;
        s.place(Placement::linked(0, 0, 0, 4 * BAR)).unwrap();
        s.place(Placement::linked(1, 0, 0, 2 * BAR)).unwrap();
        s.place(Placement::linked(1, 0, 2 * BAR, 2 * BAR)).unwrap();
        assert_eq!(sections(&p).len(), 2, "the bass's second placement");
        p.arrangement.placements[2].start = 3 * BAR;
        p.arrangement.placements[2].length = BAR;
        // 0..2 both, 2..3 drums only, 3..4 both again.
        assert_eq!(
            sections(&p).iter().map(|s| (s.0, s.1)).collect::<Vec<_>>(),
            vec![(0, 2 * BAR), (2 * BAR, 3 * BAR), (3 * BAR, 4 * BAR)]
        );
        p.arrangement.mark(BAR, "B").unwrap();
        assert_eq!(sections(&p).len(), 4);
    }
}
