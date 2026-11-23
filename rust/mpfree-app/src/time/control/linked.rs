//! A linked song list's edits (the Arrange refactor's 4b.2, plan D8: one
//! song, two views). The list is the song's sections
//! ([`song_sections`]), so editing the list edits the song, by phase 3's
//! time edits ([`TimeEdit`]), which take everything under a section with
//! it: its blocks and marker, the tempo, the song's lanes and the morph's
//! rows, and the audio tracks' regions ([`TimeControl::act_with_audio`]
//! gives them the same edit). Each edit is one UNDO.
//!
//! - **A step taken out**: its section deleted; the song after it moves
//!   up.
//! - **A step moved** (its grip): its section moved; the sections between
//!   close up behind it and make room for it.
//! - **× N** on a section that follows its scene: the section made N
//!   passes of its row as the row is now, with room put in or bars taken
//!   out at its end, and its blocks made again from the row
//!   ([`offline::play_part`]). On one that doesn't (the song's own, or
//!   changed in the song): + puts a copy of its last pass after it,
//!   everything in it with it, and – takes its last pass out.
//! - **Another scene** (‹ ›): the section made from the next or the
//!   previous row with clips, keeping its passes, as long as that row's
//!   passes need, and named after the row.
//! - **+ Add a scene**: room for one pass of the next row with clips after
//!   the picked section (at the end with none), made from that row.
//! - **Back to the scene** (4b.3), on a section changed in the song: made
//!   from its row again, as many passes of the row as now fit its bars
//!   best (half way, the fewer; one at least), with room put in or bars
//!   taken out at its end when they don't fit exactly. It follows its
//!   scene again.
//!
//! An edit of the song inside a section that follows its scene makes it
//! the song's own (`Arrangement::mark_changed`): the list's edits leave
//! it as it is, its passes plain bars, until Back to the scene.
//!
//! The Song row (the song's bars before its first marker) is the song's
//! own: it can be taken out, but it stays first, has no scene and plays
//! once.
//!
//! A section made from a scene is the row's clips, linked by their ids,
//! played as the list plays one step from a start with nothing before it:
//! its blocks don't depend on where it is, so moving it moves what it
//! plays. So on a song whose sections all follow their scenes, the song
//! after any of these edits is the song List → song makes of the new
//! list, but for what one step leaves the next, which List → song writes
//! into the blocks: the scenes' mutes, which a linked song brings in as
//! each section starts instead (4b.4), and a legato clip taking over the
//! clip playing before it.

use std::cmp::Ordering;

use mpfree_play::SongChange;
use mpfree_seq::arrange::{scene_length, scene_title, song_sections, FromScene, SongSection};
use mpfree_seq::scenes::{MAX_LIST_STEPS, MAX_TIMES};
use mpfree_seq::time_edit::Refused;
use mpfree_seq::{TimeEdit, SLOTS};
use mpfree_time_ui::view::TimeAction;
use mpfree_transport::time::Tick;

use super::{bar_beat, next_filled, refused_said, TimeControl};
use crate::time::offline;

/// What an edit of a linked list does to the song, worked out from the
/// song as it is, before anything changes (the audio tracks are asked
/// about the time edit first).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::time) struct LinkedEdit {
    /// The one time edit.
    pub(in crate::time) time: Option<TimeEdit>,
    /// After it: the marker on this tick taken out (the copy a copied
    /// pass made of its section's marker: a pass is not a section).
    unmark: Option<Tick>,
    /// After it: the section's marker.
    marker: Option<MarkerAfter>,
    /// After it: song ticks `at..at + len` made from row `row`, `passes`
    /// times.
    remake: Option<Remake>,
    /// The step picked after.
    pick: Option<usize>,
    /// For the log.
    said: String,
}

/// A section's marker after the edit, on tick `at` (put there when there
/// is none): its passes, and its new scene and name when it has them.
#[derive(Clone, Debug, PartialEq)]
struct MarkerAfter {
    at: Tick,
    name: Option<String>,
    scene: Option<FromScene>,
    passes: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Remake {
    at: Tick,
    len: Tick,
    row: usize,
    passes: u16,
}

impl LinkedEdit {
    /// An edit that changes nothing, with its line.
    fn nothing(said: String) -> LinkedEdit {
        LinkedEdit {
            time: None,
            unmark: None,
            marker: None,
            remake: None,
            pick: None,
            said,
        }
    }

