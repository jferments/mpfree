//! The arrangement (roadmap T13): clips on a timeline.
//!
//! A [`Placement`] (a song block) puts a clip on a track's lane from a
//! song tick for a length. It names its clip by its lasting id
//! ([`ClipId`], the Arrange refactor's D1), wherever the clip lives: in a
//! cell of the grid, or among the song's own clips ([`SongClips`]). An
//! edit to the clip shows in every block of it, the way a pattern plays
//! wherever the song uses it, and a clip moved to another cell (or its
//! row dragged) keeps its id, so its blocks stay linked without being
//! touched. A block plays through its own lane's track: a clip of the
//! drums placed on the bass lane sounds through the bass.
//! [`Project::make_unique`] gives a block a copy of its own among the
//! song's clips, with a new id, to change on its own. A block longer
//! than its clip loops it; `offset` starts it part way in. [`Marker`]s
//! name places in the song. Deleting a clip the song plays moves it to
//! the song's own clips, so the song plays on (`mpfree-play`'s takes); a
//! block whose clip is gone (an undone capture, say) stays and plays
//! nothing until its clip comes back.
//!
//! Scenes and the arrangement convert both ways:
//!
//! - Scenes → song and List → song play the rows offline through Rec song
//!   in the app's time engine (the Arrange refactor's 4.1, plan D7:
//!   `mpfree-app`'s `time/offline.rs`), so whatever they do when they play
//!   comes out right. [`scenes_to_song`] lays rows by rule instead, for
//!   what can't run the engine (the screens' preview songs, tests): one
//!   after another, each as long as [`scene_length`] (its follow's bars,
//!   else its longest clip in whole bars), as linked placements, a marker
//!   on each named after the scene. The scenes' mutes carry from row to
//!   row as they do when the rows play (a muted track's clip is left out),
//!   a one-shot clip's block ends with its pass, and what only playing
//!   shows is named (a clip's follow action, legato).
//! - [`song_to_scenes`]: each section of the song (between the places
//!   anything starts or ends, and the markers) becomes a row in a run of
//!   empty rows, chained by follow actions (Next after the section's
//!   bars; the last one stops), so launching the first row plays the
//!   song. Each clip is copied turned round to where the section starts
//!   in it ([`turned`]), without the one-shot, legato and follow action a
//!   song block ignores, so the row plays as the block did.
//!   The song's audio comes too (audio tracks phase 2, step 2d;
//!   [`SongSound`]): each audio lane's regions in a section become an
//!   audio clip in the section's row, cut at its edges as the audio
//!   tracks cut a region (each part keeping exactly its share of its
//!   fades, and the part after a cut playing on from the frame the cut
//!   was at), so rows played in order run on sample for sample; a block
//!   of an audio clip turns round as any clip does, its region across
//!   the turn cut in two. A lane's regions make a section where nothing
//!   else plays, but don't cut one (their first start and last end do).
//!   A section a clip can't hold exactly is refused before anything
//!   changes ([`ScenesRefused`]): more than [`MAX_CLIP_REGIONS`] regions
//!   on a lane, or regions and a block on one lane.
//!
//! Every conversion is exact or says how it is not (plan D7): each way
//! the result plays differently is a [`Difference`], by track and bar.
//!
//! The Song view's drags (Jay, 14 Nov: drag and drop) edit it in place:
//! [`Arrangement::move_placement`] moves or copies a placement along its
//! lane or onto another track's (it stays linked to its clip),
//! [`Arrangement::set_length`] makes one longer or shorter, and
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
use crate::scenes::{SongList, MAX_TIMES};
use crate::time_edit::{Refused, TimeEdit};
use crate::{
    AudioClip, Clip, ClipId, ClipRegion, Cond, Direction, Follow, FollowAction, Join, ModelError,
    Project, Scene, Track, TrackId, MAX_CLIP_REGIONS, MAX_TRACKS, ROWS, SCENES, SLOTS,
};
use mpfree_context::chord::Heard;
use mpfree_context::harmony::RowHarmony;
use mpfree_transport::time::{
    valid_bpm, MapError, Segment, TempoMap, Tick, MAX_SEGMENTS, MIN_SPAN_TICKS,
};
use std::sync::Arc;

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

/// A clip on the timeline (a song block): see the module notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The lane it is on: the track it plays through.
    pub track: usize,
    /// The song tick it starts on.
    pub start: Tick,
    /// How long it plays, in song ticks.
    pub length: Tick,
    /// Where in the clip it starts, in the clip's own ticks (0: its top).
    pub offset: Tick,
    /// The clip it plays, by its lasting id, in a cell or among the
    /// song's own clips. [`ClipId::NONE`], or an id no clip has now: it
    /// plays nothing.
    pub clip: ClipId,
}

impl Placement {
    /// Clip `clip` on `track`'s lane from `start` for `length` song ticks.
    pub fn new(track: usize, clip: ClipId, start: Tick, length: Tick) -> Placement {
        Placement {
            track,
            start,
            length,
            offset: 0,
            clip,
        }
    }

    /// A block of the clip in cell `row` of `track` (in `tracks`), on that
    /// track's lane from `start` for `length` song ticks: it names the
    /// clip's id. An empty cell's block, or one of a clip with no id yet,
    /// names none and plays nothing.
    pub fn of(tracks: &[Track], track: usize, row: usize, start: Tick, length: Tick) -> Placement {
        let clip = tracks
            .get(track)
            .and_then(|t| t.slots.get(row)?.as_ref())
            .map_or(ClipId::NONE, |c| c.id);
        Placement::new(track, clip, start, length)
    }

    pub fn end(&self) -> Tick {
        self.start + self.length
    }

    pub fn contains(&self, tick: Tick) -> bool {
        (self.start..self.end()).contains(&tick)
    }

    /// The clip it plays, wherever it lives: its own lane's cells are
    /// looked through first, then the song's clips, then the other
    /// tracks'. None when no clip has its id. Allocates nothing.
    pub fn clip<'a>(&self, tracks: &'a [Track], song: &'a SongClips) -> Option<&'a Clip> {
        let id = self.clip;
        if !id.is_given() {
            return None;
        }
        let on = |t: &'a Track| t.slots.clips().find(|c| c.id == id);
        tracks
            .get(self.track)
            .and_then(on)
            .or_else(|| song.get(id))
            .or_else(|| {
                tracks
                    .iter()
                    .enumerate()
                    .filter(|&(t, _)| t != self.track)
                    .find_map(|(_, t)| on(t))
            })
    }

    /// The song tick its clip's tick 0 falls on in the pass it starts in:
    /// `start` moved back by `offset` at the clip's speed (to a whole tick).
    pub fn origin(&self, clip: &Clip) -> Tick {
        let back = clip
            .speed
            .to_song(self.offset.rem_euclid(clip.length.max(1)) as f64);
        self.start - back.round() as Tick
    }

    /// What plays of it from song tick `at` on (inside it), from where
    /// its clip (`clip`, None when it has none now) has got to there: the
    /// part after a cut at `at`.
    pub fn from(&self, clip: Option<&Clip>, at: Tick) -> Placement {
        Placement {
            offset: self.offset_by(clip, at - self.start),
            start: at,
            length: self.end() - at,
            ..self.clone()
        }
    }

    /// Its offset `by` song ticks further into its clip (`clip`, None
    /// when it has none now), at the clip's speed and going round the
    /// clip's length: where the clip has got to `by` ticks after its
    /// start (before it, for a negative `by`).
    pub fn offset_by(&self, clip: Option<&Clip>, by: Tick) -> Tick {
        let speed = clip.map_or(crate::Speed::NORMAL, |c| c.speed);
        let length = clip.map_or(1, |c| c.length.max(1));
        let moved = speed.to_clip(by as f64).round() as Tick;
        (self.offset + moved).rem_euclid(length)
    }
}

/// The song's own clips (the Arrange refactor's D1): clips no cell holds
/// that the song's blocks play, a block's copy made unique or a deleted
/// clip the song still plays. Each has its lasting id; they are kept in
/// order of id, none twice. Copies of the song share them until one
/// changes, so a Song view undo step keeps its blocks, not its clips.
#[derive(Clone, Debug, Default)]
pub struct SongClips(Arc<Vec<Clip>>);

impl PartialEq for SongClips {
    fn eq(&self, other: &SongClips) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl SongClips {
    /// The clip with id `id`.
    pub fn get(&self, id: ClipId) -> Option<&Clip> {
        self.at(id).ok().map(|k| &self.0[k])
    }

    /// The clip with id `id`, to change (the song's copies stop sharing).
    pub fn get_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        let k = self.at(id).ok()?;
        Arc::make_mut(&mut self.0).get_mut(k)
    }

    fn at(&self, id: ClipId) -> Result<usize, usize> {
        self.0.binary_search_by_key(&id, |c| c.id)
    }

    /// Whether these are the very clips `other` holds, shared, not a
    /// copy of them.
    pub fn shares(&self, other: &SongClips) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// In order of id.
    pub fn iter(&self) -> std::slice::Iter<'_, Clip> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Keep `clip`, in its place by id. `BadPlacement`, keeping nothing,
    /// for a clip with no id or one a song clip has.
    pub fn keep(&mut self, clip: Clip) -> Result<(), ModelError> {
        if !clip.id.is_given() {
            return Err(ModelError::BadPlacement);
        }
        match self.at(clip.id) {
            Ok(_) => Err(ModelError::BadPlacement),
            Err(k) => {
                Arc::make_mut(&mut self.0).insert(k, clip);
                Ok(())
            }
        }
    }

    /// Take the clip with id `id` out.
    pub fn take(&mut self, id: ClipId) -> Option<Clip> {
        let k = self.at(id).ok()?;
        Some(Arc::make_mut(&mut self.0).remove(k))
    }

    /// Keep only the clips `keep` says to (changing nothing, the copies
    /// still shared, when it keeps them all).
    pub fn retain(&mut self, mut keep: impl FnMut(&Clip) -> bool) {
        if self.0.iter().all(&mut keep) {
            return;
        }
        Arc::make_mut(&mut self.0).retain(keep);
    }

    /// Give each clip with no id, or one `taken` says another clip has,
    /// the next id from `next` (moved on), in order; the rest keep
    /// theirs. How many were given.
    pub(crate) fn give_ids(
        &mut self,
        next: &mut u32,
        mut taken: impl FnMut(ClipId) -> bool,
    ) -> usize {
        if self.0.iter().all(|c| c.id.is_given() && !taken(c.id)) {
            return 0;
        }
        let clips = Arc::make_mut(&mut self.0);
        let mut given = 0;
        for c in clips.iter_mut() {
            if !c.id.is_given() || taken(c.id) {
                c.id = ClipId(*next);
                *next = next.saturating_add(1);
                given += 1;
            }
        }
        clips.sort_by_key(|c| c.id);
        given
    }
}

/// A named place in the song, where a section starts: the song's
/// sections run from a marker to the next ([`song_sections`]). When the
/// song list is linked to the song (the Arrange refactor's phase 4b,
/// plan D8: one song, two views), each section is a step of the list: it
/// plays `passes` times (× N, each pass as long), and a section made from
/// a scene keeps its row in `scene`.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub at: Tick,
    pub name: String,
    /// The scene the section was made from (None: the song's own).
    pub scene: Option<FromScene>,
    /// How many passes the section plays, 1 to [`MAX_TIMES`]: × N on the
    /// list, a line at each pass in the Song view.
    pub passes: u16,
}

/// The scene a section was made from (phase 4b): its row, how long one
/// pass of it was when the section was made, and whether the section has
/// been edited in the song since. Until then it follows the row: its
/// blocks are the row's clips (linked by id, so an edit of a clip on the
/// grid reaches it), and a list edit makes it again from the row as the
/// row is then. Once changed it plays its own blocks, keeping its name
/// and place, and Back to the scene puts the row's clips back.
///
/// The pass is the section's own, not the row's length now: lengthening
/// a clip on the grid moves nothing in the song. It is where the song's
/// last section ends while it follows (no marker after it says).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FromScene {
    pub row: usize,
    /// Song ticks, more than 0.
    pub pass: Tick,
    pub changed: bool,
}

impl FromScene {
    /// Made from row `row` now, a pass `pass` ticks long.
    pub fn following(row: usize, pass: Tick) -> FromScene {
        FromScene {
            row,
            pass,
            changed: false,
        }
    }
}

impl Marker {
    /// A plain marker: the song's own section, one pass.
    pub fn new(at: Tick, name: &str) -> Marker {
        Marker {
            at,
            name: name.to_string(),
            scene: None,
            passes: 1,
        }
    }

    /// Only a place and a name: no scene, one pass (what a marker was
    /// before phase 4b, and all an older build reads).
    pub fn is_plain(&self) -> bool {
        self.scene.is_none() && self.passes == 1
    }
}

/// One step of a linked song list: a section of the song
/// ([`song_sections`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SongSection {
    /// Its marker's index; None for the song's bars before the first
    /// marker (an unnamed step, "Song").
    pub marker: Option<usize>,
    pub start: Tick,
    pub end: Tick,
    pub scene: Option<FromScene>,
    pub passes: u16,
}

impl SongSection {
    /// How long one pass plays.
    pub fn pass(&self) -> Tick {
        (self.end - self.start) / Tick::from(self.passes.max(1))
    }
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
/// track), the clips only the song has, and markers sorted by tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arrangement {
    pub placements: Vec<Placement>,
    /// The clips only the song has ([`SongClips`]).
    pub clips: SongClips,
    pub markers: Vec<Marker>,
    /// The song's own lanes, on song ticks ([`SONG_LANE_LEN`] long, not
    /// looping), for parameters not on a track: the morph
    /// ([`MORPH_LANE`]).
    pub lanes: Moves,
    /// The morph's rows from each tick on, in order.
    pub morphs: Vec<MorphRows>,
    /// The song list (sequencer S4): scenes in order, each so many times,
    /// the third way to see the song beside the grid and the timeline.
    /// It plays on its own (`mpfree-perform`); List → song plays it onto
    /// the timeline (the app's time engine).
    pub list: SongList,
    /// The song's tempo points, in order, at least a 1/16 apart. None:
    /// the project's tempo throughout.
    pub tempo: Vec<TempoPoint>,
}

impl Arrangement {
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// Whether it holds what the Arrange refactor's phase 4b added: a
    /// linked song list, or a marker with its section's scene or passes
    /// (what a build before can't read, `save::FORMAT_4`).
    pub fn uses_sections(&self) -> bool {
        self.list.linked || self.markers.iter().any(|m| !m.is_plain())
    }

    /// Where the last placement ends (0 for none).
    pub fn end(&self) -> Tick {
        self.placements
            .iter()
            .map(Placement::end)
            .max()
            .unwrap_or(0)
    }

    /// The clip block `p` plays ([`Placement::clip`]).
    pub fn clip_of<'a>(&'a self, tracks: &'a [Track], p: &Placement) -> Option<&'a Clip> {
        p.clip(tracks, &self.clips)
    }

    /// Whether a block plays clip `id`.
    pub fn plays(&self, id: ClipId) -> bool {
        id.is_given() && self.placements.iter().any(|p| p.clip == id)
    }

