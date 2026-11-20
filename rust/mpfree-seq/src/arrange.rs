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
//!   The scenes' mutes carry from row to row as they do when the rows
//!   play (a muted track's clip is left out), and a one-shot clip's block
//!   ends with its pass.
//! - [`song_to_scenes`]: each section of the song (between the places
//!   anything starts or ends, and the markers) becomes a row in a run of
//!   empty rows, chained by follow actions (Next after the section's
//!   bars; the last one stops), so launching the first row plays the
//!   song. Each clip is copied turned round to where the section starts
//!   in it ([`turned`]), without the one-shot, legato and follow action a
//!   song block ignores, so the row plays as the block did.
//!
//! Every conversion is exact or says how it is not (plan D7): each way
//! the result plays differently is a [`Difference`], by track and bar.
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

use crate::mpfree_moves::{Kind, Map, Moves, Point, Target};
use crate::mpfree_param::ParamAddr;
use crate::scenes::SongList;
use crate::{
    Clip, Cond, Direction, Follow, FollowAction, Join, ModelError, Project, Scene, Track,
    MAX_TRACKS, ROWS, SCENES, SLOTS,
};
use mpfree_transport::time::{
    valid_bpm, MapError, Segment, TempoMap, Tick, MAX_SEGMENTS, MIN_SPAN_TICKS,
};

/// Placements in a song.
pub const MAX_PLACEMENTS: usize = 4096;
/// Markers in a song.
pub const MAX_MARKERS: usize = 256;
/// Changes of the morph's rows in a song.
pub const MAX_MORPHS: usize = 1024;
/// Tempo points in a song: the tempo map's segments, less one for the
/// project's tempo before the first point.
pub const MAX_TEMPO_POINTS: usize = MAX_SEGMENTS - 1;
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

