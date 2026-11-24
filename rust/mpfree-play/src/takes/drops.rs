//! Drops on the Clips grid (Jay, 14 Nov: drag and drop): a clip moved to
//! an empty slot, two clips swapped, a clip copied, and a whole row moved
//! or copied, each as one take that undo takes back.
//!
//! A clip that moves keeps its lasting id (`mpfree_seq::ClipId`), so the
//! song's blocks that play it are untouched and play it on, on their own
//! lanes (the Arrange refactor's 1.3); a copy is a clip of its own, with
//! a new id. When a row moves, the rows' scenes, the morph's rows and the
//! song list's steps, which name rows, go with it. Undo and redo work
//! them out again from the drop ([`ClipDrop::after`] and
//! [`ClipDrop::before`]).

use mpfree_seq::{ModelError, Project, Scene, ROWS, SCENES};

use super::{slot_mut, Take, Takes};

/// A place on the grid: (track, slot).
pub type Cell = (usize, usize);

/// What a drop on the Clips grid did, as its take keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipDrop {
    /// The clip at the first place went to the second, which was empty.
    Moved(Cell, Cell),
    /// The clips at the two places changed places.
    Swapped(Cell, Cell),
    /// A copy of the clip at the first place went into the second, which
    /// was empty.
    Copied(Cell, Cell),
    /// Row `from`, every track's clip in it and its scene, went in before
    /// row `to` (`to` is a gap, 0 to [`ROWS`]); the rows between moved
    /// up or down one.
    RowMoved { from: usize, to: usize },
    /// A copy of row `from` went in before row `to`, and the rows from
    /// `to` on moved down one (the grid's last row was empty).
    RowCopied { from: usize, to: usize },
}

impl ClipDrop {
    /// Where the clip that was at `at` is after the drop.
    pub fn after(&self, (t, s): Cell) -> Cell {
        match *self {
            ClipDrop::Moved(a, b) | ClipDrop::Swapped(a, b) if (t, s) == a => b,
            ClipDrop::Swapped(a, b) if (t, s) == b => a,
            ClipDrop::RowMoved { from, to } => (t, moved(s, from, landing(from, to))),
            // The rows from `to` move down one; the last row was empty
            // (scene banks, the Arrange refactor's 5.6: all 64 rows).
            ClipDrop::RowCopied { to, .. } if s >= to && s + 1 < ROWS => (t, s + 1),
            _ => (t, s),
        }
    }

    /// Where the clip at `at` was before the drop: None for the copy the
    /// drop made.
    pub fn before(&self, (t, s): Cell) -> Option<Cell> {
        match *self {
            ClipDrop::Moved(a, b) | ClipDrop::Swapped(a, b) if (t, s) == b => Some(a),
            ClipDrop::Swapped(a, b) if (t, s) == a => Some(b),
            ClipDrop::Copied(_, b) if (t, s) == b => None,
            ClipDrop::RowMoved { from, to } => Some((t, moved(s, landing(from, to), from))),
            ClipDrop::RowCopied { to, .. } if s == to => None,
            ClipDrop::RowCopied { to, .. } if s > to && s < ROWS => Some((t, s - 1)),
            _ => Some((t, s)),
        }
    }

    /// A whole row moved or was copied.
    pub fn is_row(&self) -> bool {
        matches!(self, ClipDrop::RowMoved { .. } | ClipDrop::RowCopied { .. })
    }
}

/// The row a moved row lands on, dropped in gap `to`.
fn landing(from: usize, to: usize) -> usize {
    if to > from {
        to - 1
    } else {
        to
    }
}

/// Where item `r` is after the item at `a` moves to index `b`.
fn moved(r: usize, a: usize, b: usize) -> usize {
    if r == a {
        b
    } else if a < r && r <= b {
        r - 1
    } else if b <= r && r < a {
        r + 1
    } else {
        r
    }
}

