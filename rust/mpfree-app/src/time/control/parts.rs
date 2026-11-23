//! The song's parts follow their rows (the Arrange refactor's follow-up
//! to 4b, plan D8: one song, two views). A part of a linked song list
//! that follows its scene (made from a row, not changed in the song)
//! plays what the row holds now: when an action puts a clip in the row or
//! takes one out (a clip recorded, kept, pasted, dropped there or moved
//! away, a clip deleted, Song → scenes filling it), each part that
//! follows the row is made again from it, in place
//! ([`TimeControl::follow_rows`]).
//!
//! - **In place:** the part keeps its bars. The row is played through
//!   the offline time engine as many passes as fill them
//!   ([`offline::play_part`]), the blocks cut at the part's end, so a
//!   longer clip doesn't push the song on and a shorter row leaves no
//!   silence. Its marker, passes and pass length stay as they were.
//! - **Which parts:** only a linked list's, made from a row the action
//!   changed, and not changed in the song: a part edited in the song is
//!   the song's own and keeps what it holds. A clip moved within its row
//!   to another track changes the row too (its blocks go to the new
//!   track's lane); its notes edited, or the clip made longer, don't
//!   (its blocks name the clip by id and play it as it is).
//! - **One UNDO:** the song's change rides on the action's take
//!   ([`mpfree_play::Take::followed`]), so UNDO takes back the clip and
//!   the parts together. A new clip REC put in an empty cell is no take,
//!   so its parts are a take of their own ("Parts · row 5").
//! - Undo and redo never remake parts: the take holds the song before
//!   and after.

use mpfree_seq::arrange::{scene_length, song_sections};
use mpfree_seq::scenes::MAX_TIMES;
use mpfree_seq::{ClipId, Placement};
use mpfree_transport::time::Tick;

use super::TimeControl;
use crate::time::offline;

/// What a row holds, as (track, clip id) in track order.
pub(in crate::time) type RowClips = Vec<(usize, ClipId)>;

impl TimeControl {
    /// The rows a linked list's parts follow (made from them, not changed
    /// in the song), each with what it holds now; none for a list that
    /// isn't linked.
    pub(in crate::time) fn followed_rows(&self) -> Vec<(usize, RowClips)> {
        let song = &self.project.arrangement;
        if !song.list.linked {
            return Vec::new();
        }
        let mut rows: Vec<usize> = song
            .markers
            .iter()
            .filter_map(|m| m.scene)
            .filter(|f| !f.changed)
            .map(|f| f.row)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows.into_iter().map(|r| (r, self.row_clips(r))).collect()
    }

    /// Whether the song plays clip `id` in a block that isn't in a part
    /// following row `row`: one deleted from the row goes from those parts
    /// with it, and the song keeps playing it only in the others.
    pub(in crate::time) fn plays_apart_from_row(&self, id: ClipId, row: usize) -> bool {
        let song = &self.project.arrangement;
        let bar = self.info.meter.bar_ticks().max(1);
        let follow: Vec<(Tick, Tick)> = if song.list.linked {
            song_sections(&self.project, bar)
                .iter()
                .filter(|s| s.scene.is_some_and(|f| !f.changed && f.row == row))
                .map(|s| (s.start, s.end))
                .collect()
        } else {
            Vec::new()
        };
        song.placements
            .iter()
            .any(|p| p.clip == id && !follow.iter().any(|&(a, b)| p.start >= a && p.end() <= b))
    }

    /// What row `row` holds now.
    fn row_clips(&self, row: usize) -> RowClips {
        (0..self.project.tracks.len())
            .filter_map(|t| self.project.clip(t, row).map(|c| (t, c.id)))
            .collect()
    }

    /// After an action that may have changed the rows in `before` (from
    /// [`TimeControl::followed_rows`] before it; `edits` the take count
    /// then): each part that follows a row that holds something else now
    /// made again from it, in place (see the module notes). What to add
    /// to the action's line, or None when no part changed. A part that
    /// can't be made again leaves the song as it was and says why; the
    /// action stays done.
    pub(in crate::time) fn follow_rows(
        &mut self,
        before: &[(usize, RowClips)],
        edits: u32,
    ) -> Option<String> {
        let changed: Vec<usize> = before
            .iter()
            .filter(|(row, held)| self.row_clips(*row) != *held)
            .map(|(row, _)| *row)
            .collect();
        let first = *changed.first()?;
        let bar = self.info.meter.bar_ticks().max(1);
        let was = self.project.arrangement.clone();
        let mut song = was.clone();
        let tracks = &self.project.tracks;
        for s in song_sections(&self.project, bar) {
            let Some(from) = s.scene.filter(|f| !f.changed && changed.contains(&f.row)) else {
                continue;
            };
            let len = s.end - s.start;
            let pass = scene_length(&self.project, from.row, bar).max(1);
            let times = ((len + pass - 1) / pass).clamp(1, Tick::from(MAX_TIMES)) as u16;
            let blocks = match offline::play_part(&self.project, from.row, times, s.start, bar) {
                Ok(b) => b,
                Err(why) => {
                    return Some(format!(
                        "; the song's parts from row {} stay as they were: {why}",
                        from.row + 1
                    ))
                }
            };
            let blocks: Vec<Placement> = blocks
                .into_iter()
                .filter(|p| p.start < s.end)
                .map(|p| Placement {
                    length: p.length.min(s.end - p.start),
                    ..p
                })
                .collect();
            if song.fill(tracks, s.start, s.end, &blocks).is_err() {
                return Some(format!(
                    "; the song's parts from row {} stay as they were: the song would hold too \
                     many blocks",
                    from.row + 1
                ));
            }
        }
        if song == was {
            return None;
        }
        if let Err(e) = song.check(tracks) {
            return Some(format!(
                "; the song's parts from row {} stay as they were ({e:?})",
                first + 1
            ));
        }
        self.project.arrangement = song;
        // The blocks changed: the Song view's pick names another now.
        self.selected = None;
        let number = (self.takes.edits() > edits).then(|| self.takes.edits());
        self.takes.follow(&self.project, number, first, was);
        if let Err(e) = self.compile() {
            return Some(format!("; the song's parts: {e}"));
        }
        let rows = match changed.as_slice() {
            [_] => format!("row {}", first + 1),
            _ => "their rows".to_string(),
        };
        Some(format!("; the song's parts from {rows} with it"))
    }
}
