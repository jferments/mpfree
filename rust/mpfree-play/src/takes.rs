//! Take undo, on the control thread: every note a take recorded or erased,
//! kept by take, so the last take can be undone and redone as a whole.
//!
//! The control thread hands each period's [`Player::erased`] and
//! [`Recorder::taken`] notes to [`Takes::erase`] and [`Takes::add`] (and
//! the locks, [`Player::erased_locks`] and [`Recorder::taken_locks`], to
//! [`Takes::erase_lock`] and [`Takes::add_lock`]), which
//! change the clips and remember what changed. [`Takes::undo`] puts the
//! erased notes back and takes the recorded ones out; [`Takes::redo`] does
//! the take again. Notes are named by value, as the clips store them, so
//! edits to other notes in between do not disturb them.
//!
//! A take's notes can still arrive after the next take has begun: a note
//! that quantise moves past a punch out waits for the playhead. Each note
//! carries its take's number, so it joins the right take. A take that is
//! undone while it is still recording goes on as a new take, and redo can
//! no longer bring back the undone part.
//!
//! CAPTURE keeps what was just played as a take of its own
//! ([`Takes::capture_clip`] for a new clip, [`Takes::capture_notes`] for
//! notes added to a clip that was playing), so UNDO takes a capture back
//! like any take. Captures are numbered on their own, from 1.
//!
//! An edit of a clip's notes, such as a step tapped on or a step's velocity
//! changed ([`Takes::edit`] with an `mpfree_seq::steps::Edit`), is a take
//! too, numbered on its own. Turning a Q-Link through many values can join
//! one edit, so one UNDO takes the whole turn back. An edit's parameter
//! locks (a lock set on a step, a step's locks going with it) undo and
//! redo with its notes.
//!
//! A track added at the end of the project ([`Takes::add_track`]) is a
//! take too, so UNDO takes it away again (with whatever it holds then,
//! which redo puts back). Takes are undone last first, so the takes made
//! on the new track are undone before it goes, and no take ever names a
//! track that moved.
//!
//! A Song view edit (a block moved, copied, made longer or deleted, a
//! section moved: [`Takes::song_edit`]) is an edit too: it keeps the song
//! before and after, so UNDO puts the song back as it was, and redo as it
//! became, as long as nothing else changed the song in between (live to
//! song, say); otherwise they leave it alone.
//!
//! Song → scenes fills rows: clips in their cells and each row's scene
//! (its name and follow action). [`Takes::rows_edit`] keeps every cell and
//! scene it changed, before and after, as one edit, so UNDO empties the
//! rows again and redo fills them, cell by cell and scene by scene, as
//! long as each is as the edit left it.
//!
//! A recorded note's pad pressure goes in with it ([`Takes::add_touched`]):
//! into the clip's pressure first, which only ever grows, so undo and
//! redo of the note need not touch it.
//!
//! Knob moves recorded as locks ([`Recorder::taken_locks`]) go in with
//! [`Takes::add_lock`], with the take they were recorded in: each replaces
//! a lock of the same parameter on the same tick and row, and a turn that
//! spans many periods stays one lock in the take.
//!
//! A host that numbers recorded changes ([`crate::recorded`]) hands each
//! to [`Takes::record`], which calls the one above that fits and decides
//! as `PlayView::record` does, so its view and the project agree.
//!
//! Perform FX gestures recorded as moves ([`Recorder::taken_moves`]) go in
//! with [`Takes::put_move`]: each replaces what its lane had over the
//! ticks it covers, and the take keeps what it replaced, so undo puts the
//! lane back exactly. A step edit that moves lanes (a page copied, a clip
//! made longer or shorter) keeps the lanes before and after.
//!
//! Most of this allocates. Recording does not, once a take has begun:
//! a new take's lists start with room for [`TAKE_ROOM`] notes and locks
//! each, so [`Takes::add`], [`Takes::erase`] and the lock versions only
//! allocate after that many (and the clip's own list, unless the host
//! reserved room in it; see `PlayView::make_room`). A take beginning
//! allocates its lists, and can free the oldest take and the undone ones.
//!
//! [`Player::erased`]: crate::Player::erased
//! [`Player::erased_locks`]: crate::Player::erased_locks
//! [`Recorder::taken`]: crate::Recorder::taken
//! [`Recorder::taken_locks`]: crate::Recorder::taken_locks
//! [`Recorder::taken_moves`]: crate::Recorder::taken_moves

use mpfree_seq::arrange::Arrangement;
use mpfree_seq::mpfree_moves::{Kind, Map, Point, Target};
use mpfree_seq::steps::{Edit, MovesEdit};
use mpfree_seq::{
    touch_room, Clip, ClipSettings, DestId, Lock, ModelError, Note, Project, Scene, SceneSettings,
    Touch, TouchKeep, TouchPoint, Track, MAX_TRACKS, SLOTS,
};
use mpfree_transport::time::Tick;

mod drops;

pub use drops::{Cell, ClipDrop};

use crate::recorded::Rec;
use crate::{Erased, ErasedLock, Taken, TakenLock, TakenMove};

/// What one recorded change did to the project ([`Takes::record`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    /// A clip changed.
    pub changed: bool,
    /// Pressure values the clip had no room for (the note went in
    /// without them).
    pub touch_lost: usize,
}

/// A gesture to write into a clip's lanes: its target, map and points (as
/// for `Moves::put`).
pub type MoveWrite = (Target, Map, Vec<(f64, f32, Kind)>);

/// A gesture a take wrote into a clip's lane, and what it replaced there.
#[derive(Clone, Debug, PartialEq)]
pub struct MovePut {
    pub track: usize,
    pub slot: usize,
    pub target: Target,
    pub map: Map,
    /// As written (`Moves::put`), for redo.
    pub points: Vec<(f64, f32, Kind)>,
    pub len: u32,
    /// What the lane had: (from, to, points) over clip ticks `from..to`,
    /// for undo (`Moves::restore`).
    pub saved: Vec<(u32, u32, Vec<Point>)>,
}

/// What a Song view edit did ([`Takes::song_edit`]), for the undo list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SongChange {
    /// A placement moved, or a copy of it put, on `track` from `start`.
    Moved {
        track: usize,
        start: Tick,
    },
    Copied {
        track: usize,
        start: Tick,
    },
    /// A placement on `track` made `length` song ticks long.
    Resized {
        track: usize,
        length: Tick,
    },
    /// A section moved, or a copy of it put, from song tick `at` on.
    SectionMoved {
        at: Tick,
    },
    SectionCopied {
        at: Tick,
    },
    /// A placement on `track` at `start` deleted, or given its own copy
    /// of its clip.
    Deleted {
        track: usize,
        start: Tick,
    },
    Unique {
        track: usize,
        start: Tick,
    },
    /// Scenes → song: `rows` rows onto the song's end.
    Scenes {
        rows: usize,
    },
    /// The song list edited (sequencer S4): a step put in, taken out or
    /// changed, or the list's loop switch.
    List,
    /// List → song: the list's `steps` steps onto the song's end.
    ListToSong {
        steps: usize,
    },
    /// A tempo point (tempo ramps) added, moved, given a tempo, made to
    /// glide or not, or deleted: the one at song tick `at` (where it was,
    /// for a delete).
    Tempo {
        at: Tick,
    },
}

/// A Song view edit: what it did, and the song before and after it.
#[derive(Clone, Debug, PartialEq)]
pub struct SongEdit {
    pub change: SongChange,
    pub before: Arrangement,
    pub after: Arrangement,
}

/// Takes kept for undo when no limit is given.
pub const DEFAULT_LIMIT: usize = 32;

/// Notes and locks a recorded take has room for in each of its lists
/// before adding one allocates.
pub const TAKE_ROOM: usize = 256;

/// Pressure values a clip may hold before [`Takes::reclaim_touch`] frees
/// the ones nothing names any more: half its room (refactor B4b).
pub const TOUCH_RECLAIM_AT: usize = mpfree_seq::MAX_TOUCH_POINTS / 2;

/// Whether take `t` puts a clip in slot `slot` of `track`, or takes one
/// out, or adds the track: the clip there is another one on its far
/// side ([`Takes::reclaim_touch`]).
fn swaps_clip(t: &Take, track: usize, slot: usize) -> bool {
    t.made
        .iter()
        .chain(&t.removed)
        .any(|(tr, sl, _)| (*tr, *sl) == (track, slot))
        || t.track_added.as_ref().is_some_and(|(i, _)| *i == track)
}

/// Each note take `t` added to or erased from clip `slot` of `track`.
fn notes_in(t: &mut Take, track: usize, slot: usize, f: &mut impl FnMut(&mut Note)) {
    for (tr, sl, n) in t.added.iter_mut().chain(t.erased.iter_mut()) {
        if (*tr, *sl) == (track, slot) {
            f(n);
        }
    }
}

/// What one take changed. Each note is (track, slot, note).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Take {
    pub number: u32,
    /// Made by CAPTURE, not by recording.
    pub captured: bool,
    /// An edit of a clip's notes (the step sequencer), not a recording.
    pub edited: bool,
    /// What a joining edit turns (see [`Takes::edit`]).
    join: Option<u64>,
    pub added: Vec<(usize, usize, Note)>,
    pub erased: Vec<(usize, usize, Note)>,
    /// Clips the take put in empty slots, as (track, slot, clip). Undo
    /// empties the slot and keeps the clip as it was then; redo puts it
    /// back if the slot is still empty.
    pub made: Vec<(usize, usize, Clip)>,
    /// Clips the take took out of their slots (ERASE + a clip), as
    /// (track, slot, clip). Undo puts each back if its slot is still
    /// empty; redo takes it out again.
    pub removed: Vec<(usize, usize, Clip)>,
    /// Clips the take made longer or shorter, as (track, slot, from, to)
    /// in ticks (a page added or taken off).
    pub resized: Vec<(usize, usize, Tick, Tick)>,
    /// Parameter locks an edit put in and took out, as (track, slot, lock).
    pub locks_added: Vec<(usize, usize, Lock)>,
    pub locks_erased: Vec<(usize, usize, Lock)>,
    /// Gestures recorded or captured into lanes, in order.
    pub moves: Vec<MovePut>,
    /// Lanes an edit moved, as (track, slot, before and after).
    pub moves_edited: Vec<(usize, usize, MovesEdit)>,
    /// A track the take added at the end of the project, as (its index,
    /// the track). Undo takes it off the end, kept as it is then; redo
    /// puts it back if the project is that long again.
    pub track_added: Option<(usize, Track)>,
    /// A drop on the Clips grid ([`Takes::drop_clip`], [`Takes::drop_row`]):
    /// its clips are in `removed` and `made`, and undo and redo move the
    /// song's linked blocks, the scenes and the morph's rows with them.
    pub dropped: Option<ClipDrop>,
    /// A Song view edit ([`Takes::song_edit`]).
    pub song: Option<Box<SongEdit>>,
    /// A clip's settings changed ([`Takes::set_clip`]), as (track, slot,
    /// before, after).
    pub settings: Option<(usize, usize, ClipSettings, ClipSettings)>,
    /// COPY + an empty slot put a copy there ([`Takes::paste_clip`]): the
    /// clip is in `made`.
    pub pasted: bool,
    /// The clip in `made` was launched as it went in (the Sequencer's first
    /// step on an empty track, [`Takes::new_clip`]): redo launches it
    /// again, as the step did. Undo needs nothing: a clip that leaves its
    /// slot stops.
    pub launched: bool,
    /// A whole clip put in place of the clip there
    /// ([`Takes::replace_clip`]), and why: the clip it replaced is in
    /// `removed`, the new one in `made`.
    pub replaced: Option<Replaced>,
    /// A scene's name, mutes or follow action changed
    /// ([`Takes::set_scene`]).
    pub scene: Option<Box<SceneEdit>>,
    /// Whole rows changed at once ([`Takes::rows_edit`]: Song → scenes):
    /// their scenes, before and after. The clips are in `made` and
    /// `removed`.
    pub rows: Option<Box<RowsEdit>>,
}

/// Why a take put a whole clip in place of another ([`Take::replaced`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replaced {
    /// Reload clip: its saved state (sequencer S4).
    Reload,
    /// Mutate: a variation, the clip as it was kept beside it (S6).
    Mutate,
    /// Restore: the clip as it was before its first Mutate (S6).
    Restore,
}

/// A scene editor change: row `row`'s settings before and after.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneEdit {
    pub row: usize,
    pub before: SceneSettings,
    pub after: SceneSettings,
}