impl Takes {
    /// A clip dragged on the Clips grid: the clip at `from` to slot `to`.
    /// Into an empty slot it moves, or with `copy` a copy goes there;
    /// onto another clip the two change places. Its number and what it
    /// did. Nothing changes on an error: `BadSlot` with no clip at
    /// `from`, or `to` the same place or not a slot; `NoRoom` for a copy
    /// onto a clip.
    pub fn drop_clip(
        &mut self,
        project: &mut Project,
        from: Cell,
        to: Cell,
        copy: bool,
    ) -> Result<(u32, ClipDrop), ModelError> {
        if from == to {
            return Err(ModelError::BadSlot);
        }
        let a = project
            .clip(from.0, from.1)
            .cloned()
            .ok_or(ModelError::BadSlot)?;
        let there = slot_mut(project, to.0, to.1)
            .ok_or(ModelError::BadSlot)?
            .clone();
        let drop = match (&there, copy) {
            (None, false) => ClipDrop::Moved(from, to),
            (None, true) => ClipDrop::Copied(from, to),
            (Some(_), false) => ClipDrop::Swapped(from, to),
            (Some(_), true) => return Err(ModelError::NoRoom),
        };
        let mut take = Take {
            edited: true,
            dropped: Some(drop),
            ..Take::default()
        };
        match (drop, there) {
            // A copy is a clip of its own, with a new id; a move or a swap
            // keeps each clip's.
            (ClipDrop::Copied(..), _) => {
                let mut copy = a;
                copy.id = project.new_clip_id();
                take.made.push((to.0, to.1, copy));
            }
            (_, None) => {
                take.removed.push((from.0, from.1, a.clone()));
                take.made.push((to.0, to.1, a));
            }
            (_, Some(b)) => {
                take.removed.push((from.0, from.1, a.clone()));
                take.removed.push((to.0, to.1, b.clone()));
                take.made.push((from.0, from.1, b));
                take.made.push((to.0, to.1, a));
            }
        }
        Ok((self.put_drop(project, take), drop))
    }

    /// A row's number dragged on the Clips grid: row `from` (every
    /// track's clip in it, and its scene) goes in before row `to`, a gap
    /// from 0 to [`ROWS`]. Moved, the rows between move up or down one;
    /// with `copy`, a copy goes in and the rows from `to` on move down
    /// one, so the last row must be empty. Nothing is written over. Its
    /// number and what it did. Nothing changes on an error: `BadSlot`
    /// for a row out of range, a copy after the last row or a move that
    /// lands where it started;
    /// `NoRoom` for a copy with the last row in use.
    pub fn drop_row(
        &mut self,
        project: &mut Project,
        from: usize,
        to: usize,
        copy: bool,
    ) -> Result<(u32, ClipDrop), ModelError> {
        // A copy goes into a row, so not after the last one.
        if from >= ROWS
            || to > ROWS
            || (copy && to == ROWS)
            || (!copy && (to == from || to == from + 1))
        {
            return Err(ModelError::BadSlot);
        }
        // A row moves within all the grid's rows, its four banks ([`ROWS`]:
        // scene banks, the Arrange refactor's 5.6). A copy needs the last
        // row empty.
        if copy && !project.row_empty(ROWS - 1) {
            return Err(ModelError::NoRoom);
        }
        let drop = if copy {
            ClipDrop::RowCopied { from, to }
        } else {
            ClipDrop::RowMoved { from, to }
        };
        let mut take = Take {
            edited: true,
            dropped: Some(drop),
            ..Take::default()
        };
        for (t, track) in project.tracks.iter().enumerate() {
            for (s, c) in track.slots.iter() {
                let (_, s2) = drop.after((t, s));
                if s2 != s {
                    take.removed.push((t, s, c.clone()));
                    take.made.push((t, s2, c.clone()));
                }
                if copy && s == from {
                    take.made.push((t, to, c.copied()));
                }
            }
        }
        // Each copy a clip of its own, with a new id.
        for (_, _, c) in &mut take.made {
            if !c.id.is_given() {
                c.id = project.new_clip_id();
            }
        }
        // The slots empty first, then filled: a row's clips move through
        // each other's places.
        for (t, s, _) in &take.removed {
            if let Some(slot) = slot_mut(project, *t, *s) {
                *slot = None;
            }
        }
        for (t, s, c) in &take.made {
            if let Some(slot) = slot_mut(project, *t, *s) {
                *slot = Some(c.clone());
            }
        }
        take.made.sort_by_key(|(t, s, _)| (*t, *s));
        let number = self.put_drop_filled(project, take);
        Ok((number, drop))
    }

    /// Fill the slots a clip drop's take names and keep the take. Its
    /// number.
    fn put_drop(&mut self, project: &mut Project, take: Take) -> u32 {
        for (t, s, _) in &take.removed {
            if let Some(slot) = slot_mut(project, *t, *s) {
                *slot = None;
            }
        }
        for (t, s, c) in &take.made {
            if let Some(slot) = slot_mut(project, *t, *s) {
                *slot = Some(c.clone());
            }
        }
        self.put_drop_filled(project, take)
    }

    fn put_drop_filled(&mut self, project: &mut Project, mut take: Take) -> u32 {
        if let Some(drop) = take.dropped {
            follow_rows(project, drop, false);
        }
        self.edits += 1;
        take.number = self.edits;
        self.push(take).number
    }
}