    fn changes(&self) -> bool {
        self.time.is_some() || self.marker.is_some() || self.remake.is_some()
    }
}

/// Whether `a` is an edit of the song list's steps (what a linked list
/// makes an edit of the song).
pub fn edits_steps(a: &TimeAction) -> bool {
    matches!(
        a,
        TimeAction::ListTimes { .. }
            | TimeAction::ListScene { .. }
            | TimeAction::ListAdd
            | TimeAction::ListRemove(_)
            | TimeAction::ListMove { .. }
            | TimeAction::ListBack(_)
    )
}

/// The time edit that makes section `s` `len` ticks long at its end: room
/// put in, or the bars past `len` taken out.
fn resize(s: &SongSection, len: Tick) -> Option<TimeEdit> {
    let span = s.end - s.start;
    if len > span {
        Some(TimeEdit::Insert {
            at: s.end,
            len: len - span,
        })
    } else if len < span {
        Some(TimeEdit::Delete {
            from: s.start + len,
            to: s.end,
        })
    } else {
        None
    }
}

/// What a list step is called: its marker's name, or "Song" for the
/// bars before the first marker.
fn step_name(project: &mpfree_seq::Project, s: &SongSection) -> String {
    s.marker.map_or_else(
        || "the Song row".to_string(),
        |k| project.arrangement.markers[k].name.clone(),
    )
}

/// Why the Song row can't take an edit: it is the song's own bars before
/// its first marker (as the list says, `mpfree_time_ui::view::SONG_ROW_WHY`).
const SONG_ROW: &str =
    "song list: the bars before the first marker: add a marker on the Song view's ruler to make them a part";

impl TimeControl {
    /// The edit `a` (an edit of the steps, [`edits_steps`]) makes of a
    /// linked list's song, with bars of `bar` ticks: see the module notes.
    /// Err says why it can't be made.
    pub(in crate::time) fn linked_edit(
        &self,
        a: &TimeAction,
        bar: Tick,
    ) -> Result<LinkedEdit, String> {
        let bar = bar.max(1);
        let p = &self.project;
        let parts = song_sections(p, bar);
        let n = parts.len();
        let part = |k: usize| parts.get(k).ok_or("song list: no such step");
        let lead = usize::from(parts.first().is_some_and(|s| s.marker.is_none()));
        let filled: Vec<usize> = (0..SLOTS).filter(|&r| p.row_filled(r)).collect();
        const NO_CLIPS: &str =
            "song list: make a clip on the grid first: a step plays a row with clips";
        match *a {
            TimeAction::ListRemove(k) => {
                let s = part(k)?;
                Ok(LinkedEdit {
                    time: Some(TimeEdit::Delete {
                        from: s.start,
                        to: s.end,
                    }),
                    pick: (n > 1).then(|| k.min(n - 2)),
                    said: format!(
                        "song list: {} taken out, and its bars with it; the song after it moved up (UNDO puts it back)",
                        step_name(p, s)
                    ),
                    ..LinkedEdit::nothing(String::new())
                })
            }
            TimeAction::ListMove { step, to } => {
                let s = part(step)?;
                if s.marker.is_none() || to < lead {
                    return Err("song list: the Song row stays first".into());
                }
                if to > n {
                    return Err("song list: no such place".into());
                }
                let name = step_name(p, s);
                if to == step || to == step + 1 {
                    return Ok(LinkedEdit::nothing(format!(
                        "song list: {name} is already there"
                    )));
                }
                let at = parts.get(to).map_or(parts[n - 1].end, |t| t.start);
                let now = if to > step { to - 1 } else { to };
                Ok(LinkedEdit {
                    time: Some(TimeEdit::Move {
                        from: s.start,
                        to: s.end,
                        at,
                    }),
                    pick: Some(now),
                    said: format!(
                        "song list: {name} is step {} now, and its bars moved with it (UNDO takes it back)",
                        now + 1
                    ),
                    ..LinkedEdit::nothing(String::new())
                })
            }
            TimeAction::ListTimes { step, delta } => {
                let s = part(step)?;
                if s.marker.is_none() {
                    return Err(SONG_ROW.into());
                }
                let name = step_name(p, s);
                let to =
                    (i32::from(s.passes) + delta.signum()).clamp(1, i32::from(MAX_TIMES)) as u16;
                if to == s.passes {
                    return Ok(LinkedEdit::nothing(format!("song list: {name} ×{to}")));
                }
                let said = format!("song list: {name} ×{to} (UNDO takes it back)");
                let mut e = match s.scene {
                    // Made again from its row as the row is now.
                    Some(from) if !from.changed => {
                        let pass = scene_length(p, from.row, bar);
                        let len = pass * Tick::from(to);
                        LinkedEdit {
                            time: resize(s, len),
                            marker: Some(MarkerAfter {
                                at: s.start,
                                name: None,
                                scene: Some(FromScene::following(from.row, pass)),
                                passes: to,
                            }),
                            remake: Some(Remake {
                                at: s.start,
                                len,
                                row: from.row,
                                passes: to,
                            }),
                            ..LinkedEdit::nothing(said)
                        }
                    }
                    // Its last pass copied after it, or taken out.
                    _ => {
                        let pass = s.pass();
                        let (time, unmark) = if to > s.passes {
                            (
                                TimeEdit::Copy {
                                    from: s.end - pass,
                                    to: s.end,
                                    at: s.end,
                                },
                                Some(s.end),
                            )
                        } else {
                            (
                                TimeEdit::Delete {
                                    from: s.end - pass,
                                    to: s.end,
                                },
                                None,
                            )
                        };
                        LinkedEdit {
                            time: Some(time),
                            unmark,
                            marker: Some(MarkerAfter {
                                at: s.start,
                                name: None,
                                scene: None,
                                passes: to,
                            }),
                            ..LinkedEdit::nothing(said)
                        }
                    }
                };
                e.pick = Some(step);
                Ok(e)
            }
            TimeAction::ListScene { step, delta } => {
                let s = part(step)?;
                if s.marker.is_none() {
                    return Err(SONG_ROW.into());
                }
                let was = s.scene.map(|f| f.row);
                let row = match was {
                    Some(r) => next_filled(&filled, r, delta),
                    None if delta >= 0 => filled.first().copied(),
                    None => filled.last().copied(),
                }
                .ok_or(NO_CLIPS)?;
                if was == Some(row) {
                    return Err("song list: no other row has a clip".into());
                }
                let pass = scene_length(p, row, bar);
                let len = pass * Tick::from(s.passes);
                let name = scene_title(p, row);
                Ok(LinkedEdit {
                    time: resize(s, len),
                    marker: Some(MarkerAfter {
                        at: s.start,
                        name: Some(name.clone()),
                        scene: Some(FromScene::following(row, pass)),
                        passes: s.passes,
                    }),
                    remake: Some(Remake {
                        at: s.start,
                        len,
                        row,
                        passes: s.passes,
                    }),
                    pick: Some(step),
                    said: format!(
                        "song list: step {} plays {name} now (UNDO takes it back)",
                        step + 1
                    ),
                    ..LinkedEdit::nothing(String::new())
                })
            }
            TimeAction::ListAdd => {
                if n >= MAX_LIST_STEPS {
                    return Err(format!("song list: full ({MAX_LIST_STEPS} steps)"));
                }
                // After the pick (the end with none): the next row with a
                // clip after the scene there.
                let k = self.list_pick.map_or(n, |k| (k + 1).min(n)).max(lead);
                let at = parts
                    .get(k)
                    .map(|s| s.start)
                    .or_else(|| parts.last().map(|s| s.end))
                    .unwrap_or(0);
                let row = match k.checked_sub(1).and_then(|j| parts[j].scene) {
                    Some(from) => next_filled(&filled, from.row, 1),
                    None => filled.first().copied(),
                }
                .ok_or(NO_CLIPS)?;
                let pass = scene_length(p, row, bar);
                let name = scene_title(p, row);
                Ok(LinkedEdit {
                    time: Some(TimeEdit::Insert { at, len: pass }),
                    marker: Some(MarkerAfter {
                        at,
                        name: Some(name.clone()),
                        scene: Some(FromScene::following(row, pass)),
                        passes: 1,
                    }),
                    remake: Some(Remake {
                        at,
                        len: pass,
                        row,
                        passes: 1,
                    }),
                    pick: Some(k),
                    said: format!(
                        "song list: step {} plays {name}, at {}; the song after it moved on (UNDO takes it out)",
                        k + 1,
                        bar_beat(at, self.info.meter)
                    ),
                    ..LinkedEdit::nothing(String::new())
                })
            }
            TimeAction::ListBack(step) => {
                let s = part(step)?;
                let name = step_name(p, s);
                let from = match s.scene {
                    _ if s.marker.is_none() => return Err(SONG_ROW.into()),
                    None => return Err(format!("song list: {name} has no scene")),
                    Some(f) if !f.changed => {
                        return Err(format!("song list: {name} follows its scene already"))
                    }
                    Some(f) => f,
                };
                if !p.row_filled(from.row) {
                    return Err(format!(
                        "song list: {} has no clips now: make one on the grid first",
                        scene_title(p, from.row)
                    ));
                }
                // As many passes as fit its bars best (half way, the
                // fewer), one at least.
                let pass = scene_length(p, from.row, bar);
                let span = s.end - s.start;
                let passes =
                    ((2 * span + pass - 1) / (2 * pass)).clamp(1, Tick::from(MAX_TIMES)) as u16;
                let len = pass * Tick::from(passes);
                let times = if passes > 1 {
                    format!(" ×{passes}")
                } else {
                    String::new()
                };
                let moved = match len.cmp(&span) {
                    Ordering::Greater => "; the song after it moved on",
                    Ordering::Less => "; the song after it moved up",
                    Ordering::Equal => "",
                };
                Ok(LinkedEdit {
                    time: resize(s, len),
                    marker: Some(MarkerAfter {
                        at: s.start,
                        name: None,
                        scene: Some(FromScene::following(from.row, pass)),
                        passes,
                    }),
                    remake: Some(Remake {
                        at: s.start,
                        len,
                        row: from.row,
                        passes,
                    }),
                    pick: Some(step),
                    said: format!(
                        "song list: {name} plays its scene again{times}{moved} (UNDO takes it back)"
                    ),
                    ..LinkedEdit::nothing(String::new())
                })
            }
            _ => Err("song list: not an edit of its steps".into()),
        }
    }

