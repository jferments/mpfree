//! The play view: the project compiled for the audio thread.
//!
//! The control thread compiles a new view after every edit and swaps it in;
//! the audio thread only reads it. Recording is the exception: a recorded
//! or erased note or lock goes into the view in place
//! ([`PlayView::add_note`] and the others), where a compile would put it,
//! in room set aside by [`PlayView::make_room`], so recording neither
//! compiles nor allocates on every note. Compiling sorts each clip's notes,
//! applies quantise and speed, and resolves which swing each clip uses, so
//! playback is a binary search and a few multiplications per clip.
//!
//! **Cues** (roadmap T9). A launch made while the music plays waits for
//! the next bar line: the track's clip plays up to it, then the new clip
//! starts there from its first tick. A stop waits the same way. "Next"
//! means the next bar line the playhead plays, so in a loop it can be the
//! loop's start. The player switches on that exact tick and reports it in
//! [`crate::Player::started`]. The control thread then makes the new clip
//! the track's launch with [`Started::apply`] and compiles again. Until
//! that view arrives, the player and the recorder carry the switch
//! themselves, so nothing depends on when it arrives. A host that compiles
//! on another thread says which switches each view shows
//! ([`PlayView::set_includes`]), so a view a little late, or one that cues
//! the same clip again, is never taken for the switch.

use std::ops::Range;

use mpfree_param::RelAddr;
use mpfree_seq::arrange::SONG_LANE_LEN;
use mpfree_seq::mpfree_moves::{Kind, Map, Moves, Target};
use mpfree_seq::{
    touch_fits, Clip, DestId, Lock, ModelError, Note, Project, Quantise, Source, Speed, Touch,
    TouchPoint, MAX_LOCKS, MAX_NOTES, MAX_TRACKS, SLOTS,
};
use mpfree_transport::time::{ceil_to, Swing, Tick};

use crate::recorded::{Includes, Rec};

/// The slot number the song's own lanes go by, past any placement's.
pub(crate) const SONG_SLOT: usize = usize::MAX;

/// A clip playing on a track: which slot, and the song tick where its first
/// pass starts. Before `origin` the clip is silent; from it on, it loops.
/// A cue that starts makes one with an [`anchored`] origin, which plays
/// wherever the song goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Launch {
    pub track: usize,
    pub slot: usize,
    pub origin: Tick,
    /// The song tick the clip started on: its trig conditions count
    /// passes from here (pass 1 starts on it). The origin, unless the
    /// origin was [`anchored`] on a later bar line.
    pub start: Tick,
}

/// A launch waiting for the next bar line the playhead plays: the clip in
/// `slot` starts there in place of the track's clip, or with `None` the
/// track's clip stops there. See the module notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cue {
    pub track: usize,
    pub slot: Option<usize>,
}

/// A cue the player carried out: on song tick `at`, a bar line, `track`
/// switched to clip `slot`, or stopped. `origin` is the new clip's origin,
/// [`anchored`] on `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Started {
    pub track: usize,
    pub slot: Option<usize>,
    pub at: Tick,
    pub origin: Tick,
    /// The track's mute from `at` on, when the cue switched it (a
    /// scene's mutes): the control thread sets `Track::mute` to it.
    pub mute: Option<bool>,
}

impl Started {
    /// The control thread's part: the started clip becomes the track's
    /// launch (after a stop the track has none), and the cue goes.
    pub fn apply(&self, launches: &mut Vec<Launch>, cues: &mut Vec<Cue>) {
        launches.retain(|l| l.track != self.track);
        if let Some(slot) = self.slot {
            launches.push(Launch {
                track: self.track,
                slot,
                origin: self.origin,
                start: self.at,
            });
        }
        if let Some(i) = cues
            .iter()
            .position(|c| c.track == self.track && c.slot == self.slot)
        {
            cues.remove(i);
        }
    }
}

/// The earliest song tick an [`anchored`] clip plays from: about 12 days
/// before the start at 120 BPM, so in practice wherever the song goes.
pub const EARLIEST: Tick = -(1 << 31);

/// The origin of a clip that starts on song tick `at` and then plays
/// wherever the song goes, back across loop jumps and locates too: `at`
/// moved back by whole passes to [`EARLIEST`] or before. The clip's tick 0
/// still falls on `at`. `num` passes of a clip are exactly `length * den`
/// song ticks, so the origin moves back by multiples of that and stays a
/// whole tick.
pub fn anchored(at: Tick, length: Tick, speed: Speed) -> Tick {
    let period = length.max(1) * Tick::from(speed.den.max(1));
    let back = (at - EARLIEST).max(0);
    at - (back + period - 1) / period * period
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    Model(ModelError),
    NoTrack(usize),
    EmptySlot {
        track: usize,
        slot: usize,
    },
    /// One clip per track plays at a time.
    TwoClips(usize),
    /// One cue per track waits at a time.
    TwoCues(usize),
}