    /// Every block and song clip in range, and each block's clip on a lane
    /// of its kind: a loop's on a Loop track, a clip's off one, an audio
    /// clip's on an audio track and a note clip's off one (a block whose
    /// clip is gone plays nothing and is fine). The cells' clips
    /// are the project's to check.
    pub fn check(&self, tracks: &[Track]) -> Result<(), ModelError> {
        if self.placements.len() > MAX_PLACEMENTS
            || self.markers.len() > MAX_MARKERS
            || self.clips.len() > MAX_PLACEMENTS
        {
            return Err(ModelError::BadPlacement);
        }
        self.list.check()?;
        for c in self.clips.iter() {
            c.check()?;
        }
        for (i, p) in self.placements.iter().enumerate() {
            if p.length <= 0 || p.start < 0 || p.offset < 0 || p.track >= tracks.len() {
                return Err(ModelError::BadPlacement);
            }
            // An audio clip's block on an audio track's lane, and only
            // there (the audio tracks rethink's phase 2, step 2d).
            let clip = self.clip_of(tracks, p);
            if clip.is_some_and(|c| c.is_audio_clip() != tracks[p.track].is_audio()) {
                return Err(ModelError::AudioTrack);
            }
            if clip.is_some_and(|c| c.looped.is_some() != tracks[p.track].is_loops()) {
                return Err(ModelError::BadLoop);
            }
            if let Some(prev) = i.checked_sub(1).map(|j| &self.placements[j]) {
                let order = (prev.track, prev.start) < (p.track, p.start);
                if !order || (prev.track == p.track && prev.end() > p.start) {
                    return Err(ModelError::BadPlacement);
                }
            }
        }
        if self.markers.iter().any(|m| {
            m.at < 0
                || m.passes == 0
                || m.passes > MAX_TIMES
                || m.scene.is_some_and(|s| {
                    s.row >= SCENES || s.pass <= 0 || s.pass > Tick::from(SONG_LANE_LEN)
                })
        }) || self.markers.windows(2).any(|w| w[0].at > w[1].at)
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
                let rest = rest.from(rest.clip(tracks, &self.clips), to);
                if self.placements.len() >= MAX_PLACEMENTS {
                    return Err(ModelError::BadPlacement);
                }
                self.placements.insert(i, rest);
                break;
            }
        }
        self.drop_unplayed();
        Ok(())
    }

    /// Song ticks `from..to` playing `blocks` instead of what played there
    /// (a linked list's section made again from its scene, the Arrange
    /// refactor's 4b.2): every track's blocks there taken out as
    /// [`Arrangement::clear`] takes them, then `blocks` put in. Refused
    /// (`BadPlacement`), the song as it was: a block outside `from..to`,
    /// or one that runs into another.
    pub fn fill(
        &mut self,
        tracks: &[Track],
        from: Tick,
        to: Tick,
        blocks: &[Placement],
    ) -> Result<(), ModelError> {
        if blocks.iter().any(|p| p.start < from || p.end() > to) {
            return Err(ModelError::BadPlacement);
        }
        let mut song = self.clone();
        for t in 0..tracks.len() {
            song.clear(tracks, t, from, to)?;
        }
        for p in blocks {
            song.place(p.clone())?;
        }
        *self = song;
        Ok(())
    }

    /// Let go of the song's clips no block plays now (their last block
    /// deleted or cut away): a Song view undo step keeps them in the song
    /// as it was. Allocates a list of the ids played.
    pub fn drop_unplayed(&mut self) {
        if self.clips.is_empty() {
            return;
        }
        let mut played: Vec<ClipId> = self.placements.iter().map(|p| p.clip).collect();
        played.sort_unstable();
        self.clips.retain(|c| played.binary_search(&c.id).is_ok());
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

    /// Delete block `i`: a song clip it was the last to play goes with
    /// it.
    pub fn remove(&mut self, i: usize) -> Option<Placement> {
        let p = (i < self.placements.len()).then(|| self.placements.remove(i))?;
        if self.clips.get(p.clip).is_some() && !self.plays(p.clip) {
            self.clips.take(p.clip);
        }
        Some(p)
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

    /// Add a marker in its place.
    pub fn mark(&mut self, at: Tick, name: &str) -> Result<(), ModelError> {
        if at < 0 || self.markers.len() >= MAX_MARKERS {
            return Err(ModelError::BadPlacement);
        }
        let i = self.markers.partition_point(|m| m.at <= at);
        self.markers.insert(i, Marker::new(at, name));
        Ok(())
    }

    /// A name for a new marker: the first letter, A to Z, no marker has
    /// as its name, else "Section N" for the markers' count plus one.
    pub fn new_marker_name(&self) -> String {
        ('A'..='Z')
            .map(String::from)
            .find(|n| self.markers.iter().all(|m| &m.name != n))
            .unwrap_or_else(|| format!("Section {}", self.markers.len() + 1))
    }

    /// A marker named `name` added on song tick `at` (the Arrange
    /// refactor's 3.3), and its index. Refused (`BadPlacement`), changing
    /// nothing: before the start, one too many, an empty name, or a tick
    /// a marker has already (a section is never empty).
    pub fn add_marker(&mut self, at: Tick, name: &str) -> Result<usize, ModelError> {
        if name.trim().is_empty() || self.markers.iter().any(|m| m.at == at) {
            return Err(ModelError::BadPlacement);
        }
        self.mark(at, name)?;
        Ok(self.markers.partition_point(|m| m.at < at))
    }

    /// Marker `k` named `name`. Refused (`BadPlacement`) for no such
    /// marker or an empty name.
    pub fn rename_marker(&mut self, k: usize, name: &str) -> Result<(), ModelError> {
        if name.trim().is_empty() {
            return Err(ModelError::BadPlacement);
        }
        let m = self.markers.get_mut(k).ok_or(ModelError::BadPlacement)?;
        m.name = name.to_string();
        Ok(())
    }

    /// Marker `k` moved to song tick `at`, keeping its name, and its index
    /// now (past another marker it changes places with it). Only the
    /// marker moves: its section's edge, not what plays (to move what
    /// plays, move the section). Refused (`BadPlacement`), changing
    /// nothing: no such marker, before the start, or onto another marker.
    pub fn move_marker(&mut self, k: usize, at: Tick) -> Result<usize, ModelError> {
        if k >= self.markers.len()
            || at < 0
            || self
                .markers
                .iter()
                .enumerate()
                .any(|(j, m)| j != k && m.at == at)
        {
            return Err(ModelError::BadPlacement);
        }
        let mut m = self.markers.remove(k);
        m.at = at;
        let i = self.markers.partition_point(|n| n.at < at);
        self.markers.insert(i, m);
        Ok(i)
    }

    /// Marker `k` taken away, and returned: its bars join the section
    /// before. `BadPlacement` for no such marker.
    pub fn remove_marker(&mut self, k: usize) -> Result<Marker, ModelError> {
        if k >= self.markers.len() {
            return Err(ModelError::BadPlacement);
        }
        Ok(self.markers.remove(k))
    }

    /// Placement `i` moved to `track` from `start`, or with `copy` a copy
    /// of it put there and the placement left as it is (Jay, 14 Nov: drag
    /// and drop on the Song view). Either way it stays linked to its clip,
    /// on another track's lane too, and plays through that track (the
    /// Arrange refactor's D1). Refused (`BadPlacement`), changing nothing,
    /// when it would overlap a placement there or start before 0; and
    /// (`BadLoop`) when it goes to another track and either is a Loop
    /// track: a loop's block stays on its Loop track's lane, as its cell
    /// stays in its column (loop-tracks.md); and (`AudioTrack`) when it
    /// goes between an audio track and another kind: an audio clip's block
    /// stays on audio tracks' lanes. Its index now.
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
        // An audio clip's block goes only to another audio track's lane,
        // and a note clip's never onto one (phase 2, step 2d).
        let audio = |t: usize| tracks.get(t).is_some_and(Track::is_audio);
        if audio(track) != audio(p.track) {
            return Err(ModelError::AudioTrack);
        }
        let mut moved = p.clone();
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

    /// Block `i` cut in two at song tick `at` (Split, the Arrange
    /// refactor's phase 6): the part before keeps its place, and the part
    /// after starts at `at` from where its clip had got to there, so the
    /// song plays as it did. Both play the same clip. Refused
    /// (`BadPlacement`), changing nothing: no such block, `at` not inside
    /// it (its start or end is no cut), or a song full of blocks. The
    /// part after's index.
    pub fn split(&mut self, tracks: &[Track], i: usize, at: Tick) -> Result<usize, ModelError> {
        let p = self.placements.get(i).ok_or(ModelError::BadPlacement)?;
        if at <= p.start || at >= p.end() || self.placements.len() >= MAX_PLACEMENTS {
            return Err(ModelError::BadPlacement);
        }
        let rest = p.from(p.clip(tracks, &self.clips), at);
        self.placements[i].length = at - self.placements[i].start;
        self.placements.insert(i + 1, rest);
        Ok(i + 1)
    }

    /// Every block across song tick `at` split there, on every lane
    /// (Split at the playhead with no block picked). All or none: refused
    /// (`BadPlacement`) when the blocks would be too many, the song as it
    /// was. How many were split.
    pub fn split_all(&mut self, tracks: &[Track], at: Tick) -> Result<usize, ModelError> {
        let mut song = self.clone();
        let mut split = 0;
        let mut i = 0;
        while i < song.placements.len() {
            let p = &song.placements[i];
            if p.start < at && at < p.end() {
                i = song.split(tracks, i, at)?;
                split += 1;
            }
            i += 1;
        }
        *self = song;
        Ok(split)
    }

    /// Blocks `set` (indexes, in any order) that play across song tick
    /// `at` split there, the others left as they are (Split with several
    /// blocks picked, 6.5). All or none: refused (`BadPlacement`),
    /// changing nothing, for an empty set, an index past the blocks, none
    /// of them across `at`, or too many blocks. The parts after the cut,
    /// in order.
    pub fn split_these(
        &mut self,
        tracks: &[Track],
        set: &[usize],
        at: Tick,
    ) -> Result<Vec<usize>, ModelError> {
        let mut set = set.to_vec();
        set.sort_unstable();
        set.dedup();
        if set.is_empty() || set.last() >= Some(&self.placements.len()) {
            return Err(ModelError::BadPlacement);
        }
        let mut song = self.clone();
        let mut cut = Vec::new();
        // From the last, so the ones before keep their indexes.
        for &i in set.iter().rev() {
            let p = &song.placements[i];
            if p.start < at && at < p.end() {
                cut.push(p.track);
                song.split(tracks, i, at)?;
            }
        }
        if cut.is_empty() {
            return Err(ModelError::BadPlacement);
        }
        let mut after: Vec<usize> = cut
            .iter()
            .map(|&track| {
                song.placements
                    .partition_point(|q| (q.track, q.start) < (track, at))
            })
            .collect();
        after.sort_unstable();
        *self = song;
        Ok(after)
    }

    /// A copy of block `i` right after it (Duplicate, phase 6): the same
    /// clip from the same place in it, as long, from where the block ends.
    /// Refused (`BadPlacement`), changing nothing, for no such block, a
    /// block on its lane in the way, or a song full of blocks. The copy's
    /// index.
    pub fn duplicate(&mut self, i: usize) -> Result<usize, ModelError> {
        let p = self.placements.get(i).ok_or(ModelError::BadPlacement)?;
        let copy = Placement {
            start: p.end(),
            ..p.clone()
        };
        self.place(copy)
    }

    /// Block `i`'s start moved to song tick `start`, its end staying
    /// (trim from the left, phase 6). What plays stays where it was in the
    /// song: the offset moves with the start, so a later start leaves out
    /// the clip's first ticks and an earlier one brings in the ticks
    /// before (past the clip's top, its pass before, as it goes round).
    /// Refused (`BadPlacement`), changing nothing: no such block, `start`
    /// below 0 or at or past its end, or into the block before it on its
    /// lane.
    pub fn trim_start(
        &mut self,
        tracks: &[Track],
        i: usize,
        start: Tick,
    ) -> Result<(), ModelError> {
        let p = self.placements.get(i).ok_or(ModelError::BadPlacement)?;
        let before = i
            .checked_sub(1)
            .map(|j| &self.placements[j])
            .filter(|q| q.track == p.track);
        if start < 0 || start >= p.end() || before.is_some_and(|q| q.end() > start) {
            return Err(ModelError::BadPlacement);
        }
        let offset = p.offset_by(p.clip(tracks, &self.clips), start - p.start);
        let p = &mut self.placements[i];
        p.length = p.end() - start;
        p.start = start;
        p.offset = offset;
        Ok(())
    }

    /// What block `i` plays slid `by` song ticks later under it (earlier
    /// for a negative `by`; slide, phase 6): the block keeps its place and
    /// length, and its clip's ticks move along inside it, going round the
    /// clip's length. Refused (`BadPlacement`) for no such block.
    pub fn slide(&mut self, tracks: &[Track], i: usize, by: Tick) -> Result<(), ModelError> {
        let p = self.placements.get(i).ok_or(ModelError::BadPlacement)?;
        let offset = p.offset_by(p.clip(tracks, &self.clips), -by);
        self.placements[i].offset = offset;
        Ok(())
    }

    /// Blocks `set` (indexes, in any order) moved together `lanes` lanes
    /// down (up for a negative one) and `by` song ticks later (earlier
    /// for a negative one), or with `copy` copies of them put there, the
    /// blocks keeping their places to each other (several blocks picked,
    /// the Arrange refactor's phase 6). Each stays linked to its clip, as
    /// [`Arrangement::move_placement`] keeps one. All or none: refused,
    /// changing nothing, for an empty set or an index past the blocks (an
    /// index given twice counts once), or when one would leave the lanes,
    /// start before 0 or land on a block outside the set
    /// (`BadPlacement`), go to another lane from or onto a Loop track's
    /// (`BadLoop`), or onto an audio track (`AudioTrack`). The moved
    /// blocks' (or the copies') indexes now, in order.
    pub fn move_placements(
        &mut self,
        tracks: &[Track],
        set: &[usize],
        lanes: i32,
        by: Tick,
        copy: bool,
    ) -> Result<Vec<usize>, ModelError> {
        let mut set = set.to_vec();
        set.sort_unstable();
        set.dedup();
        if set.is_empty() || set.last() >= Some(&self.placements.len()) {
            return Err(ModelError::BadPlacement);
        }
        let loops = |t: usize| tracks.get(t).is_some_and(Track::is_loops);
        let mut moved = Vec::with_capacity(set.len());
        for &i in &set {
            let p = &self.placements[i];
            let track =
                i64::try_from(p.track).map_err(|_| ModelError::BadPlacement)? + i64::from(lanes);
            let track = usize::try_from(track).map_err(|_| ModelError::BadPlacement)?;
            if track >= tracks.len() || p.start + by < 0 {
                return Err(ModelError::BadPlacement);
            }
            if track != p.track && (loops(track) || loops(p.track)) {
                return Err(ModelError::BadLoop);
            }
            if tracks[track].is_audio() {
                return Err(ModelError::AudioTrack);
            }
            moved.push(Placement {
                track,
                start: p.start + by,
                ..p.clone()
            });
        }
        if copy && self.placements.len() + moved.len() > MAX_PLACEMENTS {
            return Err(ModelError::BadPlacement);
        }
        let mut song = self.clone();
        if !copy {
            for &i in set.iter().rev() {
                song.placements.remove(i);
            }
        }
        for p in &moved {
            song.place(p.clone())?;
        }
        let at: Vec<usize> = moved
            .iter()
            .map(|p| {
                song.placements
                    .partition_point(|q| (q.track, q.start) < (p.track, p.start))
            })
            .collect();
        *self = song;
        let mut at = at;
        at.sort_unstable();
        Ok(at)
    }

    /// Copies of blocks `set` right after them, as one (Duplicate with
    /// several blocks picked, phase 6): the copies keep their lanes and
    /// their places to each other, the first starting where the last of
    /// the set ends. All or none, as [`Arrangement::move_placements`]
    /// with `copy`. The copies' indexes, in order.
    pub fn duplicate_placements(
        &mut self,
        tracks: &[Track],
        set: &[usize],
    ) -> Result<Vec<usize>, ModelError> {
        let blocks: Vec<&Placement> = set
            .iter()
            .map(|&i| self.placements.get(i).ok_or(ModelError::BadPlacement))
            .collect::<Result<_, _>>()?;
        let start = blocks
            .iter()
            .map(|p| p.start)
            .min()
            .ok_or(ModelError::BadPlacement)?;
        let end = blocks
            .iter()
            .map(|p| p.end())
            .max()
            .ok_or(ModelError::BadPlacement)?;
        self.move_placements(tracks, set, 0, end - start, true)
    }

    /// Blocks `set` deleted together; a song clip only they played goes
    /// with them. Refused (`BadPlacement`), changing nothing, for an empty
    /// set or an index past the blocks. How many went.
    pub fn remove_placements(&mut self, set: &[usize]) -> Result<usize, ModelError> {
        let mut set = set.to_vec();
        set.sort_unstable();
        set.dedup();
        if set.is_empty() || set.last() >= Some(&self.placements.len()) {
            return Err(ModelError::BadPlacement);
        }
        for &i in set.iter().rev() {
            self.remove(i);
        }
        Ok(set.len())
    }

    /// Section `k`: from marker `k` to the next, the last one to the end
    /// of the song in whole bars of `bar` ticks from its marker (a bar at
    /// least), or while it follows its scene as long as its passes, as a
    /// linked list has it ([`Arrangement::parts`]). None past the markers.
    pub fn section(&self, k: usize, bar: Tick) -> Option<(Tick, Tick)> {
        let m = self.markers.get(k)?;
        let end = match self.markers.get(k + 1) {
            Some(n) => n.at,
            None => self.last_end(m, bar),
        };
        Some((m.at, end))
    }

    /// Where the song's last section, from marker `m`, ends: as long as
    /// its passes while it follows its scene, else at the end of the song
    /// in whole bars of `bar` ticks from `m` (a bar at least).
    fn last_end(&self, m: &Marker, bar: Tick) -> Tick {
        match m.scene {
            Some(s) if !s.changed => m.at + s.pass * Tick::from(m.passes.max(1)),
            _ => {
                let bar = bar.max(1);
                let span = (self.end() - m.at).max(1);
                m.at + (span + bar - 1) / bar * bar
            }
        }
    }

    /// The edit that moves section `k` ([`Arrangement::section`]) to
    /// before section `gap` (after the last for `gap` = the markers'
    /// count), or with `copy` puts a copy of it there. None for no such
    /// section or gap.
    pub fn section_edit(&self, k: usize, gap: usize, copy: bool, bar: Tick) -> Option<TimeEdit> {
        let n = self.markers.len();
        if k >= n || gap > n {
            return None;
        }
        let (from, to) = self.section(k, bar)?;
        let at = match self.markers.get(gap) {
            Some(m) => m.at,
            None => self.section(n - 1, bar)?.1,
        };
        Some(if copy {
            TimeEdit::Copy { from, to, at }
        } else {
            TimeEdit::Move { from, to, at }
        })
    }

    /// Section `k` ([`Arrangement::section`]) moved, on every track, to
    /// before section `gap` (after the last for `gap` = the markers'
    /// count), or with `copy` a copy of it put there (a second verse).
    /// The sections close up behind it and make room for it, so nothing is
    /// written over; what comes before the first marker stays. It is one
    /// [`TimeEdit`] ([`Arrangement::edited`], the project's tempo `bpm`
    /// before the first tempo point): the blocks, markers, tempo points,
    /// song lanes and morph rows all go with their section, a block
    /// across an edge cut there in phase and joined again where it meets
    /// itself. Moved to where it is, it stays. Refused, changing nothing:
    /// `BadRange` for no such section or gap, else what the edit says.
    pub fn move_section(
        &mut self,
        tracks: &[Track],
        k: usize,
        gap: usize,
        copy: bool,
        bar: Tick,
        bpm: f64,
    ) -> Result<(), Refused> {
        let edit = self
            .section_edit(k, gap, copy, bar)
            .ok_or(Refused::BadRange)?;
        if !edit.moves_nothing() {
            *self = self.edited(tracks, edit, bpm)?;
        }
        Ok(())
    }
}

impl Project {
    /// The clip song block `p` plays ([`Placement::clip`]).
    pub fn block_clip(&self, p: &Placement) -> Option<&Clip> {
        p.clip(&self.tracks, &self.arrangement.clips)
    }

    /// Give song block `i` a copy of its clip of its own, among the
    /// song's clips with a new id, to change on its own (Make unique). A
    /// block already playing a song clip no other block plays keeps it.
    /// The id it plays now; `BadPlacement`, changing nothing, for no such
    /// block or one whose clip is gone.
    pub fn make_unique(&mut self, i: usize) -> Result<ClipId, ModelError> {
        let p = self
            .arrangement
            .placements
            .get(i)
            .ok_or(ModelError::BadPlacement)?;
        let id = p.clip;
        let song = &self.arrangement;
        let alone = song.placements.iter().filter(|q| q.clip == id).count() == 1;
        if alone && song.clips.get(id).is_some() {
            return Ok(id);
        }
        let mut copy = self.block_clip(p).ok_or(ModelError::BadPlacement)?.copied();
        copy.id = self.new_clip_id();
        let new = copy.id;
        self.arrangement.clips.keep(copy)?;
        self.arrangement.placements[i].clip = new;
        // The clip it played, if a song clip no block plays now, goes.
        if self.arrangement.clips.get(id).is_some() && !self.arrangement.plays(id) {
            self.arrangement.clips.take(id);
        }
        Ok(new)
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

/// Row `row`'s chords as every part asks them (contract 4 of the
/// composing tools): its Harmony line over its length
/// ([`scene_length`]), suggested when every chord on it is. `None` while
/// the row has no line yet.
pub fn row_harmony(project: &Project, row: usize, bar: Tick) -> Option<RowHarmony> {
    let line = &project.scene(row)?.chords;
    if line.is_empty() {
        return None;
    }
    Some(RowHarmony {
        line: line.clone(),
        length: scene_length(project, row, bar),
        suggested: line.spans().iter().all(|s| s.heard == Heard::Suggested),
    })
}

/// Rows one after another from `at`, each for its length in song ticks,
/// as placements of the rows' clips with a marker each: see the module
/// notes. `BadPlacement` for a clip with no id yet. The
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
            if let Some(bpm) = s.bpm() {
                if !put_scene_tempo(&mut song, t, bpm, None) {
                    differences.push(Difference::song(t, Differs::SceneTempo));
                }
            }
        }
        for (track, tr) in project.tracks.iter().enumerate() {
            let Some(clip) = tr.slots[row].as_ref() else {
                // A Keep playing track (5.2): the clip it plays runs on
                // through this row, its block longer by the row.
                if scene.is_some_and(|s| s.keeps(track)) && ends[track] == Some(t) {
                    ends[track] = Some(t + length);
                    let last = song
                        .placements
                        .iter_mut()
                        .rev()
                        .find(|p| p.track == track && p.start + p.length == t);
                    if let Some(p) = last {
                        p.length += length;
                    }
                }
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
            // Linked by its id: a clip without one (never put in through
            // the project) can't be named.
            if !clip.id.is_given() {
                return Err(ModelError::BadPlacement);
            }
            song.place(Placement::new(track, clip.id, t, len))?;
        }
        song.mark(t, &scene_title(project, row))?;
        t += length;
    }
    Ok(ToSong { song, differences })
}

/// A row's scene tempo (the Arrange refactor's 5.2b) as the song plays it
/// (Scenes → song, List → song, [`scenes_to_song`]): a tempo point on song
/// tick `at`, where the row starts, unless the song's tempo there is `bpm`
/// already (`before`: the tempo before the song's first point, if known).
/// The song's last point, when it ramps and comes before `at`, stops
/// ramping first: a ramp on the last point waits for a point after it, so
/// it played flat and still does. False, with no point put in, when the
/// song can't hold another ([`MAX_TEMPO_POINTS`]).
pub fn put_scene_tempo(song: &mut Arrangement, at: Tick, bpm: f64, before: Option<f64>) -> bool {
    let now = song
        .tempo
        .iter()
        .rev()
        .find(|p| p.at <= at)
        .map(|p| p.bpm)
        .or(before);
    if now == Some(bpm) {
        return true;
    }
    if song.set_tempo(at, bpm, Some(false)).is_err() {
        return false;
    }
    let k = song.tempo.partition_point(|p| p.at < at);
    if k + 1 == song.tempo.len() && k > 0 {
        song.tempo[k - 1].ramp = false;
    }
    true
}

/// Row `row`'s scene by name, as a marker made from it is named: its
/// name, or "Scene N" for one with none.
pub fn scene_title(project: &Project, row: usize) -> String {
    project
        .scene(row)
        .map(|s| s.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("Scene {}", row + 1))
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

/// The song's sections, as a linked song list shows them (phase 4b, plan
/// D8), in bars of `bar` ticks: each from its marker to the next; the
/// song's bars before the first marker as an unnamed section (the whole
/// song, with no marker); the last one as long as its passes
/// ([`FromScene::pass`] times its passes) while it follows its scene, else
/// to the end of the song in whole bars (one bar at least). None for an
/// empty song with no marker.
pub fn song_sections(project: &Project, bar: Tick) -> Vec<SongSection> {
    project.arrangement.parts(bar)
}

/// The song's sections that play a scene, for the Song view's Chords lane
/// (composing step 6, board C): where each starts and ends, and the
/// scene's row. A section made from a scene names it; one Scenes → song
/// made is known by its blocks: every block that starts in it plays a
/// clip of the same row's cells (linked by id). A section of the song's
/// own clips, or of clips from several rows, plays no one scene.
pub fn scene_sections(project: &Project, bar: Tick) -> Vec<(Tick, Tick, usize)> {
    let song = &project.arrangement;
    song.parts(bar)
        .iter()
        .filter_map(|s| {
            if let Some(f) = &s.scene {
                return Some((s.start, s.end, f.row));
            }
            let mut row = None;
            for p in song
                .placements
                .iter()
                .filter(|p| p.start >= s.start && p.start < s.end)
            {
                match project.find_clip(p.clip) {
                    Some(crate::ClipAt::Cell { row: r, .. }) if row.is_none_or(|w| w == r) => {
                        row = Some(r)
                    }
                    _ => return None,
                }
            }
            Some((s.start, s.end, row?))
        })
        .collect()
}

/// What a section that follows its scene holds ([`Arrangement::mark_changed`]):
/// its scene and passes, how far it reaches, and the blocks that play in
/// it, from its start.
#[derive(PartialEq)]
struct Held {
    scene: FromScene,
    passes: u16,
    reach: Tick,
    blocks: Vec<Placement>,
}

impl Arrangement {
    /// The song's sections ([`song_sections`]).
    pub fn parts(&self, bar: Tick) -> Vec<SongSection> {
        let bar = bar.max(1);
        let end = self.end();
        let mut out = Vec::with_capacity(self.markers.len() + 1);
        let lead = match self.markers.first() {
            None => (end > 0).then(|| (end + bar - 1) / bar * bar),
            Some(m) => (m.at > 0).then_some(m.at),
        };
        if let Some(to) = lead {
            out.push(SongSection {
                marker: None,
                start: 0,
                end: to,
                scene: None,
                passes: 1,
            });
        }
        out.extend((0..self.markers.len()).filter_map(|k| self.part_of(k, bar)));
        out
    }

    /// The section playing at song tick `t` ([`Arrangement::parts`]),
    /// found without making the list (each period of a linked song looks):
    /// None before the song's start or past its last section.
    pub fn part_at(&self, t: Tick, bar: Tick) -> Option<SongSection> {
        let k = self.markers.partition_point(|m| m.at <= t);
        let Some(i) = k.checked_sub(1) else {
            let bar = bar.max(1);
            let end = match self.markers.first() {
                Some(m) => m.at,
                None => (self.end() + bar - 1) / bar * bar,
            };
            return (0..end).contains(&t).then_some(SongSection {
                marker: None,
                start: 0,
                end,
                scene: None,
                passes: 1,
            });
        };
        self.part_of(i, bar).filter(|s| t < s.end)
    }

    /// The first section that starts after song tick `t` (0 or later),
    /// found without making the list.
    pub fn part_after(&self, t: Tick, bar: Tick) -> Option<SongSection> {
        self.part_of(self.markers.partition_point(|m| m.at <= t), bar)
    }

    /// Where the song's sections start and end, the first to the last
    /// ([`Arrangement::parts`]); None with none.
    pub fn parts_span(&self, bar: Tick) -> Option<(Tick, Tick)> {
        let start = self
            .part_at(0, bar)
            .map_or_else(|| self.markers.first().map(|m| m.at), |s| Some(s.start))?;
        let end = match self.markers.last() {
            Some(m) => self.last_end(m, bar),
            None => self.part_at(0, bar)?.end,
        };
        Some((start, end))
    }

    /// Marker `k`'s section.
    fn part_of(&self, k: usize, bar: Tick) -> Option<SongSection> {
        let m = self.markers.get(k)?;
        let end = match self.markers.get(k + 1) {
            Some(n) => n.at,
            None => self.last_end(m, bar),
        };
        Some(SongSection {
            marker: Some(k),
            start: m.at,
            end,
            scene: m.scene,
            passes: m.passes,
        })
    }

    /// What section `s` holds while it follows its scene; None for one
    /// that doesn't. The last section reaches to the song's end, so a
    /// block put after it is in it. A block across its edges counts as
    /// the part of it inside, cut in phase, so a block joined to its copy
    /// across an edge holds what the two did. Blocks that meet with the
    /// clip where the one before had got to count as one, so a block split
    /// in phase (Split, phase 6) holds what it did whole.
    fn held(&self, tracks: &[Track], s: &SongSection) -> Option<Held> {
        let scene = s.scene.filter(|f| !f.changed)?;
        let last = s.marker? + 1 == self.markers.len();
        let reach = if last { s.end.max(self.end()) } else { s.end };
        let mut blocks: Vec<Placement> = self
            .placements
            .iter()
            .filter(|p| p.start < reach && p.end() > s.start)
            .map(|p| {
                let q = if p.start < s.start {
                    p.from(p.clip(tracks, &self.clips), s.start)
                } else {
                    p.clone()
                };
                Placement {
                    start: q.start - s.start,
                    length: q.end().min(reach) - q.start,
                    ..q
                }
            })
            .collect();
        blocks.sort_by_key(|p| (p.track, p.start));
        let mut joined: Vec<Placement> = Vec::with_capacity(blocks.len());
        for b in blocks {
            match joined.last_mut() {
                Some(a)
                    if a.track == b.track
                        && a.clip == b.clip
                        && a.end() == b.start
                        && a.offset_by(a.clip(tracks, &self.clips), a.length) == b.offset =>
                {
                    a.length += b.length;
                }
                _ => joined.push(b),
            }
        }
        Some(Held {
            scene,
            passes: s.passes,
            reach: reach - s.start,
            blocks: joined,
        })
    }

    /// After an edit of the song that isn't a linked list's own (the
    /// Arrange refactor's 4b.3, plan D8): each section that followed its
    /// scene and holds something else now (a block moved, deleted, put
    /// in, made longer or shorter or given a clip of its own, Rec song
    /// over it, or its bars changed: a marker moved, put in or taken away,
    /// bars put in or taken out) is the song's own: changed in the song,
    /// its passes plain bars (× 1). A section moved or copied whole, or
    /// only moved later or earlier, holds what it held, so it still
    /// follows. `before` is the song before the edit, on `tracks`; the
    /// markers marked, in order.
    pub fn mark_changed(
        &mut self,
        tracks: &[Track],
        before: &Arrangement,
        bar: Tick,
    ) -> Vec<usize> {
        if !self
            .markers
            .iter()
            .any(|m| m.scene.is_some_and(|f| !f.changed))
        {
            return Vec::new();
        }
        let was: Vec<Held> = before
            .parts(bar)
            .iter()
            .filter_map(|s| before.held(tracks, s))
            .collect();
        let marked: Vec<usize> = self
            .parts(bar)
            .iter()
            .filter(|s| self.held(tracks, s).is_some_and(|h| !was.contains(&h)))
            .filter_map(|s| s.marker)
            .collect();
        for &k in &marked {
            let m = &mut self.markers[k];
            if let Some(f) = &mut m.scene {
                f.changed = true;
            }
            m.passes = 1;
        }
        marked
    }
}

/// The song's sections: where anything starts or ends and the markers, in
/// order, merged where the same placements play on, without the silent
/// ones. Each is (start, end, the placement playing on each track).
/// `cover` is each audio lane's regions as one span, its first region's
/// start to its last one's end ([`SongSound::cover`]): a stretch under one
/// is never silent, and its ends cut, but its own gaps don't.
pub fn sections(
    project: &Project,
    cover: &[(Tick, Tick)],
) -> Vec<(Tick, Tick, Vec<Option<usize>>)> {
    let song = &project.arrangement;
    let mut cuts: Vec<Tick> = song
        .placements
        .iter()
        .flat_map(|p| [p.start, p.end()])
        .chain(song.markers.iter().map(|m| m.at))
        .chain(cover.iter().flat_map(|&(a, b)| [a, b]))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let marked = |t: Tick| song.markers.iter().any(|m| m.at == t);
    let mut out: Vec<(Tick, Tick, Vec<Option<usize>>)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let playing: Vec<Option<usize>> =
            (0..project.tracks.len()).map(|t| song.at(t, a)).collect();
        let sounds = cover.iter().any(|&(f, t)| f < b && t > a);
        if playing.iter().all(Option::is_none) && !sounds {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.1 == a && last.2 == playing && !marked(a) => last.1 = b,
            _ => out.push((a, b, playing)),
        }
    }
    out
}

/// The song's audio as Song → scenes turns it (audio tracks phase 2,
/// step 2d): each audio track's regions by its lasting id, in order, none
/// overlapping, on song ticks, exactly as they sound; and the tempo map
/// and rate the song plays them along, which say where a cut falls in a
/// recording.
#[derive(Clone, Copy, Debug)]
pub struct SongSound<'a> {
    pub regions: &'a [(TrackId, Vec<ClipRegion>)],
    pub map: TempoMap,
    pub rate: u32,
}

