//! The note editor, the Sequencer tab's Notes view (the MPC's Grid View
//! and List Edit; it was the Arrange page's until the Play, Pads and
//! Sequencer redesign): one clip's notes through one of three lenses ([`Lens`]): a
//! piano roll, a list, or a tracker (one row per step, empty steps
//! included). The tools run along the bottom, and beside it the Edit
//! options: Timing Correct; the selection's nudge, transpose, velocity,
//! length, chance, condition and ratchet; and the Euclidean fill.
//!
//! [`Editor`] is the editor's state (the clip, the selection, the lens,
//! the tool, the snap, the Timing Correct settings, the sound being
//! worked on and the Euclidean pattern). [`Editor::act`]
//! turns an [`EditAction`] into an [`Edit`] from [`mpfree_seq::notes`],
//! which the app applies as one undoable take, then calls
//! [`Editor::applied`] so the selection follows the notes.
//!
//! [`NoteGrid`] draws the clip and reports touches:
//!
//! - a note (on the roll, or a row of the list or the tracker):
//!   `Select(index)`;
//! - empty space on the roll: `Cell { row: note, col: tick }`, the tick
//!   under the finger snapped down to the snap's line;
//! - an empty step of the tracker: `Cell { row: `[`PEN`]`, col: tick }`,
//!   the step's line (the app draws the editor's sound there);
//! - a note dragged: when the finger lifts, `Marker { index, frame }`,
//!   the move packed into `frame` ([`drop_frame`], [`drop_of`]);
//! - the velocity lane: `Value(v)`, 0 to 1, as the finger goes down and
//!   moves;
//! - the buttons along the bottom: `Tab(k)` for [`EDIT_BUTTONS`]`[k]`
//!   (the last, Edit, opens the Edit options' drawer: the page's own);
//! - the view moved (a pinch, two fingers, a finger on the ruler, the
//!   keys or the overview, Fit): `View`, when the fingers lift, so the
//!   page can keep it and Snap Auto follow it. The view itself
//!   ([`zoom::RollView`]) is the screen's alone: nothing of it goes to the
//!   time engine but the division Snap Auto picks
//!   ([`EditAction::AutoSnap`]).
//!
//! On the roll, a note played with pad pressure shows it as a lighter
//! contour along its bottom, as high as the pressure, from where each
//! value was played to the next (`NoteGrid::touch`).
//!
//! [`edit_rows`] fills the Edit options list; its `Step` and `Select`
//! become [`EditAction::Row`] and [`EditAction::PickRow`].

pub mod gesture;
pub mod zoom;

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gesture::Pinch;
use zoom::RollView;

use mpfree_seq::notes::{self, Snap, Tc, TcKind, Which, DIVISIONS};
use mpfree_seq::steps::{Edit, Euclid, Grid, MAX_EUCLID_STEPS};
use mpfree_seq::{Clip, Cond, Join, Lock, Note, TouchPoint};
use mpfree_transport::time::{Tick, PPQ};
use mpfree_ui::widgets::OptionRow;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The buttons along the bottom, left to right. Some show a state in
/// place of their name (see [`NoteGrid::button_label`]).
pub const EDIT_BUTTONS: [&str; 8] = [
    "‹ Steps",
    "Grid",
    "Tool",
    "Snap",
    "All",
    "Duplicate",
    "Delete",
    "Edit",
];
/// [`EDIT_BUTTONS`]' last: Edit, which opens the Edit options' drawer
/// (the page's own; [`crate::view::TimePage::edit_drawer`]).
pub const EDIT_BUTTON: usize = 7;
/// The Edit options, top to bottom. The first six are Timing Correct's
/// settings (TC applies them); the next seven change the selection at
/// once; the last four are the Euclidean fill: the sound it writes, then
/// its hits, steps and rotation, each turn writing the pattern.
pub const EDIT_ROWS: [&str; 17] = [
    "Correct",
    "Division",
    "Swing",
    "Window",
    "Strength",
    "Shift",
    "Nudge",
    "Transpose",
    "Velocity",
    "Length",
    "Chance",
    "Condition",
    "Ratchet",
    "Euclid",
    "Hits",
    "Steps",
    "Rotate",
];
/// The first of the Euclidean fill's rows in [`EDIT_ROWS`].
pub const EUCLID_ROW: usize = 13;
/// A tracker step tapped: `Cell`'s row says the editor's sound.
pub const PEN: usize = usize::MAX;
/// The step the tracker and the Euclidean fill use when the snap is off
/// or finer than a grid allows (1/32): a 1/16.
pub const STEP_FALLBACK: Tick = PPQ as Tick / 4;
/// [`Editor::snap`]: no snap.
pub const SNAP_OFF: usize = DIVISIONS.len();
/// [`Editor::snap`]: Snap Auto, the snap following the roll's zoom
/// ([`zoom::auto_division`]) in straight divisions ...
pub const SNAP_AUTO: usize = DIVISIONS.len() + 1;
/// ... or in triplets (Auto triplets).
pub const SNAP_AUTO_T: usize = DIVISIONS.len() + 2;
/// The snaps the Snap button steps through: the divisions, off, Auto and
/// Auto triplets.
pub const SNAPS: usize = DIVISIONS.len() + 3;
/// The snap a new editor starts with: Auto (piano roll plan 1A.1).
pub const DEFAULT_SNAP: usize = SNAP_AUTO;
/// The division Snap Auto starts on, and Timing Correct's: 1/16.
const SIXTEENTH: usize = 6;
/// The length of a note drawn with no snap: a 1/16.
const FREE_LEN: Tick = PPQ as Tick / 4;
/// The velocity a drawn note gets.
const DRAW_VELOCITY: u8 = 100;
/// Join keys, so turning a row or dragging the velocity lane makes one
/// undoable take.
const JOIN: u64 = 0xED17_0000_0000;

/// How the editor shows the clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lens {
    /// The piano roll, with the velocity lane.
    #[default]
    Grid,
    /// One row per note, in time order.
    List,
    /// One row per step at the snap, empty steps too, with every note's
    /// chance, condition, ratchet and locks.
    Tracker,
}

impl Lens {
    pub fn name(self) -> &'static str {
        match self {
            Lens::Grid => "Grid",
            Lens::List => "List",
            Lens::Tracker => "Tracker",
        }
    }

    fn next(self) -> Lens {
        match self {
            Lens::Grid => Lens::List,
            Lens::List => Lens::Tracker,
            Lens::Tracker => Lens::Grid,
        }
    }
}

/// What a tap on the roll does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Taps select; a drag moves the selection.
    #[default]
    Select,
    /// A tap on empty space puts a note there.
    Draw,
    /// A tap on a note takes it out.
    Erase,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Draw => "Draw",
            Tool::Erase => "Erase",
        }
    }

    fn next(self) -> Tool {
        match self {
            Tool::Select => Tool::Draw,
            Tool::Draw => Tool::Erase,
            Tool::Erase => Tool::Select,
        }
    }
}

/// A touch on the editor, for the app to carry out (see the module notes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditAction {
    /// Open the editor on the clip the Play page's steps show.
    Open,
    /// Button `k` of [`EDIT_BUTTONS`].
    Button(usize),
    /// A note tapped, by its index in the clip, on the picture of
    /// revision `rev` ([`crate::EditInfo::rev`]). The app refuses it if
    /// the clip has moved on since, so a tap never lands on another note
    /// (piano roll plan 0A.1).
    Note { index: usize, rev: u64 },
    /// Empty space on the roll tapped.
    Empty { tick: Tick, note: u8 },
    /// An empty step of the tracker tapped, by its line.
    Step(Tick),
    /// The selection dragged by this many ticks and notes.
    Drop { ticks: Tick, notes: i32 },
    /// The velocity lane: the selection's velocity, 0 to 1.
    Velocity(f32),
    /// An Edit option stepped.
    Row { row: usize, delta: i32 },
    /// An Edit option picked (the wheel steps it).
    PickRow(usize),
    /// The wheel: the list's or the tracker's rows (the roll scrolls on
    /// the screen).
    Scroll(i32),
    /// Snap Auto's division (an index into [`DIVISIONS`]), as the roll's
    /// zoom picks it ([`zoom::auto_division`]): sent when it changes, once
    /// the fingers lift. Not an edit.
    AutoSnap(usize),
    /// Timing Correct: the TC button, and Correct in the Edit options'
    /// drawer. The picked notes, or all of them with none picked.
    Correct,
}

/// What [`Editor::act`] asks of the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Done {
    /// The change to the clip (empty for none), and its join key.
    pub edit: Edit,
    pub join: Option<u64>,
    /// A line for the log (empty for none).
    pub line: String,
    /// Close the editor.
    pub close: bool,
    /// After the edit, select the notes it put in.
    select_added: bool,
    /// The selected notes before the edit that stay selected if the edit
    /// leaves them alone (empty when the selection starts again).
    kept: Vec<Note>,
    /// After the edit, select every note of this sound (the Euclidean
    /// fill), in place of the above.
    select_sound: Option<u8>,
}

impl Done {
    fn line(line: String) -> Done {
        Done {
            line,
            ..Done::default()
        }
    }
}

/// The note editor's state (see the module notes).
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    pub track: usize,
    pub slot: usize,
    /// The selected notes, as indexes into the clip's notes.
    pub sel: Vec<usize>,
    pub lens: Lens,
    pub tool: Tool,
    /// Index into the divisions, or [`SNAP_OFF`], [`SNAP_AUTO`] or
    /// [`SNAP_AUTO_T`].
    pub snap: usize,
    /// Snap Auto's division now, an index into [`DIVISIONS`]: the screen
    /// picks it from the roll's zoom ([`EditAction::AutoSnap`]).
    pub auto: usize,
    pub tc: Tc,
    /// The Edit option picked (the wheel steps it).
    pub row: usize,
    /// The roll's lowest note.
    pub low: u8,
    /// The list's or the tracker's first row.
    pub first: usize,
    /// The sound being worked on (a pad or note number): the last note
    /// tapped or drawn. A tracker step tapped with Draw gets it, and the
    /// Euclidean fill writes it.
    pub sound: u8,
    pub euclid: Euclid,
    /// Something else changed the clip and took a picked note with it, so
    /// the picks were cleared ([`Editor::follow`]); the legend says so
    /// until the next touch.
    pub cleared: bool,
}

impl Editor {
    /// The editor on clip `slot` of `track`: the roll starts at the C
    /// under the clip's lowest note, or at the lowest note itself when
    /// starting from the C would leave its highest notes off the roll
    /// (about [`ROLL_ROWS`] rows show); or on the pads.
    pub fn new(track: usize, slot: usize, clip: &Clip) -> Editor {
        let low = if pads(clip) {
            0
        } else {
            let min = clip.notes.iter().map(|n| n.note).min().unwrap_or(48);
            let max = clip.notes.iter().map(|n| n.note).max().unwrap_or(48);
            let c = min - min % 12;
            let low = if max >= c + ROLL_ROWS { min } else { c };
            low.min(128 - ROLL_ROWS)
        };
        // The lowest sound the clip plays, or the first pad, or C3.
        let sound = clip
            .notes
            .iter()
            .map(|n| n.note)
            .min()
            .unwrap_or(if pads(clip) { 0 } else { 60 });
        Editor {
            track,
            slot,
            sel: Vec::new(),
            lens: Lens::Grid,
            tool: Tool::Select,
            snap: DEFAULT_SNAP,
            auto: SIXTEENTH,
            tc: Tc::default(),
            row: 0,
            low,
            first: 0,
            sound,
            euclid: Euclid::default(),
            cleared: false,
        }
    }

    /// The division the snap is now: its own, Snap Auto's, or none.
    fn division(&self) -> Option<usize> {
        match self.snap {
            SNAP_AUTO | SNAP_AUTO_T => Some(self.auto),
            s => (s < DIVISIONS.len()).then_some(s),
        }
    }

    /// The snap in ticks, or 0 for none.
    pub fn snap_ticks(&self) -> Tick {
        self.division()
            .and_then(|d| DIVISIONS.get(d))
            .map_or(0, |d| d.0)
    }

    /// The snap as its chip says it: "1/16", "Off", "Auto · 1/16" or,
    /// with Auto triplets, "Auto T · 1/16T".
    pub fn snap_name(&self) -> String {
        let name = self
            .division()
            .and_then(|d| DIVISIONS.get(d))
            .map_or("Off", |d| d.1);
        match self.snap {
            SNAP_AUTO => format!("Auto · {name}"),
            SNAP_AUTO_T => format!("Auto T · {name}"),
            _ => name.to_string(),
        }
    }

    /// Snap Auto is on (straight or triplets): the roll's zoom picks the
    /// division.
    pub fn snap_auto(&self) -> bool {
        matches!(self.snap, SNAP_AUTO | SNAP_AUTO_T)
    }

    /// The steps of the tracker and the Euclidean fill: the snap, or a
    /// 1/16 when the snap is off or finer than 1/32 (the legend says so).
    pub fn grid(&self) -> Grid {
        step_grid(self.snap_ticks())
    }

    fn snap_mode(&self) -> Snap {
        match self.snap_ticks() {
            0 => Snap::Off,
            d => Snap::Absolute(d),
        }
    }

    /// A step of a length or a duplicate: the snap, or a 1/16 with none.
    fn step(&self) -> Tick {
        match self.snap_ticks() {
            0 => FREE_LEN,
            d => d,
        }
    }

    /// The selection, in range and in order.
    fn picked(&self, clip: &Clip) -> Vec<usize> {
        let mut s: Vec<usize> = self
            .sel
            .iter()
            .copied()
            .filter(|&i| i < clip.notes.len())
            .collect();
        s.sort_unstable();
        s.dedup();
        s
    }