/// After undo (`back`) or redo moved a drop's clips: for a row, the
/// scenes, the morph's rows and the song list's steps follow it (the
/// song's blocks name their clips by id and need nothing).
pub(super) fn follow(project: &mut Project, drop: ClipDrop, back: bool) {
    follow_rows(project, drop, back);
}

/// A row drop: the scenes, the morph's rows, the song list's steps and
/// the sections made from a scene follow the row.
fn follow_rows(project: &mut Project, drop: ClipDrop, back: bool) {
    if !drop.is_row() {
        return;
    }
    let row = |r: usize| -> Option<usize> {
        if back {
            drop.before((0, r)).map(|c| c.1)
        } else {
            Some(drop.after((0, r)).1)
        }
    };
    let mut was = project.scenes.clone();
    was.resize(SCENES, Scene::default());
    let mut now = vec![Scene::default(); SCENES];
    for (r, s) in was.iter().enumerate() {
        // A plain row writes nothing: the grid's last row, empty for a
        // copy, stays in place under the row that moves into it.
        if let Some(r2) = row(r).filter(|&r2| r2 < SCENES && !plain(s)) {
            now[r2] = s.clone();
        }
    }
    if let (ClipDrop::RowCopied { from, to }, false) = (drop, back) {
        now[to] = was[from].clone();
    }
    while now.last().is_some_and(plain) {
        now.pop();
    }
    project.scenes = now;
    for m in &mut project.arrangement.morphs {
        m.a = row(m.a).unwrap_or(m.a).min(SCENES - 1);
        m.b = row(m.b).unwrap_or(m.b).min(SCENES - 1);
    }
    // The song list's steps play the same scenes where they went, and a
    // linked list's sections follow theirs.
    for step in &mut project.arrangement.list.steps {
        step.row = row(step.row).unwrap_or(step.row).min(SCENES - 1);
    }
    for from in project
        .arrangement
        .markers
        .iter_mut()
        .filter_map(|m| m.scene.as_mut())
    {
        from.row = row(from.row).unwrap_or(from.row).min(SCENES - 1);
    }
}