/// A note ready to play: where it starts in the clip (after quantise) and
/// how long it lasts in song ticks (after speed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NoteView {
    pub local: f64,
    pub len: f64,
    pub note: u8,
    pub velocity: u8,
    /// The clip's note as stored, to name it when replace recording erases it.
    pub src: Note,
}

/// A parameter lock ready to play: where it sits in the clip (after
/// quantise, like its note).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LockView {
    pub local: f64,
    pub addr: RelAddr,
    pub value: f32,
    /// As the clip stores it, to name it when replace recording erases it.
    pub src: Lock,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClipView {
    pub slot: usize,
    pub origin: Tick,
    /// The song tick the clip started on: its pass 1 (see [`Launch`]).
    pub start: Tick,
    /// One pass, in the clip's own ticks.
    pub length: Tick,
    pub speed: Speed,
    pub quantise: Option<Quantise>,
    /// One pass, in song ticks.
    pub song_len: f64,
    /// The clip's own swing, else the track's; `None` follows the transport.
    pub swing: Option<Swing>,
    /// Sorted by `local`.
    pub notes: Vec<NoteView>,
    /// Sorted by `local`; equal ones in the order the clip holds them.
    pub locks: Vec<LockView>,
    /// The clip's moves, on its own ticks (not quantised or swung).
    pub moves: Moves,
    /// The clip's pad pressure, as it holds it ([`Clip::touch`]): each
    /// note's [`Touch`] names its run here.
    pub touch: Vec<TouchPoint>,
}

impl ClipView {
    /// A note of the clip, compiled: quantise and speed applied.
    fn note_view(&self, n: &Note) -> NoteView {
        let at = self
            .quantise
            .map_or(n.tick as f64, |q| q.apply(n.tick as f64));
        NoteView {
            // Quantise can move a note at the very end onto the next pass's start.
            local: at.rem_euclid(self.length as f64),
            len: self.speed.to_song(n.len as f64),
            note: n.note,
            velocity: n.velocity,
            src: *n,
        }
    }

    /// A lock of the clip, compiled: quantise applied, like its note.
    fn lock_view(&self, l: &Lock) -> LockView {
        let at = self
            .quantise
            .map_or(l.tick as f64, |q| q.apply(l.tick as f64));
        LockView {
            local: at.rem_euclid(self.length as f64),
            addr: l.addr,
            value: l.value,
            src: *l,
        }
    }

    /// Put `n`, the clip's newest note, where a compile would: after the
    /// notes at or before its place, which come earlier in the clip. False
    /// if there is no room left (that would allocate).
    fn add_note(&mut self, n: &Note) -> bool {
        if self.notes.len() == self.notes.capacity() {
            return false;
        }
        let v = self.note_view(n);
        let i = self.notes.partition_point(|x| x.local <= v.local);
        self.notes.insert(i, v);
        true
    }

    /// Take out a note equal to `n`. False if there is none.
    fn remove_note(&mut self, n: &Note) -> bool {
        match self.notes.iter().position(|x| x.src == *n) {
            Some(i) => {
                self.notes.remove(i);
                true
            }
            None => false,
        }
    }

    /// Put lock `l` where a compile would: in the place of `replaced` (the
    /// clip put it in that lock's place), else after the locks at or before
    /// its place, as the clip's newest. False if `replaced` is not there,
    /// or there is no room left.
    fn add_lock(&mut self, l: &Lock, replaced: Option<&Lock>) -> bool {
        let v = self.lock_view(l);
        if let Some(old) = replaced {
            return match self.locks.iter().position(|x| x.src == *old) {
                // Same tick, so the same place.
                Some(i) if self.locks[i].local == v.local => {
                    self.locks[i] = v;
                    true
                }
                _ => false,
            };
        }
        if self.locks.len() == self.locks.capacity() {
            return false;
        }
        let i = self.locks.partition_point(|x| x.local <= v.local);
        self.locks.insert(i, v);
        true
    }

    /// Take out a lock equal to `l`. False if there is none.
    fn remove_lock(&mut self, l: &Lock) -> bool {
        match self.locks.iter().position(|x| x.src == *l) {
            Some(i) => {
                self.locks.remove(i);
                true
            }
            None => false,
        }
    }

    fn make_room(&mut self, notes: usize, locks: usize) {
        self.notes.reserve(notes);
        self.locks.reserve(locks);
        self.touch.reserve(crate::record::TOUCH_ROOM);
        // Each lane the clip has gets room for two recorded parts (a new
        // lane still needs a compile).
        let lanes: Vec<_> = self
            .moves
            .lanes()
            .iter()
            .map(|l| (l.target, l.map))
            .collect();
        for (target, map) in lanes {
            let _ = self.moves.reserve(target, map, crate::record::MOVE_ROOM);
        }
    }

    /// A note's pressure run added to the clip at `touch` (as
    /// `Clip::add_touch` put it there): added here in the same place.
    /// False when this view's pressure is not where the clip's was, or
    /// there is no room left.
    fn add_touch(&mut self, touch: Touch, points: &[TouchPoint]) -> bool {
        if self.touch.len() != touch.first as usize
            || points.len() != touch.len as usize
            || self.touch.capacity() - self.touch.len() < points.len()
        {
            return false;
        }
        self.touch.extend_from_slice(points);
        true
    }