    /// Carry out `a` on `clip` (see [`Done`]). Only selection and
    /// settings change here; the clip changes when the app applies the
    /// edit.
    pub fn act(&mut self, a: EditAction, clip: &Clip) -> Done {
        self.cleared = false;
        let sel = self.picked(clip);
        let kept: Vec<Note> = sel.iter().map(|&i| clip.notes[i]).collect();
        let none = sel.is_empty();
        let base = JOIN | (self.track as u64) << 24 | (self.slot as u64) << 8;
        let key = move |k: u64| Some(base | k);
        // The edit, its join key, whether the notes it puts in are
        // selected after, whether the selected notes it leaves alone stay
        // selected, and the log line.
        let (edit, join, select_added, keep, line) = match a {
            EditAction::Open => return Done::default(),
            EditAction::Button(0) => {
                return Done {
                    close: true,
                    line: "edit: closed".into(),
                    ..Done::default()
                }
            }
            EditAction::Button(1) => {
                self.lens = self.lens.next();
                // The tracker opens on the first picked note's step.
                self.first = match (self.lens, sel.first()) {
                    (Lens::Tracker, Some(&i)) => {
                        let rows = track_rows(&clip.notes, clip.length, self.grid());
                        rows.iter()
                            .position(|r| r.note == Some(i))
                            .map_or(0, |k| k.saturating_sub(2))
                    }
                    _ => 0,
                };
                return Done::line(format!("edit: {}", self.lens.name().to_lowercase()));
            }
            EditAction::Button(2) => {
                self.tool = self.tool.next();
                return Done::line(format!("edit: tool {}", self.tool.name()));
            }
            EditAction::Button(3) => {
                self.snap = (self.snap + 1) % SNAPS;
                // Snap Auto's division moved onto its kind (1/16 and
                // 1/16T): the roll asks for the one its zoom wants after.
                let triplet = matches!(self.auto, 3 | 5 | 7 | 9 | 11);
                self.auto = match self.snap {
                    SNAP_AUTO_T if !triplet && self.auto >= 2 => self.auto + 1,
                    SNAP_AUTO if triplet => self.auto - 1,
                    _ => self.auto,
                };
                return Done::line(format!("edit: snap {}", self.snap_name()));
            }
            EditAction::Button(4) => {
                self.sel = if none {
                    (0..clip.notes.len()).collect()
                } else {
                    Vec::new()
                };
                return Done::line(format!("edit: {} selected", self.sel.len()));
            }
            // The copies are selected, so Duplicate again carries on.
            EditAction::Button(5) => (
                notes::duplicate(clip, &sel, self.step()),
                None,
                true,
                false,
                format!("edit: {} notes duplicated", sel.len()),
            ),
            EditAction::Button(6) => (
                notes::delete(clip, &sel),
                None,
                false,
                false,
                format!("edit: {} notes deleted", sel.len()),
            ),
            EditAction::Correct => {
                let which = if none {
                    Which::All
                } else {
                    Which::Selected(sel.clone())
                };
                let e = notes::timing_correct(clip, self.tc, &which);
                let n = e.add.len();
                // The selection stays as it was: the corrected notes when
                // some were selected, none after correcting them all.
                (
                    e,
                    None,
                    !none,
                    true,
                    format!("edit: timing correct moved {n} notes"),
                )
            }
            EditAction::Button(_) => return Done::default(),
            EditAction::Note { index: i, .. } if i < clip.notes.len() => match self.tool {
                Tool::Erase => (
                    notes::delete(clip, &[i]),
                    None,
                    false,
                    true,
                    format!("edit: note {} erased", i + 1),
                ),
                // A tap on a selected note keeps the group, so a drag
                // moves it; on another note it selects that one. Its sound
                // is the one worked on.
                _ => {
                    if !sel.contains(&i) {
                        self.sel = vec![i];
                    }
                    self.sound = clip.notes[i].note;
                    return Done::default();
                }
            },
            EditAction::Note { .. } => return Done::default(),
            EditAction::Empty { tick, note } => match self.tool {
                Tool::Draw if tick < clip.length => {
                    let n = Note::new(tick, self.step(), note.min(127), DRAW_VELOCITY);
                    self.sound = n.note;
                    (
                        notes::add(clip, n),
                        None,
                        true,
                        false,
                        format!("edit: note {} drawn at tick {tick}", note_name(note)),
                    )
                }
                Tool::Select => {
                    self.sel.clear();
                    return Done::default();
                }
                _ => return Done::default(),
            },
            EditAction::Step(tick) => match self.tool {
                // The editor's sound, a step long, on the step's line.
                Tool::Draw if (0..clip.length).contains(&tick) => {
                    let n = Note::new(tick, self.grid().size(), self.sound, DRAW_VELOCITY);
                    (
                        notes::add(clip, n),
                        None,
                        true,
                        false,
                        format!("edit: {} drawn at tick {tick}", self.sound_name(clip)),
                    )
                }
                Tool::Select => {
                    self.sel.clear();
                    return Done::default();
                }
                _ => return Done::default(),
            },
            EditAction::Drop { ticks, notes: by } => (
                notes::shift(clip, &sel, ticks, by),
                None,
                true,
                true,
                format!("edit: {} notes moved", sel.len()),
            ),
            // One a drag of the velocity lane (sent when the finger lifts),
            // so each drag is its own take.
            EditAction::Velocity(v) => (
                notes::set_velocity(clip, &sel, lane_velocity(v)),
                None,
                true,
                true,
                String::new(),
            ),
            EditAction::PickRow(r) => {
                self.row = r.min(EDIT_ROWS.len() - 1);
                return Done::default();
            }
            EditAction::Row { row, delta } => {
                self.row = row.min(EDIT_ROWS.len() - 1);
                let d = delta.signum();
                match row {
                    0..=5 => {
                        self.step_tc(row, d);
                        return Done::default();
                    }
                    6 => (
                        notes::nudge(clip, &sel, i64::from(d), self.snap_mode()),
                        key(6),
                        true,
                        true,
                        String::new(),
                    ),
                    7 => (
                        notes::transpose(clip, &sel, d),
                        key(7),
                        true,
                        true,
                        String::new(),
                    ),
                    8 => (
                        notes::velocity(clip, &sel, 4 * d),
                        key(8),
                        true,
                        true,
                        String::new(),
                    ),
                    9 => (
                        notes::edit_end(clip, &sel, Tick::from(d) * self.step()),
                        key(9),
                        true,
                        true,
                        String::new(),
                    ),
                    10 => (
                        notes::chance(clip, &sel, 5 * d),
                        key(10),
                        true,
                        true,
                        String::new(),
                    ),
                    11 => (
                        notes::condition(clip, &sel, d),
                        key(11),
                        true,
                        true,
                        String::new(),
                    ),
                    12 => (
                        notes::ratchet(clip, &sel, d),
                        key(12),
                        true,
                        true,
                        String::new(),
                    ),
                    // The sound the fill writes: its notes are picked, so
                    // the roll shows which.
                    13 => {
                        let top = if pads(clip) { 15 } else { 127 };
                        self.sound = (i32::from(self.sound) + d).clamp(0, top) as u8;
                        self.sel = (0..clip.notes.len())
                            .filter(|&i| clip.notes[i].note == self.sound)
                            .collect();
                        return Done::line(format!("edit: euclid on {}", self.sound_name(clip)));
                    }
                    14..=16 => {
                        let mut e = self.euclid;
                        match row {
                            14 => {
                                e.hits = (i32::from(e.hits) + d).clamp(0, i32::from(e.steps)) as u8
                            }
                            15 => {
                                let steps = i32::from(e.steps) + d;
                                e.steps = steps.clamp(1, i32::from(MAX_EUCLID_STEPS)) as u8;
                            }
                            _ => {
                                let steps = i32::from(e.steps.max(1));
                                e.rotate = (i32::from(e.rotate) + d).rem_euclid(steps) as u8;
                            }
                        }
                        let e = e.fit();
                        self.euclid = e;
                        // New hits play as loud as the sound's first note.
                        let velocity = clip
                            .notes
                            .iter()
                            .filter(|n| n.note == self.sound)
                            .min_by_key(|n| n.tick)
                            .map_or(DRAW_VELOCITY, |n| n.velocity);
                        let edit = self.grid().euclid(clip, self.sound, e, velocity);
                        return Done {
                            edit,
                            // One key for the three rows: a run of turns is
                            // one take.
                            join: key(14),
                            line: String::new(),
                            close: false,
                            select_added: false,
                            kept: Vec::new(),
                            select_sound: Some(self.sound),
                        };
                    }
                    _ => return Done::default(),
                }
            }
            EditAction::AutoSnap(d) => {
                if d < DIVISIONS.len() {
                    self.auto = d;
                }
                return Done::default();
            }
            EditAction::Scroll(d) => {
                let rows = match self.lens {
                    Lens::Grid => 0,
                    Lens::List => clip.notes.len(),
                    Lens::Tracker => track_rows(&clip.notes, clip.length, self.grid()).len(),
                };
                if self.lens == Lens::Grid {
                    self.low = (i32::from(self.low) + d).clamp(0, 127) as u8;
                } else {
                    let last = rows.saturating_sub(1) as i64;
                    self.first = (self.first as i64 + i64::from(d)).clamp(0, last) as usize;
                }
                return Done::default();
            }
        };
        Done {
            edit,
            join,
            line,
            close: false,
            select_added,
            kept: if keep { kept } else { Vec::new() },
            select_sound: None,
        }
    }

    /// The editor's sound as the rows name it: a pad or a note.
    fn sound_name(&self, clip: &Clip) -> String {
        sound_name(self.sound, pads(clip))
    }

    /// After the app applied `done`'s edit to `clip`: the selection is the
    /// notes the edit put in (when it selects them) and the selected notes
    /// it left alone.
    pub fn applied(&mut self, clip: &Clip, done: &Done) {
        if let Some(s) = done.select_sound {
            self.sel = (0..clip.notes.len())
                .filter(|&i| clip.notes[i].note == s)
                .collect();
            return;
        }
        let mut want: Vec<Note> = Vec::new();
        let mut erased = done.edit.erase.clone();
        for n in &done.kept {
            match erased.iter().position(|e| e == n) {
                Some(j) => {
                    erased.swap_remove(j);
                }
                None => want.push(*n),
            }
        }
        if done.select_added {
            want.extend(done.edit.add.iter().copied());
        }
        let mut sel: Vec<usize> = find_notes(clip, &want).into_iter().flatten().collect();
        sel.sort_unstable();
        self.sel = sel;
    }

    /// After something other than the editor's own edit changed the clip
    /// from `was` to `now` (a recorded note, UNDO, an edit from Steps):
    /// each pick follows its note, found again by its values. If a picked
    /// note is gone, every pick is cleared and [`Editor::cleared`] says
    /// so, since half a selection would surprise. True when cleared.
    pub fn follow(&mut self, was: &Clip, now: &Clip) -> bool {
        let want: Vec<Note> = self.picked(was).iter().map(|&i| was.notes[i]).collect();
        let found = find_notes(now, &want);
        if found.iter().all(Option::is_some) {
            let mut sel: Vec<usize> = found.into_iter().flatten().collect();
            sel.sort_unstable();
            self.sel = sel;
            false
        } else {
            self.sel.clear();
            self.cleared = true;
            true
        }
    }

    fn step_tc(&mut self, row: usize, d: i32) {
        let tc = &mut self.tc;
        match row {
            0 => {
                const KINDS: [TcKind; 4] =
                    [TcKind::Start, TcKind::End, TcKind::Length, TcKind::Legato];
                let at = KINDS.iter().position(|&k| k == tc.kind).unwrap_or(0) as i32;
                tc.kind = KINDS[(at + d).rem_euclid(4) as usize];
            }
            1 => {
                let at = DIVISIONS
                    .iter()
                    .position(|v| v.0 == tc.division)
                    .unwrap_or(SIXTEENTH) as i32;
                let to = (at + d).clamp(0, DIVISIONS.len() as i32 - 1) as usize;
                tc.division = DIVISIONS[to].0;
            }
            2 => tc.swing = (i32::from(tc.swing) + d).clamp(50, 75) as u8,
            3 => tc.window = (i32::from(tc.window) + 5 * d).clamp(0, 100) as u8,
            4 => tc.strength = (i32::from(tc.strength) + 5 * d).clamp(0, 100) as u8,
            _ => tc.shift = (tc.shift + Tick::from(d)).clamp(-(PPQ as Tick), PPQ as Tick),
        }
    }
}

/// Where each of `want` is in `clip`, by value, each of the clip's notes
/// matched at most once (two notes equal in every way sound the same, so
/// either will do); None for one that isn't there. The clip's notes are
/// sorted by start and pitch once, so a big selection costs a sort, not a
/// search of the clip per note.
fn find_notes(clip: &Clip, want: &[Note]) -> Vec<Option<usize>> {
    if want.is_empty() {
        return Vec::new();
    }
    let key = |n: &Note| (n.tick, n.note);
    let mut order: Vec<usize> = (0..clip.notes.len()).collect();
    order.sort_unstable_by_key(|&i| key(&clip.notes[i]));
    let mut used = vec![false; clip.notes.len()];
    want.iter()
        .map(|w| {
            let k = key(w);
            let from = order.partition_point(|&i| key(&clip.notes[i]) < k);
            let at = order[from..]
                .iter()
                .copied()
                .take_while(|&i| key(&clip.notes[i]) == k)
                .find(|&i| !used[i] && clip.notes[i] == *w);
            if let Some(i) = at {
                used[i] = true;
            }
            at
        })
        .collect()
}

/// The Edit options as the list shows them, for `ed` on `clip`.
pub fn edit_rows(ed: &Editor, clip: &Clip) -> Vec<OptionRow> {
    let sel: Vec<&Note> = ed.picked(clip).iter().map(|&i| &clip.notes[i]).collect();
    let tc = ed.tc;
    let division = DIVISIONS
        .iter()
        .find(|d| d.0 == tc.division)
        .map_or_else(|| format!("{} ticks", tc.division), |d| d.1.to_string());
    let kind = match tc.kind {
        TcKind::Start => "Start",
        TcKind::End => "End",
        TcKind::Length => "Length",
        TcKind::Legato => "Legato",
    };
    let of_sel = |f: &dyn Fn(&Note) -> i64, show: &dyn Fn(i64) -> String| -> String {
        match sel.first() {
            None => "pick notes".into(),
            Some(n) if sel.iter().all(|m| f(m) == f(n)) => show(f(n)),
            Some(_) => "mixed".into(),
        }
    };
    let values = [
        kind.to_string(),
        division,
        format!("{} %", tc.swing),
        format!("{} %", tc.window),
        format!("{} %", tc.strength),
        format!("{} ticks", tc.shift),
        if sel.is_empty() {
            "pick notes".into()
        } else {
            // The division alone ("by 1/16", Snap Auto's too), to fit.
            match ed.snap_ticks() {
                0 => "by Off".into(),
                t => format!("by {}", step_name(t)),
            }
        },
        of_sel(&|n| i64::from(n.note), &|v| note_name(v as u8)),
        of_sel(&|n| i64::from(n.velocity), &|v| v.to_string()),
        of_sel(&|n| n.len, &length_name),
        of_sel(&|n| i64::from(n.chance), &|v| format!("{v} %")),
        of_sel(&|n| cond_index(n.cond), &|v| {
            cond_name(Cond::all()[v as usize]).to_string()
        }),
        of_sel(&|n| i64::from(n.ratchet), &|v| hits_name(v as u8)),
        sound_name(ed.sound, pads(clip)),
        ed.euclid.hits.to_string(),
        format!("{} × {}", ed.euclid.steps, step_name(ed.grid().size())),
        ed.euclid.rotate.to_string(),
    ];
    EDIT_ROWS
        .iter()
        .zip(values)
        .map(|(name, value)| OptionRow {
            name: name.to_string(),
            value,
        })
        .collect()
}

/// A trig condition as the editor names it ("Always", "1:2", "FILL" ...).
pub fn cond_name(c: Cond) -> String {
    match c {
        Cond::Always => "Always".into(),
        c => c.to_string(),
    }
}

/// Where `c` is in [`Cond::all`] (0 for one out of range).
fn cond_index(c: Cond) -> i64 {
    Cond::all().iter().position(|&a| a == c).unwrap_or(0) as i64
}

/// A ratchet: "1 hit", "3 hits".
pub fn hits_name(n: u8) -> String {
    if n == 1 {
        "1 hit".into()
    } else {
        format!("{n} hits")
    }
}

/// A sound as the editor names it: "Pad 3" on a pad clip, else the note.
pub fn sound_name(n: u8, pads: bool) -> String {
    if pads {
        format!("Pad {}", u16::from(n) + 1)
    } else {
        note_name(n)
    }
}

/// The steps for a snap of `snap` ticks: the snap, or [`STEP_FALLBACK`]
/// when it is off (0) or finer than 1/32.
pub fn step_grid(snap: Tick) -> Grid {
    Grid::new(snap)
        .or_else(|| Grid::new(STEP_FALLBACK))
        .expect("a 1/16 is a step size")
}

/// A step size as a division's name ("1/16"), else in ticks.
pub fn step_name(size: Tick) -> String {
    length_name(size)
}

/// A row of the tracker: the step, and the note on it (an index into the
/// clip's notes) or None for an empty step. `first` marks a step's first
/// row; the rest of its notes follow on rows of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackRow {
    pub step: usize,
    pub note: Option<usize>,
    pub first: bool,
}

/// Something with a tick and a sound: the clip's notes, or the editor's
/// boxes for them.
pub trait Placed {
    fn tick(&self) -> Tick;
    fn sound(&self) -> u8;
}

impl Placed for Note {
    fn tick(&self) -> Tick {
        self.tick
    }
    fn sound(&self) -> u8 {
        self.note
    }
}

/// The tracker's rows for `notes` in a clip `length` ticks long, cut into
/// `grid`'s steps: every step in order, its notes lowest sound first (then
/// earliest), an empty step as one row of its own.
pub fn track_rows<N: Placed>(notes: &[N], length: Tick, grid: Grid) -> Vec<TrackRow> {
    let steps = grid.steps(length);
    let mut on: Vec<Vec<(u8, Tick, usize)>> = vec![Vec::new(); steps];
    for (i, n) in notes.iter().enumerate() {
        let (step, nudge) = grid.place(length, n.tick());
        if let Some(v) = on.get_mut(step) {
            v.push((n.sound(), nudge, i));
        }
    }
    let mut rows = Vec::with_capacity(steps + notes.len());
    for (step, mut v) in on.into_iter().enumerate() {
        if v.is_empty() {
            rows.push(TrackRow {
                step,
                note: None,
                first: true,
            });
            continue;
        }
        v.sort_unstable();
        for (k, &(_, _, i)) in v.iter().enumerate() {
            rows.push(TrackRow {
                step,
                note: Some(i),
                first: k == 0,
            });
        }
    }
    rows
}

/// A count of locks: "1 lock", "3 locks".
fn locks_name(n: usize) -> String {
    if n == 1 {
        "1 lock".into()
    } else {
        format!("{n} locks")
    }
}

/// The clip plays the pads (every note below 16, or none): the roll shows
/// 16 pad rows.
pub fn pads(clip: &Clip) -> bool {
    clip.notes.iter().all(|n| n.note < 16)
}

/// A MIDI note's name, C3 being 60 (the MPC's way).
pub fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[usize::from(n % 12)], i32::from(n / 12) - 2)
}

/// A length as a division's name where it is one, else in ticks.
pub fn length_name(len: i64) -> String {
    DIVISIONS
        .iter()
        .find(|d| d.0 == len)
        .map_or_else(|| format!("{len} ticks"), |d| d.1.to_string())
}

/// A clip tick as bar.beat.tick (from 1.1.0), `bar` ticks to the bar.
pub fn position_name(tick: Tick, bar: Tick) -> String {
    let beat = PPQ as Tick;
    let bar = bar.max(1);
    format!(
        "{}.{}.{:03}",
        tick / bar + 1,
        (tick % bar) / beat + 1,
        tick % beat
    )
}

/// A drag packed into a `Marker`'s frame: ticks in the low 32 bits,
/// notes in the next 16 (each offset so it is never negative).
pub fn drop_frame(ticks: Tick, notes: i32) -> u64 {
    let t = (ticks.clamp(-(1 << 30), 1 << 30) + (1 << 31)) as u64;
    let n = (notes.clamp(-127, 127) + 128) as u64;
    n << 32 | t
}

