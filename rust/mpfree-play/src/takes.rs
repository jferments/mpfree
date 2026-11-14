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

use mpfree_seq::mpfree_moves::{Kind, Map, Point, Target};
use mpfree_seq::steps::{Edit, MovesEdit};
use mpfree_seq::{
    touch_room, Clip, DestId, Lock, ModelError, Note, Project, Touch, TouchPoint, Track, MAX_TRACKS,
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

/// Takes kept for undo when no limit is given.
pub const DEFAULT_LIMIT: usize = 32;

/// Notes and locks a recorded take has room for in each of its lists
/// before adding one allocates.
pub const TAKE_ROOM: usize = 256;

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
    /// the note keeps it.
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
        if project.tracks.len() >= MAX_TRACKS {
            return Err(ModelError::TooManyTracks);
        }
        let dest = (0..=MAX_TRACKS as DestId)
            .find(|d| project.tracks.iter().all(|t| t.dest != *d))
            .unwrap_or(project.tracks.len() as DestId);
        let index = project.tracks.len();
        let track = Track::new(name, dest);
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
                t.edited && t.join == join && t.resized.is_empty() && same_clip(t)
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

    /// The takes that can be undone, oldest first.
    pub fn done(&self) -> &[Take] {
        &self.done
    }

    /// The takes that can be redone, the next one last.
    pub fn undone(&self) -> &[Take] {
        &self.undone
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

    /// A new capture, which ends redo.
    fn next_capture(&mut self) -> &mut Take {
        self.captures += 1;
        self.push(Take {
            number: self.captures,
            captured: true,
            ..Take::default()
        })
    }

    fn push(&mut self, take: Take) -> &mut Take {
        self.undone.clear();
        if self.done.len() == self.limit {
            self.done.remove(0);
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
