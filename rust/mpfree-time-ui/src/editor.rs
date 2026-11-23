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
    "Draw",
    "Snap",
    "Select",
    "Duplicate",
    "Delete",
    "Edit",
];
/// [`EDIT_BUTTONS`]' Select: opens and closes the Select menu (the
/// roll's own; plan 1B.3).
pub const SELECT_BUTTON: usize = 4;
/// The Select menu (plan 1B.3), its first column then its second: what
/// to pick, Add to pick (a switch: taps and boxes add to the picks), Fit
/// to picked (the view, the roll's own), then the filters, each picking
/// from the whole clip.
pub const SELECT_ITEMS: [&str; 13] = [
    "All",
    "None",
    "Invert",
    "Add to pick",
    "Fit to picked",
    "Same pitch",
    "Pitch range",
    "Time range",
    "Quiet notes",
    "Loud notes",
    "With a condition",
    "With repeats",
    "Overlapping",
];
/// [`SELECT_ITEMS`]' Add to pick and Fit to picked; the filters start
/// after them, in the menu's second column.
pub const SELECT_ADD: usize = 3;
pub const SELECT_FIT: usize = 4;
pub const SELECT_FILTERS: usize = 5;
/// A Select menu item tapped: `Tab`'s index is this plus the item's.
pub const MENU: usize = 100;
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
/// A finger holding a note while another taps note `col`
/// ([`EditAction::Add`]): `Cell`'s row says so (plan 1B.2).
pub const ADD: usize = usize::MAX - 1;
/// A box drawn on empty roll with Draw off ([`EditAction::Pick`]):
/// `Span`'s index is this plus its lowest row times 256 plus its highest
/// ([`box_of`]); a drawn note's `Span` names its row, under 128.
pub const BOX: usize = 1 << 16;
/// A handle dragged ([`EditAction::Resize`]): `Stretch`'s index is the
/// note's, plus this for its start handle; `to` is the ticks it moved.
pub const START_HANDLE: usize = 1 << 24;
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
/// The velocity a drawn note gets until another is used.
const DRAW_VELOCITY: u8 = 100;
/// A step, for a tap with Draw on (plan 1B.1, call 3): a note no longer
/// than this comes out, as a tap on a lit step does; a longer one is
/// picked. A 1/16, the Steps view's step.
pub const TAP_STEP: Tick = PPQ as Tick / 4;
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