/// The velocity the lane's value `v` (0 to 1, [`EditAction::Velocity`])
/// sets: 1 to 127.
pub fn lane_velocity(v: f32) -> u8 {
    (1.0 + v.clamp(0.0, 1.0) * 126.0).round() as u8
}

/// The drag a `Marker`'s frame holds ([`drop_frame`]).
pub fn drop_of(frame: u64) -> (Tick, i32) {
    let t = (frame & 0xFFFF_FFFF) as Tick - (1 << 31);
    let n = ((frame >> 32) & 0xFFFF) as i32 - 128;
    (t, n)
}

// ---------------------------------------------------------------- the widget

/// A note as the editor shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteBox {
    pub tick: Tick,
    pub len: Tick,
    pub note: u8,
    pub velocity: u8,
    pub chance: u8,
    pub cond: Cond,
    pub ratchet: u8,
    pub selected: bool,
    /// Its pad pressure in [`NoteGrid::touch`] (none: `len` 0).
    pub touch: mpfree_seq::Touch,
    /// Its step is on; an off note keeps its place and plays nothing
    /// ([`Note::on`]), so the roll draws it as an outline.
    pub on: bool,
    /// Its ratchet's velocity ramp ([`Note::ramp`]): the roll shades each
    /// hit as it will play.
    pub ramp: i8,
    /// How it starts ([`Note::join`]): a tie or a slide is marked at its
    /// start.
    pub join: Join,
}

impl NoteBox {
    /// The box for `n`, its pressure left out.
    pub fn of(n: &Note, selected: bool) -> NoteBox {
        NoteBox {
            tick: n.tick,
            len: n.len,
            note: n.note,
            velocity: n.velocity,
            chance: n.chance,
            cond: n.cond,
            ratchet: n.ratchet,
            selected,
            touch: mpfree_seq::Touch::NONE,
            on: n.on,
            ramp: n.ramp,
            join: n.join,
        }
    }
}

impl Placed for NoteBox {
    fn tick(&self) -> Tick {
        self.tick
    }
    fn sound(&self) -> u8 {
        self.note
    }
}

/// A lock as the tracker counts it: its tick and its row (None for the
/// whole step).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockMark {
    pub tick: Tick,
    pub sound: Option<u8>,
}

impl LockMark {
    pub fn of(l: &Lock) -> LockMark {
        LockMark {
            tick: l.tick,
            sound: l.sound,
        }
    }
}

/// The roll's notes in buckets a bar long, each note in every bar it
/// sounds in (cut at the clip's end), so a roll that shows part of the
/// clip looks only at the notes near it (piano roll plan 0C.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoteIndex {
    bar: Tick,
    buckets: Vec<Vec<u32>>,
}

impl NoteIndex {
    /// The index of `notes` in a clip `length` long, in buckets `bar`
    /// long.
    pub fn new(notes: &[NoteBox], length: Tick, bar: Tick) -> NoteIndex {
        let bar = bar.max(1);
        let length = length.max(1);
        let count = ((length + bar - 1) / bar) as usize;
        let mut buckets = vec![Vec::new(); count];
        for (i, n) in notes.iter().enumerate() {
            let start = n.tick.clamp(0, length - 1);
            let end = (n.tick + n.len.max(1)).clamp(start + 1, length);
            let (first, last) = ((start / bar) as usize, ((end - 1) / bar) as usize);
            for b in &mut buckets[first..=last.min(count - 1)] {
                b.push(i as u32);
            }
        }
        NoteIndex { bar, buckets }
    }

    /// The notes in the bars that ticks `from..to` touch, each once, in
    /// index order (the roll's drawing order, the last on top). It may
    /// name notes just outside the ticks: the caller checks each place.
    pub fn near(&self, from: Tick, to: Tick) -> Vec<usize> {
        let Some(last) = self.buckets.len().checked_sub(1) else {
            return Vec::new();
        };
        if to <= from {
            return Vec::new();
        }
        let first = (from.max(0) / self.bar) as usize;
        let end = ((to - 1).max(0) / self.bar) as usize;
        let mut out: Vec<usize> = self
            .buckets
            .get(first..=end.min(last))
            .into_iter()
            .flatten()
            .flatten()
            .map(|&i| i as usize)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// What the grid works out from its notes once and keeps while they stay
/// the same (piano roll plan 0C.2): the tracker's rows, the list's order
/// and the roll's note index, each worked out when first wanted. A grid
/// built again ([`NoteGrid::show`]) starts without them, and each is
/// checked against what it was worked out from (the notes' count and
/// where they are kept, the clip's length, the bar and the tracker's
/// step), so a grid given other notes works them out again. Change a
/// grid's notes in place and call [`NoteGrid::forget`].
#[derive(Clone, Debug, Default)]
struct Kept {
    rows: RefCell<Option<(Stamp, Arc<[TrackRow]>)>>,
    order: RefCell<Option<(Stamp, Arc<[usize]>)>>,
    index: RefCell<Option<(Stamp, Arc<NoteIndex>)>>,
}

/// What [`Kept`]'s answers were worked out from.
type Stamp = (usize, usize, Tick, Tick, Tick);

/// A cache: never part of what a grid shows.
impl PartialEq for Kept {
    fn eq(&self, _: &Kept) -> bool {
        true
    }
}

/// `cell`'s answer if it was worked out from `stamp`, else `make`'s, kept.
fn kept<T: ?Sized>(
    cell: &RefCell<Option<(Stamp, Arc<T>)>>,
    stamp: Stamp,
    make: impl FnOnce() -> Arc<T>,
) -> Arc<T> {
    let mut c = cell.borrow_mut();
    match &*c {
        Some((s, v)) if *s == stamp => Arc::clone(v),
        _ => {
            let v = make();
            *c = Some((stamp, Arc::clone(&v)));
            v
        }
    }
}

/// The roll or the list (see the module notes).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteGrid {
    pub title: String,
    pub legend: String,
    pub notes: Vec<NoteBox>,
    /// The clip's length and a bar, in clip ticks.
    pub length: Tick,
    pub bar: Tick,
    /// What the roll shows and how big (piano roll plan 1A.1): the
    /// screen's own, kept for each clip by the page and moved here by a
    /// pinch, two fingers, the ruler, the keys, the overview or Fit.
    pub view: RollView,
    /// The Edit options' drawer is open (the page's): the Edit button is
    /// lit.
    pub drawer: bool,
    /// Pad rows (Pad 1 to 16) in place of note names.
    pub pads: bool,
    pub playhead: Option<Tick>,
    /// The snap in ticks (0: none), for the drag and the lines.
    pub snap: Tick,
    pub snap_name: String,
    /// The snap is Snap Auto: a drag moves notes by whole snaps, each
    /// keeping its distance from the grid (a fixed snap puts the dragged
    /// note's start on a line, as before).
    pub snap_auto: bool,
    pub tool: Tool,
    pub lens: Lens,
    /// The list's or the tracker's first row.
    pub first: usize,
    pub color: Color,
    /// The tracker's step, in ticks ([`step_grid`]).
    pub step: Tick,
    /// The clip's locks, for the tracker's Locks column.
    pub locks: Vec<LockMark>,
    /// The notes' pad pressure, each [`NoteBox::touch`] a run of it, timed
    /// in clip ticks from its note's start (as `Clip::touch`).
    pub touch: Vec<TouchPoint>,
    drag: Option<Drag>,
    /// What the fingers that aren't dragging a note are doing.
    hand: Hand,
    /// When the overview was last tapped, for a double tap.
    overview_tap: Option<Instant>,
    kept: Kept,
}

/// What fingers on the roll are doing, besides dragging a note or the
/// velocity lane ([`Drag`]).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hand {
    Idle,
    /// A finger on empty roll: its tap acts when it lifts (so a pinch
    /// starting there draws nothing), unless it moved away first.
    Wait {
        finger: u64,
        at: (i32, i32),
        now: (i32, i32),
        cell: (u8, Tick),
        moved: bool,
    },
    /// One finger scrolling on the ruler, the keys or the overview.
    Scroll {
        finger: u64,
        on: Strip,
        at: (i32, i32),
        now: (i32, i32),
        from: RollView,
        moved: bool,
    },
    /// Two fingers moving the view ([`gesture`]).
    Pinch(Pinch),
    /// A finger left from a pinch: it does nothing until it lifts, or a
    /// second finger joins it for another pinch.
    Spent {
        finger: u64,
        now: (i32, i32),
    },
}

/// Where one finger scrolls the roll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strip {
    /// In time.
    Ruler,
    /// In pitch.
    Keys,
    /// The window over the whole clip: a drag moves it, a tap jumps, a
    /// double tap fits the whole clip.
    Overview,
}

/// A finger moved this far (pixels) on the roll is no tap (the plan's
/// 24 px, about 3 mm).
const TAP_SLOP: i32 = 24;
/// A finger moved this far on a strip starts scrolling.
const STRIP_SLOP: i32 = 6;
/// Two taps on the overview this close together are a double tap.
const DOUBLE_TAP: Duration = Duration::from_millis(400);

/// A note being dragged: by which finger, which note, where the finger
/// went down, and the move so far (snapped).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    /// Only this finger moves or drops it ([`NoteGrid::touch`]).
    finger: u64,
    index: usize,
    x: i32,
    y: i32,
    /// Where the finger is now (a second finger joining starts a pinch
    /// from here).
    now: (i32, i32),
    ticks: Tick,
    notes: i32,
    /// The finger is in the velocity lane: the value under it, 0 to 1,
    /// shown on the picked notes and sent once, when it lifts.
    lane: Option<f32>,
}

const KEYS_W: i32 = 64;
const RULER_H: i32 = 22;
/// The overview strip over the ruler, with Fit under the keys and Follow
/// at its right end.
const OVERVIEW_H: i32 = 32;
const FOLLOW_W: i32 = 104;
const VEL_H: i32 = 64;
const TOOLS_H: i32 = 48;
const GAP: i32 = 6;
/// About how many rows the roll shows in the note editor.
pub const ROLL_ROWS: u8 = 22;
const LIST_H: i32 = 40;
/// A tracker row: as tall as the list's, for a fingertip (plan 1A.1).
const TRACK_H: i32 = 40;
/// The tracker's columns. A note's row fills them all; an empty step's
/// row shows its position and dots.
const TRACK_COLS: [(&str, i32); 9] = [
    ("Step", 82),
    ("Note", 76),
    ("Vel", 50),
    ("Length", 88),
    ("Nudge", 64),
    ("Chance", 72),
    ("Cond", 64),
    ("Ratchet", 74),
    ("Locks", 84),
];
/// The list's columns: number, position, note, velocity, length, chance.
const LIST_COLS: [(&str, i32); 6] = [
    ("#", 48),
    ("Position", 130),
    ("Note", 90),
    ("Velocity", 100),
    ("Length", 120),
    ("Chance", 90),
];

impl Default for NoteGrid {
    fn default() -> NoteGrid {
        NoteGrid {
            title: "Edit".into(),
            legend: String::new(),
            notes: Vec::new(),
            length: 4 * PPQ as Tick,
            bar: 4 * PPQ as Tick,
            view: RollView::default(),
            drawer: false,
            pads: false,
            playhead: None,
            snap: PPQ as Tick / 4,
            snap_name: "1/16".into(),
            snap_auto: false,
            tool: Tool::Select,
            lens: Lens::Grid,
            first: 0,
            color: crate::view::track_color(2),
            step: STEP_FALLBACK,
            locks: Vec::new(),
            touch: Vec::new(),
            drag: None,
            hand: Hand::Idle,
            overview_tap: None,
            kept: Kept::default(),
        }
    }
}

struct Geo {
    /// The overview strip (empty off the roll), Fit under the keys and
    /// Follow at its right end.
    overview: Rect,
    fit: Rect,
    follow: Rect,
    ruler: Rect,
    keys: Rect,
    roll: Rect,
    vel: Rect,
    tools: Rect,
    /// The view kept within this roll ([`RollView::clamp`]).
    v: RollView,
}

/// The velocity lane's value at `y`, 0 (its bottom) to 1 (its top).
fn lane_value(g: &Geo, y: i32) -> f32 {
    f32::from((g.vel.bottom() - 3 - y).clamp(0, g.vel.h - 6) as u16) / (g.vel.h - 6).max(1) as f32
}

impl NoteGrid {
    pub fn new() -> NoteGrid {
        NoteGrid::default()
    }

    /// It shows what `other` shows (a drag in progress aside).
    pub fn same(&self, other: &NoteGrid) -> bool {
        self.notes == other.notes
            && self.playhead == other.playhead
            && self.title == other.title
            && self.legend == other.legend
            && (self.length, self.bar, self.pads, self.drawer)
                == (other.length, other.bar, other.pads, other.drawer)
            && self.view == other.view
            && (self.snap, self.tool, self.lens, self.first)
                == (other.snap, other.tool, other.lens, other.first)
            && (&self.snap_name, self.snap_auto) == (&other.snap_name, other.snap_auto)
            && self.color == other.color
            && self.step == other.step
            && self.locks == other.locks
            && self.touch == other.touch
    }

    /// Note `i`'s pressure, empty for none.
    pub fn touch_of(&self, i: usize) -> &[TouchPoint] {
        self.notes
            .get(i)
            .and_then(|n| self.touch.get(n.touch.range()))
            .unwrap_or(&[])
    }

    /// Show what `other` shows, keeping what the fingers are doing.
    pub fn show(&mut self, other: NoteGrid) {
        let (drag, hand, overview_tap) = (self.drag, self.hand, self.overview_tap);
        *self = NoteGrid {
            drag,
            hand,
            overview_tap,
            ..other
        };
    }

    fn geo(&self, r: Rect) -> Geo {
        // Matches PaintCx::titled_panel with the default theme's padding.
        let body = r.inset(14).take_top(34).1;
        let (tools, rest) = body.take_bottom(TOOLS_H);
        let rest = Rect::new(rest.x, rest.y, rest.w, rest.h - GAP);
        let roll_lens = self.lens == Lens::Grid;
        let (vel, rest) = if !roll_lens {
            (Rect::new(rest.x, rest.bottom(), rest.w, 0), rest)
        } else {
            let (v, rest) = rest.take_bottom(VEL_H);
            (v, Rect::new(rest.x, rest.y, rest.w, rest.h - GAP))
        };
        let (strip, rest) = if roll_lens {
            let (o, rest) = rest.take_top(OVERVIEW_H);
            (o, Rect::new(rest.x, rest.y + 4, rest.w, rest.h - 4))
        } else {
            (Rect::new(rest.x, rest.y, rest.w, 0), rest)
        };
        let (ruler, rest) = rest.take_top(RULER_H);
        let (keys, roll) = rest.take_left(KEYS_W);
        let follow = Rect::new(roll.right() - FOLLOW_W, strip.y, FOLLOW_W, strip.h);
        Geo {
            overview: Rect::new(roll.x, strip.y, (roll.w - FOLLOW_W - 6).max(0), strip.h),
            fit: Rect::new(keys.x, strip.y, keys.w - 6, strip.h),
            follow: if roll_lens {
                follow
            } else {
                Rect::new(0, 0, 0, 0)
            },
            ruler: Rect::new(roll.x, ruler.y, roll.w, ruler.h),
            keys,
            roll,
            vel: Rect::new(roll.x, vel.y, roll.w, vel.h),
            tools,
            v: self.view.clamped(roll, self.length, self.rows()),
        }
    }

    /// How many rows the roll has: 16 pads, or 128 notes.
    fn rows(&self) -> u8 {
        if self.pads {
            16
        } else {
            128
        }
    }

    /// The x of clip tick `t` on the roll.
    fn x_of(&self, g: &Geo, t: Tick) -> i32 {
        g.v.x_of(g.roll, t)
    }

    /// The note of the row at `y`, if it is on the roll.
    fn note_at(&self, g: &Geo, y: i32) -> Option<u8> {
        g.v.note_at(g.roll, y, self.rows())
    }

    /// The y of the top of `note`'s row, if any of it shows.
    fn y_of(&self, g: &Geo, note: u8) -> Option<i32> {
        let y = g.v.row_top(g.roll, note);
        (note < self.rows() && y + g.v.row_h > g.roll.y && y < g.roll.bottom()).then_some(y)
    }

    /// The view a clip opens with on this roll, in area `r` (the page
    /// keeps it for the clip after: [`RollView::open`]).
    pub fn open_view(&self, r: Rect) -> RollView {
        let roll = NoteGrid {
            lens: Lens::Grid,
            ..NoteGrid::default()
        }
        .geo(r)
        .roll;
        RollView::open(
            self.notes.iter().map(|n| (n.tick, n.note)),
            self.length,
            self.bar,
            roll,
            self.rows(),
        )
    }

    /// The view as this roll in `r` shows it (kept within the clip).
    pub fn view_in(&self, r: Rect) -> RollView {
        self.geo(r).v
    }

    /// The roll's area in `r`.
    pub fn roll_rect(&self, r: Rect) -> Rect {
        self.geo(r).roll
    }