/// Rows changed as one edit ([`Takes::rows_edit`]).
#[derive(Clone, Debug, PartialEq)]
pub struct RowsEdit {
    /// The rows it changed, in order.
    pub rows: Vec<usize>,
    /// The scenes it changed, as (row, before, after), whole: a plain
    /// row's scene is `Scene::default()`.
    pub scenes: Vec<(usize, Scene, Scene)>,
}

/// Row `row`'s whole scene (a plain one past the last).
fn whole_scene(project: &Project, row: usize) -> Scene {
    project.scene(row).cloned().unwrap_or_default()
}

/// Give row `row` the scene `s`, whole, leaving no plain rows at the end
/// of the project's scenes. Nothing past the last row.
fn put_whole_scene(project: &mut Project, row: usize, s: &Scene) {
    if let Some(at) = project.scene_mut(row) {
        at.clone_from(s);
    }
    drop_plain_scenes(project);
}

/// Take the plain scenes off the end of the project's scenes.
fn drop_plain_scenes(project: &mut Project) {
    while project
        .scenes
        .last()
        .is_some_and(|s| *s == Scene::default())
    {
        project.scenes.pop();
    }
}

/// Row `row`'s settings (a plain row's are empty).
fn scene_settings(project: &Project, row: usize) -> SceneSettings {
    project
        .scene(row)
        .map(mpfree_seq::Scene::settings)
        .unwrap_or_default()
}

/// Give row `row` the settings `s`, leaving no plain rows at the end of
/// the project's scenes. `BadScene` past the last row, or as
/// [`mpfree_seq::Scene::set_settings`].
fn put_scene(project: &mut Project, row: usize, s: &SceneSettings) -> Result<(), ModelError> {
    s.check()?;
    project
        .scene_mut(row)
        .ok_or(ModelError::BadScene)?
        .set_settings(s)?;
    drop_plain_scenes(project);
    Ok(())
}

/// The takes that can be undone, oldest first, and the ones that can be
/// redone.
#[derive(Clone, Debug)]
pub struct Takes {
    done: Vec<Take>,
    undone: Vec<Take>,
    limit: usize,
    /// Captures so far, to number them.
    captures: u32,
    /// Edits so far, to number them.
    edits: u32,
    /// Captures join this one while it is the latest take
    /// ([`Takes::joining`]).
    joining: Option<u32>,
    /// What the latest new take put out as it went in, for
    /// [`Takes::withdraw`]: the oldest take of a full list, and the takes
    /// redo could bring. Kept until the next new take.
    evicted: Option<Take>,
    cleared: Vec<Take>,
}

impl Default for Takes {
    fn default() -> Self {
        Takes::new(DEFAULT_LIMIT)
    }
}

impl Takes {
    /// Keep up to `limit` takes for undo (at least 1); older ones are
    /// forgotten and stay as they are.
    pub fn new(limit: usize) -> Takes {
        Takes {
            done: vec![],
            undone: vec![],
            limit: limit.max(1),
            captures: 0,
            edits: 0,
            joining: None,
            evicted: None,
            cleared: vec![],
        }
    }

    /// Add a recorded note to its clip and remember it with its take.
    /// `BadSlot` if the clip is gone; the clip's own refusals as they are.
    pub fn add(&mut self, project: &mut Project, k: &Taken) -> Result<(), ModelError> {
        let clip = clip(project, k.track, k.slot).ok_or(ModelError::BadSlot)?;
        clip.add_note(k.note)?;
        self.entry(k.take).added.push((k.track, k.slot, k.note));
        Ok(())
    }

    /// [`Takes::add`] for a recorded note with its pad pressure, `points`
    /// (`Recorder::touch_points`): they go into the clip first
    /// (`Clip::add_touch`) and the note names them there. Pressure the
    /// clip has no room for is left off the note (the second value says
    /// so); the note still goes in. The note as added. Undo and redo take
    /// the note out and put it back; its pressure stays in the clip, so
    /// the note keeps it (even through [`Takes::reclaim_touch`], which
    /// renumbers it).
    pub fn add_touched(
        &mut self,
        project: &mut Project,
        k: &Taken,
        points: &[TouchPoint],
    ) -> Result<(Note, bool), ModelError> {
        let c = clip(project, k.track, k.slot).ok_or(ModelError::BadSlot)?;
        let mut note = k.note;
        let (touch, kept) = match c.add_touch(points) {
            Ok(t) => (t, true),
            Err(_) => (Touch::NONE, false),
        };
        note.touch = touch;
        self.add(project, &Taken { note, ..*k })?;
        Ok((note, kept))
    }

    /// Make recorded change `rec` to the project and remember it with its
    /// take, with a note's pressure `touch` or a gesture's `moves`:
    /// [`Takes::add_touched`], [`Takes::erase`], [`Takes::add_lock`],
    /// [`Takes::erase_lock`] or [`Takes::put_move`]. A change its clip
    /// refuses, or one to a clip that is gone, changes nothing.
    pub fn record(
        &mut self,
        project: &mut Project,
        rec: &Rec,
        touch: &[TouchPoint],
        moves: &[(f64, f32, Kind)],
    ) -> Recorded {
        let mut done = Recorded::default();
        match rec {
            Rec::Note(k) => {
                if let Ok((_, kept)) = self.add_touched(project, k, touch) {
                    done.changed = true;
                    if !kept {
                        done.touch_lost = touch.len();
                    }
                }
            }
            Rec::Erase(e) => done.changed = self.erase(project, e),
            Rec::Lock(k) => done.changed = self.add_lock(project, k).is_ok(),
            Rec::EraseLock(e) => done.changed = self.erase_lock(project, e),
            Rec::Move(k) => done.changed = self.put_move(project, k, moves).is_ok(),
        }
        done
    }

    /// Put a recorded lock in its clip, in place of a lock of the same
    /// parameter on the same tick and row, and remember it with its take.
    /// A later value of a lock this take recorded replaces it in the take
    /// too, so undo puts back what was there before the take. `BadSlot` if
    /// the clip is gone; the clip's own refusals as they are; nothing
    /// changes on an error. The lock it replaced, if any (the new one took
    /// its place in the clip's list; otherwise it went on the end).
    pub fn add_lock(
        &mut self,
        project: &mut Project,
        k: &TakenLock,
    ) -> Result<Option<Lock>, ModelError> {
        let c = clip(project, k.track, k.slot).ok_or(ModelError::BadSlot)?;
        let new = k.lock;
        let key = |x: &Lock| x.same_place(&new);
        let i = c.locks.iter().position(key);
        let old = i.map(|i| c.locks.remove(i));
        if let Err(e) = c.add_lock(new) {
            if let (Some(i), Some(o)) = (i, old) {
                c.locks.insert(i, o);
            }
            return Err(e);
        }
        // In the old one's place, so the clip's order does not depend on
        // how a turn was split into periods.
        if let Some(i) = i {
            let l = c.locks.pop().expect("just added");
            c.locks.insert(i, l);
        }
        let at = (k.track, k.slot);
        let take = self.entry(k.take);
        let ours = old.and_then(|o| {
            take.locks_added
                .iter()
                .rposition(|&(tr, sl, x)| (tr, sl) == at && x == o)
        });
        match (ours, old) {
            (Some(j), _) => take.locks_added[j].2 = new,
            (None, Some(o)) => {
                take.locks_erased.push((k.track, k.slot, o));
                take.locks_added.push((k.track, k.slot, new));
            }
            (None, None) => take.locks_added.push((k.track, k.slot, new)),
        }
        Ok(old)
    }

    /// Write a recorded gesture into its clip's lane (replacing what the
    /// lane had over the ticks it covers) and remember it with its take.
    /// `BadSlot` if the clip is gone or is no longer `k.len` ticks long;
    /// `BadMoves` if the lane is full or the points are not in order.
    /// Nothing changes on an error.
    pub fn put_move(
        &mut self,
        project: &mut Project,
        k: &TakenMove,
        points: &[(f64, f32, Kind)],
    ) -> Result<(), ModelError> {
        let c = clip(project, k.track, k.slot).ok_or(ModelError::BadSlot)?;
        let put = write_move(c, k.track, k.slot, k.target, k.map, points, k.len)?;
        self.entry(k.take).moves.push(put);
        Ok(())
    }

    /// CAPTURE's moves: write `gestures` (target, map and points, as for
    /// [`Takes::put_move`]) into clip `slot` of `track`, joining capture
    /// `number`, which must be the latest take. All go in or none. The
    /// number of gestures written.
    pub fn capture_moves(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        number: u32,
        gestures: &[MoveWrite],
    ) -> Result<usize, ModelError> {
        if !self
            .done
            .last()
            .is_some_and(|t| t.captured && t.number == number && self.undone.is_empty())
        {
            return Err(ModelError::BadSlot);
        }
        let c = clip(project, track, slot).ok_or(ModelError::BadSlot)?;
        let before = c.moves.clone();
        let len = u32::try_from(c.length).map_err(|_| ModelError::ClipLength)?;
        let mut puts = Vec::with_capacity(gestures.len());
        for (target, map, points) in gestures {
            match write_move(c, track, slot, *target, *map, points, len) {
                Ok(p) => puts.push(p),
                Err(e) => {
                    c.moves = before;
                    return Err(e);
                }
            }
        }
        let n = puts.len();
        self.done.last_mut().expect("checked").moves.extend(puts);
        Ok(n)
    }

    /// Remove an erased note from its clip and remember it with its take.
    /// False if the clip does not hold it (it was edited meanwhile); then
    /// nothing is remembered.
    pub fn erase(&mut self, project: &mut Project, e: &Erased) -> bool {
        let Some(clip) = clip(project, e.track, e.slot) else {
            return false;
        };
        if !clip.remove_note(&e.note) {
            return false;
        }
        self.entry(e.take).erased.push((e.track, e.slot, e.note));
        true
    }

    /// Remove a lock replace recording erased from its clip and remember
    /// it with its take. False if the clip does not hold it (it was edited
    /// meanwhile); then nothing is remembered.
    pub fn erase_lock(&mut self, project: &mut Project, e: &ErasedLock) -> bool {
        let Some(clip) = clip(project, e.track, e.slot) else {
            return false;
        };
        if !clip.remove_lock(&e.lock) {
            return false;
        }
        self.entry(e.take)
            .locks_erased
            .push((e.track, e.slot, e.lock));
        true
    }

    /// CAPTURE into an empty slot: put `new` in clip `slot` of `track`, as
    /// a take of its own. The capture's number. `BadSlot` if there is no
    /// such slot or it holds a clip; the clip's own refusals as they are.
    /// Nothing changes on an error.
    pub fn capture_clip(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        new: Clip,
    ) -> Result<u32, ModelError> {
        new.check()?;
        let s = slot_mut(project, track, slot).ok_or(ModelError::BadSlot)?;
        if s.is_some() {
            return Err(ModelError::BadSlot);
        }
        *s = Some(new.clone());
        let take = self.next_capture();
        take.made.push((track, slot, new));
        Ok(take.number)
    }

    /// Several clips into empty slots at once (a MIDI file's tracks, in
    /// the app's import), as one capture: one UNDO takes them all away.
    /// `clips` are (track, slot, clip). The capture's number. Every clip
    /// is checked and every slot must be there, empty and named once,
    /// before any goes in: `BadSlot` or the clip's own refusal, and nothing
    /// changes, otherwise. An empty list is `BadSlot` too.
    pub fn capture_clips(
        &mut self,
        project: &mut Project,
        clips: Vec<(usize, usize, Clip)>,
    ) -> Result<u32, ModelError> {
        if clips.is_empty() {
            return Err(ModelError::BadSlot);
        }
        for (i, (track, slot, c)) in clips.iter().enumerate() {
            c.check()?;
            let free = project
                .tracks
                .get(*track)
                .and_then(|t| t.slots.get(*slot))
                .is_some_and(Option::is_none);
            let twice = clips[..i].iter().any(|o| (o.0, o.1) == (*track, *slot));
            if !free || twice {
                return Err(ModelError::BadSlot);
            }
        }
        for (track, slot, c) in &clips {
            *slot_mut(project, *track, *slot).expect("checked") = Some(c.clone());
        }
        let take = self.next_capture();
        take.made.extend(clips);
        Ok(take.number)
    }

