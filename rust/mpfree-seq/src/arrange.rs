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
//! The Song view's drags (Jay, 14 Nov: drag and drop) edit it in place:
//! [`Arrangement::move_placement`] moves or copies a placement along its
//! lane or onto another track's (it takes its own copy of a linked clip
//! there), [`Arrangement::set_length`] makes one longer or shorter, and
//! [`Arrangement::move_section`] moves or copies a section (marker to
//! marker) on every track, the others closing up or making room.
//!
//! The song has lanes of its own too ([`Arrangement::lanes`]), for what
//! is not on a track: the morph crossfader's moves, with the rows it
//! morphs between from each tick on ([`Arrangement::morphs`]). Live to song
//! (T11, `mpfree-perform`) records them; song mode plays the lanes.
//!
//! Plain values checked by [`Arrangement::check`]; `mpfree-play` plays
//! them (song mode) and `mpfree-perform` records into them (T11).

use crate::mpfree_moves::{Kind, Map, Moves, Target};
use crate::mpfree_param::ParamAddr;
use crate::{Clip, Follow, FollowAction, ModelError, Project, Track, SCENES, SLOTS};
use mpfree_transport::time::Tick;

/// Placements in a song.
pub const MAX_PLACEMENTS: usize = 4096;
/// Markers in a song.
pub const MAX_MARKERS: usize = 256;
/// Changes of the morph's rows in a song.
pub const MAX_MORPHS: usize = 1024;
/// How long the song's lanes are, in ticks: they never loop (about 72
/// hours at 120 BPM).
pub const SONG_LANE_LEN: u32 = 1 << 30;

/// The song lane of the morph crossfader.
pub const MORPH_LANE: Target = Target::Fixed(ParamAddr::morph_pos(0));

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

    /// What plays of it from song tick `at` on (inside it), from where the
    /// clip has got to there: the part after a cut at `at`.
    pub fn from(&self, tracks: &[Track], at: Tick) -> Placement {
        let speed = self.clip(tracks).map_or(crate::Speed::NORMAL, |c| c.speed);
        let length = self.clip(tracks).map_or(1, |c| c.length.max(1));
        let moved = speed.to_clip((at - self.start) as f64).round() as Tick;
        Placement {
            offset: (self.offset + moved).rem_euclid(length),
            start: at,
            length: self.end() - at,
            ..self.clone()
        }
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

/// From `at` on, the morph is between rows `a` and `b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MorphRows {
    pub at: Tick,
    pub a: usize,
    pub b: usize,
}