fn plain(s: &Scene) -> bool {
    *s == Scene::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Placement, Track};

    fn project() -> Project {
        let mut p = Project::new();
        for (i, name) in ["Drums", "Bass", "Keys"].iter().enumerate() {
            p.tracks.push(Track::new(name, i as u16));
        }
        p.tracks[0].slots[0] = Some(Clip::new("Beat", 1920));
        p.tracks[0].slots[1] = Some(Clip::new("Fill", 1920));
        p.tracks[1].slots[0] = Some(Clip::new("Bass A", 1920));
        p.tracks[1].slots[2] = Some(Clip::new("Bass C", 1920));
        // Beat 1, Fill 2, Bass A 3, Bass C 4.
        p.give_clip_ids();
        p
    }

    /// Where block `b`'s clip is now.
    fn home(p: &Project, b: &Placement) -> Option<mpfree_seq::ClipAt> {
        p.find_clip(b.clip)
    }

    fn name(p: &Project, t: usize, s: usize) -> Option<&str> {
        p.clip(t, s).map(|c| c.name.as_str())
    }

    #[test]
    fn a_clip_moves_swaps_and_copies_and_undo_takes_each_back() {
        let mut p = project();
        let mut takes = Takes::new(8);
        // Into an empty slot: moved.
        let (n, d) = takes.drop_clip(&mut p, (0, 1), (0, 3), false).unwrap();
        assert_eq!(d, ClipDrop::Moved((0, 1), (0, 3)));
        assert_eq!((name(&p, 0, 1), name(&p, 0, 3)), (None, Some("Fill")));
        // Onto a clip: swapped, so nothing is lost.
        let (_, d) = takes.drop_clip(&mut p, (0, 0), (1, 0), false).unwrap();
        assert_eq!(d, ClipDrop::Swapped((0, 0), (1, 0)));
        assert_eq!(
            (name(&p, 0, 0), name(&p, 1, 0)),
            (Some("Bass A"), Some("Beat"))
        );
        // A copy into an empty slot; never onto a clip.
        takes.drop_clip(&mut p, (1, 2), (2, 5), true).unwrap();
        assert_eq!(
            (name(&p, 1, 2), name(&p, 2, 5)),
            (Some("Bass C"), Some("Bass C"))
        );
        assert_eq!(
            takes.drop_clip(&mut p, (1, 2), (0, 0), true),
            Err(ModelError::NoRoom)
        );
        assert_eq!(
            takes.drop_clip(&mut p, (2, 9), (2, 1), false),
            Err(ModelError::BadSlot)
        );
        assert_eq!(
            takes.drop_clip(&mut p, (0, 0), (0, 0), false),
            Err(ModelError::BadSlot)
        );
        // Undo, one at a time, back to where it began.
        takes.undo(&mut p);
        assert_eq!(name(&p, 2, 5), None);
        takes.undo(&mut p);
        assert_eq!(
            (name(&p, 0, 0), name(&p, 1, 0)),
            (Some("Beat"), Some("Bass A"))
        );
        assert_eq!(takes.undo(&mut p), Some(n));
        // As it began, but for the next clip id: the copy took one, and
        // UNDO never winds it back (Arrange 1.1).
        let mut want = project();
        want.next_clip = p.next_clip;
        assert!(want.next_clip > 0);
        assert_eq!(p, want);
        // And redo does them again.
        takes.redo(&mut p);
        takes.redo(&mut p);
        assert_eq!(
            (name(&p, 0, 3), name(&p, 1, 0)),
            (Some("Fill"), Some("Beat"))
        );
    }

    /// Arrange 1.3: the song's blocks name their clips by id, so a clip
    /// dragged to another cell, its own track's or another's, leaves them
    /// as they are: they play it on, on their own lanes, and nothing about
    /// the song changes through UNDO and REDO.
    #[test]
    fn song_blocks_stay_as_they_are_when_their_clip_moves() {
        use mpfree_seq::ClipAt::Cell;
        let mut p = project();
        p.arrangement
            .place(Placement::of(&p.tracks, 1, 0, 0, 1920))
            .unwrap();
        p.arrangement
            .place(Placement::of(&p.tracks, 1, 2, 1920, 1920))
            .unwrap();
        p.arrangement
            .place(Placement::of(&p.tracks, 2, 0, 0, 3840))
            .unwrap();
        let song = p.arrangement.clone();
        let mut takes = Takes::new(8);
        // Bass A to row 4 of its track, then Bass C to the Keys' grid,
        // where Keys' block sits: both fine, the song as it was.
        takes.drop_clip(&mut p, (1, 0), (1, 3), false).unwrap();
        takes.drop_clip(&mut p, (1, 2), (2, 1), false).unwrap();
        assert_eq!(p.arrangement, song);
        let homes: Vec<_> = p
            .arrangement
            .on_track(1)
            .iter()
            .map(|b| home(&p, b))
            .collect();
        assert_eq!(
            homes,
            [
                Some(Cell { track: 1, row: 3 }),
                Some(Cell { track: 2, row: 1 })
            ]
        );
        let played: Vec<_> = p
            .arrangement
            .on_track(1)
            .iter()
            .map(|b| p.block_clip(b).map(|c| c.name.as_str()))
            .collect();
        assert_eq!(played, [Some("Bass A"), Some("Bass C")]);
        p.check().unwrap();
        takes.undo(&mut p);
        takes.undo(&mut p);
        assert_eq!(p.arrangement, song);
        assert_eq!(p, {
            let mut q = project();
            q.arrangement = song.clone();
            q
        });
        takes.redo(&mut p);
        assert_eq!(
            home(&p, &p.arrangement.on_track(1)[0]),
            Some(Cell { track: 1, row: 3 })
        );
    }

    #[test]
    fn a_row_moves_or_is_copied_with_its_scene_and_nothing_is_written_over() {
        let mut p = project();
        p.scene_mut(2).unwrap().name = "Drop".into();
        p.arrangement
            .place(Placement::of(&p.tracks, 1, 2, 0, 1920))
            .unwrap();
        // The song list plays rows 3 then 2.
        let step = |row| mpfree_seq::ListStep { row, times: 1 };
        p.arrangement.list.steps = vec![step(2), step(1)];
        // And a section of the song made from row 3 (Arrange 4b).
        let section = |row| mpfree_seq::Marker {
            scene: Some(mpfree_seq::FromScene::following(row, 1920)),
            ..mpfree_seq::Marker::new(0, "Drop")
        };
        p.arrangement.markers = vec![section(2)];
        let mut takes = Takes::new(8);
        // Row 3 in between rows 1 and 2: rows 2 moves down one.
        let (_, d) = takes.drop_row(&mut p, 2, 1, false).unwrap();
        assert_eq!(
            p.arrangement.list.steps,
            vec![step(1), step(2)],
            "the list's steps follow their rows"
        );
        assert_eq!(
            p.arrangement.markers,
            vec![section(1)],
            "and so does the section made from it"
        );
        assert_eq!(d, ClipDrop::RowMoved { from: 2, to: 1 });
        assert_eq!(name(&p, 1, 1), Some("Bass C"));
        assert_eq!(name(&p, 0, 2), Some("Fill"), "the row under moved down");
        assert_eq!(name(&p, 0, 0), Some("Beat"), "the row above stayed");
        assert_eq!(p.scene(1).map(|s| s.name.as_str()), Some("Drop"));
        assert_eq!(
            home(&p, &p.arrangement.placements[0]),
            Some(mpfree_seq::ClipAt::Cell { track: 1, row: 1 }),
            "its block plays it where it went"
        );
        p.check().unwrap();
        // A move that lands where it started does nothing.
        assert_eq!(
            takes.drop_row(&mut p, 1, 2, false),
            Err(ModelError::BadSlot)
        );
        // A copy of row 1 at the top: every row moves down one.
        takes.drop_row(&mut p, 0, 0, true).unwrap();
        assert_eq!(
            (name(&p, 0, 0), name(&p, 0, 1)),
            (Some("Beat"), Some("Beat"))
        );
        assert_eq!(name(&p, 1, 2), Some("Bass C"));
        assert_eq!(p.scene(2).map(|s| s.name.as_str()), Some("Drop"));
        // With the last row in use, a copy has no room.
        p.tracks[2].slots[ROWS - 1] = Some(Clip::new("End", 480));
        assert_eq!(takes.drop_row(&mut p, 0, 0, true), Err(ModelError::NoRoom));
        p.tracks[2].slots[ROWS - 1] = None;
        takes.undo(&mut p);
        takes.undo(&mut p);
        let mut want = project();
        want.scene_mut(2).unwrap().name = "Drop".into();
        want.arrangement
            .place(Placement::of(&want.tracks, 1, 2, 0, 1920))
            .unwrap();
        want.arrangement.list.steps = vec![step(2), step(1)];
        want.arrangement.markers = vec![section(2)];
        want.next_clip = p.next_clip;
        assert_eq!(p, want);
    }

    /// Scene banks (the Arrange refactor's 5.6): the rows are one list of
    /// 64 across banks A to D. A row moved within bank A leaves a clip
    /// and a scene in row 41 (bank C) where they are; a row copied above
    /// them moves them down one, the song's block of the clip with them,
    /// and UNDO and REDO put them back.
    #[test]
    fn rows_in_other_banks_move_down_when_a_row_goes_in_above() {
        let mut p = project();
        p.tracks[1].slots[40] = Some(Clip::new("Far", 1920));
        p.give_clip_ids();
        p.scene_mut(40).unwrap().name = "Bank C".into();
        p.arrangement
            .place(Placement::of(&p.tracks, 1, 40, 3840, 1920))
            .unwrap();
        let mut takes = Takes::default();
        takes.drop_row(&mut p, 0, 3, false).unwrap();
        // Where the clip is, where the scene is, and the row the song's
        // block of it plays from.
        let far = |p: &Project| {
            (
                (0..ROWS).find(|&r| name(p, 1, r) == Some("Far")),
                (0..ROWS).find(|&r| p.scene(r).is_some_and(|s| s.name == "Bank C")),
                p.arrangement
                    .placements
                    .iter()
                    .find(|b| b.start == 3840)
                    .and_then(|b| p.tracks[1].slots.row_of(b.clip)),
            )
        };
        let at = |row: usize| (Some(row), Some(row), Some(row));
        assert_eq!(far(&p), at(40));
        takes.drop_row(&mut p, 1, 0, true).unwrap();
        assert_eq!(far(&p), at(41), "down one, under the copy");
        p.check().unwrap();
        takes.undo(&mut p);
        assert_eq!(far(&p), at(40));
        takes.undo(&mut p);
        assert_eq!(far(&p), at(40));
        takes.redo(&mut p);
        takes.redo(&mut p);
        assert_eq!(far(&p), at(41));
    }

    #[test]
    fn after_and_before_undo_each_other() {
        for d in [
            ClipDrop::Moved((0, 1), (2, 3)),
            ClipDrop::Swapped((0, 1), (2, 3)),
            ClipDrop::RowMoved { from: 2, to: 9 },
            ClipDrop::RowMoved { from: 9, to: 2 },
            ClipDrop::RowCopied { from: 4, to: 1 },
        ] {
            for t in 0..3 {
                for s in 0..ROWS - 1 {
                    if matches!(d, ClipDrop::Moved(_, b) if b == (t, s)) {
                        continue; // empty before the drop
                    }
                    assert_eq!(d.before(d.after((t, s))), Some((t, s)), "{d:?} {t} {s}");
                }
            }
        }
    }
}