    /// COPY + an empty slot (sequencer S3): put `clip`, a copy of one
    /// copied before, in clip `slot` of `track`, as an edit of its own that
    /// undo takes away again. The edit's number. `BadSlot` if there is no
    /// such slot or it holds a clip; the clip's own refusals as they are.
    /// Nothing changes on an error.
    pub fn paste_clip(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        clip: Clip,
    ) -> Result<u32, ModelError> {
        self.put_new(project, track, slot, clip, true, false)
    }

    /// The Sequencer's first step on an empty track: put `clip`, a new
    /// clip that already holds the step's notes, in clip `slot` of
    /// `track`, as one edit. Undo takes it away whole, leaving the slot
    /// empty, and redo puts it back; `launched` says the clip was launched
    /// as it went in, so redo launches it again ([`Take::launched`]). The
    /// edit's number. `BadSlot` if there is no such slot or it holds a
    /// clip; the clip's own refusals as they are. Nothing changes on an
    /// error.
    pub fn new_clip(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        clip: Clip,
        launched: bool,
    ) -> Result<u32, ModelError> {
        self.put_new(project, track, slot, clip, false, launched)
    }

    fn put_new(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        clip: Clip,
        pasted: bool,
        launched: bool,
    ) -> Result<u32, ModelError> {
        clip.check()?;
        let s = slot_mut(project, track, slot).ok_or(ModelError::BadSlot)?;
        if s.is_some() {
            return Err(ModelError::BadSlot);
        }
        *s = Some(clip.clone());
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            pasted,
            launched,
            made: vec![(track, slot, clip)],
            ..Take::default()
        });
        Ok(number)
    }

    /// Reload clip (sequencer S4, Elektron's temp save and reload): put
    /// `saved`, a state of clip `slot` of `track` kept earlier, in place of
    /// the clip there now ([`Takes::replace_clip`]).
    pub fn reload_clip(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        saved: Clip,
    ) -> Result<u32, ModelError> {
        self.replace_clip(project, track, slot, saved, Replaced::Reload)
    }

    /// Put `clip` in place of clip `slot` of `track`, as an edit of its
    /// own, for `why`: a reload, a Mutate or a Restore (S6,
    /// [`mpfree_seq::mutate`]). Undo puts back the clip as it was, redo
    /// `clip` again. The edit's number. `BadSlot` if the slot is empty;
    /// `clip`'s own refusals as they are. Nothing changes on an error.
    pub fn replace_clip(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        clip: Clip,
        why: Replaced,
    ) -> Result<u32, ModelError> {
        clip.check()?;
        let s = slot_mut(project, track, slot)
            .filter(|s| s.is_some())
            .ok_or(ModelError::BadSlot)?;
        let now = s.replace(clip.clone()).expect("checked");
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            replaced: Some(why),
            removed: vec![(track, slot, now)],
            made: vec![(track, slot, clip)],
            ..Take::default()
        });
        Ok(number)
    }

    /// ERASE + a clip (Jay after live test 3): take clip `slot` of
    /// `track` out of its slot, as an edit of its own that undo puts back.
    /// The edit's number. `BadSlot` if there is no clip there; nothing
    /// changes then.
    pub fn remove_clip(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
    ) -> Result<u32, ModelError> {
        let gone = slot_mut(project, track, slot)
            .and_then(Option::take)
            .ok_or(ModelError::BadSlot)?;
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            removed: vec![(track, slot, gone)],
            ..Take::default()
        });
        Ok(number)
    }

    /// A new empty track named `name` at the end of the project, as an
    /// edit of its own. Its destination is the lowest one no track uses
    /// (a new project's tracks use their own numbers). Its edit number and
    /// index, or `TooManyTracks` at [`MAX_TRACKS`].
    pub fn add_track(
        &mut self,
        project: &mut Project,
        name: &str,
    ) -> Result<(u32, usize), ModelError> {
        self.add_track_as(project, Track::new(name, 0))
    }

    /// [`Takes::add_track`] for `track` (a Loop track, say), given the
    /// lowest destination no track uses.
    pub fn add_track_as(
        &mut self,
        project: &mut Project,
        mut track: Track,
    ) -> Result<(u32, usize), ModelError> {
        if project.tracks.len() >= MAX_TRACKS {
            return Err(ModelError::TooManyTracks);
        }
        track.dest = (0..=MAX_TRACKS as DestId)
            .find(|d| project.tracks.iter().all(|t| t.dest != *d))
            .unwrap_or(project.tracks.len() as DestId);
        // Its lasting id, kept by the take, so REDO brings back the same
        // track.
        project.give_track_id(&mut track);
        let index = project.tracks.len();
        project.tracks.push(track.clone());
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            track_added: Some((index, track)),
            ..Take::default()
        });
        Ok((number, index))
    }

    /// A Song view edit the caller has made: `before` is the song as it
    /// was, the project's is the song now. An edit of its own, which undo
    /// takes back and redo does again while nothing else has changed the
    /// song (see the module notes). The edit's number.
    pub fn song_edit(&mut self, project: &Project, change: SongChange, before: Arrangement) -> u32 {
        self.song_edit_joining(project, change, before, None)
    }

    /// [`Takes::song_edit`], where `join` names what is being turned (a
    /// tempo point's tempo on the wheel): the edit joins the latest take
    /// instead when that is a song edit with the same key, nothing was
    /// undone since and nothing else changed the song since (it is as that
    /// take left it), so the wheel turned through many values is one undo.
    pub fn song_edit_joining(
        &mut self,
        project: &Project,
        change: SongChange,
        before: Arrangement,
        join: Option<u64>,
    ) -> u32 {
        let joins = join.is_some()
            && self.undone.is_empty()
            && self.done.last().is_some_and(|t| {
                t.join == join && t.song.as_ref().is_some_and(|e| e.after == before)
            });
        if joins {
            let take = self.done.last_mut().expect("checked");
            if let Some(e) = take.song.as_mut() {
                e.change = change;
                e.after = project.arrangement.clone();
            }
            return take.number;
        }
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            join,
            song: Some(Box::new(SongEdit {
                change,
                before,
                after: project.arrangement.clone(),
            })),
            ..Take::default()
        });
        number
    }

    /// Change clip `slot` of `track`'s speed, swing, quantise and follow
    /// action to `to` (the clip settings card), as an edit of its own.
    /// `join` names what is being turned: a change joins the latest take
    /// instead when that is a settings change of the same clip with the
    /// same key and nothing undone since, so the wheel turned through many
    /// values is one undo. Undo puts the settings back while the clip
    /// still has the ones it was given, and redo the same way. The edit's
    /// number; `BadSlot` if the clip is gone, or the clip's own refusal;
    /// nothing changes on an error, nor when `to` is what it has.
    pub fn set_clip(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        to: ClipSettings,
        join: Option<u64>,
    ) -> Result<u32, ModelError> {
        let c = clip(project, track, slot).ok_or(ModelError::BadSlot)?;
        let from = c.settings();
        if from == to {
            return Ok(self.edits);
        }
        c.set_settings(to)?;
        let joins = join.is_some()
            && self.undone.is_empty()
            && self.done.last().is_some_and(|t| {
                t.join == join
                    && t.settings
                        .is_some_and(|(tr, sl, ..)| (tr, sl) == (track, slot))
            });
        if joins {
            let take = self.done.last_mut().expect("checked");
            if let Some(s) = take.settings.as_mut() {
                s.3 = to;
            }
            return Ok(take.number);
        }
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            join,
            settings: Some((track, slot, from, to)),
            ..Take::default()
        });
        Ok(number)
    }

    /// The scene editor (sequencer S4): give row `row` the settings `to`
    /// (its name, mutes and follow action; its morph values stay), as an
    /// edit of its own. `join` names what is being turned: a change joins
    /// the latest take instead when that is a change of the same row with
    /// the same key and nothing undone since. Undo puts the settings back
    /// while the row still has the ones it was given, and redo the same
    /// way. The edit's number; `BadScene` past the last row or for
    /// settings out of range; nothing changes on an error, nor when `to`
    /// is what it has.
    pub fn set_scene(
        &mut self,
        project: &mut Project,
        row: usize,
        to: SceneSettings,
        join: Option<u64>,
    ) -> Result<u32, ModelError> {
        if row >= mpfree_seq::SCENES {
            return Err(ModelError::BadScene);
        }
        let from = scene_settings(project, row);
        if from == to {
            return Ok(self.edits);
        }
        put_scene(project, row, &to)?;
        let joins = join.is_some()
            && self.undone.is_empty()
            && self
                .done
                .last()
                .is_some_and(|t| t.join == join && t.scene.as_ref().is_some_and(|e| e.row == row));
        if joins {
            let take = self.done.last_mut().expect("checked");
            if let Some(e) = take.scene.as_mut() {
                e.after = to;
            }
            return Ok(take.number);
        }
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            join,
            scene: Some(Box::new(SceneEdit {
                row,
                before: from,
                after: to,
            })),
            ..Take::default()
        });
        Ok(number)
    }

    /// Song → scenes, made by the caller: `before` is the project as it
    /// was and `project` as it is now, with the same tracks. Every cell
    /// and scene that changed, in any row, goes into one edit of its own:
    /// a clip put in an empty cell goes in `made`, a clip taken out in
    /// `removed`, a clip changed in both, and each row's scene before and
    /// after. Undo takes each new clip out and puts back the one that was
    /// there, and gives each row its scene back while it has the one the
    /// edit gave it; redo the same way. The edit's number, or the latest
    /// edit's when nothing changed (no take then).
    pub fn rows_edit(&mut self, before: &Project, project: &Project) -> u32 {
        let mut made = Vec::new();
        let mut removed = Vec::new();
        let mut rows = Vec::new();
        let mut scenes = Vec::new();
        for row in 0..SLOTS {
            let mut changed = false;
            for track in 0..project.tracks.len() {
                let (was, now) = (before.clip(track, row), project.clip(track, row));
                if was == now {
                    continue;
                }
                changed = true;
                if let Some(c) = was {
                    removed.push((track, row, c.clone()));
                }
                if let Some(c) = now {
                    made.push((track, row, c.clone()));
                }
            }
            let (was, now) = (whole_scene(before, row), whole_scene(project, row));
            if was != now {
                changed = true;
                scenes.push((row, was, now));
            }
            if changed {
                rows.push(row);
            }
        }
        if rows.is_empty() {
            return self.edits;
        }
        self.edits += 1;
        let number = self.edits;
        self.push(Take {
            number,
            edited: true,
            made,
            removed,
            rows: Some(Box::new(RowsEdit { rows, scenes })),
            ..Take::default()
        });
        number
    }

    /// CAPTURE into a clip that holds notes already (the one that was
    /// playing): add `notes` to clip `slot` of `track`, as a take of its
    /// own, with their pressure: each note's `touch` names its run in
    /// `touch`, which goes into the clip first (`Clip::touch`). The
    /// capture's number, and the pressure values the clip had no room for
    /// (then every note goes in without its pressure). All the notes go in
    /// or none: `BadSlot` if the clip is gone, `TooManyNotes` or `BadNote`
    /// (its index in `notes`) if one does not fit.
    pub fn capture_notes(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        notes: &[Note],
        touch: &[TouchPoint],
    ) -> Result<(u32, usize), ModelError> {
        let c = clip(project, track, slot).ok_or(ModelError::BadSlot)?;
        let (before, had) = (c.notes.len(), c.touch.len());
        let room = !touch.is_empty() && touch_room(had, touch);
        let lost = if room || touch.is_empty() {
            0
        } else {
            touch.len()
        };
        let mut notes = notes.to_vec();
        for n in &mut notes {
            if room && !n.touch.is_none() {
                n.touch.first += had as u32;
            } else {
                n.touch = Touch::NONE;
            }
        }
        if room {
            c.touch.extend_from_slice(touch);
        }
        for (i, n) in notes.iter().enumerate() {
            if let Err(e) = c.add_note(*n) {
                c.notes.truncate(before);
                c.touch.truncate(had);
                return Err(match e {
                    ModelError::BadNote(_) => ModelError::BadNote(i),
                    e => e,
                });
            }
        }
        let take = self.next_capture();
        take.added.extend(notes.iter().map(|&n| (track, slot, n)));
        Ok((take.number, lost))
    }

    /// Edit clip `slot` of `track`: `edit`'s notes to take out come out
    /// and its new notes go in, all or none, as a take of its own. `join`
    /// names what is being turned (such as `steps::Change::key`): an edit
    /// joins the latest take instead when that is an edit of the same clip
    /// with the same key and nothing undone since, so a Q-Link turned
    /// through many values is one undo. The edit's number. `BadSlot` if
    /// the clip is gone; `MissingNote` or the clip's own refusals as they
    /// are; nothing changes on an error. An empty edit changes nothing and
    /// makes no take.
    pub fn edit(
        &mut self,
        project: &mut Project,
        track: usize,
        slot: usize,
        edit: &Edit,
        join: Option<u64>,
    ) -> Result<u32, ModelError> {
        let c = clip(project, track, slot).ok_or(ModelError::BadSlot)?;
        if edit.is_empty() {
            return Ok(self.edits);
        }
        c.apply(edit)?;
        let same_clip = |t: &Take| {
            t.added
                .iter()
                .chain(&t.erased)
                .all(|&(tr, sl, _)| (tr, sl) == (track, slot))
                && t.locks_added
                    .iter()
                    .chain(&t.locks_erased)
                    .all(|&(tr, sl, _)| (tr, sl) == (track, slot))
        };
        let joins = join.is_some()
            && edit.length.is_none()
            && edit.moves.is_none()
            && self.undone.is_empty()
            && self.done.last().is_some_and(|t| {
                t.edited
                    && t.join == join
                    && t.resized.is_empty()
                    && t.settings.is_none()
                    && same_clip(t)
            });
        if !joins {
            self.edits += 1;
            let number = self.edits;
            let take = self.push(Take {
                number,
                edited: true,
                join,
                ..Take::default()
            });
            take.erased
                .extend(edit.erase.iter().map(|&n| (track, slot, n)));
            take.added
                .extend(edit.add.iter().map(|&n| (track, slot, n)));
            take.resized
                .extend(edit.length.map(|(from, to)| (track, slot, from, to)));
            take.locks_erased
                .extend(edit.erase_locks.iter().map(|&l| (track, slot, l)));
            take.locks_added
                .extend(edit.add_locks.iter().map(|&l| (track, slot, l)));
            take.moves_edited
                .extend(edit.moves.as_deref().map(|m| (track, slot, m.clone())));
            return Ok(number);
        }
        let take = self.done.last_mut().expect("checked");
        // A note this take put in and now takes out was never there; a note
        // it took out and now puts back is as it was.
        for &n in &edit.erase {
            match take
                .added
                .iter()
                .position(|&(tr, sl, x)| (tr, sl, x) == (track, slot, n))
            {
                Some(i) => {
                    take.added.remove(i);
                }
                None => take.erased.push((track, slot, n)),
            }
        }
        for &n in &edit.add {
            match take
                .erased
                .iter()
                .position(|&(tr, sl, x)| (tr, sl, x) == (track, slot, n))
            {
                Some(i) => {
                    take.erased.remove(i);
                }
                None => take.added.push((track, slot, n)),
            }
        }
        // The same for locks: a lock turned through many values is one.
        for &l in &edit.erase_locks {
            match take.locks_added.iter().position(|&x| x == (track, slot, l)) {
                Some(i) => {
                    take.locks_added.remove(i);
                }
                None => take.locks_erased.push((track, slot, l)),
            }
        }
        for &l in &edit.add_locks {
            match take
                .locks_erased
                .iter()
                .position(|&x| x == (track, slot, l))
            {
                Some(i) => {
                    take.locks_erased.remove(i);
                }
                None => take.locks_added.push((track, slot, l)),
            }
        }
        Ok(take.number)
    }

    /// Undo the latest take: its recorded notes come out, its erased notes
    /// go back and the clips it made leave their slots. Its number, or
    /// None if there is nothing to undo. Notes whose clip is gone, or no
    /// longer has room or reach for them (it was shortened), are skipped.
    pub fn undo(&mut self, project: &mut Project) -> Option<u32> {
        let mut take = self.done.pop()?;
        for (track, slot, note) in take.added.iter().rev() {
            if let Some(c) = clip(project, *track, *slot) {
                c.remove_note(note);
            }
        }
        for (track, slot, lock) in take.locks_added.iter().rev() {
            if let Some(c) = clip(project, *track, *slot) {
                c.remove_lock(lock);
            }
        }
        for put in take.moves.iter().rev() {
            if let Some(c) = clip(project, put.track, put.slot) {
                for (from, to, old) in put.saved.iter().rev() {
                    let _ = c.moves.restore(put.target, put.map, *from, *to, old);
                }
            }
        }
        // The lanes first: the length then checks them.
        for (track, slot, m) in take.moves_edited.iter().rev() {
            if let Some(c) = clip(project, *track, *slot) {
                c.moves = m.before.clone();
            }
        }
        for &(track, slot, from, to) in take.resized.iter().rev() {
            resize(project, track, slot, to, from);
        }
        for (track, slot, note) in take.erased.iter().rev() {
            if let Some(c) = clip(project, *track, *slot) {
                let _ = c.add_note(*note);
            }
        }
        for (track, slot, lock) in take.locks_erased.iter().rev() {
            if let Some(c) = clip(project, *track, *slot) {
                let _ = c.add_lock(*lock);
            }
        }
        for (track, slot, made) in take.made.iter_mut().rev() {
            // Kept as it is now, with any notes recorded into it since.
            if let Some(c) = slot_mut(project, *track, *slot).and_then(Option::take) {
                *made = c;
            }
        }
        for (track, slot, gone) in take.removed.iter().rev() {
            // A slot filled meanwhile keeps its clip.
            if let Some(s) = slot_mut(project, *track, *slot).filter(|s| s.is_none()) {
                *s = Some(gone.clone());
            }
        }
        if let Some(d) = take.dropped {
            drops::follow(project, d, true);
        }
        if let Some(e) = take.song.as_deref() {
            if project.arrangement == e.after {
                project.arrangement = e.before.clone();
            }
        }
        if let Some((track, slot, before, after)) = take.settings {
            if let Some(c) = clip(project, track, slot).filter(|c| c.settings() == after) {
                let _ = c.set_settings(before);
            }
        }
        if let Some(e) = take.scene.as_deref() {
            if scene_settings(project, e.row) == e.after {
                let _ = put_scene(project, e.row, &e.before);
            }
        }
        if let Some(e) = take.rows.as_deref() {
            for (row, was, now) in e.scenes.iter().rev() {
                if whole_scene(project, *row) == *now {
                    put_whole_scene(project, *row, was);
                }
            }
        }
        if let Some((index, kept)) = &mut take.track_added {
            // Kept as it is now (its name, its clips), for redo.
            if project.tracks.len() == *index + 1 {
                if let Some(t) = project.tracks.pop() {
                    *kept = t;
                }
            }
        }
        let n = take.number;
        self.undone.push(take);
        Some(n)
    }

    /// Redo the latest undone take. Its number, or None.
    pub fn redo(&mut self, project: &mut Project) -> Option<u32> {
        let mut take = self.undone.pop()?;
        if let Some((index, t)) = &take.track_added {
            if project.tracks.len() == *index {
                project.tracks.push(t.clone());
            }
        }
        for (track, slot, gone) in &mut take.removed {
            // Kept as it is now, for the next undo.
            if let Some(c) = slot_mut(project, *track, *slot).and_then(Option::take) {
                *gone = c;
            }
        }
        for (track, slot, made) in &take.made {
            // A slot filled meanwhile keeps its clip.
            if let Some(s) = slot_mut(project, *track, *slot).filter(|s| s.is_none()) {
                *s = Some(made.clone());
            }
        }
        if let Some(d) = take.dropped {
            drops::follow(project, d, false);
        }
        if let Some(e) = take.song.as_deref() {
            if project.arrangement == e.before {
                project.arrangement = e.after.clone();
            }
        }
        if let Some((track, slot, before, after)) = take.settings {
            if let Some(c) = clip(project, track, slot).filter(|c| c.settings() == before) {
                let _ = c.set_settings(after);
            }
        }
        if let Some(e) = take.scene.as_deref() {
            if scene_settings(project, e.row) == e.before {
                let _ = put_scene(project, e.row, &e.after);
            }
        }
        if let Some(e) = take.rows.as_deref() {
            for (row, was, now) in &e.scenes {
                if whole_scene(project, *row) == *was {
                    put_whole_scene(project, *row, now);
                }
            }
        }
        for (track, slot, note) in &take.erased {
            if let Some(c) = clip(project, *track, *slot) {
                c.remove_note(note);
            }
        }
        for (track, slot, lock) in &take.locks_erased {
            if let Some(c) = clip(project, *track, *slot) {
                c.remove_lock(lock);
            }
        }
        for (track, slot, m) in &take.moves_edited {
            if let Some(c) = clip(project, *track, *slot) {
                c.moves = m.after.clone();
            }
        }
        for &(track, slot, from, to) in &take.resized {
            resize(project, track, slot, from, to);
        }
        for (track, slot, note) in &take.added {
            if let Some(c) = clip(project, *track, *slot) {
                let _ = c.add_note(*note);
            }
        }
        for (track, slot, lock) in &take.locks_added {
            if let Some(c) = clip(project, *track, *slot) {
                let _ = c.add_lock(*lock);
            }
        }
        for put in &mut take.moves {
            if let Some(c) = clip(project, put.track, put.slot) {
                if let Ok(again) = write_move(
                    c,
                    put.track,
                    put.slot,
                    put.target,
                    put.map,
                    &put.points,
                    put.len,
                ) {
                    put.saved = again.saved;
                }
            }
        }
        let n = take.number;
        self.done.push(take);
        Some(n)
    }

    /// Forget the take undo just took back, so redo can't bring it again
    /// (an edit that could not go in).
    pub fn forget_undone(&mut self) -> Option<Take> {
        self.undone.pop()
    }

    /// Take back the edit to clip `slot` of `track` just made by
    /// [`Takes::edit`] or [`Takes::new_clip`], when what plays could not
    /// follow it (the play view would not compile): the clip and the takes
    /// are as they were before it, and redo can't bring it. `edit` and
    /// `join` are what `edit` was given (an empty edit for `new_clip`);
    /// `fresh` says the edit made a take of its own (the edit count went
    /// up) rather than joining the latest one. A take of its own goes, and
    /// what its push put out comes back: the oldest take of a full list,
    /// and the takes redo could bring. A joining edit comes out of that
    /// take alone, the turns before it staying. `BadSlot`, with nothing
    /// changed, if the latest take is not an edit.
    pub fn withdraw(
        &mut self,
        project: &mut Project,
        (track, slot): (usize, usize),
        edit: &Edit,
        join: Option<u64>,
        fresh: bool,
    ) -> Result<(), ModelError> {
        if !self.done.last().is_some_and(|t| t.edited) {
            return Err(ModelError::BadSlot);
        }
        if fresh {
            self.undo(project);
            self.undone.pop();
            self.edits -= 1;
            if let Some(t) = self.evicted.take() {
                self.done.insert(0, t);
            }
            self.undone = std::mem::take(&mut self.cleared);
            return Ok(());
        }
        // The edit the other way round joins the take as the edit did, so
        // what it put in and took out cancels.
        let back = Edit {
            erase: edit.add.clone(),
            add: edit.erase.clone(),
            erase_locks: edit.add_locks.clone(),
            add_locks: edit.erase_locks.clone(),
            ..Edit::default()
        };
        self.edit(project, track, slot, &back, join).map(|_| ())
    }

    /// The takes that can be undone, oldest first.
    pub fn done(&self) -> &[Take] {
        &self.done
    }

    /// The takes that can be redone, the next one last.
    pub fn undone(&self) -> &[Take] {
        &self.undone
    }

    /// Free the pressure values clip `slot` of `track` holds that nothing
    /// can play again (refactor B4b): what none of its notes names, nor
    /// a note its Restore would bring back, nor a note an undo or redo
    /// would put back in this clip. Every run that is kept is renumbered,
    /// in the clip, its original and those takes, so undo and redo still
    /// find their notes. The values freed (0: nothing to free, or no such
    /// clip). The caller compiles the view again after.
    ///
    /// The takes that name this clip are the ones back to the take that
    /// put it in its slot (or added its track), and the ones redo would
    /// bring up to the one that would take it out or put another in:
    /// past those, a take's notes name another clip's pressure, kept with
    /// that clip in the take. What [`Takes::withdraw`] could put back is
    /// forgotten: it only ever follows the edit just made, and this never
    /// runs between the two. Allocates; for the control thread.
    pub fn reclaim_touch(&mut self, project: &mut Project, track: usize, slot: usize) -> usize {
        let Some(c) = clip(project, track, slot) else {
            return 0;
        };
        let original = c.original.iter().flat_map(|o| &o.notes);
        let mut runs: Vec<Touch> = c.notes.iter().chain(original).map(|n| n.touch).collect();
        self.each_note_of(track, slot, &mut |n| runs.push(n.touch));
        let keep = TouchKeep::of(c.touch.len(), runs);
        if keep.keeps_all() {
            return 0;
        }
        let freed = c.touch.len() - keep.kept();
        c.touch = keep.kept_of(&c.touch).collect();
        let original = c.original.iter_mut().flat_map(|o| o.notes.iter_mut());
        for n in c.notes.iter_mut().chain(original) {
            n.touch = keep.run(n.touch);
        }
        self.each_note_of(track, slot, &mut |n| n.touch = keep.run(n.touch));
        self.evicted = None;
        self.cleared.clear();
        freed
    }

    /// Each note an undo or a redo would add to or take from clip `slot`
    /// of `track` as it is now ([`Takes::reclaim_touch`]).
    fn each_note_of(&mut self, track: usize, slot: usize, f: &mut impl FnMut(&mut Note)) {
        for t in self.done.iter_mut().rev() {
            notes_in(t, track, slot, f);
            if swaps_clip(t, track, slot) {
                break;
            }
        }
        for t in self.undone.iter_mut().rev() {
            if swaps_clip(t, track, slot) {
                break;
            }
            notes_in(t, track, slot, f);
        }
    }

    /// Captures so far: the next one is number `captures() + 1`.
    pub fn captures(&self) -> u32 {
        self.captures
    }

    /// Edits so far: the next one is number `edits() + 1`.
    pub fn edits(&self) -> u32 {
        self.edits
    }

    /// Forget every take, for example when another project is loaded.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }

    /// The entry for recorded take `number`: an earlier one still being
    /// finished, or a new one, which ends redo.
    fn entry(&mut self, number: u32) -> &mut Take {
        if let Some(i) = self
            .done
            .iter()
            .rposition(|t| t.number == number && !t.captured && !t.edited)
        {
            return &mut self.done[i];
        }
        self.push(Take {
            number,
            added: Vec::with_capacity(TAKE_ROOM),
            erased: Vec::with_capacity(TAKE_ROOM),
            locks_added: Vec::with_capacity(TAKE_ROOM),
            locks_erased: Vec::with_capacity(TAKE_ROOM),
            ..Take::default()
        })
    }

    /// Run `f`, whose captures join capture `number` while it is the
    /// latest take and nothing is undone since: CAPTURE of several tracks'
    /// notes is one take, which one UNDO takes back. A capture that can't
    /// join is a take of its own, as outside `f`.
    pub fn joining<R>(&mut self, number: u32, f: impl FnOnce(&mut Takes) -> R) -> R {
        let was = self.joining.replace(number);
        let r = f(self);
        self.joining = was;
        r
    }

    /// A new capture, which ends redo, or the one being joined.
    fn next_capture(&mut self) -> &mut Take {
        let join = self.joining.filter(|&n| {
            self.undone.is_empty()
                && self
                    .done
                    .last()
                    .is_some_and(|t| t.captured && t.number == n)
        });
        if join.is_some() {
            return self.done.last_mut().expect("checked");
        }
        self.captures += 1;
        self.push(Take {
            number: self.captures,
            captured: true,
            ..Take::default()
        })
    }

    fn push(&mut self, take: Take) -> &mut Take {
        self.cleared = std::mem::take(&mut self.undone);
        self.evicted = None;
        if self.done.len() == self.limit {
            self.evicted = Some(self.done.remove(0));
        }
        self.done.push(take);
        self.done.last_mut().expect("just pushed")
    }
}