    /// A recorded note with its pressure `points`, decided here as the
    /// clip decides it (`Takes::add_touched`): the pressure goes on the
    /// end of the clip's when it fits (else the note has none), then the
    /// note goes in if the clip can hold it. False only when there is no
    /// room in place.
    fn record_note(&mut self, mut n: Note, points: &[TouchPoint]) -> bool {
        n.touch = Touch::NONE;
        if !points.is_empty() && touch_fits(self.touch.len(), points) {
            let touch = Touch {
                first: self.touch.len() as u32,
                len: points.len() as u32,
            };
            if !self.add_touch(touch, points) {
                return false;
            }
            n.touch = touch;
        }
        if self.notes.len() >= MAX_NOTES || !n.fits(self.length, &self.touch) {
            return true;
        }
        self.add_note(&n)
    }

    /// A recorded lock, decided here as the clip decides it
    /// (`Takes::add_lock`): in the place of the lock in its place, else
    /// as the newest, when the clip can hold it. False only when there is
    /// no room in place.
    fn record_lock(&mut self, l: &Lock) -> bool {
        if !l.fits(self.length) {
            return true;
        }
        let old = self
            .locks
            .iter()
            .find(|x| x.src.same_place(l))
            .map(|x| x.src);
        if old.is_none() && self.locks.len() >= MAX_LOCKS {
            return true;
        }
        self.add_lock(l, old.as_ref())
    }

    /// A recorded gesture written into this clip's moves in place, as
    /// `Takes::put_move` writes it into the clip. False when its lane is
    /// new or has no room left.
    fn put_move(
        &mut self,
        target: Target,
        map: Map,
        points: &[(f64, f32, Kind)],
        len: u32,
    ) -> bool {
        len == self.length as u32
            && self.moves.fits(target, points.len())
            && self.moves.put(target, map, points, len).is_ok()
    }
}

/// A waiting cue, compiled: the clip it starts (its origin comes when it
/// starts), or none for a stop.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CueView {
    pub slot: Option<usize>,
    pub clip: Option<ClipView>,
    /// The track's mute from the cue's bar line on (a scene's mutes).
    pub mute: Option<bool>,
}

/// A placement of the song, compiled: its clip (from `clip.start`, with
/// its origin) until `end`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegionView {
    pub end: Tick,
    pub clip: ClipView,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrackView {
    pub dest: DestId,
    pub mute: bool,
    pub clip: Option<ClipView>,
    pub cue: Option<CueView>,
    /// The track's placements, in order, in song mode (else none).
    pub regions: Vec<RegionView>,
}

impl TrackView {
    /// The region playing at `tick`.
    pub fn region_at(&self, tick: Tick) -> Option<&RegionView> {
        let k = self.regions.partition_point(|r| r.end <= tick);
        self.regions.get(k).filter(|r| r.clip.start <= tick)
    }
}

/// The project compiled for playback. Build it on the control thread; the
/// audio thread only reads it. Two views are equal when they play the
/// same: what each says it includes is left out.
#[derive(Clone, Debug, Default)]
pub struct PlayView {
    pub(crate) tracks: Vec<TrackView>,
    pub(crate) armed: Option<usize>,
    /// Song mode: a track with no clip launched plays its placements.
    pub(crate) song: bool,
    /// The song's own lanes (the morph), played in song mode.
    pub(crate) song_lanes: Option<ClipView>,
    /// What it includes, when its host says ([`PlayView::set_includes`]).
    pub(crate) includes: Option<Includes>,
}

impl PartialEq for PlayView {
    fn eq(&self, other: &Self) -> bool {
        (&self.tracks, self.armed, self.song, &self.song_lanes)
            == (&other.tracks, other.armed, other.song, &other.song_lanes)
    }
}

impl PlayView {
    /// Nothing plays.
    pub fn empty() -> PlayView {
        PlayView::default()
    }

    /// Compile `project` with the clips in `launches` playing.
    pub fn compile(project: &Project, launches: &[Launch]) -> Result<PlayView, CompileError> {
        PlayView::compile_cued(project, launches, &[])
    }

    /// Compile `project` with the clips in `launches` playing and the
    /// `cues` waiting for the next bar line. It checks the tracks and the
    /// clips it compiles, not the whole project: an edit's compile costs
    /// what plays, and a clip out of range is refused when it is launched
    /// or cued (`CompileError::Model`).
    pub fn compile_cued(
        project: &Project,
        launches: &[Launch],
        cues: &[Cue],
    ) -> Result<PlayView, CompileError> {
        project.check_tracks().map_err(CompileError::Model)?;
        let mut tracks: Vec<TrackView> = project
            .tracks
            .iter()
            .map(|t| TrackView {
                dest: t.dest,
                mute: t.mute,
                clip: None,
                cue: None,
                regions: Vec::new(),
            })
            .collect();
        for l in launches {
            let clip = clip_view(project, l.track, l.slot, l.origin, l.start)?;
            if tracks[l.track].clip.is_some() {
                return Err(CompileError::TwoClips(l.track));
            }
            tracks[l.track].clip = Some(clip);
        }
        for c in cues {
            let track = tracks.get(c.track).ok_or(CompileError::NoTrack(c.track))?;
            if track.cue.is_some() {
                return Err(CompileError::TwoCues(c.track));
            }
            let clip = c
                .slot
                .map(|slot| clip_view(project, c.track, slot, 0, 0))
                .transpose()?;
            tracks[c.track].cue = Some(CueView {
                slot: c.slot,
                clip,
                mute: None,
            });
        }
        Ok(PlayView {
            tracks,
            armed: None,
            song: false,
            song_lanes: None,
            includes: None,
        })
    }