/// The song: placements sorted by track, then start (none overlapping on a
/// track), and markers sorted by tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arrangement {
    pub placements: Vec<Placement>,
    pub markers: Vec<Marker>,
    /// The song's own lanes, on song ticks ([`SONG_LANE_LEN`] long, not
    /// looping), for parameters not on a track: the morph
    /// ([`MORPH_LANE`]).
    pub lanes: Moves,
    /// The morph's rows from each tick on, in order.
    pub morphs: Vec<MorphRows>,
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
        if self.morphs.len() > MAX_MORPHS
            || self
                .morphs
                .iter()
                .any(|m| m.at < 0 || m.a >= SCENES || m.b >= SCENES)
            || self.morphs.windows(2).any(|w| w[0].at >= w[1].at)
            || self
                .lanes
                .lanes()
                .iter()
                .any(|l| matches!(l.target, Target::Track(_)))
        {
            return Err(ModelError::BadPlacement);
        }
        self.lanes
            .check(SONG_LANE_LEN)
            .map_err(|_| ModelError::BadMoves)
    }

    /// Take track `track`'s placements out of song ticks `from..to`: the
    /// ones inside go, the ones across an edge are cut there (a cut at the
    /// start moves the offset on, so the rest plays as before), and one
    /// across the whole stretch becomes two.
    pub fn clear(
        &mut self,
        tracks: &[Track],
        track: usize,
        from: Tick,
        to: Tick,
    ) -> Result<(), ModelError> {
        if to <= from {
            return Ok(());
        }
        let mut i = self.placements.partition_point(|p| p.track < track);
        while i < self.placements.len() {
            let p = &self.placements[i];
            if p.track != track || p.start >= to {
                break;
            }
            if p.end() <= from {
                i += 1;
                continue;
            }
            let (start, end) = (p.start, p.end());
            let rest = p.clone();
            if start < from {
                self.placements[i].length = from - start;
                i += 1;
            } else {
                self.placements.remove(i);
            }
            if end > to {
                let rest = rest.from(tracks, to);
                if self.placements.len() >= MAX_PLACEMENTS {
                    return Err(ModelError::BadPlacement);
                }
                self.placements.insert(i, rest);
                break;
            }
        }
        Ok(())
    }

    /// Write morph moves recorded over song ticks `from..to` (points at
    /// song ticks, positions 0 to 1) into [`MORPH_LANE`], over what was
    /// there, with the rows they morph between (`rows`, in order, within
    /// `from..to`): the rows in force at `to` go on after it.
    pub fn put_morph(
        &mut self,
        points: &[(f64, f32, Kind)],
        rows: &[MorphRows],
        from: Tick,
        to: Tick,
    ) -> Result<(), ModelError> {
        if to <= from || to > Tick::from(SONG_LANE_LEN) {
            return Err(ModelError::BadPlacement);
        }
        // The rows the song had at `to` go on after the pass.
        let after = self.morph_rows_at(to);
        self.lanes
            .put(MORPH_LANE, Map::linear(0.0, 1.0), points, SONG_LANE_LEN)
            .map_err(|_| ModelError::BadMoves)?;
        self.morphs.retain(|m| m.at < from || m.at > to);
        let k = self.morphs.partition_point(|m| m.at < from);
        let mut new: Vec<MorphRows> = Vec::with_capacity(rows.len() + 1);
        for m in rows.iter().filter(|m| (from..to).contains(&m.at)) {
            match new.last_mut() {
                Some(last) if last.at >= m.at => *last = *m,
                _ => new.push(*m),
            }
        }
        new.extend(after.map(|a| MorphRows { at: to, ..a }));
        self.morphs.splice(k..k, new);
        // Only real changes.
        self.morphs.dedup_by(|b, a| (a.a, a.b) == (b.a, b.b));
        if self.morphs.len() > MAX_MORPHS {
            return Err(ModelError::BadPlacement);
        }
        Ok(())
    }

    /// The morph's rows in force at song tick `at`.
    pub fn morph_rows_at(&self, at: Tick) -> Option<MorphRows> {
        let k = self.morphs.partition_point(|m| m.at <= at);
        k.checked_sub(1).map(|j| self.morphs[j])
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

    /// Placement `i` moved to `track` from `start`, or with `copy` a copy
    /// of it put there and the placement left as it is (Jay, 14 Nov: drag
    /// and drop on the Song view). Onto another track a placement linked
    /// to a slot takes its own copy of the clip, as the slot would name
    /// that track's clip. Refused (`BadPlacement`), changing nothing, when
    /// it would overlap a placement there or start before 0, or goes to
    /// another track linked to an empty slot. Its index now.
    pub fn move_placement(
        &mut self,
        tracks: &[Track],
        i: usize,
        track: usize,
        start: Tick,
        copy: bool,
    ) -> Result<usize, ModelError> {
        let p = self.placements.get(i).ok_or(ModelError::BadPlacement)?;
        if track >= tracks.len() || start < 0 {
            return Err(ModelError::BadPlacement);
        }
        let mut moved = p.clone();
        if track != p.track && p.slot().is_some() {
            let clip = p.clip(tracks).ok_or(ModelError::BadPlacement)?;
            moved.source = Source::Own(Box::new(clip.clone()));
        }
        moved.track = track;
        moved.start = start;
        if copy {
            return self.place(moved);
        }
        let old = self.placements.remove(i);
        self.place(moved)
            .inspect_err(|_| self.placements.insert(i, old))
    }

    /// Placement `i` made `length` song ticks long: past its clip's end
    /// the clip plays again from its top. Refused (`BadPlacement`),
    /// changing nothing, when it is not above 0 or would run into the next
    /// placement on its track.
    pub fn set_length(&mut self, i: usize, length: Tick) -> Result<(), ModelError> {
        let p = self.placements.get(i).ok_or(ModelError::BadPlacement)?;
        let next = self.placements.get(i + 1).filter(|q| q.track == p.track);
        if length <= 0 || next.is_some_and(|q| q.start < p.start + length) {
            return Err(ModelError::BadPlacement);
        }
        self.placements[i].length = length;
        Ok(())
    }

    /// Section `k`: from marker `k` to the next, the last one to the end
    /// of the song in whole bars of `bar` ticks from its marker (a bar at
    /// least). None past the markers.
    pub fn section(&self, k: usize, bar: Tick) -> Option<(Tick, Tick)> {
        let m = self.markers.get(k)?;
        let end = match self.markers.get(k + 1) {
            Some(n) => n.at,
            None => {
                let bar = bar.max(1);
                let span = (self.end() - m.at).max(1);
                m.at + (span + bar - 1) / bar * bar
            }
        };
        Some((m.at, end))
    }

    /// Section `k` ([`Arrangement::section`]) moved, on every track, to
    /// before section `gap` (after the last for `gap` = the markers'
    /// count), or with `copy` a copy of it put there (a second verse).
    /// The sections close up behind it and make room for it, so nothing is
    /// written over; what comes before the first marker stays. A placement
    /// across a section's edge is cut there first, each part playing as
    /// before (as [`Arrangement::clear`] cuts). Moved to where it is, it
    /// stays. Refused, changing nothing: `BadPlacement` for no such
    /// section or gap or past [`MAX_PLACEMENTS`] or [`MAX_MARKERS`];
    /// `BadMoves` while the song has morph moves, which would not move
    /// with it.
    pub fn move_section(
        &mut self,
        tracks: &[Track],
        k: usize,
        gap: usize,
        copy: bool,
        bar: Tick,
    ) -> Result<(), ModelError> {
        let n = self.markers.len();
        if k >= n || gap > n {
            return Err(ModelError::BadPlacement);
        }
        if !self.lanes.is_empty() || !self.morphs.is_empty() {
            return Err(ModelError::BadMoves);
        }
        let mut order: Vec<usize> = (0..n).collect();
        if copy {
            order.insert(gap, k);
        } else if gap == k || gap == k + 1 {
            return Ok(());
        } else {
            order.remove(k);
            order.insert(if gap > k { gap - 1 } else { gap }, k);
        }
        let spans: Vec<(Tick, Tick)> = (0..n)
            .map(|s| self.section(s, bar))
            .collect::<Option<_>>()
            .ok_or(ModelError::BadPlacement)?;
        let mut cut = self.placements.clone();
        for &(at, _) in &spans {
            cut = cut
                .into_iter()
                .flat_map(|p| {
                    if p.start < at && at < p.end() {
                        let rest = p.from(tracks, at);
                        vec![
                            Placement {
                                length: at - p.start,
                                ..p
                            },
                            rest,
                        ]
                    } else {
                        vec![p]
                    }
                })
                .collect();
        }
        let first = spans[0].0;
        let mut song = Arrangement {
            lanes: self.lanes.clone(),
            morphs: self.morphs.clone(),
            ..Arrangement::default()
        };
        song.placements = cut.iter().filter(|p| p.start < first).cloned().collect();
        let mut t = first;
        for &s in &order {
            let (a, b) = spans[s];
            song.placements
                .extend(
                    cut.iter()
                        .filter(|p| (a..b).contains(&p.start))
                        .map(|p| Placement {
                            start: p.start - a + t,
                            ..p.clone()
                        }),
                );
            if song.markers.len() >= MAX_MARKERS {
                return Err(ModelError::BadPlacement);
            }
            song.markers.push(Marker {
                at: t,
                name: self.markers[s].name.clone(),
            });
            t += b - a;
        }
        song.placements.sort_by_key(|p| (p.track, p.start));
        song.check(tracks)?;
        *self = song;
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
    fn clearing_cuts_placements_and_keeps_the_rest_in_phase() {
        let p = project();
        let mut s = Arrangement::default();
        // Slot 1 (two bars) from 0 for 6 bars, and slot 0 from bar 7.
        s.place(Placement::linked(0, 1, 0, 6 * BAR)).unwrap();
        s.place(Placement::linked(0, 0, 7 * BAR, BAR)).unwrap();
        s.place(Placement::linked(1, 0, 0, 8 * BAR)).unwrap();
        s.clear(&p.tracks, 0, BAR, 3 * BAR + 100).unwrap();
        let t0: Vec<(Tick, Tick, Tick)> = s
            .on_track(0)
            .iter()
            .map(|p| (p.start, p.length, p.offset))
            .collect();
        // The rest starts 3 bars and 100 ticks in: a bar and 100 ticks
        // into its two-bar clip.
        assert_eq!(
            t0,
            [
                (0, BAR, 0),
                (3 * BAR + 100, 3 * BAR - 100, BAR + 100),
                (7 * BAR, BAR, 0)
            ]
        );
        assert_eq!(s.on_track(1).len(), 1, "other tracks stay");
        s.clear(&p.tracks, 0, 3 * BAR, 9 * BAR).unwrap();
        assert_eq!(s.on_track(0).len(), 1);
        assert_eq!(s.check(&p.tracks), Ok(()));
    }

    #[test]
    fn morph_moves_go_into_the_song_lane_with_their_rows() {
        let p = project();
        let mut s = Arrangement::default();
        let rows = |at, a, b| MorphRows { at, a, b };
        s.put_morph(
            &[
                (0.0, 0.0, Kind::Ramp),
                (1000.0, 1.0, Kind::Ramp),
                (2000.0, 0.5, Kind::Ramp),
            ],
            &[rows(0, 0, 1), rows(1500, 2, 3)],
            0,
            4 * BAR,
        )
        .unwrap();
        assert_eq!(s.morphs, [rows(0, 0, 1), rows(1500, 2, 3)]);
        let lane = s.lanes.lane(MORPH_LANE).unwrap();
        assert!((lane.value_at(500.0).unwrap() - 0.5).abs() < 1e-3);
        // A second pass over part of it: what was there after it goes on.
        s.put_morph(
            &[(100.0, 0.25, Kind::Ramp), (200.0, 0.25, Kind::Ramp)],
            &[rows(100, 5, 6)],
            100,
            200,
        )
        .unwrap();
        assert_eq!(
            s.morphs,
            [
                rows(0, 0, 1),
                rows(100, 5, 6),
                rows(200, 0, 1),
                rows(1500, 2, 3)
            ]
        );
        assert_eq!(s.morph_rows_at(150), Some(rows(100, 5, 6)));
        assert_eq!(s.check(&p.tracks), Ok(()));
        s.morphs.push(rows(1500, 0, 1));
        assert_eq!(s.check(&p.tracks), Err(ModelError::BadPlacement));
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

    #[test]
    fn a_placement_moves_or_copies_along_and_across_lanes() {
        let p = project();
        let mut s = Arrangement::default();
        s.place(Placement::linked(0, 0, 0, BAR)).unwrap();
        s.place(Placement::linked(0, 1, 4 * BAR, 2 * BAR)).unwrap();
        s.place(Placement::linked(1, 0, 0, BAR)).unwrap();
        // Along its lane, past nothing: its index stays.
        assert_eq!(s.move_placement(&p.tracks, 0, 0, 2 * BAR, false), Ok(0));
        assert_eq!(s.placements[0].start, 2 * BAR);
        // Onto the next block: refused, nothing changes.
        let before = s.clone();
        assert_eq!(
            s.move_placement(&p.tracks, 0, 0, 5 * BAR, false),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(s, before);
        // Over its own old place is fine.
        assert_eq!(
            s.move_placement(&p.tracks, 0, 0, 2 * BAR + 100, false),
            Ok(0)
        );
        // Across to the bass: its own copy of the drums' clip.
        let j = s.move_placement(&p.tracks, 1, 1, 4 * BAR, false).unwrap();
        assert_eq!((s.placements[j].track, s.placements[j].start), (1, 4 * BAR));
        assert_eq!(s.placements[j].slot(), None);
        assert_eq!(s.placements[j].clip(&p.tracks).unwrap().name, "b");
        // A copy along the lane stays linked; the first stays where it is.
        let n = s.placements.len();
        let c = s.move_placement(&p.tracks, 0, 0, 8 * BAR, true).unwrap();
        assert_eq!(s.placements.len(), n + 1);
        assert_eq!(s.placements[c].slot(), Some(0));
        assert_eq!(s.placements[0].start, 2 * BAR + 100);
        assert_eq!(s.check(&p.tracks), Ok(()));
        // A copy onto itself overlaps.
        assert_eq!(
            s.move_placement(&p.tracks, 0, 0, 2 * BAR + 100, true),
            Err(ModelError::BadPlacement)
        );
    }

    #[test]
    fn a_placement_gets_longer_up_to_the_next_one() {
        let mut s = Arrangement::default();
        s.place(Placement::linked(0, 0, 0, BAR)).unwrap();
        s.place(Placement::linked(0, 0, 4 * BAR, BAR)).unwrap();
        s.place(Placement::linked(1, 0, BAR, BAR)).unwrap();
        assert_eq!(s.set_length(0, 4 * BAR), Ok(()));
        assert_eq!(s.set_length(0, 4 * BAR + 1), Err(ModelError::BadPlacement));
        assert_eq!(s.set_length(0, 0), Err(ModelError::BadPlacement));
        // The last on its track: as long as it likes.
        assert_eq!(s.set_length(1, 64 * BAR), Ok(()));
        assert_eq!(s.placements[0].length, 4 * BAR);
    }

    /// The song for the section tests: A (bars 1-2), B (3-4), C (5-8),
    /// drums all along in one block, bass a block in each.
    fn sections_song() -> Project {
        let mut p = project();
        let s = &mut p.arrangement;
        s.place(Placement::linked(0, 0, 0, 8 * BAR)).unwrap();
        s.place(Placement::linked(1, 0, 0, 2 * BAR)).unwrap();
        s.place(Placement::linked(1, 1, 2 * BAR, 2 * BAR)).unwrap();
        s.place(Placement::linked(1, 0, 4 * BAR, 3 * BAR)).unwrap();
        for (at, name) in [(0, "A"), (2 * BAR, "B"), (4 * BAR, "C")] {
            s.mark(at, name).unwrap();
        }
        p
    }

    fn names(s: &Arrangement) -> Vec<(Tick, &str)> {
        s.markers.iter().map(|m| (m.at, m.name.as_str())).collect()
    }

    fn lane(s: &Arrangement, track: usize) -> Vec<(Tick, Tick, Tick)> {
        s.on_track(track)
            .iter()
            .map(|p| (p.start, p.length, p.offset))
            .collect()
    }

    #[test]
    fn a_section_moves_and_the_others_close_up() {
        let mut p = sections_song();
        assert_eq!(p.arrangement.section(1, BAR), Some((2 * BAR, 4 * BAR)));
        // C ends at the song's end in whole bars: 3 bars of bass, 4 of drums.
        assert_eq!(p.arrangement.section(2, BAR), Some((4 * BAR, 8 * BAR)));
        assert_eq!(p.arrangement.section(3, BAR), None);
        // B before A.
        p.arrangement
            .move_section(&p.tracks, 1, 0, false, BAR)
            .unwrap();
        let s = &p.arrangement;
        assert_eq!(names(s), [(0, "B"), (2 * BAR, "A"), (4 * BAR, "C")]);
        // The bass: B's two-bar clip first, then A's.
        assert_eq!(
            lane(s, 1),
            [
                (0, 2 * BAR, 0),
                (2 * BAR, 2 * BAR, 0),
                (4 * BAR, 3 * BAR, 0)
            ]
        );
        assert_eq!(s.on_track(1)[0].slot(), Some(1));
        // The drums' long block is cut at the edges, each part in phase
        // (its clip is one bar, so every part starts at its top).
        assert_eq!(
            lane(s, 0),
            [
                (0, 2 * BAR, 0),
                (2 * BAR, 2 * BAR, 0),
                (4 * BAR, 4 * BAR, 0)
            ]
        );
        assert_eq!(s.check(&p.tracks), Ok(()));
        // Moved to where it is: nothing changes.
        let before = p.arrangement.clone();
        p.arrangement
            .move_section(&p.tracks, 1, 2, false, BAR)
            .unwrap();
        assert_eq!(p.arrangement, before);
        // The first after the last.
        p.arrangement
            .move_section(&p.tracks, 0, 3, false, BAR)
            .unwrap();
        assert_eq!(
            names(&p.arrangement),
            [(0, "A"), (2 * BAR, "C"), (6 * BAR, "B")]
        );
    }

    #[test]
    fn a_copied_section_makes_room_and_what_comes_first_stays() {
        let mut p = sections_song();
        // Something before the first marker stays where it is.
        p.arrangement.markers[0].at = BAR;
        p.arrangement.markers[0].name = "A".into();
        p.arrangement
            .move_section(&p.tracks, 1, 3, true, BAR)
            .unwrap();
        let s = &p.arrangement;
        assert_eq!(
            names(s),
            [(BAR, "A"), (2 * BAR, "B"), (4 * BAR, "C"), (8 * BAR, "B")]
        );
        // The bass: bar 1 (before A), A's last bar, B, C, and B again.
        assert_eq!(
            lane(s, 1),
            [
                (0, BAR, 0),
                (BAR, BAR, 0),
                (2 * BAR, 2 * BAR, 0),
                (4 * BAR, 3 * BAR, 0),
                (8 * BAR, 2 * BAR, 0)
            ]
        );
        assert_eq!(s.on_track(1)[4].slot(), Some(1), "linked like the first");
        assert_eq!(s.check(&p.tracks), Ok(()));
    }

    #[test]
    fn sections_with_morph_moves_stay_put() {
        let mut p = sections_song();
        p.arrangement
            .put_morph(&[(0.0, 0.5, Kind::Step)], &[], 0, BAR)
            .unwrap();
        let before = p.arrangement.clone();
        assert_eq!(
            p.arrangement.move_section(&p.tracks, 1, 0, false, BAR),
            Err(ModelError::BadMoves)
        );
        assert_eq!(p.arrangement, before);
        assert_eq!(
            p.arrangement.move_section(&p.tracks, 5, 0, false, BAR),
            Err(ModelError::BadPlacement)
        );
    }
}