/// Write a gesture into clip `c`'s lane, keeping what it replaces.
fn write_move(
    c: &mut Clip,
    track: usize,
    slot: usize,
    target: Target,
    map: Map,
    points: &[(f64, f32, Kind)],
    len: u32,
) -> Result<MovePut, ModelError> {
    if i64::from(len) != c.length {
        return Err(ModelError::BadSlot);
    }
    let mut saved = Vec::new();
    c.moves
        .put_saving(target, map, points, len, |from, to, old| {
            saved.push((from, to, old.to_vec()))
        })
        .map_err(|_| ModelError::BadMoves)?;
    Ok(MovePut {
        track,
        slot,
        target,
        map,
        points: points.to_vec(),
        len,
        saved,
    })
}

/// Make a clip `from` ticks long `to` ticks long, unless it has changed
/// length since or holds a note past the new end.
fn resize(project: &mut Project, track: usize, slot: usize, from: Tick, to: Tick) {
    if let Some(c) = clip(project, track, slot) {
        let _ = c.apply(&Edit {
            length: Some((from, to)),
            ..Edit::default()
        });
    }
}

fn slot_mut(project: &mut Project, track: usize, slot: usize) -> Option<&mut Option<Clip>> {
    project.tracks.get_mut(track)?.slots.get_mut(slot)
}