    /// Song mode on (the arrangement's placements compiled in: a track
    /// with no clip launched plays them on their ticks, and the song's own
    /// lanes play as [`crate::SONG_TRACK`]'s) or off. Call it on the
    /// control thread after compiling, with the same project. With song
    /// mode on it checks the arrangement and each clip it places, once.
    pub fn set_song(&mut self, project: &Project, on: bool) -> Result<(), CompileError> {
        for t in &mut self.tracks {
            t.regions.clear();
        }
        self.song = on;
        self.song_lanes = None;
        if !on {
            return Ok(());
        }
        project
            .arrangement
            .check(&project.tracks)
            .map_err(CompileError::Model)?;
        // The slots placed (a placement's own clip is checked with the
        // arrangement), each checked once however often it is placed.
        let mut seen = [[false; SLOTS]; MAX_TRACKS];
        for p in &project.arrangement.placements {
            if let (Source::Slot(s), Some(clip)) = (&p.source, p.clip(&project.tracks)) {
                if let Some(done) = seen.get_mut(p.track).and_then(|t| t.get_mut(*s)) {
                    if !*done {
                        clip.check().map_err(CompileError::Model)?;
                        *done = true;
                    }
                }
            }
        }
        let lanes = &project.arrangement.lanes;
        if !lanes.is_empty() {
            // One long pass from tick 0 that never loops.
            let len = Tick::from(SONG_LANE_LEN);
            self.song_lanes = Some(ClipView {
                slot: SONG_SLOT,
                origin: 0,
                start: 0,
                length: len,
                speed: Speed::NORMAL,
                quantise: None,
                song_len: len as f64,
                swing: None,
                notes: Vec::new(),
                locks: Vec::new(),
                moves: lanes.clone(),
                touch: Vec::new(),
            });
        }
        for (i, p) in project.arrangement.placements.iter().enumerate() {
            let track = project
                .tracks
                .get(p.track)
                .ok_or(CompileError::NoTrack(p.track))?;
            // Linked to a slot emptied since: silent until it has a clip.
            let Some(clip) = p.clip(&project.tracks) else {
                continue;
            };
            // A placement's own clip is named by a number past the slots,
            // for the player's keys.
            let slot = match p.source {
                Source::Slot(s) => s,
                Source::Own(_) => SLOTS + i,
            };
            let view = view_of(clip, track.swing, slot, p.origin(clip), p.start);
            if let Some(t) = self.tracks.get_mut(p.track) {
                t.regions.push(RegionView {
                    end: p.end(),
                    clip: view,
                });
            }
        }
        Ok(())
    }

    /// Whether song mode is on.
    pub fn song(&self) -> bool {
        self.song
    }

    /// Arm `track` for recording, or none. The recorder records into its
    /// playing clip, and replace recording erases from it. Set it before
    /// swapping the view in.
    pub fn set_armed(&mut self, track: Option<usize>) {
        self.armed = track;
    }

    pub fn armed(&self) -> Option<usize> {
        self.armed
    }

    /// Say what this view includes: the recorded changes made to the
    /// project it was compiled from, and the cues started on each track
    /// that it shows as launches (see [`crate::recorded`]). Call it on the
    /// control thread before handing the view over. A view with this set
    /// stops a switch the player carries once it shows as many switches on
    /// that track as the player has made.
    pub fn set_includes(&mut self, includes: Includes) {
        self.includes = Some(includes);
    }

    /// What it includes, if its host said.
    pub fn includes(&self) -> Option<Includes> {
        self.includes
    }

    /// Make recorded change `rec` to this view in place, deciding as the
    /// project decides it (`Takes::record`), with a note's pressure
    /// `touch` or a gesture's `moves`. Real-time safe. False when the view
    /// has no room for it in place; then compile instead. A change to a
    /// clip the view does not show, or an erase of a note or lock it no
    /// longer holds (an edit took it out), needs nothing.
    pub fn record(&mut self, rec: &Rec, touch: &[TouchPoint], moves: &[(f64, f32, Kind)]) -> bool {
        let (track, slot) = rec.clip();
        let mut shown = self.shown(track, slot).into_iter().flatten();
        match rec {
            Rec::Note(k) => shown.all(|c| c.record_note(k.note, touch)),
            Rec::Lock(k) => shown.all(|c| c.record_lock(&k.lock)),
            Rec::Erase(e) => {
                for c in shown {
                    c.remove_note(&e.note);
                }
                true
            }
            Rec::EraseLock(e) => {
                for c in shown {
                    c.remove_lock(&e.lock);
                }
                true
            }
            Rec::Move(k) => shown.all(|c| c.put_move(k.target, k.map, moves, k.len)),
        }
    }