    /// Snap Auto's division at this roll's zoom in `r`, coming from
    /// division `prev` ([`zoom::auto_division`]).
    pub fn auto_division(&self, r: Rect, prev: usize, triplets: bool) -> usize {
        zoom::auto_division(prev, self.view_in(r).ppt, triplets)
    }

    /// A finger is on the roll doing something (a drag, a pinch, a
    /// scroll, a tap not yet lifted): the page leaves Snap Auto alone.
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.hand != Hand::Idle
    }

    /// Page the view on to show tick `t` while Follow is on, in area `r`:
    /// true if it moved.
    pub fn follow_to(&mut self, r: Rect, t: Tick) -> bool {
        if !self.view.follow || self.busy() {
            return false;
        }
        let g = self.geo(r);
        let mut v = g.v;
        let moved = v.follow_to(
            g.roll,
            self.length,
            self.rows(),
            t.rem_euclid(self.length.max(1)),
        );
        if moved {
            self.view = v;
        }
        moved
    }

    /// Scroll the roll `d` rows up (down if negative): SHIFT + wheel.
    pub fn scroll_rows(&mut self, r: Rect, d: i32) {
        let g = self.geo(r);
        let mut v = g.v;
        v.lift += d * v.row_h;
        self.view = v.clamped(g.roll, self.length, self.rows());
    }

    /// Where note `i` is drawn on the roll, if it shows (moved by a drag
    /// in progress when it is selected).
    pub fn note_rect(&self, r: Rect, i: usize) -> Option<Rect> {
        self.note_rect_in(&self.geo(r), i)
    }

    /// [`NoteGrid::note_rect`] in the roll's places `g`, worked out once
    /// for every note drawn or touched.
    fn note_rect_in(&self, g: &Geo, i: usize) -> Option<Rect> {
        let n = self.notes.get(i)?;
        let (dt, dn) = match self.drag {
            Some(d) if n.selected && d.lane.is_none() => (d.ticks, d.notes),
            _ => (0, 0),
        };
        let note = u8::try_from(i32::from(n.note) + dn).ok()?;
        let y = self.y_of(g, note)?;
        let tick = (n.tick + dt).rem_euclid(self.length.max(1));
        let x0 = self.x_of(g, tick);
        let x1 = self.x_of(g, (tick + n.len).min(self.length)).max(x0 + 3);
        Some(Rect::new(x0, y + 1, x1 - x0, g.v.row_h - 2))
    }

    /// Where row `k` of the list or the tracker is (0 is the heading), if
    /// it shows.
    pub fn list_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let g = self.geo(r);
        let area = Rect::new(
            g.keys.x,
            g.ruler.y,
            g.keys.w + g.roll.w,
            g.roll.bottom() - g.ruler.y,
        );
        let h = if self.lens == Lens::Tracker {
            TRACK_H
        } else {
            LIST_H
        };
        let y = area.y + k as i32 * h;
        (y + h <= area.bottom()).then(|| Rect::new(area.x, y, area.w, h))
    }

    /// The tracker's steps.
    fn grid(&self) -> Grid {
        step_grid(self.step)
    }

    /// What the kept answers are worked out from ([`Kept`]).
    fn stamp(&self) -> Stamp {
        (
            self.notes.len(),
            self.notes.as_ptr() as usize,
            self.length,
            self.bar,
            self.step,
        )
    }

    /// Drop what was worked out from the notes ([`Kept`]), after changing
    /// them in place.
    pub fn forget(&mut self) {
        self.kept = Kept::default();
    }

    /// The tracker's rows, worked out once for these notes.
    pub fn track_rows(&self) -> Arc<[TrackRow]> {
        kept(&self.kept.rows, self.stamp(), || {
            track_rows(&self.notes, self.length, self.grid()).into()
        })
    }

    /// The roll's note index, worked out once for these notes.
    pub fn index(&self) -> Arc<NoteIndex> {
        kept(&self.kept.index, self.stamp(), || {
            Arc::new(NoteIndex::new(&self.notes, self.length, self.bar))
        })
    }

    /// The notes the roll may show in `g`, in drawing order: every note
    /// while the whole clip is in view or a note is being dragged (it may
    /// be dragged into view), else the index's notes near the ticks in
    /// view.
    fn in_view(&self, g: &Geo) -> Vec<usize> {
        let (from, to) = (
            g.v.tick_at(g.roll, g.roll.x),
            g.v.tick_at(g.roll, g.roll.right()) + 1,
        );
        let dragging = self.drag.is_some_and(|d| d.lane.is_none());
        if dragging || (from <= 0 && to >= self.length) {
            return (0..self.notes.len()).collect();
        }
        self.index().near(from, to)
    }

    /// The buttons along the bottom.
    pub fn button_rects(&self, r: Rect) -> [Rect; 8] {
        let g = self.geo(r);
        let w = (g.tools.w - 7 * 8) / 8;
        let mut out = [Rect::new(0, 0, 0, 0); 8];
        for (k, b) in out.iter_mut().enumerate() {
            *b = Rect::new(g.tools.x + k as i32 * (w + 8), g.tools.y, w, g.tools.h);
        }
        out
    }

    /// What button `k` says: some show a state.
    pub fn button_label(&self, k: usize) -> String {
        let any = self.notes.iter().any(|n| n.selected);
        match k {
            1 => self.lens.name().into(),
            2 => self.tool.name().into(),
            // Snap Auto names itself ("Auto · 1/16").
            3 if self.snap_name.starts_with("Auto") => self.snap_name.clone(),
            3 => format!("Snap {}", self.snap_name),
            4 => if any { "None" } else { "All" }.into(),
            _ => EDIT_BUTTONS.get(k).copied().unwrap_or("").into(),
        }
    }

    fn row_name(&self, note: u8) -> String {
        sound_name(note, self.pads)
    }

    /// The tick a dragged note's start lands on: snapped to a line.
    fn snapped(&self, t: Tick) -> Tick {
        match self.snap {
            0 => t,
            s => (t as f64 / s as f64).round() as Tick * s,
        }
    }

    fn paint_roll(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let v = g.v;
        let rh = v.row_h;
        let was = cx.canvas.clip();
        // Rows: the black keys darker, C lines brighter; their names in
        // the key column.
        cx.canvas.set_clip(was.intersect(Rect::new(
            g.keys.x,
            g.roll.y,
            g.keys.w + g.roll.w,
            g.roll.h,
        )));
        for note in v.notes_shown(g.roll, self.rows()) {
            let y = v.row_top(g.roll, note);
            let black = !self.pads && matches!(note % 12, 1 | 3 | 6 | 8 | 10);
            let fill = if black { t.bg } else { t.surface };
            cx.canvas
                .fill_rect(Rect::new(g.roll.x, y, g.roll.w, rh), fill);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(g.keys.x, y, g.keys.w - 6, rh),
                Align::Right,
                if note % 12 == 0 || self.pads {
                    t.text
                } else {
                    t.text_faint
                },
                &self.row_name(note),
            );
            if !self.pads && note % 12 == 0 {
                cx.canvas
                    .fill_rect(Rect::new(g.roll.x, y + rh - 1, g.roll.w, 1), t.outline);
            }
        }
        // Lines: bars, beats, and the snap when they are far enough apart,
        // over the ticks in view only.
        cx.canvas.set_clip(was.intersect(g.roll));
        let beat = PPQ as Tick;
        let (from, to) = (
            v.tick_at(g.roll, g.roll.x),
            v.tick_at(g.roll, g.roll.right()).min(self.length),
        );
        let px = |ticks: Tick| ticks as f64 * v.ppt;
        let mut lines: Vec<(Tick, Color, i32)> = Vec::new();
        if self.snap > 0 && px(self.snap) >= 6.0 {
            lines.push((self.snap, t.outline.mix(t.surface, 0.6), 1));
        }
        if px(beat) >= 6.0 {
            lines.push((beat, t.outline, 1));
        }
        lines.push((self.bar.max(1), t.text_faint, 2));
        for (step, color, w) in lines {
            let mut at = from.div_euclid(step) * step;
            while at <= to {
                let x = self.x_of(g, at);
                cx.canvas
                    .fill_rect(Rect::new(x, g.roll.y, w, g.roll.h), color);
                at += step;
            }
        }
        // The clip's end, when it shows.
        if (from..=to).contains(&self.length) {
            let x = self.x_of(g, self.length);
            cx.canvas
                .fill_rect(Rect::new(x - 1, g.roll.y, 2, g.roll.h), t.text);
        }
        // The ruler: bar numbers as close as fit (every bar, or every 2nd,
        // 4th ...), and beats too once a beat is wide enough.
        cx.canvas.set_clip(was.intersect(g.ruler));
        let bar = self.bar.max(1);
        let mut every = 1;
        while px(bar * every) < 36.0 && every < 1 << 12 {
            every *= 2;
        }
        let beats = px(beat) >= 48.0;
        let mut at = from.div_euclid(beat) * beat;
        while at <= to {
            let b = at / bar;
            let label = if at % bar == 0 {
                (b % every == 0).then(|| format!("{}", b + 1))
            } else if beats {
                Some(format!("{}.{}", b + 1, at % bar / beat + 1))
            } else {
                None
            };
            if let Some(label) = label {
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    Rect::new(self.x_of(g, at) + 3, g.ruler.y, 60, g.ruler.h),
                    Align::Left,
                    if at % bar == 0 { t.text } else { t.text_faint },
                    &label,
                );
            }
            at += beat;
        }
        cx.canvas.set_clip(was);
        self.paint_overview(cx, g);
        cx.canvas.set_clip(was.intersect(g.roll));
        // The notes as they will play, then the velocity lane under them.
        let top = t.accent;
        let shade = |v: u8| t.raised.mix(self.color, 0.35 + 0.65 * f32::from(v) / 127.0);
        // The velocity lane dragged: the picked notes as they would be.
        let preview = self.drag.and_then(|d| d.lane).map(lane_velocity);
        let vel_of = |n: &NoteBox| match preview {
            Some(v) if n.selected => NoteBox { velocity: v, ..*n },
            _ => *n,
        };
        let shown = self.in_view(g);
        for &i in &shown {
            let Some(nr) = self.note_rect_in(g, i) else {
                continue;
            };
            let n = &vel_of(&self.notes[i]);
            let fill = shade(n.velocity);
            if !n.on {
                // Off: its place and settings kept, nothing played.
                cx.canvas.stroke_round_rect(nr, 3, 1, self.color);
            } else {
                for (hr, v) in self.hit_rects(g, nr, n) {
                    cx.canvas.fill_round_rect(hr, 3, shade(v));
                }
                self.paint_touch(cx, g, nr, i, fill.mix(t.text, 0.5));
            }
            self.paint_join(cx, g, nr, n.join, if n.on { fill } else { self.color });
            if n.selected {
                cx.canvas.stroke_round_rect(nr, 3, 2, top);
            }
        }
        cx.canvas.set_clip(was.intersect(Rect::new(
            g.keys.x,
            g.vel.y,
            g.keys.w + g.vel.w,
            g.vel.h,
        )));
        cx.canvas.fill_round_rect(g.vel, 6, t.surface);
        let lane = |v: u8| (f32::from(v) / 127.0 * (g.vel.h - 6) as f32).round() as i32;
        for &i in &shown {
            let n = &vel_of(&self.notes[i]);
            let x = self.x_of(g, n.tick);
            let h = lane(n.velocity);
            let c = if n.selected {
                top
            } else if n.on {
                self.color
            } else {
                t.outline
            };
            cx.canvas
                .fill_rect(Rect::new(x, g.vel.bottom() - 3 - h, 4, h.max(1)), c);
            // A ratchet's later hits, thinner, at their own velocities.
            let hits = i64::from(n.ratchet.max(1));
            for k in 1..hits {
                let at = n.tick + n.len * k / hits;
                if at >= self.length {
                    break;
                }
                let h = lane(mpfree_seq::ramped(n.velocity, n.ramp, k as u8, n.ratchet));
                cx.canvas.fill_rect(
                    Rect::new(self.x_of(g, at), g.vel.bottom() - 3 - h, 2, h.max(1)),
                    c.mix(t.surface, 0.3),
                );
            }
        }
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            Rect::new(g.keys.x, g.vel.y, g.keys.w - 6, g.vel.h),
            Align::Right,
            t.text_faint,
            "Vel",
        );
        if let Some(p) = self.playhead {
            let x = self.x_of(g, p.rem_euclid(self.length.max(1)));
            cx.canvas.set_clip(was.intersect(Rect::new(
                g.roll.x,
                g.ruler.y,
                g.roll.w,
                g.roll.bottom() - g.ruler.y,
            )));
            cx.canvas.fill_rect(
                Rect::new(x, g.ruler.y, 2, g.roll.bottom() - g.ruler.y),
                t.good,
            );
        }
        cx.canvas.set_clip(was);
    }

    /// The overview strip: the whole clip, its notes as marks, the part
    /// on the roll outlined, the playhead; Fit under the keys and Follow
    /// at its right end.
    fn paint_overview(&self, cx: &mut PaintCx, g: &Geo) {
        if g.overview.is_empty() {
            return;
        }
        let t = cx.theme.clone();
        let o = g.overview;
        cx.canvas.fill_round_rect(o, 6, t.surface);
        let len = self.length.max(1) as f64;
        let ox = |tick: Tick| {
            o.x + (tick.clamp(0, self.length) as f64 / len * f64::from(o.w)).round() as i32
        };
        // One mark a column, from its lowest note to its highest, so a
        // clip of 8,192 notes costs a column's worth of drawing.
        if let (Some(lo), Some(hi)) = (
            self.notes.iter().map(|n| n.note).min(),
            self.notes.iter().map(|n| n.note).max(),
        ) {
            let inner = (o.h - 8).max(1);
            let span = f64::from(hi - lo).max(1.0);
            let mut cols: Vec<Option<(i32, i32)>> = vec![None; o.w.max(0) as usize];
            for n in &self.notes {
                let y = o.bottom()
                    - 4
                    - (f64::from(n.note - lo) / span * f64::from(inner - 2)).round() as i32
                    - 2;
                let (a, b) = (ox(n.tick), ox(n.tick + n.len).max(ox(n.tick) + 1));
                for x in a.max(o.x)..b.min(o.right()) {
                    let c = &mut cols[(x - o.x) as usize];
                    *c = Some(c.map_or((y, y + 2), |(top, bot)| (top.min(y), bot.max(y + 2))));
                }
            }
            for (k, c) in cols.iter().enumerate() {
                if let Some((top, bot)) = c {
                    cx.canvas
                        .fill_rect(Rect::new(o.x + k as i32, *top, 1, bot - top), t.text_faint);
                }
            }
        }
        let shown = g.v.ticks_shown(g.roll);
        let (a, b) = (ox(g.v.start), ox(g.v.start + shown).max(ox(g.v.start) + 6));
        cx.canvas
            .stroke_round_rect(Rect::new(a, o.y, b - a, o.h), 4, 2, t.accent);
        if let Some(p) = self.playhead {
            let x = ox(p.rem_euclid(self.length.max(1)));
            cx.canvas.fill_rect(Rect::new(x, o.y, 2, o.h), t.good);
        }
        // Fit and Follow.
        for (r, label, lit) in [(g.fit, "Fit", false), (g.follow, "Follow", g.v.follow)] {
            let (fill, ink) = if lit {
                (t.picked(), t.on_picked())
            } else {
                (t.raised, t.text)
            };
            cx.canvas.fill_round_rect(r, 8, fill);
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                r,
                Align::Center,
                ink,
                label,
            );
        }
    }

    /// Note `n`'s hits in its box `nr`, each with the velocity it plays
    /// at: the whole box for a plain note; a ratchet's hits evenly spread
    /// over its length with a gap between them where there is room,
    /// shaded by its ramp ([`mpfree_seq::ramped`]). A hit past the clip's
    /// end is left out, as the box is cut there.
    /// Nothing is allocated: the roll asks for every note on every frame.
    fn hit_rects(&self, g: &Geo, nr: Rect, n: &NoteBox) -> impl Iterator<Item = (Rect, u8)> {
        let hits = i32::from(n.ratchet.max(1));
        let full = n.len as f64 * g.v.ppt;
        let split = hits > 1 && full >= f64::from(2 * hits);
        let (velocity, ramp, ratchet) = (n.velocity, n.ramp, n.ratchet);
        let x_of = move |k: i32| nr.x + (full * f64::from(k) / f64::from(hits)).round() as i32;
        (0..if split { hits } else { 1 }).map_while(move |k| {
            if !split {
                return Some((nr, velocity));
            }
            let (x0, x1) = (x_of(k), x_of(k + 1).min(nr.right()));
            let gap = i32::from(x1 - x0 >= 5);
            let v = mpfree_seq::ramped(velocity, ramp, k as u8, ratchet);
            (x0 < nr.right()).then(|| (Rect::new(x0, nr.y, (x1 - x0 - gap).max(1), nr.h), v))
        })
    }

    /// A joined note's mark, just before its start: a tie a level stub
    /// from the note it continues, a slide a short rising slant.
    fn paint_join(&self, cx: &mut PaintCx, g: &Geo, nr: Rect, join: Join, color: Color) {
        let mid = nr.y + nr.h / 2;
        let left = (nr.x - 8).max(g.roll.x);
        match join {
            Join::Attack => {}
            Join::Tie => {
                if nr.x > left {
                    cx.canvas
                        .fill_rect(Rect::new(left, mid - 1, nr.x - left, 3), color);
                }
            }
            Join::Slide => {
                for k in 0..4 {
                    let x = nr.x - 8 + 2 * k;
                    if x >= g.roll.x {
                        cx.canvas
                            .fill_rect(Rect::new(x, mid + 3 - 2 * k, 2, 2), color);
                    }
                }
            }
        }
    }

    /// Note `i`'s pressure in its box `nr`: from each value's tick to the
    /// next one's (the last to the note's end), as high as the value.
    fn paint_touch(&self, cx: &mut PaintCx, g: &Geo, nr: Rect, i: usize, color: Color) {
        let points = self.touch_of(i);
        let Some(n) = self.notes.get(i) else {
            return;
        };
        let px = |ticks: i64| {
            let x = nr.x as f64 + ticks as f64 * g.v.ppt;
            (x.round() as i32).clamp(nr.x, nr.right())
        };
        let room = (nr.h - 2).max(1);
        for (k, p) in points.iter().enumerate() {
            let from = i64::from(p.at);
            if from >= n.len {
                break;
            }
            let to = points
                .get(k + 1)
                .map_or(n.len, |q| i64::from(q.at).min(n.len));
            let (x0, x1) = (px(from), px(to));
            let h = (f32::from(p.value.min(127)) / 127.0 * room as f32).round() as i32;
            if h > 0 && x1 > x0 {
                cx.canvas
                    .fill_rect(Rect::new(x0, nr.bottom() - 1 - h, x1 - x0, h), color);
            }
        }
    }

    fn paint_list(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let Some(head) = self.list_rect(r, 0) else {
            return;
        };
        let mut x = head.x + 8;
        for (name, w) in LIST_COLS {
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                Rect::new(x, head.y, w, head.h),
                Align::Left,
                t.text_dim,
                name,
            );
            x += w;
        }
        // In time order, as the list shows them.
        let order = self.list_order();
        for (k, &i) in order.iter().skip(self.first).enumerate() {
            let Some(row) = self.list_rect(r, k + 1) else {
                break;
            };
            let n = &self.notes[i];
            if n.selected {
                cx.canvas
                    .fill_round_rect(row, 6, t.raised.mix(t.accent, 0.3));
            } else if k % 2 == 0 {
                cx.canvas.fill_rect(row, t.surface);
            }
            let cells = [
                format!("{}", i + 1),
                position_name(n.tick, self.bar),
                self.note_name(n),
                n.velocity.to_string(),
                length_name(n.len),
                format!("{} %", n.chance),
            ];
            // An off note's row is faint: it plays nothing.
            let ink = if n.on { t.text } else { t.text_faint };
            let mut x = row.x + 8;
            for ((_, w), text) in LIST_COLS.iter().zip(cells) {
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    Rect::new(x, row.y, *w, row.h),
                    Align::Left,
                    ink,
                    &text,
                );
                x += w;
            }
        }
    }

    /// A step's line as bar.beat, with the step in the beat after a third
    /// dot when steps are shorter than a beat ("2.3.2" is bar 2, beat 3,
    /// its second 1/16).
    fn step_name(&self, step: usize) -> String {
        let beat = PPQ as Tick;
        let bar = self.bar.max(1);
        let size = self.grid().size();
        let t = step as Tick * size;
        let head = format!("{}.{}", t / bar + 1, (t % bar) / beat + 1);
        if size < beat {
            format!("{head}.{}", (t % beat) / size + 1)
        } else {
            head
        }
    }

    /// The cells of tracker row `row`, and whether they are faint (an
    /// empty step, or a note whose step is off).
    fn track_cells(&self, row: &TrackRow) -> ([String; 9], bool) {
        let dot = || "·".to_string();
        let pos = if row.first {
            self.step_name(row.step)
        } else {
            String::new()
        };
        let grid = self.grid();
        let at_step = |tick: Tick| grid.place(self.length, tick).0 == row.step;
        let Some(i) = row.note else {
            // Trigless locks on an empty step still show.
            let locks = self.locks.iter().filter(|l| at_step(l.tick)).count();
            let lock = if locks > 0 { locks_name(locks) } else { dot() };
            return (
                [pos, dot(), dot(), dot(), dot(), dot(), dot(), dot(), lock],
                true,
            );
        };
        let n = &self.notes[i];
        let nudge = grid.place(self.length, n.tick).1;
        // The row's own locks on the step, and the whole step's on its
        // first row.
        let locks = self
            .locks
            .iter()
            .filter(|l| at_step(l.tick))
            .filter(|l| l.sound == Some(n.note) || (row.first && l.sound.is_none()))
            .count();
        let cells = [
            pos,
            self.note_name(n),
            n.velocity.to_string(),
            length_name(n.len),
            if nudge == 0 {
                dot()
            } else {
                format!("{nudge:+}")
            },
            if n.chance == 100 {
                dot()
            } else {
                format!("{} %", n.chance)
            },
            if n.cond == Cond::Always {
                dot()
            } else {
                n.cond.to_string()
            },
            match (n.ratchet, n.ramp) {
                (1, _) => dot(),
                (r, 0) => format!("×{r}"),
                (r, ramp) => format!("×{r} {ramp:+}"),
            },
            if locks == 0 { dot() } else { locks_name(locks) },
        ];
        // An off note's row is faint, as an empty step's: it plays nothing.
        (cells, !n.on)
    }

    /// A note's row name, with how it starts when it is joined and "off"
    /// when its step is off: "C3", "D3 tie", "E3 off".
    fn note_name(&self, n: &NoteBox) -> String {
        let mut name = self.row_name(n.note);
        for word in [n.join.word(), if n.on { "" } else { "off" }] {
            if !word.is_empty() {
                name.push(' ');
                name.push_str(word);
            }
        }
        name
    }

    fn paint_tracker(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let Some(head) = self.list_rect(r, 0) else {
            return;
        };
        let mut x = head.x + 8;
        for (name, w) in TRACK_COLS {
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                Rect::new(x, head.y, w, head.h),
                Align::Left,
                t.text_dim,
                name,
            );
            x += w;
        }
        let beat = PPQ as Tick;
        let bar = self.bar.max(1);
        let size = self.grid().size();
        let playing = self.playhead.map(|p| {
            self.grid()
                .place(self.length, p.rem_euclid(self.length.max(1)))
                .0
        });
        for (k, row) in self.track_rows().iter().skip(self.first).enumerate() {
            let Some(rr) = self.list_rect(r, k + 1) else {
                break;
            };
            let line = row.step as Tick * size;
            let picked = row.note.is_some_and(|i| self.notes[i].selected);
            if picked {
                cx.canvas
                    .fill_round_rect(rr, t.round(6), t.raised.mix(t.accent, 0.3));
            } else if row.first && line % beat == 0 {
                // Beats stand out, as a tracker's every fourth row does.
                cx.canvas.fill_rect(rr, t.surface);
            }
            if row.first && line % bar == 0 && row.step > 0 {
                cx.canvas
                    .fill_rect(Rect::new(rr.x, rr.y, rr.w, 1), t.text_faint);
            }
            if playing == Some(row.step) {
                cx.canvas
                    .fill_rect(Rect::new(rr.x, rr.y + 2, 4, rr.h - 4), t.good);
            }
            let (cells, faint) = self.track_cells(row);
            let mut x = rr.x + 8;
            for (c, ((_, w), text)) in TRACK_COLS.iter().zip(cells).enumerate() {
                let ink = if c == 0 {
                    if line % beat == 0 {
                        t.text_dim
                    } else {
                        t.text_faint
                    }
                } else if faint || text == "·" {
                    t.text_faint
                } else {
                    t.text
                };
                cx.text(
                    if c == 0 { FontId::Mono } else { FontId::Sans },
                    t.size_small - 1.0,
                    Rect::new(x, rr.y, *w, rr.h),
                    Align::Left,
                    ink,
                    &text,
                );
                x += w;
            }
        }
    }

    /// The notes' indexes in time order (then by note), as the list
    /// shows them, worked out once for these notes.
    pub fn list_order(&self) -> Arc<[usize]> {
        kept(&self.kept.order, self.stamp(), || {
            let mut order: Vec<usize> = (0..self.notes.len()).collect();
            order.sort_by_key(|&i| (self.notes[i].tick, self.notes[i].note));
            order.into()
        })
    }
}

