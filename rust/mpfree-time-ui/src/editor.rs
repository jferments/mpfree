//! The note editor of the Arrange page (the MPC's Grid View and List
//! Edit): one clip's notes through one of three lenses ([`Lens`]): a
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
//! - the buttons along the bottom: `Tab(k)` for [`EDIT_BUTTONS`]`[k]`.
//!
//! On the roll, a note played with pad pressure shows it as a lighter
//! contour along its bottom, as high as the pressure, from where each
//! value was played to the next (`NoteGrid::touch`).
//!
//! [`edit_rows`] fills the Edit options list; its `Step` and `Select`
//! become [`EditAction::Row`] and [`EditAction::PickRow`].

use mpfree_seq::notes::{self, Snap, Tc, TcKind, Which, DIVISIONS};
use mpfree_seq::steps::{Edit, Euclid, Grid, MAX_EUCLID_STEPS};
use mpfree_seq::{Clip, Cond, Lock, Note, TouchPoint};
use mpfree_transport::time::{Tick, PPQ};
use mpfree_ui::widgets::OptionRow;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The buttons along the bottom, left to right. Some show a state in
/// place of their name (see [`NoteGrid::button_label`]).
pub const EDIT_BUTTONS: [&str; 8] = [
    "‹ Clips",
    "Grid",
    "Tool",
    "Snap",
    "All",
    "Duplicate",
    "Delete",
    "TC",
];
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
/// The snaps the Snap button steps through: the divisions, then off.
pub const SNAPS: usize = DIVISIONS.len() + 1;
/// The snap a new editor starts with: 1/16.
pub const DEFAULT_SNAP: usize = 6;
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
    /// A note tapped, by its index in the clip.
    Note(usize),
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
    /// The wheel: the roll's notes, or the list's rows.
    Scroll(i32),
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
    /// Index into the divisions, or [`DIVISIONS`]`.len()` for no snap.
    pub snap: usize,
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
            tc: Tc::default(),
            row: 0,
            low,
            first: 0,
            sound,
            euclid: Euclid::default(),
        }
    }

    /// The snap in ticks, or 0 for none.
    pub fn snap_ticks(&self) -> Tick {
        DIVISIONS.get(self.snap).map_or(0, |d| d.0)
    }

    pub fn snap_name(&self) -> &'static str {
        DIVISIONS.get(self.snap).map_or("Off", |d| d.1)
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
            EditAction::Button(7) => {
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
            EditAction::Note(i) if i < clip.notes.len() => match self.tool {
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
            EditAction::Note(_) => return Done::default(),
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
            EditAction::Velocity(v) => {
                let v = (1.0 + v.clamp(0.0, 1.0) * 126.0).round() as u8;
                (
                    notes::set_velocity(clip, &sel, v),
                    key(1),
                    true,
                    true,
                    String::new(),
                )
            }
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
        let mut used = vec![false; clip.notes.len()];
        let mut sel = Vec::with_capacity(want.len());
        for n in &want {
            if let Some(i) = (0..clip.notes.len()).find(|&i| !used[i] && clip.notes[i] == *n) {
                used[i] = true;
                sel.push(i);
            }
        }
        sel.sort_unstable();
        self.sel = sel;
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
                    .unwrap_or(DEFAULT_SNAP) as i32;
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
            format!("by {}", ed.snap_name())
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

/// The roll or the list (see the module notes).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteGrid {
    pub title: String,
    pub legend: String,
    pub notes: Vec<NoteBox>,
    /// The clip's length and a bar, in clip ticks.
    pub length: Tick,
    pub bar: Tick,
    /// The roll's lowest note.
    pub low: u8,
    /// Pad rows (Pad 1 to 16) in place of note names.
    pub pads: bool,
    pub playhead: Option<Tick>,
    /// The snap in ticks (0: none), for the drag and the lines.
    pub snap: Tick,
    pub snap_name: String,
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
}

/// A note being dragged: which, where the finger went down, and the move
/// so far (snapped).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    index: usize,
    x: i32,
    y: i32,
    ticks: Tick,
    notes: i32,
    /// The finger is in the velocity lane.
    velocity: bool,
}