impl SongSound<'_> {
    /// No regions, along `map` at `rate` (a block of an audio clip still
    /// turns along them).
    pub fn none(map: TempoMap, rate: u32) -> SongSound<'static> {
        SongSound {
            regions: &[],
            map,
            rate,
        }
    }

    /// Track `track`'s regions (none for a track not named).
    fn of(&self, track: TrackId) -> &[ClipRegion] {
        self.regions
            .iter()
            .find(|(t, _)| *t == track)
            .map_or(&[], |(_, r)| r.as_slice())
    }

    /// Each lane's regions as one span, first start to last end
    /// ([`sections`]).
    fn cover(&self, project: &Project) -> Vec<(Tick, Tick)> {
        project
            .tracks
            .iter()
            .filter(|t| t.is_audio())
            .filter_map(|t| {
                let r = self.of(t.id);
                Some((r.first()?.start, r.iter().map(|r| r.end).max()?))
            })
            .collect()
    }

    /// `r`'s frames from song tick `at`, where it starts, to song tick
    /// `t`, on its own rounding (its `phase`), as the audio tracks count
    /// them (`Region::frames_to`): to its end, its length.
    fn frames_of(&self, r: &ClipRegion, at: Tick, t: Tick) -> i64 {
        let x = self.map.frames_between(at as f64, t as f64, self.rate);
        (f64::from(r.phase) + x).round() as i64
    }

    /// `r`, on song ticks `at..end`, cut at song tick `cut` inside it, as
    /// the audio tracks cut a region (`mpfree-audiotracks`' `cut`): the
    /// part before ends there and counts the frames after in `left`; the
    /// part after plays on from where `r` had got to, counting the frames
    /// before in `into`, on `r`'s rounding, so the parts play exactly as
    /// `r` did. Each keeps its ticks (the caller moves them).
    fn cut(&self, r: &ClipRegion, at: Tick, end: Tick, cut: Tick) -> (ClipRegion, ClipRegion) {
        let add = |n: u32, k: i64| (i64::from(n) + k).clamp(0, i64::from(u32::MAX)) as u32;
        let k = self.frames_of(r, at, cut);
        let len = self.frames_of(r, at, end);
        let (mut before, mut after) = (*r, *r);
        before.left = add(r.left, len - k);
        after.src = r.src + k;
        let x = self.map.frames_between(at as f64, cut as f64, self.rate);
        after.phase = (f64::from(r.phase) + x - k as f64) as f32;
        after.into = add(r.into, k);
        (before, after)
    }
}