impl Widget for NoteGrid {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.legend);
        let g = self.geo(r);
        match self.lens {
            Lens::Grid => self.paint_roll(cx, &g),
            Lens::List => self.paint_list(cx, r),
            Lens::Tracker => self.paint_tracker(cx, r),
        }
        for (k, br) in self.button_rects(r).iter().enumerate() {
            let lit = (k == 1 && self.lens != Lens::Grid)
                || (k == 2 && self.tool != Tool::Select)
                || (k == 7 && self.drawer);
            let fill = if lit { t.picked() } else { t.raised };
            let ink = if lit { t.on_picked() } else { t.text };
            cx.canvas.fill_round_rect(*br, t.round(10), fill);
            cx.text(
                FontId::Bold,
                t.size_small,
                *br,
                Align::Center,
                ink,
                &self.button_label(k),
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geo(r);
        let mut out: Vec<Target> = self
            .button_rects(r)
            .iter()
            .enumerate()
            .map(|(k, b)| Target::tap("button", k, self.button_label(k), *b))
            .collect();
        if matches!(self.lens, Lens::Tracker | Lens::List) {
            let id = if self.lens == Lens::Tracker {
                "tracker-row"
            } else {
                "list-row"
            };
            for (k, rr) in (1..).map_while(|k| self.list_rect(r, k).map(|rr| (k, rr))) {
                out.push(Target::tap(
                    id,
                    self.first + k - 1,
                    format!("row {}", self.first + k),
                    rr,
                ));
            }
            return out;
        }
        if !g.vel.is_empty() {
            out.push(Target::drag("velocity", 0, "velocity lane", g.vel).towards(0, -g.vel.h / 3));
        }
        if !g.overview.is_empty() {
            out.push(Target::tap("fit", 0, "Fit", g.fit));
            out.push(Target::tap("follow", 0, "Follow", g.follow));
            // The ruler and the overview scroll in time only while part
            // of the clip shows; the keys while part of the rows do.
            if g.v.ticks_shown(g.roll) < self.length {
                out.push(
                    Target::drag("overview", 0, "overview", g.overview)
                        .towards(g.overview.w / 4, 0),
                );
                out.push(Target::drag("ruler", 0, "ruler", g.ruler).towards(-g.ruler.w / 4, 0));
            }
            if i32::from(self.rows()) * g.v.row_h > g.roll.h {
                out.push(Target::drag("keys", 0, "keys", g.keys).towards(0, g.keys.h / 4));
            }
        }
        let shown = self.in_view(&g);
        for &i in &shown {
            if let Some(nr) = self.note_rect_in(&g, i) {
                out.push(Target::tap("note", i, format!("note {}", i + 1), nr));
            }
        }
        // An empty place on the roll adds a note: its top left corner,
        // where the sample clips leave room.
        let spot = Rect::new(g.roll.x + 2, g.roll.y + 2, 12, 12);
        let free = shown.iter().all(|&i| {
            self.note_rect_in(&g, i)
                .is_none_or(|nr| nr.intersect(spot).is_empty())
        });
        if free && !g.roll.is_empty() {
            out.push(Target::tap("roll", 0, "empty roll", spot));
        }
        out
    }

    /// Touches are followed finger by finger. One finger edits notes: a
    /// drag (a note or the velocity lane) is its finger's alone, and a tap
    /// on empty roll acts when it lifts. One finger on the ruler, the keys
    /// or the overview scrolls. A second finger anywhere on the roll turns
    /// whatever the first was doing into a pinch or a two-finger scroll
    /// with nothing sent (a drag stops where it was, a tap is no tap); the
    /// view moves and nothing goes to the time engine, but `View` when
    /// the fingers lift. A second finger on a button stops a drag and does
    /// nothing else.
    fn touch(&mut self, f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = self.geo(r);
        let mine = self.finger_now().is_some_and(|(o, ..)| o == f);
        let busy = self.finger_now().is_some() || matches!(self.hand, Hand::Pinch(_));
        match phase {
            Touch::Down if busy && !mine => {
                // A second finger, or a third.
                let on_roll = self.lens == Lens::Grid
                    && [g.roll, g.ruler, g.keys, g.overview, g.vel]
                        .iter()
                        .any(|a| a.contains(x, y));
                let first = self.finger_now();
                self.drag = None;
                self.hand = match (first, on_roll) {
                    (Some((a, ax, ay)), true) => {
                        Hand::Pinch(Pinch::new((a, ax, ay), (f, x, y), g.v))
                    }
                    (Some((a, ax, ay)), false) => Hand::Spent {
                        finger: a,
                        now: (ax, ay),
                    },
                    (None, _) => self.hand,
                };
                None
            }
            Touch::Move | Touch::Up | Touch::Cancel if busy && !mine && !self.pinched(f) => None,
            Touch::Down => {
                // This finger's own gesture that never heard its lift ends.
                self.drag = None;
                self.hand = Hand::Idle;
                self.down(f, x, y, r, &g)
            }
            Touch::Move => {
                if let Hand::Pinch(mut p) = self.hand {
                    p.moved(f, x, y);
                    self.view = p.view(g.roll, self.length, self.rows());
                    self.view.follow = false;
                    self.hand = Hand::Pinch(p);
                    return None;
                }
                match self.hand {
                    Hand::Wait {
                        finger,
                        at,
                        cell,
                        moved,
                        ..
                    } => {
                        let far = (x - at.0).abs().max((y - at.1).abs()) >= TAP_SLOP;
                        self.hand = Hand::Wait {
                            finger,
                            at,
                            now: (x, y),
                            cell,
                            moved: moved || far,
                        };
                        return None;
                    }
                    Hand::Scroll {
                        finger,
                        on,
                        at,
                        from,
                        moved,
                        ..
                    } => {
                        let moved = moved || (x - at.0).abs().max((y - at.1).abs()) >= STRIP_SLOP;
                        if moved {
                            self.view = self.scrolled(&g, on, from, (x - at.0, y - at.1));
                        }
                        self.hand = Hand::Scroll {
                            finger,
                            on,
                            at,
                            now: (x, y),
                            from,
                            moved,
                        };
                        return None;
                    }
                    Hand::Spent { finger, .. } => {
                        self.hand = Hand::Spent {
                            finger,
                            now: (x, y),
                        };
                        return None;
                    }
                    _ => {}
                }
                let d = self.drag?;
                if d.lane.is_some() {
                    self.drag = Some(Drag {
                        lane: Some(lane_value(&g, y)),
                        now: (x, y),
                        ..d
                    });
                    return None;
                }
                let n = self.notes.get(d.index)?;
                let raw = g.v.tick_at(g.roll, x) - g.v.tick_at(g.roll, d.x);
                let ticks = match self.snap {
                    s if self.snap_auto && s > 0 => (raw as f64 / s as f64).round() as Tick * s,
                    _ => self.snapped(n.tick + raw) - n.tick,
                };
                let notes = (d.y - y) / g.v.row_h.max(1);
                self.drag = Some(Drag {
                    ticks,
                    notes,
                    now: (x, y),
                    ..d
                });
                None
            }
            Touch::Up | Touch::Cancel => {
                let up = phase == Touch::Up;
                match self.hand {
                    Hand::Pinch(p) => {
                        // The other finger stays: it does nothing more
                        // until it lifts or another finger joins it.
                        self.hand = p.other(f).map_or(Hand::Idle, |(o, ox, oy)| Hand::Spent {
                            finger: o,
                            now: (ox, oy),
                        });
                        return Some(self.view_event());
                    }
                    Hand::Spent { .. } => {
                        self.hand = Hand::Idle;
                        return None;
                    }
                    Hand::Wait {
                        cell: (note, tick),
                        moved,
                        ..
                    } => {
                        self.hand = Hand::Idle;
                        return (up && !moved).then_some(EventKind::Cell {
                            row: usize::from(note),
                            col: tick as usize,
                        });
                    }
                    Hand::Scroll {
                        on,
                        at,
                        moved,
                        from,
                        ..
                    } => {
                        self.hand = Hand::Idle;
                        if !up {
                            return None;
                        }
                        if !moved && on == Strip::Overview {
                            self.overview_tapped(&g, at.0);
                        } else if !moved {
                            self.view = from;
                            return None;
                        }
                        return Some(self.view_event());
                    }
                    Hand::Idle => {}
                }
                let d = self.drag.take()?;
                if !up {
                    return None;
                }
                if let Some(v) = d.lane {
                    return Some(EventKind::Value(v));
                }
                ((d.ticks, d.notes) != (0, 0)).then(|| EventKind::Marker {
                    index: d.index,
                    frame: drop_frame(d.ticks, d.notes),
                })
            }
        }
    }
}