/// A tempo point on the song (sequencer roadmap, tempo ramps): from `at`
/// on the song plays `bpm`, and with `ramp` it glides from there to the
/// next point's tempo, arriving on that point (the transport plays the
/// glide in 1/16 steps; see `mpfree_transport::TempoMap`). A ramp on the
/// last point waits for a point after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoPoint {
    pub at: Tick,
    pub bpm: f64,
    pub ramp: bool,
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
    /// The song list (sequencer S4): scenes in order, each so many times,
    /// the third way to see the song beside the grid and the timeline.
    /// It plays on its own (`mpfree-perform`); List → song lays it on the
    /// timeline ([`list_to_song`]).
    pub list: SongList,
    /// The song's tempo points, in order, at least a 1/16 apart. None:
    /// the project's tempo throughout.
    pub tempo: Vec<TempoPoint>,
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
        self.list.check()?;
        for (i, p) in self.placements.iter().enumerate() {
            if p.length <= 0 || p.start < 0 || p.offset < 0 || p.track >= tracks.len() {
                return Err(ModelError::BadPlacement);
            }
            match &p.source {
                Source::Slot(s) if *s >= ROWS => return Err(ModelError::BadPlacement),
                Source::Own(c) => {
                    c.check()?;
                    // A loop's own copy only on a Loop track, a clip's off one.
                    if c.looped.is_some() != tracks[p.track].is_loops() {
                        return Err(ModelError::BadLoop);
                    }
                }
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
        self.check_tempo()?;
        self.lanes
            .check(SONG_LANE_LEN)
            .map_err(|_| ModelError::BadMoves)
    }

    /// The tempo points: [`MAX_TEMPO_POINTS`] at most, none before the
    /// start, in order at least a 1/16 apart, each tempo in range.
    fn check_tempo(&self) -> Result<(), ModelError> {
        if self.tempo.len() > MAX_TEMPO_POINTS
            || self.tempo.iter().any(|p| p.at < 0 || !valid_bpm(p.bpm))
            || self
                .tempo
                .windows(2)
                .any(|w| w[1].at - w[0].at < MIN_SPAN_TICKS)
        {
            return Err(ModelError::BadTempo);
        }
        Ok(())
    }

    /// The song's tempo map with the project at `project_bpm`: the
    /// project's tempo until the first point (unless it is on bar 1), then
    /// each point, gliding where it ramps.
    pub fn tempo_map(&self, project_bpm: f64) -> Result<TempoMap, MapError> {
        let mut segs = [Segment {
            start: 0,
            bpm: project_bpm,
        }; MAX_SEGMENTS];
        let lead = usize::from(self.tempo.first().is_none_or(|p| p.at > 0));
        let n = lead + self.tempo.len();
        if n > MAX_SEGMENTS {
            return Err(MapError::TooManySegments);
        }
        for (seg, p) in segs[lead..n].iter_mut().zip(&self.tempo) {
            *seg = Segment {
                start: p.at,
                bpm: p.bpm,
            };
        }
        let mut map = TempoMap::new(&segs[..n])?;
        for (i, p) in self.tempo.iter().enumerate() {
            if p.ramp && i + 1 < self.tempo.len() {
                map = map.ramp(lead + i)?;
            }
        }
        Ok(map)
    }

    /// Put a tempo point at `at` (in place of one already there, keeping
    /// its ramp when `ramp` is None). Refused, changing nothing, with
    /// `BadTempo`: before the start, a tempo out of range, within a 1/16
    /// of another point, or past [`MAX_TEMPO_POINTS`].
    pub fn set_tempo(&mut self, at: Tick, bpm: f64, ramp: Option<bool>) -> Result<(), ModelError> {
        if at < 0 || !valid_bpm(bpm) {
            return Err(ModelError::BadTempo);
        }
        let i = self.tempo.partition_point(|p| p.at < at);
        if let Some(p) = self.tempo.get_mut(i).filter(|p| p.at == at) {
            p.bpm = bpm;
            p.ramp = ramp.unwrap_or(p.ramp);
            return Ok(());
        }
        let near = |j: usize| {
            self.tempo
                .get(j)
                .is_some_and(|p| (p.at - at).abs() < MIN_SPAN_TICKS)
        };
        if self.tempo.len() >= MAX_TEMPO_POINTS || near(i) || (i > 0 && near(i - 1)) {
            return Err(ModelError::BadTempo);
        }
        self.tempo.insert(
            i,
            TempoPoint {
                at,
                bpm,
                ramp: ramp.unwrap_or(false),
            },
        );
        Ok(())
    }

    /// Take the tempo point at `at` away. False if there is none.
    pub fn remove_tempo(&mut self, at: Tick) -> bool {
        let before = self.tempo.len();
        self.tempo.retain(|p| p.at != at);
        self.tempo.len() < before
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
            // The song's own copy has no id until the song keeps its
            // clips once (phase 1.3).
            let clip = p.clip(tracks).ok_or(ModelError::BadPlacement)?.copied();
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
    /// another track linked to an empty slot; and (`BadLoop`) when it goes
    /// to another track and either is a Loop track: a loop's block stays on
    /// its Loop track's lane, as its cell stays in its column
    /// (loop-tracks.md). Its index now.
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
        let loops = |t: usize| tracks.get(t).is_some_and(Track::is_loops);
        if track != p.track && (loops(track) || loops(p.track)) {
            return Err(ModelError::BadLoop);
        }
        let mut moved = p.clone();
        if track != p.track && p.slot().is_some() {
            let clip = p.clip(tracks).ok_or(ModelError::BadPlacement)?;
            moved.source = Source::Own(Box::new(clip.copied()));
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
    /// `BadMoves` while the song has morph moves or tempo points, which
    /// would not move with it.
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
        if !self.lanes.is_empty() || !self.morphs.is_empty() || !self.tempo.is_empty() {
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
        // Everything but the placements and markers stays as it was: the
        // song list names rows, not ticks.
        let mut song = Arrangement {
            placements: Vec::new(),
            markers: Vec::new(),
            lanes: self.lanes.clone(),
            morphs: self.morphs.clone(),
            list: self.list.clone(),
            tempo: self.tempo.clone(),
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
/// as linked placements with a marker each: see the module notes. The
/// scenes' mutes carry from row to row from none (a track a scene muted
/// gets no blocks until a scene unmutes it), and a one-shot clip's block
/// ends with its pass. What it can't carry is in the differences: a
/// scene's program changes, a clip's own follow action, and a legato
/// clip that would take over a clip playing on its track.
pub fn scenes_to_song(
    project: &Project,
    rows: &[(usize, Tick)],
    at: Tick,
) -> Result<ToSong, ModelError> {
    let mut song = Arrangement::default();
    let mut differences = Vec::new();
    let mut muted = [false; MAX_TRACKS];
    // Where each track's last block ends: a clip still plays there (a
    // muted one too, silent).
    let mut ends: Vec<Option<Tick>> = vec![None; project.tracks.len()];
    let mut t = at;
    for &(row, length) in rows {
        if row >= SCENES || length <= 0 {
            return Err(ModelError::BadPlacement);
        }
        let scene = project.scene(row);
        if let Some(s) = scene {
            for (track, m) in muted.iter_mut().enumerate() {
                if let Some(now) = s.mutes_track(track) {
                    *m = now;
                }
            }
            if !s.programs.is_empty() {
                differences.push(Difference::song(t, Differs::SceneProgram));
            }
        }
        for (track, tr) in project.tracks.iter().enumerate() {
            let Some(clip) = tr.slots[row].as_ref() else {
                continue;
            };
            let mut len = length;
            if clip.once {
                // Its pass, as `mpfree_perform::once_end` ends it.
                let pass = clip.speed.to_song(clip.length as f64).max(1.0);
                len = len.min(pass.round() as Tick);
            }
            if clip.follow.is_some() {
                differences.push(Difference::on(track, t, Differs::ClipFollow));
            }
            if clip.legato && ends[track] == Some(t) {
                differences.push(Difference::on(track, t, Differs::Legato));
            }
            ends[track] = Some(t + len);
            if muted.get(track).copied().unwrap_or(false) {
                continue;
            }
            song.place(Placement::linked(track, row, t, len))?;
        }
        let name = scene
            .map(|s| s.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Scene {}", row + 1));
        song.mark(t, &name)?;
        t += length;
    }
    Ok(ToSong { song, differences })
}

/// Each step of the song list: where it starts, from tick 0, and how
/// long it plays (its scene's length, [`scene_length`], times its times),
/// with bars of `bar` ticks.
pub fn list_spans(project: &Project, bar: Tick) -> Vec<(Tick, Tick)> {
    let mut at = 0;
    project
        .arrangement
        .list
        .steps
        .iter()
        .map(|s| {
            let len = scene_length(project, s.row, bar) * Tick::from(s.times);
            let span = (at, len);
            at += len;
            span
        })
        .collect()
}

/// List → song: the song list laid on the timeline from song tick `at`,
/// each step its scene's clips for its span (a clip shorter than the span
/// repeats), with the scene's name above where it starts, as
/// [`scenes_to_song`] lays rows (mutes carried from step to step, a
/// one-shot's block ending with its pass). The caller adds it to the song.
pub fn list_to_song(project: &Project, at: Tick, bar: Tick) -> Result<ToSong, ModelError> {
    let rows: Vec<(usize, Tick)> = project
        .arrangement
        .list
        .steps
        .iter()
        .zip(list_spans(project, bar))
        .map(|(s, (_, len))| (s.row, len))
        .collect();
    scenes_to_song(project, &rows, at)
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
/// first run of empty rows ([`Project::row_empty`]) long enough, and sets
/// every field of each row's scene; returns the rows and the ways they
/// play differently from the song, or `NoRoom` (and changes nothing) when
/// there is no such run.
pub fn song_to_scenes(project: &mut Project, bar: Tick) -> Result<ToScenes, ModelError> {
    let bar = bar.max(1);
    let parts = sections(project);
    if parts.is_empty() {
        return Ok(ToScenes::default());
    }
    let n = parts.len();
    // A run of empty rows on the grid: the rows it shows ([`SLOTS`]).
    let first = (0..(SLOTS + 1).saturating_sub(n))
        .find(|&r| (r..r + n).all(|k| project.row_empty(k)))
        .ok_or(ModelError::NoRoom)?;
    let mut differences = song_differences(project, &parts, bar);
    let mut p = project.clone();
    for (k, (a, b, playing)) in parts.iter().enumerate() {
        let row = first + k;
        for (track, at) in playing.iter().enumerate() {
            let Some(i) = at else { continue };
            let block = &project.arrangement.placements[*i];
            // A placement of an emptied slot plays nothing: nothing to copy.
            let Some(clip) = block.clip(&project.tracks) else {
                continue;
            };
            let (copy, why) = section_copy(block, clip, *a);
            differences.extend(why.into_iter().map(|w| Difference::on(track, *a, w)));
            // A copy: a clip of its own, with a new id.
            p.put_new_clip(track, row, copy)?;
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
        let action = if k + 1 == n {
            FollowAction::Stop
        } else {
            FollowAction::Next
        };
        // Every field of the row's scene, not only the two it sets.
        *p.scene_mut(row).ok_or(ModelError::NoRoom)? = Scene {
            name,
            follow: Some(Follow::after(bars as u16, action)),
            ..Scene::default()
        };
    }
    p.check()?;
    *project = p;
    differences.sort_by_key(|d| (d.at, d.track.map_or(0, |t| t + 1)));
    Ok(ToScenes {
        rows: (first..first + n).collect(),
        differences,
    })
}

/// What Song → scenes did ([`song_to_scenes`]): the rows it filled, in
/// order, and how they play differently from the song (none: exact).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToScenes {
    pub rows: Vec<usize>,
    pub differences: Vec<Difference>,
}

/// What Scenes → song and List → song make ([`scenes_to_song`],
/// [`list_to_song`]): the blocks and markers to add to the song, and how
/// they play differently from the rows (none: exact).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToSong {
    pub song: Arrangement,
    pub differences: Vec<Difference>,
}

/// One way a conversion plays differently from what it was made from
/// (plan D7): what, where (a song tick: the song's for Song → scenes, the
/// new blocks' for Scenes → song and List → song) and on which track
/// (None: the whole song).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Difference {
    pub track: Option<usize>,
    pub at: Tick,
    pub what: Differs,
}

/// What a [`Difference`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Differs {
    /// Song → scenes: a section starts or ends off a bar line, and the
    /// rows change on bar lines.
    OffBar,
    /// Song → scenes: a section longer than a follow's most bars
    /// ([`crate::scenes::MAX_FOLLOW_BARS`]): its row moves on after that.
    TooLong,
    /// Song → scenes: a stretch of silence this long, which the rows
    /// leave out (before the first section too: the rows start when
    /// launched).
    Silence { length: Tick },
    /// Song → scenes: the song's tempo points; the rows play at the
    /// project's tempo.
    Tempo,
    /// Song → scenes: the song's own lanes (the morph's moves and rows);
    /// the rows don't play them.
    Morph,
    /// Song → scenes: the clip plays in Random order, which a copy in
    /// another slot rolls its own way.
    Random,
    /// Song → scenes: the clip has notes with a chance below 100 %, which
    /// a copy in another slot rolls its own way.
    Chance,
    /// Song → scenes: the clip plays Reverse or Ping-pong and its row
    /// starts it from partway in, where its direction starts again.
    Direction,
    /// Song → scenes: the clip has conditions that count passes or follow
    /// the last result (A:B, 1ST, PRE, NEI), and its row starts it again
    /// from partway in, where they count again.
    Conditions,
    /// Song → scenes: a Loop track's cell from partway in: its loop
    /// starts again from its top.
    LoopCell,
    /// Song → scenes: the section starts between two of the clip's own
    /// ticks (at its speed): the copy turns round to the nearest.
    BetweenTicks,
    /// Song → scenes: the block carries on into the section, and its row
    /// sends the clip's program change again.
    Program,
    /// Song → scenes: a note held over the section's start (or tied into
    /// a note on it), which the row change cuts there.
    HeldNote,
    /// Scenes → song, List → song: the scene's program changes, which the
    /// song doesn't send.
    SceneProgram,
    /// Scenes → song, List → song: the clip's own follow action, which
    /// the song leaves out (the clip plays on for the row).
    ClipFollow,
    /// Scenes → song, List → song: a legato clip after a clip on its
    /// track: the song starts it from its top instead of the other's
    /// place.
    Legato,
}

impl Difference {
    fn song(at: Tick, what: Differs) -> Difference {
        Difference {
            track: None,
            at,
            what,
        }
    }

    fn on(track: usize, at: Tick, what: Differs) -> Difference {
        Difference {
            track: Some(track),
            at,
            what,
        }
    }

    /// In a musician's words, with bars of `bar` ticks: "Bass, bar 9: a
    /// note held over the row change is cut there".
    pub fn said(&self, project: &Project, bar: Tick) -> String {
        let bar = bar.max(1);
        let bars = |t: Tick| {
            if t % bar != 0 {
                format!("{:.1} bars", t as f64 / bar as f64)
            } else if t == bar {
                "1 bar".to_string()
            } else {
                format!("{} bars", t / bar)
            }
        };
        let what = match self.what {
            Differs::OffBar => {
                "a part starts or ends off a bar line; the rows change on bar lines".into()
            }
            Differs::TooLong => format!(
                "a part is longer than {} bars; its row moves on after that",
                crate::scenes::MAX_FOLLOW_BARS
            ),
            Differs::Silence { length } => format!("{} of silence left out", bars(length)),
            Differs::Tempo => {
                "the song's tempo changes; the rows play at the project's tempo".into()
            }
            Differs::Morph => "the song's morph moves; the rows don't play them".into(),
            Differs::Random => "plays in Random order; the copy rolls its own order".into(),
            Differs::Chance => "notes with a chance; the copy rolls them its own way".into(),
            Differs::Direction => "its play direction starts again here".into(),
            Differs::Conditions => {
                "its A:B, 1ST, PRE and NEI conditions count again from here".into()
            }
            Differs::LoopCell => "its loop starts again from the top here".into(),
            Differs::BetweenTicks => {
                "starts between two of the clip's ticks; the copy starts on the nearest".into()
            }
            Differs::Program => "its program change is sent again here".into(),
            Differs::HeldNote => "a note held over the row change is cut there".into(),
            Differs::SceneProgram => "the scene's program changes aren't in the song".into(),
            Differs::ClipFollow => "the clip's follow action isn't in the song; it plays on".into(),
            Differs::Legato => "the clip is legato; the song starts it from its top".into(),
        };
        let at = format!("bar {}", self.at.div_euclid(bar) + 1);
        match self.track {
            Some(t) => {
                let name = project
                    .tracks
                    .get(t)
                    .map_or_else(|| format!("Track {}", t + 1), |t| t.name.clone());
                format!("{name}, {at}: {what}")
            }
            None => format!("{at}: {what}"),
        }
    }
}

/// The ways Song → scenes can't follow the whole song (`parts` are its
/// sections): silences, sections off the bar lines or too long, tempo
/// points and the song's own lanes.
fn song_differences(
    project: &Project,
    parts: &[(Tick, Tick, Vec<Option<usize>>)],
    bar: Tick,
) -> Vec<Difference> {
    let song = &project.arrangement;
    let mut out = Vec::new();
    let mut last = 0;
    for (a, b, _) in parts {
        if *a > last {
            out.push(Difference::song(
                last,
                Differs::Silence { length: a - last },
            ));
        }
        if a % bar != 0 || b % bar != 0 {
            out.push(Difference::song(*a, Differs::OffBar));
        }
        if b - a > Tick::from(crate::scenes::MAX_FOLLOW_BARS) * bar {
            out.push(Difference::song(*a, Differs::TooLong));
        }
        last = *b;
    }
    if let Some(t) = song.tempo.first() {
        out.push(Difference::song(t.at, Differs::Tempo));
    }
    if !song.lanes.is_empty() || !song.morphs.is_empty() {
        let at = song.morphs.first().map_or(0, |m| m.at);
        out.push(Difference::song(at, Differs::Morph));
    }
    out
}

/// The copy of `block`'s `clip` for a row from song tick `a` (inside the
/// block): turned round to where the block has got to there, without the
/// one-shot, legato and follow action the song ignores, and the ways it
/// plays differently from the block.
fn section_copy(block: &Placement, clip: &Clip, a: Tick) -> (Clip, Vec<Differs>) {
    let len = clip.length.max(1);
    let into = clip.speed.to_clip((a - block.start) as f64);
    let by = (block.offset + into.round() as Tick).rem_euclid(len);
    // The block carries on into the section: its row starts it again.
    let carried = a > block.start;
    let mut why = Vec::new();
    if clip.direction == Direction::Random {
        why.push(Differs::Random);
    }
    if clip.notes.iter().any(|n| n.chance < 100) {
        why.push(Differs::Chance);
    }
    if by != 0 || carried {
        if into.fract() != 0.0 {
            why.push(Differs::BetweenTicks);
        }
        if clip.looped.is_some() {
            why.push(Differs::LoopCell);
        }
        let direction = match clip.direction {
            Direction::Forward | Direction::Random => false,
            Direction::Reverse => by != 0,
            Direction::PingPong => true,
        };
        if direction {
            why.push(Differs::Direction);
        }
        let counts = |c: Cond| matches!(c, Cond::Every { .. } | Cond::First | Cond::NotFirst);
        let follows = |c: Cond| matches!(c, Cond::Pre | Cond::NotPre | Cond::Nei | Cond::NotNei);
        if clip
            .notes
            .iter()
            .any(|n| counts(n.cond) || (carried && follows(n.cond)))
        {
            why.push(Differs::Conditions);
        }
        if carried && clip.program.is_some() {
            why.push(Differs::Program);
        }
        if carried && held_over(block, clip, a) {
            why.push(Differs::HeldNote);
        }
    }
    let mut copy = turned(clip, by);
    copy.once = false;
    copy.legato = false;
    copy.follow = None;
    (copy, why)
}

/// Whether a note of `block`'s `clip` that played sounds over song tick
/// `a` (swing aside), or one tied or slid into on `a` joins one before.
fn held_over(block: &Placement, clip: &Clip, a: Tick) -> bool {
    let origin = block.origin(clip) as f64;
    let pass = clip.speed.to_song(clip.length.max(1) as f64);
    let a = a as f64;
    clip.notes.iter().any(|n| {
        let t = clip.speed.to_song(n.tick as f64);
        // Its last start before `a`, and on `a`.
        let k = ((a - origin - t) / pass).ceil() - 1.0;
        let before = origin + k * pass + t;
        let on = before + pass;
        let held = before >= block.start as f64 && before + clip.speed.to_song(n.len as f64) > a;
        held || (n.join != Join::Attack && (on - a).abs() < 0.5)
    })
}

/// `clip` turned round by `by` of its ticks: what played from its tick
/// `by` plays from its top, so launched it plays as it played from `by`.
/// Its notes, locks and moves turn (and those it was mutated from); its
/// pressure goes with its notes as it is. A ramp of a move cut at `by`
/// gets a point on each side of the cut, on its line.
pub fn turned(clip: &Clip, by: Tick) -> Clip {
    let len = clip.length.max(1);
    let by = by.rem_euclid(len);
    let mut c = clip.clone();
    if by == 0 {
        return c;
    }
    let turn = |t: Tick, len: Tick| (t - by).rem_euclid(len);
    for n in &mut c.notes {
        n.tick = turn(n.tick, len);
    }
    for l in &mut c.locks {
        l.tick = turn(l.tick, len);
    }
    c.moves = turned_moves(&clip.moves, by, len);
    if let Some(o) = c.original.as_mut() {
        let olen = o.length.max(1);
        for n in &mut o.notes {
            n.tick = turn(n.tick, olen);
        }
        for l in &mut o.locks {
            l.tick = turn(l.tick, olen);
        }
        o.moves = turned_moves(&o.moves, by.rem_euclid(olen), olen);
    }
    c
}

/// `moves` of a clip `len` ticks long, turned round by `by` (0 < by < len).
fn turned_moves(moves: &Moves, by: Tick, len: Tick) -> Moves {
    let mut out = Moves::new();
    let (by, len) = (by as u32, len.max(1) as u32);
    for lane in moves.lanes() {
        let mut points: Vec<Point> = lane
            .points
            .iter()
            .map(|p| Point {
                tick: (p.tick + len - by) % len,
                ..*p
            })
            .collect();
        // A ramp gliding over `by` (from a point before it to one after,
        // in the same pass): the turned lane can't glide over its end, so
        // it ends on the tick before the cut and starts again on it.
        let i = lane.points.partition_point(|p| p.tick < by);
        let gliding = i > 0
            && lane.points[i - 1].kind == Kind::Ramp
            && lane
                .points
                .get(i)
                .is_some_and(|q| q.tick > by && q.kind != Kind::End);
        if gliding {
            if let Some(u) = lane.state_at(f64::from(by)) {
                points.push(Point::ramp(0, u));
            }
            if lane.points[i - 1].tick + 1 < by {
                if let Some(u) = lane.state_at(f64::from(by - 1)) {
                    points.push(Point::ramp(len - 1, u));
                }
            }
        }
        points.sort_by_key(|p| p.tick);
        // Room was checked when the clip's lanes were made.
        let _ = out.restore(lane.target, lane.map, 0, len, &points);
    }
    out
}

#[cfg(test)]
mod tests {

    /// A song's tempo points (tempo ramps): kept in order a 1/16 apart,
    /// one in place of another on the same tick, and the tempo map they
    /// make with the project's tempo before the first.
    #[test]
    fn tempo_points_keep_their_order_and_make_the_songs_tempo_map() {
        use mpfree_transport::time::RAMP_STEP;
        let mut a = Arrangement::default();
        let map = a.tempo_map(92.0).unwrap();
        assert_eq!(map.segments().len(), 1);
        assert_eq!(map.bpm_at(100_000), 92.0);
        a.set_tempo(7_680, 140.0, None).unwrap();
        a.set_tempo(3_840, 100.0, Some(true)).unwrap();
        assert_eq!(
            a.tempo
                .iter()
                .map(|p| (p.at, p.bpm, p.ramp))
                .collect::<Vec<_>>(),
            [(3_840, 100.0, true), (7_680, 140.0, false)]
        );
        // Bar 1 plays the project's tempo, then 100 gliding to 140.
        let map = a.tempo_map(92.0).unwrap();
        assert_eq!(map.segments().len(), 3);
        assert!(!map.is_ramp(0) && map.is_ramp(1) && !map.is_ramp(2));
        assert_eq!(map.bpm_at(0), 92.0);
        assert_eq!(map.bpm_at(5_760), 120.0);
        assert_eq!(map.next_change_after(3_840), Some(3_840 + RAMP_STEP));
        // On the same tick: the tempo changes, the ramp stays.
        a.set_tempo(3_840, 110.0, None).unwrap();
        assert_eq!((a.tempo[0].bpm, a.tempo[0].ramp), (110.0, true));
        // A point on bar 1 takes the project's place.
        a.set_tempo(0, 80.0, None).unwrap();
        let map = a.tempo_map(92.0).unwrap();
        assert_eq!(map.segments().len(), 3);
        assert_eq!(map.bpm_at(-960), 80.0);
        // A ramp on the last point waits for a point after it.
        a.set_tempo(7_680, 140.0, Some(true)).unwrap();
        assert!(!a.tempo_map(92.0).unwrap().is_ramp(2));
        a.check(&[]).unwrap();
        // Refused, changing nothing.
        let before = a.clone();
        for (at, bpm) in [
            (-1, 90.0),
            (960, 19.0),
            (960, 401.0),
            (3_841, 90.0),
            (3_700, 90.0),
        ] {
            assert_eq!(
                a.set_tempo(at, bpm, None),
                Err(ModelError::BadTempo),
                "{at} {bpm}"
            );
        }
        assert_eq!(a, before);
        assert!(a.remove_tempo(3_840) && !a.remove_tempo(3_840));
        assert_eq!(a.tempo.len(), 2);
        // No more than the tempo map holds.
        let mut full = Arrangement::default();
        for k in 0..MAX_TEMPO_POINTS {
            full.set_tempo(960 * (k as Tick + 1), 120.0, None).unwrap();
        }
        assert_eq!(full.set_tempo(0, 90.0, None), Err(ModelError::BadTempo));
        assert_eq!(full.tempo_map(92.0).unwrap().segments().len(), MAX_SEGMENTS);
        // Points out of order don't pass the check.
        full.tempo.swap(0, 1);
        assert_eq!(full.check(&[]), Err(ModelError::BadTempo));
    }

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

    /// The song list: each step's span is its scene's length times its
    /// times; List → song lays them out in order, a scene that comes
    /// twice twice; the list checks its steps.
    #[test]
    fn the_song_list_has_spans_and_lays_out_as_a_song() {
        use crate::scenes::{ListStep, MAX_TIMES};
        let mut p = project();
        let step = |row, times| ListStep { row, times };
        p.arrangement.list.steps = vec![step(0, 2), step(1, 1), step(0, 3)];
        assert_eq!(
            list_spans(&p, BAR),
            vec![(0, 2 * BAR), (2 * BAR, 2 * BAR), (4 * BAR, 3 * BAR)]
        );
        let song = list_to_song(&p, 8 * BAR, BAR).unwrap().song;
        let blocks: Vec<(usize, Tick, Tick)> = song
            .placements
            .iter()
            .filter(|b| b.track == 0)
            .map(|b| (b.slot().unwrap(), b.start, b.length))
            .collect();
        assert_eq!(
            blocks,
            vec![
                (0, 8 * BAR, 2 * BAR),
                (1, 10 * BAR, 2 * BAR),
                (0, 12 * BAR, 3 * BAR)
            ]
        );
        assert_eq!(song.markers.len(), 3);
        assert_eq!(p.check(), Ok(()));
        // Round: after the last, the first again; else none.
        assert_eq!(p.arrangement.list.after(1), Some(2));
        assert_eq!(p.arrangement.list.after(2), None);
        p.arrangement.list.looped = true;
        assert_eq!(p.arrangement.list.after(2), Some(0));
        for bad in [step(SCENES, 1), step(0, 0), step(0, MAX_TIMES + 1)] {
            p.arrangement.list.steps = vec![bad];
            assert_eq!(p.check(), Err(ModelError::BadScene), "{bad:?}");
        }
    }

    #[test]
    fn scenes_become_a_song_and_the_song_becomes_scenes() {
        let mut p = project();
        p.scene_mut(1).unwrap().name = "Chorus".into();
        p.scene_mut(1).unwrap().follow = Some(Follow::after(4, FollowAction::Next));
        assert_eq!(scene_length(&p, 0, BAR), BAR);
        assert_eq!(scene_length(&p, 1, BAR), 4 * BAR, "its follow's bars");
        let song = scenes_to_song(&p, &[(0, BAR), (1, 4 * BAR), (0, BAR)], 0)
            .unwrap()
            .song;
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
        let rows = song_to_scenes(&mut p, BAR).unwrap().rows;
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
        assert_eq!(
            song_to_scenes(&mut p, BAR).map(|c| c.rows),
            Ok(vec![5, 6, 7])
        );
        let mut q = before;
        q.tracks[0].slots[6] = Some(Clip::new("x", BAR));
        q.tracks[0].slots[11] = Some(Clip::new("x", BAR));
        let again = q.clone();
        assert_eq!(
            song_to_scenes(&mut q, BAR).map(|c| c.rows),
            Err(ModelError::NoRoom)
        );
        assert_eq!(q, again, "nothing changed");
    }

    /// Song → scenes takes only truly empty rows (`Project::row_empty`):
    /// a row with no clip but a name, mutes, a follow, morph values or a
    /// program change is a row someone set up, and is skipped.
    #[test]
    fn song_to_scenes_never_takes_a_row_set_up_without_clips() {
        use crate::mpfree_param::{Owner, ParamAddr};
        let mut p = project();
        p.arrangement = scenes_to_song(&p, &[(0, BAR), (1, 2 * BAR)], 0)
            .unwrap()
            .song;
        assert_eq!(sections(&p).len(), 2);
        assert!(p.row_empty(2) && !p.row_empty(0), "no clip, a plain scene");
        p.scene_mut(2).unwrap().name = "Intro".into();
        p.scene_mut(3).unwrap().mutes = Some(0);
        p.scene_mut(4).unwrap().follow = Some(Follow::after(2, FollowAction::Next));
        p.scene_mut(5)
            .unwrap()
            .values
            .push((ParamAddr::new(Owner::Inst(0), 1), 0.5));
        let pc = crate::ClipProgram {
            program: 3,
            bank: None,
        };
        p.scene_mut(6).unwrap().set_program(1, Some(pc)).unwrap();
        p.scene_mut(8).unwrap().name = "Outro".into();
        for r in 2..=6 {
            assert!(!p.row_empty(r), "row {r} is set up");
        }
        // Rows 2 to 6 are set up and 8 is named: the first run of two
        // empty rows is 9 and 10 (7 alone is too short).
        assert_eq!(song_to_scenes(&mut p, BAR).map(|c| c.rows), Ok(vec![9, 10]));
        assert_eq!(p.scene(2).unwrap().name, "Intro", "left as it was");
        assert_eq!(p.scene(3).unwrap().mutes, Some(0));
        assert!(p.scene(5).unwrap().values.len() == 1);
        assert_eq!(p.scene(6).unwrap().program(1), Some(pc));
    }

    /// A clip turned round: notes and locks by their ticks, a move's
    /// ramp cut at the turn ending and starting again on its line, and
    /// what it was mutated from turned with it.
    #[test]
    fn a_clip_turns_round_to_play_from_partway_in() {
        use crate::mpfree_param::{OnTrack, RelAddr};
        use crate::Note;
        let mut c = Clip::new("c", 4 * BAR);
        c.notes = (0..4)
            .map(|k| Note::new(k * BAR, 100, 10 + k as u8, 100))
            .collect();
        c.locks.push(crate::Lock::new(
            3 * BAR,
            Some(13),
            RelAddr::new(OnTrack::Inst, 1),
            0.5,
        ));
        let lane = MORPH_LANE;
        let map = Map::linear(0.0, 1.0);
        let len = (4 * BAR) as u32;
        // A ramp from 0.0 on bar 2 to 1.0 on bar 4 glides over bar 3.
        let pts = [
            Point::ramp(BAR as u32, 0.0),
            Point::step(3 * BAR as u32, 1.0),
        ];
        c.moves.restore(lane, map, 0, len, &pts).unwrap();
        c.original = Some(Box::new(crate::Original {
            length: 4 * BAR,
            notes: c.notes.clone(),
            locks: vec![],
            moves: Moves::new(),
            variation: 1,
        }));
        assert_eq!(turned(&c, 0), c);
        let t = turned(&c, 2 * BAR);
        let ticks: Vec<(Tick, u8)> = t.notes.iter().map(|n| (n.tick, n.note)).collect();
        assert_eq!(ticks, [(2 * BAR, 10), (3 * BAR, 11), (0, 12), (BAR, 13)]);
        assert_eq!(t.locks[0].tick, BAR);
        let o = t.original.as_ref().unwrap();
        assert_eq!(o.notes[2].tick, 0, "its original turns too");
        let lane_of = |c: &Clip| c.moves.lane(lane).unwrap().clone();
        let (before, after) = (lane_of(&c), lane_of(&t));
        // The same value wherever it plays, but the tick before the turn.
        for k in 0..len / 64 {
            let at = f64::from(k * 64);
            let from = (at + 2.0 * BAR as f64) % f64::from(len);
            if from as u32 == 2 * BAR as u32 - 1 {
                continue;
            }
            let (a, b) = (after.state_at(at), before.state_at(from));
            assert!(
                a.zip(b).map_or(a == b, |(a, b)| (a - b).abs() < 1e-4),
                "tick {at}: {a:?} {b:?}"
            );
        }
        assert_eq!(after.state_at(0.0), Some(0.5), "it glides on from the turn");
        assert_eq!(t.check(), Ok(()));
    }

    /// Song → scenes from a song in the exact set: a section that starts
    /// halfway into a four-bar block gets the clip turned round to there;
    /// the copies leave off one-shot, legato and follow; nothing differs.
    #[test]
    fn song_to_scenes_turns_each_clip_to_where_its_section_starts() {
        use crate::Note;
        let mut p = project();
        let mut four = Clip::new("four", 4 * BAR);
        four.notes = (0..4)
            .map(|k| Note::new(k * BAR, 100, 10 + k as u8, 100))
            .collect();
        four.once = true;
        four.legato = true;
        four.follow = Some(Follow::after(2, FollowAction::Stop));
        p.tracks[0].slots[2] = Some(four);
        let s = &mut p.arrangement;
        // Drums: the four-bar clip for 8 bars; bass joins on bar 3.
        s.place(Placement::linked(0, 2, 0, 8 * BAR)).unwrap();
        s.place(Placement::linked(1, 0, 2 * BAR, 6 * BAR)).unwrap();
        p.give_clip_ids();
        let had = p.next_clip;
        let done = song_to_scenes(&mut p, BAR).unwrap();
        assert_eq!(done.rows, [3, 4]);
        assert_eq!(done.differences, [], "exact");
        // Each copy a clip of its own, with a new id (Arrange 1.1).
        let ids: Vec<u32> = [(0, 3), (0, 4), (1, 4)]
            .map(|(t, r)| p.clip(t, r).unwrap().id.0)
            .to_vec();
        assert_eq!(ids, [had, had + 1, had + 2]);
        let copy = p.tracks[0].slots[4].as_ref().unwrap();
        let ticks: Vec<(Tick, u8)> = copy.notes.iter().map(|n| (n.tick, n.note)).collect();
        assert_eq!(ticks, [(2 * BAR, 10), (3 * BAR, 11), (0, 12), (BAR, 13)]);
        assert!(!copy.once && !copy.legato && copy.follow.is_none());
        let first = p.tracks[0].slots[3].as_ref().unwrap();
        assert_eq!(first.notes[0].tick, 0, "the first row's starts at its top");
        assert_eq!(p.tracks[1].slots[4].as_ref().unwrap().name, "a");
        assert_eq!(
            p.scene(4).unwrap().follow,
            Some(Follow::after(6, FollowAction::Stop))
        );
    }

    /// Outside the exact set Song → scenes still converts, and names each
    /// difference by track and bar.
    #[test]
    fn song_to_scenes_names_each_difference_by_track_and_bar() {
        use crate::{ClipProgram, Note, Speed};
        // A song of one four-bar block on drums from `start` for 8 bars,
        // with the bass joining on bar 3 (so drums carry on into it).
        let song = |clip: Clip, start: Tick| {
            let mut p = project();
            p.tracks[0].slots[2] = Some(clip);
            let s = &mut p.arrangement;
            s.place(Placement::linked(0, 2, start, 8 * BAR)).unwrap();
            s.place(Placement::linked(1, 0, start + 2 * BAR, 6 * BAR))
                .unwrap();
            p
        };
        let four = || {
            let mut c = Clip::new("four", 4 * BAR);
            c.notes = (0..4).map(|k| Note::new(k * BAR, 100, 10, 100)).collect();
            c
        };
        let diffs = |mut p: Project| -> Vec<(Option<usize>, Tick, Differs)> {
            song_to_scenes(&mut p, BAR)
                .unwrap()
                .differences
                .iter()
                .map(|d| (d.track, d.at, d.what))
                .collect()
        };
        let on_drums = |what| vec![(Some(0), 2 * BAR, what)];
        // Random: every copy (the first row's too).
        let mut c = four();
        c.direction = Direction::Random;
        assert_eq!(
            diffs(song(c, 0)),
            [
                (Some(0), 0, Differs::Random),
                (Some(0), 2 * BAR, Differs::Random)
            ]
        );
        let mut c = four();
        c.notes[1].chance = 50;
        assert_eq!(diffs(song(c, 0)).len(), 2, "chance: each copy");
        // From partway in.
        let mut c = four();
        c.direction = Direction::Reverse;
        assert_eq!(diffs(song(c, 0)), on_drums(Differs::Direction));
        let mut c = four();
        c.notes[2].cond = Cond::Every { a: 1, b: 2 };
        assert_eq!(diffs(song(c, 0)), on_drums(Differs::Conditions));
        let mut c = four();
        c.notes[2].cond = Cond::Pre;
        assert_eq!(diffs(song(c, 0)), on_drums(Differs::Conditions));
        let mut c = four();
        c.program = Some(ClipProgram {
            program: 4,
            bank: None,
        });
        assert_eq!(diffs(song(c, 0)), on_drums(Differs::Program));
        // A note on bar 2 held for two bars, over bar 3.
        let mut c = four();
        c.notes[1].len = 2 * BAR;
        assert_eq!(diffs(song(c, 0)), on_drums(Differs::HeldNote));
        let mut c = four();
        c.notes[2].join = Join::Tie;
        assert_eq!(diffs(song(c, 0)), on_drums(Differs::HeldNote), "a tie");
        let mut p = song(four(), 0);
        let mut loops = Track::new_loops("loops", 2);
        loops.slots[2] = Some(Clip::new_loop(0, 4 * BAR));
        p.tracks[0] = loops;
        assert_eq!(diffs(p), on_drums(Differs::LoopCell));
        // At 3/4 speed, a section 2 bars and a tick in is 5,760.75 of the
        // clip's ticks in: between two of them (and off the bar line).
        let mut c = four();
        c.speed = Speed::new(3, 4).unwrap();
        let mut p = song(c, 0);
        p.arrangement.placements[1].start = 2 * BAR + 1;
        p.arrangement.placements[1].length = 6 * BAR - 1;
        let d = diffs(p);
        assert!(
            d.contains(&(Some(0), 2 * BAR + 1, Differs::BetweenTicks)),
            "{d:?}"
        );
        assert!(d.contains(&(None, 0, Differs::OffBar)), "{d:?}");
        // The whole song: a lead-in and a gap of silence, a tempo point,
        // the morph's lane.
        let mut p = song(four(), BAR);
        p.arrangement
            .place(Placement::linked(1, 0, 11 * BAR, BAR))
            .unwrap();
        p.arrangement.set_tempo(4 * BAR, 100.0, None).unwrap();
        let rows = [MorphRows { at: 0, a: 0, b: 1 }];
        p.arrangement
            .put_morph(&[(0.0, 0.5, Kind::Ramp)], &rows, 0, BAR)
            .unwrap();
        assert_eq!(
            diffs(p),
            [
                (None, 0, Differs::Silence { length: BAR }),
                (None, 0, Differs::Morph),
                (None, 4 * BAR, Differs::Tempo),
                (None, 9 * BAR, Differs::Silence { length: 2 * BAR }),
            ]
        );
        // Longer than a follow's most bars.
        let mut p = project();
        p.arrangement
            .place(Placement::linked(0, 0, 0, 1000 * BAR))
            .unwrap();
        assert_eq!(diffs(p), [(None, 0, Differs::TooLong)]);
        // In words.
        let p = project();
        let d = Difference::on(1, 8 * BAR, Differs::HeldNote);
        assert_eq!(
            d.said(&p, BAR),
            "bass, bar 9: a note held over the row change is cut there"
        );
        let d = Difference::song(0, Differs::Silence { length: 2 * BAR });
        assert_eq!(d.said(&p, BAR), "bar 1: 2 bars of silence left out");
    }

    /// Scenes → song carries the scenes' mutes from row to row (a muted
    /// track gets no block), ends a one-shot's block with its pass, and
    /// names what it can't carry: a scene's program changes, a clip's
    /// follow action and a legato clip after another.
    #[test]
    fn scenes_to_song_carries_mutes_and_one_shots_and_names_the_rest() {
        let mut p = project();
        for r in 2..5 {
            p.tracks[0].slots[r] = Some(Clip::new("d", BAR));
            p.tracks[1].slots[r] = Some(Clip::new("b", BAR));
        }
        // Row 2 mutes the bass; row 3 leaves the mutes; row 4 unmutes.
        p.scene_mut(2).unwrap().mutes = Some(0b10);
        p.scene_mut(4).unwrap().mutes = Some(0);
        let pc = crate::ClipProgram {
            program: 3,
            bank: None,
        };
        p.scene_mut(4).unwrap().set_program(0, Some(pc)).unwrap();
        // A one-shot of one bar in a 4-bar row; a follow; a legato clip
        // after the drums' row-3 clip.
        let d = |r: usize, p: &mut Project| p.tracks[0].slots[r].as_mut().unwrap().clone();
        let mut once = d(2, &mut p);
        once.once = true;
        p.tracks[0].slots[2] = Some(once);
        p.tracks[0].slots[3].as_mut().unwrap().follow = Some(Follow::after(1, FollowAction::Stop));
        p.tracks[0].slots[4].as_mut().unwrap().legato = true;
        let done = scenes_to_song(&p, &[(2, 4 * BAR), (3, BAR), (4, BAR)], 0).unwrap();
        let blocks: Vec<(usize, usize, Tick, Tick)> = done
            .song
            .placements
            .iter()
            .map(|b| (b.track, b.slot().unwrap(), b.start, b.length))
            .collect();
        assert_eq!(
            blocks,
            [
                (0, 2, 0, BAR),
                (0, 3, 4 * BAR, BAR),
                (0, 4, 5 * BAR, BAR),
                (1, 4, 5 * BAR, BAR),
            ]
        );
        let what: Vec<(Option<usize>, Tick, Differs)> = done
            .differences
            .iter()
            .map(|d| (d.track, d.at, d.what))
            .collect();
        assert_eq!(
            what,
            [
                (Some(0), 4 * BAR, Differs::ClipFollow),
                (None, 5 * BAR, Differs::SceneProgram),
                (Some(0), 5 * BAR, Differs::Legato),
            ]
        );
        // After a one-shot that ended, a legato clip has nothing to take
        // over.
        let rows = [(2, 4 * BAR), (4, BAR)];
        let done = scenes_to_song(&p, &rows, 0).unwrap();
        assert!(!done.differences.iter().any(|d| d.what == Differs::Legato));
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

    /// A loop's block stays on its Loop track's lane and a clip's block
    /// off a Loop track's (loop-tracks.md part 5); a loop's own copy
    /// anywhere else doesn't check.
    #[test]
    fn a_loops_block_stays_on_its_loop_tracks_lane() {
        let mut p = project();
        let mut a = Track::new_loops("Loops A", 2);
        a.slots[0] = Some(Clip::new_loop(0, BAR));
        p.tracks.push(a);
        p.tracks.push(Track::new_loops("Loops B", 3));
        let mut s = Arrangement::default();
        s.place(Placement::linked(0, 0, 0, BAR)).unwrap();
        s.place(Placement::linked(2, 0, 0, BAR)).unwrap();
        let before = s.clone();
        for (i, to) in [(1, 0), (1, 3), (0, 2)] {
            for copy in [false, true] {
                assert_eq!(
                    s.move_placement(&p.tracks, i, to, 4 * BAR, copy),
                    Err(ModelError::BadLoop)
                );
                assert_eq!(s, before);
            }
        }
        // Along its own lane it moves as any block does.
        assert_eq!(s.move_placement(&p.tracks, 1, 2, 2 * BAR, false), Ok(1));
        assert_eq!(s.check(&p.tracks), Ok(()));
        // Its own copy on a note track doesn't check, nor a clip's on a
        // Loop track.
        let mut bad = s.clone();
        bad.placements[1].track = 0;
        bad.placements[1].start = 4 * BAR;
        bad.placements[1].source = Source::Own(Box::new(Clip::new_loop(0, BAR)));
        bad.placements.sort_by_key(|p| (p.track, p.start));
        assert_eq!(bad.check(&p.tracks), Err(ModelError::BadLoop));
        let mut bad = s.clone();
        bad.placements[1].source = Source::Own(Box::new(Clip::new("a", BAR)));
        assert_eq!(bad.check(&p.tracks), Err(ModelError::BadLoop));
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

    /// A section move keeps the song list (it names rows, not ticks).
    #[test]
    fn a_section_move_keeps_the_song_list() {
        let mut p = sections_song();
        p.arrangement
            .list
            .steps
            .push(crate::scenes::ListStep { row: 2, times: 3 });
        p.arrangement.list.looped = true;
        let list = p.arrangement.list.clone();
        p.arrangement
            .move_section(&p.tracks, 1, 0, false, BAR)
            .unwrap();
        assert_eq!(p.arrangement.list, list);
    }

    /// Tempo points don't move with a section yet: a song with them
    /// keeps its sections where they are.
    #[test]
    fn sections_with_tempo_points_stay_put() {
        let mut p = sections_song();
        p.arrangement.set_tempo(BAR, 100.0, None).unwrap();
        let before = p.arrangement.clone();
        assert_eq!(
            p.arrangement.move_section(&p.tracks, 1, 0, false, BAR),
            Err(ModelError::BadMoves)
        );
        assert_eq!(p.arrangement, before);
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