fn clip(project: &mut Project, track: usize, slot: usize) -> Option<&mut Clip> {
    slot_mut(project, track, slot)?.as_mut()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Track, SLOTS};

    fn project() -> Project {
        let mut p = Project::new();
        let mut t = Track::new("t", 0);
        t.slots[0] = Some(Clip::new("c", 3840));
        p.tracks.push(t);
        p
    }

    fn taken(tick: i64, take: u32) -> Taken {
        Taken {
            track: 0,
            slot: 0,
            note: Note {
                tick,
                len: 10,
                note: 1,
                velocity: 100,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
                on: true,
                ramp: 0,
                join: mpfree_seq::Join::Attack,
            },
            take,
        }
    }

    fn ticks(p: &Project) -> Vec<i64> {
        p.tracks[0].slots[0]
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .map(|n| n.tick)
            .collect()
    }

    /// Refactor B4b: freeing a clip's unused pressure keeps every value a
    /// note, its Restore, an undo or a redo can still bring back, and
    /// renumbers them all alike; a take naming another clip's pressure is
    /// left as it is.
    #[test]
    fn reclaiming_pressure_keeps_what_undo_redo_and_restore_bring_back() {
        let run = |v: u8| -> Vec<TouchPoint> {
            (0..3)
                .map(|k| TouchPoint {
                    at: k * 5,
                    value: v + k as u8,
                })
                .collect()
        };
        let c = |p: &mut Project| p.tracks[0].slots[0].clone().unwrap();
        let mut p = project();
        let mut t = Takes::default();
        // Take 1: A, then pressure no note names (a note whose take fell
        // off the undo list), then B. Take 2: C, and A erased.
        let (a, _) = t.add_touched(&mut p, &taken(0, 1), &run(10)).unwrap();
        let orphan = c(&mut p).touch.len();
        clip(&mut p, 0, 0).unwrap().add_touch(&run(70)).unwrap();
        let (b, _) = t.add_touched(&mut p, &taken(100, 1), &run(20)).unwrap();
        let (cc, _) = t.add_touched(&mut p, &taken(200, 2), &run(30)).unwrap();
        let erased = Erased {
            track: 0,
            slot: 0,
            note: a,
            take: 2,
        };
        assert!(t.erase(&mut p, &erased));
        // Take 3: D, undone (redo brings it back).
        let (d, _) = t.add_touched(&mut p, &taken(300, 3), &run(40)).unwrap();
        assert_eq!(t.undo(&mut p), Some(3));
        // A note only Restore would bring back.
        let r = clip(&mut p, 0, 0).unwrap().add_touch(&run(50)).unwrap();
        let restored = Note {
            touch: r,
            ..taken(400, 0).note
        };
        clip(&mut p, 0, 0).unwrap().original = Some(Box::new(mpfree_seq::Original {
            length: 3840,
            notes: vec![restored],
            locks: Vec::new(),
            moves: mpfree_seq::mpfree_moves::Moves::new(),
            variation: 1,
        }));
        let before = c(&mut p);
        let values = |cl: &Clip, n: &Note| cl.touch_of(n).to_vec();
        assert_eq!(before.touch.len(), 18);
        assert_eq!(values(&before, &b), run(20));

        assert_eq!(t.reclaim_touch(&mut p, 0, 0), 3, "the orphan's three");
        let now = c(&mut p);
        assert_eq!(now.touch.len(), 15);
        assert!(!now.touch.iter().any(|x| x.value >= 70), "{orphan}");
        // What plays now plays the same.
        assert_eq!(now.notes.len(), before.notes.len());
        for (n, m) in now.notes.iter().zip(&before.notes) {
            assert_eq!(values(&now, n), values(&before, m));
        }
        let o = &now.original.as_ref().unwrap().notes[0];
        assert_eq!(values(&now, o), run(50), "Restore's note");
        // Redo brings D back with its pressure; undo of take 2 brings A
        // back and takes C out.
        assert_eq!(t.redo(&mut p), Some(3));
        let dn = *c(&mut p).notes.iter().find(|n| n.tick == 300).unwrap();
        assert_eq!(values(&c(&mut p), &dn), run(40));
        assert_eq!(t.undo(&mut p), Some(3));
        assert_eq!(t.undo(&mut p), Some(2));
        let now = c(&mut p);
        let an = now.notes.iter().find(|n| n.tick == 0).unwrap();
        assert_eq!(values(&now, an), run(10));
        assert!(!now.notes.iter().any(|n| n.tick == 200), "C taken out");
        assert_eq!(t.redo(&mut p), Some(2));
        assert_eq!(t.redo(&mut p), Some(3));
        assert_eq!(c(&mut p).notes.len(), 3, "B, C, D");
        let _ = (cc, d);
        // Nothing more to free.
        assert_eq!(t.reclaim_touch(&mut p, 0, 0), 0);

        // Another clip put in its place (a Mutate): the takes before it
        // name the old clip's pressure, and stay as they are.
        let mut new = Clip::new("new", 3840);
        new.add_touch(&run(90)).unwrap();
        let keep = new.add_touch(&run(100)).unwrap();
        new.add_note(Note {
            touch: keep,
            ..taken(0, 0).note
        })
        .unwrap();
        t.replace_clip(&mut p, 0, 0, new, Replaced::Mutate).unwrap();
        let old_takes: Vec<Take> = t.done()[..t.done().len() - 1].to_vec();
        assert_eq!(t.reclaim_touch(&mut p, 0, 0), 3);
        assert_eq!(&t.done()[..t.done().len() - 1], old_takes.as_slice());
        let now = c(&mut p);
        assert_eq!(values(&now, &now.notes[0]), run(100));
        // Undo puts the old clip back, and its takes still find their notes.
        assert!(t.undo(&mut p).is_some());
        assert!(t.undo(&mut p).is_some());
        let old = c(&mut p);
        assert!(!old.notes.iter().any(|n| n.tick == 300), "D out");
    }

    /// The clip settings card: a change is an edit of its own, a turn of
    /// one setting joins one, and undo and redo put the settings back
    /// while the clip still has the ones they were given.
    #[test]
    fn a_clips_settings_change_as_one_take_a_turn_joining_one() {
        use mpfree_seq::{Follow, FollowAction, Quantise, Speed};
        let mut p = project();
        let mut t = Takes::default();
        let c = |p: &Project| p.tracks[0].slots[0].as_ref().unwrap().settings();
        let first = c(&p);
        let mut half = first;
        half.speed = Speed::new(1, 2).unwrap();
        assert_eq!(t.set_clip(&mut p, 0, 0, half, Some(1)), Ok(1));
        assert_eq!(c(&p), half);
        // The same setting turned on: one take.
        let mut quarter = half;
        quarter.speed = Speed::new(1, 4).unwrap();
        assert_eq!(t.set_clip(&mut p, 0, 0, quarter, Some(1)), Ok(1));
        assert_eq!(t.done().len(), 1);
        // Another setting: another take.
        let mut q = quarter;
        q.quantise = Quantise::new(240, 60);
        q.follow = Some(Follow::after(4, FollowAction::Next));
        assert_eq!(t.set_clip(&mut p, 0, 0, q, Some(2)), Ok(2));
        // Nothing new: no take.
        assert_eq!(t.set_clip(&mut p, 0, 0, q, Some(3)), Ok(2));
        assert_eq!(t.done().len(), 2);
        // Out of range: refused, the clip as it was.
        let mut bad = q;
        bad.quantise = Some(Quantise {
            grid: 0,
            strength: 50,
        });
        assert_eq!(
            t.set_clip(&mut p, 0, 0, bad, None),
            Err(ModelError::BadQuantise)
        );
        assert_eq!(t.set_clip(&mut p, 0, 3, q, None), Err(ModelError::BadSlot));
        assert_eq!(c(&p), q);
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(c(&p), quarter);
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(c(&p), first);
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(c(&p), quarter);
        // Changed since by other means: redo leaves it.
        p.tracks[0].slots[0].as_mut().unwrap().speed = Speed::NORMAL;
        assert_eq!(t.redo(&mut p), Some(2));
        assert_eq!(c(&p).speed, Speed::NORMAL);
        assert_eq!(c(&p).quantise, None);
    }

    /// COPY + an empty slot: the copy goes in as an edit of its own, which
    /// undo takes away and redo puts back; a slot with a clip refuses it.
    #[test]
    fn a_pasted_clip_is_an_edit_undo_takes_away() {
        let mut p = project();
        let mut t = Takes::default();
        let copy = p.tracks[0].slots[0].clone().unwrap();
        assert_eq!(
            t.paste_clip(&mut p, 0, 0, copy.clone()),
            Err(ModelError::BadSlot)
        );
        assert_eq!(t.paste_clip(&mut p, 0, 5, copy.clone()), Ok(1));
        assert_eq!(p.tracks[0].slots[5].as_ref(), Some(&copy));
        assert!(t.done()[0].pasted && t.done()[0].edited);
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(p.tracks[0].slots[5].is_none());
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.tracks[0].slots[5].as_ref(), Some(&copy));
        assert_eq!(t.paste_clip(&mut p, 0, 99, copy), Err(ModelError::BadSlot));
    }

    /// The Sequencer's first step on an empty track: the new clip with
    /// its note is one edit. Undo takes it away whole, leaving the slot
    /// empty; redo puts it back, and the take says it was launched so
    /// redo can launch it again. A slot with a clip refuses it.
    #[test]
    fn a_first_step_clip_is_one_edit_undo_takes_away_whole() {
        use mpfree_seq::steps::Grid;
        let mut p = project();
        let mut t = Takes::default();
        let g = Grid::new(240).unwrap();
        let mut c = Clip::new("Pattern 2", 3840);
        let e = g.toggle(&c, 36, 0, 100);
        c.apply(&e).unwrap();
        assert_eq!(
            t.new_clip(&mut p, 0, 0, c.clone(), true),
            Err(ModelError::BadSlot)
        );
        assert!(t.done().is_empty(), "a refusal makes no take");
        assert_eq!(t.new_clip(&mut p, 0, 1, c.clone(), true), Ok(1));
        let take = &t.done()[0];
        assert!(take.edited && take.launched && !take.pasted);
        assert_eq!(p.tracks[0].slots[1].as_ref(), Some(&c));
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(
            p.tracks[0].slots[1].is_none(),
            "the clip goes with its note"
        );
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.tracks[0].slots[1].as_ref(), Some(&c));
        assert!(t.done()[0].launched);
    }

    /// An edit what plays could not follow is taken back whole: a take of
    /// its own goes, giving back the oldest take a full list let go and
    /// the takes redo could bring; a joining edit comes out of its take
    /// alone, the turn before it staying.
    #[test]
    fn a_withdrawn_edit_leaves_the_clip_and_the_takes_as_they_were() {
        use mpfree_seq::steps::{Change, Grid};
        let g = Grid::new(240).unwrap();
        let c = |p: &Project| p.tracks[0].slots[0].clone().unwrap();
        let mut p = project();
        // A full list (two takes): the third edit pushes the first out.
        let mut t = Takes::new(2);
        for step in [0, 4] {
            let e = g.toggle(&c(&p), 36, step, 100);
            t.edit(&mut p, 0, 0, &e, None).unwrap();
        }
        let before = (c(&p), t.done().to_vec(), t.edits());
        let e = g.toggle(&c(&p), 36, 8, 100);
        assert_eq!(t.edit(&mut p, 0, 0, &e, None), Ok(3));
        assert_eq!(t.done()[0].number, 2, "take 1 was pushed out");
        t.withdraw(&mut p, (0, 0), &e, None, true).unwrap();
        assert_eq!((c(&p), t.done().to_vec(), t.edits()), before);
        // With a take to redo: the edit ends redo, and taking it back
        // brings redo back.
        assert_eq!(t.undo(&mut p), Some(2));
        let before = (c(&p), t.done().to_vec(), t.undone().to_vec(), t.edits());
        let e = g.toggle(&c(&p), 36, 12, 100);
        assert_eq!(t.edit(&mut p, 0, 0, &e, None), Ok(3));
        assert!(t.undone().is_empty());
        t.withdraw(&mut p, (0, 0), &e, None, true).unwrap();
        let after = (c(&p), t.done().to_vec(), t.undone().to_vec(), t.edits());
        assert_eq!(after, before);
        assert_eq!(t.redo(&mut p), Some(2));
        assert_eq!(ticks(&p), vec![0, 960]);
        // A turn: 100 -> 90 is a take, 90 -> 80 joins it; taking the
        // second detent back leaves the take at 90.
        let mut p = project();
        let mut t = Takes::default();
        let tap = g.toggle(&c(&p), 36, 0, 100);
        t.edit(&mut p, 0, 0, &tap, None).unwrap();
        let key = Some(Change::Velocity(0).key(36, 0));
        let e = g.change(&c(&p), 36, 0, Change::Velocity(90));
        assert_eq!(t.edit(&mut p, 0, 0, &e, key), Ok(2));
        let before = (c(&p), t.done().to_vec(), t.edits());
        let e = g.change(&c(&p), 36, 0, Change::Velocity(80));
        assert_eq!(t.edit(&mut p, 0, 0, &e, key), Ok(2), "joins");
        t.withdraw(&mut p, (0, 0), &e, key, false).unwrap();
        assert_eq!((c(&p), t.done().to_vec(), t.edits()), before);
        assert_eq!(g.row(&c(&p), 36)[0].map(|s| s.velocity), Some(90));
        // Nothing to take back when the latest take is not an edit.
        t.add(&mut p, &taken(480, 1)).unwrap();
        assert_eq!(
            t.withdraw(&mut p, (0, 0), &e, key, false),
            Err(ModelError::BadSlot)
        );
    }

    /// The scene editor: a row's settings as an edit of their own; the
    /// same row turned again joins; undo and redo; the scenes after the
    /// last named one are left out; past the last row it refuses.
    #[test]
    fn a_scene_edit_is_a_take_and_undo_takes_it_back() {
        use mpfree_seq::{Follow, FollowAction, SceneSettings};
        let mut p = project();
        let mut t = Takes::default();
        let named = SceneSettings {
            name: "Drop".into(),
            ..SceneSettings::default()
        };
        let n = t.set_scene(&mut p, 2, named.clone(), Some(1)).unwrap();
        assert_eq!(p.scenes.len(), 3);
        assert_eq!(p.scene(2).unwrap().name, "Drop");
        let follow = |bars| SceneSettings {
            follow: Some(Follow::after(bars, FollowAction::Next)),
            ..named.clone()
        };
        let m = t.set_scene(&mut p, 2, follow(4), Some(2)).unwrap();
        assert_eq!(m, n + 1);
        assert_eq!(
            t.set_scene(&mut p, 2, follow(5), Some(2)).unwrap(),
            m,
            "joins"
        );
        assert_eq!(
            t.set_scene(&mut p, 2, follow(5), Some(2)).unwrap(),
            m,
            "no change"
        );
        t.undo(&mut p);
        assert_eq!(p.scene(2).unwrap().settings(), named);
        t.undo(&mut p);
        assert!(p.scenes.is_empty(), "a plain row at the end goes");
        t.redo(&mut p);
        t.redo(&mut p);
        assert_eq!(p.scene(2).unwrap().settings(), follow(5));
        assert_eq!(
            t.set_scene(&mut p, mpfree_seq::SCENES, named, None),
            Err(ModelError::BadScene)
        );
    }

    /// Song → scenes as one edit: every cell and scene it changed, so
    /// undo gives back the project exactly and redo the filled rows; a
    /// change of nothing is no take; a scene changed since is left alone.
    #[test]
    fn rows_filled_at_once_are_one_edit_and_undo_gives_the_project_back() {
        use mpfree_seq::{Follow, FollowAction};
        let mut p = project();
        p.tracks.push(Track::new("u", 1));
        p.tracks[1].slots[3] = Some(Clip::new("old", 3840));
        p.scene_mut(1).unwrap().name = "Verse".into();
        let mut t = Takes::default();
        let before = p.clone();
        assert_eq!(t.rows_edit(&before, &p), 0, "nothing changed");
        assert!(t.done().is_empty());
        // Two rows filled, one clip changed, one taken out.
        for row in [4, 5] {
            p.tracks[0].slots[row] = Some(Clip::new("a", 3840 * 2));
            p.tracks[1].slots[row] = Some(Clip::new("b", 3840));
            let s = p.scene_mut(row).unwrap();
            s.name = format!("Part {row}");
            s.follow = Some(Follow::after(2, FollowAction::Next));
        }
        p.tracks[0].slots[0] = Some(Clip::new("c2", 3840));
        p.tracks[1].slots[3] = None;
        let after = p.clone();
        let n = t.rows_edit(&before, &p);
        assert_eq!(n, 1);
        let e = t.done()[0].rows.as_deref().unwrap();
        assert_eq!(e.rows, [0, 3, 4, 5]);
        assert_eq!(e.scenes.len(), 2);
        assert_eq!(t.undo(&mut p), Some(n));
        assert_eq!(p, before, "undo gives the project back");
        assert_eq!(p.scenes.len(), 2, "the plain rows at the end go");
        assert_eq!(t.redo(&mut p), Some(n));
        assert_eq!(p, after, "redo fills the rows again");
        // A scene changed since stays as it is; the rest goes back.
        p.scene_mut(5).unwrap().name = "Mine".into();
        t.undo(&mut p);
        assert_eq!(p.scene(5).unwrap().name, "Mine");
        assert_eq!(whole_scene(&p, 4), Scene::default());
        assert!(p.tracks[0].slots[4].is_none() && p.tracks[1].slots[5].is_none());
        assert_eq!(p.tracks[1].slots[3], before.tracks[1].slots[3]);
    }

    /// Reload clip: the saved state goes in as an edit of its own; undo
    /// puts back the clip as it was, redo the saved state; an empty slot
    /// refuses it.
    #[test]
    fn a_reload_puts_the_saved_state_back_and_undo_takes_it_away() {
        let mut p = project();
        let mut t = Takes::default();
        let saved = p.tracks[0].slots[0].clone().unwrap();
        let mut mangled = saved.clone();
        mangled.notes.clear();
        p.tracks[0].slots[0] = Some(mangled.clone());
        assert_eq!(
            t.reload_clip(&mut p, 0, 5, saved.clone()),
            Err(ModelError::BadSlot)
        );
        assert!(p.tracks[0].slots[5].is_none());
        assert_eq!(t.reload_clip(&mut p, 0, 0, saved.clone()), Ok(1));
        assert_eq!(p.tracks[0].slots[0].as_ref(), Some(&saved));
        let take = &t.done()[0];
        assert!(take.replaced == Some(Replaced::Reload) && take.edited && !take.pasted);
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(p.tracks[0].slots[0].as_ref(), Some(&mangled));
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.tracks[0].slots[0].as_ref(), Some(&saved));
    }

    /// Mutate and Restore (S6) are takes of their own: undo puts back the
    /// clip before each, its original with it.
    #[test]
    fn mutate_and_restore_are_takes_undo_takes_back() {
        use mpfree_seq::{Amount, Pitches};
        let mut p = project();
        let mut t = Takes::default();
        let c = p.tracks[0].slots[0].as_mut().unwrap();
        c.add_note(Note::new(0, 240, 36, 100)).unwrap();
        let plain = c.clone();
        let v = plain.mutated(4, Amount::Little, Pitches::Pads).unwrap();
        assert_eq!(
            t.replace_clip(&mut p, 0, 0, v.clone(), Replaced::Mutate),
            Ok(1)
        );
        let r = v.restored().unwrap();
        assert_eq!(
            t.replace_clip(&mut p, 0, 0, r.clone(), Replaced::Restore),
            Ok(2)
        );
        assert_eq!(p.tracks[0].slots[0].as_ref(), Some(&plain));
        assert_eq!(t.done()[1].replaced, Some(Replaced::Restore));
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(p.tracks[0].slots[0].as_ref(), Some(&v));
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(p.tracks[0].slots[0].as_ref(), Some(&plain));
        let mut broken = v.clone();
        broken.original.as_mut().unwrap().length = 0;
        assert_eq!(
            t.replace_clip(&mut p, 0, 0, broken, Replaced::Mutate),
            Err(ModelError::ClipLength)
        );
        assert_eq!(p.tracks[0].slots[0].as_ref(), Some(&plain));
    }

    #[test]
    fn a_song_edit_undoes_and_redoes_unless_the_song_changed_since() {
        use mpfree_seq::arrange::Placement;
        let mut p = project();
        p.arrangement
            .place(Placement::linked(0, 0, 0, 3840))
            .unwrap();
        let mut t = Takes::default();
        let before = p.arrangement.clone();
        p.arrangement
            .move_placement(&p.tracks, 0, 0, 7680, false)
            .unwrap();
        let moved = p.arrangement.clone();
        let change = SongChange::Moved {
            track: 0,
            start: 7680,
        };
        assert_eq!(t.song_edit(&p, change, before.clone()), 1);
        assert_eq!(t.done()[0].song.as_ref().unwrap().change, change);
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(p.arrangement, before);
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.arrangement, moved);
        // Something else changes the song (a marker, say): undo leaves it.
        p.arrangement.mark(0, "A").unwrap();
        let now = p.arrangement.clone();
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(p.arrangement, now);
    }

    #[test]
    fn a_tempo_point_turned_on_the_wheel_is_one_undo() {
        let mut p = project();
        let mut t = Takes::default();
        let start = p.arrangement.clone();
        let change = SongChange::Tempo { at: 3840 };
        let turn = |p: &mut Project, t: &mut Takes, bpm: f64, join| {
            let before = p.arrangement.clone();
            p.arrangement.set_tempo(3840, bpm, None).unwrap();
            t.song_edit_joining(p, change, before, join)
        };
        assert_eq!(turn(&mut p, &mut t, 100.0, Some(7)), 1);
        assert_eq!(turn(&mut p, &mut t, 101.0, Some(7)), 1);
        assert_eq!(turn(&mut p, &mut t, 102.0, Some(7)), 1);
        assert_eq!(t.done().len(), 1);
        // Another key, or none, is an edit of its own.
        assert_eq!(turn(&mut p, &mut t, 103.0, Some(8)), 2);
        assert_eq!(turn(&mut p, &mut t, 104.0, None), 3);
        assert_eq!(t.undo(&mut p), Some(3));
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(p.arrangement.tempo[0].bpm, 102.0);
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(p.arrangement, start);
        // After an undo, a turn starts a new edit.
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(turn(&mut p, &mut t, 90.0, Some(7)), 4);
        // Something else changed the song since: no join.
        p.arrangement.mark(0, "A").unwrap();
        assert_eq!(turn(&mut p, &mut t, 91.0, Some(7)), 5);
    }

    #[test]
    fn an_added_track_undoes_after_its_own_takes_and_redoes_with_them() {
        let mut p = project();
        p.tracks.push(Track::new("t2", 1));
        let mut t = Takes::default();
        assert_eq!(t.add_track(&mut p, "Track 3"), Ok((1, 2)));
        assert_eq!((p.tracks.len(), p.tracks[2].dest), (3, 2));
        // A clip recorded on it, then its name changed (not a take).
        let mut c = Clip::new("new", 3840);
        c.notes.push(taken(0, 9).note);
        p.tracks[2].slots[4] = Some(c);
        p.tracks[2].name = "Keys".into();
        t.remove_clip(&mut p, 2, 4).unwrap();
        assert_eq!(t.undo(&mut p), Some(2), "the clip first");
        assert!(p.tracks[2].slots[4].is_some());
        assert_eq!(t.undo(&mut p), Some(1), "then the track");
        assert_eq!(p.tracks.len(), 2);
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.tracks[2].name, "Keys", "back as it went");
        assert!(p.tracks[2].slots[4].is_some());
        // A destination no track uses: a gap is filled first.
        p.tracks[0].dest = 7;
        assert_eq!(t.add_track(&mut p, "Track 4").map(|r| r.1), Ok(3));
        assert_eq!(p.tracks[3].dest, 0);
        // 128 at most.
        while p.tracks.len() < MAX_TRACKS {
            t.add_track(&mut p, "more").unwrap();
        }
        assert_eq!(
            t.add_track(&mut p, "one too many"),
            Err(ModelError::TooManyTracks)
        );
        p.check().unwrap();
    }

    /// Instrument redesign step 2c: an added track gets the project's next
    /// lasting id, UNDO and REDO take it away and bring it back with it,
    /// and a track added after the UNDO gets a new one, never its id.
    #[test]
    fn an_added_track_has_a_lasting_id_that_undo_and_redo_keep() {
        use mpfree_seq::TrackId;
        let mut p = project();
        p.tracks.push(Track::new("t2", 1));
        assert_eq!(p.give_track_ids(), p.tracks.len());
        let mut t = Takes::default();
        let (_, at) = t.add_track(&mut p, "Track 3").unwrap();
        let id = p.tracks[at].id;
        assert_eq!(id, TrackId(at as u32 + 1));
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.tracks[at].id, id, "the same track back");
        assert_eq!(t.undo(&mut p), Some(1));
        let (_, again) = t.add_track(&mut p, "Track 3").unwrap();
        assert_eq!(again, at);
        assert_eq!(p.tracks[again].id, TrackId(id.0 + 1), "a new id, not its");
        assert_eq!(p.track_with(TrackId(id.0 + 1)), Some(again));
        assert_eq!(p.track_with(id), None);
    }

    #[test]
    fn a_new_take_ends_redo() {
        let mut p = project();
        let mut t = Takes::default();
        t.add(&mut p, &taken(0, 1)).unwrap();
        t.add(&mut p, &taken(100, 2)).unwrap();
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(t.undone().len(), 1);
        t.add(&mut p, &taken(200, 3)).unwrap();
        assert!(t.undone().is_empty());
        assert_eq!(t.redo(&mut p), None);
        assert_eq!(ticks(&p), vec![0, 200]);
    }

    #[test]
    fn old_takes_are_forgotten_past_the_limit() {
        let mut p = project();
        let mut t = Takes::new(2);
        for take in 1..=3 {
            t.add(&mut p, &taken(i64::from(take) * 100, take)).unwrap();
        }
        let kept: Vec<u32> = t.done().iter().map(|t| t.number).collect();
        assert_eq!(kept, vec![2, 3]);
        assert_eq!(t.undo(&mut p), Some(3));
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(t.undo(&mut p), None);
        assert_eq!(ticks(&p), vec![100], "take 1 stays");
    }

    #[test]
    fn a_removed_clip_comes_back_with_undo_and_goes_again_with_redo() {
        let mut p = project();
        let mut t = Takes::default();
        t.add(&mut p, &taken(0, 1)).unwrap();
        let before = p.clip(0, 0).cloned().unwrap();
        assert_eq!(t.remove_clip(&mut p, 0, 0), Ok(1));
        assert_eq!(p.clip(0, 0), None);
        assert_eq!(t.remove_clip(&mut p, 0, 0), Err(ModelError::BadSlot));
        assert_eq!(t.remove_clip(&mut p, 0, 5), Err(ModelError::BadSlot));
        assert_eq!(t.done().len(), 2, "the removal is a take of its own");
        assert!(t.done()[1].edited);
        assert_eq!(t.undo(&mut p), Some(1));
        assert_eq!(p.clip(0, 0), Some(&before), "back with its notes");
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.clip(0, 0), None);
        // A slot filled meanwhile keeps its clip on undo.
        p.tracks[0].slots[0] = Some(Clip::new("new", 3840));
        t.undo(&mut p);
        assert_eq!(p.clip(0, 0).unwrap().name, "new");
    }

    #[test]
    fn captures_joining_one_are_one_take_that_one_undo_takes_back() {
        let mut p = project();
        p.tracks.push(Track::new("u", 1));
        let mut t = Takes::default();
        let c = |name: &str| Clip::new(name, 3840);
        let first = t.capture_clip(&mut p, 0, 1, c("Capture 1")).unwrap();
        let joined = t.joining(first, |t| t.capture_clip(&mut p, 1, 0, c("Capture 1")));
        assert_eq!(joined, Ok(first));
        assert_eq!(t.captures(), 1);
        assert_eq!(t.done().len(), 1);
        assert_eq!(t.done()[0].made.len(), 2);
        // Outside it, a capture is a take of its own again.
        assert_eq!(t.capture_clip(&mut p, 0, 2, c("Capture 2")), Ok(2));
        assert_eq!(t.undo(&mut p), Some(2));
        // One UNDO takes both tracks' clips away, one REDO puts them back.
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(p.clip(0, 1).is_none() && p.clip(1, 0).is_none());
        assert_eq!(t.redo(&mut p), Some(1));
        assert!(p.clip(0, 1).is_some() && p.clip(1, 0).is_some());
        // A capture can't join one that isn't the latest take.
        t.add(&mut p, &taken(0, 7)).unwrap();
        let alone = t.joining(1, |t| t.capture_clip(&mut p, 1, 1, c("Capture 3")));
        assert_eq!(alone, Ok(3));
        // Nor one with a take undone since.
        assert_eq!(t.undo(&mut p), Some(3));
        let alone = t.joining(1, |t| t.capture_clip(&mut p, 1, 1, c("Capture 4")));
        assert_eq!(alone, Ok(4));
    }

    #[test]
    fn a_captured_clip_is_undone_and_redone_as_a_take() {
        let mut p = project();
        let mut t = Takes::default();
        t.add(&mut p, &taken(0, 1)).unwrap();
        let mut c = Clip::new("Capture 1", 3840);
        c.add_note(Note {
            tick: 240,
            len: 10,
            note: 4,
            velocity: 90,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
            touch: mpfree_seq::Touch::NONE,
            on: true,
            ramp: 0,
            join: mpfree_seq::Join::Attack,
        })
        .unwrap();
        assert_eq!(t.capture_clip(&mut p, 0, 1, c.clone()), Ok(1));
        assert_eq!(p.clip(0, 1), Some(&c));
        // Recorded take 1 goes on: its notes still join take 1, not the
        // capture, though both are number 1.
        t.add(&mut p, &taken(100, 1)).unwrap();
        assert_eq!(
            t.done()
                .iter()
                .map(|t| (t.number, t.captured, t.added.len()))
                .collect::<Vec<_>>(),
            vec![(1, false, 2), (1, true, 0)]
        );
        // Notes recorded into the captured clip are kept with it on undo.
        p.tracks[0].slots[1]
            .as_mut()
            .unwrap()
            .add_note(Note {
                tick: 0,
                len: 5,
                note: 1,
                velocity: 1,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
                on: true,
                ramp: 0,
                join: mpfree_seq::Join::Attack,
            })
            .unwrap();
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(p.clip(0, 1).is_none());
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.clip(0, 1).map(|c| c.notes.len()), Some(2));
        // Undone again, and the slot filled meanwhile: redo leaves it.
        t.undo(&mut p);
        p.tracks[0].slots[1] = Some(Clip::new("mine", 960));
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.clip(0, 1).map(|c| c.name.as_str()), Some("mine"));
        // The second capture is number 2; a full slot is refused.
        assert_eq!(
            t.capture_clip(&mut p, 0, 1, c.clone()),
            Err(ModelError::BadSlot)
        );
        assert_eq!(t.capture_clip(&mut p, 0, 2, c.clone()), Ok(2));
        assert_eq!(
            t.capture_clip(&mut p, 0, SLOTS, c),
            Err(ModelError::BadSlot)
        );
    }

    #[test]
    fn clips_captured_together_are_one_take_or_none_go_in() {
        let mut p = Project::new();
        p.tracks.push(Track::new("t1", 0));
        p.tracks.push(Track::new("t2", 1));
        let mut t = Takes::default();
        let c = |name: &str| Clip::new(name, 3840);
        p.tracks[1].slots[0] = Some(c("taken"));
        // A full slot, a slot named twice, a track past the end, a bad
        // clip or nothing: refused, and nothing goes in.
        for bad in [
            vec![(0, 0, c("a")), (1, 0, c("b"))],
            vec![(0, 0, c("a")), (0, 0, c("b"))],
            vec![(0, 0, c("a")), (p.tracks.len(), 0, c("b"))],
            vec![(0, 0, c("a")), (1, 1, Clip::new("short", 1))],
            vec![],
        ] {
            assert!(t.capture_clips(&mut p, bad).is_err());
            assert!(p.clip(0, 0).is_none() && p.clip(1, 1).is_none());
            assert_eq!(p.clip(1, 0).map(|c| c.name.as_str()), Some("taken"));
        }
        assert!(t.done().is_empty());
        let made = vec![(0, 0, c("Drums")), (1, 1, c("Bass"))];
        assert_eq!(t.capture_clips(&mut p, made), Ok(1));
        assert_eq!(p.clip(0, 0).map(|c| c.name.as_str()), Some("Drums"));
        assert_eq!(p.clip(1, 1).map(|c| c.name.as_str()), Some("Bass"));
        assert_eq!(t.done().len(), 1);
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(p.clip(0, 0).is_none() && p.clip(1, 1).is_none());
        assert_eq!(p.clip(1, 0).map(|c| c.name.as_str()), Some("taken"));
        assert_eq!(t.redo(&mut p), Some(1));
        assert!(p.clip(0, 0).is_some() && p.clip(1, 1).is_some());
    }

    #[test]
    fn captured_notes_go_in_together_or_not_at_all() {
        let mut p = project();
        let mut t = Takes::default();
        let n = |tick| Note {
            tick,
            len: 10,
            note: 2,
            velocity: 100,
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
            touch: mpfree_seq::Touch::NONE,
            on: true,
            ramp: 0,
            join: mpfree_seq::Join::Attack,
        };
        let pressed = |tick, first| Note {
            touch: Touch { first, len: 2 },
            ..n(tick)
        };
        let point = |at, value| TouchPoint { at, value };
        let touch = [point(0, 50), point(5, 70), point(0, 20), point(3, 10)];
        assert_eq!(
            t.capture_notes(&mut p, 0, 0, &[n(0), pressed(5000, 0)], &touch[..2]),
            Err(ModelError::BadNote(1))
        );
        let c = p.clip(0, 0).unwrap();
        assert!(ticks(&p).is_empty() && c.touch.is_empty() && t.done().is_empty());
        assert_eq!(
            t.capture_notes(&mut p, 0, 3, &[n(0)], &[]),
            Err(ModelError::BadSlot)
        );
        assert_eq!(
            t.capture_notes(&mut p, 0, 0, &[n(0), n(960)], &[]),
            Ok((1, 0))
        );
        assert_eq!(ticks(&p), vec![0, 960]);
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(ticks(&p).is_empty());
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(ticks(&p), vec![0, 960]);
        // With pressure: it goes after the clip's own, and the notes name
        // it there.
        clip(&mut p, 0, 0).unwrap().touch.push(point(0, 1));
        assert_eq!(
            t.capture_notes(&mut p, 0, 0, &[pressed(100, 0), pressed(200, 2)], &touch),
            Ok((2, 0))
        );
        let c = p.clip(0, 0).unwrap();
        assert_eq!(c.touch_of(&c.notes[2]), &touch[..2]);
        assert_eq!(c.touch_of(&c.notes[3]), &touch[2..]);
        // A clip with no room left: the notes go in without it, counted.
        let c = clip(&mut p, 0, 0).unwrap();
        c.touch
            .resize(mpfree_seq::MAX_TOUCH_POINTS - 3, point(0, 0));
        assert_eq!(
            t.capture_notes(&mut p, 0, 0, &[pressed(300, 0), pressed(400, 2)], &touch),
            Ok((3, 4))
        );
        let c = p.clip(0, 0).unwrap();
        assert_eq!(c.touch.len(), mpfree_seq::MAX_TOUCH_POINTS - 3);
        assert!(c.notes[4..].iter().all(|n| n.touch.is_none()));
    }

    #[test]
    fn missing_clips_and_notes_change_nothing() {
        let mut p = project();
        let mut t = Takes::default();
        let gone = Taken {
            slot: 5,
            ..taken(0, 1)
        };
        assert_eq!(t.add(&mut p, &gone), Err(ModelError::BadSlot));
        let e = Erased {
            track: 0,
            slot: 0,
            note: taken(0, 1).note,
            take: 1,
        };
        assert!(!t.erase(&mut p, &e), "the clip does not hold it");
        assert!(t.done().is_empty());
        // A note out of the clip's range is refused, and not remembered.
        assert!(t.add(&mut p, &taken(5000, 1)).is_err());
        assert!(t.done().is_empty());
    }

    #[test]
    fn step_edits_are_takes_and_a_turn_joins_one() {
        use mpfree_seq::steps::{Change, Grid};
        let mut p = project();
        let mut t = Takes::default();
        let g = Grid::new(240).unwrap();
        let c = |p: &Project| p.tracks[0].slots[0].clone().unwrap();
        // Work out an edit from the clip as it is, and apply it.
        fn step(
            t: &mut Takes,
            p: &mut Project,
            f: impl Fn(&Clip) -> Edit,
            join: Option<u64>,
        ) -> Result<u32, ModelError> {
            let e = f(p.tracks[0].slots[0].as_ref().unwrap());
            t.edit(p, 0, 0, &e, join)
        }
        // Two steps tapped on: two edits.
        assert_eq!(
            step(&mut t, &mut p, |c| g.toggle(c, 36, 0, 100), None),
            Ok(1)
        );
        assert_eq!(
            step(&mut t, &mut p, |c| g.toggle(c, 36, 4, 100), None),
            Ok(2)
        );
        // A velocity turned 100 -> 90 -> 80 -> 70 on step 4 joins one edit.
        for v in [90, 80, 70] {
            assert_eq!(
                step(
                    &mut t,
                    &mut p,
                    |c| g.change(c, 36, 4, Change::Velocity(v)),
                    Some(Change::Velocity(v).key(36, 4))
                ),
                Ok(3)
            );
        }
        let last = t.done().last().unwrap();
        assert!(last.edited && !last.captured);
        assert_eq!(
            (last.erased.len(), last.added.len()),
            (1, 1),
            "only the first and last values"
        );
        assert_eq!(g.row(&c(&p), 36)[4].map(|s| s.velocity), Some(70));
        // One undo takes the whole turn back; another the second step.
        assert_eq!(t.undo(&mut p), Some(3));
        assert_eq!(g.row(&c(&p), 36)[4].map(|s| s.velocity), Some(100));
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(ticks(&p), vec![0]);
        // A joining edit after an undo starts a new edit.
        assert_eq!(
            step(
                &mut t,
                &mut p,
                |c| g.change(c, 36, 0, Change::Chance(50)),
                Some(Change::Chance(50).key(36, 0))
            ),
            Ok(4)
        );
        assert!(t.undone().is_empty());
        // A recorded note does not join an edit, and an edit never joins a take.
        t.add(&mut p, &taken(480, 4)).unwrap();
        assert_eq!(t.done().len(), 3, "take 4 is its own");
        assert_eq!(
            step(
                &mut t,
                &mut p,
                |c| g.change(c, 36, 0, Change::Chance(60)),
                Some(Change::Chance(60).key(36, 0))
            ),
            Ok(5)
        );
        // Refused edits change nothing.
        let before = c(&p);
        assert_eq!(
            t.edit(&mut p, 0, 3, &g.toggle(&before, 36, 1, 100), None),
            Err(ModelError::BadSlot)
        );
        let stale = g.toggle(&before, 36, 4, 100);
        t.edit(&mut p, 0, 0, &stale, None).unwrap();
        assert_eq!(
            t.edit(
                &mut p,
                0,
                0,
                &Edit {
                    erase: stale.add.clone(),
                    ..Edit::default()
                },
                None
            ),
            Ok(7)
        );
        assert_eq!(
            t.edit(
                &mut p,
                0,
                0,
                &Edit {
                    erase: stale.add.clone(),
                    ..Edit::default()
                },
                None
            ),
            Err(ModelError::MissingNote)
        );
        assert_eq!(t.edits(), 7);
    }

    #[test]
    fn pages_added_and_taken_off_undo_and_redo() {
        use mpfree_seq::steps::Grid;
        let mut p = project();
        let mut t = Takes::default();
        let g = Grid::new(240).unwrap();
        // The clip, its notes in order (a clip keeps them as written).
        let c = |p: &Project| {
            let mut c = p.tracks[0].slots[0].clone().unwrap();
            c.notes.sort_by_key(|n| (n.tick, n.note));
            c
        };
        for i in [0, 6, 15] {
            let e = g.toggle(&c(&p), 36, i, 100);
            t.edit(&mut p, 0, 0, &e, None).unwrap();
        }
        // A step early on step 0, so it moves with the clip's end.
        let e = g.change(&c(&p), 36, 0, mpfree_seq::steps::Change::Nudge(-40));
        t.edit(&mut p, 0, 0, &e, None).unwrap();
        let one_page = c(&p);
        // A copied page, then another, as two edits.
        let e = g.add_page(&c(&p), true);
        assert_eq!(t.edit(&mut p, 0, 0, &e, None), Ok(5));
        let two_pages = c(&p);
        let e = g.add_page(&c(&p), true);
        t.edit(&mut p, 0, 0, &e, None).unwrap();
        assert_eq!((c(&p).length, c(&p).notes.len()), (3 * 3840, 9));
        assert_eq!(
            t.done().last().unwrap().resized,
            vec![(0, 0, 2 * 3840, 3 * 3840)]
        );
        // Undo goes back a page at a time, notes and length exactly.
        t.undo(&mut p);
        assert_eq!(c(&p), two_pages);
        t.undo(&mut p);
        assert_eq!(c(&p), one_page);
        t.redo(&mut p);
        assert_eq!(c(&p), two_pages);
        // Taking the page off again is an edit too, and undoes.
        let e = g.remove_page(&c(&p));
        t.edit(&mut p, 0, 0, &e, None).unwrap();
        assert_eq!(c(&p), one_page);
        t.undo(&mut p);
        assert_eq!(c(&p), two_pages);
        // A length change never joins a turn.
        let e = g.add_page(&c(&p), false);
        let n = t.edits();
        assert_eq!(t.edit(&mut p, 0, 0, &e, Some(1)), Ok(n + 1));
    }

    #[test]
    fn locks_undo_and_redo_with_their_steps() {
        use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::steps::{lock_key, Grid};
        let cut = RelAddr::new(OnTrack::Inst, 3);
        let mut p = project();
        let mut t = Takes::default();
        let g = Grid::new(240).unwrap();
        let c = |p: &Project| p.tracks[0].slots[0].clone().unwrap();
        let tap = g.toggle(&c(&p), 36, 2, 100);
        t.edit(&mut p, 0, 0, &tap, None).unwrap();
        // A Q-Link turned on held step 2: one undo for the whole turn.
        let key = Some(lock_key(Some(36), 2, cut));
        for v in [0.2, 0.4, 0.6] {
            let e = g.lock(&c(&p), Some(36), 2, cut, v);
            assert_eq!(t.edit(&mut p, 0, 0, &e, key), Ok(2));
        }
        let last = t.done().last().unwrap();
        assert_eq!((last.locks_erased.len(), last.locks_added.len()), (0, 1));
        assert_eq!(c(&p).locks.len(), 1);
        assert_eq!(c(&p).locks[0].value, 0.6);
        // Tapping the step off takes its lock; undo brings both back.
        let off = g.toggle(&c(&p), 36, 2, 100);
        t.edit(&mut p, 0, 0, &off, None).unwrap();
        assert!(c(&p).locks.is_empty());
        assert_eq!(t.undo(&mut p), Some(3));
        assert_eq!(
            (c(&p).locks.len(), g.row(&c(&p), 36)[2].is_some()),
            (1, true)
        );
        // Undo the turn, then redo it.
        assert_eq!(t.undo(&mut p), Some(2));
        assert!(c(&p).locks.is_empty());
        assert_eq!(t.redo(&mut p), Some(2));
        assert_eq!(c(&p).locks[0].value, 0.6);
        // A page with a lock taken off and put back.
        let two = g.add_page(&c(&p), true);
        t.edit(&mut p, 0, 0, &two, None).unwrap();
        assert_eq!(c(&p).locks.len(), 2);
        let one = g.remove_page(&c(&p));
        t.edit(&mut p, 0, 0, &one, None).unwrap();
        assert_eq!(c(&p).locks.len(), 1);
        t.undo(&mut p).unwrap();
        assert_eq!(c(&p).locks.len(), 2, "the lock on step 18 comes back");
        assert_eq!(g.locks(&c(&p), Some(36), 18).len(), 1);
    }
}