impl NoteGrid {
    /// The one finger at work (a drag, a tap waiting, a scroll, or the
    /// finger left from a pinch) and where it is now.
    fn finger_now(&self) -> Option<(u64, i32, i32)> {
        if let Some(d) = self.drag {
            return Some((d.finger, d.now.0, d.now.1));
        }
        match self.hand {
            Hand::Wait { finger, now, .. }
            | Hand::Scroll { finger, now, .. }
            | Hand::Spent { finger, now } => Some((finger, now.0, now.1)),
            Hand::Idle | Hand::Pinch(_) => None,
        }
    }

    /// Finger `f` is one of a pinch's.
    fn pinched(&self, f: u64) -> bool {
        matches!(self.hand, Hand::Pinch(p) if p.has(f))
    }

    /// The view as an event, for the page when the fingers lift.
    fn view_event(&self) -> EventKind {
        EventKind::View {
            start: self.view.start as f64,
            frames_per_px: 1.0 / self.view.ppt,
        }
    }

    /// The view `from` scrolled by a finger moved (`dx`, `dy`) on strip
    /// `on`: the ruler and the keys follow the finger; the overview moves
    /// its window, at the overview's scale. Follow goes off.
    fn scrolled(&self, g: &Geo, on: Strip, from: RollView, (dx, dy): (i32, i32)) -> RollView {
        let mut v = from;
        match on {
            Strip::Ruler => v.start = from.start - (f64::from(dx) / from.ppt).round() as Tick,
            Strip::Keys => v.lift = from.lift + dy,
            Strip::Overview => {
                let per_px = self.length.max(1) as f64 / f64::from(g.overview.w.max(1));
                v.start = from.start + (f64::from(dx) * per_px).round() as Tick;
            }
        }
        v.follow = false;
        v.clamped(g.roll, self.length, self.rows())
    }

    /// A tap on the overview at `x`: the window jumps to centre there, or,
    /// a second tap soon after, the whole clip fits.
    fn overview_tapped(&mut self, g: &Geo, x: i32) {
        let now = Instant::now();
        let double = self
            .overview_tap
            .is_some_and(|t| now.duration_since(t) <= DOUBLE_TAP);
        self.overview_tap = (!double).then_some(now);
        if double {
            self.view =
                g.v.zoom_time(g.roll, self.length, self.rows(), 0.0, 0.0, g.roll.x);
            return;
        }
        let per_px = self.length.max(1) as f64 / f64::from(g.overview.w.max(1));
        let at = (f64::from(x - g.overview.x) * per_px) as Tick;
        let mut v = g.v;
        v.start = at - v.ticks_shown(g.roll) / 2;
        self.view = v.clamped(g.roll, self.length, self.rows());
    }

    /// Fit: the picked notes when some are, else every note, across the
    /// roll and up it (an empty clip: the whole clip, the rows as they
    /// are).
    fn fit(&mut self, g: &Geo) {
        let picked = self.notes.iter().any(|n| n.selected);
        let which = || self.notes.iter().filter(move |n| !picked || n.selected);
        let lo = which().map(|n| n.note).min();
        let hi = which().map(|n| n.note).max();
        let (from, to) = if picked {
            (
                which().map(|n| n.tick).min().unwrap_or(0),
                which()
                    .map(|n| n.tick + n.len.max(1))
                    .max()
                    .unwrap_or(self.length),
            )
        } else {
            (0, self.length)
        };
        let rows = self.rows();
        self.view = match (lo, hi) {
            (Some(lo), Some(hi)) => g.v.fit(g.roll, self.length, rows, (from, to), (lo, hi)),
            _ => g.v.zoom_time(g.roll, self.length, rows, 0.0, 0.0, g.roll.x),
        };
    }

    /// A finger coming down alone.
    fn down(&mut self, f: u64, x: i32, y: i32, r: Rect, g: &Geo) -> Option<EventKind> {
        if let Some(k) = self.button_rects(r).iter().position(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(k));
        }
        if self.lens == Lens::Tracker {
            let rows = self.track_rows();
            let k = (1..)
                .map_while(|k| self.list_rect(r, k).map(|rr| (k, rr)))
                .find(|(_, rr)| rr.contains(x, y))?
                .0;
            let row = rows.get(self.first + k - 1)?;
            return Some(match row.note {
                Some(i) => EventKind::Select(i),
                None => EventKind::Cell {
                    row: PEN,
                    col: row.step * self.grid().size() as usize,
                },
            });
        }
        if self.lens == Lens::List {
            let order = self.list_order();
            let k = (1..)
                .map_while(|k| self.list_rect(r, k).map(|rr| (k, rr)))
                .find(|(_, rr)| rr.contains(x, y))?
                .0;
            return order.get(self.first + k - 1).map(|&i| EventKind::Select(i));
        }
        if g.fit.contains(x, y) {
            self.fit(g);
            return Some(self.view_event());
        }
        if g.follow.contains(x, y) {
            self.view = RollView {
                follow: !g.v.follow,
                ..g.v
            };
            return None;
        }
        let strip = if g.overview.contains(x, y) {
            Some(Strip::Overview)
        } else if g.ruler.contains(x, y) {
            Some(Strip::Ruler)
        } else if g.keys.contains(x, y) && y >= g.roll.y && y < g.roll.bottom() {
            Some(Strip::Keys)
        } else {
            None
        };
        if let Some(on) = strip {
            self.hand = Hand::Scroll {
                finger: f,
                on,
                at: (x, y),
                now: (x, y),
                from: g.v,
                moved: false,
            };
            return None;
        }
        if g.vel.contains(x, y) {
            // Shown as the finger moves, sent once when it lifts: one
            // edit, one UNDO.
            self.drag = Some(Drag {
                finger: f,
                index: 0,
                x,
                y,
                now: (x, y),
                ticks: 0,
                notes: 0,
                lane: Some(lane_value(g, y)),
            });
            return None;
        }
        if !g.roll.contains(x, y) {
            return None;
        }
        // The topmost note under the finger (drawn last).
        if let Some(i) = self
            .in_view(g)
            .into_iter()
            .rev()
            .find(|&i| self.note_rect_in(g, i).is_some_and(|nr| nr.contains(x, y)))
        {
            self.drag = Some(Drag {
                finger: f,
                index: i,
                x,
                y,
                now: (x, y),
                ticks: 0,
                notes: 0,
                lane: None,
            });
            return Some(EventKind::Select(i));
        }
        // Empty roll: the cell under the finger, sent when it lifts.
        let note = self.note_at(g, y)?;
        let tick = g.v.cell_at(g.roll, x, self.snap);
        if tick >= self.length {
            return None;
        }
        self.hand = Hand::Wait {
            finger: f,
            at: (x, y),
            now: (x, y),
            cell: (note, tick),
            moved: false,
        };
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(notes: &[(Tick, Tick, u8)]) -> Clip {
        let mut c = Clip::new("Bass", 4 * 960);
        for &(t, l, n) in notes {
            c.add_note(Note::new(t, l, n, 100)).unwrap();
        }
        c
    }

    /// Act, apply the edit the way the app does, and follow the selection.
    fn run(ed: &mut Editor, c: &mut Clip, a: EditAction) -> Done {
        let done = ed.act(a, c);
        c.apply(&done.edit).unwrap();
        ed.applied(c, &done);
        done
    }

    fn ticks_of(c: &Clip, sel: &[usize]) -> Vec<(Tick, u8)> {
        let mut v: Vec<(Tick, u8)> = sel
            .iter()
            .map(|&i| (c.notes[i].tick, c.notes[i].note))
            .collect();
        v.sort_unstable();
        v
    }

    /// Picks follow their notes when something else changes the clip: two
    /// equal notes are matched once each, a note moved in the clip's list
    /// is found again, and one picked note gone clears every pick and says
    /// so until the next touch.
    #[test]
    fn picks_follow_their_notes_or_are_cleared() {
        let was = clip(&[(0, 240, 60), (480, 240, 62), (480, 240, 62), (960, 240, 64)]);
        let mut ed = Editor::new(0, 0, &was);
        ed.sel = vec![1, 2, 3];
        // The first note taken out and a new one added at the end: the
        // picked notes are now at 0, 1 and 2.
        let mut now = was.clone();
        now.notes.remove(0);
        now.add_note(Note::new(0, 240, 67, 100)).unwrap();
        assert!(!ed.follow(&was, &now));
        assert_eq!(ed.sel, [0, 1, 2]);
        assert_eq!(ticks_of(&now, &ed.sel), [(480, 62), (480, 62), (960, 64)]);
        assert!(!ed.cleared);
        // One of the two equal notes taken out: one pick has nowhere to go.
        let mut gone = now.clone();
        gone.notes.remove(1);
        assert!(ed.follow(&now, &gone));
        assert!(ed.sel.is_empty());
        assert!(ed.cleared);
        // The next touch ends the notice.
        ed.act(EditAction::Scroll(1), &gone);
        assert!(!ed.cleared);
        // Nothing picked: nothing to clear.
        assert!(!ed.follow(&gone, &was));
        assert!(!ed.cleared);
    }

    #[test]
    fn a_drag_moves_the_selection_and_it_stays_selected() {
        let mut c = clip(&[(0, 240, 48), (480, 240, 50), (960, 240, 52)]);
        let mut ed = Editor::new(1, 0, &c);
        assert_eq!(ed.low, 48);
        // From G2 to D#4 does not fit from C2: the roll starts at G2.
        let wide = clip(&[(0, 240, 55), (480, 240, 75)]);
        assert_eq!(Editor::new(1, 0, &wide).low, 55);
        ed.act(EditAction::Note { index: 1, rev: 0 }, &c);
        assert_eq!(ed.sel, vec![1]);
        ed.act(EditAction::Button(4), &c);
        assert!(ed.sel.is_empty(), "All with some selected selects none");
        ed.act(EditAction::Button(4), &c);
        assert_eq!(ed.sel, vec![0, 1, 2]);
        // A tap on one of the group keeps the group.
        ed.act(EditAction::Note { index: 2, rev: 0 }, &c);
        assert_eq!(ed.sel.len(), 3);
        let done = run(
            &mut ed,
            &mut c,
            EditAction::Drop {
                ticks: 240,
                notes: 2,
            },
        );
        assert_eq!(done.line, "edit: 3 notes moved");
        assert_eq!(
            ticks_of(&c, &ed.sel),
            vec![(240, 50), (720, 52), (1200, 54)]
        );
        // An empty tap with the select tool clears it.
        ed.act(
            EditAction::Empty {
                tick: 3000,
                note: 60,
            },
            &c,
        );
        assert!(ed.sel.is_empty());
    }

    #[test]
    fn draw_erase_duplicate_and_delete() {
        let mut c = clip(&[(0, 240, 3)]);
        let mut ed = Editor::new(0, 0, &c);
        assert_eq!(ed.low, 0, "a pad clip starts on the pads");
        ed.act(EditAction::Note { index: 0, rev: 0 }, &c);
        ed.act(EditAction::Button(2), &c);
        assert_eq!(ed.tool, Tool::Draw);
        run(&mut ed, &mut c, EditAction::Empty { tick: 480, note: 5 });
        assert_eq!(c.notes.len(), 2);
        assert_eq!(
            ticks_of(&c, &ed.sel),
            vec![(480, 5)],
            "only the new note is selected"
        );
        assert_eq!(c.notes[1].len, 240, "a 1/16 long, the snap");
        // Past the clip's end: nothing.
        assert!(ed
            .act(
                EditAction::Empty {
                    tick: 4000,
                    note: 5
                },
                &c
            )
            .edit
            .is_empty());
        let done = run(&mut ed, &mut c, EditAction::Button(5));
        assert_eq!(done.line, "edit: 1 notes duplicated");
        assert_eq!(
            ticks_of(&c, &ed.sel),
            vec![(720, 5)],
            "the copy is selected"
        );
        run(&mut ed, &mut c, EditAction::Button(6));
        assert_eq!(c.notes.len(), 2);
        assert!(ed.sel.is_empty());
        ed.act(EditAction::Button(2), &c);
        assert_eq!(ed.tool, Tool::Erase);
        run(&mut ed, &mut c, EditAction::Note { index: 0, rev: 0 });
        assert_eq!(c.notes.len(), 1);
    }

    #[test]
    fn timing_correct_takes_the_selection_or_everything() {
        let mut c = clip(&[(10, 100, 36), (250, 100, 38), (500, 100, 40)]);
        let mut ed = Editor::new(0, 0, &c);
        ed.act(EditAction::Note { index: 1, rev: 0 }, &c);
        let done = run(&mut ed, &mut c, EditAction::Correct);
        assert_eq!(done.line, "edit: timing correct moved 1 notes");
        let mut ticks: Vec<Tick> = c.notes.iter().map(|n| n.tick).collect();
        ticks.sort_unstable();
        assert_eq!(ticks, vec![10, 240, 500]);
        assert_eq!(ticks_of(&c, &ed.sel), vec![(240, 38)]);
        ed.act(EditAction::Button(4), &c);
        assert!(ed.sel.is_empty());
        // Half strength, a 1/8 grid, with nothing selected: every note.
        ed.act(EditAction::Row { row: 4, delta: -1 }, &c);
        for _ in 0..9 {
            ed.act(EditAction::Row { row: 4, delta: -1 }, &c);
        }
        assert_eq!(ed.tc.strength, 50);
        ed.act(EditAction::Row { row: 1, delta: -1 }, &c);
        ed.act(EditAction::Row { row: 1, delta: -1 }, &c);
        assert_eq!(ed.tc.division, 480);
        run(&mut ed, &mut c, EditAction::Correct);
        let mut ticks: Vec<Tick> = c.notes.iter().map(|n| n.tick).collect();
        ticks.sort_unstable();
        assert_eq!(ticks, vec![5, 120, 490]);
    }

    #[test]
    fn the_rows_change_the_selection_and_join_one_take() {
        let mut c = clip(&[(0, 240, 60), (480, 240, 62)]);
        let mut ed = Editor::new(2, 3, &c);
        ed.act(EditAction::Button(4), &c);
        let a = run(&mut ed, &mut c, EditAction::Row { row: 7, delta: 1 });
        let b = run(&mut ed, &mut c, EditAction::Row { row: 7, delta: 1 });
        assert!(a.join.is_some() && a.join == b.join);
        assert_eq!(ticks_of(&c, &ed.sel), vec![(0, 62), (480, 64)]);
        run(&mut ed, &mut c, EditAction::Row { row: 6, delta: 1 });
        assert_eq!(ticks_of(&c, &ed.sel), vec![(240, 62), (720, 64)]);
        run(&mut ed, &mut c, EditAction::Row { row: 8, delta: -1 });
        assert!(c.notes.iter().all(|n| n.velocity == 96));
        run(&mut ed, &mut c, EditAction::Row { row: 9, delta: 1 });
        assert!(c.notes.iter().all(|n| n.len == 480));
        run(&mut ed, &mut c, EditAction::Velocity(1.0));
        assert!(c.notes.iter().all(|n| n.velocity == 127));
        let rows = edit_rows(&ed, &c);
        assert_eq!(rows.len(), EDIT_ROWS.len());
        assert_eq!(rows[7].value, "mixed");
        assert_eq!(rows[8].value, "127");
        assert_eq!(rows[9].value, "1/8");
        // With nothing selected the rows say so, and change nothing.
        ed.sel.clear();
        assert_eq!(edit_rows(&ed, &c)[8].value, "pick notes");
        assert!(ed
            .act(EditAction::Row { row: 7, delta: 1 }, &c)
            .edit
            .is_empty());
    }

    #[test]
    fn settings_buttons_and_scrolling() {
        let c = clip(&[(0, 240, 60)]);
        let mut ed = Editor::new(0, 0, &c);
        // Snap Auto first, on a 1/16 until the roll asks for another.
        assert_eq!(ed.snap_name(), "Auto · 1/16");
        assert!(ed.snap_auto());
        ed.act(EditAction::AutoSnap(4), &c);
        assert_eq!(
            (ed.snap_name().as_str(), ed.snap_ticks()),
            ("Auto · 1/8", 480)
        );
        // Auto triplets moves the division onto a triplet.
        ed.act(EditAction::Button(3), &c);
        assert_eq!(ed.snap_name(), "Auto T · 1/8T");
        ed.act(EditAction::AutoSnap(99), &c);
        assert_eq!(ed.snap_ticks(), 320, "no such division: kept");
        // Then the fixed snaps, coarse to fine, then off, then Auto again.
        ed.act(EditAction::Button(3), &c);
        assert_eq!(ed.snap_name(), "1 bar");
        for _ in 0..DIVISIONS.len() {
            ed.act(EditAction::Button(3), &c);
        }
        assert_eq!((ed.snap_name().as_str(), ed.snap_ticks()), ("Off", 0));
        ed.act(EditAction::Button(3), &c);
        assert_eq!(
            ed.snap_name(),
            "Auto · 1/8",
            "back on its straight division"
        );
        ed.act(EditAction::Button(3), &c);
        ed.act(EditAction::Button(3), &c);
        assert_eq!(ed.snap_name(), "1 bar");
        assert_eq!(ed.act(EditAction::Button(1), &c).line, "edit: list");
        assert_eq!(ed.lens, Lens::List);
        ed.act(EditAction::Scroll(5), &c);
        assert_eq!(ed.first, 0, "one note: the list cannot scroll");
        assert_eq!(ed.act(EditAction::Button(1), &c).line, "edit: tracker");
        // A bar of whole-bar steps (the snap is 1 bar): one row.
        ed.act(EditAction::Scroll(5), &c);
        assert_eq!(ed.first, 0);
        assert_eq!(ed.act(EditAction::Button(1), &c).line, "edit: grid");
        ed.act(EditAction::Scroll(-12), &c);
        assert_eq!(ed.low, 48);
        assert!(ed.act(EditAction::Button(0), &c).close);
        ed.act(EditAction::Row { row: 0, delta: -1 }, &c);
        assert_eq!(ed.tc.kind, TcKind::Legato);
        ed.act(EditAction::Row { row: 2, delta: 1 }, &c);
        assert_eq!(ed.tc.swing, 51);
    }