const KEYS_W: i32 = 64;
const RULER_H: i32 = 22;
const VEL_H: i32 = 64;
const TOOLS_H: i32 = 48;
const GAP: i32 = 6;
const ROW_H: i32 = 18;
/// About how many rows the roll shows on the Arrange page.
pub const ROLL_ROWS: u8 = 22;
const LIST_H: i32 = 34;
/// A tracker row: tighter than the list's, so a bar of 1/16s shows.
const TRACK_H: i32 = 26;
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
            low: 48,
            pads: false,
            playhead: None,
            snap: PPQ as Tick / 4,
            snap_name: "1/16".into(),
            tool: Tool::Select,
            lens: Lens::Grid,
            first: 0,
            color: Color::rgb(0x6EA8FE),
            step: STEP_FALLBACK,
            locks: Vec::new(),
            touch: Vec::new(),
            drag: None,
        }
    }
}

struct Geo {
    ruler: Rect,
    keys: Rect,
    roll: Rect,
    vel: Rect,
    tools: Rect,
    rows: i32,
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
            && (self.length, self.bar, self.low, self.pads)
                == (other.length, other.bar, other.low, other.pads)
            && (self.snap, self.tool, self.lens, self.first)
                == (other.snap, other.tool, other.lens, other.first)
            && self.snap_name == other.snap_name
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

    /// Show what `other` shows, keeping a drag in progress.
    pub fn show(&mut self, other: NoteGrid) {
        let drag = self.drag;
        *self = NoteGrid { drag, ..other };
    }

    fn geo(&self, r: Rect) -> Geo {
        // Matches PaintCx::titled_panel with the default theme's padding.
        let body = r.inset(14).take_top(34).1;
        let (tools, rest) = body.take_bottom(TOOLS_H);
        let rest = Rect::new(rest.x, rest.y, rest.w, rest.h - GAP);
        let (vel, rest) = if self.lens != Lens::Grid {
            (Rect::new(rest.x, rest.bottom(), rest.w, 0), rest)
        } else {
            let (v, rest) = rest.take_bottom(VEL_H);
            (v, Rect::new(rest.x, rest.y, rest.w, rest.h - GAP))
        };
        let (ruler, rest) = rest.take_top(RULER_H);
        let (keys, roll) = rest.take_left(KEYS_W);
        let rows = if self.pads {
            16
        } else {
            (roll.h / ROW_H).max(1)
        };
        Geo {
            ruler: Rect::new(roll.x, ruler.y, roll.w, ruler.h),
            keys,
            roll,
            vel: Rect::new(roll.x, vel.y, roll.w, vel.h),
            tools,
            rows,
        }
    }

    fn row_h(&self, g: &Geo) -> i32 {
        (g.roll.h / g.rows).max(1)
    }

    /// The x of clip tick `t` on the roll.
    fn x_of(&self, g: &Geo, t: Tick) -> i32 {
        let len = self.length.max(1) as f64;
        g.roll.x + (t as f64 / len * f64::from(g.roll.w)).round() as i32
    }

    /// The clip tick at `x`, not snapped.
    fn tick_at(&self, g: &Geo, x: i32) -> Tick {
        let f = f64::from(x - g.roll.x) / f64::from(g.roll.w.max(1));
        (f * self.length as f64)
            .floor()
            .clamp(0.0, (self.length - 1).max(0) as f64) as Tick
    }

    /// The note of the row at `y`, if it is on the roll.
    fn note_at(&self, g: &Geo, y: i32) -> Option<u8> {
        let row = (g.roll.bottom() - 1 - y) / self.row_h(g);
        let n = i32::from(self.low) + row;
        (y >= g.roll.y && y < g.roll.bottom() && row < g.rows && (0..=127).contains(&n))
            .then_some(n as u8)
    }

    /// The y of the top of `note`'s row, if it shows.
    fn y_of(&self, g: &Geo, note: u8) -> Option<i32> {
        let row = i32::from(note) - i32::from(self.low);
        (0..g.rows)
            .contains(&row)
            .then(|| g.roll.bottom() - (row + 1) * self.row_h(g))
    }