    pub fn tracks(&self) -> usize {
        self.tracks.len()
    }

    /// Notes in the playing clips.
    pub fn notes(&self) -> usize {
        self.tracks
            .iter()
            .filter_map(|t| t.clip.as_ref())
            .map(|c| c.notes.len())
            .sum()
    }

    /// Parameter locks in the playing clips.
    pub fn locks(&self) -> usize {
        self.tracks
            .iter()
            .filter_map(|t| t.clip.as_ref())
            .map(|c| c.locks.len())
            .sum()
    }

    /// Room for `notes` more notes and `locks` more locks in the armed
    /// track's clips (the one playing and the one cued), so that recording
    /// can put them in with [`PlayView::add_note`] and
    /// [`PlayView::add_lock`] without allocating, and room in each of
    /// their lanes for [`crate::record::MOVE_ROOM`] more points, for
    /// [`PlayView::put_move`]. Call it on the control thread, after
    /// compiling.
    pub fn make_room(&mut self, notes: usize, locks: usize) {
        let Some(t) = self.armed.and_then(|t| self.tracks.get_mut(t)) else {
            return;
        };
        if let Some(c) = t.clip.as_mut() {
            c.make_room(notes, locks);
        }
        if let Some(c) = t.cue.as_mut().and_then(|c| c.clip.as_mut()) {
            c.make_room(notes, locks);
        }
    }

    /// The compiled clips that show clip `slot` of `track`: the one
    /// playing and the one cued. When an edit cannot go into one of them
    /// in place, the host compiles, so the other need not be edited.
    fn shown(&mut self, track: usize, slot: usize) -> [Option<&mut ClipView>; 2] {
        let Some(t) = self.tracks.get_mut(track) else {
            return [None, None];
        };
        let playing = t.clip.as_mut().filter(|c| c.slot == slot);
        let cued = t
            .cue
            .as_mut()
            .filter(|c| c.slot == Some(slot))
            .and_then(|c| c.clip.as_mut());
        [playing, cued]
    }

    /// Note `n` was added to clip `slot` of `track`, at the end of its
    /// notes (as [`mpfree_seq::Clip::add_note`] and [`crate::Takes::add`]
    /// do): put it in this view where compiling the project again would,
    /// without allocating. Real-time safe. False when the view has no
    /// room for it ([`PlayView::make_room`]); then compile instead. A clip
    /// the view does not show needs nothing.
    pub fn add_note(&mut self, track: usize, slot: usize, n: &Note) -> bool {
        self.shown(track, slot)
            .into_iter()
            .flatten()
            .all(|c| c.add_note(n))
    }

    /// A note's pressure run went into clip `slot` of `track` at `touch`
    /// (as `Takes::add_touched` puts it, before the note): put it in this
    /// view in the same place, before [`PlayView::add_note`] adds the note.
    /// Real-time safe. False when there is no room, or this view's
    /// pressure is not where the clip's was; then compile instead.
    pub fn add_touch(
        &mut self,
        track: usize,
        slot: usize,
        touch: Touch,
        points: &[TouchPoint],
    ) -> bool {
        touch.is_none()
            || self
                .shown(track, slot)
                .into_iter()
                .flatten()
                .all(|c| c.add_touch(touch, points))
    }

    /// The pad pressure of clip `slot` of `track` as compiled: the clip
    /// playing, the one cued, or a placement of it in the song (a
    /// placement's own clip goes by its number past the slots).
    pub(crate) fn touch_of(&self, track: usize, slot: usize) -> &[TouchPoint] {
        let Some(t) = self.tracks.get(track) else {
            return &[];
        };
        let cued = t.cue.as_ref().and_then(|c| c.clip.as_ref());
        t.clip
            .iter()
            .chain(cued)
            .chain(t.regions.iter().map(|r| &r.clip))
            .find(|c| c.slot == slot)
            .map_or(&[], |c| c.touch.as_slice())
    }

    /// A note equal to `n` was taken out of clip `slot` of `track` (as
    /// [`crate::Takes::erase`] does): take it out of this view too.
    /// Real-time safe. False when the view does not hold it; then compile
    /// instead.
    pub fn remove_note(&mut self, track: usize, slot: usize, n: &Note) -> bool {
        self.shown(track, slot)
            .into_iter()
            .flatten()
            .all(|c| c.remove_note(n))
    }