/// What a tap on the roll does: the Draw button, lit while Draw is on
/// (piano roll plan 1B.1, the plan's call 2: one Draw button in place of
/// the Select, Draw and Erase cycle; ERASE held + a tap erases).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Draw off, editing: a tap picks a note, a drag moves the picked
    /// notes, a double tap on empty space puts a note in.
    #[default]
    Select,
    /// Draw on, the step feel: a tap on empty space puts a note in, a tap
    /// on a note no longer than a step takes it out, a drag on empty space
    /// draws a note as long as the drag.
    Draw,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Draw => "Draw",
        }
    }

    fn next(self) -> Tool {
        match self {
            Tool::Select => Tool::Draw,
            Tool::Draw => Tool::Select,
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
    /// A note picked (a hold on the roll, a row of the list or the
    /// tracker), by its index in the clip, on the picture of revision
    /// `rev` ([`crate::EditInfo::rev`]). The app refuses every action
    /// naming a note if the clip has moved on since, so a touch never
    /// lands on another note (piano roll plan 0A.1). A note already
    /// picked keeps the others with it.
    Note { index: usize, rev: u64 },
    /// A note on the roll tapped (plan 1B.1): with Draw on, a note no
    /// longer than a step comes out ([`TAP_STEP`]) and a longer one is
    /// picked; with Draw off it is picked in place of the others.
    Tap { index: usize, rev: u64 },
    /// A note taken out: ERASE held + a tap.
    Erase { index: usize, rev: u64 },
    /// A note added to the picks, or taken out of them when it was
    /// picked (plan 1B.2: a finger holding a note while another taps one,
    /// as holding steps does on the step pads).
    Add { index: usize, rev: u64 },
    /// The notes in a box picked in place of the others (plan 1B.2: a
    /// drag on empty roll with Draw off): every note sounding somewhere
    /// from tick `from` to `to` on a row from `low` to `high`. Found by
    /// value, so it needs no picture.
    Pick {
        from: Tick,
        to: Tick,
        low: u8,
        high: u8,
    },
    /// A handle of note `index` dragged `ticks`, as its finger lifted
    /// (plan 1B.2): its start (`start`, the end staying, as Edit Start) or
    /// its end. The picked notes change with it when it is one of them,
    /// else it changes alone and is picked.
    Resize {
        index: usize,
        rev: u64,
        start: bool,
        ticks: Tick,
    },
    /// Empty space on the roll tapped: with Draw on a note one snap long
    /// goes in, with Draw off the picks are let go.
    Empty { tick: Tick, note: u8 },
    /// A note `len` long put in at `tick`, whatever the tool: a drag on
    /// empty space with Draw on, a double tap with Draw off. It stops at
    /// the clip's end.
    Draw { tick: Tick, note: u8, len: Tick },
    /// An empty step of the tracker tapped, by its line.
    Step(Tick),
    /// Note `index` dragged by this many ticks and notes, as its finger
    /// lifted: the picked notes move with it when it is one of them, else
    /// it is picked alone and moves.
    Drop {
        index: usize,
        rev: u64,
        ticks: Tick,
        notes: i32,
    },
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
    /// An item of the Select menu, [`SELECT_ITEMS`]`[k]` (plan 1B.3).
    Choose(usize),
    /// COPY held + a tap on note `index` (plan 1B.4): the picked notes are
    /// copied when it is one of them, else it alone, picked after: the
    /// editor picks them and the app copies the picks.
    Copy { index: usize, rev: u64 },
    /// COPY held + a tap on empty roll, or on an empty line of the
    /// tracker (plan 1B.4): the notes copied last go in with the first at
    /// `tick`, from this clip or another, their pressure and locks with
    /// them. The app makes the edit ([`Editor::pasted`]).
    Paste { tick: Tick },
}

impl EditAction {
    /// The revision of the picture an action naming a note was made on
    /// (the app refuses it if the clip has moved on).
    pub fn rev(&self) -> Option<u64> {
        match *self {
            EditAction::Note { rev, .. }
            | EditAction::Tap { rev, .. }
            | EditAction::Erase { rev, .. }
            | EditAction::Add { rev, .. }
            | EditAction::Resize { rev, .. }
            | EditAction::Copy { rev, .. }
            | EditAction::Drop { rev, .. } => Some(rev),
            _ => None,
        }
    }
}

/// An edit [`Editor::act`] works out: the change, its join key, whether
/// the notes it puts in are picked after, whether the picked notes it
/// leaves alone stay picked, and the log line.
type Made = (Edit, Option<u64>, bool, bool, String);

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
    /// The velocity last used (a note picked or drawn, the lane): the
    /// next drawn note gets it.
    pub velocity: u8,
    /// Something else changed the clip and took a picked note with it, so
    /// the picks were cleared ([`Editor::follow`]); the legend says so
    /// until the next touch.
    pub cleared: bool,
    /// Add to pick (plan 1B.3): a tap, a box or a filter adds to the
    /// picks (a tap on a picked note takes it out) in place of replacing
    /// them.
    pub add: bool,
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
            velocity: DRAW_VELOCITY,
            cleared: false,
            add: false,
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

    /// Note `n` was picked: its sound is the one worked on, and its
    /// velocity the next drawn note's.
    fn worked_on(&mut self, n: Note) {
        self.sound = n.note;
        self.velocity = n.velocity;
    }

    /// A note `len` long at `tick` on `note`, at the velocity last used,
    /// stopping at the clip's end (plan 1B.1: a choice, not the model's
    /// rule); picked after. None past the end.
    fn draw(&mut self, clip: &Clip, tick: Tick, note: u8, len: Tick) -> Option<Made> {
        if !(0..clip.length).contains(&tick) {
            return None;
        }
        let len = len.max(1).min(clip.length - tick);
        let n = Note::new(tick, len, note.min(127), self.velocity);
        self.sound = n.note;
        Some((
            notes::add(clip, n),
            None,
            true,
            false,
            format!("edit: note {} drawn at tick {tick}", note_name(n.note)),
        ))
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

    /// The picks after picking `new`: in place of `sel`, or added to it
    /// with Add to pick, in order and once each.
    fn picks_with(&self, sel: Vec<usize>, new: Vec<usize>) -> Vec<usize> {
        if !self.add {
            return new;
        }
        let mut all = [sel, new].concat();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// The notes COPY + a tap on note `index` of `clip` copies (plan
    /// 1B.4), the picks `sel` before it: the picks when it is one of
    /// them, else it alone, or none when it is no note.
    fn copying(index: usize, sel: Vec<usize>, clip: &Clip) -> Vec<usize> {
        if sel.contains(&index) {
            sel
        } else if index < clip.notes.len() {
            vec![index]
        } else {
            Vec::new()
        }
    }

    /// The paste COPY + a tap on empty space makes (plan 1B.4): `edit`,
    /// one take, the notes it puts in picked after.
    pub fn pasted(edit: Edit, line: String) -> Done {
        Done {
            edit,
            select_added: true,
            line,
            ..Done::default()
        }
    }

    /// Select menu item `k` ([`SELECT_ITEMS`], plan 1B.3) on `clip`, the
    /// picks `sel` before it. The filters pick from the whole clip, in
    /// place of the picks or added to them with Add to pick; the ranges
    /// and Same pitch work from the picks (Same pitch from the sound
    /// worked on with none). Nothing changes in the clip.
    fn choose(&mut self, k: usize, clip: &Clip, sel: Vec<usize>) -> Done {
        let notes = &clip.notes;
        let all = 0..notes.len();
        let which = |f: &dyn Fn(&Note) -> bool| -> Vec<usize> {
            all.clone().filter(|&i| f(&notes[i])).collect()
        };
        let name = SELECT_ITEMS.get(k).copied().unwrap_or("");
        let first = || sel.iter().map(|&i| notes[i]);
        let new = match k {
            0 => all.clone().collect(),
            1 => Vec::new(),
            2 => all.clone().filter(|i| !sel.contains(i)).collect(),
            SELECT_ADD => {
                self.add = !self.add;
                let on = if self.add { "on" } else { "off" };
                return Done::line(format!("edit: add to pick {on}"));
            }
            SELECT_FIT => return Done::default(),
            5 => {
                let mut pitches: Vec<u8> = first().map(|n| n.note).collect();
                if pitches.is_empty() {
                    pitches.push(self.sound);
                }
                which(&|n| pitches.contains(&n.note))
            }
            6 | 7 if sel.is_empty() => {
                return Done::line(format!(
                    "edit: pick notes first for {}",
                    name.to_lowercase()
                ))
            }
            6 => {
                let lo = first().map(|n| n.note).min().unwrap_or(0);
                let hi = first().map(|n| n.note).max().unwrap_or(127);
                which(&|n| (lo..=hi).contains(&n.note))
            }
            7 => {
                let from = first().map(|n| n.tick).min().unwrap_or(0);
                let to = first().map(|n| n.tick + n.len.max(1)).max().unwrap_or(0);
                which(&|n| (from..to).contains(&n.tick))
            }
            // Quiet and loud: either side of halfway between the clip's
            // softest and loudest notes.
            8 | 9 => {
                let lo = notes.iter().map(|n| n.velocity).min().unwrap_or(0);
                let hi = notes.iter().map(|n| n.velocity).max().unwrap_or(0);
                let mid = (u16::from(lo) + u16::from(hi)) / 2;
                if k == 8 {
                    which(&|n| u16::from(n.velocity) < mid)
                } else {
                    which(&|n| u16::from(n.velocity) > mid)
                }
            }
            10 => which(&|n| n.cond != Cond::Always),
            11 => which(&|n| n.ratchet > 1),
            // Overlapping: a note sounding over another of its pitch.
            12 => {
                let mut by: Vec<usize> = all.clone().collect();
                by.sort_by_key(|&i| (notes[i].note, notes[i].tick));
                let mut over = vec![false; notes.len()];
                let mut end: Option<(u8, Tick, usize)> = None;
                for &i in &by {
                    let n = notes[i];
                    match end {
                        Some((p, e, j)) if p == n.note && n.tick < e => {
                            over[i] = true;
                            over[j] = true;
                            if n.tick + n.len.max(1) > e {
                                end = Some((p, n.tick + n.len.max(1), i));
                            }
                        }
                        _ => end = Some((n.note, n.tick + n.len.max(1), i)),
                    }
                }
                all.clone().filter(|&i| over[i]).collect()
            }
            _ => return Done::default(),
        };
        if let Some(&i) = new.first() {
            self.worked_on(notes[i]);
        }
        self.sel = if k <= 2 {
            new
        } else {
            self.picks_with(sel, new)
        };
        Done::line(format!(
            "edit: {} picked ({})",
            self.sel.len(),
            name.to_lowercase()
        ))
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
                let on = if self.tool == Tool::Draw { "on" } else { "off" };
                return Done::line(format!("edit: draw {on}"));
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
            // A note picked: one already picked keeps the others with it.
            // Its sound is the one worked on, its velocity the next drawn.
            EditAction::Note { index: i, .. } if i < clip.notes.len() => {
                if !sel.contains(&i) {
                    self.sel = if self.add {
                        [sel, vec![i]].concat()
                    } else {
                        vec![i]
                    };
                }
                self.worked_on(clip.notes[i]);
                return Done::default();
            }
            // A tap with Draw on takes out a note no longer than a step,
            // only the one tapped (never every note of its step and
            // pitch); a longer one, or any with Draw off, is picked alone.
            EditAction::Tap { index: i, .. } if i < clip.notes.len() => {
                let n = clip.notes[i];
                if self.tool == Tool::Draw && n.len <= TAP_STEP {
                    self.sound = n.note;
                    (
                        notes::delete(clip, &[i]),
                        None,
                        false,
                        true,
                        format!("edit: note {} taken out", note_name(n.note)),
                    )
                } else if self.add {
                    // Add to pick: in, or out again.
                    self.sel = sel;
                    match self.sel.iter().position(|&s| s == i) {
                        Some(k) => {
                            self.sel.remove(k);
                        }
                        None => {
                            self.sel.push(i);
                            self.worked_on(n);
                        }
                    }
                    return Done::line(format!("edit: {} picked", self.sel.len()));
                } else {
                    self.sel = vec![i];
                    self.worked_on(n);
                    return Done::default();
                }
            }
            EditAction::Erase { index: i, .. } if i < clip.notes.len() => (
                notes::delete(clip, &[i]),
                None,
                false,
                true,
                format!("edit: note {} erased", note_name(clip.notes[i].note)),
            ),
            // A finger holding a note, another tapping: the note tapped
            // joins the picks, or leaves them.
            EditAction::Add { index: i, .. } if i < clip.notes.len() => {
                self.sel = sel;
                match self.sel.iter().position(|&s| s == i) {
                    Some(k) => {
                        self.sel.remove(k);
                    }
                    None => {
                        self.sel.push(i);
                        self.worked_on(clip.notes[i]);
                    }
                }
                return Done::line(format!("edit: {} picked", self.sel.len()));
            }
            EditAction::Note { .. }
            | EditAction::Tap { .. }
            | EditAction::Erase { .. }
            | EditAction::Add { .. } => return Done::default(),
            // A box: the notes sounding in it, in place of the picks.
            EditAction::Pick {
                from,
                to,
                low,
                high,
            } => {
                let (lo, hi) = (low.min(high), low.max(high));
                let (a, b) = (from.min(to), from.max(to).max(from.min(to) + 1));
                let boxed: Vec<usize> = (0..clip.notes.len())
                    .filter(|&i| {
                        let n = clip.notes[i];
                        (lo..=hi).contains(&n.note) && n.tick < b && n.tick + n.len.max(1) > a
                    })
                    .collect();
                if let Some(&i) = boxed.first() {
                    self.worked_on(clip.notes[i]);
                }
                self.sel = self.picks_with(sel, boxed);
                return Done::line(format!("edit: {} picked", self.sel.len()));
            }
            // A handle: the start or the end of the picks, or of the note
            // alone when it wasn't picked.
            EditAction::Resize {
                index,
                start,
                ticks,
                ..
            } => {
                let alone = index < clip.notes.len() && !sel.contains(&index);
                let changed = if alone { vec![index] } else { sel.clone() };
                let (e, what) = if start {
                    (notes::edit_start(clip, &changed, ticks), "starts")
                } else {
                    (notes::edit_end(clip, &changed, ticks), "ends")
                };
                // The picks a handle left as they were stay picked.
                (
                    e,
                    None,
                    true,
                    !alone,
                    format!("edit: {} notes' {what} moved", changed.len()),
                )
            }
            EditAction::Empty { tick, note } => match self.tool {
                Tool::Draw => match self.draw(clip, tick, note, self.step()) {
                    Some(d) => d,
                    None => return Done::default(),
                },
                Tool::Select => {
                    self.sel.clear();
                    return Done::default();
                }
            },
            EditAction::Draw { tick, note, len } => match self.draw(clip, tick, note, len) {
                Some(d) => d,
                None => return Done::default(),
            },
            EditAction::Step(tick) => match self.tool {
                // The editor's sound, a step long, on the step's line.
                Tool::Draw if (0..clip.length).contains(&tick) => {
                    let n = Note::new(tick, self.grid().size(), self.sound, self.velocity);
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
                Tool::Draw => return Done::default(),
            },
            // The note dragged moves its picks with it, or alone when it
            // wasn't picked.
            EditAction::Drop {
                index,
                ticks,
                notes: by,
                ..
            } => {
                let moved = match index < clip.notes.len() && !sel.contains(&index) {
                    true => vec![index],
                    false => sel.clone(),
                };
                if let Some(&i) = moved.first() {
                    self.sound = clip.notes[i].note;
                }
                (
                    notes::shift(clip, &moved, ticks, by),
                    None,
                    true,
                    false,
                    format!("edit: {} notes moved", moved.len()),
                )
            }
            // One a drag of the velocity lane (sent when the finger lifts),
            // so each drag is its own take.
            EditAction::Velocity(v) => (
                notes::set_velocity(clip, &sel, {
                    self.velocity = lane_velocity(v);
                    self.velocity
                }),
                None,
                true,
                true,
                String::new(),
            ),
            EditAction::Choose(k) => return self.choose(k, clip, sel),
            // The app keeps the copy and makes the paste.
            EditAction::Copy { index, .. } => {
                self.sel = Self::copying(index, sel, clip);
                if let Some(&i) = self.sel.first() {
                    self.worked_on(clip.notes[i]);
                }
                return Done::default();
            }
            EditAction::Paste { .. } => return Done::default(),
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

/// Note `i` of `old`, found in `new`: at the same place if it is still
/// there, else the first equal note (its place, length, pitch and
/// velocity), else none.
fn refind(old: &[NoteBox], new: &[NoteBox], i: usize) -> Option<usize> {
    let n = old.get(i)?;
    let key = |m: &NoteBox| (m.tick, m.len, m.note, m.velocity);
    if new.get(i).is_some_and(|m| key(m) == key(n)) {
        return Some(i);
    }
    new.iter().position(|m| key(m) == key(n))
}

/// The velocity the lane's value `v` (0 to 1, [`EditAction::Velocity`])
/// sets: 1 to 127.
pub fn lane_velocity(v: f32) -> u8 {
    (1.0 + v.clamp(0.0, 1.0) * 126.0).round() as u8
}

/// A box pick's lowest and highest rows, from its `Span`'s index
/// ([`BOX`]); None for a drawn note's.
pub fn box_of(index: usize) -> Option<(u8, u8)> {
    let k = index.checked_sub(BOX)?;
    Some(((k >> 8 & 0x7F) as u8, (k & 0x7F) as u8))
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
    /// Add to pick is on (the editor's, plan 1B.3): lit in the Select
    /// menu, and the Select button lit.
    pub add: bool,
    /// The Select menu is open over the roll (the roll's own, plan 1B.3).
    pub menu: bool,
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
    /// When and where empty roll was last tapped with Draw off, for a
    /// double tap (a note goes in).
    empty_tap: Option<(Instant, (i32, i32))>,
    kept: Kept,
}

/// What fingers on the roll are doing, besides dragging a note or the
/// velocity lane ([`Drag`]).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hand {
    Idle,
    /// A finger on a note, not yet a tap, a hold or a drag (piano roll
    /// plan 1B.1): nothing is sent until it lifts (a tap), stays down
    /// [`HOLD`] seconds (a hold picks the note, [`NoteGrid::tick`]) or
    /// moves [`TAP_SLOP`] (a drag).
    Press {
        finger: u64,
        index: usize,
        at: (i32, i32),
        now: (i32, i32),
        /// Seconds down so far.
        held: f32,
        /// The hold picked the note: its lift taps nothing.
        picked: bool,
    },
    /// A finger drawing a note on empty roll with Draw on: from the cell
    /// it went down on to the end of the cell under it now, at least a
    /// snap long, sent when it lifts.
    Pen {
        finger: u64,
        note: u8,
        from: Tick,
        to: Tick,
        now: (i32, i32),
    },
    /// A finger drawing a box on empty roll with Draw off (plan 1B.2):
    /// the notes in it are picked when it lifts.
    Box {
        finger: u64,
        at: (i32, i32),
        now: (i32, i32),
    },
    /// A finger holding a note it picked (a hold) while others tap notes
    /// (plan 1B.2): each tap adds its note to the picks or takes it out,
    /// sent as the tapping finger lifts. `tap` is a finger down on note
    /// `index` at `at`, until it lifts or moves away.
    Holding {
        finger: u64,
        now: (i32, i32),
        tap: Option<(u64, usize, (i32, i32))>,
    },
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
/// Two taps on the overview, or on empty roll with Draw off, this close
/// together are a double tap.
const DOUBLE_TAP: Duration = Duration::from_millis(400);
/// A finger held this long on a note without moving [`TAP_SLOP`] picks
/// it (seconds; the plan's 0.4 s, as on Steps).
pub const HOLD: f32 = 0.4;

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
    /// The finger drags a handle (plan 1B.2): the note's start (true) or
    /// its end, by `ticks`; `notes` stays 0.
    handle: Option<bool>,
}

/// Handles (plan 1B.2) are this wide, outside a picked note's ends; on a
/// note narrower than a 1/16 at the zoom they hang below it, this tall.
const HANDLE_W: i32 = 28;
const HANDLE_H: i32 = 22;
/// Handles show while at most this many picked notes are in view; past
/// that the Q-Links and the Length option do the work.
const MAX_HANDLES: usize = 64;
/// With Draw off a touch this close beside a note, in its row, gets it
/// (the plan's near miss, up to about 3 mm).
const NEAR: i32 = 24;

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
            add: false,
            menu: false,
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
            empty_tap: None,
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
            && (self.length, self.bar, self.pads, self.drawer, self.add)
                == (other.length, other.bar, other.pads, other.drawer, other.add)
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
    /// A finger on a note or dragging one follows its note to where it
    /// now is in `other`'s notes (by value), or lets go of it when it is
    /// gone, so its lift never acts on another note.
    pub fn show(&mut self, other: NoteGrid) {
        let (mut drag, mut hand) = (self.drag, self.hand);
        let refind = |i: usize| refind(&self.notes, &other.notes, i);
        if let Some(d) = drag.as_mut().filter(|d| d.lane.is_none()) {
            match refind(d.index) {
                Some(i) => d.index = i,
                None => drag = None,
            }
        }
        if let Hand::Press { index, .. } = &mut hand {
            match refind(*index) {
                Some(i) => *index = i,
                None => hand = Hand::Idle,
            }
        }
        let (overview_tap, empty_tap, menu) = (self.overview_tap, self.empty_tap, self.menu);
        *self = NoteGrid {
            drag,
            hand,
            overview_tap,
            empty_tap,
            menu,
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
        self.rect_of(g, i, true)
    }

    /// Where note `i` is drawn: with `dragged`, where a drag would leave
    /// it (moved, or its start or end where a handle is), else where it
    /// is in the clip.
    fn rect_of(&self, g: &Geo, i: usize, dragged: bool) -> Option<Rect> {
        let n = self.notes.get(i)?;
        let (tick, len, note) = match self.drag {
            Some(d) if dragged && self.moves(d, i) => self.dragged(d, n)?,
            _ => (n.tick, n.len, n.note),
        };
        let y = self.y_of(g, note)?;
        let x0 = self.x_of(g, tick);
        let x1 = self.x_of(g, (tick + len).min(self.length)).max(x0 + 3);
        Some(Rect::new(x0, y + 1, x1 - x0, g.v.row_h - 2))
    }

    /// Note `n`'s start, length and row with drag `d` (as the edit it
    /// makes would leave it: `shift`, `edit_start`, `edit_end`); None
    /// when it would go off the keys.
    fn dragged(&self, d: Drag, n: &NoteBox) -> Option<(Tick, Tick, u8)> {
        Some(match d.handle {
            Some(true) => {
                let end = n.tick + n.len.max(1);
                let t = (n.tick + d.ticks).min(end - 1).max(0);
                match t < self.length {
                    true => (t, end - t, n.note),
                    false => (n.tick, n.len, n.note),
                }
            }
            Some(false) => (n.tick, (n.len + d.ticks).max(1), n.note),
            None => (
                (n.tick + d.ticks).rem_euclid(self.length.max(1)),
                n.len,
                u8::try_from(i32::from(n.note) + d.notes).ok()?,
            ),
        })
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
        match k {
            1 => self.lens.name().into(),
            // Snap Auto names itself ("Auto · 1/16").
            3 if self.snap_name.starts_with("Auto") => self.snap_name.clone(),
            3 => format!("Snap {}", self.snap_name),
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
        // While notes move or a handle drags, where they were: an outline
        // under where they go (plan 1B.2).
        if let Some(d) = self
            .drag
            .filter(|d| d.lane.is_none() && (d.ticks, d.notes) != (0, 0))
        {
            for &i in &shown {
                if self.moves(d, i) {
                    if let Some(was) = self.rect_of(g, i, false) {
                        cx.canvas.stroke_round_rect(was, 3, 1, t.text_faint);
                    }
                }
            }
        }
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
        // A note being drawn with Draw on: where it goes when the finger
        // lifts.
        if let Hand::Pen { note, from, to, .. } = self.hand {
            if let Some(y) = self.y_of(g, note) {
                let x0 = self.x_of(g, from);
                let x1 = self.x_of(g, to).max(x0 + 3);
                let nr = Rect::new(x0, y + 1, x1 - x0, g.v.row_h - 2);
                cx.canvas.fill_round_rect(nr, 3, shade(DRAW_VELOCITY));
                cx.canvas.stroke_round_rect(nr, 3, 2, top);
            }
        }
        // A box being drawn with Draw off: the notes in it are picked
        // when the finger lifts.
        if let Hand::Box { at, now, .. } = self.hand {
            let (x0, y0) = (at.0.min(now.0), at.1.min(now.1));
            let b = Rect::new(x0, y0, (at.0 - now.0).abs(), (at.1 - now.1).abs());
            cx.canvas.stroke_round_rect(b, 2, 2, top);
        }
        // The picked notes' handles (plan 1B.2): a grip each side, or
        // under a short note.
        let grip = top.mix(t.raised, 0.45);
        for (_, hs) in self.handles(g) {
            for h in hs {
                cx.canvas.fill_round_rect(h.inset(3), 4, grip);
                cx.canvas.fill_rect(
                    Rect::new(h.x + h.w / 2 - 1, h.y + h.h / 4, 2, h.h / 2),
                    t.ink_on(grip),
                );
            }
        }
        self.paint_readout(cx, g);
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

    /// The Select menu's items (plan 1B.3), in [`SELECT_ITEMS`]' order:
    /// two columns of 40 px rows over the roll, standing on the buttons
    /// above Select, what to pick on the left and the filters on the
    /// right.
    fn menu_rects(&self, r: Rect) -> Vec<Rect> {
        const W: i32 = 210;
        const H: i32 = 40;
        const PAD: i32 = 8;
        let rows = (SELECT_ITEMS.len() - SELECT_FILTERS).max(SELECT_FILTERS) as i32;
        let (pw, ph) = (3 * PAD + 2 * W, 2 * PAD + rows * (H + 4) - 4);
        let g = self.geo(r);
        let b = self.button_rects(r)[SELECT_BUTTON];
        let x = b.x.min(r.right() - pw).max(r.x);
        let y = (g.tools.y - GAP - ph).max(r.y);
        (0..SELECT_ITEMS.len())
            .map(|k| {
                let (col, row) = match k < SELECT_FILTERS {
                    true => (0, k as i32),
                    false => (1, (k - SELECT_FILTERS) as i32),
                };
                Rect::new(x + PAD + col * (W + PAD), y + PAD + row * (H + 4), W, H)
            })
            .collect()
    }

    /// The Select menu: a panel of its items, Add to pick lit while on.
    fn paint_menu(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let items = self.menu_rects(r);
        let (Some(first), Some(last)) = (items.first(), items.last()) else {
            return;
        };
        let right = items
            .iter()
            .map(|m| m.right())
            .max()
            .unwrap_or(first.right());
        let bottom = items
            .iter()
            .map(|m| m.bottom())
            .max()
            .unwrap_or(last.bottom());
        let panel = Rect::new(
            first.x - 8,
            first.y - 8,
            right - first.x + 16,
            bottom - first.y + 16,
        );
        cx.canvas.fill_round_rect(panel, t.round(12), t.surface);
        cx.canvas
            .stroke_round_rect(panel, t.round(12), 1, t.outline);
        for (k, m) in items.iter().enumerate() {
            let lit = k == SELECT_ADD && self.add;
            let fill = if lit { t.accent } else { t.raised };
            cx.canvas.fill_round_rect(*m, t.round(8), fill);
            cx.text(
                FontId::Sans,
                t.size_small,
                *m,
                Align::Center,
                t.ink_on(fill),
                SELECT_ITEMS[k],
            );
        }
    }

    /// While a note moves or a handle drags (plan 1B.2), what it will be,
    /// in words, above the finger: the moved note's position and row, or
    /// the start and the length a handle leaves.
    fn paint_readout(&self, cx: &mut PaintCx, g: &Geo) {
        let Some(d) = self.drag.filter(|d| d.lane.is_none()) else {
            return;
        };
        let Some((tick, len, note)) = self.notes.get(d.index).and_then(|n| self.dragged(d, n))
        else {
            return;
        };
        let text = match d.handle {
            Some(true) => format!(
                "Start {} · {}",
                position_name(tick, self.bar),
                length_name(len)
            ),
            Some(false) => format!("Length {}", length_name(len)),
            None => format!(
                "{} · {}",
                position_name(tick, self.bar),
                sound_name(note, self.pads)
            ),
        };
        let t = cx.theme.clone();
        let size = t.size_small;
        let w = cx.measure(FontId::Sans, size, &text) + 20;
        let h = 30;
        // Above the finger, clear of it, kept on the roll.
        let x = (d.now.0 - w / 2).clamp(g.roll.x, (g.roll.right() - w).max(g.roll.x));
        let y = (d.now.1 - 64 - h).max(g.roll.y + 2);
        let r = Rect::new(x, y, w, h);
        cx.canvas.fill_round_rect(r, 6, t.raised);
        cx.canvas.stroke_round_rect(r, 6, 1, t.outline);
        cx.text(
            FontId::Sans,
            size,
            r,
            Align::Center,
            t.ink_on(t.raised),
            &text,
        );
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
                || (k == SELECT_BUTTON && (self.menu || self.add))
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
        if self.menu {
            self.paint_menu(cx, r);
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
        // The Select menu covers the rest while it is open.
        if self.menu {
            for (k, mr) in self.menu_rects(r).into_iter().enumerate() {
                out.push(Target::tap("select", k, SELECT_ITEMS[k], mr));
            }
            return out;
        }
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
        // The picked notes' handles: dragged out, a start earlier and an
        // end later.
        for (i, [a, b]) in self.handles(&g) {
            out.push(
                Target::drag("handle", 2 * i, format!("note {} start", i + 1), a)
                    .towards(-2 * HANDLE_W, 0),
            );
            out.push(
                Target::drag("handle", 2 * i + 1, format!("note {} end", i + 1), b)
                    .towards(2 * HANDLE_W, 0),
            );
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
                // A finger holding a note it picked: another finger on a
                // note taps it into the picks or out of them, as it lifts
                // (plan 1B.2); a finger elsewhere does nothing then.
                let held = match self.hand {
                    Hand::Press {
                        finger,
                        now,
                        picked: true,
                        ..
                    }
                    | Hand::Holding {
                        finger,
                        now,
                        tap: None,
                    } => Some((finger, now)),
                    _ => None,
                };
                if let Some((finger, now)) = held {
                    if let Some(i) = self.note_under(&g, x, y) {
                        self.hand = Hand::Holding {
                            finger,
                            now,
                            tap: Some((f, i, (x, y))),
                        };
                        return None;
                    }
                }
                if matches!(self.hand, Hand::Holding { .. }) {
                    return None;
                }
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
            Touch::Move | Touch::Up | Touch::Cancel
                if busy && !mine && !self.pinched(f) && !self.tapping(f) =>
            {
                None
            }
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
                    // A press that moves far enough is a drag of its note.
                    Hand::Press {
                        finger, index, at, ..
                    } if (x - at.0).abs().max((y - at.1).abs()) >= TAP_SLOP => {
                        self.hand = Hand::Idle;
                        self.drag = Some(Drag {
                            finger,
                            index,
                            x: at.0,
                            y: at.1,
                            now: (x, y),
                            ticks: 0,
                            notes: 0,
                            lane: None,
                            handle: None,
                        });
                    }
                    Hand::Press { ref mut now, .. } => {
                        *now = (x, y);
                        return None;
                    }
                    // The holding finger moves; a tapping finger that
                    // moves away taps nothing.
                    Hand::Holding { finger, now, tap } => {
                        self.hand = if f == finger {
                            Hand::Holding {
                                finger,
                                now: (x, y),
                                tap,
                            }
                        } else {
                            let far =
                                |at: (i32, i32)| (x - at.0).abs().max((y - at.1).abs()) >= TAP_SLOP;
                            Hand::Holding {
                                finger,
                                now,
                                tap: tap.filter(|&(_, _, at)| !far(at)),
                            }
                        };
                        return None;
                    }
                    Hand::Box { finger, at, .. } => {
                        self.hand = Hand::Box {
                            finger,
                            at,
                            now: (x, y),
                        };
                        return None;
                    }
                    // With Draw off, a finger moved on empty roll draws a
                    // box: its notes are picked when it lifts.
                    Hand::Wait {
                        finger,
                        at,
                        moved: false,
                        ..
                    } if self.tool == Tool::Select
                        && (x - at.0).abs().max((y - at.1).abs()) >= TAP_SLOP =>
                    {
                        self.hand = Hand::Box {
                            finger,
                            at,
                            now: (x, y),
                        };
                        return None;
                    }
                    // With Draw on, a finger moved on empty roll draws a
                    // note as long as its drag.
                    Hand::Wait {
                        finger,
                        at,
                        cell: (note, from),
                        moved: false,
                        ..
                    } if self.tool == Tool::Draw
                        && (x - at.0).abs().max((y - at.1).abs()) >= TAP_SLOP =>
                    {
                        self.hand = Hand::Pen {
                            finger,
                            note,
                            from,
                            to: self.pen_end(&g, from, x),
                            now: (x, y),
                        };
                        return None;
                    }
                    Hand::Pen {
                        finger, note, from, ..
                    } => {
                        self.hand = Hand::Pen {
                            finger,
                            note,
                            from,
                            to: self.pen_end(&g, from, x),
                            now: (x, y),
                        };
                        return None;
                    }
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
                if let Some(start) = d.handle {
                    // The edge moves by whole snaps with Snap Auto, onto a
                    // line with a fixed snap.
                    let edge = if start { n.tick } else { n.tick + n.len };
                    let ticks = match self.snap {
                        s if self.snap_auto && s > 0 => (raw as f64 / s as f64).round() as Tick * s,
                        _ => self.snapped(edge + raw) - edge,
                    };
                    self.drag = Some(Drag {
                        ticks,
                        now: (x, y),
                        ..d
                    });
                    return None;
                }
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
                    // The holding finger lifts: nothing more; a tapping
                    // finger lifts: its note joins the picks or leaves.
                    Hand::Holding { finger, now, tap } => {
                        if f == finger {
                            self.hand = Hand::Idle;
                            return None;
                        }
                        self.hand = Hand::Holding {
                            finger,
                            now,
                            tap: None,
                        };
                        return match tap {
                            Some((t, i, _)) if t == f && up => {
                                Some(EventKind::Cell { row: ADD, col: i })
                            }
                            _ => None,
                        };
                    }
                    Hand::Box { at, now, .. } => {
                        self.hand = Hand::Idle;
                        return if up { self.boxed(&g, at, now) } else { None };
                    }
                    // A tap on a note, unless a hold picked it.
                    Hand::Press { index, picked, .. } => {
                        self.hand = Hand::Idle;
                        return (up && !picked).then_some(EventKind::Toggle(index));
                    }
                    Hand::Pen { note, from, to, .. } => {
                        self.hand = Hand::Idle;
                        return up.then_some(EventKind::Span {
                            index: usize::from(note),
                            from,
                            to,
                        });
                    }
                    Hand::Wait {
                        at,
                        cell: (note, tick),
                        moved,
                        ..
                    } => {
                        self.hand = Hand::Idle;
                        if !up || moved {
                            return None;
                        }
                        return Some(self.empty_tapped(at, note, tick));
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
                if let Some(start) = d.handle {
                    let flag = if start { START_HANDLE } else { 0 };
                    return (d.ticks != 0).then_some(EventKind::Stretch {
                        index: d.index + flag,
                        to: d.ticks,
                    });
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
    /// Time passing, `dt` seconds (from the app's tick): a finger held on
    /// a note for [`HOLD`] without moving picks it, `Select(index)`, and
    /// its lift then taps nothing (plan 1B.1, as holding a step on Steps).
    pub fn tick(&mut self, dt: f32) -> Option<EventKind> {
        let Hand::Press {
            index,
            ref mut held,
            ref mut picked,
            ..
        } = self.hand
        else {
            return None;
        };
        if *picked {
            return None;
        }
        *held += dt.max(0.0);
        if *held < HOLD {
            return None;
        }
        *picked = true;
        Some(EventKind::Select(index))
    }

    /// Note `i` moves with drag `d`: the picked notes when the note
    /// dragged is one of them, else the note dragged alone.
    fn moves(&self, d: Drag, i: usize) -> bool {
        if d.lane.is_some() {
            return false;
        }
        match self.notes.get(d.index) {
            Some(n) if n.selected => self.notes.get(i).is_some_and(|m| m.selected),
            _ => i == d.index,
        }
    }

    /// Where a note drawn from tick `from` ends with the finger at `x`:
    /// the end of the snap's cell under the finger (the tick under it
    /// with no snap), at least a snap on from `from`, and never past the
    /// clip's end.
    fn pen_end(&self, g: &Geo, from: Tick, x: i32) -> Tick {
        let t = g.v.tick_at(g.roll, x);
        let (end, unit) = match self.snap {
            0 => (t, FREE_LEN),
            s => ((t.div_euclid(s) + 1) * s, s),
        };
        end.max(from + unit).min(self.length.max(from + 1))
    }

    /// A tap on empty roll at `at`, on row `note` at the cell from
    /// `tick`: `Cell` (with Draw on a note goes in, with Draw off the
    /// picks go), or with Draw off a second tap there soon after a first:
    /// a note one snap long, `Span`, without turning Draw on.
    fn empty_tapped(&mut self, at: (i32, i32), note: u8, tick: Tick) -> EventKind {
        let now = Instant::now();
        let double = self.tool == Tool::Select
            && self.empty_tap.is_some_and(|(t, p)| {
                now.duration_since(t) <= DOUBLE_TAP
                    && (p.0 - at.0).abs().max((p.1 - at.1).abs()) < TAP_SLOP
            });
        if double {
            self.empty_tap = None;
            let unit = if self.snap > 0 { self.snap } else { FREE_LEN };
            return EventKind::Span {
                index: usize::from(note),
                from: tick,
                to: (tick + unit).min(self.length),
            };
        }
        self.empty_tap = (self.tool == Tool::Select).then_some((now, at));
        EventKind::Cell {
            row: usize::from(note),
            col: tick as usize,
        }
    }

    /// The one finger at work (a drag, a tap waiting, a scroll, or the
    /// finger left from a pinch) and where it is now.
    fn finger_now(&self) -> Option<(u64, i32, i32)> {
        if let Some(d) = self.drag {
            return Some((d.finger, d.now.0, d.now.1));
        }
        match self.hand {
            Hand::Press { finger, now, .. }
            | Hand::Pen { finger, now, .. }
            | Hand::Box { finger, now, .. }
            | Hand::Holding { finger, now, .. }
            | Hand::Wait { finger, now, .. }
            | Hand::Scroll { finger, now, .. }
            | Hand::Spent { finger, now } => Some((finger, now.0, now.1)),
            Hand::Idle | Hand::Pinch(_) => None,
        }
    }

    /// Finger `f` is one of a pinch's.
    fn pinched(&self, f: u64) -> bool {
        matches!(self.hand, Hand::Pinch(p) if p.has(f))
    }

    /// Finger `f` taps a note while another holds one.
    fn tapping(&self, f: u64) -> bool {
        matches!(self.hand, Hand::Holding { tap: Some((t, ..)), .. } if t == f)
    }

    /// The topmost note under (`x`, `y`) on the roll (drawn last).
    fn note_under(&self, g: &Geo, x: i32, y: i32) -> Option<usize> {
        if !g.roll.contains(x, y) {
            return None;
        }
        self.in_view(g)
            .into_iter()
            .rev()
            .find(|&i| self.note_rect_in(g, i).is_some_and(|nr| nr.contains(x, y)))
    }

    /// With Draw off, the note nearest (`x`, `y`) in its row, up to
    /// [`NEAR`] beside it (the plan's near miss).
    fn note_near(&self, g: &Geo, x: i32, y: i32) -> Option<usize> {
        if self.tool != Tool::Select || !g.roll.contains(x, y) {
            return None;
        }
        let row = self.note_at(g, y)?;
        self.in_view(g)
            .into_iter()
            .filter(|&i| self.notes[i].note == row)
            .filter_map(|i| {
                let nr = self.note_rect_in(g, i)?;
                let off = (nr.x - x).max(x - (nr.right() - 1)).max(0);
                (off <= NEAR).then_some((off, i))
            })
            .min()
            .map(|(_, i)| i)
    }

    /// A picked note's handles, its start's and its end's: outside its
    /// ends, or under it when it is narrower than a 1/16 at the zoom.
    fn handles_of(&self, g: &Geo, nr: Rect) -> [Rect; 2] {
        let sixteenth = (PPQ as f64 / 4.0 * g.v.ppt).round() as i32;
        if nr.w >= sixteenth {
            [
                Rect::new(nr.x - HANDLE_W, nr.y, HANDLE_W, nr.h),
                Rect::new(nr.right(), nr.y, HANDLE_W, nr.h),
            ]
        } else {
            let mid = nr.x + nr.w / 2;
            let y = nr.bottom() + 1;
            [
                Rect::new(mid - HANDLE_W, y, HANDLE_W, HANDLE_H),
                Rect::new(mid, y, HANDLE_W, HANDLE_H),
            ]
        }
    }

    /// The picked notes in view that show handles, with them: none while
    /// more than [`MAX_HANDLES`] are, or while a note is moved.
    fn handles(&self, g: &Geo) -> Vec<(usize, [Rect; 2])> {
        if self.lens != Lens::Grid
            || self
                .drag
                .is_some_and(|d| d.lane.is_none() && d.handle.is_none())
        {
            return Vec::new();
        }
        let picked: Vec<usize> = self
            .in_view(g)
            .into_iter()
            .filter(|&i| self.notes[i].selected)
            .collect();
        if picked.len() > MAX_HANDLES {
            return Vec::new();
        }
        picked
            .into_iter()
            .filter_map(|i| Some((i, self.handles_of(g, self.note_rect_in(g, i)?))))
            .collect()
    }

    /// The handle under (`x`, `y`): its note, and whether it is the
    /// start's.
    fn handle_at(&self, g: &Geo, x: i32, y: i32) -> Option<(usize, bool)> {
        self.handles(g).into_iter().rev().find_map(|(i, [a, b])| {
            if a.contains(x, y) {
                Some((i, true))
            } else if b.contains(x, y) {
                Some((i, false))
            } else {
                None
            }
        })
    }

    /// A box from `at` to `now` on the roll, as the finger lifts: the
    /// ticks and rows it covers, `Span` with [`BOX`] (plan 1B.2).
    fn boxed(&self, g: &Geo, at: (i32, i32), now: (i32, i32)) -> Option<EventKind> {
        let clamp_x = |x: i32| x.clamp(g.roll.x, g.roll.right() - 1);
        let clamp_y = |y: i32| y.clamp(g.roll.y, g.roll.bottom() - 1);
        let (x0, x1) = (clamp_x(at.0.min(now.0)), clamp_x(at.0.max(now.0)));
        let (y0, y1) = (clamp_y(at.1.min(now.1)), clamp_y(at.1.max(now.1)));
        let high = self.note_at(g, y0)?;
        let low = self.note_at(g, y1)?;
        let from = g.v.tick_at(g.roll, x0).max(0);
        let to = (g.v.tick_at(g.roll, x1) + 1).min(self.length);
        Some(EventKind::Span {
            index: BOX + (usize::from(low) << 8) + usize::from(high),
            from,
            to,
        })
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
        // The Select menu, while open: an item, or a touch anywhere else
        // closes it and does nothing more (plan 1B.3).
        if self.menu {
            let item = self.menu_rects(r).iter().position(|m| m.contains(x, y));
            self.menu = item == Some(SELECT_ADD);
            return match item {
                Some(SELECT_FIT) => {
                    self.fit(g);
                    Some(self.view_event())
                }
                Some(k) => Some(EventKind::Tab(MENU + k)),
                None => None,
            };
        }
        if let Some(k) = self.button_rects(r).iter().position(|b| b.contains(x, y)) {
            // Select opens its menu: the roll's own.
            if k == SELECT_BUTTON {
                self.menu = true;
                return None;
            }
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
                handle: None,
            });
            return None;
        }
        if !g.roll.contains(x, y) {
            return None;
        }
        // A picked note's handle (beside it, or under a short one), when
        // no note is under the finger: its start or end follows the
        // finger, and every picked note's with it.
        if self.note_under(g, x, y).is_none() {
            if let Some((i, start)) = self.handle_at(g, x, y) {
                self.drag = Some(Drag {
                    finger: f,
                    index: i,
                    x,
                    y,
                    now: (x, y),
                    ticks: 0,
                    notes: 0,
                    lane: None,
                    handle: Some(start),
                });
                return None;
            }
        }
        // The topmost note under the finger (drawn last), or with Draw
        // off the nearest beside it in its row.
        if let Some(i) = self.note_under(g, x, y).or_else(|| self.note_near(g, x, y)) {
            // A tap, a hold or a drag: nothing goes until it is one.
            self.hand = Hand::Press {
                finger: f,
                index: i,
                at: (x, y),
                now: (x, y),
                held: 0.0,
                picked: false,
            };
            return None;
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
                index: 2,
                rev: 0,
                ticks: 240,
                notes: 2,
            },
        );
        assert_eq!(done.line, "edit: 3 notes moved");
        assert_eq!(
            ticks_of(&c, &ed.sel),
            vec![(240, 50), (720, 52), (1200, 54)]
        );
        // A note that isn't picked moves alone, and is picked after.
        let lone = c.notes.iter().position(|n| n.tick == 720).unwrap();
        ed.sel = vec![(lone + 1) % 3];
        let done = run(
            &mut ed,
            &mut c,
            EditAction::Drop {
                index: lone,
                rev: 0,
                ticks: 240,
                notes: 0,
            },
        );
        assert_eq!(done.line, "edit: 1 notes moved");
        assert_eq!(ticks_of(&c, &ed.sel), vec![(960, 52)]);
        assert_eq!(c.notes.len(), 3);
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
        // ERASE + a tap, whatever the tool.
        run(&mut ed, &mut c, EditAction::Erase { index: 0, rev: 0 });
        assert_eq!(c.notes.len(), 1);
    }

    /// Draw on is the step feel (piano roll plan 1B.1): a tap takes out a
    /// note no longer than a step, only the one tapped, and picks a
    /// longer one; Draw off, a tap picks the note alone. A drawn note
    /// gets the velocity last used and stops at the clip's end.
    #[test]
    fn draw_on_taps_notes_out_or_picks_them_and_draws_to_the_end() {
        // Two notes of one step and pitch, and a long one.
        let mut c = clip(&[(0, 240, 60), (0, 120, 60), (480, 960, 62)]);
        let mut ed = Editor::new(0, 0, &c);
        ed.act(EditAction::Button(2), &c);
        assert_eq!(ed.tool, Tool::Draw);
        let done = run(&mut ed, &mut c, EditAction::Tap { index: 1, rev: 0 });
        assert_eq!(done.line, "edit: note C3 taken out");
        assert_eq!(c.notes.len(), 2, "only the note tapped");
        assert_eq!((c.notes[0].tick, c.notes[0].len), (0, 240));
        // The long note: picked, nothing taken out.
        let long = c.notes.iter().position(|n| n.len == 960).unwrap();
        let done = ed.act(
            EditAction::Tap {
                index: long,
                rev: 0,
            },
            &c,
        );
        assert!(done.edit.is_empty());
        assert_eq!(ed.sel, [long]);
        // Its velocity is the next drawn note's.
        c.notes[long].velocity = 77;
        ed.act(
            EditAction::Note {
                index: long,
                rev: 0,
            },
            &c,
        );
        assert_eq!(ed.velocity, 77);
        // Drawn to past the end: it stops there.
        run(
            &mut ed,
            &mut c,
            EditAction::Draw {
                tick: 3600,
                note: 64,
                len: 960,
            },
        );
        let n = c.notes.iter().find(|n| n.note == 64).unwrap();
        assert_eq!((n.tick, n.len, n.velocity), (3600, 240, 77));
        assert_eq!(ticks_of(&c, &ed.sel), [(3600, 64)]);
        // Draw off: a tap picks the note alone, even out of a group.
        ed.act(EditAction::Button(2), &c);
        assert_eq!(ed.tool, Tool::Select);
        ed.sel = (0..c.notes.len()).collect();
        let done = ed.act(EditAction::Tap { index: 0, rev: 0 }, &c);
        assert!(done.edit.is_empty());
        assert_eq!(ed.sel, [0]);
        // An empty tap lets the picks go; Draw puts a note in anyway.
        ed.act(EditAction::Empty { tick: 0, note: 70 }, &c);
        assert!(ed.sel.is_empty());
        run(
            &mut ed,
            &mut c,
            EditAction::Draw {
                tick: 960,
                note: 70,
                len: 240,
            },
        );
        assert_eq!(c.notes.len(), 4);
        assert_eq!(EditAction::Tap { index: 0, rev: 9 }.rev(), Some(9));
        assert_eq!(EditAction::Empty { tick: 0, note: 1 }.rev(), None);
    }

    /// Picking (plan 1B.2): a box picks the notes sounding in it, in
    /// place of the others; a note tapped while another is held joins the
    /// picks or leaves them; a handle moves the picks' starts or ends, or
    /// the note's alone when it wasn't picked.
    #[test]
    fn a_box_picks_a_held_note_adds_and_handles_change_starts_and_ends() {
        let mut c = clip(&[
            (0, 240, 60),
            (240, 240, 62),
            (960, 480, 64),
            (1920, 240, 72),
        ]);
        let mut ed = Editor::new(0, 0, &c);
        // A box over the first beat and a half, rows 60 to 64: the notes
        // sounding there, whichever way it was drawn.
        let done = ed.act(
            EditAction::Pick {
                from: 1200,
                to: 100,
                low: 64,
                high: 60,
            },
            &c,
        );
        assert!(done.edit.is_empty());
        assert_eq!(done.line, "edit: 3 picked");
        assert_eq!(ticks_of(&c, &ed.sel), [(0, 60), (240, 62), (960, 64)]);
        // A note that only sounds into the box counts; one past it doesn't.
        ed.act(
            EditAction::Pick {
                from: 1000,
                to: 1100,
                low: 0,
                high: 127,
            },
            &c,
        );
        assert_eq!(ticks_of(&c, &ed.sel), [(960, 64)]);
        // Held and tapped: in, then out again.
        ed.act(EditAction::Add { index: 3, rev: 0 }, &c);
        assert_eq!(ticks_of(&c, &ed.sel), [(960, 64), (1920, 72)]);
        assert_eq!(ed.sound, 72, "the note added is the one worked on");
        ed.act(EditAction::Add { index: 2, rev: 0 }, &c);
        assert_eq!(ticks_of(&c, &ed.sel), [(1920, 72)]);
        assert_eq!(EditAction::Add { index: 0, rev: 4 }.rev(), Some(4));
        // An end handle of a picked note: every pick's end, one edit.
        ed.act(EditAction::Add { index: 0, rev: 0 }, &c);
        let done = run(
            &mut ed,
            &mut c,
            EditAction::Resize {
                index: 0,
                rev: 0,
                start: false,
                ticks: 240,
            },
        );
        assert_eq!(done.line, "edit: 2 notes' ends moved");
        let lens = |c: &Clip| -> Vec<(Tick, Tick, u8)> {
            let mut v: Vec<_> = c.notes.iter().map(|n| (n.tick, n.len, n.note)).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(
            lens(&c),
            [
                (0, 480, 60),
                (240, 240, 62),
                (960, 480, 64),
                (1920, 480, 72)
            ]
        );
        assert_eq!(ed.sel.len(), 2, "they stay picked");
        // A start handle of a note not picked: it alone, picked after;
        // its end stays.
        let i = c.notes.iter().position(|n| n.note == 64).unwrap();
        run(
            &mut ed,
            &mut c,
            EditAction::Resize {
                index: i,
                rev: 0,
                start: true,
                ticks: -240,
            },
        );
        assert_eq!(
            lens(&c),
            [
                (0, 480, 60),
                (240, 240, 62),
                (720, 720, 64),
                (1920, 480, 72)
            ]
        );
        assert_eq!(ticks_of(&c, &ed.sel), [(720, 64)]);
        assert_eq!(
            EditAction::Resize {
                index: 0,
                rev: 7,
                start: true,
                ticks: 1
            }
            .rev(),
            Some(7)
        );
    }

    /// COPY + a tap (plan 1B.4): on a picked note the picks stay, on
    /// another it is picked alone, and a note gone picks nothing; the
    /// action carries its picture's revision, and the app's paste picks
    /// the notes it put in.
    #[test]
    fn copy_picks_what_it_copies_and_a_paste_picks_what_went_in() {
        let c = clip(&[(0, 240, 60), (240, 240, 62), (480, 240, 64)]);
        let mut ed = Editor::new(0, 0, &c);
        ed.sel = vec![0, 2];
        ed.act(EditAction::Copy { index: 2, rev: 0 }, &c);
        assert_eq!(ed.sel, [0, 2]);
        ed.act(EditAction::Copy { index: 1, rev: 0 }, &c);
        assert_eq!(ed.sel, [1]);
        ed.act(EditAction::Copy { index: 9, rev: 0 }, &c);
        assert!(ed.sel.is_empty());
        assert_eq!(EditAction::Copy { index: 1, rev: 7 }.rev(), Some(7));
        assert_eq!(EditAction::Paste { tick: 0 }.rev(), None);
        assert_eq!(ed.act(EditAction::Paste { tick: 0 }, &c), Done::default());
        let copied = notes::copy(&c, &[0, 1]);
        let carried = notes::carry(&c, &copied, 960, |_| true).unwrap();
        let done = Editor::pasted(carried.edit, "paste".into());
        let mut d = c.clone();
        d.apply(&done.edit).unwrap();
        ed.applied(&d, &done);
        let mut picked: Vec<Tick> = ed.sel.iter().map(|&i| d.notes[i].tick).collect();
        picked.sort_unstable();
        assert_eq!(picked, [960, 1200]);
    }

    /// The Select menu (plan 1B.3): All, None and Invert; the filters,
    /// each picking from the whole clip (Same pitch and the ranges from
    /// the picks); Add to pick making taps, boxes and filters add.
    #[test]
    fn the_select_menu_picks_by_filter_and_add_to_pick_adds() {
        let mut c = clip(&[
            (0, 240, 60),
            (240, 240, 62),
            (480, 960, 60),
            (960, 240, 60),
            (1920, 240, 72),
        ]);
        c.notes[1].velocity = 30;
        c.notes[4].velocity = 127;
        c.notes[2].cond = Cond::Fill;
        c.notes[3].ratchet = 3;
        let mut ed = Editor::new(0, 0, &c);
        let pick = |ed: &mut Editor, k: usize| ed.act(EditAction::Choose(k), &c);
        let picked = |ed: &mut Editor, k: usize| {
            pick(ed, k);
            let mut s = ed.sel.clone();
            s.sort_unstable();
            s
        };
        assert_eq!(picked(&mut ed, 0), [0, 1, 2, 3, 4], "All");
        assert_eq!(picked(&mut ed, 1), Vec::<usize>::new(), "None");
        ed.sel = vec![0, 1];
        assert_eq!(picked(&mut ed, 2), [2, 3, 4], "Invert");
        // Same pitch: the picks' pitches, or the sound worked on.
        ed.sel = vec![1];
        assert_eq!(picked(&mut ed, 5), [1]);
        ed.sel = vec![0];
        assert_eq!(picked(&mut ed, 5), [0, 2, 3]);
        assert_eq!(pick(&mut ed, 5).line, "edit: 3 picked (same pitch)");
        // The ranges work from the picks, and say so with none.
        ed.sel = vec![1, 4];
        assert_eq!(picked(&mut ed, 6), [1, 4], "62 to 72 leaves the 60s");
        ed.sel = vec![0, 4];
        assert_eq!(picked(&mut ed, 6), [0, 1, 2, 3, 4], "60 to 72");
        ed.sel = vec![1];
        assert_eq!(picked(&mut ed, 6), [1]);
        ed.sel = vec![0, 1];
        assert_eq!(picked(&mut ed, 7), [0, 1], "starting from 0 to 480");
        ed.sel.clear();
        assert_eq!(
            pick(&mut ed, 7).line,
            "edit: pick notes first for time range"
        );
        // Quiet and loud: either side of halfway (30 to 127).
        assert_eq!(picked(&mut ed, 8), [1]);
        assert_eq!(picked(&mut ed, 9), [0, 2, 3, 4]);
        assert_eq!(picked(&mut ed, 10), [2], "with a condition");
        assert_eq!(picked(&mut ed, 11), [3], "with repeats");
        // Overlapping: the long note and the two of its pitch it covers
        // (the first only touches its start, the last starts under it).
        assert_eq!(picked(&mut ed, 12), [2, 3]);
        // Add to pick: a switch; then a tap adds or takes out, a box and
        // a filter add.
        assert_eq!(pick(&mut ed, SELECT_ADD).line, "edit: add to pick on");
        assert!(ed.add);
        ed.sel = vec![4];
        ed.act(EditAction::Tap { index: 1, rev: 0 }, &c);
        assert_eq!(ed.sel, [4, 1]);
        ed.act(EditAction::Tap { index: 4, rev: 0 }, &c);
        assert_eq!(ed.sel, [1]);
        ed.act(
            EditAction::Pick {
                from: 0,
                to: 100,
                low: 0,
                high: 127,
            },
            &c,
        );
        assert_eq!(ed.sel, [0, 1]);
        assert_eq!(picked(&mut ed, 11), [0, 1, 3]);
        ed.act(EditAction::Note { index: 4, rev: 0 }, &c);
        assert_eq!(ed.sel.len(), 4, "a hold adds too");
        // All, None and Invert are whole picks either way.
        assert_eq!(picked(&mut ed, 1), Vec::<usize>::new());
        pick(&mut ed, SELECT_ADD);
        assert!(!ed.add);
        // Fit to picked is the roll's: nothing here.
        assert_eq!(pick(&mut ed, SELECT_FIT), Done::default());
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
        assert_eq!(g.touch(1, Touch::Down, x, y, R), None);
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
        assert_eq!(g.touch(6, Touch::Down, x, y, R), None);
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
        // A note: nothing until the finger lifts, then a tap.
        assert_eq!(g.touch(1, Touch::Down, x, y, R), None);
        assert_eq!(g.touch(1, Touch::Up, x, y, R), Some(EventKind::Toggle(1)));
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
        assert_eq!(g.button_label(2), "Draw");
        assert_eq!(g.button_label(3), "Snap 1/16");
        g.snap_name = "Auto · 1/8".into();
        assert_eq!(g.button_label(3), "Auto · 1/8");
    }

    /// A finger on a note is a tap, a hold or a drag before anything
    /// goes (piano roll plan 1B.1): a hold picks the note and its lift
    /// taps nothing; a drag moves the note alone when it isn't picked.
    /// With Draw on a drag on empty roll draws a note as long as the drag;
    /// with Draw off a double tap there puts one in.
    #[test]
    fn a_finger_is_a_tap_a_hold_or_a_drag_before_anything_goes() {
        let mut g = grid();
        let n0 = g.note_rect(R, 0).unwrap();
        let (x, y) = mid(n0);
        // A hold: nothing until 0.4 s, then the pick; the lift taps nothing.
        assert_eq!(g.touch(1, Touch::Down, x, y, R), None);
        assert_eq!(g.tick(0.3), None);
        assert_eq!(g.touch(1, Touch::Move, x + 10, y, R), None, "not far");
        assert_eq!(g.tick(0.15), Some(EventKind::Select(0)));
        assert_eq!(g.tick(1.0), None, "once");
        assert_eq!(g.touch(1, Touch::Up, x + 10, y, R), None);
        // A drag of a note that isn't picked: it moves alone.
        let beat = (f64::from(g.geo(R).roll.w) / 4.0).round() as i32;
        let n1 = g.note_rect(R, 1).unwrap();
        assert_eq!(g.touch(2, Touch::Down, x, y, R), None);
        assert_eq!(g.touch(2, Touch::Move, x + beat + 3, y, R), None);
        assert_ne!(g.note_rect(R, 0), Some(n0), "the note dragged moves");
        assert_eq!(g.note_rect(R, 1), Some(n1), "the picked one stays");
        assert!(matches!(
            g.touch(2, Touch::Up, x + beat + 3, y, R),
            Some(EventKind::Marker { index: 0, .. })
        ));
        // A press follows its note when the notes change under it, and
        // lets go when its note is gone.
        assert_eq!(g.touch(3, Touch::Down, x, y, R), None);
        let mut swapped = g.clone();
        swapped.notes.swap(0, 1);
        g.show(swapped.clone());
        assert_eq!(g.touch(3, Touch::Up, x, y, R), Some(EventKind::Toggle(1)));
        assert_eq!(g.touch(3, Touch::Down, x, y, R), None);
        let mut gone = swapped;
        gone.notes.remove(1);
        g.show(gone);
        assert_eq!(g.touch(3, Touch::Up, x, y, R), None);
        // Draw on: a drag on empty roll draws, from the cell it started
        // in to the end of the cell under the finger, shown as it goes.
        let mut g = grid();
        g.tool = Tool::Draw;
        g.snap = 240;
        let n0 = g.note_rect(R, 0).unwrap();
        let (ex, ey) = (n0.x + n0.w * 3, n0.y + n0.h / 2);
        let before = painted(&g);
        assert_eq!(g.touch(4, Touch::Down, ex, ey, R), None);
        assert_eq!(g.touch(4, Touch::Move, ex + beat, ey, R), None);
        assert!(differ_in(&before, &painted(&g), g.geo(R).roll));
        match g.touch(4, Touch::Up, ex + beat, ey, R) {
            Some(EventKind::Span { index, from, to }) => {
                assert_eq!(index, 50);
                assert_eq!(from % 240, 0);
                assert_eq!(to % 240, 0);
                assert!(to - from >= 960, "{from}..{to}");
            }
            other => panic!("{other:?}"),
        }
        // A tap there with Draw on: a cell, twice (no double tap).
        for _ in 0..2 {
            assert_eq!(g.touch(5, Touch::Down, ex, ey, R), None);
            assert!(matches!(
                g.touch(5, Touch::Up, ex, ey, R),
                Some(EventKind::Cell { row: 50, .. })
            ));
        }
        // Draw off: the second of two quick taps puts a note in.
        g.tool = Tool::Select;
        g.touch(6, Touch::Down, ex, ey, R);
        assert!(matches!(
            g.touch(6, Touch::Up, ex, ey, R),
            Some(EventKind::Cell { .. })
        ));
        g.touch(6, Touch::Down, ex + 5, ey, R);
        match g.touch(6, Touch::Up, ex + 5, ey, R) {
            Some(EventKind::Span {
                index: 50,
                from,
                to,
            }) => assert_eq!(to - from, 240),
            other => panic!("{other:?}"),
        }
        // A drag on empty roll with Draw off draws a box (plan 1B.2).
        g.touch(7, Touch::Down, ex, ey, R);
        g.touch(7, Touch::Move, ex + beat, ey, R);
        assert!(matches!(
            g.touch(7, Touch::Up, ex + beat, ey, R),
            Some(EventKind::Span { index, .. }) if box_of(index) == Some((50, 50))
        ));
    }

    /// Picking on the roll (plan 1B.2): a drag on empty roll with Draw off
    /// draws a box and picks what it covers as it lifts; a finger holding
    /// a note lets another tap notes into the picks or out; a picked
    /// note's handles drag its start or end (under a short note), drawn
    /// where they go over an outline of where they were, with a readout;
    /// with Draw off a near miss beside a note gets it.
    #[test]
    fn a_box_a_held_note_and_handles_pick_and_change_notes() {
        let mut g = grid();
        let (n0, n1) = (g.note_rect(R, 0).unwrap(), g.note_rect(R, 1).unwrap());
        let beat = (f64::from(g.geo(R).roll.w) / 4.0).round() as i32;
        // A box from right of the picked note down to the first one.
        let before = painted(&g);
        let at = (n1.right() + 60, n1.y + 2);
        assert_eq!(g.touch(1, Touch::Down, at.0, at.1, R), None);
        assert_eq!(g.touch(1, Touch::Move, n0.x + 5, n0.bottom() - 2, R), None);
        assert!(
            differ_in(&before, &painted(&g), g.geo(R).roll),
            "the box shows"
        );
        match g.touch(1, Touch::Up, n0.x + 5, n0.bottom() - 2, R) {
            Some(EventKind::Span { index, from, to }) => {
                assert_eq!(box_of(index), Some((50, 52)));
                assert!(from < 240 && to > 1440, "{from}..{to}");
            }
            other => panic!("{other:?}"),
        }
        // Hold a note, then tap another: it joins or leaves the picks as
        // the tapping finger lifts; a tap that moves away does nothing,
        // and the holding finger's lift sends nothing.
        let (x0, y0) = mid(n0);
        let (x1, y1) = mid(n1);
        g.touch(2, Touch::Down, x0, y0, R);
        assert_eq!(g.tick(0.5), Some(EventKind::Select(0)));
        assert_eq!(g.touch(3, Touch::Down, x1, y1, R), None);
        assert_eq!(
            g.touch(3, Touch::Up, x1, y1, R),
            Some(EventKind::Cell { row: ADD, col: 1 })
        );
        g.touch(4, Touch::Down, x1, y1, R);
        g.touch(4, Touch::Move, x1 + 2 * TAP_SLOP, y1, R);
        assert_eq!(g.touch(4, Touch::Up, x1 + 2 * TAP_SLOP, y1, R), None);
        assert_eq!(g.touch(2, Touch::Up, x0, y0, R), None);
        // The picked note's end handle: a beat later, drawn as it goes,
        // with its old place outlined and a readout.
        let before = painted(&g);
        let end = (n1.right() + HANDLE_W / 2, y1);
        assert_eq!(g.touch(5, Touch::Down, end.0, end.1, R), None);
        assert_eq!(g.touch(5, Touch::Move, end.0 + beat, end.1, R), None);
        let grown = g.note_rect(R, 1).unwrap().right() - n1.right();
        assert!((grown - beat).abs() <= 1, "{grown} {beat}");
        assert!(differ_in(&before, &painted(&g), g.geo(R).roll));
        assert_eq!(
            g.touch(5, Touch::Up, end.0 + beat, end.1, R),
            Some(EventKind::Stretch { index: 1, to: 960 })
        );
        assert_eq!(g.note_rect(R, 1), Some(n1), "back until the clip says");
        // Its start handle, a beat earlier.
        let start = (n1.x - HANDLE_W / 2, y1);
        g.touch(6, Touch::Down, start.0, start.1, R);
        g.touch(6, Touch::Move, start.0 - beat, start.1, R);
        assert_eq!(
            g.touch(6, Touch::Up, start.0 - beat, start.1, R),
            Some(EventKind::Stretch {
                index: 1 + START_HANDLE,
                to: -960
            })
        );
        // A note narrower than a 1/16: its handles hang below it.
        g.notes[1].len = 60;
        let short = g.note_rect(R, 1).unwrap();
        let below = (
            short.x + short.w / 2 + HANDLE_W / 2,
            short.bottom() + HANDLE_H / 2,
        );
        g.touch(7, Touch::Down, below.0, below.1, R);
        g.touch(7, Touch::Move, below.0 + beat, below.1, R);
        assert!(matches!(
            g.touch(7, Touch::Up, below.0 + beat, below.1, R),
            Some(EventKind::Stretch { index: 1, .. })
        ));
        // Swept as controls: both handles of the picked note.
        let handles: Vec<_> = g
            .targets(R)
            .into_iter()
            .filter(|t| t.id == "handle")
            .collect();
        assert_eq!(handles.len(), 2);
        // A near miss beside the first note, Draw off: that note.
        let near = (n0.right() + NEAR / 2, y0);
        g.touch(8, Touch::Down, near.0, near.1, R);
        assert_eq!(
            g.touch(8, Touch::Up, near.0, near.1, R),
            Some(EventKind::Toggle(0))
        );
        // With Draw on the same place is empty: a note goes in there.
        g.tool = Tool::Draw;
        g.touch(9, Touch::Down, near.0, near.1, R);
        assert!(matches!(
            g.touch(9, Touch::Up, near.0, near.1, R),
            Some(EventKind::Cell { row: 50, .. })
        ));
    }

    /// Select opens its menu over the roll (plan 1B.3): an item sends
    /// its pick and closes it, Add to pick keeps it open, Fit to picked
    /// moves the view, and a touch anywhere else only closes it.
    #[test]
    fn the_select_button_opens_its_menu_over_the_roll() {
        let mut g = grid();
        let b = g.button_rects(R)[SELECT_BUTTON];
        assert_eq!(g.button_label(SELECT_BUTTON), "Select");
        let before = painted(&g);
        assert_eq!(g.touch(1, Touch::Down, mid(b).0, mid(b).1, R), None);
        g.touch(1, Touch::Up, mid(b).0, mid(b).1, R);
        assert!(g.menu);
        assert!(differ_in(&before, &painted(&g), g.geo(R).roll), "it shows");
        let items: Vec<Target> = g
            .targets(R)
            .into_iter()
            .filter(|t| t.id == "select")
            .collect();
        assert_eq!(items.len(), SELECT_ITEMS.len());
        assert!(
            g.targets(R)
                .iter()
                .all(|t| t.id == "select" || t.id == "button"),
            "it covers the roll"
        );
        let at = |k: usize| mid(items[k].rect);
        let (x, y) = at(5);
        assert_eq!(
            g.touch(2, Touch::Down, x, y, R),
            Some(EventKind::Tab(MENU + 5))
        );
        g.touch(2, Touch::Up, x, y, R);
        assert!(!g.menu, "closed after a pick");
        // Add to pick keeps it open.
        g.touch(3, Touch::Down, mid(b).0, mid(b).1, R);
        g.touch(3, Touch::Up, mid(b).0, mid(b).1, R);
        let (x, y) = at(SELECT_ADD);
        assert_eq!(
            g.touch(4, Touch::Down, x, y, R),
            Some(EventKind::Tab(MENU + SELECT_ADD))
        );
        g.touch(4, Touch::Up, x, y, R);
        assert!(g.menu);
        // Fit to picked: the view, the menu closed.
        let (x, y) = at(SELECT_FIT);
        assert!(matches!(
            g.touch(5, Touch::Down, x, y, R),
            Some(EventKind::View { .. })
        ));
        g.touch(5, Touch::Up, x, y, R);
        assert!(!g.menu);
        // A touch elsewhere only closes it: no note goes in.
        g.menu = true;
        let n0 = g.note_rect(R, 0).unwrap();
        let (x, y) = mid(n0);
        assert_eq!(g.touch(6, Touch::Down, x, y, R), None);
        assert_eq!(g.touch(6, Touch::Up, x, y, R), None);
        assert!(!g.menu);
        // A new picture keeps it as it is; Add to pick lights Select.
        g.menu = true;
        let mut next = grid();
        next.add = true;
        g.show(next);
        assert!(g.menu && g.add);
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