    /// Where note `i` is drawn on the roll, if it shows (moved by a drag
    /// in progress when it is selected).
    pub fn note_rect(&self, r: Rect, i: usize) -> Option<Rect> {
        let g = self.geo(r);
        let n = self.notes.get(i)?;
        let (dt, dn) = match self.drag {
            Some(d) if n.selected && !d.velocity => (d.ticks, d.notes),
            _ => (0, 0),
        };
        let note = u8::try_from(i32::from(n.note) + dn).ok()?;
        let y = self.y_of(&g, note)?;
        let tick = (n.tick + dt).rem_euclid(self.length.max(1));
        let x0 = self.x_of(&g, tick);
        let x1 = self.x_of(&g, (tick + n.len).min(self.length)).max(x0 + 3);
        Some(Rect::new(x0, y + 1, x1 - x0, self.row_h(&g) - 2))
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

    /// The tracker's rows.
    pub fn track_rows(&self) -> Vec<TrackRow> {
        track_rows(&self.notes, self.length, self.grid())
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
            3 => format!("Snap {}", self.snap_name),
            4 => if any { "None" } else { "All" }.into(),
            7 => if any { "TC sel." } else { "TC all" }.into(),
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

    fn paint_roll(&self, cx: &mut PaintCx, r: Rect, g: &Geo) {
        let t = cx.theme.clone();
        let rh = self.row_h(g);
        // Rows: the black keys darker, C lines brighter.
        for row in 0..g.rows {
            let note = i32::from(self.low) + row;
            if note > 127 {
                break;
            }
            let y = g.roll.bottom() - (row + 1) * rh;
            let black = !self.pads && matches!(note % 12, 1 | 3 | 6 | 8 | 10);
            let fill = if black { t.bg } else { t.surface };
            cx.canvas
                .fill_rect(Rect::new(g.roll.x, y, g.roll.w, rh), fill);
            if self.pads || note % 12 == 0 || rh >= 14 {
                cx.text(
                    FontId::Sans,
                    (t.size_small - 2.0).min(rh as f32),
                    Rect::new(g.keys.x, y, g.keys.w - 6, rh),
                    Align::Right,
                    if note % 12 == 0 { t.text } else { t.text_faint },
                    &self.row_name(note as u8),
                );
            }
            if !self.pads && note % 12 == 0 {
                cx.canvas
                    .fill_rect(Rect::new(g.roll.x, y + rh - 1, g.roll.w, 1), t.outline);
            }
        }
        // Lines: bars, beats, and the snap when they are far enough apart.
        let beat = PPQ as Tick;
        let px = |ticks: Tick| f64::from(g.roll.w) * ticks as f64 / self.length.max(1) as f64;
        let mut lines: Vec<(Tick, Color, i32)> = Vec::new();
        if self.snap > 0 && px(self.snap) >= 6.0 {
            lines.push((self.snap, t.outline.mix(t.surface, 0.6), 1));
        }
        if px(beat) >= 6.0 {
            lines.push((beat, t.outline, 1));
        }
        lines.push((self.bar.max(1), t.text_faint, 2));
        for (step, color, w) in lines {
            let mut at = 0;
            while at < self.length {
                let x = self.x_of(g, at);
                cx.canvas
                    .fill_rect(Rect::new(x, g.roll.y, w, g.roll.h), color);
                at += step;
            }
        }
        // The ruler: bar numbers.
        let mut bar = 0;
        while bar * self.bar.max(1) < self.length {
            let x = self.x_of(g, bar * self.bar.max(1));
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(x + 3, g.ruler.y, 60, g.ruler.h),
                Align::Left,
                t.text_faint,
                &format!("{}", bar + 1),
            );
            bar += 1;
        }
        // The notes, then the velocity lane under them.
        let top = t.accent;
        for (i, n) in self.notes.iter().enumerate() {
            let Some(nr) = self.note_rect(r, i) else {
                continue;
            };
            let shade = 0.35 + 0.65 * f32::from(n.velocity) / 127.0;
            let fill = t.raised.mix(self.color, shade);
            cx.canvas.fill_round_rect(nr, 3, fill);
            self.paint_touch(cx, g, nr, i, fill.mix(t.text, 0.5));
            if n.selected {
                cx.canvas.stroke_round_rect(nr, 3, 2, top);
            }
        }
        cx.canvas.fill_round_rect(g.vel, 6, t.surface);
        for n in &self.notes {
            let x = self.x_of(g, n.tick);
            let h = (f32::from(n.velocity) / 127.0 * (g.vel.h - 6) as f32).round() as i32;
            let c = if n.selected { top } else { self.color };
            cx.canvas
                .fill_rect(Rect::new(x, g.vel.bottom() - 3 - h, 4, h.max(1)), c);
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
            cx.canvas.fill_rect(
                Rect::new(x, g.ruler.y, 2, g.roll.bottom() - g.ruler.y),
                t.good,
            );
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
            let x = nr.x as f64 + f64::from(g.roll.w) * ticks as f64 / self.length.max(1) as f64;
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
                self.row_name(n.note),
                n.velocity.to_string(),
                length_name(n.len),
                format!("{} %", n.chance),
            ];
            let mut x = row.x + 8;
            for ((_, w), text) in LIST_COLS.iter().zip(cells) {
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    Rect::new(x, row.y, *w, row.h),
                    Align::Left,
                    t.text,
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
    /// empty step).
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
            self.row_name(n.note),
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
            if n.ratchet == 1 {
                dot()
            } else {
                format!("×{}", n.ratchet)
            },
            if locks == 0 { dot() } else { locks_name(locks) },
        ];
        (cells, false)
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
                    .fill_round_rect(rr, 6, t.raised.mix(t.accent, 0.3));
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
    /// shows them.
    pub fn list_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.notes.len()).collect();
        order.sort_by_key(|&i| (self.notes[i].tick, self.notes[i].note));
        order
    }
}