    /// Lock `l` was put in clip `slot` of `track` (as
    /// [`crate::Takes::add_lock`] does): in the place of `replaced`, the
    /// lock it returned, or else at the end of the clip's locks. Put it in
    /// this view where compiling again would. Real-time safe. False when
    /// there is no room, or `replaced` is not in the view; then compile
    /// instead.
    pub fn add_lock(
        &mut self,
        track: usize,
        slot: usize,
        l: &Lock,
        replaced: Option<&Lock>,
    ) -> bool {
        self.shown(track, slot)
            .into_iter()
            .flatten()
            .all(|c| c.add_lock(l, replaced))
    }

    /// A recorded gesture was written into clip `slot` of `track` (as
    /// [`crate::Takes::put_move`] does): write it into this view the same
    /// way. Real-time safe. False when the lane is new or out of the room
    /// [`PlayView::make_room`] set aside; then compile instead.
    pub fn put_move(
        &mut self,
        track: usize,
        slot: usize,
        target: Target,
        map: Map,
        points: &[(f64, f32, Kind)],
        len: u32,
    ) -> bool {
        self.shown(track, slot)
            .into_iter()
            .flatten()
            .all(|c| c.put_move(target, map, points, len))
    }

    /// A lock equal to `l` was taken out of clip `slot` of `track` (as
    /// [`crate::Takes::erase_lock`] does): take it out of this view too.
    /// Real-time safe. False when the view does not hold it.
    pub fn remove_lock(&mut self, track: usize, slot: usize, l: &Lock) -> bool {
        self.shown(track, slot)
            .into_iter()
            .flatten()
            .all(|c| c.remove_lock(l))
    }

    /// Track `track`'s cue switches its mute to `mute` when it starts (a
    /// scene's mutes), or leaves it (`None`). False when the track has no
    /// cue. Call it on the control thread, after compiling.
    pub fn set_cue_mute(&mut self, track: usize, mute: Option<bool>) -> bool {
        match self.tracks.get_mut(track).and_then(|t| t.cue.as_mut()) {
            Some(c) => {
                c.mute = mute;
                true
            }
            None => false,
        }
    }

    /// The cue waiting on `track`: `Some(Some(slot))` for a launch,
    /// `Some(None)` for a stop.
    pub fn cue(&self, track: usize) -> Option<Option<usize>> {
        self.tracks.get(track)?.cue.as_ref().map(|c| c.slot)
    }
}

/// One clip compiled: notes sorted, quantise and speed applied.
fn clip_view(
    project: &Project,
    track: usize,
    slot: usize,
    origin: Tick,
    start: Tick,
) -> Result<ClipView, CompileError> {
    let t = project
        .tracks
        .get(track)
        .ok_or(CompileError::NoTrack(track))?;
    let clip = t
        .slots
        .get(slot)
        .and_then(Option::as_ref)
        .filter(|_| slot < SLOTS)
        .ok_or(CompileError::EmptySlot { track, slot })?;
    clip.check().map_err(CompileError::Model)?;
    Ok(view_of(clip, t.swing, slot, origin, start))
}

/// `clip` compiled, named `slot`, on a track whose swing is `track_swing`.
fn view_of(
    clip: &Clip,
    track_swing: Option<Swing>,
    slot: usize,
    origin: Tick,
    start: Tick,
) -> ClipView {
    let mut view = ClipView {
        slot,
        origin,
        start,
        length: clip.length,
        speed: clip.speed,
        quantise: clip.quantise,
        song_len: clip.speed.to_song(clip.length as f64),
        swing: clip.swing.or(track_swing),
        notes: Vec::new(),
        locks: Vec::new(),
        moves: clip.moves.clone(),
        touch: clip.touch.clone(),
    };
    view.notes = clip.notes.iter().map(|n| view.note_view(n)).collect();
    view.notes.sort_by(|a, b| a.local.total_cmp(&b.local));
    view.locks = clip.locks.iter().map(|l| view.lock_view(l)).collect();
    view.locks.sort_by(|a, b| a.local.total_cmp(&b.local));
    view
}

/// A cue carried out on the audio thread that the view does not show yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Switch {
    slot: Option<usize>,
    origin: Tick,
    /// The tick it started on.
    start: Tick,
    /// The track's mute from then on, when the cue switches it.
    mute: Option<bool>,
}

/// The cues carried out on each track until the view shows them. The
/// player and the recorder each keep one and step it on the same spans, so
/// they agree on which clip plays, for any period size and whenever the
/// new view arrives.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Switches {
    on: [Option<Switch>; MAX_TRACKS],
    /// Switches made on each track so far, for a view that says how many
    /// it shows ([`Includes::switches`]).
    made: [u32; MAX_TRACKS],
}

impl Switches {
    pub fn new() -> Switches {
        Switches {
            on: [None; MAX_TRACKS],
            made: [0; MAX_TRACKS],
        }
    }

    /// Forget the switches the view has caught up with. A view that says
    /// what it includes has caught up once it shows as many switches on
    /// the track as were made; any other once its cue is gone (the control
    /// thread applied it, or took it back).
    pub fn follow(&mut self, view: &PlayView) {
        for (i, s) in self.on.iter_mut().enumerate() {
            let Some(sw) = *s else {
                continue;
            };
            let shown = match view.includes {
                Some(inc) => inc.switches[i] >= self.made[i],
                None => view
                    .tracks
                    .get(i)
                    .and_then(|t| t.cue.as_ref())
                    .is_none_or(|c| c.slot != sw.slot),
            };
            if shown {
                *s = None;
            }
        }
    }