    #[test]
    fn tracker_steps_draw_the_sound_and_the_euclid_rows_fill_it() {
        let mut c = clip(&[(0, 240, 1), (960, 240, 3)]);
        let mut ed = Editor::new(0, 0, &c);
        assert_eq!(ed.sound, 1, "the lowest sound to begin with");
        ed.act(EditAction::Note { index: 1, rev: 0 }, &c);
        assert_eq!(ed.sound, 3, "the note tapped");
        ed.act(EditAction::Note { index: 0, rev: 0 }, &c);
        // Draw on an empty step puts the sound there, a step long.
        ed.act(EditAction::Button(2), &c);
        let done = run(&mut ed, &mut c, EditAction::Step(480));
        assert_eq!(done.line, "edit: Pad 2 drawn at tick 480");
        assert_eq!(ticks_of(&c, &ed.sel), vec![(480, 1)]);
        assert_eq!(c.notes[2].len, 240);
        assert!(ed.act(EditAction::Step(3840), &c).edit.is_empty());
        // Select on an empty step lets the notes go.
        ed.act(EditAction::Button(2), &c);
        ed.act(EditAction::Button(2), &c);
        assert_eq!(ed.tool, Tool::Select);
        ed.act(EditAction::Step(720), &c);
        assert!(ed.sel.is_empty());
        // Hits down from 4 to 3: 3 of 16 on pad 2 (x.....x....x....),
        // the note drawn at 480 goes, and the sound's notes are picked.
        let a = run(&mut ed, &mut c, EditAction::Row { row: 14, delta: -1 });
        let on = |c: &Clip, p: u8| -> Vec<Tick> {
            let mut v: Vec<Tick> = c
                .notes
                .iter()
                .filter(|n| n.note == p)
                .map(|n| n.tick)
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(on(&c, 1), vec![0, 1440, 2640]);
        assert_eq!(on(&c, 3), vec![960], "other sounds stay");
        assert_eq!(ticks_of(&c, &ed.sel), vec![(0, 1), (1440, 1), (2640, 1)]);
        // Rotate one step later: the same take.
        let b = run(&mut ed, &mut c, EditAction::Row { row: 16, delta: 1 });
        assert!(a.join.is_some() && a.join == b.join);
        assert_eq!(on(&c, 1), vec![240, 1680, 2880]);
        let rows = edit_rows(&ed, &c);
        assert_eq!(rows[EUCLID_ROW].value, "Pad 2");
        assert_eq!(
            (
                rows[14].value.as_str(),
                rows[15].value.as_str(),
                rows[16].value.as_str()
            ),
            ("3", "16 × 1/16", "1")
        );
        // The Euclid row picks another sound and writes nothing.
        let done = ed.act(
            EditAction::Row {
                row: EUCLID_ROW,
                delta: 1,
            },
            &c,
        );
        assert!(done.edit.is_empty());
        assert_eq!(done.line, "edit: euclid on Pad 3");
        assert!(ed.sel.is_empty(), "pad 3 plays nothing");
        ed.act(
            EditAction::Row {
                row: EUCLID_ROW,
                delta: 1,
            },
            &c,
        );
        assert_eq!(ticks_of(&c, &ed.sel), vec![(960, 3)], "pad 4's note");
        // Chance, condition and ratchet on the picked note.
        run(&mut ed, &mut c, EditAction::Row { row: 10, delta: -1 });
        run(&mut ed, &mut c, EditAction::Row { row: 11, delta: 1 });
        run(&mut ed, &mut c, EditAction::Row { row: 12, delta: 1 });
        let rows = edit_rows(&ed, &c);
        assert_eq!(
            (
                rows[10].value.as_str(),
                rows[11].value.as_str(),
                rows[12].value.as_str()
            ),
            ("95 %", "1:2", "2 hits")
        );
        ed.sel.clear();
        assert_eq!(edit_rows(&ed, &c)[11].value, "pick notes");
        // With the snap off the steps are 1/16s.
        ed.snap = DIVISIONS.len();
        assert_eq!(ed.grid().size(), STEP_FALLBACK);
    }

    #[test]
    fn the_tracker_opens_on_the_picked_note_and_scrolls_its_rows() {
        let notes: Vec<(Tick, Tick, u8)> = (0..16).map(|k| (k * 240, 120, 60)).collect();
        let mut c = clip(&notes);
        c.add_note(Note::new(0, 120, 64, 100)).unwrap();
        let mut ed = Editor::new(0, 0, &c);
        ed.act(EditAction::Note { index: 10, rev: 0 }, &c);
        ed.act(EditAction::Button(1), &c);
        ed.act(EditAction::Button(1), &c);
        assert_eq!(ed.lens, Lens::Tracker);
        // 16 steps and the chord's second note: note 10 is on row 11.
        let rows = track_rows(&c.notes, c.length, ed.grid());
        assert_eq!(rows.len(), 17);
        assert_eq!(ed.first, 9);
        ed.act(EditAction::Scroll(50), &c);
        assert_eq!(ed.first, 16);
        ed.act(EditAction::Scroll(-50), &c);
        assert_eq!(ed.first, 0);
    }

    #[test]
    fn the_tracker_shows_every_step_and_each_notes_settings() {
        let mut odd = Note::new(960, 240, 0, 90);
        odd.chance = 50;
        odd.cond = Cond::Every { a: 1, b: 2 };
        odd.ratchet = 3;
        let mut g = NoteGrid {
            notes: vec![
                NoteBox::of(&Note::new(0, 240, 2, 100), false),
                NoteBox::of(&Note::new(0, 240, 0, 120), false),
                NoteBox::of(&Note::new(258, 240, 1, 80), true),
                NoteBox::of(&odd, false),
            ],
            length: 3840,
            bar: 3840,
            pads: true,
            lens: Lens::Tracker,
            locks: vec![
                LockMark {
                    tick: 960,
                    sound: Some(0),
                },
                // A trigless lock on an empty step.
                LockMark {
                    tick: 480,
                    sound: None,
                },
            ],
            ..NoteGrid::default()
        };
        let rows = g.track_rows();
        assert_eq!(rows.len(), 17, "16 steps, one with two notes");
        // Step 0: pad 1 first, then pad 3 on a row of its own.
        assert_eq!(
            (rows[0], rows[1]),
            (
                TrackRow {
                    step: 0,
                    note: Some(1),
                    first: true
                },
                TrackRow {
                    step: 0,
                    note: Some(0),
                    first: false
                }
            )
        );
        let cells = |k: usize| g.track_cells(&rows[k]);
        assert_eq!(cells(0).0[..2], ["1.1.1".to_string(), "Pad 1".to_string()]);
        assert_eq!(cells(1).0[0], "");
        assert_eq!(cells(2).0[4], "+18");
        assert_eq!(cells(2).0[5], "·");
        let (empty, faint) = cells(3);
        assert!(faint);
        assert_eq!((empty[0].as_str(), empty[8].as_str()), ("1.1.3", "1 lock"));
        let busy = cells(5).0;
        assert_eq!(busy[0], "1.2.1");
        assert_eq!(busy[5..].to_vec(), vec!["50 %", "1:2", "×3", "1 lock"]);
        // A note's row picks it; an empty step's row asks for the sound
        // on its line.
        let probe = g.clone();
        let row = move |k: usize| mid(probe.list_rect(R, k + 1).unwrap());
        let (x, y) = row(2);
        assert_eq!(g.touch(1, Touch::Down, x, y, R), Some(EventKind::Select(2)));
        let (x, y) = row(4);
        assert_eq!(
            g.touch(2, Touch::Down, x, y, R),
            Some(EventKind::Cell { row: PEN, col: 720 })
        );
        // Scrolled: the first row shown is row `first`.
        g.first = 5;
        let (x, y) = row(0);
        assert_eq!(g.touch(3, Touch::Down, x, y, R), Some(EventKind::Select(3)));
        assert_eq!(g.button_label(1), "Tracker");
    }

    /// A note's pressure shows along its bottom, as high as the value,
    /// from each value's tick to the next; a note without any is drawn as
    /// before.
    #[test]
    fn the_roll_shows_each_notes_pressure() {
        let paint = painted;
        let plain = grid();
        let mut pressed = grid();
        // Note 1 of 2 (240 ticks): pressed fully for its first half, then
        // let go.
        pressed.notes[0].touch = mpfree_seq::Touch { first: 0, len: 2 };
        pressed.touch = vec![
            TouchPoint { at: 0, value: 127 },
            TouchPoint { at: 120, value: 0 },
        ];
        assert_eq!(pressed.touch_of(0).len(), 2);
        assert!(pressed.touch_of(1).is_empty() && plain.touch_of(0).is_empty());
        assert!(!plain.same(&pressed));
        let (a, b) = (paint(&plain), paint(&pressed));
        let nr = pressed.note_rect(R, 0).unwrap();
        let y = nr.y + nr.h / 2;
        let (left, right) = (nr.x + nr.w / 4, nr.x + 3 * nr.w / 4);
        assert_ne!(a.pixel(left, y), b.pixel(left, y), "pressed");
        assert_eq!(a.pixel(right, y), b.pixel(right, y), "let go");
        let (x, y) = mid(pressed.note_rect(R, 1).unwrap());
        assert_eq!(a.pixel(x, y), b.pixel(x, y), "the other note");
    }

    /// The grid painted alone in `R`, in the dark theme.
    fn painted(g: &NoteGrid) -> mpfree_ui::SoftCanvas {
        use mpfree_ui::{Fonts, SoftCanvas, Theme};
        let mut c = SoftCanvas::new(R.right(), R.bottom());
        let theme = Theme::dark();
        let mut fonts = Fonts::embedded();
        g.paint(
            &mut PaintCx {
                canvas: &mut c,
                theme: &theme,
                fonts: &mut fonts,
            },
            R,
        );
        c
    }

    /// Whether `a` and `b` differ anywhere in `r`.
    fn differ_in(a: &mpfree_ui::SoftCanvas, b: &mpfree_ui::SoftCanvas, r: Rect) -> bool {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| a.pixel(x, y) != b.pixel(x, y)))
    }

    /// The roll draws each note as it will play: an off note as an outline
    /// with its inside left as the roll is, a ratchet's hits apart and
    /// shaded by its ramp, a tie or a slide marked just before its start;
    /// the list and the tracker say the same in words.
    #[test]
    fn the_roll_draws_notes_as_they_will_play() {
        let mut n = Note::new(960, 480, 52, 64);
        n.on = false;
        n.ratchet = 4;
        n.ramp = 40;
        n.join = Join::Slide;
        let b = NoteBox::of(&n, false);
        assert_eq!(
            (b.on, b.ratchet, b.ramp, b.join),
            (false, 4, 40, Join::Slide)
        );
        // The second note left out, for comparing.
        let mut gone = grid();
        gone.notes.truncate(1);
        let mut plain = grid();
        plain.notes[1].selected = false;
        let nr = plain.note_rect(R, 1).unwrap();
        let blank = painted(&gone);
        assert!(
            differ_in(&blank, &painted(&plain), nr.inset(2)),
            "an on note is filled"
        );
        // Off: only its outline shows.
        let mut off = plain.clone();
        off.notes[1].on = false;
        let shown = painted(&off);
        assert!(!differ_in(&blank, &shown, nr.inset(2)), "nothing inside");
        assert!(differ_in(&blank, &shown, nr), "its outline");
        // A ratchet: four hits ramping down 40, a gap between each.
        let mut rat = plain.clone();
        rat.notes[1].ratchet = 4;
        rat.notes[1].ramp = -40;
        let hits: Vec<_> = rat.hit_rects(&rat.geo(R), nr, &rat.notes[1]).collect();
        let vels: Vec<u8> = hits.iter().map(|h| h.1).collect();
        assert_eq!(vels, vec![64, 51, 37, 24]);
        assert_eq!(hits[0].0.x, nr.x);
        assert!(
            hits.windows(2).all(|w| w[0].0.right() < w[1].0.x),
            "{hits:?}"
        );
        assert!(hits[3].0.right() <= nr.right());
        let (shown, y) = (painted(&rat), nr.y + nr.h / 2);
        let gap = hits[0].0.right();
        assert_eq!(shown.pixel(gap, y), blank.pixel(gap, y), "the gap");
        let (first, last) = (mid(hits[0].0), mid(hits[3].0));
        assert_ne!(shown.pixel(first.0, y), shown.pixel(last.0, y), "the ramp");
        assert_eq!(
            plain
                .hit_rects(&plain.geo(R), nr, &plain.notes[1])
                .collect::<Vec<_>>(),
            vec![(nr, 64)],
            "a plain note is one hit"
        );
        // A tie or a slide is marked just before the note.
        let before = Rect::new(nr.x - 8, nr.y, 8, nr.h);
        let unjoined = painted(&plain);
        for join in [Join::Tie, Join::Slide] {
            let mut joined = plain.clone();
            joined.notes[1].join = join;
            assert!(differ_in(&unjoined, &painted(&joined), before), "{join:?}");
        }
        // At the clip's start the mark stays off the note names.
        let mut first = plain.clone();
        first.notes[0].join = Join::Slide;
        let keys = plain.geo(R).keys;
        assert!(!differ_in(&unjoined, &painted(&first), keys));
        // In words: off and joined in the note's name, the ramp beside the
        // ratchet, an off note's row faint.
        let mut words = rat.clone();
        words.notes[1].on = false;
        words.notes[1].join = Join::Tie;
        let rows = words.track_rows();
        let row = rows.iter().find(|r| r.note == Some(1)).unwrap();
        let (cells, faint) = words.track_cells(row);
        let name = words.row_name(52);
        assert_eq!(cells[1], format!("{name} tie off"));
        assert_eq!(cells[7], "×4 -40");
        assert!(faint, "an off note's row");
        let row = rows.iter().find(|r| r.note == Some(0)).unwrap();
        assert!(!words.track_cells(row).1, "an on note's row");
    }

    /// Piano roll plan 0C.2: the note index finds the notes near a span
    /// of ticks, each once and in drawing order, a long note in every bar
    /// it sounds in and cut at the clip's end; the tracker's rows, the
    /// list's order and the index are worked out once for a grid's notes
    /// and again for other notes.
    /// An eight-bar clip on a roll of the page's width, opened as the
    /// page opens it.
    fn long_grid() -> (NoteGrid, Rect) {
        let r = Rect::new(12, 128, 1116, 670);
        let mut g = NoteGrid {
            notes: vec![
                NoteBox::of(&Note::new(0, 240, 60, 100), false),
                NoteBox::of(&Note::new(3840, 480, 64, 90), false),
                NoteBox::of(&Note::new(7 * 3840, 240, 67, 80), true),
            ],
            length: 8 * 3840,
            bar: 3840,
            ..NoteGrid::default()
        };
        g.view = g.open_view(r);
        (g, r)
    }

    /// Two fingers zoom (the way their first movement picks) and scroll,
    /// sending nothing until they lift, and then only the view; one
    /// finger scrolls on the ruler (time) and the keys (pitch).
    #[test]
    fn two_fingers_zoom_and_one_scrolls_on_the_ruler_and_keys() {
        let (mut g, r) = long_grid();
        let geo = g.geo(r);
        let v0 = geo.v;
        assert_eq!(v0.row_h, geo.roll.h / 8, "C3 to G3 fill the rows");
        let (cx, cy) = (geo.roll.x + 300, geo.roll.y + 200);
        // A pinch across, starting on empty roll: no note is drawn.
        assert_eq!(g.touch(1, Touch::Down, cx, cy, r), None);
        assert_eq!(g.touch(2, Touch::Down, cx + 100, cy, r), None);
        for k in 1..=10 {
            assert_eq!(g.touch(2, Touch::Move, cx + 100 + 10 * k, cy + 1, r), None);
        }
        assert!(g.busy());
        let v = g.view_in(r);
        assert!((v.ppt / v0.ppt - 2.0).abs() < 0.01, "{} {}", v.ppt, v0.ppt);
        assert_eq!(v.row_h, v0.row_h, "time only");
        assert!(matches!(
            g.touch(1, Touch::Up, cx, cy, r),
            Some(EventKind::View { .. })
        ));
        assert_eq!(g.touch(2, Touch::Up, cx + 200, cy, r), None, "no tap");
        assert!(!g.busy());
        // The ruler: a finger dragged left shows later ticks.
        let ruler = geo.ruler;
        let before = g.view_in(r).start;
        g.touch(3, Touch::Down, ruler.x + 400, ruler.y + 5, r);
        g.touch(3, Touch::Move, ruler.x + 300, ruler.y + 5, r);
        assert!(matches!(
            g.touch(3, Touch::Up, ruler.x + 300, ruler.y + 5, r),
            Some(EventKind::View { .. })
        ));
        let after = g.view_in(r);
        assert!((after.start - before - (100.0 / after.ppt) as Tick).abs() <= 1);
        // The keys: dragged down, higher rows show.
        let keys = g.geo(r).keys;
        let lift = g.view_in(r).lift;
        g.touch(4, Touch::Down, keys.x + 20, keys.y + 100, r);
        g.touch(4, Touch::Move, keys.x + 20, keys.y + 150, r);
        g.touch(4, Touch::Up, keys.x + 20, keys.y + 150, r);
        assert_eq!(g.view_in(r).lift, lift + 50);
        // A tap on the ruler moves nothing.
        let v = g.view_in(r);
        assert_eq!(g.touch(5, Touch::Down, ruler.x + 10, ruler.y + 5, r), None);
        assert_eq!(g.touch(5, Touch::Up, ruler.x + 10, ruler.y + 5, r), None);
        assert_eq!(g.view_in(r), v);
    }

    /// The overview: a tap jumps there, a drag moves the window, a double
    /// tap fits the whole clip. Fit fits the picked notes (or all), and
    /// Follow is a switch the playhead pages by.
    #[test]
    fn the_overview_fit_and_follow() {
        let (mut g, r) = long_grid();
        let geo = g.geo(r);
        let o = geo.overview;
        let shown = geo.v.ticks_shown(geo.roll);
        assert!(shown < g.length, "two bars of eight show");
        // A tap three quarters along: the window centres there.
        let x = o.x + o.w * 3 / 4;
        g.touch(1, Touch::Down, x, o.y + 5, r);
        assert!(matches!(
            g.touch(1, Touch::Up, x, o.y + 5, r),
            Some(EventKind::View { .. })
        ));
        let v = g.view_in(r);
        let centre = v.start + shown / 2;
        assert!(
            (centre - g.length * 3 / 4).abs() <= g.length / i64::from(o.w),
            "{v:?}"
        );
        // A second tap soon after: the whole clip.
        g.touch(2, Touch::Down, x, o.y + 5, r);
        g.touch(2, Touch::Up, x, o.y + 5, r);
        let whole = g.view_in(r);
        assert_eq!(
            (whole.start, whole.x_of(geo.roll, g.length)),
            (0, geo.roll.right())
        );
        // Fit: the picked note (in bar 8) across the roll.
        g.touch(3, Touch::Down, geo.fit.x + 5, geo.fit.y + 5, r);
        g.touch(3, Touch::Up, geo.fit.x + 5, geo.fit.y + 5, r);
        let fit = g.view_in(r);
        assert!(fit.start <= 7 * 3840 && fit.start + fit.ticks_shown(geo.roll) >= 7 * 3840 + 240);
        assert!(fit.notes_shown(geo.roll, 128).contains(&67));
        // Follow: lit, then the playhead pages the roll.
        assert!(!g.view.follow);
        g.touch(4, Touch::Down, geo.follow.x + 5, geo.follow.y + 5, r);
        g.touch(4, Touch::Up, geo.follow.x + 5, geo.follow.y + 5, r);
        assert!(g.view.follow);
        g.view = RollView {
            start: 0,
            ..g.view_in(r)
        };
        assert!(g.follow_to(r, 5 * 3840));
        let v = g.view_in(r);
        assert!(v.start <= 5 * 3840 && 5 * 3840 < v.start + v.ticks_shown(geo.roll));
        // A drag on the ruler turns it off.
        let ruler = geo.ruler;
        g.touch(5, Touch::Down, ruler.x + 400, ruler.y + 5, r);
        g.touch(5, Touch::Move, ruler.x + 380, ruler.y + 5, r);
        g.touch(5, Touch::Up, ruler.x + 380, ruler.y + 5, r);
        assert!(!g.view.follow);
        assert!(!g.follow_to(r, 0));
        // Every one of them is a control the audit can find.
        let ids: Vec<&str> = g.targets(r).iter().map(|t| t.id).collect();
        for id in ["fit", "follow", "overview", "ruler", "keys"] {
            assert!(ids.contains(&id), "{id}");
        }
    }

    /// Snap Auto: a drag moves the notes by whole snaps, so a note played
    /// a little late stays a little late; a fixed snap puts its start on
    /// a line, as before.
    #[test]
    fn a_snap_auto_drag_keeps_each_notes_feel() {
        let mut g = grid();
        g.notes[1] = NoteBox::of(&Note::new(970, 480, 52, 64), true);
        let n1 = g.note_rect(R, 1).unwrap();
        let (x, y) = mid(n1);
        let beat = (f64::from(g.geo(R).roll.w) / 4.0).round() as i32;
        let drop = |g: &mut NoteGrid| {
            g.touch(1, Touch::Down, x, y, R);
            g.touch(1, Touch::Move, x + beat + 3, y, R);
            match g.touch(1, Touch::Up, x + beat + 3, y, R) {
                Some(EventKind::Marker { frame, .. }) => drop_of(frame).0,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(drop(&mut g), 950, "fixed: onto the line at 1920");
        g.snap_auto = true;
        assert_eq!(drop(&mut g), 960, "Auto: a beat on, 10 ticks late still");
    }

    #[test]
    fn the_note_index_and_what_the_grid_keeps() {
        let bar = 960;
        let boxes = |notes: &[(Tick, Tick)]| -> Vec<NoteBox> {
            notes
                .iter()
                .map(|&(t, l)| NoteBox::of(&Note::new(t, l, 60, 100), false))
                .collect()
        };
        // Bar 1; bars 1 to 3; bar 3; bar 4, running past the end.
        let notes = boxes(&[
            (0, 240),
            (480, 2 * bar),
            (2 * bar + 10, 10),
            (3 * bar, 5 * bar),
        ]);
        let ix = NoteIndex::new(&notes, 4 * bar, bar);
        assert_eq!(ix.near(0, 4 * bar), vec![0, 1, 2, 3]);
        assert_eq!(ix.near(bar, bar + 1), vec![1], "the long note");
        assert_eq!(ix.near(2 * bar, 3 * bar), vec![1, 2]);
        assert_eq!(ix.near(3 * bar, 9 * bar), vec![3], "cut at the end");
        assert!(ix.near(bar, bar).is_empty());
        assert!(NoteIndex::new(&[], 4 * bar, bar).near(0, bar).is_empty());
        // Kept: the same answer while the notes are the same.
        let g = grid();
        assert!(Arc::ptr_eq(&g.track_rows(), &g.track_rows()));
        assert!(Arc::ptr_eq(&g.list_order(), &g.list_order()));
        assert!(Arc::ptr_eq(&g.index(), &g.index()));
        let rows = g.track_rows();
        // Another step: other rows.
        let mut finer = g.clone();
        finer.step /= 2;
        assert!(finer.track_rows().len() > rows.len());
        // Notes changed in place, then forgotten: worked out again.
        let mut moved = g.clone();
        moved.notes[0].tick = 2000;
        moved.forget();
        assert_eq!(moved.list_order().to_vec(), vec![1, 0]);
        assert_eq!(g.list_order().to_vec(), vec![0, 1]);
        // A grid shown again starts afresh.
        let mut shown = g.clone();
        shown.show(moved.clone());
        assert_eq!(shown.list_order().to_vec(), vec![1, 0]);
    }

    /// Touches are followed finger by finger: a drag is its own finger's,
    /// and a second finger coming down stops it with nothing sent.
    #[test]
    fn a_drag_is_its_fingers_alone_and_a_second_finger_stops_it() {
        let mut g = grid();
        let n1 = g.note_rect(R, 1).unwrap();
        let (x, y) = mid(n1);
        let beat = (f64::from(g.geo(R).roll.w) / 4.0).round() as i32;
        assert_eq!(g.touch(1, Touch::Down, x, y, R), Some(EventKind::Select(1)));
        // Another finger's moves and lift leave the drag be.
        assert_eq!(g.touch(2, Touch::Move, x + beat + 3, y, R), None);
        assert_eq!(g.note_rect(R, 1), Some(n1));
        assert_eq!(g.touch(1, Touch::Move, x + beat + 3, y, R), None);
        let moved = g.note_rect(R, 1).unwrap();
        assert_ne!(moved, n1);
        assert_eq!(g.touch(2, Touch::Up, x, y, R), None);
        assert_eq!(g.note_rect(R, 1), Some(moved), "still dragging");
        // A second finger down: the drag stops where it was with nothing
        // sent, and the two fingers move the view (a pinch), never a note.
        let n0 = g.note_rect(R, 0).unwrap();
        let empty = (n0.x + n0.w * 3, n0.y + n0.h / 2);
        assert_eq!(g.touch(2, Touch::Down, empty.0, empty.1, R), None);
        assert_eq!(g.note_rect(R, 1), Some(n1), "back in place");
        let view = g.view;
        assert_eq!(g.touch(1, Touch::Move, x + 2 * beat, y, R), None);
        assert_ne!(g.view, view, "zoomed in time");
        assert!(
            matches!(
                g.touch(1, Touch::Up, x + 2 * beat, y, R),
                Some(EventKind::View { .. })
            ),
            "no drop, only the view"
        );
        // The finger left does nothing until it lifts.
        let zoomed = g.view;
        assert_eq!(g.touch(2, Touch::Move, empty.0 + 90, empty.1, R), None);
        assert_eq!(
            (g.view, g.touch(2, Touch::Up, empty.0, empty.1, R)),
            (zoomed, None)
        );
        g.view = view;
        // With every finger up, a tap works again (when it lifts).
        assert_eq!(g.touch(3, Touch::Down, empty.0, empty.1, R), None);
        assert!(matches!(
            g.touch(3, Touch::Up, empty.0, empty.1, R),
            Some(EventKind::Cell { row: 50, .. })
        ));
        // The velocity lane: a second finger stops it, and nothing is sent.
        let v = g.geo(R).vel;
        assert_eq!(g.touch(4, Touch::Down, v.x + 10, v.y + 3, R), None);
        assert!(g.drag.is_some());
        assert_eq!(g.touch(5, Touch::Down, v.x + 40, v.y + 3, R), None);
        assert_eq!(g.drag, None);
        assert_eq!(g.touch(4, Touch::Move, v.x + 10, v.bottom(), R), None);
        assert!(!matches!(
            g.touch(4, Touch::Up, v.x + 10, v.bottom(), R),
            Some(EventKind::Value(_))
        ));
        assert_eq!(g.touch(5, Touch::Up, v.x + 40, v.y + 3, R), None);
        g.view = view;
        // A finger whose lift was lost starts afresh when it comes down.
        assert_eq!(g.touch(6, Touch::Down, x, y, R), Some(EventKind::Select(1)));
        assert_eq!(g.touch(6, Touch::Down, empty.0, empty.1, R), None);
        assert!(g.touch(6, Touch::Up, empty.0, empty.1, R).is_some());
        assert_eq!(g.note_rect(R, 1), Some(n1));
    }

    #[test]
    fn names_and_the_packed_drag() {
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(61), "C#3");
        assert_eq!(note_name(0), "C-2");
        assert_eq!(position_name(0, 3840), "1.1.000");
        assert_eq!(position_name(3840 + 960 + 120, 3840), "2.2.120");
        assert_eq!(length_name(240), "1/16");
        assert_eq!(length_name(250), "250 ticks");
        for (t, n) in [(0, 0), (-240, 3), (3840, -12), (-1, -127), (99_999, 127)] {
            assert_eq!(drop_of(drop_frame(t, n)), (t, n));
        }
    }

    fn grid() -> NoteGrid {
        NoteGrid {
            notes: vec![
                NoteBox::of(&Note::new(0, 240, 50, 100), false),
                NoteBox::of(&Note::new(960, 480, 52, 64), true),
            ],
            length: 3840,
            bar: 3840,
            // C2 at the bottom, 40 px rows (the clip fits across).
            view: RollView {
                lift: 48 * 40,
                row_h: 40,
                ..RollView::default()
            },
            ..NoteGrid::default()
        }
    }

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 830,
        h: 640,
    };

    fn mid(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn the_roll_reports_notes_empty_space_and_buttons() {
        let mut g = grid();
        let n1 = g.note_rect(R, 1).unwrap();
        let (x, y) = mid(n1);
        assert_eq!(g.touch(1, Touch::Down, x, y, R), Some(EventKind::Select(1)));
        assert_eq!(g.touch(1, Touch::Up, x, y, R), None, "a tap is not a drag");
        // Empty space: the row's note and the tick snapped down, when the
        // finger lifts.
        let n0 = g.note_rect(R, 0).unwrap();
        let (x, y) = (n0.x + n0.w * 3, n0.y + n0.h / 2);
        assert_eq!(g.touch(2, Touch::Down, x, y, R), None);
        match g.touch(2, Touch::Up, x, y, R) {
            Some(EventKind::Cell { row, col }) => {
                assert_eq!(row, 50);
                assert_eq!(col % 240, 0);
                assert!(col > 240);
            }
            other => panic!("{other:?}"),
        }
        let b = g.button_rects(R);
        assert_eq!(
            g.touch(3, Touch::Down, mid(b[7]).0, mid(b[7]).1, R),
            Some(EventKind::Tab(7))
        );
        assert_eq!(g.button_label(7), "Edit");
        assert_eq!(g.button_label(3), "Snap 1/16");
        g.snap_name = "Auto · 1/8".into();
        assert_eq!(g.button_label(3), "Auto · 1/8");
    }

    #[test]
    fn a_drag_snaps_and_is_packed_on_lifting() {
        let mut g = grid();
        let n1 = g.note_rect(R, 1).unwrap();
        let (x, y) = mid(n1);
        g.touch(1, Touch::Down, x, y, R);
        // Right by about a beat and up two rows.
        let beat = (f64::from(g.geo(R).roll.w) / 4.0).round() as i32;
        let rh = g.geo(R).v.row_h;
        assert_eq!(
            g.touch(1, Touch::Move, x + beat + 3, y - 2 * rh - 2, R),
            None
        );
        // The selected note is drawn where it would land.
        assert_eq!(g.note_rect(R, 1).unwrap().y, n1.y - 2 * rh);
        match g.touch(1, Touch::Up, x + beat + 3, y - 2 * rh - 2, R) {
            Some(EventKind::Marker { index: 1, frame }) => assert_eq!(drop_of(frame), (960, 2)),
            other => panic!("{other:?}"),
        }
        assert_eq!(g.note_rect(R, 1), Some(n1), "back in place once dropped");
    }

    #[test]
    fn the_velocity_lane_and_the_list() {
        let mut g = grid();
        let v = g.geo(R).vel;
        // A drag of the lane is shown on the picked notes as the finger
        // moves, and sent once, as the finger lifts: one edit.
        let painted_before = painted(&g);
        assert_eq!(g.touch(1, Touch::Down, v.x + 10, v.y + 3, R), None);
        assert_eq!(g.drag.and_then(|d| d.lane).map(lane_velocity), Some(127));
        assert_eq!(g.touch(1, Touch::Move, v.x + 10, v.bottom(), R), None);
        assert_eq!(g.drag.and_then(|d| d.lane), Some(0.0));
        assert!(
            differ_in(&painted_before, &painted(&g), v),
            "the picked note's bar follows the finger"
        );
        assert_eq!(
            g.touch(1, Touch::Up, v.x + 10, v.bottom(), R),
            Some(EventKind::Value(0.0))
        );
        assert_eq!(g.drag, None);
        g.lens = Lens::List;
        // Row 1 is the first note in time order.
        let row = g.list_rect(R, 2).unwrap();
        assert_eq!(
            g.touch(2, Touch::Down, mid(row).0, mid(row).1, R),
            Some(EventKind::Select(1))
        );
        assert_eq!(g.button_label(1), "List", "the lens showing");
    }

    #[test]
    fn it_paints_every_lens() {
        use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};
        let mut g = grid();
        g.playhead = Some(1000);
        g.locks = vec![LockMark {
            tick: 960,
            sound: Some(52),
        }];
        for lens in [Lens::Grid, Lens::List, Lens::Tracker] {
            g.lens = lens;
            let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), R.w, R.h);
            ui.add("notes", g.clone());
            ui.add_page("Edit", "col(notes:*)").unwrap();
            let mut c = SoftCanvas::new(R.w, R.h);
            assert!(!ui.paint(&mut c).is_empty());
        }
    }
}