impl Widget for NoteGrid {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.legend);
        let g = self.geo(r);
        match self.lens {
            Lens::Grid => self.paint_roll(cx, r, &g),
            Lens::List => self.paint_list(cx, r),
            Lens::Tracker => self.paint_tracker(cx, r),
        }
        for (k, br) in self.button_rects(r).iter().enumerate() {
            let lit = (k == 1 && self.lens != Lens::Grid) || (k == 2 && self.tool != Tool::Select);
            let fill = if lit { t.accent } else { t.raised };
            let ink = if lit { t.on_accent } else { t.text };
            cx.canvas.fill_round_rect(*br, 10, fill);
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
        for i in 0..self.notes.len() {
            if let Some(nr) = self.note_rect(r, i) {
                out.push(Target::tap("note", i, format!("note {}", i + 1), nr));
            }
        }
        // An empty place on the roll adds a note: its top left corner,
        // where the sample clips leave room.
        let spot = Rect::new(g.roll.x + 2, g.roll.y + 2, 12, 12);
        let free = (0..self.notes.len()).all(|i| {
            self.note_rect(r, i)
                .is_none_or(|nr| nr.intersect(spot).is_empty())
        });
        if free && !g.roll.is_empty() {
            out.push(Target::tap("roll", 0, "empty roll", spot));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = self.geo(r);
        match phase {
            Touch::Down => {
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
                if g.vel.contains(x, y) {
                    self.drag = Some(Drag {
                        index: 0,
                        x,
                        y,
                        ticks: 0,
                        notes: 0,
                        velocity: true,
                    });
                    let v = f32::from((g.vel.bottom() - 3 - y).clamp(0, g.vel.h - 6) as u16)
                        / (g.vel.h - 6).max(1) as f32;
                    return Some(EventKind::Value(v));
                }
                // The topmost note under the finger (drawn last).
                if let Some(i) = (0..self.notes.len())
                    .rev()
                    .find(|&i| self.note_rect(r, i).is_some_and(|nr| nr.contains(x, y)))
                {
                    self.drag = Some(Drag {
                        index: i,
                        x,
                        y,
                        ticks: 0,
                        notes: 0,
                        velocity: false,
                    });
                    return Some(EventKind::Select(i));
                }
                let note = self.note_at(&g, y)?;
                if !g.roll.contains(x, y) {
                    return None;
                }
                let t = self.tick_at(&g, x);
                let t = match self.snap {
                    0 => t,
                    s => t - t.rem_euclid(s),
                };
                Some(EventKind::Cell {
                    row: usize::from(note),
                    col: t as usize,
                })
            }
            Touch::Move => {
                let d = self.drag?;
                if d.velocity {
                    let v = f32::from((g.vel.bottom() - 3 - y).clamp(0, g.vel.h - 6) as u16)
                        / (g.vel.h - 6).max(1) as f32;
                    return Some(EventKind::Value(v));
                }
                let n = self.notes.get(d.index)?;
                let raw = self.tick_at(&g, x) - self.tick_at(&g, d.x);
                let ticks = self.snapped(n.tick + raw) - n.tick;
                let notes = (d.y - y) / self.row_h(&g).max(1);
                self.drag = Some(Drag { ticks, notes, ..d });
                None
            }
            Touch::Up => {
                let d = self.drag.take()?;
                (!d.velocity && (d.ticks, d.notes) != (0, 0)).then(|| EventKind::Marker {
                    index: d.index,
                    frame: drop_frame(d.ticks, d.notes),
                })
            }
            Touch::Cancel => {
                self.drag = None;
                None
            }
        }
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

    #[test]
    fn a_drag_moves_the_selection_and_it_stays_selected() {
        let mut c = clip(&[(0, 240, 48), (480, 240, 50), (960, 240, 52)]);
        let mut ed = Editor::new(1, 0, &c);
        assert_eq!(ed.low, 48);
        // From G2 to D#4 does not fit from C2: the roll starts at G2.
        let wide = clip(&[(0, 240, 55), (480, 240, 75)]);
        assert_eq!(Editor::new(1, 0, &wide).low, 55);
        ed.act(EditAction::Note(1), &c);
        assert_eq!(ed.sel, vec![1]);
        ed.act(EditAction::Button(4), &c);
        assert!(ed.sel.is_empty(), "All with some selected selects none");
        ed.act(EditAction::Button(4), &c);
        assert_eq!(ed.sel, vec![0, 1, 2]);
        // A tap on one of the group keeps the group.
        ed.act(EditAction::Note(2), &c);
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
        ed.act(EditAction::Note(0), &c);
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
        run(&mut ed, &mut c, EditAction::Note(0));
        assert_eq!(c.notes.len(), 1);
    }

    #[test]
    fn timing_correct_takes_the_selection_or_everything() {
        let mut c = clip(&[(10, 100, 36), (250, 100, 38), (500, 100, 40)]);
        let mut ed = Editor::new(0, 0, &c);
        ed.act(EditAction::Note(1), &c);
        let done = run(&mut ed, &mut c, EditAction::Button(7));
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
        run(&mut ed, &mut c, EditAction::Button(7));
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
        assert_eq!(ed.snap_name(), "1/16");
        for _ in 0..DIVISIONS.len() - DEFAULT_SNAP {
            ed.act(EditAction::Button(3), &c);
        }
        assert_eq!((ed.snap_name(), ed.snap_ticks()), ("Off", 0));
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
        ed.act(EditAction::Note(1), &c);
        assert_eq!(ed.sound, 3, "the note tapped");
        ed.act(EditAction::Note(0), &c);
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
        ed.act(EditAction::Note(10), &c);
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
        use mpfree_ui::{Fonts, SoftCanvas, Theme};
        let paint = |g: &NoteGrid| {
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
        };
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
            low: 48,
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
        // Empty space: the row's note and the tick snapped down.
        let n0 = g.note_rect(R, 0).unwrap();
        let (x, y) = (n0.x + n0.w * 3, n0.y + n0.h / 2);
        match g.touch(2, Touch::Down, x, y, R) {
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
        assert_eq!(g.button_label(7), "TC sel.");
        assert_eq!(g.button_label(3), "Snap 1/16");
    }

    #[test]
    fn a_drag_snaps_and_is_packed_on_lifting() {
        let mut g = grid();
        let n1 = g.note_rect(R, 1).unwrap();
        let (x, y) = mid(n1);
        g.touch(1, Touch::Down, x, y, R);
        // Right by about a beat and up two rows.
        let beat = (f64::from(g.geo(R).roll.w) / 4.0).round() as i32;
        let rh = g.row_h(&g.geo(R));
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
        match g.touch(1, Touch::Down, v.x + 10, v.y + 3, R) {
            Some(EventKind::Value(x)) => assert!(x > 0.95, "{x}"),
            other => panic!("{other:?}"),
        }
        match g.touch(1, Touch::Move, v.x + 10, v.bottom(), R) {
            Some(EventKind::Value(x)) => assert_eq!(x, 0.0),
            other => panic!("{other:?}"),
        }
        assert_eq!(g.touch(1, Touch::Up, v.x + 10, v.bottom(), R), None);
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