/// Why Song → scenes changed nothing ([`song_to_scenes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenesRefused {
    /// The rows can't be made (`NoRoom`: no run of empty rows long enough).
    Model(ModelError),
    /// Audio track `track` has `regions` regions in song ticks `from..to`,
    /// more than a clip holds ([`MAX_CLIP_REGIONS`]).
    Regions {
        track: usize,
        from: Tick,
        to: Tick,
        regions: usize,
    },
    /// Audio track `track` has regions and a block in song ticks
    /// `from..to`, and a cell holds one clip.
    Both { track: usize, from: Tick, to: Tick },
}

impl From<ModelError> for ScenesRefused {
    fn from(e: ModelError) -> ScenesRefused {
        ScenesRefused::Model(e)
    }
}

/// Each section of the song into a row: see the module notes. Uses the
/// first run of empty rows ([`Project::row_empty`]) long enough, and sets
/// every field of each row's scene; returns the rows and the ways they
/// play differently from the song. Refused, changing nothing: no such run
/// (`NoRoom`), or a section an audio clip can't hold exactly
/// ([`ScenesRefused`]).
pub fn song_to_scenes(
    project: &mut Project,
    bar: Tick,
    sound: &SongSound,
) -> Result<ToScenes, ScenesRefused> {
    let bar = bar.max(1);
    let parts = sections(project, &sound.cover(project));
    if parts.is_empty() {
        return Ok(ToScenes::default());
    }
    let n = parts.len();
    // A run of empty rows within one bank of [`SLOTS`] (scene banks, the
    // Arrange refactor's 5.6), the first there is from bank A on: the
    // rows chain by Next, which goes round within a bank.
    let first = (0..ROWS)
        .step_by(SLOTS)
        .flat_map(|bank| bank..(bank + SLOTS + 1).saturating_sub(n))
        .find(|&r| (r..r + n).all(|k| project.row_empty(k)))
        .ok_or(ModelError::NoRoom)?;
    let mut differences = song_differences(project, &parts, bar);
    let mut p = project.clone();
    for (k, (a, b, playing)) in parts.iter().enumerate() {
        let row = first + k;
        for (track, at) in playing.iter().enumerate() {
            // The lane's regions here, cut at the section's edges.
            let tr = &project.tracks[track];
            let heard = if tr.is_audio() {
                regions_in(sound.of(tr.id), *a, *b, sound)
            } else {
                Vec::new()
            };
            let refused = |regions: usize| ScenesRefused::Regions {
                track,
                from: *a,
                to: *b,
                regions,
            };
            let Some(i) = at else {
                if !heard.is_empty() {
                    if heard.len() > MAX_CLIP_REGIONS {
                        return Err(refused(heard.len()));
                    }
                    p.put_new_clip(track, row, regions_clip(tr, &heard, *a, *b, bar, sound))?;
                }
                continue;
            };
            if !heard.is_empty() {
                return Err(ScenesRefused::Both {
                    track,
                    from: *a,
                    to: *b,
                });
            }
            let block = &project.arrangement.placements[*i];
            // A block whose clip is gone plays nothing: nothing to copy.
            let Some(clip) = project.block_clip(block) else {
                continue;
            };
            let (mut copy, why) = section_copy(block, clip, *a);
            // A block of an audio clip: its regions turned round too.
            if let Some(audio) = &clip.audio {
                let turned =
                    turned_audio(audio, clip.length, block_turn(block, clip, *a), *a, sound);
                if turned.regions.len() > MAX_CLIP_REGIONS {
                    return Err(refused(turned.regions.len()));
                }
                copy.audio = Some(turned);
            }
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
        // A tempo point on its start: the row's tempo (5.2b).
        let tempo = project
            .arrangement
            .tempo
            .iter()
            .find(|t| t.at == *a)
            .and_then(|t| row_tempo(t.bpm));
        // Every field of the row's scene, not only the three it sets.
        *p.scene_mut(row).ok_or(ModelError::NoRoom)? = Scene {
            name,
            follow: Some(Follow::after(bars as u16, action)),
            tempo,
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

/// What [`scenes_to_song`] makes: the blocks and markers to add to the
/// song, and how they play differently from the rows (none: exact).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToSong {
    pub song: Arrangement,
    pub differences: Vec<Difference>,
}

/// One way a conversion plays differently from what it was made from
/// (plan D7): what, where (a song tick: the song's for Song → scenes, the
/// new blocks' for Scenes → song, List → song and [`scenes_to_song`]) and
/// on which track (None: the whole song).
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
    /// Scenes → song, List → song ([`scenes_to_song`] too): the scene's
    /// program changes, which the song doesn't send.
    SceneProgram,
    /// Scenes → song, List → song ([`scenes_to_song`] too): the scene's
    /// tempo, which the song had no room for as a tempo point.
    SceneTempo,
    /// [`scenes_to_song`]: the clip's own follow action, which the song
    /// leaves out (the clip plays on for the row).
    ClipFollow,
    /// [`scenes_to_song`]: a legato clip after a clip on its track: the
    /// song starts it from its top instead of the other's place.
    Legato,
}

impl Difference {
    /// One that is the whole song's, at song tick `at`.
    pub fn song(at: Tick, what: Differs) -> Difference {
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
    /// note held over the row change is cut there" (the track, the bar
    /// and [`Difference::what_said`]).
    pub fn said(&self, project: &Project, bar: Tick) -> String {
        let what = self.what_said(bar);
        let at = format!("bar {}", self.bar(bar));
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

    /// The bar it is in, from 1, with bars of `bar` ticks.
    pub fn bar(&self, bar: Tick) -> Tick {
        self.at.div_euclid(bar.max(1)) + 1
    }

    /// What is different, in a musician's words ("a note held over the
    /// row change is cut there"), with bars of `bar` ticks.
    pub fn what_said(&self, bar: Tick) -> String {
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
        match self.what {
            Differs::OffBar => {
                "a part starts or ends off a bar line; the rows change on bar lines".into()
            }
            Differs::TooLong => format!(
                "a part is longer than {} bars; its row moves on after that",
                crate::scenes::MAX_FOLLOW_BARS
            ),
            Differs::Silence { length } => format!("{} of silence left out", bars(length)),
            Differs::Tempo => {
                "the song's tempo changes inside a part or glides; a row changes tempo only as it starts"
                    .into()
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
            Differs::SceneTempo => {
                "the scene's tempo isn't in the song: no room for another tempo change".into()
            }
            Differs::ClipFollow => "the clip's follow action isn't in the song; it plays on".into(),
            Differs::Legato => "the clip is legato; the song starts it from its top".into(),
        }
    }
}

/// A song tempo as a row's ([`crate::scenes::scene_tempo`]), only when it
/// is one exactly: 0.1 BPM steps in the app's range.
fn row_tempo(bpm: f64) -> Option<u16> {
    crate::scenes::scene_tempo(bpm).filter(|&t| (f64::from(t) / 10.0 - bpm).abs() < 1e-9)
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
    // A tempo point on a part's start becomes its row's tempo (5.2b); one
    // anywhere else, a ramp to another, or a tempo a scene can't hold, the
    // rows can't play.
    let last = song.tempo.len().saturating_sub(1);
    let off = song.tempo.iter().enumerate().find(|(k, t)| {
        !parts.iter().any(|(a, _, _)| *a == t.at)
            || (t.ramp && *k < last)
            || row_tempo(t.bpm).is_none()
    });
    if let Some((_, t)) = off {
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
    let into = clip.speed.to_clip((a - block.start) as f64);
    let by = block_turn(block, clip, a);
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

/// Where in its clip `block` is at song tick `a`: the clip tick a copy
/// turns round to ([`turned`]).
fn block_turn(block: &Placement, clip: &Clip, a: Tick) -> Tick {
    let len = clip.length.max(1);
    let into = clip.speed.to_clip((a - block.start) as f64);
    (block.offset + into.round() as Tick).rem_euclid(len)
}

/// `regions` (song ticks, in order) in song ticks `a..b`, on ticks from
/// `a`: each across an edge cut there as the audio tracks cut a region
/// along the song's tempo map (`mpfree-audiotracks`' `cut`): the frames
/// cut off count in its `into` and `left`, so it keeps exactly its share
/// of its fades, and a part after a cut plays on from where the region
/// had got to.
fn regions_in(regions: &[ClipRegion], a: Tick, b: Tick, sound: &SongSound) -> Vec<ClipRegion> {
    regions
        .iter()
        .filter(|r| r.start < b && r.end > a)
        .map(|r| {
            let mut c = *r;
            if r.start < a {
                c = sound.cut(&c, r.start, r.end, a).1;
                c.start = a;
            }
            if r.end > b {
                c = sound.cut(&c, c.start, r.end, b).0;
                c.end = b;
            }
            c.start -= a;
            c.end -= a;
            c
        })
        .collect()
}

/// The audio clip of song ticks `a..b` on audio track `track`: `heard`,
/// its regions there ([`regions_in`]). As To a cell makes one: named after
/// its track and bars ("Vocal 5–8"), as long as the section, made at the
/// song's tempo where it starts, its gain 1 (each region keeps its own),
/// its sound ending where its last region's does.
fn regions_clip(
    track: &Track,
    heard: &[ClipRegion],
    a: Tick,
    b: Tick,
    bar: Tick,
    sound: &SongSound,
) -> Clip {
    let (first, last) = (a / bar + 1, (b + bar - 1) / bar);
    let name = if last > first {
        format!("{} {first}–{last}", track.name)
    } else {
        format!("{} {first}", track.name)
    };
    let end = heard.last().map_or(0, |r| {
        r.src + sound.frames_of(r, a + r.start, a + r.end).max(1)
    });
    let mut clip = Clip::new(&name, b - a);
    clip.audio = Some(AudioClip {
        regions: heard.to_vec(),
        bpm: sound.map.bpm_at(a),
        gain: 1.0,
        last_frame: end,
    });
    clip
}

/// `audio`, the regions of a clip `len` ticks long, turned round by `by`
/// as [`turned`] turns notes, where the song played clip tick `by` at song
/// tick `at`: a region across `by` is cut there as the song played it
/// (as in [`regions_in`]), its part after starting the turned clip and its
/// part before ending it. Its sound ends where its new last region's does.
fn turned_audio(audio: &AudioClip, len: Tick, by: Tick, at: Tick, sound: &SongSound) -> AudioClip {
    let len = len.max(1);
    let by = by.rem_euclid(len);
    if by == 0 {
        return audio.clone();
    }
    // Clip tick `t` of the pass that played `by` at `at`, on the song.
    let song = |t: Tick| at - by + t;
    // Each part, turned, with the song ticks it played.
    let mut parts: Vec<(ClipRegion, Tick, Tick)> = Vec::new();
    for r in &audio.regions {
        let moved = |mut c: ClipRegion, from: Tick, to: Tick| {
            let shift = if from >= by { -by } else { len - by };
            c.start = from + shift;
            c.end = to + shift;
            (c, song(from), song(to))
        };
        if r.end <= by || r.start >= by {
            parts.push(moved(*r, r.start, r.end));
            continue;
        }
        let (before, after) = sound.cut(r, song(r.start), song(r.end), at);
        parts.push(moved(before, r.start, by));
        parts.push(moved(after, by, r.end));
    }
    parts.sort_by_key(|p| p.0.start);
    let last_frame = parts.last().map_or(audio.last_frame, |(r, from, to)| {
        r.src + sound.frames_of(r, *from, *to).max(1)
    });
    AudioClip {
        regions: parts.into_iter().map(|p| p.0).collect(),
        last_frame,
        ..audio.clone()
    }
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
    use crate::{ClipAt, RecId};

    const BAR: Tick = 3840;

    /// No audio, along a steady 120 BPM at 44.1 kHz.
    fn quiet() -> SongSound<'static> {
        SongSound::none(TempoMap::constant(120.0).unwrap(), 44_100)
    }

    fn project() -> Project {
        let mut p = Project::new();
        for name in ["drums", "bass"] {
            let mut t = Track::new(name, 0);
            t.slots[0] = Some(Clip::new("a", BAR));
            t.slots[1] = Some(Clip::new("b", 2 * BAR));
            p.tracks.push(t);
        }
        // Ids 1 and 2 on the drums, 3 and 4 on the bass.
        p.give_clip_ids();
        p
    }

    /// The row of block `b`'s clip on its own lane's track (None: it
    /// plays a clip that lives elsewhere, or none).
    fn row(p: &Project, b: &Placement) -> Option<usize> {
        p.tracks[b.track].slots.row_of(b.clip)
    }

    /// Song → scenes with audio (audio tracks phase 2, step 2d): Vocal
    /// (an audio track) first, then Drums with a two-bar block, markers
    /// at bars 1 and 2 ("Verse", "Chorus").
    fn audio_song() -> Project {
        let mut p = Project::new();
        p.tracks.push(Track::new_audio("Vocal", 0));
        let mut drums = Track::new("Drums", 0);
        drums.slots[0] = Some(Clip::new("Beat", 2 * BAR));
        p.tracks.push(drums);
        p.give_track_ids();
        p.give_clip_ids();
        let beat = Placement::of(&p.tracks, 1, 0, 0, 2 * BAR);
        p.arrangement.place(beat).unwrap();
        p.arrangement.mark(0, "Verse").unwrap();
        p.arrangement.mark(BAR, "Chorus").unwrap();
        p
    }

    /// Frames in `ticks` at 120 BPM and 44.1 kHz: a bar is two seconds.
    fn frames(ticks: Tick) -> i64 {
        ticks * 88_200 / BAR
    }

    /// Each audio lane's regions in a section become an audio clip in its
    /// row: cut at the section's edges, each part keeping its share of its
    /// fades (`into`, `left`) and the part after a cut playing on from the
    /// frame the cut was at, so the rows run on sample for sample; the
    /// regions past the last block make a section of their own.
    #[test]
    fn a_lanes_regions_become_an_audio_clip_in_each_sections_row() {
        let mut p = audio_song();
        let vocal = p.tracks[0].id;
        let mut one = ClipRegion::new(BAR / 2, 3 * BAR / 2, RecId(1), 1000);
        (one.fade_in, one.fade_out, one.gain) = (100, 200, 0.5);
        let two = ClipRegion::new(7 * BAR / 4, 5 * BAR / 2, RecId(2), 0);
        let regions = vec![(vocal, vec![one, two])];
        let sound = SongSound {
            regions: &regions,
            ..quiet()
        };
        let done = song_to_scenes(&mut p, BAR, &sound).unwrap();
        // Verse, Chorus, and the half bar the Vocal sings on after it.
        assert_eq!(done.rows, [1, 2, 3], "row 0 holds the Beat");
        let clip = |row: usize| p.tracks[0].slots[row].clone().unwrap();
        let regions_of = |row: usize| clip(row).audio.unwrap().regions;
        let cut = |r: ClipRegion, start, end, src, into, left| ClipRegion {
            start,
            end,
            src,
            into,
            left,
            ..r
        };
        assert_eq!(
            regions_of(1),
            [cut(one, BAR / 2, BAR, 1000, 0, frames(BAR / 2) as u32)]
        );
        assert_eq!(
            regions_of(2),
            [
                cut(
                    one,
                    0,
                    BAR / 2,
                    1000 + frames(BAR / 2),
                    frames(BAR / 2) as u32,
                    0
                ),
                cut(two, 3 * BAR / 4, BAR, 0, 0, frames(BAR / 2) as u32),
            ]
        );
        assert_eq!(
            regions_of(3),
            [cut(
                two,
                0,
                BAR / 2,
                frames(BAR / 4),
                frames(BAR / 4) as u32,
                0
            )]
        );
        // Named and made as To a cell names and makes one.
        let c = clip(2);
        assert_eq!((c.name.as_str(), c.length), ("Vocal 2", BAR));
        let a = c.audio.unwrap();
        // Its sound ends where its last region does: a quarter bar of
        // recording 2 from its frame 0.
        assert_eq!((a.bpm, a.gain, a.last_frame), (120.0, 1.0, frames(BAR / 4)));
        assert_eq!(clip(3).length, BAR / 2);
        // The Drums have nothing in the last row: the block ended.
        assert!(p.tracks[1].slots[3].is_none());
        p.check().unwrap();
    }

    /// A block of an audio clip turns round as a note clip does: its
    /// region across the turn is cut there as the song played it, the
    /// part after starting the copy and the part before ending it.
    #[test]
    fn a_block_of_an_audio_clip_turns_round_cutting_its_region() {
        let mut p = audio_song();
        let mut c = Clip::new("Hook", 2 * BAR);
        c.audio = Some(AudioClip::of(
            RecId(7),
            500,
            500 + frames(2 * BAR),
            2 * BAR,
            120.0,
        ));
        p.tracks[0].slots[0] = Some(c);
        p.give_clip_ids();
        p.arrangement
            .place(Placement::of(&p.tracks, 0, 0, 0, 2 * BAR))
            .unwrap();
        let done = song_to_scenes(&mut p, BAR, &quiet()).unwrap();
        assert_eq!(done.rows, [1, 2]);
        let audio = |row: usize| p.tracks[0].slots[row].clone().unwrap().audio.unwrap();
        // The Verse's row: the clip as it was.
        assert_eq!(
            audio(1).regions,
            [ClipRegion::new(0, 2 * BAR, RecId(7), 500)]
        );
        // The Chorus's, a bar in: the second bar first, then the first.
        let turned = audio(2);
        let mut after = ClipRegion::new(0, BAR, RecId(7), 500 + frames(BAR));
        after.into = frames(BAR) as u32;
        let mut before = ClipRegion::new(BAR, 2 * BAR, RecId(7), 500);
        before.left = frames(BAR) as u32;
        assert_eq!(turned.regions, [after, before]);
        assert_eq!(turned.last_frame, 500 + frames(BAR));
        p.check().unwrap();
    }

    /// A section a clip can't hold exactly is refused before anything
    /// changes: more regions on a lane than a clip holds, or regions and a
    /// block on one lane.
    #[test]
    fn a_section_a_clip_cant_hold_is_refused_and_nothing_changes() {
        let p = audio_song();
        let vocal = p.tracks[0].id;
        let many: Vec<ClipRegion> = (0..17)
            .map(|k| ClipRegion::new(k * 100, k * 100 + 50, RecId(1), 0))
            .collect();
        let regions = vec![(vocal, many)];
        let sound = SongSound {
            regions: &regions,
            ..quiet()
        };
        let mut q = p.clone();
        assert_eq!(
            song_to_scenes(&mut q, BAR, &sound),
            Err(ScenesRefused::Regions {
                track: 0,
                from: 0,
                to: BAR,
                regions: 17
            })
        );
        assert_eq!(q, p, "nothing changed");
        // A block of an audio clip on the Vocal's lane, and a region there
        // too (the audio tracks never let them overlap; here they meet in
        // one section).
        let mut q = p.clone();
        let mut c = Clip::new("Hook", BAR / 2);
        c.audio = Some(AudioClip::of(RecId(7), 0, frames(BAR / 2), BAR / 2, 120.0));
        q.tracks[0].slots[5] = Some(c);
        q.give_clip_ids();
        q.arrangement
            .place(Placement::of(&q.tracks, 0, 5, 0, BAR / 2))
            .unwrap();
        let before = q.clone();
        let regions = vec![(vocal, vec![ClipRegion::new(BAR / 4, BAR / 2, RecId(1), 0)])];
        let sound = SongSound {
            regions: &regions,
            ..quiet()
        };
        assert_eq!(
            song_to_scenes(&mut q, BAR, &sound),
            Err(ScenesRefused::Both {
                track: 0,
                from: 0,
                to: BAR / 2
            })
        );
        assert_eq!(q, before, "nothing changed");
    }

    #[test]
    fn clearing_cuts_placements_and_keeps_the_rest_in_phase() {
        let p = project();
        let mut s = Arrangement::default();
        // Slot 1 (two bars) from 0 for 6 bars, and slot 0 from bar 7.
        s.place(Placement::of(&p.tracks, 0, 1, 0, 6 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 0, 0, 7 * BAR, BAR))
            .unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 0, 8 * BAR)).unwrap();
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
        assert_eq!(s.place(Placement::of(&p.tracks, 1, 0, BAR, BAR)), Ok(0));
        assert_eq!(s.place(Placement::of(&p.tracks, 0, 0, 0, 2 * BAR)), Ok(0));
        assert_eq!(s.place(Placement::of(&p.tracks, 0, 1, 2 * BAR, BAR)), Ok(1));
        assert_eq!(
            s.place(Placement::of(&p.tracks, 0, 1, BAR, BAR)),
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

    /// Arrange 1.3: a block names its clip by id, so it follows the clip
    /// wherever it moves, until Make unique gives it a song clip of its
    /// own; a block whose clip is gone stays, silent, until it is back.
    #[test]
    fn a_block_follows_its_clip_by_id_until_made_unique() {
        let mut p = project();
        p.arrangement
            .place(Placement::of(&p.tracks, 0, 1, 0, 4 * BAR))
            .unwrap();
        assert_eq!(p.arrangement.placements[0].clip, ClipId(2));
        p.tracks[0].slots[1].as_mut().unwrap().name = "edited".into();
        let name = |p: &Project| {
            p.block_clip(&p.arrangement.placements[0])
                .map(|c| c.name.clone())
        };
        assert_eq!(name(&p).as_deref(), Some("edited"));
        // The clip moved to another row, then to the bass's grid: the
        // block is untouched and still plays it, on its own lane.
        let c = p.tracks[0].slots[1].take();
        p.tracks[0].slots[5] = c;
        assert_eq!(name(&p).as_deref(), Some("edited"));
        let c = p.tracks[0].slots[5].take();
        p.tracks[1].slots[7] = c;
        assert_eq!(name(&p).as_deref(), Some("edited"));
        assert_eq!(
            p.find_clip(ClipId(2)),
            Some(ClipAt::Cell { track: 1, row: 7 })
        );
        assert_eq!(p.arrangement.placements[0].track, 0);
        assert_eq!(p.check(), Ok(()));
        // Made unique: a copy among the song's clips with the next id.
        assert_eq!(p.make_unique(0), Ok(ClipId(5)));
        assert_eq!(p.find_clip(ClipId(5)), Some(ClipAt::Song));
        p.tracks[1].slots[7].as_mut().unwrap().name = "again".into();
        assert_eq!(name(&p).as_deref(), Some("edited"), "its own copy now");
        assert_eq!(p.make_unique(0), Ok(ClipId(5)), "already its own");
        assert_eq!(p.check(), Ok(()));
        // A cell emptied: its block stays, silent, and plays again when
        // the clip comes back with its id.
        p.arrangement
            .place(Placement::of(&p.tracks, 1, 0, 0, BAR))
            .unwrap();
        let gone = p.tracks[1].slots[0].take();
        assert_eq!(p.check(), Ok(()));
        assert!(p.block_clip(&p.arrangement.placements[1]).is_none());
        p.tracks[1].slots[0] = gone;
        assert!(p.block_clip(&p.arrangement.placements[1]).is_some());
        // Deleting a block lets go of the song clip only it played.
        assert!(p.arrangement.remove(0).is_some());
        assert!(p.arrangement.clips.is_empty());
        // A song clip with a cell's id doesn't check: a clip lives in one
        // place.
        let mut twin = p.tracks[0].slots[0].clone().unwrap();
        twin.name = "twin".into();
        p.arrangement.clips.keep(twin).unwrap();
        assert_eq!(p.check(), Err(ModelError::BadPlacement));
    }

    #[test]
    fn an_offset_moves_the_origin_back_at_the_clips_speed() {
        let mut c = Clip::new("c", BAR);
        c.speed = crate::Speed::new(1, 2).unwrap();
        let mut pl = Placement::new(0, ClipId(1), 10 * BAR, BAR);
        pl.offset = 960;
        assert_eq!(pl.origin(&c), 10 * BAR - 1920, "half speed: twice as far");
    }

    /// The song list: each step's span is its scene's length times its
    /// times, one after another (List → song plays them: the app's time
    /// engine); the list checks its steps.
    #[test]
    fn the_song_list_has_spans_and_checks_its_steps() {
        use crate::scenes::{ListStep, MAX_TIMES};
        let mut p = project();
        let step = |row, times| ListStep { row, times };
        p.arrangement.list.steps = vec![step(0, 2), step(1, 1), step(0, 3)];
        assert_eq!(
            list_spans(&p, BAR),
            vec![(0, 2 * BAR), (2 * BAR, 2 * BAR), (4 * BAR, 3 * BAR)]
        );
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

    /// A linked song (phase 4b) from list steps, as linking builds it:
    /// the rows laid by rule ([`scenes_to_song`]), each step one section
    /// of its passes, its marker carrying its scene and passes.
    fn linked(p: &mut Project, steps: &[(usize, u16)]) {
        let rows: Vec<(usize, Tick)> = steps
            .iter()
            .map(|&(row, n)| (row, scene_length(p, row, BAR) * Tick::from(n)))
            .collect();
        let mut song = scenes_to_song(p, &rows, 0).unwrap().song;
        for (m, &(row, passes)) in song.markers.iter_mut().zip(steps) {
            m.scene = Some(FromScene::following(row, scene_length(p, row, BAR)));
            m.passes = passes;
        }
        song.list.linked = true;
        p.arrangement = song;
    }

    /// Phase 4b.1: a linked list reads the song's sections in order, each
    /// with its marker's bars, scene and passes; the bars before the first
    /// marker are a section with none; the last runs its scene's passes,
    /// or to the song's end once it is the song's own.
    #[test]
    fn a_linked_list_is_the_songs_sections_with_their_scenes_and_passes() {
        let mut p = project();
        // Row 2 is two bars: × 3 is six.
        linked(&mut p, &[(0, 2), (1, 3), (0, 1)]);
        p.check().unwrap();
        let parts = song_sections(&p, BAR);
        let from = |row| Some(FromScene::following(row, scene_length(&p, row, BAR)));
        let section = |k, start, end, scene, passes| SongSection {
            marker: Some(k),
            start,
            end,
            scene,
            passes,
        };
        assert_eq!(
            parts,
            vec![
                section(0, 0, 2 * BAR, from(0), 2),
                section(1, 2 * BAR, 8 * BAR, from(1), 3),
                section(2, 8 * BAR, 9 * BAR, from(0), 1),
            ]
        );
        for (s, m) in parts.iter().zip(&p.arrangement.markers) {
            assert_eq!((s.start, s.scene, s.passes), (m.at, m.scene, m.passes));
            let row = m.scene.unwrap().row;
            assert_eq!(s.pass(), scene_length(&p, row, BAR), "a pass is its scene");
        }
        // The last section follows its scene: a pass as long as it was
        // made, past the blocks for a row whose follow plays longer than
        // its clips; the row's length now doesn't move it.
        p.arrangement.markers[2].scene = Some(FromScene::following(0, 4 * BAR));
        assert_eq!(song_sections(&p, BAR)[2].end, 12 * BAR);
        p.scene_mut(0).unwrap().follow = Some(Follow::after(8, FollowAction::Next));
        assert_eq!(song_sections(&p, BAR)[2].end, 12 * BAR);
        // Changed in the song, it runs to the song's end in whole bars.
        p.arrangement.markers[2].scene = Some(FromScene {
            changed: true,
            ..FromScene::following(0, 4 * BAR)
        });
        assert_eq!(song_sections(&p, BAR)[2].end, 9 * BAR);
        // Bars before the first marker: a section with no marker.
        p.arrangement.markers.remove(0);
        let parts = song_sections(&p, BAR);
        assert_eq!(
            parts[0],
            SongSection {
                marker: None,
                start: 0,
                end: 2 * BAR,
                scene: None,
                passes: 1,
            }
        );
        assert_eq!(parts.len(), 3);
        // No marker: the whole song, in whole bars; no song: nothing.
        p.arrangement.markers.clear();
        assert_eq!(
            song_sections(&p, BAR)
                .iter()
                .map(|s| (s.marker, s.start, s.end))
                .collect::<Vec<_>>(),
            vec![(None, 0, 9 * BAR)]
        );
        p.arrangement = Arrangement::default();
        assert!(song_sections(&p, BAR).is_empty());
    }

    /// Phase 4b.3: an edit inside a section that follows its scene makes
    /// it the song's own (changed, × 1); one that moves sections whole, or
    /// changes only the tempo, leaves them following.
    #[test]
    fn an_edit_inside_a_section_makes_it_the_songs_own_and_a_whole_move_doesnt() {
        let mut p = project();
        // Row 1 × 2 (two bars), row 2 × 3 (six), row 1 × 1 (one).
        linked(&mut p, &[(0, 2), (1, 3), (0, 1)]);
        let tracks = p.tracks.clone();
        let changed = |song: &Arrangement| -> Vec<(bool, u16)> {
            song.markers
                .iter()
                .map(|m| (m.scene.is_some_and(|f| f.changed), m.passes))
                .collect()
        };
        let mut song = p.arrangement.clone();
        // The last section moved first, and a copy of the second after it:
        // whole sections, so nothing is marked.
        let before = song.clone();
        song.move_section(&tracks, 2, 0, false, BAR, 120.0).unwrap();
        song.move_section(&tracks, 2, 3, true, BAR, 120.0).unwrap();
        assert!(song.mark_changed(&tracks, &before, BAR).is_empty());
        assert_eq!(
            changed(&song),
            [(false, 1), (false, 2), (false, 3), (false, 3)]
        );
        // A block split in phase (Split, phase 6) plays as it did whole:
        // nothing is marked.
        let before = song.clone();
        let i = song.at(1, 4 * BAR + BAR / 2).unwrap();
        song.split(&tracks, i, 4 * BAR + BAR / 2).unwrap();
        assert!(song.mark_changed(&tracks, &before, BAR).is_empty());
        // So is a tempo point.
        let before = song.clone();
        song.tempo.push(TempoPoint {
            at: 4 * BAR,
            bpm: 90.0,
            ramp: false,
        });
        assert!(song.mark_changed(&tracks, &before, BAR).is_empty());
        // A bar of the bass taken out of the third section (its block runs
        // on into the copy, joined to it): it is the song's own, its three
        // passes plain bars, its bars as they were; the copy still follows.
        let before = song.clone();
        song.clear(&tracks, 1, 4 * BAR, 5 * BAR).unwrap();
        assert_eq!(song.mark_changed(&tracks, &before, BAR), [2]);
        assert_eq!(
            changed(&song),
            [(false, 1), (false, 2), (true, 1), (false, 3)]
        );
        let parts = song.parts(BAR);
        assert_eq!((parts[2].start, parts[2].end), (3 * BAR, 9 * BAR));
        // A block put after the last section: it is in it now, and the
        // song's own, running to the song's end.
        let before = song.clone();
        let end = song.parts(BAR)[3].end;
        song.place(Placement::of(&tracks, 0, 0, end, BAR)).unwrap();
        assert_eq!(song.mark_changed(&tracks, &before, BAR), [3]);
        assert_eq!(song.parts(BAR)[3].end, end + BAR);
        // A marker put inside the first section: its first bar is all of
        // it now, and the new marker's section is the song's own.
        let before = song.clone();
        song.add_marker(BAR / 2, "Cut").unwrap();
        assert_eq!(song.mark_changed(&tracks, &before, BAR), [0]);
        assert_eq!(song.markers[1].scene, None);
        // Nothing follows any more: nothing to look at.
        let before = song.clone();
        song.markers[2].scene = None;
        assert!(song.mark_changed(&tracks, &before, BAR).is_empty());
        // The last section, following with a pass longer than its blocks
        // (a follow longer than its clips), moves with its silence.
        let mut song = p.arrangement.clone();
        song.markers[2].scene = Some(FromScene::following(0, 4 * BAR));
        assert_eq!(song.section(2, BAR), Some((8 * BAR, 12 * BAR)));
        assert_eq!(song.parts(BAR)[2].end, 12 * BAR);
    }

    /// Phase 4b.4: the section found at a tick, the one after it and the
    /// sections' span, found without making the list, are the list's.
    #[test]
    fn a_part_found_at_a_tick_is_the_one_the_list_has() {
        let mut p = project();
        linked(&mut p, &[(0, 2), (1, 3), (0, 1)]);
        let check = |song: &Arrangement| {
            let parts = song.parts(BAR);
            for t in (0..14).map(|k| k * BAR / 2) {
                let at = parts.iter().find(|s| (s.start..s.end).contains(&t));
                assert_eq!(song.part_at(t, BAR).as_ref(), at, "{t}");
                let after = parts.iter().find(|s| s.start > t);
                assert_eq!(song.part_after(t, BAR).as_ref(), after, "{t}");
            }
            let span = parts
                .first()
                .zip(parts.last())
                .map(|(a, b)| (a.start, b.end));
            assert_eq!(song.parts_span(BAR), span);
        };
        let mut song = p.arrangement.clone();
        check(&song);
        // The bars before the first marker, the last section's silence,
        // no marker, no song.
        song.markers.remove(0);
        song.markers[1].scene = Some(FromScene::following(0, 4 * BAR));
        check(&song);
        assert_eq!(song.parts_span(BAR), Some((0, 12 * BAR)));
        song.markers.clear();
        check(&song);
        check(&Arrangement::default());
        assert_eq!(Arrangement::default().parts_span(BAR), None);
    }

    /// A marker's passes are 1 to 99, its scene a row with a pass; a
    /// linked list has no steps of its own.
    #[test]
    fn a_marker_checks_its_scene_and_passes_and_a_linked_list_its_steps() {
        use crate::scenes::ListStep;
        let mut p = project();
        linked(&mut p, &[(0, 1)]);
        assert_eq!(p.check(), Ok(()));
        for passes in [0, MAX_TIMES + 1] {
            let mut q = p.clone();
            q.arrangement.markers[0].passes = passes;
            assert_eq!(q.check(), Err(ModelError::BadPlacement), "×{passes}");
        }
        for (row, pass) in [(SCENES, BAR), (0, 0), (0, Tick::from(SONG_LANE_LEN) + 1)] {
            let mut q = p.clone();
            q.arrangement.markers[0].scene = Some(FromScene::following(row, pass));
            assert_eq!(q.check(), Err(ModelError::BadPlacement), "{row} {pass}");
        }
        let mut q = p.clone();
        q.arrangement.list.steps = vec![ListStep { row: 0, times: 1 }];
        assert_eq!(q.check(), Err(ModelError::BadScene));
        q.arrangement.list.linked = false;
        assert_eq!(q.check(), Ok(()), "a list of its own keeps its steps");
    }

    /// A stretch filled again (4b.2, a linked section made again from its
    /// scene): what played there goes on every track, cut at its edges as
    /// a clear cuts; the new blocks go in; a block outside the stretch, or
    /// one running into another, is refused and nothing changes.
    #[test]
    fn a_stretch_filled_again_plays_only_its_new_blocks() {
        let p = project();
        let mut s = Arrangement::default();
        s.place(Placement::of(&p.tracks, 0, 1, 0, 6 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, BAR, 2 * BAR))
            .unwrap();
        let was = s.clone();
        let new = [Placement::of(&p.tracks, 1, 1, 2 * BAR, 2 * BAR)];
        s.fill(&p.tracks, 2 * BAR, 4 * BAR, &new).unwrap();
        let said: Vec<(usize, Tick, Tick, Tick)> = s
            .placements
            .iter()
            .map(|b| (b.track, b.start, b.length, b.offset))
            .collect();
        // The drums' block is cut either side, the rest in phase; the
        // bass's old block is cut where the stretch starts.
        assert_eq!(
            said,
            [
                (0, 0, 2 * BAR, 0),
                (0, 4 * BAR, 2 * BAR, 0),
                (1, BAR, BAR, 0),
                (1, 2 * BAR, 2 * BAR, 0),
            ]
        );
        let mut t = was.clone();
        let outside = [Placement::of(&p.tracks, 1, 1, 3 * BAR, 2 * BAR)];
        assert_eq!(
            t.fill(&p.tracks, 2 * BAR, 4 * BAR, &outside),
            Err(ModelError::BadPlacement)
        );
        let twice = [new[0].clone(), new[0].clone()];
        assert_eq!(
            t.fill(&p.tracks, 2 * BAR, 4 * BAR, &twice),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(t, was);
    }

    /// A section moved or copied takes its marker's scene and passes with
    /// it (phase 3's section moves, which a linked list's edits use).
    #[test]
    fn a_moved_section_keeps_its_scene_and_passes() {
        let mut p = project();
        linked(&mut p, &[(0, 2), (1, 3)]);
        let tracks = p.tracks.clone();
        p.arrangement
            .move_section(&tracks, 1, 0, false, BAR, 120.0)
            .unwrap();
        let said: Vec<(Tick, Option<usize>, u16)> = p
            .arrangement
            .markers
            .iter()
            .map(|m| (m.at, m.scene.map(|s| s.row), m.passes))
            .collect();
        assert_eq!(said, vec![(0, Some(1), 3), (6 * BAR, Some(0), 2)]);
        p.arrangement
            .move_section(&tracks, 0, 2, true, BAR, 120.0)
            .unwrap();
        assert_eq!(p.arrangement.markers[2].scene.map(|s| s.row), Some(1));
        assert_eq!(p.arrangement.markers[2].passes, 3);
    }

    /// Composing tools S2.5: a row's chords run for the row's length and
    /// say when they are all suggested; a row with no line has none, and
    /// a row with only chords isn't an empty row.
    #[test]
    fn a_rows_chords_run_for_its_length() {
        use mpfree_context::chord::{Chord, ChordTimeline, Quality};
        let mut p = project();
        assert_eq!(row_harmony(&p, 0, BAR), None);
        let cm = Chord::new(0, Quality::Minor7);
        p.scene_mut(0).unwrap().chords = ChordTimeline::of(&[(cm, BAR / 2)], Heard::Suggested);
        let r = row_harmony(&p, 0, BAR).unwrap();
        assert_eq!((r.length, r.line.len(), r.suggested), (BAR, BAR / 2, true));
        p.scene_mut(0).unwrap().chords = ChordTimeline::of(&[(cm, BAR), (cm, BAR)], Heard::Set);
        p.scene_mut(0).unwrap().follow = Some(Follow::after(4, FollowAction::Next));
        let r = row_harmony(&p, 0, BAR).unwrap();
        assert_eq!((r.length, r.suggested), (4 * BAR, false));
        let free = (0..SCENES).find(|&row| !p.row_filled(row)).unwrap();
        assert!(p.row_empty(free));
        p.scene_mut(free).unwrap().chords = ChordTimeline::of(&[(cm, BAR)], Heard::Set);
        assert!(
            !p.row_empty(free),
            "a row set up without clips is never taken"
        );
        assert_eq!(p.check(), Ok(()));
        p.scene_mut(free).unwrap().chords = ChordTimeline::of(
            &vec![(cm, 1); crate::scenes::MAX_ROW_CHORDS + 1],
            Heard::Set,
        );
        assert_eq!(p.check(), Err(ModelError::BadScene));
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
        assert_eq!(sections(&p, &[]).len(), 3);
        let rows = song_to_scenes(&mut p, BAR, &quiet()).unwrap().rows;
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
            song_to_scenes(&mut p, BAR, &quiet()).map(|c| c.rows),
            Ok(vec![5, 6, 7])
        );
        // No run of 3 left in bank A: bank B's first rows (scene banks,
        // 5.6), and never a run across two banks, since Next goes round
        // within one.
        let mut q = before;
        q.tracks[0].slots[6] = Some(Clip::new("x", BAR));
        q.tracks[0].slots[11] = Some(Clip::new("x", BAR));
        q.tracks[0].slots[17] = Some(Clip::new("x", BAR));
        let mut b = q.clone();
        assert_eq!(
            song_to_scenes(&mut b, BAR, &quiet()).map(|c| c.rows),
            Ok(vec![18, 19, 20])
        );
        // Every bank with no run of 3: no room.
        for row in (16..ROWS).step_by(2) {
            q.tracks[0].slots[row] = Some(Clip::new("x", BAR));
        }
        let again = q.clone();
        assert_eq!(
            song_to_scenes(&mut q, BAR, &quiet()).map(|c| c.rows),
            Err(ScenesRefused::Model(ModelError::NoRoom))
        );
        assert_eq!(q, again, "nothing changed");
    }

    /// Phase 6's block tools. Split cuts a block in phase (at the clip's
    /// speed), never on an edge; Split at the playhead cuts every lane;
    /// Duplicate puts a copy after the block where there is room; trim
    /// from the left keeps what plays where it was; slide moves what
    /// plays inside the block.
    /// Several blocks picked (phase 6): moved, copied, duplicated, split
    /// and deleted together, keeping their places to each other, all or
    /// none.
    #[test]
    fn several_blocks_move_copy_duplicate_and_go_together_all_or_none() {
        let mut p = project();
        p.tracks.push(Track::new("keys", 0));
        p.tracks.push(Track::new_loops("loops", 0));
        p.tracks.push(Track::new_audio("vox", 0));
        let blocks = |s: &Arrangement| -> Vec<(usize, Tick, Tick)> {
            s.placements
                .iter()
                .map(|b| (b.track, b.start, b.length))
                .collect()
        };
        let mut s = Arrangement::default();
        // Drums' a at bar 1, bass's b from bar 2, drums' b from bar 5.
        s.place(Placement::of(&p.tracks, 0, 0, 0, BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 1, BAR, 2 * BAR))
            .unwrap();
        s.place(Placement::of(&p.tracks, 0, 1, 4 * BAR, 2 * BAR))
            .unwrap();
        let start = s.clone();
        let first_two = [s.at(0, 0).unwrap(), s.at(1, BAR).unwrap()];
        // Two bars later and a lane down: the drums' a onto the bass's
        // lane, the bass's b onto the keys', linked to their clips.
        let moved = s
            .move_placements(&p.tracks, &first_two, 1, 2 * BAR, false)
            .unwrap();
        assert_eq!(
            blocks(&s),
            [
                (0, 4 * BAR, 2 * BAR),
                (1, 2 * BAR, BAR),
                (2, 3 * BAR, 2 * BAR)
            ]
        );
        assert_eq!(moved, [1, 2]);
        assert_eq!(s.placements[1].clip, start.placements[0].clip);
        // Back again, in the other order and with an index twice.
        s.move_placements(&p.tracks, &[2, 1, 2], -1, -2 * BAR, false)
            .unwrap();
        assert_eq!(s, start);
        // Copies: the originals stay.
        let copies = s
            .move_placements(&p.tracks, &first_two, 0, 8 * BAR, true)
            .unwrap();
        assert_eq!(s.placements.len(), 5);
        assert_eq!(
            copies
                .iter()
                .map(|&i| (s.placements[i].track, s.placements[i].start))
                .collect::<Vec<_>>(),
            [(0, 8 * BAR), (1, 9 * BAR)]
        );
        s = start.clone();
        // All or none: one landing on a block outside the set, one before
        // bar 1, off the lanes, across a Loop lane, onto an audio track.
        for (lanes, by, err) in [
            (0, 4 * BAR, ModelError::BadPlacement),
            (0, -BAR, ModelError::BadPlacement),
            (-1, 0, ModelError::BadPlacement),
            (3, 0, ModelError::BadLoop),
            (4, 0, ModelError::AudioTrack),
        ] {
            // The bass's block alone across the Loop lane, the drums' onto
            // the audio lane.
            let lone = match lanes {
                3 => &[1][..],
                4 => &[0][..],
                _ => &first_two[..],
            };
            assert_eq!(
                s.move_placements(&p.tracks, lone, lanes, by, false),
                Err(err),
                "{lanes} {by}"
            );
            assert_eq!(s, start, "nothing changed");
        }
        // Moving onto a block of the set's own is fine: all three a bar on.
        s.move_placements(&p.tracks, &[0, 1, 2], 0, BAR, false)
            .unwrap();
        assert_eq!(
            blocks(&s),
            [(0, BAR, BAR), (0, 5 * BAR, 2 * BAR), (1, 2 * BAR, 2 * BAR)]
        );
        s = start.clone();
        assert_eq!(
            s.move_placements(&p.tracks, &[], 0, BAR, false),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(
            s.move_placements(&p.tracks, &[0, 7], 0, BAR, false),
            Err(ModelError::BadPlacement)
        );
        // Duplicate: the first two again right after the later's end (bar
        // 4), the drums' b from bar 5 not in their way.
        let copies = s.duplicate_placements(&p.tracks, &first_two).unwrap();
        assert_eq!(
            copies
                .iter()
                .map(|&i| (s.placements[i].track, s.placements[i].start))
                .collect::<Vec<_>>(),
            [(0, 3 * BAR), (1, 4 * BAR)]
        );
        // The first two again: their copies are in the way now.
        let snapshot = s.clone();
        let again = [s.at(0, 0).unwrap(), s.at(1, BAR).unwrap()];
        assert_eq!(
            s.duplicate_placements(&p.tracks, &again),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(s, snapshot);
        // Delete several: a song clip only they played goes with them.
        let mut q = p.clone();
        q.arrangement = start.clone();
        let own = q.make_unique(0).unwrap();
        let mut d = q.arrangement;
        assert!(d.clips.get(own).is_some());
        assert_eq!(d.remove_placements(&[0, 1, 0]), Ok(2));
        assert_eq!(blocks(&d), [(1, BAR, 2 * BAR)]);
        assert!(d.clips.get(own).is_none());
        assert_eq!(d.remove_placements(&[3]), Err(ModelError::BadPlacement));
        assert_eq!(d.remove_placements(&[]), Err(ModelError::BadPlacement));
        // Split the picked ones at bar 3 beat 1: the bass's b alone plays
        // across it, so it alone is cut, in phase; the drums' a is left.
        s = start.clone();
        let after = s.split_these(&p.tracks, &first_two, 2 * BAR).unwrap();
        assert_eq!(
            blocks(&s),
            [
                (0, 0, BAR),
                (0, 4 * BAR, 2 * BAR),
                (1, BAR, BAR),
                (1, 2 * BAR, BAR)
            ]
        );
        assert_eq!(after, [3]);
        assert_eq!(s.placements[3].offset, BAR);
        // Both bass and drums cut at bar 5 beat 2, the parts after picked.
        let mut t = start.clone();
        let all: Vec<usize> = (0..3).collect();
        let at = 4 * BAR + BAR / 4;
        assert_eq!(t.split_these(&p.tracks, &all, at), Ok(vec![2]));
        t = start.clone();
        t.place(Placement::of(&p.tracks, 1, 0, 4 * BAR, 2 * BAR))
            .unwrap();
        let all: Vec<usize> = (0..4).collect();
        assert_eq!(t.split_these(&p.tracks, &all, at), Ok(vec![2, 5]));
        // None across it, or no such block: nothing changes.
        let snapshot = s.clone();
        assert_eq!(
            s.split_these(&p.tracks, &[0], 2 * BAR),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(
            s.split_these(&p.tracks, &[9], 2 * BAR),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(s, snapshot);
    }

    #[test]
    fn split_duplicate_trim_and_slide_keep_what_plays_in_phase() {
        let p = project();
        let blocks = |s: &Arrangement, t: usize| -> Vec<(Tick, Tick, Tick)> {
            s.on_track(t)
                .iter()
                .map(|b| (b.start, b.length, b.offset))
                .collect()
        };
        let mut s = Arrangement::default();
        // The drums' two-bar b for 4 bars, the bass's one-bar a for 2.
        s.place(Placement::of(&p.tracks, 0, 1, 0, 4 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 0, 2 * BAR)).unwrap();
        // Split: 3 bars into a two-bar clip is a bar into it.
        assert_eq!(s.split(&p.tracks, 0, 3 * BAR), Ok(1));
        assert_eq!(blocks(&s, 0), [(0, 3 * BAR, 0), (3 * BAR, BAR, BAR)]);
        for at in [0, 3 * BAR, 4 * BAR, -1] {
            assert_eq!(
                s.split(&p.tracks, 1, at),
                Err(ModelError::BadPlacement),
                "{at}"
            );
        }
        assert_eq!(s.split(&p.tracks, 9, BAR), Err(ModelError::BadPlacement));
        // At half speed a bar of the song is half a bar of the clip.
        let mut q = p.clone();
        q.tracks[0].slots[1].as_mut().unwrap().speed = crate::Speed::new(1, 2).unwrap();
        let mut h = Arrangement::default();
        h.place(Placement::of(&q.tracks, 0, 1, 0, 4 * BAR)).unwrap();
        h.split(&q.tracks, 0, BAR).unwrap();
        assert_eq!(blocks(&h, 0), [(0, BAR, 0), (BAR, 3 * BAR, BAR / 2)]);
        // Split at the playhead: every lane across it, none on an edge.
        let mut all = s.clone();
        assert_eq!(all.split_all(&p.tracks, BAR + 100), Ok(2));
        assert_eq!(
            blocks(&all, 0),
            [
                (0, BAR + 100, 0),
                (BAR + 100, 2 * BAR - 100, BAR + 100),
                (3 * BAR, BAR, BAR)
            ]
        );
        assert_eq!(
            blocks(&all, 1),
            [(0, BAR + 100, 0), (BAR + 100, BAR - 100, 100)]
        );
        assert_eq!(all.split_all(&p.tracks, 3 * BAR), Ok(0), "on an edge");
        // Duplicate: the bass's block again after it, then no room.
        let i = s.at(1, 0).unwrap();
        let copy = s.duplicate(i).unwrap();
        assert_eq!(blocks(&s, 1), [(0, 2 * BAR, 0), (2 * BAR, 2 * BAR, 0)]);
        assert_eq!(
            s.placements[copy].clip, s.placements[i].clip,
            "the same clip"
        );
        let first = s.at(0, 0).unwrap();
        assert_eq!(
            s.duplicate(first),
            Err(ModelError::BadPlacement),
            "the drums' next block is in the way"
        );
        // Trim from the left: later leaves out the clip's first ticks,
        // earlier brings them back.
        let last = s.at(0, 3 * BAR).unwrap();
        s.trim_start(&p.tracks, last, 3 * BAR + 10).unwrap();
        assert_eq!(blocks(&s, 0)[1], (3 * BAR + 10, BAR - 10, BAR + 10));
        s.remove(first);
        let last = s.at(0, 3 * BAR + 10).unwrap();
        s.trim_start(&p.tracks, last, BAR).unwrap();
        assert_eq!(
            blocks(&s, 0),
            [(BAR, 3 * BAR, BAR)],
            "back past its top, in phase: a bar into the two-bar clip, as it played from 0"
        );
        let mut t = s.clone();
        t.place(Placement::of(&p.tracks, 0, 0, 0, BAR / 2)).unwrap();
        let k = t.at(0, BAR).unwrap();
        for start in [BAR / 2 - 1, 4 * BAR, -1] {
            assert_eq!(
                t.trim_start(&p.tracks, k, start),
                Err(ModelError::BadPlacement),
                "{start}"
            );
        }
        assert_eq!(
            t.trim_start(&p.tracks, k, BAR / 2),
            Ok(()),
            "up to the block before"
        );
        // Slide: the clip's ticks later under the block, going round.
        let k = s.at(0, BAR).unwrap();
        s.slide(&p.tracks, k, BAR + 100).unwrap();
        assert_eq!(blocks(&s, 0), [(BAR, 3 * BAR, 2 * BAR - 100)]);
        s.slide(&p.tracks, k, -100).unwrap();
        assert_eq!(blocks(&s, 0), [(BAR, 3 * BAR, 0)]);
        assert_eq!(s.check(&p.tracks), Ok(()));
    }

    /// A Keep playing track (phase 5.2) runs its clip on through a row
    /// with no clip for it, its block longer by the row; a kept track
    /// with nothing playing stays silent, and a track not kept stops.
    #[test]
    fn a_keep_playing_track_runs_its_block_on_through_a_row_without_a_clip() {
        let mut p = project();
        p.scene_mut(2).unwrap().set_keep(0, true).unwrap();
        p.scene_mut(3).unwrap().set_keep(1, true).unwrap();
        let song = scenes_to_song(&p, &[(1, 2 * BAR), (2, BAR), (3, BAR)], 0)
            .unwrap()
            .song;
        let blocks: Vec<_> = song
            .placements
            .iter()
            .map(|b| (b.track, row(&p, b), b.start, b.length))
            .collect();
        assert_eq!(
            blocks,
            [(0, Some(1), 0, 3 * BAR), (1, Some(1), 0, 2 * BAR)],
            "the drums' b runs on through row 3; the bass stops there, and row 4 keeps nothing playing"
        );
        assert_eq!(song.markers.len(), 3, "each row still marks its bars");
        // Kept, then a clip of its own: a block of its own after.
        let song = scenes_to_song(&p, &[(0, BAR), (2, BAR), (0, BAR)], 0)
            .unwrap()
            .song;
        let drums: Vec<_> = song
            .on_track(0)
            .iter()
            .map(|b| (b.start, b.length))
            .collect();
        assert_eq!(drums, [(0, 2 * BAR), (2 * BAR, BAR)]);
    }

    /// The sections that play a scene (the Song view's Chords lane): one
    /// Scenes → song made is known by its blocks, the row's clips; one
    /// made from a scene names it; one with a clip of the song's own plays
    /// no one scene.
    #[test]
    fn scene_sections_name_the_row_a_section_plays() {
        let mut p = project();
        p.arrangement = scenes_to_song(&p, &[(0, BAR), (1, 2 * BAR), (0, BAR)], 0)
            .unwrap()
            .song;
        assert_eq!(
            scene_sections(&p, BAR),
            [(0, BAR, 0), (BAR, 3 * BAR, 1), (3 * BAR, 4 * BAR, 0)]
        );
        // Bar 2's blocks made the song's own: that section plays no scene,
        // until it is linked to one.
        let k = p
            .arrangement
            .placements
            .iter()
            .position(|b| b.start == BAR)
            .unwrap();
        let own = p.new_clip_id();
        p.arrangement.placements[k].clip = own;
        assert_eq!(
            scene_sections(&p, BAR),
            [(0, BAR, 0), (3 * BAR, 4 * BAR, 0)]
        );
        p.arrangement.markers[1].scene = Some(FromScene::following(1, 2 * BAR));
        assert_eq!(scene_sections(&p, BAR)[1], (BAR, 3 * BAR, 1));
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
        assert_eq!(sections(&p, &[]).len(), 2);
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
        assert_eq!(
            song_to_scenes(&mut p, BAR, &quiet()).map(|c| c.rows),
            Ok(vec![9, 10])
        );
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
        p.give_clip_ids();
        let s = &mut p.arrangement;
        // Drums: the four-bar clip for 8 bars; bass joins on bar 3.
        s.place(Placement::of(&p.tracks, 0, 2, 0, 8 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 2 * BAR, 6 * BAR))
            .unwrap();
        let had = p.next_clip;
        let done = song_to_scenes(&mut p, BAR, &quiet()).unwrap();
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
            p.give_clip_ids();
            let s = &mut p.arrangement;
            s.place(Placement::of(&p.tracks, 0, 2, start, 8 * BAR))
                .unwrap();
            s.place(Placement::of(&p.tracks, 1, 0, start + 2 * BAR, 6 * BAR))
                .unwrap();
            p
        };
        let four = || {
            let mut c = Clip::new("four", 4 * BAR);
            c.notes = (0..4).map(|k| Note::new(k * BAR, 100, 10, 100)).collect();
            c
        };
        let diffs = |mut p: Project| -> Vec<(Option<usize>, Tick, Differs)> {
            song_to_scenes(&mut p, BAR, &quiet())
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
        // The loop cell in the clip's place, with its id.
        let mut cell = Clip::new_loop(0, 4 * BAR);
        cell.id = p.clip(0, 2).unwrap().id;
        loops.slots[2] = Some(cell);
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
            .place(Placement::of(&p.tracks, 1, 0, 11 * BAR, BAR))
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
            .place(Placement::of(&p.tracks, 0, 0, 0, 1000 * BAR))
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
        // A clip with no id yet can't be placed.
        assert_eq!(
            scenes_to_song(&p, &[(2, 4 * BAR)], 0),
            Err(ModelError::BadPlacement)
        );
        p.give_clip_ids();
        let done = scenes_to_song(&p, &[(2, 4 * BAR), (3, BAR), (4, BAR)], 0).unwrap();
        let blocks: Vec<(usize, usize, Tick, Tick)> = done
            .song
            .placements
            .iter()
            .map(|b| (b.track, row(&p, b).unwrap(), b.start, b.length))
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

    /// The Arrange refactor's 5.2b: a row's tempo is a tempo point where it
    /// starts in the song laid by rule (none where the tempo is that
    /// already, and named past the song's room), and Song → scenes gives a
    /// row the tempo point on its part's start, naming a point inside a
    /// part, a ramp to another point, or a tempo a scene can't hold.
    #[test]
    fn scene_tempi_become_tempo_points_and_back() {
        let mut p = project();
        p.scene_mut(0).unwrap().set_bpm(Some(100.0)).unwrap();
        p.scene_mut(1).unwrap().set_bpm(Some(140.5)).unwrap();
        let rows = [(0, BAR), (1, 2 * BAR), (0, BAR), (0, BAR)];
        let done = scenes_to_song(&p, &rows, 0).unwrap();
        let points = |s: &Arrangement| -> Vec<(Tick, f64, bool)> {
            s.tempo.iter().map(|t| (t.at, t.bpm, t.ramp)).collect()
        };
        assert_eq!(
            points(&done.song),
            [
                (0, 100.0, false),
                (BAR, 140.5, false),
                (3 * BAR, 100.0, false)
            ]
        );
        assert!(done.differences.is_empty(), "{:?}", done.differences);
        // And back: each row the tempo on its part's start.
        let back = |song: &Arrangement| {
            let mut q = project();
            q.arrangement = song.clone();
            let to = song_to_scenes(&mut q, BAR, &quiet()).unwrap();
            let tempi: Vec<Option<f64>> =
                to.rows.iter().map(|&r| q.scene(r).unwrap().bpm()).collect();
            let named: Vec<Tick> = to
                .differences
                .iter()
                .filter(|d| d.what == Differs::Tempo)
                .map(|d| d.at)
                .collect();
            (tempi, named)
        };
        assert_eq!(
            back(&done.song),
            (vec![Some(100.0), Some(140.5), Some(100.0), None], vec![])
        );
        // A ramp on the last point plays flat: still exact.
        let mut song = done.song.clone();
        song.tempo[2].ramp = true;
        assert!(back(&song).1.is_empty());
        // A ramp to another point, a point inside a part, and a tempo
        // finer than 0.1 BPM are named.
        let mut song = done.song.clone();
        song.tempo[1].ramp = true;
        assert_eq!(back(&song).1, [BAR]);
        let mut song = done.song.clone();
        song.set_tempo(2 * BAR, 120.0, None).unwrap();
        assert_eq!(back(&song).1, [2 * BAR]);
        let mut song = done.song.clone();
        song.set_tempo(0, 100.05, None).unwrap();
        assert_eq!(back(&song).1, [0]);
        // Past the song's room for points: named, each that changes it.
        let rows: Vec<(usize, Tick)> = (0..40).map(|k| (k % 2, BAR)).collect();
        let done = scenes_to_song(&p, &rows, 0).unwrap();
        assert_eq!(done.song.tempo.len(), MAX_TEMPO_POINTS);
        let named = done
            .differences
            .iter()
            .filter(|d| d.what == Differs::SceneTempo)
            .count();
        // The rows after the 31st point: those at 140.5 BPM (the odd ones)
        // can't change it; those at 100 play at the tempo already there.
        assert_eq!(named, 5);
        // A song whose last point ramps (and so plays flat) stops ramping
        // when a point comes after it.
        let mut song = Arrangement::default();
        song.set_tempo(0, 90.0, Some(true)).unwrap();
        assert!(put_scene_tempo(&mut song, 4 * BAR, 90.0, None));
        assert_eq!(points(&song), [(0, 90.0, true)], "the same tempo: none");
        assert!(put_scene_tempo(&mut song, 4 * BAR, 120.0, None));
        assert_eq!(points(&song), [(0, 90.0, false), (4 * BAR, 120.0, false)]);
        let mut song = Arrangement::default();
        assert!(put_scene_tempo(&mut song, 0, 120.0, Some(120.0)));
        assert!(song.tempo.is_empty(), "the project's tempo already");
    }

    #[test]
    fn sections_merge_where_nothing_changes_and_split_at_markers() {
        let mut p = project();
        // Drums 0..4 bars of one placement; bass 0..2 and 2..4 bars of two.
        let s = &mut p.arrangement;
        s.place(Placement::of(&p.tracks, 0, 0, 0, 4 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 0, 2 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 2 * BAR, 2 * BAR))
            .unwrap();
        assert_eq!(sections(&p, &[]).len(), 2, "the bass's second placement");
        p.arrangement.placements[2].start = 3 * BAR;
        p.arrangement.placements[2].length = BAR;
        // 0..2 both, 2..3 drums only, 3..4 both again.
        assert_eq!(
            sections(&p, &[])
                .iter()
                .map(|s| (s.0, s.1))
                .collect::<Vec<_>>(),
            vec![(0, 2 * BAR), (2 * BAR, 3 * BAR), (3 * BAR, 4 * BAR)]
        );
        p.arrangement.mark(BAR, "B").unwrap();
        assert_eq!(sections(&p, &[]).len(), 4);
    }

    #[test]
    fn a_placement_moves_or_copies_along_and_across_lanes() {
        let p = project();
        let mut s = Arrangement::default();
        s.place(Placement::of(&p.tracks, 0, 0, 0, BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 0, 1, 4 * BAR, 2 * BAR))
            .unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 0, BAR)).unwrap();
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
        // Across to the bass: still the drums' clip, linked (Arrange
        // 1.3), played through the bass.
        let j = s.move_placement(&p.tracks, 1, 1, 4 * BAR, false).unwrap();
        assert_eq!((s.placements[j].track, s.placements[j].start), (1, 4 * BAR));
        assert_eq!(s.placements[j].clip, ClipId(2));
        assert_eq!(s.clip_of(&p.tracks, &s.placements[j]).unwrap().name, "b");
        assert!(s.clips.is_empty(), "no copy");
        // A copy along the lane stays linked; the first stays where it is.
        let n = s.placements.len();
        let c = s.move_placement(&p.tracks, 0, 0, 8 * BAR, true).unwrap();
        assert_eq!(s.placements.len(), n + 1);
        assert_eq!(s.placements[c].clip, ClipId(1));
        assert_eq!(s.placements[0].start, 2 * BAR + 100);
        assert_eq!(s.check(&p.tracks), Ok(()));
        // A copy onto itself overlaps.
        assert_eq!(
            s.move_placement(&p.tracks, 0, 0, 2 * BAR + 100, true),
            Err(ModelError::BadPlacement)
        );
    }

    /// A loop's block stays on its Loop track's lane and a clip's block
    /// off a Loop track's (loop-tracks.md part 5); a loop's block anywhere
    /// else doesn't check, nor a clip's on a Loop track.
    #[test]
    fn a_loops_block_stays_on_its_loop_tracks_lane() {
        let mut p = project();
        let mut a = Track::new_loops("Loops A", 2);
        a.slots[0] = Some(Clip::new_loop(0, BAR));
        p.tracks.push(a);
        p.tracks.push(Track::new_loops("Loops B", 3));
        p.give_clip_ids();
        let mut s = Arrangement::default();
        s.place(Placement::of(&p.tracks, 0, 0, 0, BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 2, 0, 0, BAR)).unwrap();
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
        // The loop's block on a note track doesn't check, nor a clip's on
        // a Loop track, its clip in a cell or the song's own.
        let mut bad = s.clone();
        bad.placements[1].track = 0;
        bad.placements[1].start = 4 * BAR;
        bad.placements.sort_by_key(|p| (p.track, p.start));
        assert_eq!(bad.check(&p.tracks), Err(ModelError::BadLoop));
        let mut bad = s.clone();
        bad.placements[1].clip = ClipId(1);
        assert_eq!(bad.check(&p.tracks), Err(ModelError::BadLoop));
        let mut own = Clip::new("a", BAR);
        own.id = ClipId(40);
        bad.clips.keep(own).unwrap();
        bad.placements[1].clip = ClipId(40);
        assert_eq!(bad.check(&p.tracks), Err(ModelError::BadLoop));
    }

    #[test]
    fn a_placement_gets_longer_up_to_the_next_one() {
        let p = project();
        let mut s = Arrangement::default();
        s.place(Placement::of(&p.tracks, 0, 0, 0, BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 0, 0, 4 * BAR, BAR))
            .unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, BAR, BAR)).unwrap();
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
        s.place(Placement::of(&p.tracks, 0, 0, 0, 8 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 0, 2 * BAR)).unwrap();
        s.place(Placement::of(&p.tracks, 1, 1, 2 * BAR, 2 * BAR))
            .unwrap();
        s.place(Placement::of(&p.tracks, 1, 0, 4 * BAR, 3 * BAR))
            .unwrap();
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
            .move_section(&p.tracks, 1, 0, false, BAR, 120.0)
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
        assert_eq!(row(&p, &s.on_track(1)[0]), Some(1));
        // The drums' long block is cut at the edges, each part in phase,
        // and joined again where it meets itself: its clip is one bar, so
        // every part starts at its top and it stays one block.
        assert_eq!(lane(s, 0), [(0, 8 * BAR, 0)]);
        assert_eq!(s.check(&p.tracks), Ok(()));
        // Moved to where it is: nothing changes.
        let before = p.arrangement.clone();
        p.arrangement
            .move_section(&p.tracks, 1, 2, false, BAR, 120.0)
            .unwrap();
        assert_eq!(p.arrangement, before);
        // The first after the last.
        p.arrangement
            .move_section(&p.tracks, 0, 3, false, BAR, 120.0)
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
            .move_section(&p.tracks, 1, 3, true, BAR, 120.0)
            .unwrap();
        let s = &p.arrangement;
        assert_eq!(
            names(s),
            [(BAR, "A"), (2 * BAR, "B"), (4 * BAR, "C"), (8 * BAR, "B")]
        );
        // The bass: its first block (across A's marker, not cut: nothing
        // moves there), B, C, and B again.
        assert_eq!(
            lane(s, 1),
            [
                (0, 2 * BAR, 0),
                (2 * BAR, 2 * BAR, 0),
                (4 * BAR, 3 * BAR, 0),
                (8 * BAR, 2 * BAR, 0)
            ]
        );
        assert_eq!(row(&p, &s.on_track(1)[3]), Some(1), "linked like the first");
        // The drums play on into the copy: one block.
        assert_eq!(lane(s, 0), [(0, 10 * BAR, 0)]);
        assert_eq!(s.check(&p.tracks), Ok(()));
    }

    /// Markers added, renamed, moved and taken away (Arrange 3.3): only
    /// the markers change, each keeps its name, and a refusal changes
    /// nothing.
    #[test]
    fn markers_are_added_renamed_moved_and_taken_away() {
        let mut p = sections_song();
        let s = &mut p.arrangement;
        let blocks = s.placements.clone();
        // A, B and C are taken: the next is D.
        assert_eq!(s.new_marker_name(), "D");
        assert_eq!(s.add_marker(6 * BAR, "D"), Ok(3));
        assert_eq!(s.add_marker(BAR, "Pickup"), Ok(1));
        assert_eq!(
            names(s),
            [
                (0, "A"),
                (BAR, "Pickup"),
                (2 * BAR, "B"),
                (4 * BAR, "C"),
                (6 * BAR, "D")
            ]
        );
        let before = s.clone();
        for refused in [
            s.add_marker(2 * BAR, "E"),
            s.add_marker(-1, "E"),
            s.add_marker(3 * BAR, " "),
        ] {
            assert_eq!(refused, Err(ModelError::BadPlacement));
        }
        assert_eq!(s.rename_marker(9, "X"), Err(ModelError::BadPlacement));
        assert_eq!(s.rename_marker(1, ""), Err(ModelError::BadPlacement));
        assert_eq!(s.move_marker(1, 2 * BAR), Err(ModelError::BadPlacement));
        assert_eq!(s.move_marker(1, -BAR), Err(ModelError::BadPlacement));
        assert_eq!(
            s.remove_marker(5).map(|m| m.at),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(*s, before);
        s.rename_marker(1, "Fill").unwrap();
        // Past B and C: it changes places with them, its name kept.
        assert_eq!(s.move_marker(1, 5 * BAR), Ok(3));
        assert_eq!(
            names(s),
            [
                (0, "A"),
                (2 * BAR, "B"),
                (4 * BAR, "C"),
                (5 * BAR, "Fill"),
                (6 * BAR, "D")
            ]
        );
        assert_eq!(
            s.section(2, BAR),
            Some((4 * BAR, 5 * BAR)),
            "C ends at Fill"
        );
        assert_eq!(s.remove_marker(3).unwrap().name, "Fill");
        assert_eq!(s.section(2, BAR), Some((4 * BAR, 6 * BAR)), "C runs to D");
        assert_eq!(s.placements, blocks, "only the markers changed");
        assert_eq!(s.check(&p.tracks), Ok(()));
    }

    /// Past Z, a new marker is Section N.
    #[test]
    fn a_new_marker_is_named_by_the_next_free_letter() {
        let mut s = Arrangement::default();
        assert_eq!(s.new_marker_name(), "A");
        for (i, c) in ('A'..='Z').enumerate() {
            if c != 'C' {
                s.mark(i as Tick * BAR, &c.to_string()).unwrap();
            }
        }
        assert_eq!(s.new_marker_name(), "C");
        s.mark(30 * BAR, "C").unwrap();
        assert_eq!(s.new_marker_name(), "Section 27");
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
            .move_section(&p.tracks, 1, 0, false, BAR, 120.0)
            .unwrap();
        assert_eq!(p.arrangement.list, list);
    }

    /// A section's tempo goes with it: the tempo in effect at its start
    /// and its points, and the place it left keeps the tempo before it.
    #[test]
    fn a_moved_section_takes_its_tempo_with_it() {
        let mut p = sections_song();
        p.arrangement.set_tempo(BAR, 100.0, None).unwrap();
        p.arrangement
            .set_tempo(4 * BAR + BAR / 4, 90.0, Some(true))
            .unwrap();
        p.arrangement.set_tempo(6 * BAR, 140.0, None).unwrap();
        // B (100 BPM throughout, from A's point) before A.
        p.arrangement
            .move_section(&p.tracks, 1, 0, false, BAR, 120.0)
            .unwrap();
        let tempo: Vec<_> = p
            .arrangement
            .tempo
            .iter()
            .map(|t| (t.at, t.bpm, t.ramp))
            .collect();
        // B at 100 from the start; A at the project's 120 until its point,
        // now a bar later; C's points where they were (C hasn't moved), C
        // starting at the 100 it started at.
        assert_eq!(
            tempo,
            [
                (0, 100.0, false),
                (2 * BAR, 120.0, false),
                (3 * BAR, 100.0, false),
                (4 * BAR + BAR / 4, 90.0, true),
                (6 * BAR, 140.0, false)
            ]
        );
    }

    /// The morph's moves and rows go with their section.
    #[test]
    fn a_moved_section_takes_its_morph_moves_with_it() {
        let mut p = sections_song();
        let rows = |at, a| MorphRows { at, a, b: 3 };
        p.arrangement
            .put_morph(
                &[
                    (0.0, 0.0, Kind::Step),
                    ((2 * BAR) as f64, 0.25, Kind::Ramp),
                    ((3 * BAR) as f64, 0.75, Kind::Step),
                    ((4 * BAR) as f64, 1.0, Kind::Step),
                ],
                &[rows(0, 0), rows(2 * BAR, 1), rows(4 * BAR, 2)],
                0,
                8 * BAR,
            )
            .unwrap();
        let before = p.arrangement.clone();
        // B before A.
        p.arrangement
            .move_section(&p.tracks, 1, 0, false, BAR, 120.0)
            .unwrap();
        let morph = |s: &Arrangement, t: Tick| {
            let v = s.lanes.lane(MORPH_LANE).and_then(|l| l.state_at(t as f64));
            (v, s.morph_rows_at(t).map(|m| (m.a, m.b)))
        };
        // B's ticks, now from 0, then A's, then C's where they were.
        for t in (0..8 * BAR).step_by(BAR as usize / 8) {
            let was = if t < 2 * BAR {
                t + 2 * BAR
            } else if t < 4 * BAR {
                t - 2 * BAR
            } else {
                t
            };
            let (now, then) = (morph(&p.arrangement, t), morph(&before, was));
            assert_eq!(now.1, then.1, "rows at tick {t}");
            let (a, b) = (now.0.unwrap(), then.0.unwrap());
            assert!((a - b).abs() < 1e-5, "tick {t}: {a} for {b}");
        }
        // Refused, nothing changes.
        let moved = p.arrangement.clone();
        assert_eq!(
            p.arrangement
                .move_section(&p.tracks, 5, 0, false, BAR, 120.0),
            Err(Refused::BadRange)
        );
        assert_eq!(p.arrangement, moved);
    }
}