    /// Where the cue on `track` starts in a span playing `ticks`, if it
    /// does: the first bar line among them.
    pub fn due(
        &self,
        view: &PlayView,
        track: usize,
        ticks: &Range<Tick>,
        bar: Tick,
    ) -> Option<Tick> {
        if self.on.get(track).is_none_or(Option::is_some) {
            return None;
        }
        view.tracks.get(track)?.cue.as_ref()?;
        let at = ceil_to(ticks.start, bar);
        (at < ticks.end).then_some(at)
    }

    /// Carry out the cue on `track` at song tick `at`.
    pub fn start(&mut self, view: &PlayView, track: usize, at: Tick) -> Option<Started> {
        let cue = view.tracks.get(track)?.cue.as_ref()?;
        let origin = cue
            .clip
            .as_ref()
            .map_or(at, |c| anchored(at, c.length, c.speed));
        *self.on.get_mut(track)? = Some(Switch {
            slot: cue.slot,
            origin,
            start: at,
            mute: cue.mute,
        });
        self.made[track] = self.made[track].wrapping_add(1);
        Some(Started {
            track,
            slot: cue.slot,
            at,
            origin,
            mute: cue.mute,
        })
    }

    /// Whether `track` is muted now: as its cue set it when that started,
    /// else as the view has it.
    pub fn muted(&self, view: &PlayView, track: usize) -> bool {
        match self.on.get(track).copied().flatten().and_then(|s| s.mute) {
            Some(m) => m,
            None => view.tracks.get(track).is_some_and(|t| t.mute),
        }
    }