    /// A linked list's edit made ([`TimeControl::linked_edit`]): the time
    /// edit, then the section's marker, then its blocks made from its row;
    /// refused, the song is as it was. A line for the log, and what changed
    /// for the take UNDO takes back.
    pub(in crate::time) fn edit_linked(
        &mut self,
        a: &TimeAction,
        bar: Tick,
    ) -> Result<(String, Option<SongChange>), String> {
        let e = self.linked_edit(a, bar)?;
        if !e.changes() {
            return Ok((e.said, None));
        }
        let meter = self.info.meter;
        let at = |t: Tick| bar_beat(t, meter);
        let tracks = &self.project.tracks;
        let mut song = self.project.arrangement.clone();
        if let Some(edit) = e.time {
            song = song
                .edited(tracks, edit, self.project_bpm)
                .map_err(|r| match r {
                    Refused::BadRange => "song list: the song has no room for that".to_string(),
                    r => format!("song list: nothing changed: {}", refused_said(r, at)),
                })?;
        }
        if let Some(t) = e.unmark {
            song.markers.retain(|m| m.at != t);
        }
        if let Some(m) = &e.marker {
            if !song.markers.iter().any(|x| x.at == m.at) {
                let name = m.name.as_deref().unwrap_or_default();
                song.mark(m.at, name)
                    .map_err(|_| "song list: the song has too many parts".to_string())?;
            }
            let x = song
                .markers
                .iter_mut()
                .find(|x| x.at == m.at)
                .expect("just put there");
            if let Some(name) = &m.name {
                x.name.clone_from(name);
            }
            if let Some(from) = m.scene {
                x.scene = Some(from);
            }
            x.passes = m.passes;
        }
        if let Some(r) = e.remake {
            let blocks = offline::play_part(&self.project, r.row, r.passes, r.at, bar)
                .map_err(|why| format!("song list: nothing changed: {why}"))?;
            song.fill(tracks, r.at, r.at + r.len, &blocks)
                .map_err(|_| "song list: nothing changed: the song would hold too many blocks")?;
        }
        song.check(tracks)
            .map_err(|e| format!("song list: nothing changed ({e:?})"))?;
        self.project.arrangement = song;
        self.list_pick = e.pick;
        // The sections and blocks moved: the Song view's picks name
        // others now.
        self.selected = None;
        self.section = None;
        self.tempo_pick = None;
        Ok((e.said, Some(SongChange::List)))
    }
}
