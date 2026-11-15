//! Drops on the Clips grid (Jay, 14 Nov: drag and drop): a clip moved to
//! an empty slot, two clips swapped, a clip copied, and a whole row moved
//! or copied, each as one take that undo takes back.
//!
//! The song's blocks linked to a clip that moves go with it (to its new
//! slot, and to its new track's lane), so the song plays as it did; so do
//! the morph's rows and the rows' scenes when a row moves. Undo and redo
//! work them out again from the drop ([`ClipDrop::after`] and
//! [`ClipDrop::before`]), so an edit made to the song since is kept.

use mpfree_seq::{Arrangement, ModelError, Project, Scene, Source, SCENES, SLOTS};

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
    /// row `to` (`to` is a gap, 0 to [`SLOTS`]); the rows between moved
    /// up or down one.
    RowMoved { from: usize, to: usize },
    /// A copy of row `from` went in before row `to`, and the rows from
    /// `to` on moved down one (the last row was empty).
    RowCopied { from: usize, to: usize },
}

impl ClipDrop {
    /// Where the clip that was at `at` is after the drop.
    pub fn after(&self, (t, s): Cell) -> Cell {
        match *self {
            ClipDrop::Moved(a, b) | ClipDrop::Swapped(a, b) if (t, s) == a => b,
            ClipDrop::Swapped(a, b) if (t, s) == b => a,
            ClipDrop::RowMoved { from, to } => (t, moved(s, from, landing(from, to))),
            ClipDrop::RowCopied { to, .. } if s >= to => (t, s + 1),
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
            ClipDrop::RowCopied { to, .. } if s > to => Some((t, s - 1)),
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
    /// onto a clip; `BadPlacement` when the clip's song blocks would
    /// overlap blocks on its new track's lane.
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
        let song = relink(&project.arrangement, |c| Some(drop.after(c)))?;
        let mut take = Take {
            edited: true,
            dropped: Some(drop),
            ..Take::default()
        };
        match (drop, there) {
            (ClipDrop::Copied(..), _) => take.made.push((to.0, to.1, a)),
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
        Ok((self.put_drop(project, take, song), drop))
    }

    /// A row's number dragged on the Clips grid: row `from` (every
    /// track's clip in it, and its scene) goes in before row `to`, a gap
    /// from 0 to [`SLOTS`]. Moved, the rows between move up or down one;
    /// with `copy`, a copy goes in and the rows from `to` on move down
    /// one, so the last row must be empty. Nothing is written over. Its
    /// number and what it did. Nothing changes on an error: `BadSlot`
    /// for a row out of range or a move that lands where it started;
    /// `NoRoom` for a copy with the last row in use.
    pub fn drop_row(
        &mut self,
        project: &mut Project,
        from: usize,
        to: usize,
        copy: bool,
    ) -> Result<(u32, ClipDrop), ModelError> {
        if from >= SLOTS || to > SLOTS || (!copy && (to == from || to == from + 1)) {
            return Err(ModelError::BadSlot);
        }
        let last = SLOTS - 1;
        if copy && (project.row_filled(last) || project.scene(last).is_some_and(|s| !plain(s))) {
            return Err(ModelError::NoRoom);
        }
        let drop = if copy {
            ClipDrop::RowCopied { from, to }
        } else {
            ClipDrop::RowMoved { from, to }
        };
        let song = relink(&project.arrangement, |c| Some(drop.after(c)))?;
        let mut take = Take {
            edited: true,
            dropped: Some(drop),
            ..Take::default()
        };
        for (t, track) in project.tracks.iter().enumerate() {
            for (s, c) in track.slots.iter().enumerate() {
                let Some(c) = c else { continue };
                let (_, s2) = drop.after((t, s));
                if s2 != s {
                    take.removed.push((t, s, c.clone()));
                    take.made.push((t, s2, c.clone()));
                }
                if copy && s == from {
                    take.made.push((t, to, c.clone()));
                }
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
        let number = self.put_drop_filled(project, take, song);
        Ok((number, drop))
    }

    /// Fill the slots a clip drop's take names, link the song as it
    /// follows them and keep the take. Its number.
    fn put_drop(&mut self, project: &mut Project, take: Take, song: Option<Arrangement>) -> u32 {
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
        self.put_drop_filled(project, take, song)
    }

    fn put_drop_filled(
        &mut self,
        project: &mut Project,
        mut take: Take,
        song: Option<Arrangement>,
    ) -> u32 {
        if let Some(song) = song {
            project.arrangement = song;
        }
        if let Some(drop) = take.dropped {
            follow_rows(project, drop, false);
        }
        self.edits += 1;
        take.number = self.edits;
        self.push(take).number
    }
}

/// After undo (`back`) or redo moved a drop's clips: the song's linked
/// blocks, and for a row the morph's rows and the scenes, follow them. A
/// song edited since so that its blocks would overlap stays as it is.
pub(super) fn follow(project: &mut Project, drop: ClipDrop, back: bool) {
    let song = if back {
        relink(&project.arrangement, |c| drop.before(c))
    } else {
        relink(&project.arrangement, |c| Some(drop.after(c)))
    };
    if let Ok(Some(song)) = song {
        project.arrangement = song;
    }
    follow_rows(project, drop, back);
}

/// A row drop: the scenes and the morph's rows follow the row.
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
        if let Some(r2) = row(r).filter(|&r2| r2 < SCENES) {
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
}

fn plain(s: &Scene) -> bool {
    *s == Scene::default()
}

/// `song` with each block linked to a clip moved where `to` says its
/// clip went (None: left as it is), or None when no block moves.
/// `BadPlacement` when blocks would overlap on a lane.
fn relink(
    song: &Arrangement,
    to: impl Fn(Cell) -> Option<Cell>,
) -> Result<Option<Arrangement>, ModelError> {
    let goes = |track: usize, source: &Source| match source {
        Source::Slot(s) => to((track, *s)).filter(|c| *c != (track, *s)),
        Source::Own(_) => None,
    };
    if !song
        .placements
        .iter()
        .any(|p| goes(p.track, &p.source).is_some())
    {
        return Ok(None);
    }
    let mut out = Arrangement {
        placements: Vec::with_capacity(song.placements.len()),
        markers: song.markers.clone(),
        lanes: song.lanes.clone(),
        morphs: song.morphs.clone(),
    };
    for p in &song.placements {
        let mut q = p.clone();
        if let Some((t, s)) = goes(p.track, &p.source) {
            q.track = t;
            q.source = Source::Slot(s);
        }
        out.place(q)?;
    }
    Ok(Some(out))
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
        p
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
        assert_eq!(p, project());
        // And redo does them again.
        takes.redo(&mut p);
        takes.redo(&mut p);
        assert_eq!(
            (name(&p, 0, 3), name(&p, 1, 0)),
            (Some("Fill"), Some("Beat"))
        );
    }

    #[test]
    fn song_blocks_follow_their_clip_and_come_back_on_undo() {
        let mut p = project();
        p.arrangement
            .place(Placement::linked(1, 0, 0, 1920))
            .unwrap();
        p.arrangement
            .place(Placement::linked(1, 2, 1920, 1920))
            .unwrap();
        p.arrangement
            .place(Placement::linked(2, 0, 0, 3840))
            .unwrap();
        let mut takes = Takes::new(8);
        // Bass A to slot 4 of its track: its block follows, still linked.
        takes.drop_clip(&mut p, (1, 0), (1, 3), false).unwrap();
        let on = |p: &Project, t| {
            p.arrangement
                .on_track(t)
                .iter()
                .map(|b| (b.start, b.slot()))
                .collect::<Vec<_>>()
        };
        assert_eq!(on(&p, 1), [(0, Some(3)), (1920, Some(2))]);
        p.check().unwrap();
        // Bass C onto Keys' lane would sit on Keys' block there: refused.
        assert_eq!(
            takes.drop_clip(&mut p, (1, 2), (2, 1), false),
            Err(ModelError::BadPlacement)
        );
        assert_eq!(name(&p, 1, 2), Some("Bass C"), "nothing changed");
        takes.undo(&mut p);
        assert_eq!(on(&p, 1), [(0, Some(0)), (1920, Some(2))]);
        assert_eq!(p, {
            let mut q = project();
            q.arrangement = p.arrangement.clone();
            q
        });
    }

    #[test]
    fn a_row_moves_or_is_copied_with_its_scene_and_nothing_is_written_over() {
        let mut p = project();
        p.scene_mut(2).unwrap().name = "Drop".into();
        p.arrangement
            .place(Placement::linked(1, 2, 0, 1920))
            .unwrap();
        let mut takes = Takes::new(8);
        // Row 3 in between rows 1 and 2: rows 2 moves down one.
        let (_, d) = takes.drop_row(&mut p, 2, 1, false).unwrap();
        assert_eq!(d, ClipDrop::RowMoved { from: 2, to: 1 });
        assert_eq!(name(&p, 1, 1), Some("Bass C"));
        assert_eq!(name(&p, 0, 2), Some("Fill"), "the row under moved down");
        assert_eq!(name(&p, 0, 0), Some("Beat"), "the row above stayed");
        assert_eq!(p.scene(1).map(|s| s.name.as_str()), Some("Drop"));
        assert_eq!(
            p.arrangement.placements[0].slot(),
            Some(1),
            "its block follows"
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
        p.tracks[2].slots[SLOTS - 1] = Some(Clip::new("End", 480));
        assert_eq!(takes.drop_row(&mut p, 0, 0, true), Err(ModelError::NoRoom));
        p.tracks[2].slots[SLOTS - 1] = None;
        takes.undo(&mut p);
        takes.undo(&mut p);
        let mut want = project();
        want.scene_mut(2).unwrap().name = "Drop".into();
        want.arrangement
            .place(Placement::linked(1, 2, 0, 1920))
            .unwrap();
        assert_eq!(p, want);
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
                for s in 0..SLOTS - 1 {
                    if matches!(d, ClipDrop::Moved(_, b) if b == (t, s)) {
                        continue; // empty before the drop
                    }
                    assert_eq!(d.before(d.after((t, s))), Some((t, s)), "{d:?} {t} {s}");
                }
            }
        }
    }
}