    /// The clip `track` plays now, and its origin.
    pub fn playing<'a>(&self, view: &'a PlayView, track: usize) -> Option<(&'a ClipView, Tick)> {
        self.playing_from(view, track)
            .map(|(c, origin, _)| (c, origin))
    }

    /// The clip `track` plays now, its origin, and the tick it started on.
    pub fn playing_from<'a>(
        &self,
        view: &'a PlayView,
        track: usize,
    ) -> Option<(&'a ClipView, Tick, Tick)> {
        let t = view.tracks.get(track)?;
        match self.on.get(track).copied().flatten() {
            Some(sw) => t
                .cue
                .as_ref()?
                .clip
                .as_ref()
                .map(|c| (c, sw.origin, sw.start)),
            None => t.clip.as_ref().map(|c| (c, c.origin, c.start)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Note, Quantise, Track};

    fn project() -> Project {
        let mut p = Project::new();
        let mut t = Track::new("drums", 3);
        let mut c = Clip::new("beat", 960);
        c.notes = vec![
            Note {
                tick: 500,
                len: 10,
                note: 1,
                velocity: 90,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
            },
            Note {
                tick: 0,
                len: 10,
                note: 0,
                velocity: 100,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
            },
            Note {
                tick: 950,
                len: 10,
                note: 2,
                velocity: 80,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
            },
        ];
        c.quantise = Quantise::new(240, 100);
        c.speed = Speed::new(2, 1).unwrap();
        t.slots[2] = Some(c);
        p.tracks.push(t);
        p.tracks.push(Track::new("empty", 4));
        p
    }

    #[test]
    fn compiling_sorts_quantises_and_scales() {
        let v = PlayView::compile(
            &project(),
            &[Launch {
                track: 0,
                slot: 2,
                origin: 3840,
                start: 3840,
            }],
        )
        .unwrap();
        assert_eq!(v.tracks(), 2);
        assert_eq!(v.notes(), 3);
        let c = v.tracks[0].clip.as_ref().unwrap();
        // 950 quantises to 960, the clip's end, which is the next pass's 0.
        let at: Vec<_> = c.notes.iter().map(|n| (n.local, n.note)).collect();
        assert_eq!(at, vec![(0.0, 0), (0.0, 2), (480.0, 1)]);
        assert_eq!((c.song_len, c.notes[0].len, c.origin), (480.0, 5.0, 3840));
        assert_eq!(v.tracks[0].dest, 3);
        assert!(v.tracks[1].clip.is_none());
    }

    #[test]
    fn launches_must_name_a_clip() {
        let p = project();
        let l = |track, slot| Launch {
            track,
            slot,
            origin: 0,
            start: 0,
        };
        assert_eq!(
            PlayView::compile(&p, &[l(5, 0)]),
            Err(CompileError::NoTrack(5))
        );
        assert_eq!(
            PlayView::compile(&p, &[l(0, 1)]),
            Err(CompileError::EmptySlot { track: 0, slot: 1 })
        );
        assert_eq!(
            PlayView::compile(&p, &[l(0, 99)]),
            Err(CompileError::EmptySlot { track: 0, slot: 99 })
        );
        assert_eq!(
            PlayView::compile(&p, &[l(0, 2), l(0, 2)]),
            Err(CompileError::TwoClips(0))
        );
        assert_eq!(PlayView::compile(&p, &[]).unwrap().notes(), 0);
    }

    #[test]
    fn a_compile_checks_what_it_plays_not_the_whole_project() {
        let mut p = project();
        let l = |track, slot| Launch {
            track,
            slot,
            origin: 0,
            start: 0,
        };
        // A clip out of range in a slot nothing plays: the rest compiles.
        let mut bad = Clip::new("bad", 3840);
        bad.notes.push(Note::new(5000, 10, 1, 100));
        p.tracks[1].slots[3] = Some(bad);
        assert!(PlayView::compile(&p, &[l(0, 2)]).is_ok());
        // Launched or cued, it is refused.
        assert_eq!(
            PlayView::compile(&p, &[l(1, 3)]),
            Err(CompileError::Model(ModelError::BadNote(0)))
        );
        assert_eq!(
            PlayView::compile_cued(
                &p,
                &[],
                &[Cue {
                    track: 1,
                    slot: Some(3)
                }]
            ),
            Err(CompileError::Model(ModelError::BadNote(0)))
        );
        // Placed in the song, it is refused once song mode is on.
        let mut v = PlayView::compile(&p, &[]).unwrap();
        p.arrangement
            .placements
            .push(mpfree_seq::Placement::linked(1, 3, 0, 3840));
        assert!(v.set_song(&p, false).is_ok());
        assert_eq!(
            v.set_song(&p, true),
            Err(CompileError::Model(ModelError::BadNote(0)))
        );
        // The tracks themselves are still checked.
        p.tracks[0].swing = Some(Swing {
            grid: 0,
            amount: 50,
        });
        assert_eq!(
            PlayView::compile(&p, &[]),
            Err(CompileError::Model(ModelError::BadSwing))
        );
    }

    #[test]
    fn cues_must_name_a_clip_or_a_stop() {
        let p = project();
        let cue = |track, slot| Cue { track, slot };
        let ok = PlayView::compile_cued(&p, &[], &[cue(0, Some(2)), cue(1, None)]).unwrap();
        assert_eq!(
            (ok.cue(0), ok.cue(1), ok.cue(7)),
            (Some(Some(2)), Some(None), None)
        );
        assert_eq!(ok.notes(), 0, "a cue does not play yet");
        assert_eq!(
            PlayView::compile_cued(&p, &[], &[cue(4, None)]),
            Err(CompileError::NoTrack(4))
        );
        assert_eq!(
            PlayView::compile_cued(&p, &[], &[cue(1, Some(2))]),
            Err(CompileError::EmptySlot { track: 1, slot: 2 })
        );
        assert_eq!(
            PlayView::compile_cued(&p, &[], &[cue(0, None), cue(0, Some(2))]),
            Err(CompileError::TwoCues(0))
        );
    }

    #[test]
    fn an_anchored_origin_keeps_the_phase_and_lies_before_the_earliest_tick() {
        for (at, length, speed) in [
            (3840, 7680, Speed::NORMAL),
            (0, 3840, Speed::NORMAL),
            (11_520, 1000, Speed::new(3, 2).unwrap()),
            (-3840, 960, Speed::new(1, 8).unwrap()),
            (EARLIEST - 5, 960, Speed::NORMAL),
        ] {
            let o = anchored(at, length, speed);
            assert!(o <= EARLIEST, "{at} {length} {speed:?}");
            let song_len = speed.to_song(length as f64);
            let passes = (at - o) as f64 / song_len;
            assert_eq!(passes, passes.round(), "{at}: whole passes back");
            assert!(
                at - o < (at - EARLIEST).max(0) + length * Tick::from(speed.den),
                "{at}: the least whole passes"
            );
        }
    }

    #[test]
    fn a_started_cue_becomes_the_launch() {
        let mut launches = vec![
            Launch {
                track: 0,
                slot: 1,
                origin: 0,
                start: 0,
            },
            Launch {
                track: 1,
                slot: 0,
                origin: 0,
                start: 0,
            },
        ];
        let mut cues = vec![
            Cue {
                track: 0,
                slot: Some(2),
            },
            Cue {
                track: 1,
                slot: None,
            },
        ];
        Started {
            track: 0,
            slot: Some(2),
            at: 7680,
            origin: -100,
            mute: None,
        }
        .apply(&mut launches, &mut cues);
        assert_eq!(
            launches,
            vec![
                Launch {
                    track: 1,
                    slot: 0,
                    origin: 0,
                    start: 0,
                },
                Launch {
                    track: 0,
                    slot: 2,
                    origin: -100,
                    start: 7680,
                }
            ]
        );
        Started {
            track: 1,
            slot: None,
            at: 7680,
            origin: 7680,
            mute: None,
        }
        .apply(&mut launches, &mut cues);
        assert_eq!(
            launches,
            vec![Launch {
                track: 0,
                slot: 2,
                origin: -100,
                start: 7680,
            }]
        );
        assert!(cues.is_empty());
    }
}
