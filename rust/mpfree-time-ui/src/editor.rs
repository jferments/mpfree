//! The note editor, the Sequencer tab's Notes view (the MPC's Grid View
//! and List Edit; it was the Arrange page's until the Play, Pads and
//! Sequencer redesign): one clip's notes through one of three lenses ([`Lens`]): a
//! piano roll, a list, or a tracker (one row per step, empty steps
//! included). The tools run along the bottom, and beside it the Edit
//! options: Timing Correct; the selection's nudge, transpose, velocity,
//! length, chance, condition and ratchet; the Euclidean fill; Fit to
//! scale; and the tools (Strum, Invert, Spread, Chop, Legato, Humanize),
//! each previewed on the roll until Done or Cancel.
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
//! - a lane under the roll (Velocity, Chance or Repeats, as its chip
//!   picks; SHIFT + a stroke on Repeats sets the ramp): `Values { index,
//!   set }` once, when the finger lifts, `index` the lane's place in
//!   [`FIELDS`] and `set` each note the stroke changed with its new value
//!   (piano roll plan 2A.1); the chip, its fold and the lane's top edge
//!   are the screen's own (`View`);
//! - the Pressure lane (plan 3A.1, [`LaneKind::Pressure`]): the picked
//!   notes' pressure as a line along them, a narrow column each few
//!   pixels; a stroke sends `Values { index: `[`PRESSURE_LANE`]`, set }`
//!   once, `set` each column it drew as (its first tick, its value) in
//!   time order, then (the stroke's end tick, [`PRESSURE_END`]);
//! - the buttons along the bottom: `Tab(k)` for [`EDIT_BUTTONS`]`[k]`
//!   (the last, Edit, opens the Edit options' drawer: the page's own);
//!   while a tool is open (piano roll plan 2B.3, [`NoteTool`]), its bar
//!   in their place: `Tab(`[`TOOL_BUTTONS`]` + k)` for Cancel, its own
//!   button and Done, the roll showing the tool's preview
//!   ([`Editor::preview`]) and only moving the view;
//! - the view moved (a pinch, two fingers, a finger on the ruler, the
//!   keys or the overview, Fit): `View`, when the fingers lift, so the
//!   page can keep it and Snap Auto follow it. The view itself
//!   ([`zoom::RollView`]) is the screen's alone: nothing of it goes to the
//!   time engine but the division Snap Auto picks
//!   ([`EditAction::AutoSnap`]).
//!
//! The MPC's controls send their own actions: the note inspector's
//! Q-Links ([`EditAction::Knob`], one take a turn, [`KNOBS`]), – / + with
//! the drawer closed ([`EditAction::Nudge`]) and the wheel's cursor
//! ([`EditAction::Cursor`], drawn as `NoteGrid::cursor`; piano roll plan
//! 1C.1).
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

use crate::lane::{Bar, Lane, LaneStyle, LaneTouch};
use mpfree_seq::mpfree_param::RelAddr;
use mpfree_seq::mpfree_scale::chords::{self, ChordName, Shape};
use mpfree_seq::mpfree_scale::{NOTE_NAMES, SCALES};
use mpfree_seq::notes::{self, Field, Snap, Tc, TcKind, Which, DIVISIONS};
use mpfree_seq::steps::{Edit, Euclid, Grid, MAX_EUCLID_STEPS};
use mpfree_seq::{Clip, Cond, Holds, Join, Lock, Note, Scale, TouchPoint};
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
pub const SELECT_ITEMS: [&str; 14] = [
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
    "Off the scale",
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
/// once; the next four are the Euclidean fill: the sound it writes, then
/// its hits, steps and rotation, each turn writing the pattern; then Fit
/// to scale (plan 2B.1), and the tools Strum, Invert and Spread (plan
/// 2B.2), Chop, Legato and Humanize (plan 2B.3), each opening its tool.
pub const EDIT_ROWS: [&str; 24] = [
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
    "Fit to scale",
    "Strum",
    "Invert",
    "Spread",
    "Chop",
    "Legato",
    "Humanize",
];
/// The first of the Euclidean fill's rows in [`EDIT_ROWS`].
pub const EUCLID_ROW: usize = 13;
/// [`EDIT_ROWS`]' Fit to scale (piano roll plan 2B.1): a turn either way
/// brings the picks, or every note, onto the clip's scale.
pub const FIT_ROW: usize = 17;
/// [`EDIT_ROWS`]' tools (piano roll plans 2B.2 and 2B.3), in
/// [`ToolKind::ALL`]'s order: a step or the wheel on one opens its tool
/// ([`NoteTool`]); on the open tool's row it turns Q1.
pub const STRUM_ROW: usize = 18;
pub const INVERT_ROW: usize = 19;
pub const SPREAD_ROW: usize = 20;
pub const CHOP_ROW: usize = 21;
pub const LEGATO_ROW: usize = 22;
pub const HUMANIZE_ROW: usize = 23;
/// How much a detent of Strum's Apart moves a chord's notes apart: about
/// 5 ms at 120 BPM (SHIFT: a tick).
pub const STRUM_TICKS: Tick = PPQ as Tick / 96;
/// The tool bar's buttons (plan 2B.3): `Tab`'s index is this plus the
/// button's place along the bottom ([`TOOL_CANCEL`], [`TOOL_OPTION`],
/// [`TOOL_DONE`]).
pub const TOOL_BUTTONS: usize = 600;
/// The Pads write chip tapped (plan 1C.2): `Tab`'s index.
pub const PADS_WRITE: usize = 700;
/// The Show menu (plan 4.1): `Tab(SHOW_MENU + k)` is its `k`th offer
/// ([`NoteGrid::show_offers`]), `Tab(SHOW_NONE)` its None.
pub const SHOW_MENU: usize = 800;
pub const SHOW_NONE: usize = 790;
/// A tap on a faint note of another clip (plan 4.1): `Tab(GHOST_TAP +
/// k)` for [`NoteGrid::ghosts`]' `k`th, the cell it was on kept in
/// [`NoteGrid::ghost_cell`].
pub const GHOST_TAP: usize = 4000;
/// The most notes of one other clip the roll places (plan 4.1).
pub const GHOST_MOST: usize = 2048;
pub const TOOL_CANCEL: usize = 0;
pub const TOOL_OPTION: usize = 1;
pub const TOOL_DONE: usize = 7;
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
/// ERASE held + a drag across the roll ([`EditAction::Sweep`], plan
/// 1B.5): `Values`' index; its `set` is (note, 0) for each note the
/// finger's path crossed.
pub const ERASE_SWEEP: usize = 1 << 28;
/// The clip's end handle dragged ([`EditAction::Length`], plan 1A.2):
/// `Stretch`'s index; its `to` is the clip's new length.
pub const CLIP_END: usize = usize::MAX - 2;
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
/// The note inspector's knobs (piano roll plan 1C.1), Q1 to Q4, and
/// their cards' names ([`EditAction::Knob`]).
pub const KNOBS: [&str; 4] = ["Position", "Length", "Semitones", "Velocity"];

/// Q-Link `k`'s card name: Q3 says Scale steps while the clip's scale is
/// on (`scale`; SHIFT still moves a semitone), else [`KNOBS`]' own.
pub fn knob_name(k: usize, scale: bool) -> &'static str {
    match k {
        2 if scale => "Scale steps",
        k => KNOBS.get(k).copied().unwrap_or(""),
    }
}
/// A Q-Link turn's join key, apart from every row's: a turn's detents on
/// any knob make one undoable take, its number in the key's top bits.
const TURN_KEY: u64 = 0x80;
/// A chord written on the pads' join key (plan 1C.2): its pads make one
/// undoable take, its number in the key's top bits.
const PAD_KEY: u64 = 0x40;
/// A hold and turn's join key (plan 1C.3): a hold's detents make one
/// undoable take, its number in the key's top bits.
const HELD_KEY: u64 = 0x20;
/// Join keys, so turning a row makes one undoable take.
const JOIN: u64 = 0xED17_0000_0000;

/// A finger resting on a note while the data wheel turns (plan 1C.3):
/// the [`EventKind::Values`] `index` [`NoteGrid`] sends, plus
/// [`HELD_FIRST`] on a hold's first detent and [`HELD_FINE`] with SHIFT;
/// its `set` is (the note, the detents), then (the hold's number, 0).
pub const HELD_TURN: usize = 1 << 27;
pub const HELD_FIRST: usize = 1;
pub const HELD_FINE: usize = 2;
/// The note a hold's first detent names when the hold picked it (and
/// taps may have added more): the picks as they are ([`EditAction::Held`]).
pub const HELD_PICKS: usize = usize::MAX;

/// How the editor shows the clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lens {
    /// The piano roll, with a lane under it.
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
#[derive(Clone, Debug, PartialEq)]
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
    /// longer than the clip's step comes out (`Clip::step`, a 1/16 unless
    /// changed, as on Steps) and a longer one is
    /// picked; with Draw off it is picked in place of the others.
    Tap { index: usize, rev: u64 },
    /// A note taken out: ERASE held + a tap.
    Erase { index: usize, rev: u64 },
    /// ERASE held + a drag across the roll (plan 1B.5): every note its
    /// path crossed comes out, by its index on the picture of revision
    /// `rev`. One take, however many notes.
    Sweep { notes: Vec<usize>, rev: u64 },
    /// The clip's end handle on the ruler, as its finger lifted (plan
    /// 1A.2): the clip made `to` ticks long, the nearest whole step, by
    /// the Steps' length edit at its step size ([`Grid::resize`], their
    /// D2), never shorter than just after its last note start or lock
    /// ([`notes::shortest_steps`]). One take.
    Length { to: Tick },
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
    /// it is picked alone and moves. With `scale` (the clip's scale on and
    /// no SHIFT as the finger went down, plan 2B.1) `notes` counts notes
    /// of the scale, each note moving by its own ([`notes::shift_in_scale`]),
    /// else semitones.
    Drop {
        index: usize,
        rev: u64,
        ticks: Tick,
        notes: i32,
        scale: bool,
    },
    /// A stroke along a lane under the roll, as its finger lifted (plan
    /// 2A.1): each note it changed, by its index on the picture of
    /// revision `rev`, with its new value of `field`. One edit and one
    /// take, however many notes; the picks stay as they were.
    Lane {
        field: Field,
        rev: u64,
        set: Vec<(usize, i32)>,
    },
    /// A stroke along the Pressure lane, as its finger lifted (plan
    /// 3A.1): the line it drew on the picture of revision `rev`, as
    /// (clip tick, value) points in time order, each held to the next and
    /// the last to `end`. One take: each picked note it crossed gets a
    /// new pressure run ([`notes::draw_touch`]); the picks stay.
    Pressure {
        rev: u64,
        points: Vec<(Tick, u8)>,
        end: Tick,
    },
    /// A stroke along a lock lane, as its finger lifted (plan 3A.2): the
    /// lock of `addr` where each note plays it, by its index on the
    /// picture of revision `rev`, with its new value
    /// ([`notes::lock_notes`]: notes in one step and row share a lock).
    /// One take; the picks stay.
    Lock {
        rev: u64,
        addr: RelAddr,
        set: Vec<(usize, f32)>,
    },
    /// ERASE + a tap on a lock lane (plan 3A.2): the locks of `addr`
    /// where these notes play it come off ([`notes::unlock_notes`]). One
    /// take; the picks stay.
    Unlock {
        rev: u64,
        addr: RelAddr,
        notes: Vec<usize>,
    },
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
    /// Q-Link `k` of the note inspector turned `ticks` detents while
    /// notes are picked (plan 1C.1, [`KNOBS`]): Q1 moves them a snap a
    /// detent (`fine`, SHIFT: 4 ticks), Q2 changes their length the same
    /// way, Q3 moves them a note of the clip's scale (plan 2B.1; a
    /// semitone with no scale, or `fine`), Q4 changes their velocity
    /// keeping the differences (`fine`: widens or narrows the gap between
    /// the soft and loud ones). Every detent of turn `turn` (the app counts them,
    /// from a knob touched to the last let go) joins one take.
    Knob {
        k: usize,
        ticks: i32,
        fine: bool,
        turn: u64,
    },
    /// The data wheel (or – / +) turned `ticks` detents while a finger
    /// rests on a note of the roll (plan 1C.3, hold and turn): the
    /// velocity, as Q4 turns it (`fine`, SHIFT: the gap between the soft
    /// and loud ones). The first detent of hold `turn` (`first`) names
    /// note `index` on the picture of revision `rev` (or [`HELD_PICKS`]:
    /// the picks as they are, a hold having picked it): it is picked,
    /// unless it is one of the picks already, and every detent of the
    /// hold turns the picks, one take. A later detent names no note, so
    /// a detent the screen hasn't caught up with still turns them.
    Held {
        index: usize,
        rev: u64,
        ticks: i32,
        fine: bool,
        turn: u64,
        first: bool,
    },
    /// – / + with the Edit options' drawer closed (plan 1C.1): the picked
    /// notes a snap earlier or later, as the MPC's Nudge; `fine` (SHIFT on
    /// the knob), 4 ticks.
    Nudge { steps: i32, fine: bool },
    /// The data wheel with the drawer closed (plan 1C.1): the cursor, the
    /// line where the pads will write, this many snaps on (back if
    /// negative), round the clip.
    Cursor(i32),
    /// The clip's scale picked on the Scale menu (plan 2B.1): None is Off.
    /// A clip setting, one take.
    Scale(Option<Scale>),
    /// The clip's scale's root picked on the Scale menu (0 is C).
    Root(u8),
    /// The Chord chip's menu (plan 2B.2): what a drawn note puts in, a
    /// note or a chord of `shape`, `minor` for a Triad or a Seventh with
    /// no scale. The editor's own, not the clip's: nothing changes.
    Chord { shape: ChordShape, minor: bool },
    /// The open tool's bar (plan 2B.3): Done, Cancel or its own button.
    Tool(ToolAct),
    /// The Pads write switch (plan 1C.2): on, the pads write at the
    /// cursor; off, they play (and pick a clip of pad hits' notes).
    PadsWrite,
    /// A pad on the MPC going down or let go while the editor is open
    /// (plan 1C.2): `key` is the pad as pressed (0 to 15), `hit` the kit
    /// pad it plays (what a clip of pad hits holds), `notes` the notes the
    /// instrument's pad map gives it as written (what a clip of notes
    /// holds), `velocity` as hit. With Pads write on, a pad going down
    /// writes at the cursor, a snap long ([`notes::write`]); pads held
    /// together are one chord, one take, and the cursor steps a snap when
    /// the last of them lets go. With it off, a pad on a clip of pad hits
    /// picks all its notes, and on a clip of notes nothing changes.
    Pad {
        key: u8,
        hit: u8,
        notes: Vec<u8>,
        velocity: u8,
        down: bool,
    },
    /// The Show menu (plan 4.1): another clip of the same kind, track
    /// `track`'s slot `slot`, shows faintly behind the one edited, or
    /// stops showing; at most [`SHOW_MAX`]. The editor's own setting, not
    /// the clip's: nothing changes.
    Show { track: usize, slot: usize },
    /// The Show menu's None: no other clip shows.
    ShowNone,
    /// A tap on another clip's faint note (plan 4.1): the editor opens
    /// that clip (and its track is selected, as the Sequencer follows),
    /// the clip it left showing faintly in its place.
    Switch { track: usize, slot: usize },
}

/// What the open tool's bar sends ([`EditAction::Tool`], plan 2B.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolAct {
    /// The preview goes in: one take, one UNDO, the tool closes.
    Done,
    /// The tool closes and the clip stays as it was.
    Cancel,
    /// The tool's own button: Chop's Reset pressure (on and off),
    /// Humanize's Again (another roll of the dice).
    Option,
}

/// The tools (piano roll plans 2B.2 and 2B.3), opened from their rows of
/// the Edit options on the picked notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Strum,
    Invert,
    Spread,
    Chop,
    Legato,
    Humanize,
}

impl ToolKind {
    /// In [`EDIT_ROWS`]' order, from [`STRUM_ROW`].
    pub const ALL: [ToolKind; 6] = [
        ToolKind::Strum,
        ToolKind::Invert,
        ToolKind::Spread,
        ToolKind::Chop,
        ToolKind::Legato,
        ToolKind::Humanize,
    ];

    /// The tool on Edit option `row`, if one is.
    pub fn of_row(row: usize) -> Option<ToolKind> {
        ToolKind::ALL.get(row.checked_sub(STRUM_ROW)?).copied()
    }

    pub fn row(self) -> usize {
        STRUM_ROW + ToolKind::ALL.iter().position(|&k| k == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        EDIT_ROWS[self.row()]
    }

    /// The least notes sounding together the picks need (a chord), or 1.
    fn least(self) -> usize {
        match self {
            ToolKind::Strum | ToolKind::Invert => 2,
            ToolKind::Spread => 3,
            _ => 1,
        }
    }
}

/// The most times Invert and Spread go either way.
const MAX_TURNS: i32 = 8;

/// A tool open on the picked notes (piano roll plan 2B.3): its settings,
/// which Q1 to Q4 turn ([`NoteTool::cards`]). Its edit
/// ([`NoteTool::edit`]) is worked out afresh from the clip and the picks
/// each time, so the roll's preview ([`Editor::preview`]) is the edit Done
/// makes, and nothing goes into the clip until then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteTool {
    pub kind: ToolKind,
    /// Q1 to Q4: Strum's ticks further apart and its way (0 as each chord
    /// is, 1 up, 2 down); Invert's and Spread's times (below 0 down, or
    /// closer); Chop's pieces; Legato's share of the gap (%); Humanize's
    /// timing (ticks), velocity and length (%), and ahead or behind (-100
    /// to 100).
    pub q: [i32; 4],
    /// Humanize's dice: Again moves it on.
    pub seed: u64,
    /// Chop's Reset pressure: the pieces carry none.
    pub reset: bool,
}

impl NoteTool {
    /// `kind` as it opens: Strum, Invert and Spread at no change, Chop in
    /// two, Legato all the way, Humanize a little.
    pub fn new(kind: ToolKind) -> NoteTool {
        let q = match kind {
            ToolKind::Chop => [2, 0, 0, 0],
            ToolKind::Legato => [100, 0, 0, 0],
            ToolKind::Humanize => [20, 10, 10, 0],
            _ => [0; 4],
        };
        NoteTool {
            kind,
            q,
            seed: 1,
            reset: false,
        }
    }

    /// Q-Link `k` turned `ticks` detents; `fine` (SHIFT) by the least
    /// step. False when it had no setting to change.
    pub fn turn(&mut self, k: usize, ticks: i32, fine: bool) -> bool {
        let Some(&(step, fine_step, lo, hi)) = self.ranges().get(k) else {
            return false;
        };
        if step == 0 {
            return false;
        }
        let by = if fine { fine_step } else { step };
        let q = &mut self.q[k];
        let was = *q;
        *q = (*q + ticks * by).clamp(lo, hi);
        *q != was
    }

    /// Each card's step, SHIFT's step, least and most (a 0 step: none).
    fn ranges(&self) -> &'static [(i32, i32, i32, i32)] {
        const STRUM: i32 = STRUM_TICKS as i32;
        const BEAT: i32 = PPQ as i32;
        match self.kind {
            ToolKind::Strum => &[(STRUM, 1, -BEAT / 2, BEAT / 2), (1, 1, 0, 2)],
            ToolKind::Invert | ToolKind::Spread => &[(1, 1, -MAX_TURNS, MAX_TURNS)],
            ToolKind::Chop => &[(1, 1, 2, notes::MAX_PIECES as i32)],
            ToolKind::Legato => &[(5, 1, 0, 100)],
            ToolKind::Humanize => &[
                (5, 1, 0, BEAT / 4),
                (5, 1, 0, 100),
                (5, 1, 0, 100),
                (10, 1, -100, 100),
            ],
        }
    }

    /// The edit the tool makes on the `sel` notes of `clip` (empty at no
    /// change, or when refused).
    pub fn edit(&self, clip: &Clip, sel: &[usize]) -> Edit {
        let q = self.q;
        match self.kind {
            ToolKind::Strum => {
                let down = match q[1] {
                    1 => Some(false),
                    2 => Some(true),
                    _ => None,
                };
                notes::strum(clip, sel, Tick::from(q[0]), down)
            }
            ToolKind::Invert => notes::invert(clip, sel, q[0]),
            ToolKind::Spread => notes::spread(clip, sel, q[0]),
            ToolKind::Chop => notes::chop(clip, sel, q[0].max(0) as u32, self.reset),
            ToolKind::Legato => notes::legato(clip, sel, q[0].clamp(0, 100) as u8),
            ToolKind::Humanize => notes::humanize(
                clip,
                sel,
                notes::Humanize {
                    time: Tick::from(q[0]),
                    velocity: q[1].clamp(0, 100) as u8,
                    length: q[2].clamp(0, 100) as u8,
                    eagerness: q[3].clamp(-100, 100) as i8,
                },
                self.seed,
            ),
        }
    }

    /// Why the tool changes nothing on the `sel` notes at these settings.
    pub fn why_not(&self, clip: &Clip, sel: &[usize]) -> &'static str {
        let q = self.q;
        let chords = notes::chords_of(clip, sel);
        let chord = |least: usize| chords.iter().any(|c| c.len() >= least);
        match self.kind {
            _ if sel.is_empty() => "no notes picked",
            k if !chord(k.least()) && k.least() == 3 => "no chord of three notes or more picked",
            k if !chord(k.least()) => "no chord picked: notes that sound together",
            ToolKind::Strum if q[0] == 0 && q[1] == 0 => "nothing changes yet",
            ToolKind::Strum if q[0] > 0 => "a note would be too short",
            ToolKind::Strum => "nothing changes",
            ToolKind::Invert | ToolKind::Spread if q[0] == 0 => "nothing changes yet",
            ToolKind::Spread if q[0] < 0 => "the chords are as close as they go",
            ToolKind::Invert | ToolKind::Spread => {
                "a note would go past the lowest or highest note"
            }
            ToolKind::Chop => "a note is too short, or the clip too full",
            ToolKind::Legato if q[0] == 0 => "nothing changes yet",
            ToolKind::Legato => "no picked note starts after another",
            ToolKind::Humanize => "nothing changes",
        }
    }

    /// Q1 to Q4's names and values, "" for a blank card.
    pub fn cards(&self) -> [(&'static str, String); 4] {
        let q = self.q;
        let signed = |v: i32, up: &str, down: &str| match v {
            0 => "None".to_string(),
            v if v > 0 => format!("{up} {v}"),
            v => format!("{down} {}", -v),
        };
        let blank = || ("", String::new());
        match self.kind {
            ToolKind::Strum => [
                ("Apart", format!("{:+}t", q[0]).replace("+0t", "0t")),
                (
                    "Way",
                    ["As it is", "Up", "Down"][q[1].clamp(0, 2) as usize].to_string(),
                ),
                blank(),
                blank(),
            ],
            ToolKind::Invert => [
                ("Inversions", signed(q[0], "Up", "Down")),
                blank(),
                blank(),
                blank(),
            ],
            ToolKind::Spread => [
                ("Spread", signed(q[0], "Open", "Close")),
                blank(),
                blank(),
                blank(),
            ],
            ToolKind::Chop => [("Pieces", q[0].to_string()), blank(), blank(), blank()],
            ToolKind::Legato => [("Fill", format!("{}%", q[0])), blank(), blank(), blank()],
            ToolKind::Humanize => [
                ("Timing", format!("±{}t", q[0])),
                ("Velocity", format!("±{}%", q[1])),
                ("Length", format!("±{}%", q[2])),
                (
                    "Ahead/behind",
                    signed(q[3], "Behind", "Ahead").replace("None", "Even"),
                ),
            ],
        }
    }

    /// Where card `k`'s setting sits in its range, 0 to 1.
    pub fn pos(&self, k: usize) -> f32 {
        match self.ranges().get(k) {
            Some(&(step, _, lo, hi)) if step != 0 && hi > lo => {
                (self.q[k] - lo) as f32 / (hi - lo) as f32
            }
            _ => 0.0,
        }
    }

    /// Card `k` turns both ways from 0 (Strum's Apart, Invert, Spread,
    /// ahead or behind).
    pub fn bipolar(&self, k: usize) -> bool {
        self.ranges().get(k).is_some_and(|r| r.0 != 0 && r.2 < 0)
    }

    /// The tool's own button's words and whether it is lit: Chop's Reset
    /// pressure, Humanize's Again; None for the others.
    pub fn option(&self) -> Option<(&'static str, bool)> {
        match self.kind {
            ToolKind::Chop => Some(("Reset pressure", self.reset)),
            ToolKind::Humanize => Some(("Again", false)),
            _ => None,
        }
    }
}

impl EditAction {
    /// The revision of the picture an action naming a note was made on
    /// (the app refuses it if the clip has moved on).
    pub fn rev(&self) -> Option<u64> {
        match *self {
            EditAction::Note { rev, .. }
            | EditAction::Tap { rev, .. }
            | EditAction::Erase { rev, .. }
            | EditAction::Sweep { rev, .. }
            | EditAction::Add { rev, .. }
            | EditAction::Resize { rev, .. }
            | EditAction::Copy { rev, .. }
            | EditAction::Lane { rev, .. }
            | EditAction::Pressure { rev, .. }
            | EditAction::Lock { rev, .. }
            | EditAction::Unlock { rev, .. }
            | EditAction::Drop { rev, .. }
            | EditAction::Held {
                rev, first: true, ..
            } => Some(rev),
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
    /// The clip's scale changed to this (None: Off), one undoable take
    /// joined by `join` (piano roll plan 2B.1): a clip setting, as its
    /// step size is, for the app to set; no note changes with it.
    pub scale: Option<Option<Scale>>,
    /// After the edit, sound the notes it put in (plan 1C.2: notes drawn
    /// or moved).
    hear: bool,
}

impl Done {
    fn line(line: String) -> Done {
        Done {
            line,
            ..Done::default()
        }
    }
}

/// The scales the Scale option offers (piano roll plan 2B.1), from the
/// app: Off, the scale the selected track's pads play in on the Synth tab
/// when they have one, the built-in scales but Chromatic (every row lit is
/// no scale), then the owner's own. No project key is forced on a clip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScaleOffers {
    /// The pads' scale on the Synth tab, root and all.
    pub pads: Option<Scale>,
    /// The owner's own scales: a name, the notes going up and coming down.
    pub own: Vec<(String, u16, u16)>,
}

/// One of the Scale option's choices ([`ScaleOffers::choice`]).
#[derive(Clone, Copy, Debug, PartialEq)]
enum ScaleChoice<'a> {
    Off,
    /// The pads' scale, root and all.
    Pads(Scale),
    /// A scale's name and notes (up, down), on the clip's root.
    Notes(&'a str, u16, u16),
}

impl ScaleOffers {
    /// How many choices the Scale menu has.
    pub fn len(&self) -> usize {
        1 + usize::from(self.pads.is_some()) + SCALES.len() - 1 + self.own.len()
    }

    /// Never: Off is always a choice.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Choice `i`: Off, the pads' scale, a built-in scale, one of the
    /// owner's.
    fn choice(&self, i: usize) -> Option<ScaleChoice<'_>> {
        let pads = usize::from(self.pads.is_some());
        match i {
            0 => Some(ScaleChoice::Off),
            1 if pads == 1 => self.pads.map(ScaleChoice::Pads),
            i if i < pads + SCALES.len() => {
                let d = &SCALES[i - pads];
                Some(ScaleChoice::Notes(d.name, d.mask, d.down))
            }
            i => self
                .own
                .get(i - pads - SCALES.len())
                .map(|(n, u, d)| ScaleChoice::Notes(n, *u, *d)),
        }
    }

    /// Choice `i` as the menu says it: "Off", "Pads: D Dorian", a scale's
    /// name.
    pub fn choice_name(&self, i: usize) -> Option<String> {
        Some(match self.choice(i)? {
            ScaleChoice::Off => "Off".to_string(),
            ScaleChoice::Pads(k) => format!("Pads: {}", self.label(Some(k))),
            ScaleChoice::Notes(name, ..) => name.to_string(),
        })
    }

    /// The scale choice `i` gives a clip whose scale is `now` (None for
    /// no such choice): Off, the pads' scale, or a scale's notes on the
    /// clip's root (the pads' root when the clip had none, else C).
    pub fn scale_of(&self, i: usize, now: Option<Scale>) -> Option<Option<Scale>> {
        let root = now.or(self.pads).map_or(0, |k| k.root);
        Some(match self.choice(i)? {
            ScaleChoice::Off => None,
            ScaleChoice::Pads(k) => Some(k),
            ScaleChoice::Notes(_, up, down) => Some(Scale::with_down(root, up, down)),
        })
    }

    /// The choice that gives scale `k`, for the menu to light: the pads'
    /// when it is theirs, root and all, else the first with its notes;
    /// none for a scale from a file that none of them names.
    pub fn index_of(&self, k: Option<Scale>) -> Option<usize> {
        (0..self.len()).find(|&i| match (self.choice(i), k) {
            (Some(ScaleChoice::Off), None) => true,
            (Some(ScaleChoice::Pads(p)), Some(k)) => p == k,
            (Some(ScaleChoice::Notes(_, up, down)), Some(k)) => (k.mask, k.down) == (up, down),
            _ => false,
        })
    }

    /// Scale `k`'s name without its root: "Dorian", the owner's name, or
    /// "Own scale" for one none of the choices names.
    pub fn name_of(&self, k: Scale) -> &str {
        let built = SCALES[1..]
            .iter()
            .find(|d| (d.mask, d.down) == (k.mask, k.down));
        if let Some(d) = built {
            return d.name;
        }
        self.own
            .iter()
            .find(|(_, u, d)| (*u, *d) == (k.mask, k.down))
            .map_or("Own scale", |(n, ..)| n.as_str())
    }

    /// The clip's scale as the Scale chip says it: "D Dorian", or "Off".
    pub fn label(&self, k: Option<Scale>) -> String {
        match k {
            None => "Off".to_string(),
            Some(k) => format!(
                "{} {}",
                NOTE_NAMES[usize::from(k.root % 12)],
                self.name_of(k)
            ),
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
    /// The cursor's tick (plan 1C.1): the line the wheel moves, where the
    /// pads will write.
    pub cursor: Tick,
    /// The scales the Scale menu offers (the app's: the pads' scale and
    /// the owner's own; plan 2B.1).
    pub offers: Arc<ScaleOffers>,
    /// What a note drawn on the roll puts in (plan 2B.2): a note, or a
    /// chord on it; `minor` for a Triad or a Seventh with no scale.
    pub chord: ChordShape,
    pub minor: bool,
    /// The tool open on the picks (plan 2B.3): the roll previews its edit
    /// and the Q-Links turn its settings until Done or Cancel.
    pub open_tool: Option<NoteTool>,
    /// Pads write (plan 1C.2): the pads write at the cursor. Off whenever
    /// the editor opens, even on an empty clip.
    pub pads_write: bool,
    /// The pads held down since one wrote (a bit each), the chord they
    /// write (its number, for its join key) and whether it wrote a note:
    /// the cursor steps once, when the last of them lets go.
    pads_held: u16,
    pad_chord: u64,
    pad_wrote: bool,
    /// The notes to sound (plan 1C.2): the note picked, or the notes
    /// drawn or moved, by the last touch, numbered so each is heard once.
    pub heard: Heard,
    /// Other clips shown faintly behind this one (plan 4.1), as (track,
    /// slot), in the order picked: at most [`SHOW_MAX`].
    pub show: Vec<(usize, usize)>,
    /// The hold whose first detent picked its note (plan 1C.3): its
    /// later detents turn the picks; another hold's, nothing.
    held_turn: Option<u64>,
}

/// The most other clips the roll shows behind the one edited (plan 4.1).
pub const SHOW_MAX: usize = 3;

/// Another clip shown faintly behind the roll's notes (plan 4.1): its
/// notes placed on the edited clip's ticks where they sound against it
/// ([`notes::placed`]), in its track's colour, labelled by name and by
/// whether it plays with this clip or is there to compare.
#[derive(Clone, Debug, PartialEq)]
pub struct GhostLayer {
    pub track: usize,
    pub slot: usize,
    /// "Bass · Pattern 2": its track and its own name.
    pub name: String,
    /// Plays with (a clip of the same scene, or one playing now on
    /// another track), else Reference.
    pub with: bool,
    pub color: Color,
    /// How far into its loop it is, in song ticks, where the edited
    /// clip's pass starts: it moves on with the passes.
    pub phase: f64,
    pub notes: Vec<Note>,
}

impl GhostLayer {
    /// "Plays with" or "Reference".
    pub fn kind_name(&self) -> &'static str {
        kind_name(self.with)
    }
}

/// A clip the Show menu offers (plan 4.1): where it is, "Bass · Pattern
/// 2", Plays with or Reference, and whether it shows now.
#[derive(Clone, Debug, PartialEq)]
pub struct ShowOffer {
    pub track: usize,
    pub slot: usize,
    pub name: String,
    pub with: bool,
    pub shown: bool,
}

fn kind_name(with: bool) -> &'static str {
    match with {
        true => "Plays with",
        false => "Reference",
    }
}

/// Notes the editor asks to hear ([`Editor::heard`], plan 1C.2): the
/// screen plays them through the track's instrument when `n` moves on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heard {
    pub n: u64,
    pub notes: Vec<Note>,
}

/// The most notes one touch sounds (a chord): the earliest of them.
pub const HEARD_MAX: usize = 8;

/// The Chord chip's shapes (piano roll plan 2B.2): Off draws one note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChordShape {
    #[default]
    Off,
    Triad,
    Seventh,
    Sus2,
    Sus4,
    Power,
    Octave,
    /// The triad the Chord pads play: in the pads' scale on the Synth tab.
    Pads,
}

impl ChordShape {
    /// The Chord menu's shapes, in its order.
    pub const ALL: [ChordShape; 8] = [
        ChordShape::Off,
        ChordShape::Triad,
        ChordShape::Seventh,
        ChordShape::Sus2,
        ChordShape::Sus4,
        ChordShape::Power,
        ChordShape::Octave,
        ChordShape::Pads,
    ];

    /// Its name on the Chord menu.
    pub fn name(self) -> &'static str {
        match self {
            ChordShape::Off => "Off",
            ChordShape::Triad => "Triad",
            ChordShape::Seventh => "Seventh",
            ChordShape::Sus2 => "Sus2",
            ChordShape::Sus4 => "Sus4",
            ChordShape::Power => "Power",
            ChordShape::Octave => "Octave",
            ChordShape::Pads => "Pads",
        }
    }

    /// The Chord chip's words: "Triad", or with no scale "Minor triad",
    /// "Major seventh".
    pub fn label(self, minor: bool, scale: bool) -> String {
        match self {
            ChordShape::Triad | ChordShape::Seventh if !scale => {
                let q = if minor { "Minor" } else { "Major" };
                format!("{q} {}", self.name().to_lowercase())
            }
            s => s.name().to_string(),
        }
    }
}

/// The notes a note drawn on `root` puts in (plan 2B.2): `root` alone with
/// the chord Off, else the chord of `shape` on it, lowest first: Triad and
/// Seventh in `scale` (the clip's) or with none major or `minor`, Pads the
/// triad in `pads` (the pads' scale, the root brought into it; with none,
/// as Triad). Notes past 127 are left out.
pub fn chord_notes(
    shape: ChordShape,
    minor: bool,
    root: u8,
    scale: Option<Scale>,
    pads: Option<Scale>,
) -> Vec<u8> {
    let root16 = i16::from(root);
    let made = |shape: Shape, scale: Option<Scale>, root: i16| {
        chords::chord(root, shape, scale, minor).notes().to_vec()
    };
    match shape {
        ChordShape::Off => vec![root],
        ChordShape::Triad => made(Shape::Triad, scale, root16),
        ChordShape::Seventh => made(Shape::Seventh, scale, root16),
        ChordShape::Sus2 => made(Shape::Sus2, scale, root16),
        ChordShape::Sus4 => made(Shape::Sus4, scale, root16),
        ChordShape::Power => made(Shape::Power, scale, root16),
        ChordShape::Octave => made(Shape::Octave, scale, root16),
        ChordShape::Pads => match pads {
            Some(k) => {
                let root = k
                    .snapped(root16, mpfree_seq::mpfree_scale::Snap::Nearest)
                    .unwrap_or(root16);
                made(Shape::Triad, Some(k), root)
            }
            None => made(Shape::Triad, scale, root16),
        },
    }
}

/// The name of the chord the notes `notes` make ("Dm7", "C/E"), if any.
pub fn chord_name(notes: &[u8]) -> Option<String> {
    ChordName::of(notes).map(|c| c.to_string())
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
            cursor: 0,
            offers: Arc::default(),
            chord: ChordShape::Off,
            minor: false,
            open_tool: None,
            pads_write: false,
            pads_held: 0,
            pad_chord: 0,
            pad_wrote: false,
            heard: Heard::default(),
            show: Vec::new(),
            held_turn: None,
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

    /// How a knob or – / + moves notes (plan 1C.1): onto the snap's lines,
    /// or with Snap Auto by whole snaps, each note keeping its distance
    /// from the grid (as an Auto drag); 4 ticks a step with no snap or
    /// `fine`.
    fn move_snap(&self, fine: bool) -> Snap {
        match (self.snap_ticks(), fine) {
            (0, _) | (_, true) => Snap::Off,
            (d, false) if self.snap_auto() => Snap::Relative(d),
            (d, false) => Snap::Absolute(d),
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
        let note = note.min(127);
        self.sound = note;
        let shape = if pads(clip) {
            ChordShape::Off
        } else {
            self.chord
        };
        let chord = chord_notes(shape, self.minor, note, lit_scale(clip), self.offers.pads);
        let line = match (chord.len(), chord_name(&chord)) {
            (1, _) => format!("edit: {} drawn at tick {tick}", one_named(clip, note)),
            (_, Some(name)) => format!("edit: chord {name} drawn at tick {tick}"),
            _ => format!("edit: chord on {} drawn at tick {tick}", note_name(note)),
        };
        let added: Vec<Note> = chord
            .iter()
            .map(|&k| Note::new(tick, len, k, self.velocity))
            .collect();
        Some((notes::add_all(clip, &added), None, true, false, line))
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
            // Off the scale (plan 2B.1): the notes the roll marks.
            13 => match lit_scale(clip) {
                Some(k) => which(&|n| !k.contains(i16::from(n.note))),
                None => {
                    return Done::line("edit: the clip has no scale: pick one with Scale".into())
                }
            },
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
        // A pad let go (plan 1C.2) ends its chord whatever is open: the
        // cursor steps when the last pad that wrote lets go.
        if let EditAction::Pad {
            key: k,
            down: false,
            ..
        } = a
        {
            self.pad_up(k, clip);
            return Done::default();
        }
        // A tool open (plan 2B.3) takes the Q-Links and its bar; settings
        // and the view still change, another tool's row opens that one in
        // its place, and every other edit waits for Done or Cancel.
        if let Some(tool) = self.open_tool {
            let keeps = matches!(
                a,
                EditAction::Open
                    | EditAction::AutoSnap(_)
                    | EditAction::Scroll(_)
                    | EditAction::Cursor(_)
                    | EditAction::PickRow(_)
                    | EditAction::Chord { .. }
                    | EditAction::Tool(_)
                    | EditAction::Knob { .. }
                    | EditAction::PadsWrite
                    | EditAction::Show { .. }
                    | EditAction::ShowNone
                    | EditAction::Row { row: 0..=5, .. }
            ) || matches!(a, EditAction::Row { row, .. } if ToolKind::of_row(row).is_some());
            // A pad that only plays leaves the tool's picks be.
            if matches!(a, EditAction::Pad { .. }) && !self.pads_write {
                return Done::default();
            }
            if !keeps {
                return Done::line(format!("edit: Done or Cancel {} first", tool.kind.name()));
            }
        }
        // Notes drawn or moved are heard once they are in (plan 1C.2).
        let hears = matches!(
            a,
            EditAction::Empty { .. }
                | EditAction::Draw { .. }
                | EditAction::Step(_)
                | EditAction::Drop { .. }
                | EditAction::Knob { k: 0 | 2, .. }
                | EditAction::Nudge { .. }
                | EditAction::Row { row: 6 | 7, .. }
        );
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
                self.hear([clip.notes[i]]);
                return Done::default();
            }
            // A tap with Draw on takes out a note no longer than a step,
            // only the one tapped (never every note of its step and
            // pitch); a longer one, or any with Draw off, is picked alone.
            EditAction::Tap { index: i, .. } if i < clip.notes.len() => {
                let n = clip.notes[i];
                if self.tool == Tool::Draw && n.len <= clip.step.max(1) {
                    self.sound = n.note;
                    (
                        notes::delete(clip, &[i]),
                        None,
                        false,
                        true,
                        format!("edit: {} taken out", one_named(clip, n.note)),
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
                            self.hear([n]);
                        }
                    }
                    return Done::line(format!("edit: {} picked", self.sel.len()));
                } else {
                    self.sel = vec![i];
                    self.worked_on(n);
                    self.hear([n]);
                    return Done::default();
                }
            }
            EditAction::Erase { index: i, .. } if i < clip.notes.len() => (
                notes::delete(clip, &[i]),
                None,
                false,
                true,
                format!("edit: {} erased", one_named(clip, clip.notes[i].note)),
            ),
            // ERASE + a drag: the notes it crossed, each once.
            EditAction::Sweep {
                notes: ref swept, ..
            } => {
                let mut out: Vec<usize> = swept
                    .iter()
                    .copied()
                    .filter(|&i| i < clip.notes.len())
                    .collect();
                out.sort_unstable();
                out.dedup();
                let line = match out[..] {
                    [] => return Done::default(),
                    [i] => format!("edit: {} erased", one_named(clip, clip.notes[i].note)),
                    _ => format!("edit: {} notes erased", out.len()),
                };
                (notes::delete(clip, &out), None, false, true, line)
            }
            // The clip's end handle (plan 1A.2): the Steps' length edit at
            // the clip's step size, which takes out no note start or lock
            // this far on.
            EditAction::Length { to } => {
                let grid = notes::steps_grid(clip);
                let size = grid.size();
                let steps = usize::try_from((to + size / 2).div_euclid(size)).unwrap_or(0);
                let fewest = notes::shortest_steps(clip, grid);
                if steps < fewest {
                    return Done::line(format!(
                        "edit: a note or lock starts on step {fewest}: the clip can't end before it"
                    ));
                }
                if steps > grid.max_steps() {
                    return Done::line(format!(
                        "edit: {} steps is the longest a clip can be",
                        grid.max_steps()
                    ));
                }
                let e = grid.resize(clip, steps, false);
                if e.is_empty() {
                    return Done::default();
                }
                let st = if steps == 1 { "step" } else { "steps" };
                (
                    e,
                    None,
                    false,
                    true,
                    format!("edit: the clip is {steps} {st} long"),
                )
            }
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
                        self.hear([clip.notes[i]]);
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
                scale,
                ..
            } => {
                let moved = match index < clip.notes.len() && !sel.contains(&index) {
                    true => vec![index],
                    false => sel.clone(),
                };
                if let Some(&i) = moved.first() {
                    self.sound = clip.notes[i].note;
                }
                let edit = match lit_scale(clip).filter(|_| scale) {
                    Some(k) => notes::shift_in_scale(clip, &moved, ticks, by, k),
                    None => notes::shift(clip, &moved, ticks, by),
                };
                (
                    edit,
                    None,
                    true,
                    false,
                    format!("edit: {} notes moved", moved.len()),
                )
            }
            // A stroke along a lane: one take, every note it crossed
            // its own value. The picks stay on their notes, changed or
            // not, and no other note is picked.
            EditAction::Lane { field, set, .. } => {
                let edit = notes::set_values(clip, field, &set);
                if field == Field::Velocity {
                    if let Some(&(_, v)) = set.last() {
                        self.velocity = v.clamp(1, 127) as u8;
                    }
                }
                let kept = kept
                    .iter()
                    .map(|n| match edit.erase.iter().position(|e| e == n) {
                        Some(j) => edit.add[j],
                        None => *n,
                    })
                    .collect();
                let n = edit.erase.len();
                let line = match n {
                    0 => String::new(),
                    1 => format!("edit: 1 note's {}", field.name().to_lowercase()),
                    _ => format!("edit: {n} notes' {}", field.name().to_lowercase()),
                };
                return Done {
                    edit,
                    line,
                    kept,
                    ..Done::default()
                };
            }
            // A stroke along the Pressure lane: one take, a new run on
            // each picked note it crossed (plan 3A.1). The picks stay.
            EditAction::Pressure { points, end, .. } => {
                let Some(edit) = notes::draw_touch(clip, &sel, &points, end) else {
                    return Done {
                        line: "edit: the clip's pressure is full, so the line was not drawn".into(),
                        ..Done::default()
                    };
                };
                let kept = kept
                    .iter()
                    .map(|n| match edit.erase.iter().position(|e| e == n) {
                        Some(j) => edit.add[j],
                        None => *n,
                    })
                    .collect();
                let line = match edit.erase.len() {
                    0 => String::new(),
                    1 => "edit: pressure drawn on 1 note".into(),
                    n => format!("edit: pressure drawn on {n} notes"),
                };
                return Done {
                    edit,
                    line,
                    kept,
                    ..Done::default()
                };
            }
            // A stroke along a lock lane, or ERASE + a tap on it: one
            // take of locks (plan 3A.2). No note changes; the picks stay.
            EditAction::Lock { addr, set, .. } => {
                let edit = notes::lock_notes(clip, addr, &set);
                let line = match edit.add_locks.len() {
                    0 => String::new(),
                    1 => "edit: 1 lock set".into(),
                    n => format!("edit: {n} locks set"),
                };
                return Done {
                    edit,
                    line,
                    kept,
                    ..Done::default()
                };
            }
            EditAction::Unlock {
                addr, notes: of, ..
            } => {
                let edit = notes::unlock_notes(clip, addr, &of);
                let line = match edit.is_empty() {
                    true => "edit: no lock there to take off".into(),
                    false => "edit: lock taken off".into(),
                };
                return Done {
                    edit,
                    line,
                    kept,
                    ..Done::default()
                };
            }
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
            // The open tool's settings: the preview changes, the clip
            // doesn't.
            EditAction::Knob { k, ticks, fine, .. } if self.open_tool.is_some() => {
                if let Some(t) = self.open_tool.as_mut() {
                    t.turn(k, ticks, fine);
                }
                return Done::default();
            }
            EditAction::Tool(act) => {
                let Some(mut tool) = self.open_tool else {
                    return Done::default();
                };
                let name = tool.kind.name();
                match act {
                    ToolAct::Cancel => {
                        self.open_tool = None;
                        return Done::line(format!(
                            "edit: {name} cancelled: the clip is as it was"
                        ));
                    }
                    ToolAct::Option => {
                        match tool.kind {
                            ToolKind::Chop => tool.reset = !tool.reset,
                            ToolKind::Humanize => tool.seed = tool.seed.wrapping_add(1),
                            _ => return Done::default(),
                        }
                        self.open_tool = Some(tool);
                        return Done::default();
                    }
                    ToolAct::Done => {
                        self.open_tool = None;
                        let e = tool.edit(clip, &sel);
                        if e.is_empty() {
                            return Done::line(format!(
                                "edit: {name}: {}, so nothing changed",
                                tool.why_not(clip, &sel)
                            ));
                        }
                        let least = tool.kind.least();
                        let chords = notes::chords_of(clip, &sel)
                            .iter()
                            .filter(|c| c.len() >= least)
                            .count();
                        let (n, q) = (sel.len(), tool.q[0]);
                        let them = |n: usize, one: &str, many: &str| {
                            format!("{n} {}", if n == 1 { one } else { many })
                        };
                        let line = match tool.kind {
                            ToolKind::Strum => {
                                format!("{} strummed", them(chords, "chord", "chords"))
                            }
                            ToolKind::Invert => {
                                format!("{} inverted", them(chords, "chord", "chords"))
                            }
                            ToolKind::Spread if q < 0 => {
                                format!("{} closed", them(chords, "chord", "chords"))
                            }
                            ToolKind::Spread => {
                                format!("{} spread", them(chords, "chord", "chords"))
                            }
                            ToolKind::Chop => {
                                format!("{} chopped in {q}", them(n, "note", "notes"))
                            }
                            ToolKind::Legato => {
                                format!("{} held on, {q}% of the gap", them(n, "note", "notes"))
                            }
                            ToolKind::Humanize => format!("{} humanized", them(n, "note", "notes")),
                        };
                        // One take of its own, the notes it made picked.
                        (e, None, true, true, format!("edit: {line}"))
                    }
                }
            }
            // Hold and turn (plan 1C.3): the first detent picks the note
            // held, with the picks when it is one, and every detent of
            // the hold turns them, one take.
            EditAction::Held {
                index,
                ticks,
                fine,
                turn,
                first,
                ..
            } => {
                if first {
                    if index < clip.notes.len() && !sel.contains(&index) {
                        self.sel = self.picks_with(sel, vec![index]);
                    }
                    self.held_turn = Some(turn);
                }
                let sel = self.picked(clip);
                if self.held_turn != Some(turn) || sel.is_empty() {
                    return Done::default();
                }
                let edit = match fine {
                    true => notes::velocity_spread(clip, &sel, ticks),
                    false => notes::velocity_kept(clip, &sel, ticks),
                };
                // The picks now, the note held among them, stay picked
                // (not the ones before it, picked alone).
                return Done {
                    edit,
                    join: Some(base | HELD_KEY | (turn & 0xFFFF) << 48),
                    select_added: true,
                    kept: sel.iter().map(|&i| clip.notes[i]).collect(),
                    ..Done::default()
                };
            }
            EditAction::Knob { .. } | EditAction::Nudge { .. } if none => {
                return Done::line("edit: pick notes first".into())
            }
            // The note inspector: one take a turn, whichever knobs turn.
            EditAction::Knob {
                k,
                ticks,
                fine,
                turn,
            } => {
                let d = i64::from(ticks);
                let e = match k {
                    0 => notes::nudge(clip, &sel, d, self.move_snap(fine)),
                    1 => {
                        let by = if fine { notes::FREE_NUDGE } else { self.step() };
                        notes::edit_end(clip, &sel, d * by)
                    }
                    // Past note 0 or 127 the whole move is refused, so a
                    // chord keeps its shape; with the clip's scale on, by
                    // its notes (SHIFT: a semitone).
                    2 => match lit_scale(clip).filter(|_| !fine) {
                        Some(k) => notes::shift_in_scale(clip, &sel, 0, ticks, k),
                        None => notes::shift(clip, &sel, 0, ticks),
                    },
                    3 if fine => notes::velocity_spread(clip, &sel, ticks),
                    3 => notes::velocity_kept(clip, &sel, ticks),
                    _ => return Done::default(),
                };
                (
                    e,
                    Some(base | TURN_KEY | (turn & 0xFFFF) << 48),
                    true,
                    true,
                    String::new(),
                )
            }
            // – / +: as the Nudge row, so a run of presses is one take.
            EditAction::Nudge { steps, fine } => (
                notes::nudge(clip, &sel, i64::from(steps), self.move_snap(fine)),
                key(6),
                true,
                true,
                String::new(),
            ),
            EditAction::Cursor(d) => {
                self.move_cursor(clip, d);
                return Done::default();
            }
            // Other clips behind this one (plan 4.1): the editor's own.
            EditAction::Show { track, slot } => {
                if let Some(k) = self.show.iter().position(|&s| s == (track, slot)) {
                    self.show.remove(k);
                } else if (track, slot) == (self.track, self.slot) {
                    return Done::default();
                } else if self.show.len() >= SHOW_MAX {
                    return Done::line(format!(
                        "edit: {SHOW_MAX} clips show already: tap one to take it off first"
                    ));
                } else {
                    self.show.push((track, slot));
                }
                return Done::line(match self.show.len() {
                    0 => "edit: no other clip shows".into(),
                    1 => "edit: 1 other clip shows faintly behind this one".into(),
                    n => format!("edit: {n} other clips show faintly behind this one"),
                });
            }
            EditAction::ShowNone => {
                self.show.clear();
                return Done::line("edit: no other clip shows".into());
            }
            // The time engine opens the other clip (TimeControl).
            EditAction::Switch { .. } => return Done::default(),
            // The pads (plan 1C.2): Pads write switched, a pad picking a
            // clip of pad hits' notes, or writing at the cursor.
            EditAction::PadsWrite => {
                self.pads_write = !self.pads_write;
                self.pads_held = 0;
                self.pad_wrote = false;
                return Done::line(match self.pads_write {
                    true => "edit: Pads write on: the pads write at the cursor".into(),
                    false => "edit: Pads write off: the pads play".into(),
                });
            }
            EditAction::Pad { hit, .. } if !self.pads_write => {
                if clip.holds != Holds::Hits {
                    return Done::default();
                }
                let name = sound_name(hit, true);
                self.sound = hit;
                let mine: Vec<usize> = (0..clip.notes.len())
                    .filter(|&i| clip.notes[i].note == hit)
                    .collect();
                if mine.is_empty() {
                    return Done::line(format!("edit: {name} has no notes in this clip"));
                }
                let n = mine.len();
                self.sel = self.picks_with(sel, mine);
                return Done::line(format!(
                    "edit: {name}'s {n} note{} picked",
                    if n == 1 { "" } else { "s" }
                ));
            }
            EditAction::Pad {
                key: k,
                hit,
                notes: from_map,
                velocity,
                ..
            } => match self.pad_write(clip, k, hit, from_map, velocity) {
                Ok((edit, chord, first)) => (
                    edit,
                    Some(base | PAD_KEY | (chord & 0xFFFF) << 48),
                    true,
                    !first,
                    String::new(),
                ),
                Err(why) => return Done::line(why),
            },
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
                        self.sound = (i32::from(self.sound) + d).clamp(0, 127) as u8;
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
                            select_sound: Some(self.sound),
                            ..Done::default()
                        };
                    }
                    // Fit to scale: the picks, or every note, onto the
                    // nearest scale note, one take a turn.
                    FIT_ROW => match lit_scale(clip) {
                        None => return Done::line("edit: pick a scale first".into()),
                        Some(k) => {
                            let which = if none {
                                (0..clip.notes.len()).collect()
                            } else {
                                sel.clone()
                            };
                            let e = notes::fit_to_scale(clip, &which, k);
                            let n = e.add.len();
                            let notes = if n == 1 { "note" } else { "notes" };
                            (
                                e,
                                None,
                                !none,
                                true,
                                format!(
                                    "edit: {n} {notes} fitted to {}",
                                    self.offers.label(Some(k))
                                ),
                            )
                        }
                    },
                    // A tool's row (plan 2B.3): opens it, or turns its Q1
                    // while it is open.
                    r => match ToolKind::of_row(r) {
                        Some(kind) => match self.open_tool.as_mut() {
                            Some(t) if t.kind == kind => {
                                t.turn(0, delta, false);
                                return Done::default();
                            }
                            _ => return self.open(kind, clip, &sel),
                        },
                        None => return Done::default(),
                    },
                }
            }
            EditAction::AutoSnap(d) => {
                if d < DIVISIONS.len() {
                    self.auto = d;
                }
                return Done::default();
            }
            // What a drawn note puts in: the editor's, not the clip's.
            EditAction::Chord { shape, minor } => {
                self.chord = shape;
                self.minor = minor;
                let label = shape.label(minor, lit_scale(clip).is_some());
                return Done::line(match shape {
                    ChordShape::Off => "edit: drawing notes".to_string(),
                    _ => format!("edit: drawing chords: {label}"),
                });
            }
            // The clip's scale: a setting, not an edit of its notes.
            // A clip of pad hits takes none, its numbers being pads.
            EditAction::Scale(_) | EditAction::Root(_) if !clip.takes_scale() => {
                return Done::line("edit: a clip of pads has no scale".into())
            }
            EditAction::Scale(k) => {
                if k.is_some_and(|k| !mpfree_seq::scale_ok(k)) {
                    return Done::default();
                }
                return Done {
                    line: format!("edit: scale {}", self.offers.label(k)),
                    scale: Some(k),
                    ..Done::default()
                };
            }
            EditAction::Root(r) => match clip.scale {
                None => return Done::line("edit: pick a scale first".into()),
                Some(k) => {
                    let k = Scale::with_down(r % 12, k.mask, k.down);
                    return Done {
                        line: format!("edit: scale {}", self.offers.label(Some(k))),
                        scale: Some(Some(k)),
                        ..Done::default()
                    };
                }
            },
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
            scale: None,
            hear: hears,
        }
    }

    /// Open `kind` on the `sel` notes of `clip` (plan 2B.3), in place of
    /// a tool already open; refused, with why, without the notes it
    /// needs.
    fn open(&mut self, kind: ToolKind, clip: &Clip, sel: &[usize]) -> Done {
        if sel.is_empty() {
            return Done::line("edit: pick notes first".into());
        }
        let least = kind.least();
        if least > 1 && !notes::chords_of(clip, sel).iter().any(|c| c.len() >= least) {
            return Done::line(match least {
                3 => "edit: pick a chord of three notes or more".into(),
                _ => "edit: pick a chord: notes that sound together".into(),
            });
        }
        self.open_tool = Some(NoteTool::new(kind));
        Done::line(format!(
            "edit: {}: turn the Q-Links, then Done or Cancel",
            kind.name()
        ))
    }

    /// The open tool's preview (plan 2B.3): a copy of `clip` with the
    /// tool's edit in it, and the picks in that copy as Done leaves them
    /// (the notes the edit put in, and the picks it left alone). None with
    /// no tool open, or nothing to change.
    pub fn preview(&self, clip: &Clip) -> Option<(Clip, Vec<usize>)> {
        let tool = self.open_tool?;
        let sel = self.picked(clip);
        let edit = tool.edit(clip, &sel);
        if edit.is_empty() {
            return None;
        }
        let mut copy = clip.clone();
        copy.apply(&edit).ok()?;
        let kept: Vec<Note> = sel.iter().map(|&i| clip.notes[i]).collect();
        let picks = picks_after(&copy, &edit, &kept, true);
        Some((copy, picks))
    }

    /// The editor's sound as the rows name it: a pad or a note.
    fn sound_name(&self, clip: &Clip) -> String {
        sound_name(self.sound, pads(clip))
    }

    /// The cursor `d` snaps on (back if negative), onto the snap's line
    /// first, round the clip (plan 1C.1).
    fn move_cursor(&mut self, clip: &Clip, d: i32) {
        let len = clip.length.max(1);
        let step = self.step();
        let on = self.cursor.div_euclid(step) * step;
        let to = if on != self.cursor && d < 0 {
            on + Tick::from(d + 1) * step
        } else {
            on + Tick::from(d) * step
        };
        self.cursor = to.rem_euclid(len);
        // The tracker keeps the cursor's line third from the top.
        if self.lens == Lens::Tracker {
            let rows = track_rows(&clip.notes, clip.length, self.grid());
            let at = self.grid().place(clip.length, self.cursor).0;
            if let Some(k) = rows.iter().position(|r| r.step == at) {
                self.first = k.saturating_sub(2);
            }
        }
    }

    /// Pad `key` going down with Pads write on (plan 1C.2): its notes
    /// (the kit pad `hit` on a clip of pad hits, the pad map's `notes` on
    /// a clip of notes) at the cursor, a snap long, at `velocity`. The
    /// edit, the chord's number and whether it starts a chord (no other
    /// pad held), or why nothing is written.
    fn pad_write(
        &mut self,
        clip: &Clip,
        key: u8,
        hit: u8,
        notes: Vec<u8>,
        velocity: u8,
    ) -> Result<(Edit, u64, bool), String> {
        let written = match clip.holds {
            Holds::Hits => vec![hit],
            Holds::Notes => notes,
        };
        if written.is_empty() {
            return Err("edit: that pad plays no notes, so nothing was written".into());
        }
        let first = self.pads_held == 0;
        if first {
            self.pad_chord = self.pad_chord.wrapping_add(1);
            self.pad_wrote = false;
        }
        self.pads_held |= 1 << (key & 15);
        let tick = self.cursor.rem_euclid(clip.length.max(1));
        let len = self.step().min(clip.length - tick).max(1);
        // On a clip of notes the pad's notes are its trigger, as when it
        // is recorded (`groups::pad_group`): its chord plays whole or not
        // at all. `notes::write` settles the number (none on pad hits).
        let group = mpfree_seq::groups::pad_group(hit);
        let added: Vec<Note> = written
            .iter()
            .map(|&n| Note {
                group,
                ..Note::new(tick, len, n.min(127), velocity.clamp(1, 127))
            })
            .collect();
        let edit = notes::write(clip, &added);
        if edit.is_empty() {
            return Err(format!(
                "edit: {} has no room for more notes, so nothing was written",
                clip.name
            ));
        }
        self.pad_wrote = true;
        self.sound = written[0];
        self.velocity = velocity.clamp(1, 127);
        Ok((edit, self.pad_chord, first))
    }

    /// Pad `key` let go (plan 1C.2): when it was the last pad held of a
    /// chord that wrote, the cursor steps a snap.
    fn pad_up(&mut self, key: u8, clip: &Clip) {
        let bit = 1 << (key & 15);
        if self.pads_held & bit == 0 {
            return;
        }
        self.pads_held &= !bit;
        if self.pads_held == 0 && std::mem::take(&mut self.pad_wrote) {
            self.move_cursor(clip, 1);
        }
    }

    /// The notes picked, drawn or moved by this touch, to be heard (plan
    /// 1C.2): the earliest of them, up to [`HEARD_MAX`].
    fn hear(&mut self, notes: impl IntoIterator<Item = Note>) {
        let mut notes: Vec<Note> = notes.into_iter().collect();
        let Some(first) = notes.iter().map(|n| n.tick).min() else {
            return;
        };
        notes.retain(|n| n.tick == first);
        notes.sort_by_key(|n| n.note);
        notes.dedup_by_key(|n| n.note);
        notes.truncate(HEARD_MAX);
        self.heard = Heard {
            n: self.heard.n.wrapping_add(1),
            notes,
        };
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
        self.sel = picks_after(clip, &done.edit, &done.kept, done.select_added);
        if done.hear {
            self.hear(done.edit.add.iter().copied());
        }
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
            // An open tool closes with its picks (plan 2B.3).
            self.sel.clear();
            self.cleared = true;
            self.open_tool = None;
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

/// The picks in `clip` after `edit` went in: the `kept` notes it left
/// alone and, with `added`, the notes it put in.
fn picks_after(clip: &Clip, edit: &Edit, kept: &[Note], added: bool) -> Vec<usize> {
    let mut want: Vec<Note> = Vec::new();
    let mut erased = edit.erase.clone();
    for n in kept {
        match erased.iter().position(|e| e == n) {
            Some(j) => {
                erased.swap_remove(j);
            }
            None => want.push(*n),
        }
    }
    if added {
        want.extend(edit.add.iter().copied());
    }
    let mut sel: Vec<usize> = find_notes(clip, &want).into_iter().flatten().collect();
    sel.sort_unstable();
    sel
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
    let picks = ed.picked(clip);
    let sel: Vec<&Note> = picks.iter().map(|&i| &clip.notes[i]).collect();
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
        of_sel(&|n| i64::from(n.note), &|v| sound_name(v as u8, pads(clip))),
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
        // How many notes Fit to scale would move (the picks, or every
        // note).
        match lit_scale(clip) {
            None => "no scale".into(),
            Some(k) => {
                let off = |n: &&Note| !k.contains(i16::from(n.note));
                let n = match sel.is_empty() {
                    true => clip.notes.iter().filter(off).count(),
                    false => sel.iter().copied().filter(off).count(),
                };
                match n {
                    0 => "none off".into(),
                    n => format!("{n} off"),
                }
            }
        },
        // The chord tools (plan 2B.2): the first chord of the picks.
        chord_value(
            clip,
            &picks,
            2,
            &|chord| match notes::strum_of(clip, chord) {
                0 => "none".into(),
                g if g > 0 => format!("↑ {g}t"),
                g => format!("↓ {}t", -g),
            },
        ),
        chord_value(clip, &picks, 2, &|chord| {
            let notes: Vec<u8> = chord.iter().map(|&i| clip.notes[i].note).collect();
            match chord_name(&notes) {
                // A long name drops its bass to fit.
                Some(name) if name.len() > 7 => {
                    name.split('/').next().unwrap_or_default().to_string()
                }
                Some(name) => name,
                None => note_name(notes[0]),
            }
        }),
        chord_value(clip, &picks, 3, &|chord| {
            let (lo, hi) = (chord[0], chord[chord.len() - 1]);
            match clip.notes[hi].note - clip.notes[lo].note {
                0..=11 => "close".into(),
                _ => "open".into(),
            }
        }),
        // Chop, Legato and Humanize (plan 2B.3) open on the picks.
        tool_value(&picks),
        tool_value(&picks),
        tool_value(&picks),
    ];
    // The open tool's row says so.
    let mut values = values;
    if let Some(t) = ed.open_tool {
        values[t.kind.row()] = "on".into();
    }
    EDIT_ROWS
        .iter()
        .zip(values)
        .map(|(name, value)| OptionRow {
            name: name.to_string(),
            value,
        })
        .collect()
}

/// Chop's, Legato's and Humanize's value (plan 2B.3): "open" with notes
/// picked, else "pick notes".
fn tool_value(picks: &[usize]) -> String {
    match picks.is_empty() {
        true => "pick notes".into(),
        false => "open".into(),
    }
}

/// A chord tool's value (plan 2B.2): `show` of the first chord of at
/// least `least` of the picked notes, "pick notes" with none picked, "no
/// chord" with no such chord.
fn chord_value(
    clip: &Clip,
    picks: &[usize],
    least: usize,
    show: &dyn Fn(&[usize]) -> String,
) -> String {
    if picks.is_empty() {
        return "pick notes".into();
    }
    match notes::chords_of(clip, picks)
        .iter()
        .find(|c| c.len() >= least)
    {
        Some(chord) => show(chord),
        None => "no chord".into(),
    }
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

/// A sound as the editor names it: "Pad B5" on a pad clip, else the note.
pub fn sound_name(n: u8, pads: bool) -> String {
    if pads {
        format!("Pad {}", pad_place(n))
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

/// The scale the Notes view lights and moves by: the clip's, but none on
/// a clip of pad hits, which takes none ([`Clip::takes_scale`]), nor
/// while the roll shows pad rows (plan 2B.1: Scale hides there).
pub fn lit_scale(clip: &Clip) -> Option<Scale> {
    clip.scale.filter(|_| clip.takes_scale() && !pads(clip))
}

/// The clip plays the pads: it holds pad hits, not MIDI notes
/// ([`mpfree_seq::Holds`], which says so: nothing is guessed from its
/// notes; piano roll plan 4.2). The roll shows a row a pad, the kit's 128
/// in its 8 banks; a clip of notes shows the keys even while empty, so
/// its first notes (and chords, plan 2B.2) can be drawn.
pub fn pads(clip: &Clip) -> bool {
    clip.holds == mpfree_seq::Holds::Hits
}

/// One note as an edit's line names it: "note C3", or "Pad A1" on a pad
/// clip (plan 4.2).
fn one_named(clip: &Clip, n: u8) -> String {
    match pads(clip) {
        true => sound_name(n, true),
        false => format!("note {}", note_name(n)),
    }
}

/// A kit pad's place as the MPC names it, its bank's letter and its
/// number in the bank: "A1" for the first, "B5" for the 21st, "H16" for
/// the last.
pub fn pad_place(n: u8) -> String {
    format!("{}{}", char::from(b'A' + (n / 16).min(7)), n % 16 + 1)
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
/// notes in the next 16 (each offset so it is never negative), and over
/// them whether the notes are notes of the clip's scale
/// ([`EditAction::Drop`]'s `scale`).
pub fn drop_frame(ticks: Tick, notes: i32, scale: bool) -> u64 {
    let t = (ticks.clamp(-(1 << 30), 1 << 30) + (1 << 31)) as u64;
    let n = (notes.clamp(-127, 127) + 128) as u64;
    u64::from(scale) << 48 | n << 32 | t
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

/// The lanes as [`EventKind::Values`] numbers them: its `index` is a
/// field's place here (the ramp is the Repeats lane with SHIFT), or
/// [`PRESSURE_LANE`] after them.
pub const FIELDS: [Field; 4] = [Field::Velocity, Field::Chance, Field::Repeats, Field::Ramp];

/// The Pressure lane's `index` in [`EventKind::Values`] (plan 3A.1): its
/// `set` is (tick, value) for each column the stroke drew, in time
/// order, then (the stroke's end, [`PRESSURE_END`]).
pub const PRESSURE_LANE: usize = FIELDS.len();

/// The value that marks the end of a pressure stroke ([`PRESSURE_LANE`]).
pub const PRESSURE_END: i32 = -1;

/// The first lock lane's `index` in [`EventKind::Values`] (plan 3A.2):
/// lock lane `k` (its place in [`NoteGrid::lock_settings`]) is
/// `LOCK_LANES + k`, its `set` each note the stroke set, with the place
/// of its new value in the setting's [`LockSetting::values`].
pub const LOCK_LANES: usize = PRESSURE_LANE + 1;

/// A setting a lock lane shows and sets (plan 3A.2), as the app knows it:
/// one of the Steps' lock lanes for the clip (every setting the clip
/// locks and each lane added there), with its name, what a step with no
/// lock plays, and the values a stroke can set, lowest first, each with
/// its words ("1.2 kHz"), in the steps its knob turns by.
#[derive(Clone, Debug, PartialEq)]
pub struct LockSetting {
    pub addr: RelAddr,
    pub name: String,
    pub own: f32,
    pub values: Vec<(f32, String)>,
    /// A value either side of 0 (a tune, a pan): drawn from the middle.
    pub centred: bool,
}

impl LockSetting {
    /// The place in [`LockSetting::values`] nearest `v`.
    pub fn index_of(&self, v: f32) -> usize {
        let k = self.values.partition_point(|x| x.0 < v);
        match (k.checked_sub(1), self.values.get(k)) {
            (Some(j), Some(hi)) if v - self.values[j].0 < hi.0 - v => j,
            (Some(j), None) => j,
            _ => k.min(self.values.len().saturating_sub(1)),
        }
    }
}

/// How wide each column of the Pressure lane is, in pixels: the line is
/// drawn and set column by column.
const PRESSURE_COL: i32 = 4;

/// What the lane under the roll shows and draws (plan 2A.1, 3A.1,
/// 3A.2): a value on each note, the picked notes' pressure along them,
/// or a setting's lock where each note plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneKind {
    Value(Field),
    Pressure,
    /// One of the [`NoteGrid::lock_settings`].
    Lock(RelAddr),
}

impl LaneKind {
    /// The lanes in the order the chip steps through them (the ramp is
    /// the Repeats lane with SHIFT), before the lock lanes.
    pub const ALL: [LaneKind; 4] = [
        LaneKind::Value(Field::Velocity),
        LaneKind::Value(Field::Chance),
        LaneKind::Value(Field::Repeats),
        LaneKind::Pressure,
    ];

    /// Its name (a lock lane's is its setting's: [`NoteGrid::lane_name`]).
    pub fn name(self) -> &'static str {
        match self {
            LaneKind::Value(f) => f.name(),
            LaneKind::Pressure => "Pressure",
            LaneKind::Lock(_) => "Lock",
        }
    }
}

/// A caret 9 px wide centred on (`x`, `y`), pointing up or down (the ▾ of
/// a chip, drawn as the step size chip draws its own).
fn paint_caret(cx: &mut PaintCx, x: i32, y: i32, up: bool, c: Color) {
    for k in 0..5 {
        let row = if up { y + 2 - k } else { y - 2 + k };
        cx.canvas
            .fill_rect(Rect::new(x - 4 + k, row, 9 - 2 * k, 1), c);
    }
}

/// Notes whose starts are this close on screen (px) share a bar of the
/// lane, as notes that start together do: a strum's.
const PLACE_PX: i32 = 3;

/// Where the lane's bars stand in its strip: a little in from its top and
/// its foot.
fn lane_body(g: &Geo) -> Rect {
    Rect::new(g.vel.x, g.vel.y + 2, g.vel.w, (g.vel.h - 5).max(0))
}

/// Note `n`'s value of `field`.
fn box_value(field: Field, n: &NoteBox) -> i32 {
    match field {
        Field::Velocity => i32::from(n.velocity),
        Field::Chance => i32::from(n.chance),
        Field::Repeats => i32::from(n.ratchet),
        Field::Ramp => i32::from(n.ramp),
    }
}

/// What a bar of the lane says over it: the velocity, the chance in
/// percent, the repeats with ↑ or ↓ for a `ramp` that rises or falls, or
/// the ramp itself.
fn lane_label(field: Field, v: i32, ramp: i8) -> String {
    match field {
        Field::Velocity => v.to_string(),
        Field::Chance => format!("{v}%"),
        Field::Repeats if v > 1 && ramp > 0 => format!("{v}↑"),
        Field::Repeats if v > 1 && ramp < 0 => format!("{v}↓"),
        Field::Repeats => v.to_string(),
        Field::Ramp if v > 0 => format!("+{v}"),
        Field::Ramp => v.to_string(),
    }
}

/// The lane under the roll (piano roll plan 2A.1): which one its chip
/// picked, how tall its top edge was dragged, and whether its chip's ▾
/// folded it away. The screen's own, kept by the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneShow {
    pub kind: LaneKind,
    pub h: i32,
    pub folded: bool,
}

impl Default for LaneShow {
    fn default() -> LaneShow {
        LaneShow {
            kind: LaneKind::Value(Field::Velocity),
            h: VEL_H,
            folded: false,
        }
    }
}

/// Which rows the roll shows (piano roll plan 2B.1): the Fold chip in its
/// heading steps through them. The screen's own, kept by the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fold {
    /// Every row.
    #[default]
    All,
    /// Only the rows with notes (Ableton's Fold).
    Notes,
    /// Only the rows of the clip's scale, while it has one.
    Scale,
}

impl Fold {
    /// The Fold chip's words.
    pub fn name(self) -> &'static str {
        match self {
            Fold::All => "All rows",
            Fold::Notes => "Rows with notes",
            Fold::Scale => "Scale rows",
        }
    }

    /// The next on the chip: Scale rows only while the clip has a scale.
    fn next(self, scale: bool) -> Fold {
        match self {
            Fold::All => Fold::Notes,
            Fold::Notes if scale => Fold::Scale,
            _ => Fold::All,
        }
    }
}

/// The Scale menu (plan 2B.1), which the roll's Scale chip opens over the
/// roll (the roll's own): a root tapped is `Tab(ROOT_MENU + root)` (0 is
/// C), a scale `Tab(SCALE_MENU + i)`, `i` its place among the editor's
/// offers ([`ScaleOffers::scale_of`]). The Fold chip and the chip counting
/// hidden notes are the roll's own (`View`).
pub const ROOT_MENU: usize = 300;
/// The Chord menu (plan 2B.2), which the roll's Chord chip opens: a shape
/// tapped is `Tab(CHORD_MENU + i)`, `i` its place in [`ChordShape::ALL`];
/// Major and Minor (with no scale) are [`CHORD_MAJOR`] and
/// [`CHORD_MINOR`] after it.
pub const CHORD_MENU: usize = 500;
pub const CHORD_MAJOR: usize = 8;
pub const CHORD_MINOR: usize = 9;
pub const SCALE_MENU: usize = 400;
/// The heading's chips: the Scale chip's width, the Fold chip's, and the
/// hidden notes' chip's, which stands in for the legend while Fold hides
/// notes.
/// The Sound chip (piano roll plan 1C.2): whether a note picked, drawn
/// or moved plays through the track's instrument. The screen's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hearing {
    #[default]
    On,
    /// Silent while the music plays.
    Stopped,
    Off,
}

impl Hearing {
    /// The chip's words.
    pub fn name(self) -> &'static str {
        match self {
            Hearing::On => "Sound: On",
            Hearing::Stopped => "Sound: Stopped only",
            Hearing::Off => "Sound: Off",
        }
    }

    /// What a tap on the chip steps to.
    pub fn next(self) -> Hearing {
        match self {
            Hearing::On => Hearing::Stopped,
            Hearing::Stopped => Hearing::Off,
            Hearing::Off => Hearing::On,
        }
    }

    /// Notes are heard now, the transport `rolling` or not.
    pub fn hears(self, rolling: bool) -> bool {
        match self {
            Hearing::On => true,
            Hearing::Stopped => !rolling,
            Hearing::Off => false,
        }
    }
}

/// The Pads write and Sound chips' widths (plan 1C.2).
const PADS_CHIP_W: i32 = 104;
const SOUND_CHIP_W: i32 = 156;
const SCALE_CHIP_W: i32 = 200;
const CHORD_CHIP_W: i32 = 190;
const FOLD_CHIP_W: i32 = 150;
const HIDDEN_CHIP_W: i32 = 140;
const SHOW_CHIP_W: i32 = 104;

/// The roll's rows (plan 2B.1): how many, the note each shows, lowest
/// first, and each note's row (-1 when Fold hides it). Every note, or the
/// 16 pads, with nothing folded.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rows {
    n: u8,
    note: [u8; 128],
    row: [i16; 128],
}

impl Rows {
    /// The first `n` notes, each on its own row.
    fn all(n: u8) -> Rows {
        let mut r = Rows {
            n,
            note: [0; 128],
            row: [-1; 128],
        };
        for k in 0..n {
            r.note[usize::from(k)] = k;
            r.row[usize::from(k)] = i16::from(k);
        }
        r
    }

    /// The notes `keep` keeps, of the first `n`; every one when it keeps
    /// none.
    fn only(n: u8, keep: impl Fn(u8) -> bool) -> Rows {
        let mut r = Rows {
            n: 0,
            note: [0; 128],
            row: [-1; 128],
        };
        for k in (0..n).filter(|&k| keep(k)) {
            r.note[usize::from(r.n)] = k;
            r.row[usize::from(k)] = i16::from(r.n);
            r.n += 1;
        }
        if r.n == 0 {
            return Rows::all(n);
        }
        r
    }

    /// Note `note`'s row, if it shows.
    fn of(&self, note: u8) -> Option<u8> {
        u8::try_from(*self.row.get(usize::from(note))?).ok()
    }

    /// Row `row`'s note.
    fn note_of(&self, row: u8) -> Option<u8> {
        (row < self.n).then(|| self.note[usize::from(row)])
    }
}

/// A finger drawing along the lane: the lane as it was when it went down
/// (its bars, one for each place where notes start, or on a lock lane
/// each place a lock sits), and the notes each bar sets; on the Pressure
/// lane, each column's ticks instead. And where the finger is across.
#[derive(Clone, Debug, PartialEq)]
struct Stroke {
    kind: LaneKind,
    lane: Lane,
    notes: Vec<Vec<usize>>,
    cols: Vec<(Tick, Tick)>,
    x: i32,
}

/// A box pick's lowest and highest rows, from its `Span`'s index
/// ([`BOX`]); None for a drawn note's.
pub fn box_of(index: usize) -> Option<(u8, u8)> {
    let k = index.checked_sub(BOX)?;
    Some(((k >> 8 & 0x7F) as u8, (k & 0x7F) as u8))
}

/// The drag a `Marker`'s frame holds ([`drop_frame`]).
pub fn drop_of(frame: u64) -> (Tick, i32, bool) {
    let t = (frame & 0xFFFF_FFFF) as Tick - (1 << 31);
    let n = ((frame >> 32) & 0xFFFF) as i32 - 128;
    (t, n, frame >> 48 & 1 == 1)
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

/// A lock as the roll knows it: its tick, its row (None for the whole
/// step), its setting and value (the tracker counts them, the lock lanes
/// show them, the notes are marked by them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LockMark {
    pub tick: Tick,
    pub sound: Option<u8>,
    pub addr: RelAddr,
    pub value: f32,
}

impl LockMark {
    pub fn of(l: &Lock) -> LockMark {
        LockMark {
            tick: l.tick,
            sound: l.sound,
            addr: l.addr,
            value: l.value,
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

/// The open tool's bar along the bottom of the note editor (piano roll
/// plan 2B.3; [`NoteGrid::tool_bar`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolBar {
    pub name: &'static str,
    /// What turning the Q-Links does, or why the tool changes nothing.
    pub hint: String,
    /// The tool's own button's words and whether it is lit (Chop's Reset
    /// pressure, Humanize's Again).
    pub option: Option<(&'static str, bool)>,
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
    /// Pad rows (A1 to H16, the kit's 128 pads) in place of note names
    /// (piano roll plan 4.2).
    pub pads: bool,
    /// The kit's name for each pad, pad 1 first ("" for none), which the
    /// pad rows show after the pad's place (plan 4.2).
    pub pad_names: Arc<Vec<String>>,
    /// The bank the pads play (0 = A to 7 = H): an empty pad clip opens
    /// on its first row (plan 4.2).
    pub bank: u8,
    pub playhead: Option<Tick>,
    /// The editor's cursor (plan 1C.1): a line on the roll and a mark on
    /// the tracker's line, where the pads will write; the wheel moves it.
    pub cursor: Option<Tick>,
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
    /// The clip's locks: the tracker's Locks column, the lock lanes and
    /// the marks on notes.
    pub locks: Vec<LockMark>,
    /// The clip's step size on the Steps view, where its locks sit
    /// ([`notes::steps_grid`]).
    pub lock_step: Tick,
    /// Where the clip's end handle stops (the page's, plan 1A.2): no
    /// shorter than just after the last note start or lock
    /// ([`notes::shortest_steps`]), no longer than a clip can be, in clip
    /// ticks. It snaps to `bar` or `lock_step`.
    pub shortest: Tick,
    pub longest: Tick,
    /// The settings a lock lane can show (the app's, plan 3A.2).
    pub lock_settings: Arc<Vec<LockSetting>>,
    /// The notes' pad pressure, each [`NoteBox::touch`] a run of it, timed
    /// in clip ticks from its note's start (as `Clip::touch`).
    pub touch: Vec<TouchPoint>,
    /// The lane under the roll (the page's, plan 2A.1): its chip, fold
    /// and top edge change it here, and the page keeps it.
    pub lane: LaneShow,
    /// SHIFT was held as the last finger came down (the app's): a stroke
    /// along the Repeats lane sets the ramp; a drag or a drawn note goes
    /// by semitones with the clip's scale on.
    pub fine: bool,
    /// ERASE was held as the last finger came down (the app's, plan
    /// 1B.5): a finger on the roll erases, a tap the note under it and a
    /// drag every note its path crosses ([`Hand::Sweep`]).
    pub erase: bool,
    /// The clip's scale (plan 2B.1; none on a clip of pads): its rows lit,
    /// its roots marked, notes off it marked; drags and drawn notes go by
    /// its notes.
    pub scale: Option<Scale>,
    /// The Scale chip's words: the scale ("D Dorian") or "Off".
    pub scale_name: String,
    /// What the Scale menu offers (the editor's).
    pub offers: Arc<ScaleOffers>,
    /// The Scale menu is open over the roll (the roll's own).
    pub scale_menu: bool,
    /// What a note drawn puts in (the editor's, plan 2B.2): a note, or a
    /// chord on it, shown on the Chord chip and while drawing.
    pub chord: ChordShape,
    pub minor: bool,
    /// The Chord menu is open over the roll (the roll's own).
    pub chord_menu: bool,
    /// Which rows show (the page's, plan 2B.1): the Fold chip and the
    /// hidden notes' chip change it here, and the page keeps it.
    pub fold: Fold,
    /// The open tool's bar (plan 2B.3), in place of the buttons: Cancel,
    /// the tool's own button, what it shows, Done. While it shows, the
    /// notes are the tool's preview, and the roll and the lane only move
    /// the view.
    pub tool_bar: Option<ToolBar>,
    /// Pads write is on (the editor's, plan 1C.2): its chip is lit like
    /// REC.
    pub pads_write: bool,
    /// The Sound chip (the page's, plan 1C.2): a tap steps it here, and
    /// the page keeps it.
    pub hearing: Hearing,
    /// Other clips shown faintly behind the notes (plan 4.1), in the
    /// order picked, and what the Show menu offers.
    pub ghosts: Vec<GhostLayer>,
    pub show_offers: Vec<ShowOffer>,
    /// The Show menu is open over the roll, on this page of its offers.
    pub show_menu: bool,
    show_page: usize,
    /// The row and tick of the last faint note tapped (a COPY held pastes
    /// there in place of opening its clip).
    pub ghost_cell: (u8, Tick),
    /// A finger drawing along the lane.
    stroke: Option<Stroke>,
    /// The notes an ERASE sweep's path has crossed so far, in the order
    /// it met them ([`Hand::Sweep`]).
    swept: Vec<usize>,
    /// Hold and turn (plan 1C.3): the wheel has turned the note under
    /// the finger since it went down, and the number of the last hold
    /// turned (each a take of its own, [`EditAction::Held`]).
    turning: bool,
    holds: u64,
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

/// What fingers on the roll are doing, besides dragging a note ([`Drag`])
/// or drawing along the lane ([`Stroke`]).
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
    /// A finger on the roll with ERASE held (plan 1B.5), Draw on or off.
    /// Until it moves [`TAP_SLOP`] it is a tap, which erases the note
    /// under it (the one on top) as it lifts; after, it sweeps, and every
    /// note its path crosses ([`NoteGrid::swept`]) is marked and comes
    /// out as it lifts, one take. A second finger calls it off.
    Sweep {
        finger: u64,
        at: (i32, i32),
        now: (i32, i32),
        moved: bool,
    },
    /// A finger on the clip's end handle on the ruler (plan 1A.2): the
    /// clip's new end `to` where the finger has taken it, `grab` pixels to
    /// its right as it went down ([`NoteGrid::end_at`]), sent as it lifts.
    /// Taken to the roll's right edge, [`END_PUSH`] right of `x0` where it
    /// went down at least, so a finger resting on an end already at the
    /// edge takes nothing on (`edge`: seconds there), the end goes on a
    /// bar or step every [`END_RATE`] seconds and the roll, `span` ticks
    /// across, scrolls with it ([`NoteGrid::grow_end`]).
    End {
        finger: u64,
        now: (i32, i32),
        x0: i32,
        grab: i32,
        to: Tick,
        edge: Option<f32>,
        span: Tick,
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
    /// A finger on the lane's top edge (plan 2A.1): the lane is as tall
    /// as it was (`from`) plus how far the finger went up from `y0`.
    Edge {
        finger: u64,
        now: (i32, i32),
        y0: i32,
        from: i32,
    },
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
/// The clip's end handle (plan 1A.2): its grip's width on the ruler, how
/// near the roll's right edge it takes the end on with time and how fast
/// (seconds a unit), how wide a step must be before it snaps to steps
/// rather than bars, and how far right a finger must push first.
const END_W: i32 = 28;
const END_EDGE: i32 = 40;
const END_RATE: f32 = 0.3;
const END_STEP_PX: i32 = 24;
const END_PUSH: i32 = 12;
/// An ERASE sweep looks for notes this often along its path (pixels), so
/// a fast finger misses none as narrow as this.
const SWEEP_STEP: i32 = 3;
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
    /// The finger drags a handle (plan 1B.2): the note's start (true) or
    /// its end, by `ticks`; `notes` stays 0.
    handle: Option<bool>,
    /// `notes` counts notes of the clip's scale (it was on, and SHIFT
    /// wasn't held, as the finger went down; plan 2B.1), else semitones.
    scale: bool,
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
/// The key column on pad rows (plan 4.2): room for a pad's place and the
/// kit's name for it.
const PAD_KEYS_W: i32 = 112;
const RULER_H: i32 = 22;
/// The overview strip over the ruler, with Fit under the keys and Follow
/// at its right end.
const OVERVIEW_H: i32 = 32;
const FOLLOW_W: i32 = 104;
/// The lane under the roll opens this tall; its top edge drags it from
/// [`LANE_MIN`] to [`LANE_MAX`], and folded it is [`LANE_FOLDED`].
const VEL_H: i32 = 64;
const LANE_MIN: i32 = 48;
const LANE_MAX: i32 = 200;
const LANE_FOLDED: i32 = 26;
/// The lane's top edge answers a finger this far into the lane, besides
/// the gap over it.
const EDGE_IN: i32 = 4;
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
            pad_names: Arc::default(),
            bank: 0,
            playhead: None,
            cursor: None,
            snap: PPQ as Tick / 4,
            snap_name: "1/16".into(),
            snap_auto: false,
            tool: Tool::Select,
            lens: Lens::Grid,
            first: 0,
            color: crate::view::track_color(2),
            step: STEP_FALLBACK,
            locks: Vec::new(),
            lock_step: STEP_FALLBACK,
            shortest: STEP_FALLBACK,
            longest: mpfree_seq::MAX_CLIP_TICKS,
            lock_settings: Arc::default(),
            touch: Vec::new(),
            lane: LaneShow::default(),
            fine: false,
            erase: false,
            scale: None,
            scale_name: "Off".into(),
            offers: Arc::default(),
            scale_menu: false,
            chord: ChordShape::Off,
            minor: false,
            chord_menu: false,
            fold: Fold::All,
            tool_bar: None,
            pads_write: false,
            hearing: Hearing::On,
            ghosts: Vec::new(),
            show_offers: Vec::new(),
            show_menu: false,
            show_page: 0,
            ghost_cell: (0, 0),
            stroke: None,
            swept: Vec::new(),
            turning: false,
            holds: 0,
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
    /// The lane's bars (folded: its strip), its chip in the key column
    /// with the ▾ that folds it under, and its top edge's grip (empty
    /// when folded or off the roll).
    vel: Rect,
    chip: Rect,
    fold: Rect,
    edge: Rect,
    tools: Rect,
    /// The heading's chips (plan 2B.1): Scale and Fold at its right end
    /// (empty off the roll), and the one counting the notes Fold hides
    /// (empty while it hides none).
    scale_chip: Rect,
    chord_chip: Rect,
    fold_chip: Rect,
    hidden_chip: Rect,
    /// Pads write and Sound (plan 1C.2): on the roll by Follow, on the
    /// list and the tracker at the heading's right end.
    pads_chip: Rect,
    sound_chip: Rect,
    /// Show (plan 4.1): left of Pads write on the roll; none elsewhere.
    show_chip: Rect,
    /// The heading's line, for the legend.
    head: Rect,
    /// The rows, as Fold leaves them.
    rows: Rows,
    /// The view kept within this roll ([`RollView::clamp`]).
    v: RollView,
}

/// The Show menu's parts ([`NoteGrid::show_menu_rects`]).
struct ShowMenu {
    none: Rect,
    line: Rect,
    cells: Vec<(usize, Rect)>,
    more: Option<Rect>,
    page: usize,
    pages: usize,
    panel: Rect,
}

impl NoteGrid {
    pub fn new() -> NoteGrid {
        NoteGrid::default()
    }

    /// It shows what `other` shows (a drag in progress aside).
    pub fn same(&self, other: &NoteGrid) -> bool {
        self.notes == other.notes
            && self.playhead == other.playhead
            && self.cursor == other.cursor
            && self.title == other.title
            && self.legend == other.legend
            && (self.length, self.bar, self.pads, self.drawer, self.add)
                == (other.length, other.bar, other.pads, other.drawer, other.add)
            && (&self.pad_names, self.bank) == (&other.pad_names, other.bank)
            && self.view == other.view
            && (self.snap, self.tool, self.lens, self.first)
                == (other.snap, other.tool, other.lens, other.first)
            && (&self.snap_name, self.snap_auto) == (&other.snap_name, other.snap_auto)
            && self.color == other.color
            && self.step == other.step
            && self.locks == other.locks
            && self.lock_step == other.lock_step
            && self.lock_settings == other.lock_settings
            && self.touch == other.touch
            && self.lane == other.lane
            && (self.scale, &self.scale_name, self.fold)
                == (other.scale, &other.scale_name, other.fold)
            && self.offers == other.offers
            && (self.chord, self.minor) == (other.chord, other.minor)
            && self.tool_bar == other.tool_bar
            && (self.pads_write, self.hearing) == (other.pads_write, other.hearing)
            && self.ghosts == other.ghosts
            && self.show_offers == other.show_offers
    }

    /// Note `i`'s pressure, empty for none.
    pub fn touch_of(&self, i: usize) -> &[TouchPoint] {
        self.notes
            .get(i)
            .and_then(|n| self.touch.get(n.touch.range()))
            .unwrap_or(&[])
    }

    /// The lane the chip has picked, while it can show: a lock lane whose
    /// setting the app no longer offers shows Velocity.
    pub fn lane_kind(&self) -> LaneKind {
        match self.lane.kind {
            LaneKind::Lock(a) if self.lock_index(a).is_none() => LaneKind::Value(Field::Velocity),
            k => k,
        }
    }

    /// Lock lane `addr`'s place in [`NoteGrid::lock_settings`].
    fn lock_index(&self, addr: RelAddr) -> Option<usize> {
        self.lock_settings.iter().position(|s| s.addr == addr)
    }

    /// Lane `kind`'s name: a lock lane's is its setting's.
    pub fn lane_name(&self, kind: LaneKind) -> String {
        match kind {
            LaneKind::Lock(a) => self.lock_index(a).map_or_else(
                || kind.name().to_string(),
                |k| self.lock_settings[k].name.clone(),
            ),
            k => k.name().to_string(),
        }
    }

    /// The lanes the chip steps through: Vel, Chance, Repeats and
    /// Pressure, then each lock lane.
    fn lane_kinds(&self) -> Vec<LaneKind> {
        LaneKind::ALL
            .into_iter()
            .chain(self.lock_settings.iter().map(|s| LaneKind::Lock(s.addr)))
            .collect()
    }

    /// The clip's steps as the Steps view cuts them: where its locks sit.
    fn steps_grid(&self) -> Grid {
        Grid::new(self.lock_step)
            .or_else(|| Grid::new(mpfree_seq::steps::DEFAULT_SIZE))
            .expect("a 1/16 is a step size")
    }

    /// Note `i`'s step and the row a lock of `addr` on it sits on
    /// ([`notes::lock_row`]: its own pitch's when one is there, a pad's
    /// setting on its pad, else the whole step); None when the setting
    /// isn't the note's.
    fn lock_place(&self, i: usize, addr: RelAddr) -> Option<(usize, Option<u8>)> {
        let n = self.notes.get(i)?;
        let g = self.steps_grid();
        let step = g.place(self.length, n.tick).0;
        let own = self.locks.iter().any(|l| {
            l.addr == addr && l.sound == Some(n.note) && g.place(self.length, l.tick).0 == step
        });
        Some((step, notes::lock_row(addr, n.note, own)?))
    }

    /// The value of `addr` locked at `place` (a step and a row).
    fn lock_at(&self, place: (usize, Option<u8>), addr: RelAddr) -> Option<f32> {
        let g = self.steps_grid();
        self.locks
            .iter()
            .find(|l| {
                l.addr == addr && l.sound == place.1 && g.place(self.length, l.tick).0 == place.0
            })
            .map(|l| l.value)
    }

    /// Where the clip's locks sit, each a step and a row (None for the
    /// whole step), in order, once each: what [`NoteGrid::marks_in`]
    /// looks in.
    fn lock_places(&self) -> Vec<(usize, Option<u8>)> {
        if self.locks.is_empty() {
            return Vec::new();
        }
        let g = self.steps_grid();
        let mut v: Vec<(usize, Option<u8>)> = self
            .locks
            .iter()
            .map(|l| (g.place(self.length, l.tick).0, l.sound))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Note `i`'s marks (plan 3A.2): whether a lock sits where it plays
    /// (on its pitch or its whole step), and whether it has a condition
    /// or a chance under 100 %.
    fn marks(&self, i: usize) -> (bool, bool) {
        self.marks_in(i, &self.lock_places())
    }

    /// [`NoteGrid::marks`] with the locks' `places` found once, for a
    /// paint of every note.
    fn marks_in(&self, i: usize, places: &[(usize, Option<u8>)]) -> (bool, bool) {
        let Some(n) = self.notes.get(i) else {
            return (false, false);
        };
        let locked = !places.is_empty() && {
            let step = self.steps_grid().place(self.length, n.tick).0;
            places.binary_search(&(step, None)).is_ok()
                || places.binary_search(&(step, Some(n.note))).is_ok()
        };
        (locked, n.cond != Cond::Always || n.chance < 100)
    }

    /// The corner of note `i`'s box in `g` a tap on opens a lane (plan
    /// 3A.2): its top right, 20 px, while it has a mark and room beside
    /// it for a tap on the note itself (48 px wide).
    fn mark_zone(&self, g: &Geo, i: usize) -> Option<Rect> {
        let nr = self.note_rect_in(g, i)?;
        let (lock, when) = self.marks(i);
        ((lock || when) && nr.w >= 48 && nr.h >= 12)
            .then(|| Rect::new(nr.right() - 20, nr.y, 20, nr.h.min(20)))
    }

    /// The lane a tap on note `i`'s marks opens: the first lock lane with
    /// a lock where it plays, else Chance for a condition or a chance.
    fn mark_lane(&self, i: usize) -> Option<LaneKind> {
        let (lock, when) = self.marks(i);
        let locked = lock.then(|| {
            self.lock_settings.iter().find(|s| {
                self.lock_place(i, s.addr)
                    .is_some_and(|p| self.lock_at(p, s.addr).is_some())
            })
        });
        match locked.flatten() {
            Some(s) => Some(LaneKind::Lock(s.addr)),
            None => when.then_some(LaneKind::Value(Field::Chance)),
        }
    }

    /// Show what `other` shows, keeping what the fingers are doing.
    /// A finger on a note or dragging one follows its note to where it
    /// now is in `other`'s notes (by value), or lets go of it when it is
    /// gone, so its lift never acts on another note.
    pub fn show(&mut self, other: NoteGrid) {
        let (mut drag, mut hand) = (self.drag, self.hand);
        // A stroke names the notes it was drawn over: other notes end it.
        let stroke = self.stroke.take().filter(|_| other.notes == self.notes);
        let refind = |i: usize| refind(&self.notes, &other.notes, i);
        if let Some(d) = drag.as_mut() {
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
        // A sweep's notes follow to their places in `other`; one gone is
        // no longer there to erase.
        let mut swept: Vec<usize> = match hand {
            Hand::Sweep { .. } => self.swept.iter().filter_map(|&i| refind(i)).collect(),
            _ => Vec::new(),
        };
        swept.dedup();
        let (overview_tap, empty_tap, menu) = (self.overview_tap, self.empty_tap, self.menu);
        let (fine, erase) = (self.fine, self.erase);
        let (turning, holds) = (self.turning, self.holds);
        let (scale_menu, chord_menu) = (self.scale_menu, self.chord_menu);
        // A tool's bar closes the menus over the roll (plan 2B.3).
        let open = other.tool_bar.is_none();
        let (menu, scale_menu, chord_menu) = (menu && open, scale_menu && open, chord_menu && open);
        // The Show menu stays open for more picks (plan 4.1).
        let (show_menu, show_page, ghost_cell) = (self.show_menu, self.show_page, self.ghost_cell);
        *self = NoteGrid {
            stroke,
            swept,
            fine,
            erase,
            turning,
            holds,
            drag,
            hand,
            overview_tap,
            empty_tap,
            menu,
            scale_menu,
            chord_menu,
            show_menu,
            show_page,
            ghost_cell,
            ..other
        };
    }

    fn geo(&self, r: Rect) -> Geo {
        // Matches PaintCx::titled_panel with the default theme's padding.
        let (head, body) = r.inset(14).take_top(34);
        let rows = self.row_map();
        let (tools, rest) = body.take_bottom(TOOLS_H);
        let rest = Rect::new(rest.x, rest.y, rest.w, rest.h - GAP);
        let roll_lens = self.lens == Lens::Grid;
        let (vel, rest) = if !roll_lens {
            (Rect::new(rest.x, rest.bottom(), rest.w, 0), rest)
        } else {
            let (v, rest) = rest.take_bottom(self.lane_h(rest.h));
            (v, Rect::new(rest.x, rest.y, rest.w, rest.h - GAP))
        };
        let (strip, rest) = if roll_lens {
            let (o, rest) = rest.take_top(OVERVIEW_H);
            (o, Rect::new(rest.x, rest.y + 4, rest.w, rest.h - 4))
        } else {
            (Rect::new(rest.x, rest.y, rest.w, 0), rest)
        };
        let (ruler, rest) = rest.take_top(RULER_H);
        let (keys, roll) = rest.take_left(if self.pads { PAD_KEYS_W } else { KEYS_W });
        let follow = Rect::new(roll.right() - FOLLOW_W, strip.y, FOLLOW_W, strip.h);
        let none = Rect::new(0, 0, 0, 0);
        let open = roll_lens && !self.lane.folded;
        let chip_h = if open { 28.min(vel.h) } else { vel.h };
        // The heading's chips, right to left: Fold, Scale, Chord (not on
        // pads), and the notes Fold hides, each as tall as the title's
        // line.
        let chip = |right: i32, w: i32| Rect::new(right - w, head.y - 2, w, 26);
        let fold_chip = chip(head.right(), FOLD_CHIP_W);
        // None on pad rows (plan 4.2): a clip of pad hits has no scale.
        let scale_chip = match self.pads {
            true => Rect::new(fold_chip.x, fold_chip.y, 0, 0),
            false => chip(fold_chip.x - 6, SCALE_CHIP_W),
        };
        let chord_chip = match self.pads {
            true => Rect::new(scale_chip.x, scale_chip.y, 0, 0),
            false => chip(scale_chip.x - 6, CHORD_CHIP_W),
        };
        let left = if self.pads {
            scale_chip.x
        } else {
            chord_chip.x
        };
        let hidden = roll_lens && self.hidden(&rows) > 0;
        // Pads write and Sound: by Follow on the roll, else the heading's
        // right end.
        let (pads_chip, sound_chip) = if roll_lens {
            let sound = Rect::new(follow.x - 6 - SOUND_CHIP_W, strip.y, SOUND_CHIP_W, strip.h);
            let pads = Rect::new(sound.x - 6 - PADS_CHIP_W, strip.y, PADS_CHIP_W, strip.h);
            (pads, sound)
        } else {
            let sound = chip(head.right(), SOUND_CHIP_W);
            (chip(sound.x - 6, PADS_CHIP_W), sound)
        };
        let show_chip = match roll_lens {
            true => Rect::new(pads_chip.x - 6 - SHOW_CHIP_W, strip.y, SHOW_CHIP_W, strip.h),
            false => none,
        };
        Geo {
            pads_chip,
            sound_chip,
            show_chip,
            head: Rect::new(head.x, head.y - 2, head.w, 26),
            scale_chip: if roll_lens { scale_chip } else { none },
            chord_chip: if roll_lens && !self.pads {
                chord_chip
            } else {
                none
            },
            fold_chip: if roll_lens { fold_chip } else { none },
            hidden_chip: match hidden {
                true => chip(left - 6, HIDDEN_CHIP_W),
                false => none,
            },
            overview: Rect::new(roll.x, strip.y, (show_chip.x - 6 - roll.x).max(0), strip.h),
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
            chip: match roll_lens {
                true => Rect::new(keys.x, vel.y, keys.w - 6, chip_h),
                false => none,
            },
            fold: match open {
                true => Rect::new(keys.x, vel.y + 30, keys.w - 6, (vel.h - 30).clamp(0, 24)),
                false => none,
            },
            edge: match open {
                true => Rect::new(roll.x, vel.y - GAP, roll.w, GAP + EDGE_IN),
                false => none,
            },
            tools,
            v: self.view.clamped(roll, self.shown_length(), rows.n),
            rows,
        }
    }

    /// The fold the roll shows: Scale rows only while the clip has a
    /// scale, and only on the roll.
    fn fold_now(&self) -> Fold {
        match self.fold {
            _ if self.lens != Lens::Grid => Fold::All,
            Fold::Scale if self.scale.is_none() => Fold::All,
            f => f,
        }
    }

    /// The rows as Fold leaves them (plan 2B.1): every one; the rows with
    /// notes; or the scale's rows, and any row with a picked note, which
    /// never folds away.
    fn row_map(&self) -> Rows {
        let n = 128;
        let fold = self.fold_now();
        if fold == Fold::All {
            return Rows::all(n);
        }
        let mut with = [false; 128];
        let mut picked = [false; 128];
        for b in &self.notes {
            with[usize::from(b.note & 127)] = true;
            picked[usize::from(b.note & 127)] |= b.selected;
        }
        match (fold, self.scale) {
            (Fold::Notes, _) => Rows::only(n, |k| with[usize::from(k)]),
            (Fold::Scale, Some(s)) => {
                Rows::only(n, |k| s.contains(i16::from(k)) || picked[usize::from(k)])
            }
            _ => Rows::all(n),
        }
    }

    /// How many notes Fold hides on `rows`.
    fn hidden(&self, rows: &Rows) -> usize {
        if rows.n >= 128 {
            return 0;
        }
        self.notes
            .iter()
            .filter(|b| rows.of(b.note).is_none())
            .count()
    }

    /// The notes a note drawn on `note` puts in (plan 2B.2): it alone,
    /// or the Chord chip's chord on it, as the editor makes it.
    fn chord_on(&self, note: u8) -> Vec<u8> {
        let shape = if self.pads {
            ChordShape::Off
        } else {
            self.chord
        };
        chord_notes(shape, self.minor, note, self.scale, self.offers.pads)
    }

    /// The note a finger on the row of `note` draws (plan 2B.1): with the
    /// clip's scale on, the scale note nearest it (the lower when two
    /// are as near), unless SHIFT was held.
    fn drawn_note(&self, note: u8) -> u8 {
        match self.scale.filter(|_| !self.fine && !self.pads) {
            Some(s) => s
                .snapped(i16::from(note), mpfree_seq::mpfree_scale::Snap::Nearest)
                .map_or(note, |n| n.clamp(0, 127) as u8),
            None => note,
        }
    }

    /// How tall the lane is, with `room` for the roll, the strips over it
    /// and the lane: folded, or as its top edge left it, keeping the roll
    /// four 40 px rows at least.
    fn lane_h(&self, room: i32) -> i32 {
        if self.lane.folded {
            return LANE_FOLDED;
        }
        let most = (room - GAP - OVERVIEW_H - 4 - RULER_H - 160).clamp(LANE_MIN, LANE_MAX);
        self.lane.h.clamp(LANE_MIN, most)
    }

    /// How many rows the roll has: 128 pads or notes, less what Fold
    /// hides.
    fn rows(&self) -> u8 {
        self.row_map().n
    }

    /// The x of clip tick `t` on the roll.
    fn x_of(&self, g: &Geo, t: Tick) -> i32 {
        g.v.x_of(g.roll, t)
    }

    /// The note of the row at `y`, if it is on the roll.
    fn note_at(&self, g: &Geo, y: i32) -> Option<u8> {
        g.rows.note_of(g.v.note_at(g.roll, y, g.rows.n)?)
    }

    /// The y of the top of `note`'s row, if any of it shows (not while
    /// Fold hides it).
    fn y_of(&self, g: &Geo, note: u8) -> Option<i32> {
        let y = g.v.row_top(g.roll, g.rows.of(note)?);
        (y + g.v.row_h > g.roll.y && y < g.roll.bottom()).then_some(y)
    }

    /// The view a clip opens with on this roll, in area `r` (the page
    /// keeps it for the clip after: [`RollView::open`]).
    pub fn open_view(&self, r: Rect) -> RollView {
        let roll = NoteGrid {
            lens: Lens::Grid,
            pads: self.pads,
            ..NoteGrid::default()
        }
        .geo(r)
        .roll;
        let rows = self.row_map();
        let v = RollView::open(
            self.notes
                .iter()
                .filter_map(|n| Some((n.tick, rows.of(n.note)?))),
            self.length,
            self.bar,
            roll,
            rows.n,
        );
        // An empty pad clip: the bank the pads play, from its first pad
        // (plan 4.2).
        match self.pads && self.notes.is_empty() {
            true => self.bank_view(v, roll, self.bank),
            false => v,
        }
    }

    /// The view `v` scrolled to bank `bank`'s first pad row (plan 4.2:
    /// PAD BANK on a pad clip), that row at the bottom of the roll, or
    /// the first row Fold leaves from there up; the zoom as it was.
    fn bank_view(&self, mut v: RollView, roll: Rect, bank: u8) -> RollView {
        let rows = self.row_map();
        let first = bank.min(7) * 16;
        let row = (first..128)
            .find_map(|n| rows.of(n))
            .or_else(|| (0..first).rev().find_map(|n| rows.of(n)))
            .unwrap_or(0);
        v.lift = i32::from(row) * v.row_h;
        v.follow = false;
        v.clamped(roll, self.length, rows.n)
    }

    /// The roll in `r` scrolled to bank `bank`'s first pad row ("PAD BANK
    /// jumps to a bank", plan 4.2), if it shows pad rows; None otherwise.
    pub fn show_bank(&mut self, r: Rect, bank: u8) -> Option<RollView> {
        self.bank = bank.min(7);
        if !self.pads || self.lens != Lens::Grid {
            return None;
        }
        let g = self.geo(r);
        self.view = self.bank_view(g.v, g.roll, self.bank);
        Some(self.view)
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

    /// Scroll the view just far enough that tick `t` shows, an eighth of
    /// the roll in from its edge (the cursor, plan 1C.1), in area `r`:
    /// true if it moved. Not while fingers are on the roll.
    pub fn show_tick(&mut self, r: Rect, t: Tick) -> bool {
        if self.busy() {
            return false;
        }
        let g = self.geo(r);
        let mut v = g.v;
        if v.show_tick(g.roll, self.length, self.rows(), t) {
            self.view = v;
            return true;
        }
        false
    }

    /// SHIFT + – / + (plan 1C.1): zoom in time (`d` above 0) or out, half
    /// as much again a step, about the cursor when it shows, else the
    /// roll's middle, in area `r`: true if it moved. Not while fingers are
    /// on the roll.
    pub fn zoom_step(&mut self, r: Rect, d: i32) -> bool {
        if self.busy() || d == 0 || self.lens != Lens::Grid {
            return false;
        }
        let g = self.geo(r);
        let v = g.v;
        let shown = v.ticks_shown(g.roll);
        let at = self
            .cursor
            .filter(|c| (v.start..v.start + shown).contains(c))
            .unwrap_or(v.start + shown / 2);
        let x = self.x_of(&g, at);
        let ppt = v.ppt * 1.5f64.powi(d);
        let to = v.zoom_time(g.roll, self.length, self.rows(), ppt, at as f64, x);
        if to == v {
            return false;
        }
        self.view = to;
        true
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

    /// The note a finger drags across the roll (not by a handle), as it
    /// would land now: for hearing it as it crosses rows (plan 1C.2).
    /// None while no note is dragged.
    pub fn dragging(&self) -> Option<NoteBox> {
        let d = self.drag.filter(|d| d.handle.is_none())?;
        let n = *self.notes.get(d.index)?;
        let (tick, len, note) = self.dragged(d, &n)?;
        Some(NoteBox {
            tick,
            len,
            note,
            ..n
        })
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
                u8::try_from(match self.scale.filter(|_| d.scale) {
                    Some(k) => notes::scale_move(i32::from(n.note), d.notes, k),
                    None => i32::from(n.note) + d.notes,
                })
                .ok()
                .filter(|&p| p < 128)?,
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
    /// Where another clip's faint note is drawn (plan 4.1), if it shows.
    fn ghost_rect(&self, g: &Geo, n: &Note) -> Option<Rect> {
        let y = self.y_of(g, n.note)?;
        let x0 = self.x_of(g, n.tick);
        let x1 = self.x_of(g, n.tick + n.len).max(x0 + 3);
        if x1 < g.roll.x || x0 > g.roll.right() {
            return None;
        }
        Some(Rect::new(x0, y + 2, x1 - x0, g.v.row_h - 4))
    }

    /// The other clip whose faint note is under `(x, y)`: the last drawn
    /// (the top layer) first.
    fn ghost_under(&self, g: &Geo, (x, y): (i32, i32)) -> Option<usize> {
        (0..self.ghosts.len()).rev().find(|&k| {
            self.ghosts[k]
                .notes
                .iter()
                .any(|n| self.ghost_rect(g, n).is_some_and(|r| r.contains(x, y)))
        })
    }

    /// The Show menu's parts (plan 4.1): over the roll under the overview
    /// row, so Show stays in sight; None and a line saying what shows,
    /// then a cell for each offer on the page it shows, with its index in
    /// [`NoteGrid::show_offers`], and More when they don't all fit.
    fn show_menu_rects(&self, r: Rect) -> ShowMenu {
        const PAD: i32 = 8;
        const H: i32 = 48;
        const GAP: i32 = 4;
        let g = self.geo(r);
        let y = g.ruler.y;
        let area = Rect::new(g.keys.x, y, g.roll.right() - g.keys.x, g.tools.y - 6 - y);
        let inner = area.inset(PAD);
        let none = Rect::new(inner.x, inner.y, 120, 40);
        let line = Rect::new(
            none.right() + 12,
            inner.y,
            inner.right() - none.right() - 12,
            40,
        );
        let top = inner.y + 40 + PAD;
        let cols = if inner.w >= 720 { 3 } else { 2 };
        let cw = (inner.w - (cols - 1) * GAP) / cols;
        let fit = ((inner.bottom() - top + GAP) / (H + GAP)).max(1);
        let per = (cols * fit) as usize;
        let n = self.show_offers.len();
        let paged = n > per;
        let room = if paged {
            per.saturating_sub(1).max(1)
        } else {
            per
        };
        let pages = n.div_ceil(room).max(1);
        let page = self.show_page % pages;
        let cell = |k: usize| {
            let k = k as i32;
            Rect::new(
                inner.x + (k % cols) * (cw + GAP),
                top + (k / cols) * (H + GAP),
                cw,
                H,
            )
        };
        let cells: Vec<(usize, Rect)> = (page * room..n.min((page + 1) * room))
            .enumerate()
            .map(|(k, i)| (i, cell(k)))
            .collect();
        let more = paged.then(|| cell(cells.len()));
        let used = (cells.len() + usize::from(more.is_some())) as i32;
        let rows = (used + cols - 1) / cols;
        let bottom = match rows {
            0 => top,
            _ => top + rows * (H + GAP) - GAP + PAD,
        };
        ShowMenu {
            none,
            line,
            cells,
            more,
            page,
            pages,
            panel: Rect::new(area.x, area.y, area.w, bottom - area.y),
        }
    }

    fn in_view(&self, g: &Geo) -> Vec<usize> {
        let (from, to) = (
            g.v.tick_at(g.roll, g.roll.x),
            g.v.tick_at(g.roll, g.roll.right()) + 1,
        );
        if self.drag.is_some() || (from <= 0 && to >= self.length) {
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

    /// The open tool's bar (plan 2B.3): Cancel where the first button
    /// is, the tool's own button over the next two (none without one),
    /// what it shows across those up to the last, and Done there.
    fn tool_rects(&self, r: Rect) -> (Rect, Option<Rect>, Rect, Rect) {
        let b = self.button_rects(r);
        let span = |a: Rect, z: Rect| Rect::new(a.x, a.y, z.right() - a.x, a.h);
        let option = self
            .tool_bar
            .as_ref()
            .and_then(|t| t.option)
            .map(|_| span(b[1], b[2]));
        let hint = match option {
            Some(_) => span(b[3], b[6]),
            None => span(b[1], b[6]),
        };
        (b[TOOL_CANCEL], option, hint, b[TOOL_DONE])
    }

    /// What button `k` says: some show a state.
    pub fn button_label(&self, k: usize) -> String {
        if let Some(t) = &self.tool_bar {
            return match k {
                TOOL_CANCEL => "Cancel".into(),
                TOOL_OPTION => t.option.map_or("", |o| o.0).into(),
                TOOL_DONE => "Done".into(),
                _ => String::new(),
            };
        }
        match k {
            1 => self.lens.name().into(),
            // Snap Auto names itself ("Auto · 1/16").
            3 if self.snap_name.starts_with("Auto") => self.snap_name.clone(),
            3 => format!("Snap {}", self.snap_name),
            _ => EDIT_BUTTONS.get(k).copied().unwrap_or("").into(),
        }
    }

    /// A row's name: the note, or the pad's place and the kit's name
    /// for it ("A1 Kick", plan 4.2).
    fn row_name(&self, note: u8) -> String {
        if !self.pads {
            return note_name(note);
        }
        match self
            .pad_names
            .get(usize::from(note))
            .filter(|n| !n.is_empty())
        {
            Some(name) => format!("{} {name}", pad_place(note)),
            None => pad_place(note),
        }
    }

    /// The tick a dragged note's start lands on: snapped to a line.
    fn snapped(&self, t: Tick) -> Tick {
        match self.snap {
            0 => t,
            s => (t as f64 / s as f64).round() as Tick * s,
        }
    }

    /// Pads write, lit like REC while on, and the Sound chip, tinted
    /// while it holds notes back (plan 1C.2).
    fn paint_switches(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let quiet = t.raised.mix(t.warn, 0.35);
        let show = match self.ghosts.len() {
            0 => "Show".to_string(),
            n => format!("Show: {n}"),
        };
        let shows = !self.ghosts.is_empty() || self.show_menu;
        let switches = [
            (g.show_chip, show.as_str(), shows.then_some(t.accent)),
            (g.pads_chip, "Pads write", self.pads_write.then_some(t.bad)),
            (
                g.sound_chip,
                self.hearing.name(),
                (self.hearing != Hearing::On).then_some(quiet),
            ),
        ];
        for (r, label, lit) in switches {
            if r.is_empty() {
                continue;
            }
            let fill = lit.unwrap_or(t.raised);
            cx.canvas.fill_round_rect(r, 8, fill);
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                r,
                Align::Center,
                t.ink_on(fill),
                label,
            );
        }
    }

    /// The heading: on the roll the Scale and Fold chips at its right end
    /// (plan 2B.1), then the chip counting the notes Fold hides; on the
    /// list and the tracker Pads write and Sound there (plan 1C.2); then
    /// the legend.
    fn paint_head(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        if g.fold_chip.is_empty() {
            self.paint_switches(cx, g);
            self.paint_legend(cx, g, g.pads_chip.x);
            return;
        }
        let ink = t.ink_on(t.raised);
        let chip = |cx: &mut PaintCx, r: Rect, text: &str, fill: Color, ink: Color| {
            cx.canvas.fill_round_rect(r, t.round(8), fill);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(r.x + 8, r.y, r.w - 22, r.h),
                Align::Left,
                ink,
                text,
            );
            paint_caret(cx, r.right() - 12, r.y + r.h / 2, false, ink);
        };
        if !g.scale_chip.is_empty() {
            let (fill, scale_ink) = match self.scale.is_some() {
                true => (
                    t.raised.mix(self.color, 0.35),
                    t.ink_on(t.raised.mix(self.color, 0.35)),
                ),
                false => (t.raised, ink),
            };
            chip(
                cx,
                g.scale_chip,
                &format!("Scale: {}", self.scale_name),
                fill,
                scale_ink,
            );
        }
        let fold = self.fold_now();
        let (fill, fold_ink) = match fold {
            Fold::All => (t.raised, ink),
            _ => (
                t.raised.mix(self.color, 0.35),
                t.ink_on(t.raised.mix(self.color, 0.35)),
            ),
        };
        chip(cx, g.fold_chip, fold.name(), fill, fold_ink);
        if !g.chord_chip.is_empty() {
            let (fill, chord_ink) = match self.chord {
                ChordShape::Off => (t.raised, ink),
                _ => (
                    t.raised.mix(self.color, 0.35),
                    t.ink_on(t.raised.mix(self.color, 0.35)),
                ),
            };
            let label = self.chord.label(self.minor, self.scale.is_some());
            chip(
                cx,
                g.chord_chip,
                &format!("Chord: {label}"),
                fill,
                chord_ink,
            );
        }
        let first = match g.chord_chip.is_empty() {
            true => g.scale_chip.x,
            false => g.chord_chip.x,
        };
        let left = if g.hidden_chip.is_empty() {
            first
        } else {
            let n = self.hidden(&g.rows);
            let text = if n == 1 {
                "1 note hidden".to_string()
            } else {
                format!("{n} notes hidden")
            };
            let fill = t.raised.mix(t.warn, 0.35);
            cx.canvas.fill_round_rect(g.hidden_chip, t.round(8), fill);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                g.hidden_chip,
                Align::Center,
                t.ink_on(fill),
                &text,
            );
            g.hidden_chip.x
        };
        if g.hidden_chip.is_empty() {
            self.paint_legend(cx, g, left);
        }
    }

    /// The legend, right of the title and left of `left` (the heading's
    /// chips): the whole of it, or without its count of notes, or none
    /// where neither fits.
    fn paint_legend(&self, cx: &mut PaintCx, g: &Geo, left: i32) {
        let t = cx.theme.clone();
        if !self.legend.is_empty() {
            let head = Rect::new(g.head.x, g.head.y, left - 12 - g.head.x, g.head.h);
            // Clear of the title: the whole legend, or without its count
            // of notes, or none on a narrow roll (the drawer open).
            let title = cx.measure(FontId::Bold, t.size_small, &self.title.to_uppercase());
            let room = head.w - title - 16;
            let short = self.legend.split_once(" · ").map_or("", |(_, rest)| rest);
            let legend = [self.legend.as_str(), short]
                .into_iter()
                .find(|l| cx.measure(FontId::Sans, t.size_small, l) <= room)
                .unwrap_or("");
            cx.text(
                FontId::Sans,
                t.size_small,
                head,
                Align::Right,
                t.text_dim,
                legend,
            );
        }
    }

    fn paint_roll(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let v = g.v;
        let rh = v.row_h;
        let was = cx.canvas.clip();
        // Rows: the black keys darker, C lines brighter; their names in
        // the key column. With the clip's scale on (plan 2B.1) its rows
        // are lit in place of the white keys, its roots the brightest with
        // a mark in the key column and the line under them.
        cx.canvas.set_clip(was.intersect(Rect::new(
            g.keys.x,
            g.roll.y,
            g.keys.w + g.roll.w,
            g.roll.h,
        )));
        let scale = self.scale.filter(|_| !self.pads);
        let lit = t.surface.mix(self.color, 0.10);
        let root_fill = t.surface.mix(self.color, 0.22);
        for row in v.notes_shown(g.roll, g.rows.n) {
            let Some(note) = g.rows.note_of(row) else {
                continue;
            };
            let y = v.row_top(g.roll, row);
            let (fill, key_ink, line) = match scale {
                Some(k) if k.is_root(i16::from(note)) => (root_fill, t.text, true),
                Some(k) if k.contains(i16::from(note)) => (lit, t.text_dim, false),
                // Faint, but never under the theme's floor for text.
                Some(_) => (t.bg, t.readable(t.text_faint, t.bg), false),
                // Pad rows (plan 4.2): a line under each bank's first pad,
                // as under each C.
                None if self.pads => (t.surface, t.text, note % 16 == 0),
                None => {
                    let black = matches!(note % 12, 1 | 3 | 6 | 8 | 10);
                    let c = note % 12 == 0;
                    let ink = if c { t.text } else { t.text_faint };
                    (if black { t.bg } else { t.surface }, ink, c)
                }
            };
            cx.canvas
                .fill_rect(Rect::new(g.roll.x, y, g.roll.w, rh), fill);
            if scale.is_some_and(|k| k.is_root(i16::from(note))) {
                cx.canvas
                    .fill_rect(Rect::new(g.keys.x, y + 2, 3, rh - 4), self.color);
            }
            // A pad's place and name read from the left, a note's name
            // from the keys' edge.
            let (key, align) = match self.pads {
                true => (Rect::new(g.keys.x + 8, y, g.keys.w - 14, rh), Align::Left),
                false => (Rect::new(g.keys.x, y, g.keys.w - 6, rh), Align::Right),
            };
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                key,
                align,
                key_ink,
                &self.row_name(note),
            );
            if line {
                cx.canvas
                    .fill_rect(Rect::new(g.roll.x, y + rh - 1, g.roll.w, 1), t.outline);
            }
            // Fold left rows out under this one: a bolder line says so.
            let below = row.checked_sub(1).and_then(|b| g.rows.note_of(b));
            if below.is_some_and(|b| b + 1 != note) {
                cx.canvas.fill_rect(
                    Rect::new(g.keys.x, y + rh - 2, g.keys.w + g.roll.w, 2),
                    t.text_faint,
                );
            }
        }
        // Lines: bars, beats, and the snap when they are far enough apart,
        // over the ticks in view only.
        cx.canvas.set_clip(was.intersect(g.roll));
        let beat = PPQ as Tick;
        let (from, to) = (
            v.tick_at(g.roll, g.roll.x),
            v.tick_at(g.roll, g.roll.right())
                .min(self.length.max(self.end_now())),
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
        // The clip's end, when it shows, and while its handle drags (plan
        // 1A.2) the new end in the handle's colour.
        if (from..=to).contains(&self.length) {
            let x = self.x_of(g, self.length);
            cx.canvas
                .fill_rect(Rect::new(x - 1, g.roll.y, 2, g.roll.h), t.text);
        }
        let end = self.end_now();
        if end != self.length && (from..=to).contains(&end) {
            let x = self.x_of(g, end);
            cx.canvas
                .fill_rect(Rect::new(x - 1, g.roll.y, 3, g.roll.h), t.accent2);
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
        // The clip's end handle (plan 1A.2): a grip as a picked note's.
        if let Some(e) = self.end_grip(g) {
            let grip = t.accent2.mix(t.raised, 0.45);
            cx.canvas.fill_round_rect(e.inset(2), 4, grip);
            cx.canvas.fill_rect(
                Rect::new(e.x + e.w / 2 - 1, e.y + e.h / 4, 2, e.h / 2),
                t.ink_on(grip),
            );
        }
        cx.canvas.set_clip(was);
        self.paint_overview(cx, g);
        cx.canvas.set_clip(was.intersect(g.roll));
        // Other clips behind (plan 4.1): faint, each in its track's
        // colour, under this clip's notes.
        self.paint_ghosts(cx, g);
        // The notes as they will play, then the lane under them.
        let top = t.accent;
        let shade = |v: u8| t.raised.mix(self.color, 0.35 + 0.65 * f32::from(v) / 127.0);
        let shown = self.in_view(g);
        let places = self.lock_places();
        // While notes move or a handle drags, where they were: an outline
        // under where they go (plan 1B.2).
        if let Some(d) = self.drag.filter(|d| (d.ticks, d.notes) != (0, 0)) {
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
            let n = &self.notes[i];
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
            // Off the clip's scale (plan 2B.1): a small mark, where the
            // note will land after a drag.
            let landed = match self.drag {
                Some(d) if self.moves(d, i) => self.dragged(d, n).map_or(n.note, |m| m.2),
                _ => n.note,
            };
            if scale.is_some_and(|k| !k.contains(i16::from(landed))) {
                let mark = match nr.w >= 14 {
                    true => Rect::new(nr.x + 4, nr.y + 4, 6, 6),
                    false => Rect::new(nr.x, nr.y, nr.w, 3),
                };
                cx.canvas.fill_round_rect(mark, 3.min(mark.h / 2), t.warn);
            }
            // A lock where it plays, and a condition or a chance under
            // 100 % (plan 3A.2): a small mark each in its top right
            // corner, the lock's a square, the other's a ring.
            let (lock, when) = match nr.w >= 20 && nr.h >= 12 {
                true => self.marks_in(i, &places),
                false => (false, false),
            };
            if lock || when {
                let ink = match n.on {
                    true => t.ink_on(fill),
                    false => t.text,
                };
                let mut x = nr.right() - 9;
                if lock {
                    cx.canvas.fill_rect(Rect::new(x, nr.y + 3, 6, 6), ink);
                    x -= 9;
                }
                if when {
                    cx.canvas
                        .stroke_round_rect(Rect::new(x, nr.y + 3, 6, 6), 3, 1, ink);
                }
            }
        }
        // A note being drawn with Draw on: where it goes when the finger
        // lifts.
        if let Hand::Pen { note, from, to, .. } = self.hand {
            for k in self.chord_on(note) {
                if let Some(y) = self.y_of(g, k) {
                    let x0 = self.x_of(g, from);
                    let x1 = self.x_of(g, to).max(x0 + 3);
                    let nr = Rect::new(x0, y + 1, x1 - x0, g.v.row_h - 2);
                    cx.canvas.fill_round_rect(nr, 3, shade(DRAW_VELOCITY));
                    cx.canvas.stroke_round_rect(nr, 3, 2, top);
                }
            }
        }
        // The end handle drawn in (plan 1A.2): what would be past the new
        // end shaded, notes that would run past it too.
        let end = self.end_now();
        if end < self.length {
            let (x0, x1) = (self.x_of(g, end), self.x_of(g, self.length));
            cx.canvas.fill_rect(
                Rect::new(x0, g.roll.y, x1 - x0, g.roll.h),
                t.bg.with_alpha(150),
            );
        }
        // An ERASE sweep (plan 1B.5): the notes it has crossed, marked to
        // come out when the finger lifts.
        if matches!(self.hand, Hand::Sweep { moved: true, .. }) {
            for &i in &self.swept {
                if let Some(nr) = self.note_rect_in(g, i) {
                    cx.canvas.fill_round_rect(nr, 3, t.surface.mix(t.bad, 0.35));
                    cx.canvas.stroke_round_rect(nr, 3, 2, t.bad);
                }
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
        // The lane, its chip and its top edge's grip (in the gap over it).
        cx.canvas.set_clip(was.intersect(Rect::new(
            g.keys.x,
            g.vel.y - GAP,
            g.keys.w + g.vel.w,
            g.vel.h + GAP,
        )));
        self.paint_lane(cx, g);
        let lines = was.intersect(Rect::new(
            g.roll.x,
            g.ruler.y,
            g.roll.w,
            g.roll.bottom() - g.ruler.y,
        ));
        // The cursor: a line in the note edit colour, with a mark on the
        // ruler, under the playhead.
        if let Some(c) = self.cursor {
            let x = self.x_of(g, c.rem_euclid(self.length.max(1)));
            cx.canvas.set_clip(lines);
            cx.canvas.fill_rect(
                Rect::new(x, g.ruler.y, 2, g.roll.bottom() - g.ruler.y),
                t.accent2,
            );
            cx.canvas
                .fill_rect(Rect::new(x - 5, g.ruler.y, 12, 6), t.accent2);
        }
        if let Some(p) = self.playhead {
            let x = self.x_of(g, p.rem_euclid(self.length.max(1)));
            cx.canvas.set_clip(lines);
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
    /// The Scale menu's places (plan 2B.1), over the roll from its top:
    /// the 12 roots in a row, then a button for each scale the editor
    /// offers, six across (more when they would not fit), and the panel
    /// round them.
    fn scale_menu_rects(&self, r: Rect) -> (Vec<Rect>, Vec<Rect>, Rect) {
        const PAD: i32 = 8;
        const H: i32 = 34;
        const GAP: i32 = 4;
        let g = self.geo(r);
        let area = Rect::new(
            g.keys.x,
            g.overview.y,
            g.roll.right() - g.keys.x,
            g.tools.y - 6 - g.overview.y,
        );
        let inner = area.inset(PAD);
        let rw = (inner.w - 11 * GAP) / 12;
        let roots: Vec<Rect> = (0..12)
            .map(|k| Rect::new(inner.x + k * (rw + GAP), inner.y, rw, H))
            .collect();
        let n = self.offers.len() as i32;
        let top = inner.y + H + 2 * PAD;
        let fit = ((inner.bottom() - top + GAP) / (H + GAP)).max(1);
        let cols = 6.max((n + fit - 1) / fit);
        let rows = (n + cols - 1) / cols;
        let cw = (inner.w - (cols - 1) * GAP) / cols;
        let scales = (0..n)
            .map(|i| {
                Rect::new(
                    inner.x + (i % cols) * (cw + GAP),
                    top + (i / cols) * (H + GAP),
                    cw,
                    H,
                )
            })
            .collect();
        let bottom = top + rows * (H + GAP) - GAP + PAD;
        (
            roots,
            scales,
            Rect::new(area.x, area.y, area.w, bottom - area.y),
        )
    }

    /// Other clips' notes, faint behind this one's (plan 4.1), and a
    /// label for each in the roll's top right corner: its name and Plays
    /// with or Reference, on a swatch of its colour.
    fn paint_ghosts(&self, cx: &mut PaintCx, g: &Geo) {
        if self.ghosts.is_empty() || self.lens != Lens::Grid {
            return;
        }
        let t = cx.theme.clone();
        let (from, to) = (
            g.v.tick_at(g.roll, g.roll.x),
            g.v.tick_at(g.roll, g.roll.right()) + 1,
        );
        for layer in &self.ghosts {
            let fill = t.bg.mix(layer.color, 0.30);
            let edge = t.bg.mix(layer.color, 0.60);
            for n in layer
                .notes
                .iter()
                .filter(|n| n.tick < to && n.tick + n.len > from)
            {
                if let Some(nr) = self.ghost_rect(g, n) {
                    cx.canvas.fill_round_rect(nr, 3, fill);
                    cx.canvas.stroke_round_rect(nr, 3, 1, edge);
                }
            }
        }
        // The labels, one a line from the top.
        const LINE: i32 = 20;
        for (k, layer) in self.ghosts.iter().enumerate() {
            let text = format!("{} · {}", layer.name, layer.kind_name());
            let w = cx.measure(FontId::Sans, t.size_small - 2.0, &text) + 30;
            let w = w.min(g.roll.w - 8);
            let tag = Rect::new(
                g.roll.right() - 4 - w,
                g.roll.y + 4 + k as i32 * (LINE + 2),
                w,
                LINE,
            );
            cx.canvas.fill_round_rect(tag, 6, t.bg);
            cx.canvas.fill_round_rect(
                Rect::new(tag.x + 8, tag.y + 6, 10, LINE - 12),
                2,
                layer.color,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(tag.x + 24, tag.y, tag.w - 28, tag.h),
                Align::Left,
                t.text_dim,
                &text,
            );
        }
    }

    /// The Show menu (plan 4.1): None, a line saying what shows, each
    /// offer named with Plays with or Reference, the ones showing lit in
    /// their track's colour, and More when they don't all fit.
    fn paint_show_menu(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let m = self.show_menu_rects(r);
        cx.canvas.fill_round_rect(m.panel, t.round(12), t.surface);
        cx.canvas
            .stroke_round_rect(m.panel, t.round(12), 1, t.outline);
        let none_fill = match self.ghosts.is_empty() {
            true => t.accent,
            false => t.raised,
        };
        cx.canvas.fill_round_rect(m.none, t.round(8), none_fill);
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            m.none,
            Align::Center,
            t.ink_on(none_fill),
            "None",
        );
        let mut line = match self.show_offers.is_empty() {
            true => "No other clip of this kind to show".to_string(),
            false => format!(
                "Up to {SHOW_MAX} clips show faintly behind this one. Plays with: the same scene, or playing now"
            ),
        };
        if m.pages > 1 {
            line = format!("{line} · page {} of {}", m.page + 1, m.pages);
        }
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            m.line,
            Align::Left,
            t.text_dim,
            &line,
        );
        for &(i, c) in &m.cells {
            let o = &self.show_offers[i];
            let fill = match o.shown {
                true => t.raised.mix(crate::view::track_color(o.track), 0.55),
                false => t.raised,
            };
            let ink = t.ink_on(fill);
            cx.canvas.fill_round_rect(c, t.round(8), fill);
            let (top, bottom) = c.inset(4).take_top((c.h - 8) / 2);
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                top,
                Align::Center,
                ink,
                &o.name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                bottom,
                Align::Center,
                ink,
                kind_name(o.with),
            );
        }
        if let Some(c) = m.more {
            cx.canvas.fill_round_rect(c, t.round(8), t.raised);
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                c,
                Align::Center,
                t.ink_on(t.raised),
                "More ›",
            );
        }
    }

    /// The Scale menu: the roots, the clip's lit (dim with no scale), and
    /// the scales, the clip's lit.
    fn paint_scale_menu(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (roots, scales, panel) = self.scale_menu_rects(r);
        cx.canvas.fill_round_rect(panel, t.round(12), t.surface);
        cx.canvas
            .stroke_round_rect(panel, t.round(12), 1, t.outline);
        let root = self.scale.map(|k| usize::from(k.root % 12));
        for (k, m) in roots.iter().enumerate() {
            let fill = match root {
                Some(rk) if rk == k => t.accent,
                _ => t.raised,
            };
            let ink = match root {
                None => t.text_faint,
                Some(_) => t.ink_on(fill),
            };
            cx.canvas.fill_round_rect(*m, t.round(8), fill);
            cx.text(
                FontId::Sans,
                t.size_small,
                *m,
                Align::Center,
                ink,
                NOTE_NAMES[k],
            );
        }
        let lit = self.offers.index_of(self.scale);
        for (i, m) in scales.iter().enumerate() {
            let fill = if lit == Some(i) { t.accent } else { t.raised };
            let name = self.offers.choice_name(i).unwrap_or_default();
            cx.canvas.fill_round_rect(*m, t.round(8), fill);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                *m,
                Align::Center,
                t.ink_on(fill),
                &name,
            );
        }
    }

    /// The Chord menu's buttons (plan 2B.2), each with its index after
    /// [`CHORD_MENU`]: Major and Minor first while the clip has no scale,
    /// then the shapes (Pads only while the pads have a scale), in one
    /// row over the roll; and its panel.
    fn chord_menu_items(&self, r: Rect) -> (Vec<(usize, Rect)>, Rect) {
        const PAD: i32 = 8;
        const H: i32 = 40;
        const GAP: i32 = 4;
        let g = self.geo(r);
        let area = Rect::new(
            g.keys.x,
            g.overview.y,
            g.roll.right() - g.keys.x,
            H + 2 * PAD,
        );
        let inner = area.inset(PAD);
        let mut ids: Vec<usize> = Vec::new();
        if self.scale.is_none() {
            ids.extend([CHORD_MAJOR, CHORD_MINOR]);
        }
        ids.extend(
            (0..ChordShape::ALL.len())
                .filter(|&i| ChordShape::ALL[i] != ChordShape::Pads || self.offers.pads.is_some()),
        );
        // A wider gap after Major and Minor.
        let split = if self.scale.is_none() { 3 * GAP } else { 0 };
        let n = ids.len() as i32;
        let w = (inner.w - (n - 1) * GAP - split) / n;
        let items = ids
            .into_iter()
            .enumerate()
            .map(|(k, id)| {
                let k = k as i32;
                let past = if id < CHORD_MAJOR { split } else { 0 };
                (id, Rect::new(inner.x + k * (w + GAP) + past, inner.y, w, H))
            })
            .collect();
        (items, area)
    }

    /// The name of the Chord menu's button `id`.
    fn chord_item_name(id: usize) -> &'static str {
        match id {
            CHORD_MAJOR => "Major",
            CHORD_MINOR => "Minor",
            i => ChordShape::ALL.get(i).map_or("", |s| s.name()),
        }
    }

    /// The Chord menu: Major and Minor, the one in use lit, then the
    /// shapes, the one in use lit.
    fn paint_chord_menu(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (items, panel) = self.chord_menu_items(r);
        cx.canvas.fill_round_rect(panel, t.round(12), t.surface);
        cx.canvas
            .stroke_round_rect(panel, t.round(12), 1, t.outline);
        for (id, m) in items {
            let lit = match id {
                CHORD_MAJOR => !self.minor,
                CHORD_MINOR => self.minor,
                i => ChordShape::ALL.get(i) == Some(&self.chord),
            };
            let fill = if lit { t.accent } else { t.raised };
            cx.canvas.fill_round_rect(m, t.round(8), fill);
            cx.text(
                FontId::Sans,
                t.size_small,
                m,
                Align::Center,
                t.ink_on(fill),
                NoteGrid::chord_item_name(id),
            );
        }
    }

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

    /// The lane under the roll (plan 2A.1): its chip in the key column
    /// (the lane's name; folded, a caret up after it), the ▾ that folds it,
    /// the grip on its top edge, and its bars as [`NoteGrid::lane_of`]
    /// makes them, the stroke's while a finger draws. On Velocity a
    /// ratchet's later hits stand beside their note's bar, thinner, at
    /// their own velocities.
    /// The open tool's bar (plan 2B.3): Done lit, the tool's own button
    /// lit while on, and the tool's name and what it shows between.
    fn paint_tool_bar(&self, cx: &mut PaintCx, r: Rect, tb: &ToolBar) {
        let t = cx.theme.clone();
        let (cancel, option, hint, done) = self.tool_rects(r);
        let button = |cx: &mut PaintCx, br: Rect, text: &str, lit: bool| {
            let (fill, ink) = match lit {
                true => (t.accent, t.on_accent),
                false => (t.raised, t.text),
            };
            cx.canvas.fill_round_rect(br, t.round(10), fill);
            cx.text(FontId::Bold, t.size_small, br, Align::Center, ink, text);
        };
        button(cx, cancel, "Cancel", false);
        if let (Some(o), Some((name, lit))) = (option, tb.option) {
            button(cx, o, name, lit);
        }
        button(cx, done, "Done", true);
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(hint.x + 8, hint.y, hint.w - 16, hint.h / 2 + 2),
            Align::Left,
            t.text,
            tb.name,
        );
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            Rect::new(hint.x + 8, hint.y + hint.h / 2 - 2, hint.w - 16, hint.h / 2),
            Align::Left,
            t.text_dim,
            &tb.hint,
        );
    }

    fn paint_lane(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let kind = self.stroke.as_ref().map_or(self.lane_kind(), |s| s.kind);
        let name = match kind {
            LaneKind::Value(Field::Velocity) => "Vel".to_string(),
            k => self.lane_name(k),
        };
        if !g.chip.is_empty() {
            // Folded, a caret up after the name says a tap opens it.
            let ink = t.ink_on(t.raised);
            let (text, caret) = match self.lane.folded {
                true => (Rect::new(g.chip.x, g.chip.y, g.chip.w - 12, g.chip.h), true),
                false => (g.chip, false),
            };
            cx.canvas.fill_round_rect(g.chip, 6, t.raised);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                text,
                Align::Center,
                ink,
                &name,
            );
            if caret {
                let y = g.chip.y + g.chip.h / 2;
                paint_caret(cx, g.chip.right() - 12, y, true, ink);
            }
        }
        if !g.fold.is_empty() {
            let (x, y) = (g.fold.x + g.fold.w / 2, g.fold.y + g.fold.h / 2);
            paint_caret(cx, x, y, false, t.text_dim);
        }
        cx.canvas.fill_round_rect(g.vel, 6, t.surface);
        if self.lane.folded {
            return;
        }
        if !g.edge.is_empty() {
            let grip = Rect::new(g.edge.x + g.edge.w / 2 - 20, g.vel.y - 4, 40, 3);
            cx.canvas.fill_round_rect(grip, 1, t.text_faint);
        }
        let body = lane_body(g);
        let (lane, bar_notes) = match (&self.stroke, kind) {
            (Some(st), _) => (st.lane.clone(), st.notes.clone()),
            (None, LaneKind::Pressure) => (self.pressure_lane(g).0, Vec::new()),
            (None, LaneKind::Value(f)) => self.lane_of(g, f),
            (None, LaneKind::Lock(a)) => self
                .lock_index(a)
                .map(|k| self.lock_lane(g, k))
                .unwrap_or_default(),
        };
        lane.paint(cx, body, self.color);
        let empty = match kind {
            LaneKind::Pressure => "Pick notes to draw their pressure",
            LaneKind::Lock(_) => "No notes in view use this setting",
            LaneKind::Value(_) => "",
        };
        if lane.bars.is_empty() && !empty.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                body,
                Align::Center,
                t.text_dim,
                empty,
            );
        }
        if let LaneKind::Lock(a) = kind {
            if let Some(card) = self.lock_card(&lane, &bar_notes, a) {
                cx.text(
                    FontId::Sans,
                    11.0,
                    Rect::new(body.x + 4, body.y, body.w - 8, 14),
                    Align::Right,
                    t.text_dim,
                    &card,
                );
            }
        }
        if kind != LaneKind::Value(Field::Velocity) {
            return;
        }
        for &i in &self.in_view(g) {
            let n = &self.notes[i];
            let hits = i64::from(n.ratchet.max(1));
            for k in 1..hits {
                let at = n.tick + n.len * k / hits;
                if at >= self.length {
                    break;
                }
                let v = mpfree_seq::ramped(n.velocity, n.ramp, k as u8, n.ratchet);
                let y = lane.y_of(i32::from(v), body);
                cx.canvas.fill_rect(
                    Rect::new(self.x_of(g, at), y, 2, (body.bottom() - y).max(1)),
                    self.color.mix(t.surface, 0.3),
                );
            }
        }
    }

    /// The lane for `field` as the roll shows it now (plan 2A.1): a bar
    /// for each place in view where notes start (starts closer than
    /// [`PLACE_PX`] share one, as a strum's do), as wide as the room to
    /// the next place (6 to 28 px), and the notes each bar sets: the
    /// picked ones there when any are, else all of them, so of notes that
    /// start together one picked is set alone. A bar's value is the
    /// highest of its notes' (on the ramp the farthest from 0); it is
    /// outlined when they are picked, and faint when every one is off
    /// (or, on the ramp, none repeats).
    fn lane_of(&self, g: &Geo, field: Field) -> (Lane, Vec<Vec<usize>>) {
        let (lo, hi) = field.range();
        let mut lane = Lane::new(lo, hi, 1);
        if field == Field::Ramp {
            lane.style = LaneStyle::Centred;
        }
        let mut starts: Vec<(Tick, usize)> = self
            .in_view(g)
            .into_iter()
            .filter_map(|i| {
                let n = self.notes.get(i)?;
                let x = self.x_of(g, n.tick);
                (x >= g.roll.x && x < g.roll.right()).then_some((n.tick, i))
            })
            .collect();
        starts.sort_unstable();
        let mut places: Vec<(Tick, Vec<usize>)> = Vec::new();
        for (t, i) in starts {
            match places.last_mut() {
                Some((at, all)) if self.x_of(g, t) - self.x_of(g, *at) < PLACE_PX => all.push(i),
                _ => places.push((t, vec![i])),
            }
        }
        let xs: Vec<i32> = places.iter().map(|(t, _)| self.x_of(g, *t)).collect();
        let mut bars = Vec::with_capacity(places.len());
        let mut notes = Vec::with_capacity(places.len());
        for (k, (_, all)) in places.into_iter().enumerate() {
            let picked: Vec<usize> = all
                .iter()
                .copied()
                .filter(|&i| self.notes[i].selected)
                .collect();
            let set = if picked.is_empty() { all } else { picked };
            let x = xs[k];
            let next = xs.get(k + 1).copied().unwrap_or(g.roll.right());
            let w = (next - x).clamp(6, 28).min(g.roll.right() - x).max(2);
            let boxes = || set.iter().map(|&i| &self.notes[i]);
            let value = boxes()
                .map(|n| box_value(field, n))
                .max_by_key(|v| if field == Field::Ramp { v.abs() } else { *v })
                .unwrap_or(lo);
            let ramp = boxes().next().map_or(0, |n| n.ramp);
            bars.push(Bar {
                x,
                w,
                value,
                picked: boxes().any(|n| n.selected),
                dim: boxes().all(|n| !n.on || (field == Field::Ramp && n.ratchet <= 1)),
                label: lane_label(field, value, ramp),
            });
            notes.push(set);
        }
        lane.set_bars(bars);
        (lane, notes)
    }

    /// The Pressure lane as the roll shows it now (plan 3A.1): a column
    /// every [`PRESSURE_COL`] px across the roll wherever a picked note
    /// sounds, as high as the highest of their pressures at its first
    /// tick (0 where it has none), faint where none of them has any;
    /// and each column's ticks, from its first to the next column's.
    /// No columns while nothing picked is in view.
    fn pressure_lane(&self, g: &Geo) -> (Lane, Vec<(Tick, Tick)>) {
        let mut lane = Lane::new(0, 127, 1);
        lane.style = LaneStyle::Line;
        let picked: Vec<usize> = self
            .in_view(g)
            .into_iter()
            .filter(|&i| self.notes.get(i).is_some_and(|n| n.selected))
            .collect();
        let (mut bars, mut cols) = (Vec::new(), Vec::new());
        let mut x = g.roll.x;
        while !picked.is_empty() && x < g.roll.right() {
            let w = PRESSURE_COL.min(g.roll.right() - x);
            let from = g.v.tick_at(g.roll, x).max(0);
            let to = g.v.tick_at(g.roll, x + w);
            let (mut value, mut any) = (None::<i32>, false);
            for &i in &picked {
                let n = &self.notes[i];
                if to <= from || n.tick >= to || n.tick + n.len <= from {
                    continue;
                }
                let at = (from.max(n.tick) - n.tick) as u32;
                let run = self.touch_of(i);
                any |= !run.is_empty();
                let v = run
                    .iter()
                    .rev()
                    .find(|p| p.at <= at)
                    .map_or(0, |p| i32::from(p.value));
                value = Some(value.map_or(v, |was| was.max(v)));
            }
            if let Some(value) = value {
                bars.push(Bar {
                    x,
                    w,
                    value,
                    dim: !any,
                    ..Bar::default()
                });
                cols.push((from, to));
            }
            x += w;
        }
        lane.set_bars(bars);
        (lane, cols)
    }

    /// Lock lane `k` as the roll shows it now (plan 3A.2): a bar for each
    /// place a lock of its setting sits for the notes starting in view (a
    /// step and a row, [`notes::lock_row`]), at its first note's start, as
    /// wide as the room to the next (6 to 28 px), as high as the place's
    /// lock among the setting's values (from the middle for a centred
    /// one), or faint at what the step plays when it has none; outlined
    /// while a note there is picked. Places that start together (closer
    /// than [`PLACE_PX`]) show one bar: the first with a picked note, else
    /// the first. And each bar's notes, the place's in view.
    fn lock_lane(&self, g: &Geo, k: usize) -> (Lane, Vec<Vec<usize>>) {
        let Some(s) = self.lock_settings.get(k) else {
            return Default::default();
        };
        let top = s.values.len() as i32 - 1;
        let z = if s.centred { s.index_of(0.0) as i32 } else { 0 };
        let mut lane = Lane::new(-z, top - z, 1);
        lane.every = true;
        if s.centred {
            lane.style = LaneStyle::Centred;
        }
        if s.values.is_empty() {
            return (lane, Vec::new());
        }
        type Place = (usize, Option<u8>);
        let mut places: Vec<(Tick, Place, Vec<usize>)> = Vec::new();
        for i in self.in_view(g) {
            let Some(n) = self.notes.get(i) else {
                continue;
            };
            let x = self.x_of(g, n.tick);
            if x < g.roll.x || x >= g.roll.right() {
                continue;
            }
            let Some(place) = self.lock_place(i, s.addr) else {
                continue;
            };
            match places.iter_mut().find(|p| p.1 == place) {
                Some(p) => {
                    p.0 = p.0.min(n.tick);
                    p.2.push(i);
                }
                None => places.push((n.tick, place, vec![i])),
            }
        }
        places.sort_by_key(|p| (p.0, p.1 .1));
        let picked = |all: &[usize]| all.iter().any(|&i| self.notes[i].selected);
        let mut shown: Vec<(Tick, Place, Vec<usize>)> = Vec::new();
        for (t, place, all) in places {
            match shown.last_mut() {
                Some(last) if self.x_of(g, t) - self.x_of(g, last.0) < PLACE_PX => {
                    if picked(&all) && !picked(&last.2) {
                        (last.1, last.2) = (place, all);
                    }
                }
                _ => shown.push((t, place, all)),
            }
        }
        let xs: Vec<i32> = shown.iter().map(|p| self.x_of(g, p.0)).collect();
        let (mut bars, mut notes) = (Vec::new(), Vec::new());
        for (j, (_, place, all)) in shown.into_iter().enumerate() {
            let x = xs[j];
            let next = xs.get(j + 1).copied().unwrap_or(g.roll.right());
            let lock = self.lock_at(place, s.addr);
            bars.push(Bar {
                x,
                w: (next - x).clamp(6, 28).min(g.roll.right() - x).max(2),
                value: s.index_of(lock.unwrap_or(s.own)) as i32 - z,
                picked: picked(&all),
                dim: lock.is_none(),
                label: String::new(),
            });
            notes.push(all);
        }
        lane.set_bars(bars);
        (lane, notes)
    }

    /// What a lock lane says at its top right (plan 3A.2), for the bar
    /// under the stroke or else the first picked one: its value in words,
    /// that it is not locked (what its step plays), or how many notes
    /// share the lock, since changing it changes them all.
    fn lock_card(&self, lane: &Lane, bar_notes: &[Vec<usize>], addr: RelAddr) -> Option<String> {
        let s = self.lock_settings.get(self.lock_index(addr)?)?;
        let stroke = self.stroke.as_ref().map(|st| st.x);
        let k = match stroke {
            Some(x) => lane.bar_at(x)?,
            None => lane.bars.iter().position(|b| b.picked)?,
        };
        let v = usize::try_from(lane.shown(k)? - lane.min).ok()?;
        let mut text = s.values.get(v)?.1.clone();
        if stroke.is_none() && lane.bars.get(k)?.dim {
            text.push_str(" · not locked");
            return Some(text);
        }
        let place = self.lock_place(*bar_notes.get(k)?.first()?, addr)?;
        let share = (0..self.notes.len())
            .filter(|&i| self.lock_place(i, addr) == Some(place))
            .count();
        if share > 1 {
            text.push_str(&format!(" · shared by {share} notes"));
        }
        Some(text)
    }

    /// While a note moves or a handle drags (plan 1B.2), what it will be,
    /// in words, above the finger: the moved note's position and row, or
    /// the start and the length a handle leaves.
    fn paint_readout(&self, cx: &mut PaintCx, g: &Geo) {
        let (text, now) = if let Hand::End { to, now, .. } = self.hand {
            // The end handle (plan 1A.2): the clip's new length.
            (self.end_name(to), now)
        } else if let Hand::Sweep {
            moved: true, now, ..
        } = self.hand
        {
            // An ERASE sweep (plan 1B.5): how many notes will come out.
            let text = match self.swept.len() {
                1 => "Erase 1 note".to_string(),
                n => format!("Erase {n} notes"),
            };
            (text, now)
        } else {
            let Some(d) = self.drag else {
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
                    self.row_name(note)
                ),
            };
            (text, d.now)
        };
        let t = cx.theme.clone();
        let size = t.size_small;
        let w = cx.measure(FontId::Sans, size, &text) + 20;
        let h = 30;
        // Above the finger, clear of it, kept on the roll.
        let x = (now.0 - w / 2).clamp(g.roll.x, (g.roll.right() - w).max(g.roll.x));
        let y = (now.1 - 64 - h).max(g.roll.y + 2);
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
        // Pads write and Sound, then Fit and Follow.
        self.paint_switches(cx, g);
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
        let place = |p: Tick| {
            self.grid()
                .place(self.length, p.rem_euclid(self.length.max(1)))
                .0
        };
        let playing = self.playhead.map(place);
        let cursor = self.cursor.map(place);
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
            // The cursor's step: its first line outlined.
            if row.first && cursor == Some(row.step) {
                cx.canvas.stroke_round_rect(rr, t.round(6), 2, t.accent2);
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
        let g = self.geo(r);
        // The legend goes left of the heading's chips.
        cx.titled_panel(r, &self.title, "");
        self.paint_head(cx, &g);
        match self.lens {
            Lens::Grid => self.paint_roll(cx, &g),
            Lens::List => self.paint_list(cx, r),
            Lens::Tracker => self.paint_tracker(cx, r),
        }
        if let Some(tb) = &self.tool_bar {
            self.paint_tool_bar(cx, r, tb);
        }
        for (k, br) in self.button_rects(r).iter().enumerate() {
            if self.tool_bar.is_some() {
                break;
            }
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
        if self.scale_menu && self.lens == Lens::Grid {
            self.paint_scale_menu(cx, r);
        }
        if self.chord_menu && self.lens == Lens::Grid {
            self.paint_chord_menu(cx, r);
        }
        if self.show_menu && self.lens == Lens::Grid {
            self.paint_show_menu(cx, r);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geo(r);
        let mut out: Vec<Target> = match &self.tool_bar {
            // A tool's bar (plan 2B.3): its buttons, then only what moves
            // the view.
            Some(tb) => {
                let (cancel, option, _, done) = self.tool_rects(r);
                let mut v = vec![Target::tap("tool-cancel", 0, "Cancel", cancel)];
                if let (Some(o), Some((name, _))) = (option, tb.option) {
                    v.push(Target::tap("tool-option", 0, name, o));
                }
                v.push(Target::tap("tool-done", 0, "Done", done));
                v
            }
            None => self
                .button_rects(r)
                .iter()
                .enumerate()
                .map(|(k, b)| Target::tap("button", k, self.button_label(k), *b))
                .collect(),
        };
        // The Scale menu covers the roll while it is open (plan 2B.1).
        if self.scale_menu && self.lens == Lens::Grid {
            let (roots, scales, _) = self.scale_menu_rects(r);
            for (k, m) in roots.into_iter().enumerate() {
                out.push(Target::tap("scale-root", k, NOTE_NAMES[k], m));
            }
            for (i, m) in scales.into_iter().enumerate() {
                let name = self.offers.choice_name(i).unwrap_or_default();
                out.push(Target::tap("scale-pick", i, name, m));
            }
            return out;
        }
        // So does the Chord menu (plan 2B.2).
        if self.chord_menu && self.lens == Lens::Grid {
            for (id, m) in self.chord_menu_items(r).0 {
                out.push(Target::tap(
                    "chord-pick",
                    id,
                    NoteGrid::chord_item_name(id),
                    m,
                ));
            }
            return out;
        }
        // The Select menu covers the rest while it is open.
        if self.menu {
            for (k, mr) in self.menu_rects(r).into_iter().enumerate() {
                out.push(Target::tap("select", k, SELECT_ITEMS[k], mr));
            }
            return out;
        }
        // So does the Show menu (plan 4.1), Show itself in sight.
        if self.show_menu && self.lens == Lens::Grid {
            let m = self.show_menu_rects(r);
            out.push(Target::tap("show-none", 0, "None", m.none));
            for (i, c) in m.cells {
                out.push(Target::tap("show-pick", i, &self.show_offers[i].name, c));
            }
            if let Some(c) = m.more {
                out.push(Target::tap("show-more", 0, "More", c));
            }
            return out;
        }
        if !g.show_chip.is_empty() {
            out.push(Target::tap("show-chip", 0, "Show", g.show_chip));
        }
        // Pads write and Sound (plan 1C.2).
        out.push(Target::tap("pads-write", 0, "Pads write", g.pads_chip));
        out.push(Target::tap(
            "sound-chip",
            0,
            self.hearing.name(),
            g.sound_chip,
        ));
        let tool = self.tool_bar.is_some();
        if tool && self.lens != Lens::Grid {
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
        if !g.scale_chip.is_empty() && !self.pads && !tool {
            out.push(Target::tap("scale-chip", 0, "Scale", g.scale_chip));
        }
        if !g.chord_chip.is_empty() && !tool {
            out.push(Target::tap("chord-chip", 0, "Chord", g.chord_chip));
        }
        if !g.fold_chip.is_empty() {
            out.push(Target::tap("fold-chip", 0, "Fold", g.fold_chip));
        }
        if !g.hidden_chip.is_empty() {
            out.push(Target::tap("hidden-chip", 0, "notes hidden", g.hidden_chip));
        }
        if !g.vel.is_empty() && !self.lane.folded {
            // From near the foot to near the top, so the bars it crosses
            // change.
            let body = lane_body(&g);
            let mut foot = Some(Rect::new(body.x, body.bottom() - 8, body.w, 6));
            let kind = self.lane_kind();
            if let LaneKind::Lock(a) = kind {
                // Up from the foot of its first bar.
                let lane = self.lock_index(a).map(|k| self.lock_lane(&g, k).0);
                foot = lane
                    .and_then(|l| l.bars.first().cloned())
                    .map(|b| Rect::new(b.x, body.bottom() - 8, b.w, 6));
            }
            if kind == LaneKind::Pressure {
                // Along the first run of the picked notes' columns, side
                // by side; with none, none.
                let lane = self.pressure_lane(&g).0;
                foot = lane.bars.first().map(|a| {
                    let mut end = a.x + a.w;
                    for b in lane.bars.iter().skip(1) {
                        if b.x != end {
                            break;
                        }
                        end = b.x + b.w;
                    }
                    Rect::new(a.x, body.bottom() - 8, end - a.x, 6)
                });
            }
            if let (false, Some(foot)) = (tool, foot) {
                out.push(
                    Target::drag("lane", 0, self.lane_name(kind), foot).towards(0, 10 - body.h),
                );
            }
            out.push(Target::drag("lane-edge", 0, "lane's top edge", g.edge).towards(0, -24));
        }
        if !g.chip.is_empty() {
            out.push(Target::tap("lane-chip", 0, "lane", g.chip));
        }
        // Another clip's faint notes (plan 4.1): with Draw off, the first
        // in sight of each, where no note of this clip covers it, opens it.
        if self.tool == Tool::Select && !tool {
            for (k, layer) in self.ghosts.iter().enumerate() {
                let free = layer.notes.iter().find_map(|n| {
                    let gr = self.ghost_rect(&g, n)?.intersect(g.roll);
                    let (x, y) = (gr.x + gr.w / 2, gr.y + gr.h / 2);
                    let mine = self
                        .note_under(&g, x, y)
                        .or_else(|| self.note_near(&g, x, y));
                    let first = self.ghost_under(&g, (x, y)) == Some(k);
                    (gr.w > 0 && gr.h > 0 && mine.is_none() && first).then_some(gr)
                });
                if let Some(gr) = free {
                    out.push(Target::tap("ghost-note", k, &layer.name, gr));
                }
            }
        }
        if !g.fold.is_empty() {
            out.push(Target::tap("lane-fold", 0, "fold the lane", g.fold));
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
        if tool {
            return out;
        }
        // The clip's end handle (plan 1A.2): dragged a unit in, to its
        // stop at most, while the clip can be shorter.
        if let Some(e) = self
            .end_grip(&g)
            .filter(|_| self.length > self.shortest && self.drag.is_none())
        {
            let unit = self.end_unit(g.v.ppt);
            let to = (self.length - unit).max(self.shortest);
            let dx = (self.x_of(&g, to) - self.x_of(&g, self.length)).min(-END_W);
            out.push(Target::drag("end", 0, "clip's end", e).towards(dx, 0));
        }
        let shown = self.in_view(&g);
        for &i in &shown {
            // The part on the roll: a note running past its edge is
            // tapped where it shows.
            let nr = self.note_rect_in(&g, i).map(|nr| nr.intersect(g.roll));
            if let Some(nr) = nr.filter(|nr| !nr.is_empty()) {
                out.push(Target::tap("note", i, format!("note {}", i + 1), nr));
            }
            // Its marks' corner opens their lane (plan 3A.2).
            if let Some(z) = self.mark_zone(&g, i) {
                out.push(Target::tap(
                    "note-mark",
                    i,
                    format!("note {} marks", i + 1),
                    z,
                ));
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

    /// A finger coming down on a note of the roll puts it under the data
    /// wheel (plan 1C.3, hold and turn): the note a press would take (the
    /// one on top, or with Draw off the one beside it), not a handle.
    /// None with ERASE held, a tool open or a menu over the roll, off the
    /// piano roll, or with a finger already down: one holding a note
    /// keeps the wheel while others tap notes into the picks.
    fn held(&self, x: i32, y: i32, r: Rect) -> Option<Target> {
        let busy = self.drag.is_some() || self.stroke.is_some() || self.hand != Hand::Idle;
        let covered = self.menu || self.scale_menu || self.chord_menu || self.show_menu;
        if busy || covered || self.erase || self.tool_bar.is_some() || self.lens != Lens::Grid {
            return None;
        }
        let g = self.geo(r);
        if !g.roll.contains(x, y) {
            return None;
        }
        let under = self.note_under(&g, x, y);
        if under.is_none() && self.handle_at(&g, x, y).is_some() {
            return None;
        }
        let i = under.or_else(|| self.note_near(&g, x, y))?;
        let nr = self
            .note_rect_in(&g, i)
            .map_or(Rect::new(x, y, 1, 1), |nr| nr.intersect(g.roll));
        Some(Target::tap("note", i, format!("note {}", i + 1), nr))
    }

    /// The wheel turned `delta` detents with a finger on note `t` (plan
    /// 1C.3): its velocity, or the picks' with it, as
    /// [`EditAction::Held`]. The press is a hold from the first detent:
    /// it drags nothing, and the hold's own pick never comes. The first
    /// detent names the note where it is now (or, held long enough to be
    /// picked and other notes tapped in, the picks); later ones only the
    /// hold. None once the finger is doing something else (a pinch).
    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, _r: Rect) -> Option<EventKind> {
        if t.id != "note" || delta == 0 {
            return None;
        }
        let index = match self.hand {
            Hand::Press {
                finger, index, now, ..
            } => {
                self.hand = Hand::Holding {
                    finger,
                    now,
                    tap: None,
                };
                index
            }
            Hand::Holding { .. } => HELD_PICKS,
            _ => return None,
        };
        let first = !self.turning;
        if first {
            self.turning = true;
            self.holds += 1;
            // Picked as the editor will pick it, until its picture comes.
            if self.notes.get(index).is_some_and(|n| !n.selected) {
                let add = self.add;
                for (k, n) in self.notes.iter_mut().enumerate() {
                    n.selected = k == index || (add && n.selected);
                }
            }
        }
        let mut flags = if fine { HELD_FINE } else { 0 };
        if first {
            flags |= HELD_FIRST;
        }
        Some(EventKind::Values {
            index: HELD_TURN + flags,
            set: vec![
                (if first { index } else { HELD_PICKS }, delta),
                (self.holds as usize, 0),
            ],
        })
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
        // A stroke along the lane is its finger's alone: its lift sends
        // what it changed, once, and another finger coming down calls it
        // off with nothing sent (and does nothing else).
        if let Some(st) = self.stroke.as_mut() {
            let done = st.lane.touch(f, phase, x, y, lane_body(&g));
            let LaneTouch::Done(changed) = done else {
                if done == LaneTouch::Preview {
                    st.x = x;
                }
                return None;
            };
            let st = self.stroke.take()?;
            let field = match st.kind {
                LaneKind::Value(field) => field,
                // Each column drawn by its first tick, then where the
                // stroke ends (plan 3A.1).
                LaneKind::Pressure => {
                    let &(last, _) = changed.last()?;
                    let end = st.cols.get(last)?.1;
                    let mut set: Vec<(usize, i32)> = changed
                        .into_iter()
                        .filter_map(|(b, v)| Some((st.cols.get(b)?.0 as usize, v)))
                        .collect();
                    set.push((end as usize, PRESSURE_END));
                    return Some(EventKind::Values {
                        index: PRESSURE_LANE,
                        set,
                    });
                }
                // Each bar it set, changed or not, by its first note,
                // with the place of its value among the setting's (plan
                // 3A.2): the app leaves a lock already there be, or with
                // ERASE held takes it off.
                LaneKind::Lock(a) => {
                    let index = LOCK_LANES + self.lock_index(a)?;
                    let set: Vec<(usize, i32)> = changed
                        .into_iter()
                        .filter_map(|(b, v)| Some((*st.notes.get(b)?.first()?, v - st.lane.min)))
                        .collect();
                    return (!set.is_empty()).then_some(EventKind::Values { index, set });
                }
            };
            let set: Vec<(usize, i32)> = changed
                .into_iter()
                .flat_map(|(b, v)| st.notes.get(b).into_iter().flatten().map(move |&i| (i, v)))
                .collect();
            let index = FIELDS.iter().position(|&f| f == field)?;
            return (!set.is_empty()).then_some(EventKind::Values { index, set });
        }
        // A finger on the lane's top edge: other fingers do nothing.
        if matches!(self.hand, Hand::Edge { finger, .. } if finger != f) {
            return None;
        }
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
                self.turning = false;
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
                            handle: None,
                            scale: self.scale.is_some() && !self.fine && !self.pads,
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
                    // The end handle: the snapped end under the finger, or
                    // at the roll's right edge at least the end there, going
                    // on with time.
                    Hand::End {
                        finger,
                        x0,
                        grab,
                        to,
                        edge,
                        span,
                        ..
                    } => {
                        let at_edge = x >= g.roll.right() - END_EDGE && x >= x0 + END_PUSH;
                        let under = self.end_at(&g, x + grab);
                        self.hand = Hand::End {
                            finger,
                            now: (x, y),
                            x0,
                            grab,
                            to: if at_edge { to.max(under) } else { under },
                            edge: at_edge.then(|| edge.unwrap_or(0.0)),
                            span,
                        };
                        return None;
                    }
                    // An ERASE sweep marks every note along its path,
                    // from where it went down once it is no tap.
                    Hand::Sweep {
                        finger,
                        at,
                        now,
                        moved,
                    } => {
                        let moved = moved || (x - at.0).abs().max((y - at.1).abs()) >= TAP_SLOP;
                        if moved {
                            let from = if self.swept.is_empty() { at } else { now };
                            self.sweep_along(&g, from, (x, y));
                        }
                        self.hand = Hand::Sweep {
                            finger,
                            at,
                            now: (x, y),
                            moved,
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
                    // The lane's top edge follows the finger.
                    Hand::Edge {
                        finger, y0, from, ..
                    } => {
                        self.lane.h = (from + y0 - y).clamp(LANE_MIN, LANE_MAX);
                        self.hand = Hand::Edge {
                            finger,
                            now: (x, y),
                            y0,
                            from,
                        };
                        return None;
                    }
                    _ => {}
                }
                let d = self.drag?;
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
                // Rows up or down, as Fold leaves them: the note on the row
                // under the finger, or with the clip's scale on the scale
                // note nearest it, counted in notes of the scale.
                let by = (d.y - y) / g.v.row_h.max(1);
                let notes = match g.rows.of(n.note) {
                    Some(r0) => {
                        let r1 = (i32::from(r0) + by).clamp(0, i32::from(g.rows.n) - 1);
                        let to = i32::from(g.rows.note_of(r1 as u8).unwrap_or(n.note));
                        match self.scale.filter(|_| d.scale) {
                            Some(k) => notes::scale_steps(i32::from(n.note), to, k),
                            None => to - i32::from(n.note),
                        }
                    }
                    None => by,
                };
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
                    // The lane keeps the height the roll gave it room for;
                    // a cancel puts it back.
                    Hand::Edge { from, .. } => {
                        self.hand = Hand::Idle;
                        self.lane.h = match up {
                            true => self.geo(r).vel.h,
                            false => from,
                        };
                        return Some(self.view_event());
                    }
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
                    // The end handle: the clip's new length, when it moved;
                    // the roll keeps its view inside it.
                    Hand::End { to, .. } => {
                        self.hand = Hand::Idle;
                        let to = if up { to } else { self.length };
                        self.view = self.view.clamped(g.roll, to.max(self.length), self.rows());
                        return (to != self.length).then_some(EventKind::Stretch {
                            index: CLIP_END,
                            to,
                        });
                    }
                    // ERASE: a tap erases the note under it (or with Draw
                    // off the one beside it), as a tap with ERASE always
                    // has; a faint note is another clip's, and the app
                    // says so. A sweep sends every note it crossed.
                    Hand::Sweep { at, moved, .. } => {
                        self.hand = Hand::Idle;
                        let swept = std::mem::take(&mut self.swept);
                        if !up {
                            return None;
                        }
                        if !moved {
                            let note = self
                                .note_under(&g, at.0, at.1)
                                .or_else(|| self.note_near(&g, at.0, at.1));
                            if let Some(i) = note {
                                return Some(EventKind::Toggle(i));
                            }
                            return self
                                .ghost_under(&g, at)
                                .map(|k| EventKind::Tab(GHOST_TAP + k));
                        }
                        return (!swept.is_empty()).then(|| EventKind::Values {
                            index: ERASE_SWEEP,
                            set: swept.into_iter().map(|i| (i, 0)).collect(),
                        });
                    }
                    // A tap on a note, unless a hold picked it. On its
                    // marks' corner, it opens their lane and picks the
                    // note (plan 3A.2): never the tap's take-out.
                    Hand::Press {
                        index, picked, at, ..
                    } => {
                        self.hand = Hand::Idle;
                        if !up || picked {
                            return None;
                        }
                        let lane = self
                            .mark_zone(&g, index)
                            .filter(|z| z.contains(at.0, at.1))
                            .and_then(|_| self.mark_lane(index));
                        if let Some(kind) = lane {
                            self.lane.kind = kind;
                            self.lane.folded = false;
                            return Some(EventKind::Select(index));
                        }
                        return Some(EventKind::Toggle(index));
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
                        // With Draw off, a tap on another clip's faint
                        // note opens that clip (plan 4.1).
                        if self.tool == Tool::Select {
                            if let Some(k) = self.ghost_under(&g, at) {
                                self.empty_tap = None;
                                self.ghost_cell = (note, tick);
                                return Some(EventKind::Tab(GHOST_TAP + k));
                            }
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
                if let Some(start) = d.handle {
                    let flag = if start { START_HANDLE } else { 0 };
                    return (d.ticks != 0).then_some(EventKind::Stretch {
                        index: d.index + flag,
                        to: d.ticks,
                    });
                }
                ((d.ticks, d.notes) != (0, 0)).then(|| EventKind::Marker {
                    index: d.index,
                    frame: drop_frame(d.ticks, d.notes, d.scale),
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

    /// What the data wheel turns with a finger on note `index` (plan
    /// 1C.3), for the hint line: once it has turned, the picks' velocity
    /// ("3 notes' velocity 80 to 104", the picks following the note
    /// through each new picture), else the note's own ("Velocity 96").
    pub fn held_velocity(&self, index: usize) -> Option<String> {
        let mut them: Vec<u8> = match self.turning {
            true => self
                .notes
                .iter()
                .filter(|n| n.selected)
                .map(|n| n.velocity)
                .collect(),
            false => Vec::new(),
        };
        if them.is_empty() {
            them.push(self.notes.get(index)?.velocity);
        }
        let (lo, hi) = (*them.iter().min()?, *them.iter().max()?);
        Some(match them.len() {
            1 => format!("Velocity {lo}"),
            n if lo == hi => format!("{n} notes' velocity {lo}"),
            n => format!("{n} notes' velocity {lo} to {hi}"),
        })
    }

    /// The clip's end handle is held at the roll's right edge (plan
    /// 1A.2): [`NoteGrid::grow_end`] takes the end on with time.
    pub fn end_at_edge(&self) -> bool {
        matches!(self.hand, Hand::End { edge: Some(_), .. })
    }

    /// Time passing, `dt` seconds, with the end handle held at the roll's
    /// right edge (plan 1A.2): the end goes on to the next bar (or step,
    /// [`NoteGrid::end_unit`]) every [`END_RATE`] seconds, never past
    /// `longest`, and the roll scrolls so it shows a unit from the edge.
    pub fn grow_end(&mut self, dt: f32) {
        let Hand::End {
            finger,
            now,
            x0,
            grab,
            to,
            edge: Some(t),
            span,
        } = self.hand
        else {
            return;
        };
        let unit = self.end_unit(self.view.ppt);
        let (mut t, mut to) = (t + dt.max(0.0), to);
        while t >= END_RATE {
            t -= END_RATE;
            to = ((to.div_euclid(unit) + 1) * unit).min(self.longest.max(self.length));
        }
        if to + unit > self.view.start + span {
            self.view.start = to + unit - span;
            self.view.follow = false;
        }
        self.hand = Hand::End {
            finger,
            now,
            x0,
            grab,
            to,
            edge: Some(t),
            span,
        };
    }

    /// The clip's end as the roll shows it: where its handle is dragging
    /// it (plan 1A.2), else its length.
    pub fn end_now(&self) -> Tick {
        match self.hand {
            Hand::End { to, .. } => to,
            _ => self.length,
        }
    }

    /// How much of the time line the roll may show: past the clip's end
    /// while its handle takes it on (plan 1A.2).
    fn shown_length(&self) -> Tick {
        match self.hand {
            Hand::End { to, .. } => self.length.max(to + self.end_unit(self.view.ppt)),
            _ => self.length,
        }
    }

    /// What the end handle snaps to at `ppt` pixels a tick (plan 1A.2): a
    /// step of the clip (its Steps' step) with SHIFT, or once a step is
    /// [`END_STEP_PX`] wide; else a bar.
    fn end_unit(&self, ppt: f64) -> Tick {
        let step = self.lock_step.max(1);
        if self.fine || step as f64 * ppt >= f64::from(END_STEP_PX) {
            step
        } else {
            self.bar.max(step)
        }
    }

    /// The end the handle gives at `x` (the finger plus its grab): the nearest
    /// [`NoteGrid::end_unit`], from `shortest` to `longest`.
    fn end_at(&self, g: &Geo, x: i32) -> Tick {
        let unit = self.end_unit(g.v.ppt);
        let t = g.v.tick_at(g.roll, x);
        let snapped = (t + unit / 2).div_euclid(unit) * unit;
        snapped.clamp(self.shortest.max(1), self.longest.max(self.shortest.max(1)))
    }

    /// The clip's end handle (plan 1A.2): a grip on the ruler just inside
    /// the end (or the end being dragged), while it shows.
    fn end_grip(&self, g: &Geo) -> Option<Rect> {
        if self.lens != Lens::Grid || g.ruler.is_empty() {
            return None;
        }
        let x = self.x_of(g, self.end_now());
        let grip = Rect::new(x - END_W, g.ruler.y, END_W, g.ruler.h).intersect(g.ruler);
        (grip.w >= END_W / 2).then_some(grip)
    }

    /// The end handle's readout (plan 1A.2): "Clip 2 bars · 32 steps", or
    /// "Clip 18 steps" off a bar line.
    fn end_name(&self, to: Tick) -> String {
        let (bar, step) = (self.bar.max(1), self.lock_step.max(1));
        let steps = to / step;
        let st = if steps == 1 { "step" } else { "steps" };
        match to % bar {
            0 => {
                let bars = to / bar;
                let s = if bars == 1 { "" } else { "s" };
                format!("Clip {bars} bar{s} · {steps} {st}")
            }
            _ => format!("Clip {steps} {st}"),
        }
    }

    /// Note `i` moves with drag `d`: the picked notes when the note
    /// dragged is one of them, else the note dragged alone.
    fn moves(&self, d: Drag, i: usize) -> bool {
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

    /// An ERASE sweep's finger went from `from` to `to`: every note in
    /// view its path crosses joins [`NoteGrid::swept`] (each once; under
    /// the one on top too, as an eraser rubs out what it passes over).
    fn sweep_along(&mut self, g: &Geo, from: (i32, i32), to: (i32, i32)) {
        let steps = (to.0 - from.0).abs().max((to.1 - from.1).abs()) / SWEEP_STEP + 1;
        let shown = self.in_view(g);
        let rects: Vec<(usize, Rect)> = shown
            .into_iter()
            .filter_map(|i| Some((i, self.note_rect_in(g, i)?)))
            .collect();
        for k in 0..=steps {
            let (x, y) = (
                from.0 + (to.0 - from.0) * k / steps,
                from.1 + (to.1 - from.1) * k / steps,
            );
            if !g.roll.contains(x, y) {
                continue;
            }
            for &(i, nr) in &rects {
                if nr.contains(x, y) && !self.swept.contains(&i) {
                    self.swept.push(i);
                }
            }
        }
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
            | Hand::Sweep { finger, now, .. }
            | Hand::End { finger, now, .. }
            | Hand::Holding { finger, now, .. }
            | Hand::Wait { finger, now, .. }
            | Hand::Scroll { finger, now, .. }
            | Hand::Edge { finger, now, .. }
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
            || self.drag.is_some_and(|d| d.handle.is_none())
            || self.tool_bar.is_some()
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

    /// Fold set to `fold` (plan 2B.1), in area `r`: the time shown stays,
    /// and the note in the roll's middle stays there (or the next row up
    /// that still shows); when every row fits, they fill the roll.
    fn refold(&mut self, r: Rect, fold: Fold) {
        let g = self.geo(r);
        let mid = g.roll.y + g.roll.h / 2;
        let at = self.note_at(&g, mid).unwrap_or(60);
        self.fold = fold;
        let g = self.geo(r);
        let rows = g.rows;
        let row = (at..128)
            .find_map(|n| rows.of(n))
            .or_else(|| (0..at).rev().find_map(|n| rows.of(n)))
            .unwrap_or(0);
        let mut v = g.v;
        let rh = v.row_h;
        if i32::from(rows.n) * zoom::ROW_OPEN <= g.roll.h {
            let shown = v.ticks_shown(g.roll);
            let ticks = (v.start, v.start + shown);
            v = v.fit(g.roll, self.length, rows.n, ticks, (0, rows.n - 1));
            v.start = ticks.0;
        } else {
            v.lift = mid - g.roll.bottom() + (i32::from(row) + 1) * rh - rh / 2;
        }
        self.view = v.clamped(g.roll, self.length, rows.n);
    }

    /// Fit: the picked notes when some are, else every note, across the
    /// roll and up it (an empty clip: the whole clip, the rows as they
    /// are).
    fn fit(&mut self, g: &Geo) {
        let picked = self.notes.iter().any(|n| n.selected);
        let which = || self.notes.iter().filter(move |n| !picked || n.selected);
        // The notes' rows, as Fold leaves them.
        let lo = which().filter_map(|n| g.rows.of(n.note)).min();
        let hi = which().filter_map(|n| g.rows.of(n.note)).max();
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
        let rows = g.rows.n;
        self.view = match (lo, hi) {
            (Some(lo), Some(hi)) => g.v.fit(g.roll, self.length, rows, (from, to), (lo, hi)),
            _ => g.v.zoom_time(g.roll, self.length, rows, 0.0, 0.0, g.roll.x),
        };
    }

    /// A finger coming down alone.
    fn down(&mut self, f: u64, x: i32, y: i32, r: Rect, g: &Geo) -> Option<EventKind> {
        // The Scale menu, while open (plan 2B.1): a root or a scale, and
        // it stays open for the other; a touch anywhere else closes it
        // and does nothing more.
        if self.scale_menu {
            let (roots, scales, _) = self.scale_menu_rects(r);
            if let Some(k) = roots.iter().position(|m| m.contains(x, y)) {
                return self
                    .scale
                    .is_some()
                    .then_some(EventKind::Tab(ROOT_MENU + k));
            }
            if let Some(i) = scales.iter().position(|m| m.contains(x, y)) {
                return Some(EventKind::Tab(SCALE_MENU + i));
            }
            self.scale_menu = false;
            return None;
        }
        // The Chord menu, while open (plan 2B.2): Major or Minor keeps it
        // open for a shape, a shape closes it; a touch anywhere else
        // closes it and does nothing more.
        if self.chord_menu {
            let item = self
                .chord_menu_items(r)
                .0
                .into_iter()
                .find(|(_, m)| m.contains(x, y));
            self.chord_menu = matches!(item, Some((CHORD_MAJOR | CHORD_MINOR, _)));
            return item.map(|(id, _)| EventKind::Tab(CHORD_MENU + id));
        }
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
        // The Show menu, while open (plan 4.1): None, an offer (it stays
        // open for more), More for the next page; a touch anywhere else
        // closes it and does nothing more.
        if self.show_menu {
            let m = self.show_menu_rects(r);
            if m.none.contains(x, y) {
                return Some(EventKind::Tab(SHOW_NONE));
            }
            if let Some(&(i, _)) = m.cells.iter().find(|(_, c)| c.contains(x, y)) {
                return Some(EventKind::Tab(SHOW_MENU + i));
            }
            if m.more.is_some_and(|c| c.contains(x, y)) {
                self.show_page = (m.page + 1) % m.pages;
                return None;
            }
            if !m.panel.contains(x, y) {
                self.show_menu = false;
            }
            return None;
        }
        // Show opens it (plan 4.1), a tool open or not: the roll's own.
        if g.show_chip.contains(x, y) {
            self.show_menu = true;
            self.show_page = 0;
            return None;
        }
        // Pads write and Sound (plan 1C.2), a tool open or not: Sound is
        // the screen's own.
        if g.pads_chip.contains(x, y) {
            return Some(EventKind::Tab(PADS_WRITE));
        }
        if g.sound_chip.contains(x, y) {
            self.hearing = self.hearing.next();
            return None;
        }
        // A tool's bar (plan 2B.3): Cancel, its own button and Done; the
        // roll and the lane only move the view, and the rest that would
        // edit is still.
        if self.tool_bar.is_some() {
            let (cancel, option, _, done) = self.tool_rects(r);
            let k = if cancel.contains(x, y) {
                Some(TOOL_CANCEL)
            } else if option.is_some_and(|o| o.contains(x, y)) {
                Some(TOOL_OPTION)
            } else if done.contains(x, y) {
                Some(TOOL_DONE)
            } else {
                None
            };
            if let Some(k) = k {
                return Some(EventKind::Tab(TOOL_BUTTONS + k));
            }
            if g.tools.contains(x, y)
                || g.scale_chip.contains(x, y)
                || g.chord_chip.contains(x, y)
                || self.lens != Lens::Grid
            {
                return None;
            }
        } else if let Some(k) = self.button_rects(r).iter().position(|b| b.contains(x, y)) {
            // Select opens its menu: the roll's own.
            if k == SELECT_BUTTON {
                self.menu = true;
                return None;
            }
            return Some(EventKind::Tab(k));
        }
        // The heading's chips (plan 2B.1, 2B.2): Scale and Chord open
        // their menus over the roll; they, Fold and the hidden notes' chip
        // are the roll's own.
        if g.scale_chip.contains(x, y) {
            self.scale_menu = !self.pads;
            self.menu = false;
            return None;
        }
        if g.chord_chip.contains(x, y) {
            self.chord_menu = true;
            self.menu = false;
            return None;
        }
        if g.fold_chip.contains(x, y) {
            let next = self.fold_now().next(self.scale.is_some() && !self.pads);
            self.refold(r, next);
            return Some(self.view_event());
        }
        if g.hidden_chip.contains(x, y) {
            self.refold(r, Fold::All);
            return Some(self.view_event());
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
        // The clip's end handle on the ruler (plan 1A.2), not while a tool
        // is open: its finger drags the clip's end.
        if self.tool_bar.is_none() && self.end_grip(g).is_some_and(|e| e.contains(x, y)) {
            self.hand = Hand::End {
                finger: f,
                now: (x, y),
                x0: x,
                grab: self.x_of(g, self.length) - x,
                to: self.length,
                edge: None,
                span: g.v.ticks_shown(g.roll),
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
        // The lane's chip steps to the next lane (or opens it, folded),
        // its ▾ folds it under, and its top edge drags its height: the
        // screen's own (plan 2A.1).
        if g.chip.contains(x, y) {
            if self.lane.folded {
                self.lane.folded = false;
            } else {
                let kinds = self.lane_kinds();
                let k = kinds.iter().position(|&l| l == self.lane_kind());
                self.lane.kind = kinds[k.map_or(0, |k| (k + 1) % kinds.len())];
            }
            return Some(self.view_event());
        }
        if g.fold.contains(x, y) {
            self.lane.folded = true;
            return Some(self.view_event());
        }
        if g.edge.contains(x, y) {
            self.hand = Hand::Edge {
                finger: f,
                now: (x, y),
                y0: y,
                from: g.vel.h,
            };
            return None;
        }
        // A tool open: a finger on the roll or the lane edits nothing, but
        // a second one joins it to move the view.
        if self.tool_bar.is_some() && (g.vel.contains(x, y) || g.roll.contains(x, y)) {
            self.hand = Hand::Spent {
                finger: f,
                now: (x, y),
            };
            return None;
        }
        if g.vel.contains(x, y) {
            // Drawn along as the finger moves, sent once when it lifts:
            // one edit, one UNDO. SHIFT on Repeats draws the ramp; the
            // Pressure lane draws only along picked notes.
            if self.lane.folded {
                return None;
            }
            let kind = match self.lane_kind() {
                LaneKind::Value(Field::Repeats) if self.fine => LaneKind::Value(Field::Ramp),
                k => k,
            };
            let (mut lane, notes, cols) = match kind {
                LaneKind::Value(field) => {
                    let (lane, notes) = self.lane_of(g, field);
                    (lane, notes, Vec::new())
                }
                LaneKind::Pressure => {
                    let (lane, cols) = self.pressure_lane(g);
                    (lane, Vec::new(), cols)
                }
                LaneKind::Lock(a) => {
                    let (lane, notes) = self
                        .lock_index(a)
                        .map(|k| self.lock_lane(g, k))
                        .unwrap_or_default();
                    (lane, notes, Vec::new())
                }
            };
            if !lane.bars.is_empty() {
                lane.touch(f, Touch::Down, x, y, lane_body(g));
                self.stroke = Some(Stroke {
                    kind,
                    lane,
                    notes,
                    cols,
                    x,
                });
            }
            return None;
        }
        if !g.roll.contains(x, y) {
            return None;
        }
        // ERASE held (plan 1B.5): a tap or a sweep, whatever the tool and
        // whatever is under the finger.
        if self.erase {
            self.swept.clear();
            self.hand = Hand::Sweep {
                finger: f,
                at: (x, y),
                now: (x, y),
                moved: false,
            };
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
                    handle: Some(start),
                    scale: false,
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
        // Empty roll: the cell under the finger, sent when it lifts (on
        // the scale's nearest note with the clip's scale on).
        let note = self.drawn_note(self.note_at(g, y)?);
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
    use mpfree_seq::mpfree_param::OnTrack;

    /// A clip of these notes: of pad hits while they are all on the
    /// first 16 pads, else of notes.
    fn clip(notes: &[(Tick, Tick, u8)]) -> Clip {
        let mut c = Clip::new("Bass", 4 * 960);
        for &(t, l, n) in notes {
            c.add_note(Note::new(t, l, n, 100)).unwrap();
        }
        if c.notes.iter().any(|n| n.note >= 16) {
            c.holds = mpfree_seq::Holds::Notes;
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
                scale: false,
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
                scale: false,
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
    /// ERASE + a drag (plan 1B.5): the notes it crossed come out as one
    /// edit, each once, whatever the tool; none, nothing; it names the
    /// picture it was made on.
    #[test]
    fn a_sweep_erases_the_notes_it_crossed_as_one_edit() {
        let mut c = clip(&[
            (0, 240, 60),
            (480, 240, 62),
            (960, 240, 64),
            (1440, 240, 65),
        ]);
        c.notes[2].velocity = 80;
        let mut ed = Editor::new(0, 0, &c);
        ed.sel = vec![1, 3];
        let sweep = |notes: Vec<usize>| EditAction::Sweep { notes, rev: 0 };
        let done = run(&mut ed, &mut c, sweep(vec![2, 0, 2, 9]));
        assert_eq!(done.line, "edit: 2 notes erased");
        assert_eq!(done.edit.erase.len(), 2);
        assert_eq!(
            ticks_of(&c, &(0..c.notes.len()).collect::<Vec<_>>()),
            [(480, 62), (1440, 65)]
        );
        assert_eq!(
            ticks_of(&c, &ed.sel),
            [(480, 62), (1440, 65)],
            "the other picks stay"
        );
        ed.act(EditAction::Button(2), &c);
        assert_eq!(ed.tool, Tool::Draw);
        let done = run(&mut ed, &mut c, sweep(vec![0]));
        assert_eq!(done.line, "edit: note D3 erased");
        assert_eq!(c.notes.len(), 1);
        assert!(ed.act(sweep(Vec::new()), &c).edit.is_empty());
        assert!(ed.act(sweep(vec![7]), &c).edit.is_empty());
        assert_eq!(
            EditAction::Sweep {
                notes: vec![0],
                rev: 4
            }
            .rev(),
            Some(4)
        );
    }

    /// Hold and turn (plan 1C.3): a hold's first detent picks its note
    /// (alone, or added with Add to pick; the picks stay when it is one
    /// of them, or when the hold picked it) and every detent turns the
    /// picks' velocity as Q4 does, SHIFT widening the gap, one take a
    /// hold; a later detent of a hold whose first never came turns
    /// nothing. Only the first names the picture it was made on.
    #[test]
    fn hold_and_turn_picks_the_note_and_turns_the_picks_one_take() {
        let mut c = clip(&[(0, 240, 60), (480, 240, 64), (960, 240, 67)]);
        c.notes[0].velocity = 80;
        c.notes[1].velocity = 100;
        c.notes[2].velocity = 60;
        let mut ed = Editor::new(0, 0, &c);
        let held = |index, ticks, fine, turn, first| EditAction::Held {
            index,
            rev: 0,
            ticks,
            fine,
            turn,
            first,
        };
        // A note changed goes to the end of the clip's notes: found by
        // its pitch.
        let at = |c: &Clip, p: u8| c.notes.iter().position(|n| n.note == p).unwrap();
        let vel = |c: &Clip, p: u8| c.notes[at(c, p)].velocity;
        let picked = |ed: &Editor, c: &Clip| {
            let mut p: Vec<u8> = ed.sel.iter().map(|&i| c.notes[i].note).collect();
            p.sort_unstable();
            p
        };
        let d = run(&mut ed, &mut c, held(1, 5, false, 1, true));
        assert_eq!(picked(&ed, &c), [64]);
        assert_eq!(vel(&c, 64), 105);
        let key = d.join;
        assert!(key.is_some());
        let d = run(&mut ed, &mut c, held(HELD_PICKS, 3, false, 1, false));
        assert_eq!(d.join, key, "the same hold, the same take");
        assert_eq!((vel(&c, 60), vel(&c, 64), vel(&c, 67)), (80, 108, 60));
        // Past 127: as far as it goes.
        run(&mut ed, &mut c, held(HELD_PICKS, 50, false, 1, false));
        assert_eq!(vel(&c, 64), 127);
        // A later detent of another hold, its first not seen: nothing.
        assert!(ed
            .act(held(HELD_PICKS, -5, false, 2, false), &c)
            .edit
            .is_empty());
        // A note among the picks: they all turn, the gap kept.
        ed.sel = vec![at(&c, 60), at(&c, 64)];
        let i = at(&c, 60);
        let d = run(&mut ed, &mut c, held(i, -7, false, 3, true));
        assert_ne!(d.join, key, "a new hold, a new take");
        assert_eq!(picked(&ed, &c), [60, 64]);
        assert_eq!((vel(&c, 60), vel(&c, 64)), (73, 120));
        // SHIFT: the gap wider, as Q4 with SHIFT.
        run(&mut ed, &mut c, held(HELD_PICKS, 1, true, 3, false));
        assert_eq!((vel(&c, 60), vel(&c, 64)), (67, 126));
        // A note held long enough to be picked: the picks as they are.
        run(&mut ed, &mut c, held(HELD_PICKS, 1, false, 4, true));
        assert_eq!(picked(&ed, &c), [60, 64]);
        assert_eq!((vel(&c, 60), vel(&c, 64)), (68, 127));
        // Another note: picked alone; with Add to pick, added.
        let i = at(&c, 67);
        run(&mut ed, &mut c, held(i, 2, false, 5, true));
        assert_eq!(picked(&ed, &c), [67]);
        assert_eq!(vel(&c, 67), 62);
        ed.add = true;
        let i = at(&c, 60);
        run(&mut ed, &mut c, held(i, 1, false, 6, true));
        assert_eq!(picked(&ed, &c), [60, 67]);
        assert_eq!((vel(&c, 60), vel(&c, 67)), (69, 63));
        // Nothing picked and no note named: nothing.
        ed.sel.clear();
        assert!(ed
            .act(held(HELD_PICKS, 1, false, 7, true), &c)
            .edit
            .is_empty());
        let rev = |first| {
            EditAction::Held {
                index: 0,
                rev: 9,
                ticks: 1,
                fine: false,
                turn: 1,
                first,
            }
            .rev()
        };
        assert_eq!((rev(true), rev(false)), (Some(9), None));
    }

    /// The clip's end handle (plan 1A.2): the Steps' length edit at the
    /// clip's step size, the nearest whole step, the picks kept; shorter
    /// to just after the last note start, a long note running on past
    /// it; a step shorter or past the longest refused with why.
    #[test]
    fn the_end_handle_lengthens_or_shortens_the_clip_as_the_steps_do() {
        let mut c = clip(&[(0, 240, 3), (960, 1920, 5)]);
        let mut ed = Editor::new(0, 0, &c);
        ed.sel = vec![1];
        let done = run(&mut ed, &mut c, EditAction::Length { to: 7680 });
        assert_eq!(done.line, "edit: the clip is 32 steps long");
        assert_eq!(c.length, 7680);
        assert_eq!(ticks_of(&c, &ed.sel), [(960, 5)], "the picks stay");
        run(&mut ed, &mut c, EditAction::Length { to: 1200 });
        assert_eq!(c.length, 1200);
        assert_eq!(c.notes.len(), 2, "the long note runs on past the end");
        let done = ed.act(EditAction::Length { to: 960 }, &c);
        assert!(done.edit.is_empty());
        assert_eq!(
            done.line,
            "edit: a note or lock starts on step 5: the clip can't end before it"
        );
        let far = mpfree_seq::MAX_CLIP_TICKS + 960;
        assert!(ed.act(EditAction::Length { to: far }, &c).edit.is_empty());
        // Between steps: the nearest; the same length: nothing.
        let done = ed.act(EditAction::Length { to: 1400 }, &c);
        assert_eq!(done.edit.length, Some((1200, 1440)));
        assert_eq!(ed.act(EditAction::Length { to: 1250 }, &c), Done::default());
        assert_eq!(EditAction::Length { to: 960 }.rev(), None);
    }

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

    /// The note inspector (plan 1C.1): Q1 to Q4 move, lengthen,
    /// transpose and turn the picks, SHIFT finer; one join key a turn
    /// whichever knob turns, a new turn a new key; – / + nudge as the
    /// Nudge row; the wheel moves the cursor round the clip.
    #[test]
    fn the_note_inspector_turns_the_picks_one_take_a_turn() {
        let mut c = clip(&[(0, 240, 60), (480, 240, 64), (1920, 240, 72)]);
        c.notes[0].velocity = 80;
        c.notes[1].velocity = 100;
        let mut ed = Editor::new(0, 0, &c);
        ed.snap = 6; // 1/16
        let knob = |k, ticks, fine, turn| EditAction::Knob {
            k,
            ticks,
            fine,
            turn,
        };
        // Nothing picked: nothing changes.
        let done = ed.act(knob(0, 1, false, 1), &c);
        assert!(done.edit.is_empty() && done.line.contains("pick notes first"));
        ed.sel = vec![0, 1];
        let d = run(&mut ed, &mut c, knob(0, 2, false, 1));
        let mut at: Vec<Tick> = c.notes.iter().map(|n| n.tick).collect();
        at.sort_unstable();
        assert_eq!(at, [480, 960, 1920]);
        let key = d.join;
        assert!(key.is_some());
        // Another knob in the same turn: the same take.
        let d = run(&mut ed, &mut c, knob(1, 1, false, 1));
        assert_eq!(d.join, key);
        assert_eq!(c.notes[ed.sel[0]].len, 480);
        let d = run(&mut ed, &mut c, knob(1, -1, true, 1));
        assert_eq!(d.join, key);
        assert_eq!(c.notes[ed.sel[0]].len, 476, "SHIFT: 4 ticks");
        // A new turn, a new take: semitones, the chord's shape kept.
        let d = run(&mut ed, &mut c, knob(2, 3, false, 2));
        assert_ne!(d.join, key);
        let mut pitches: Vec<u8> = ed.sel.iter().map(|&i| c.notes[i].note).collect();
        pitches.sort_unstable();
        assert_eq!(pitches, [63, 67]);
        assert!(
            ed.act(knob(2, 100, false, 2), &c).edit.is_empty(),
            "past 127"
        );
        // Velocity keeps the difference; SHIFT widens it.
        run(&mut ed, &mut c, knob(3, 10, false, 2));
        let vels = |ed: &Editor, c: &Clip| {
            let mut v: Vec<u8> = ed.sel.iter().map(|&i| c.notes[i].velocity).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(vels(&ed, &c), [90, 110]);
        run(&mut ed, &mut c, knob(3, 1, true, 2));
        assert_eq!(vels(&ed, &c), [87, 113]);
        // SHIFT on Q1: 4 ticks.
        run(&mut ed, &mut c, knob(0, -1, true, 3));
        let mut at: Vec<Tick> = ed.sel.iter().map(|&i| c.notes[i].tick).collect();
        at.sort_unstable();
        assert_eq!(at, [476, 956]);
        // – / +: a snap, as the Nudge row (onto the line first).
        let d = run(
            &mut ed,
            &mut c,
            EditAction::Nudge {
                steps: 1,
                fine: false,
            },
        );
        assert_eq!(
            d.join,
            ed.act(EditAction::Row { row: 6, delta: 0 }, &c).join
        );
        let mut at: Vec<Tick> = ed.sel.iter().map(|&i| c.notes[i].tick).collect();
        at.sort_unstable();
        assert_eq!(at, [480, 960]);
        // Snap Auto: whole snaps, each note keeping its feel.
        ed.snap = SNAP_AUTO;
        ed.auto = 6;
        run(&mut ed, &mut c, knob(0, -1, true, 4));
        run(&mut ed, &mut c, knob(0, 1, false, 4));
        let mut at: Vec<Tick> = ed.sel.iter().map(|&i| c.notes[i].tick).collect();
        at.sort_unstable();
        assert_eq!(at, [716, 1196]);
    }

    /// The cursor (plan 1C.1): a snap a detent, onto the line first, round
    /// the clip; on the tracker its line stays third from the top.
    #[test]
    fn the_wheel_moves_the_cursor_round_the_clip() {
        let c = clip(&[(0, 240, 60)]);
        let mut ed = Editor::new(0, 0, &c);
        ed.snap = 6;
        assert_eq!(ed.cursor, 0);
        assert_eq!(ed.act(EditAction::Cursor(2), &c), Done::default());
        assert_eq!(ed.cursor, 480);
        ed.act(EditAction::Cursor(-3), &c);
        assert_eq!(ed.cursor, 4 * 960 - 240, "round the clip");
        ed.cursor = 250;
        ed.act(EditAction::Cursor(-1), &c);
        assert_eq!(ed.cursor, 240, "onto the line first");
        ed.cursor = 250;
        ed.act(EditAction::Cursor(1), &c);
        assert_eq!(ed.cursor, 480);
        ed.lens = Lens::Tracker;
        ed.cursor = 0;
        ed.act(EditAction::Cursor(8), &c);
        assert_eq!((ed.cursor, ed.first), (1920, 6));
    }

    fn pad(key: u8, hit: u8, notes: &[u8], down: bool) -> EditAction {
        EditAction::Pad {
            key,
            hit,
            notes: notes.to_vec(),
            velocity: 90,
            down,
        }
    }

    /// Pads write (plan 1C.2): off at first, even on an empty clip; on, a
    /// pad writes at the cursor a snap long, pads held together are one
    /// chord with one join key, and the cursor steps once, when the last
    /// of them lets go; a pad hit again on the line changes its note. A
    /// clip of notes takes the pad map's notes. Off, a pad picks a clip of
    /// pad hits' notes and changes nothing on a clip of notes.
    #[test]
    fn pads_write_at_the_cursor_and_a_chord_is_one_step_and_one_take() {
        let mut c = clip(&[]);
        let mut ed = Editor::new(0, 0, &c);
        ed.snap = 6;
        assert!(!ed.pads_write, "off, even on an empty clip");
        assert!(run(&mut ed, &mut c, pad(0, 36, &[], true)).edit.is_empty());
        ed.act(pad(0, 36, &[], false), &c);
        assert!(c.notes.is_empty() && ed.cursor == 0, "the pads only play");
        let done = run(&mut ed, &mut c, EditAction::PadsWrite);
        assert!(ed.pads_write && done.line.contains("on"), "{}", done.line);
        // Two pads held together: one chord, one key, the cursor still.
        let a = run(&mut ed, &mut c, pad(0, 36, &[], true));
        let b = run(&mut ed, &mut c, pad(1, 38, &[], true));
        assert_eq!(a.join, b.join);
        assert!(a.join.is_some());
        assert_eq!(
            ticks_of(&c, &ed.sel),
            [(0, 36), (0, 38)],
            "the chord picked"
        );
        assert_eq!(c.notes[0].len, 240);
        assert_eq!(c.notes[0].velocity, 90);
        ed.act(pad(0, 36, &[], false), &c);
        assert_eq!(ed.cursor, 0, "a pad still held");
        ed.act(pad(1, 38, &[], false), &c);
        assert_eq!(ed.cursor, 240, "the last let go: a snap on");
        // The next hit is the next chord, on the next line.
        let next = run(&mut ed, &mut c, pad(2, 42, &[], true));
        assert_ne!(next.join, a.join);
        ed.act(pad(2, 42, &[], false), &c);
        assert_eq!((c.notes.len(), ed.cursor), (3, 480));
        assert_eq!(ticks_of(&c, &ed.sel), [(240, 42)]);
        // A let-go of a pad that never wrote moves nothing.
        ed.act(pad(5, 40, &[], false), &c);
        assert_eq!(ed.cursor, 480);
        // Hit again on the first line: its note changes, not doubles.
        ed.cursor = 0;
        let mut soft = pad(0, 36, &[], true);
        if let EditAction::Pad { velocity, .. } = &mut soft {
            *velocity = 40;
        }
        run(&mut ed, &mut c, soft);
        ed.act(pad(0, 36, &[], false), &c);
        assert_eq!(c.notes.len(), 3);
        assert!(c.notes.iter().any(|n| n.note == 36 && n.velocity == 40));
        // Off again: a pad picks every note of its own.
        run(&mut ed, &mut c, EditAction::PadsWrite);
        ed.cursor = 720;
        run(&mut ed, &mut c, EditAction::PadsWrite);
        run(&mut ed, &mut c, pad(0, 36, &[], true));
        ed.act(pad(0, 36, &[], false), &c);
        run(&mut ed, &mut c, EditAction::PadsWrite);
        let done = ed.act(pad(0, 36, &[], true), &c);
        assert!(done.edit.is_empty());
        assert_eq!(ticks_of(&c, &ed.sel), [(0, 36), (720, 36)], "{}", done.line);
        // (The app follows the picks only after an edit, as `run` does
        // for every touch: a let-go goes by `act` alone here.)
        // A clip of notes: the pad map's notes, a chord pad's chord.
        let mut n = clip(&[]);
        n.holds = Holds::Notes;
        let mut ed = Editor::new(0, 0, &n);
        ed.snap = 6;
        run(&mut ed, &mut n, EditAction::PadsWrite);
        run(&mut ed, &mut n, pad(3, 3, &[60, 64, 67], true));
        let mut pitches: Vec<u8> = n.notes.iter().map(|x| x.note).collect();
        pitches.sort_unstable();
        assert_eq!(pitches, [60, 64, 67]);
        // Its chord is the pad's trigger (#827), as when it is recorded:
        // it plays whole or not at all. A drum clip's hits have none.
        let g = mpfree_seq::groups::pad_group(3);
        assert!(n.notes.iter().all(|x| x.group == g));
        assert!(c.notes.iter().all(|x| x.group == 0));
        let none = run(&mut ed, &mut n, pad(4, 4, &[], true));
        assert!(none.edit.is_empty() && none.line.contains("no notes"));
        ed.act(pad(3, 3, &[], false), &n);
        ed.act(pad(4, 4, &[], false), &n);
        assert_eq!(ed.cursor, 240);
        // Off, a pad on a clip of notes changes nothing.
        run(&mut ed, &mut n, EditAction::PadsWrite);
        let done = run(&mut ed, &mut n, pad(3, 3, &[60], true));
        assert_eq!(done, Done::default());
    }

    /// What a touch asks to hear (plan 1C.2): a note picked, notes drawn
    /// or moved, each touch once; a box, a velocity turn or a pad's write
    /// hears nothing.
    #[test]
    fn a_note_picked_drawn_or_moved_is_heard_once() {
        let mut c = clip(&[(0, 240, 60), (480, 240, 64)]);
        let mut ed = Editor::new(0, 0, &c);
        ed.snap = 6;
        let n0 = ed.heard.n;
        run(&mut ed, &mut c, EditAction::Tap { index: 1, rev: 0 });
        assert_eq!(ed.heard.n, n0 + 1);
        assert_eq!(ed.heard.notes, [c.notes[1]]);
        // Moved up a note: heard where it landed.
        run(
            &mut ed,
            &mut c,
            EditAction::Drop {
                index: 1,
                rev: 0,
                ticks: 0,
                notes: 1,
                scale: false,
            },
        );
        assert_eq!(ed.heard.n, n0 + 2);
        assert_eq!(ed.heard.notes[0].note, 65);
        // Drawn: heard.
        ed.tool = Tool::Draw;
        run(
            &mut ed,
            &mut c,
            EditAction::Empty {
                tick: 960,
                note: 67,
            },
        );
        assert_eq!((ed.heard.n, ed.heard.notes[0].note), (n0 + 3, 67));
        // A box, a velocity turn and a pad's write: nothing more.
        run(
            &mut ed,
            &mut c,
            EditAction::Pick {
                from: 0,
                to: 2000,
                low: 0,
                high: 127,
            },
        );
        run(
            &mut ed,
            &mut c,
            EditAction::Knob {
                k: 3,
                ticks: 2,
                fine: false,
                turn: 1,
            },
        );
        run(&mut ed, &mut c, EditAction::PadsWrite);
        run(&mut ed, &mut c, pad(0, 9, &[], true));
        assert_eq!(ed.heard.n, n0 + 3);
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
        // A lane's stroke: each note its value, the picks kept on them.
        let set = ed.sel.iter().map(|&i| (i, 127)).collect();
        let lane = EditAction::Lane {
            field: Field::Velocity,
            rev: 0,
            set,
        };
        let d = run(&mut ed, &mut c, lane);
        assert!(c.notes.iter().all(|n| n.velocity == 127));
        assert_eq!((d.join, ed.sel.len(), ed.velocity), (None, 2, 127));
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
        assert_eq!(done.line, "edit: Pad A2 drawn at tick 480");
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
        assert_eq!(rows[EUCLID_ROW].value, "Pad A2");
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
        assert_eq!(done.line, "edit: euclid on Pad A3");
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
                    addr: RelAddr::new(OnTrack::Pad(0), 3),
                    value: 0.5,
                },
                // A trigless lock on an empty step.
                LockMark {
                    tick: 480,
                    sound: None,
                    addr: RelAddr::new(OnTrack::Inst, 3),
                    value: 0.5,
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
        assert_eq!(cells(0).0[..2], ["1.1.1".to_string(), "A1".to_string()]);
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
        // The lane: a second finger stops its stroke, and nothing is sent.
        let v = g.geo(R).vel;
        assert_eq!(g.touch(4, Touch::Down, v.x + 2, v.y + 20, R), None);
        assert!(g.stroke.is_some());
        assert_eq!(g.touch(5, Touch::Down, v.x + 40, v.y + 3, R), None);
        assert_eq!(g.stroke, None);
        assert_eq!(g.touch(4, Touch::Move, v.x + 10, v.bottom(), R), None);
        assert!(!matches!(
            g.touch(4, Touch::Up, v.x + 10, v.bottom(), R),
            Some(EventKind::Values { .. })
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
            for scale in [false, true] {
                assert_eq!(drop_of(drop_frame(t, n, scale)), (t, n, scale));
            }
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
    /// ERASE held (plan 1B.5), Draw on or off: a tap erases the note under
    /// it (a hold is still a tap) and a tap on empty roll does nothing; a
    /// drag sweeps, marking every note its path crosses, each once, even
    /// from one fast move, and sends them as it lifts, following them to
    /// where a new picture has them. A second finger calls it off with
    /// nothing sent, and a sweep that crosses no note sends nothing.
    #[test]
    fn erase_held_taps_a_note_out_or_sweeps_every_note_it_crosses() {
        let mut g = grid();
        g.notes
            .push(NoteBox::of(&Note::new(1920, 240, 50, 90), false));
        g.erase = true;
        let (n0, n1, n2) = (
            g.note_rect(R, 0).unwrap(),
            g.note_rect(R, 1).unwrap(),
            g.note_rect(R, 2).unwrap(),
        );
        let (x1, y1) = mid(n1);
        assert_eq!(g.touch(1, Touch::Down, x1, y1, R), None);
        assert_eq!(g.tick(0.5), None, "a hold picks nothing");
        assert_eq!(g.touch(1, Touch::Up, x1, y1, R), Some(EventKind::Toggle(1)));
        g.tool = Tool::Draw;
        g.touch(2, Touch::Down, x1, y1, R);
        assert_eq!(g.touch(2, Touch::Up, x1, y1, R), Some(EventKind::Toggle(1)));
        let empty = (n2.right() + 80, y1);
        g.touch(3, Touch::Down, empty.0, empty.1, R);
        assert_eq!(
            g.touch(3, Touch::Up, empty.0, empty.1, R),
            None,
            "no note goes in"
        );
        // One fast move along the bottom row: the first and the third
        // notes, not the second a row up; marked, with a readout.
        let before = painted(&g);
        let (x0, y0) = mid(n0);
        let x2 = mid(n2).0;
        g.touch(4, Touch::Down, x0, y0, R);
        assert_eq!(g.touch(4, Touch::Move, x2, y0, R), None);
        assert_eq!(g.swept, [0, 2]);
        assert!(differ_in(&before, &painted(&g), g.geo(R).roll));
        // A new picture with the notes in another order: the sweep
        // follows them.
        let mut other = g.clone();
        other.notes.swap(0, 2);
        other.erase = false;
        g.show(other);
        assert!(g.erase, "kept from the finger's down");
        assert_eq!(g.swept, [2, 0]);
        // Up to the second note, crossing the others again.
        g.touch(4, Touch::Move, x0, y0, R);
        g.touch(4, Touch::Move, x1, y1, R);
        assert_eq!(
            g.touch(4, Touch::Up, x1, y1, R),
            Some(EventKind::Values {
                index: ERASE_SWEEP,
                set: vec![(2, 0), (0, 0), (1, 0)]
            })
        );
        assert!(g.swept.is_empty());
        assert!(
            !differ_in(&before, &painted(&g), g.geo(R).roll),
            "nothing marked after"
        );
        // A second finger: a pinch, and nothing erased.
        g.touch(5, Touch::Down, x0, y0, R);
        g.touch(5, Touch::Move, x2, y0, R);
        g.touch(6, Touch::Down, x1, y1 - 100, R);
        let sent: Vec<_> = [
            g.touch(5, Touch::Up, x2, y0, R),
            g.touch(6, Touch::Up, x1, y1 - 100, R),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert!(
            !sent
                .iter()
                .any(|e| matches!(e, EventKind::Values { .. } | EventKind::Toggle(_))),
            "{sent:?}"
        );
        // Along an empty row: nothing.
        let row = (n0.x + 4, y1 - 120);
        g.touch(7, Touch::Down, row.0, row.1, R);
        g.touch(7, Touch::Move, row.0 + 400, row.1, R);
        assert_eq!(g.touch(7, Touch::Up, row.0 + 400, row.1, R), None);
        // ERASE let go: a drag on a note moves it again.
        g.erase = false;
        g.tool = Tool::Select;
        g.touch(8, Touch::Down, x1, y1, R);
        g.touch(8, Touch::Move, x1 + 2 * TAP_SLOP, y1, R);
        assert!(matches!(
            g.touch(8, Touch::Up, x1 + 2 * TAP_SLOP, y1, R),
            Some(EventKind::Marker { index: 1, .. })
        ));
    }

    /// A finger on a note puts it under the data wheel (plan 1C.3): the
    /// note on top, not with ERASE held, a menu over the roll, a handle
    /// or empty roll under it, or another finger down. A turn sends the
    /// note on its first detent and only the hold after, numbered a hold
    /// each; from the first the press is a hold, which a new picture
    /// keeps, picks nothing more, moves nothing and sends nothing as it
    /// lifts. A hold that picked its note (and tapped more in) names the
    /// picks; a pinch turns nothing.
    #[test]
    fn a_finger_on_a_note_puts_its_velocity_under_the_wheel() {
        let mut g = grid();
        let (n0, n1) = (g.note_rect(R, 0).unwrap(), g.note_rect(R, 1).unwrap());
        let ((x0, y0), (x1, y1)) = (mid(n0), mid(n1));
        let t = g.held(x1, y1, R).expect("the note is held");
        assert_eq!((t.id, t.index), ("note", 1));
        assert_eq!(g.held(x1, y1 - 200, R), None, "empty roll");
        g.erase = true;
        assert_eq!(g.held(x1, y1, R), None, "ERASE held");
        g.erase = false;
        g.menu = true;
        assert_eq!(g.held(x1, y1, R), None, "the Select menu");
        g.menu = false;
        let turn = |first: bool, fine: bool, note: usize, by: i32, hold: usize| {
            let mut index = HELD_TURN;
            if first {
                index += HELD_FIRST;
            }
            if fine {
                index += HELD_FINE;
            }
            Some(EventKind::Values {
                index,
                set: vec![(note, by), (hold, 0)],
            })
        };
        g.notes[0].selected = true;
        g.notes[1].selected = false;
        g.touch(1, Touch::Down, x1, y1, R);
        assert_eq!(g.held(x0, y0, R), None, "a finger is down");
        assert_eq!(g.held_velocity(1).as_deref(), Some("Velocity 64"));
        assert_eq!(g.held_step(&t, 2, false, R), turn(true, false, 1, 2, 1));
        assert!(
            g.notes[1].selected && !g.notes[0].selected,
            "picked alone at once"
        );
        assert_eq!(g.tick(1.0), None, "the hold's pick never comes");
        assert_eq!(
            g.held_step(&t, -1, true, R),
            turn(false, true, HELD_PICKS, -1, 1)
        );
        // A new picture, the note's velocity changed: still held.
        let mut other = g.clone();
        other.notes[1].velocity = 101;
        g.show(other);
        assert_eq!(g.held_velocity(1).as_deref(), Some("Velocity 101"));
        assert_eq!(
            g.held_step(&t, 1, false, R),
            turn(false, false, HELD_PICKS, 1, 1)
        );
        assert_eq!(g.touch(1, Touch::Move, x1 + 3 * TAP_SLOP, y1, R), None);
        assert_eq!(g.touch(1, Touch::Cancel, x1 + 3 * TAP_SLOP, y1, R), None);
        assert_eq!(g.held_step(&t, 1, false, R), None, "lifted");
        // The next finger: a new hold, its first detent naming its note.
        let t0 = g.held(x0, y0, R).unwrap();
        g.touch(2, Touch::Down, x0, y0, R);
        assert_eq!(g.held_step(&t0, 1, false, R), turn(true, false, 0, 1, 2));
        assert_eq!(g.touch(2, Touch::Up, x0, y0, R), None, "no tap");
        // Held long enough to pick it, another note tapped in: the picks.
        g.touch(3, Touch::Down, x0, y0, R);
        assert_eq!(g.tick(1.0), Some(EventKind::Select(0)));
        g.touch(4, Touch::Down, x1, y1, R);
        assert_eq!(
            g.touch(4, Touch::Up, x1, y1, R),
            Some(EventKind::Cell { row: ADD, col: 1 })
        );
        let mut other = g.clone();
        other.notes[0].selected = true;
        other.notes[1].selected = true;
        other.notes[1].velocity = 90;
        g.show(other);
        assert_eq!(
            g.held_step(&t0, 3, false, R),
            turn(true, false, HELD_PICKS, 3, 3)
        );
        assert_eq!(
            g.held_velocity(0).as_deref(),
            Some("2 notes' velocity 90 to 100")
        );
        g.touch(3, Touch::Up, x0, y0, R);
        // Two fingers: a pinch, which turns nothing.
        g.touch(5, Touch::Down, x0, y0, R);
        g.touch(6, Touch::Down, x1, y1 - 100, R);
        assert_eq!(g.held_step(&t0, 1, false, R), None);
        g.touch(5, Touch::Up, x0, y0, R);
        g.touch(6, Touch::Up, x1, y1 - 100, R);
        // Draw on: the same.
        g.tool = Tool::Draw;
        g.touch(7, Touch::Down, x1, y1, R);
        assert_eq!(g.held_step(&t, 1, false, R), turn(true, false, 1, 1, 4));
        assert_eq!(g.touch(7, Touch::Cancel, x1, y1, R), None);
    }

    /// The clip's end handle on the ruler (plan 1A.2): dragged in it
    /// snaps to bars, with SHIFT to steps, and stops just after the last
    /// note start, a readout saying the length, sent as it lifts; held at
    /// the roll's right edge the end goes on a bar every `END_RATE` and the
    /// roll scrolls past the old end with it; a second finger calls it off,
    /// and a drag back to where it was sends nothing. Swept as a control
    /// while the clip can be shorter.
    #[test]
    fn the_clip_end_handle_drags_the_length_by_bars_or_steps() {
        let bar = 3840;
        let mut g = grid();
        g.length = 4 * bar;
        g.lock_step = 240;
        g.shortest = 5 * 240;
        // The whole clip across.
        g.view.ppt = 0.0;
        g.view = g.view.clamped(g.geo(R).roll, g.length, g.rows());
        let g0 = g.geo(R);
        let e = g.end_grip(&g0).expect("the end shows");
        let (ex, ey) = mid(e);
        // Where the finger goes for the end to be at `t`: it keeps its
        // place on the grip.
        let grab = g.x_of(&g0, g.length) - ex;
        let at = |g: &NoteGrid, t: Tick| g.x_of(&g0, t) - grab;
        assert!(g.targets(R).iter().any(|t| t.id == "end"));
        // Three bars: the readout says so; then past the stop.
        let before = painted(&g);
        assert_eq!(g.touch(1, Touch::Down, ex, ey, R), None);
        g.touch(1, Touch::Move, at(&g, 3 * bar), ey, R);
        assert_eq!(g.end_now(), 3 * bar);
        assert!(differ_in(&before, &painted(&g), g0.roll));
        g.touch(1, Touch::Move, g0.roll.x + 2, ey, R);
        assert_eq!(g.end_now(), 5 * 240);
        assert_eq!(
            g.touch(1, Touch::Up, g0.roll.x + 2, ey, R),
            Some(EventKind::Stretch {
                index: CLIP_END,
                to: 1200
            })
        );
        assert!(
            !differ_in(&before, &painted(&g), g0.roll),
            "as it was until the clip says"
        );
        // Off a bar line: the nearest bar, or with SHIFT the step.
        let off = 4 * bar - 5 * 240;
        g.touch(2, Touch::Down, ex, ey, R);
        g.touch(2, Touch::Move, at(&g, off), ey, R);
        assert_eq!(g.end_now(), 4 * bar);
        g.fine = true;
        g.touch(2, Touch::Move, at(&g, off) + 1, ey, R);
        assert_eq!(g.end_now(), off);
        g.touch(2, Touch::Move, ex, ey, R);
        assert_eq!(g.touch(2, Touch::Up, ex, ey, R), None, "back where it was");
        g.fine = false;
        // At the roll's right edge: a bar each END_RATE, the roll scrolled
        // past the old end.
        let edge = g0.roll.right() - 2;
        g.touch(3, Touch::Down, ex, ey, R);
        g.touch(3, Touch::Move, ex + 2, ey, R);
        assert!(
            !g.end_at_edge(),
            "a finger resting on the end takes nothing on"
        );
        g.touch(3, Touch::Move, edge, ey, R);
        assert!(g.end_at_edge());
        g.grow_end(END_RATE / 2.0);
        assert_eq!(g.end_now(), 4 * bar);
        g.grow_end(END_RATE);
        assert_eq!(g.end_now(), 5 * bar);
        g.grow_end(2.0 * END_RATE);
        assert_eq!(g.end_now(), 7 * bar);
        let v = g.geo(R).v;
        assert!(v.start > 0 && v.start + v.ticks_shown(g0.roll) > 7 * bar);
        assert_eq!(
            g.touch(3, Touch::Up, edge, ey, R),
            Some(EventKind::Stretch {
                index: CLIP_END,
                to: 7 * bar
            })
        );
        // A second finger: a pinch, and the length stays.
        g.view = g.view.clamped(g0.roll, g.length, g.rows());
        g.touch(4, Touch::Down, ex, ey, R);
        g.touch(4, Touch::Move, at(&g, 2 * bar), ey, R);
        g.touch(5, Touch::Down, ex - 200, ey + 200, R);
        let sent = [
            g.touch(4, Touch::Up, at(&g, 2 * bar), ey, R),
            g.touch(5, Touch::Up, ex - 200, ey + 200, R),
        ];
        assert!(
            !sent
                .iter()
                .any(|e| matches!(e, Some(EventKind::Stretch { .. }))),
            "{sent:?}"
        );
        // At its stop: not swept.
        g.shortest = g.length;
        assert!(!g.targets(R).iter().any(|t| t.id == "end"));
    }

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
            Some(EventKind::Marker { index: 1, frame }) => {
                assert_eq!(drop_of(frame), (960, 2, false))
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(g.note_rect(R, 1), Some(n1), "back in place once dropped");
    }

    /// The lane under the roll (plan 2A.1): a bar for each place where
    /// notes start, a stroke drawn along it and sent once, as the finger
    /// lifts, with each note's own value; of notes that start together a
    /// picked one is set alone; the chip steps through Velocity, Chance,
    /// Repeats and Pressure, SHIFT on Repeats draws the ramp, the ▾ folds
    /// it and the top edge makes it taller. Then the list.
    #[test]
    fn the_lane_sets_each_note_along_a_stroke() {
        let mut g = grid();
        // A chord at 0: note 0 and a third note, not picked.
        g.notes.push(NoteBox::of(&Note::new(0, 240, 55, 90), false));
        let geo = g.geo(R);
        let (v, body) = (geo.vel, lane_body(&geo));
        let (lane, notes) = g.lane_of(&geo, Field::Velocity);
        assert_eq!(notes, vec![vec![0, 2], vec![1]], "two places");
        assert_eq!(lane.bars[0].value, 100, "the loudest of the chord");
        assert!(lane.bars[1].picked && !lane.bars[0].picked);
        // A stroke from the first place's foot to the second's top: shown
        // as it goes, sent once on the lift, each note its own value.
        let (x0, x1) = (lane.bars[0].x + 2, lane.bars[1].x + 2);
        let painted_before = painted(&g);
        assert_eq!(g.touch(1, Touch::Down, x0, body.bottom() - 1, R), None);
        assert_eq!(g.touch(1, Touch::Move, x1, body.y + 14, R), None);
        assert!(
            differ_in(&painted_before, &painted(&g), v),
            "the bars follow the finger"
        );
        let Some(EventKind::Values { index, set }) = g.touch(1, Touch::Up, x1, body.y + 14, R)
        else {
            panic!("a stroke sends what it changed");
        };
        assert_eq!(FIELDS[index], Field::Velocity);
        // The chord's notes share their place's value, low on a rising
        // line; the second place is at the top.
        let ids: Vec<usize> = set.iter().map(|s| s.0).collect();
        assert_eq!(ids, [0, 2, 1]);
        assert!(set[0].1 == set[1].1 && set[0].1 < 30, "{set:?}");
        assert_eq!(set[2].1, 127);
        assert_eq!(g.stroke, None);
        // One of the chord picked: that one alone is set.
        g.notes[2].selected = true;
        g.touch(2, Touch::Down, x0, body.y + 14, R);
        let Some(EventKind::Values { set, .. }) = g.touch(2, Touch::Up, x0, body.y + 14, R) else {
            panic!();
        };
        assert_eq!(set, vec![(2, 127)]);
        // The chip: Chance, then Repeats, Pressure and back to Velocity;
        // the screen's own (a View).
        let chip = mid(geo.chip);
        assert!(matches!(
            g.touch(3, Touch::Down, chip.0, chip.1, R),
            Some(EventKind::View { .. })
        ));
        g.touch(3, Touch::Up, chip.0, chip.1, R);
        assert_eq!(g.lane.kind, LaneKind::Value(Field::Chance));
        let (lane, _) = g.lane_of(&geo, Field::Chance);
        assert_eq!(
            (lane.min, lane.max, lane.bars[1].label.as_str()),
            (1, 100, "100%")
        );
        g.lane.kind = LaneKind::Value(Field::Repeats);
        g.notes[1].ratchet = 3;
        g.notes[1].ramp = -20;
        let (lane, _) = g.lane_of(&geo, Field::Repeats);
        assert_eq!((lane.max, lane.bars[1].label.as_str()), (8, "3↓"));
        // SHIFT as the finger comes down on Repeats: the ramp, centred,
        // only on a note that repeats.
        g.fine = true;
        g.touch(4, Touch::Down, x1, body.y + 14, R);
        assert_eq!(
            g.stroke.as_ref().map(|s| s.kind),
            Some(LaneKind::Value(Field::Ramp))
        );
        assert_eq!(
            g.stroke.as_ref().map(|s| s.lane.style),
            Some(LaneStyle::Centred)
        );
        let Some(EventKind::Values { index, set }) = g.touch(4, Touch::Up, x1, body.y + 14, R)
        else {
            panic!();
        };
        assert_eq!((FIELDS[index], set), (Field::Ramp, vec![(1, 126)]));
        g.fine = false;
        // The ▾ folds the lane under, the roll taking the room; the chip
        // opens it again.
        let roll_h = geo.roll.h;
        let fold = mid(geo.fold);
        assert!(g.touch(5, Touch::Down, fold.0, fold.1, R).is_some());
        g.touch(5, Touch::Up, fold.0, fold.1, R);
        let folded = g.geo(R);
        assert!(g.lane.folded && folded.roll.h > roll_h && folded.fold.is_empty());
        assert_eq!(
            g.touch(6, Touch::Down, folded.vel.x + 4, folded.vel.y + 4, R),
            None
        );
        assert_eq!(g.stroke, None, "nothing to draw on folded");
        g.touch(6, Touch::Up, folded.vel.x + 4, folded.vel.y + 4, R);
        let chip = mid(folded.chip);
        g.touch(7, Touch::Down, chip.0, chip.1, R);
        g.touch(7, Touch::Up, chip.0, chip.1, R);
        assert!(!g.lane.folded);
        // The top edge: 40 px up makes it 40 px taller, kept on the lift.
        let e = g.geo(R).edge;
        assert_eq!(g.touch(8, Touch::Down, e.x + 100, e.y + 3, R), None);
        g.touch(8, Touch::Move, e.x + 100, e.y - 37, R);
        assert!(matches!(
            g.touch(8, Touch::Up, e.x + 100, e.y - 37, R),
            Some(EventKind::View { .. })
        ));
        assert_eq!(g.geo(R).vel.h, v.h + 40);
        assert_eq!(g.lane.h, v.h + 40);
        g.lens = Lens::List;
        // Row 1 is the first note in time order.
        let row = g.list_rect(R, 2).unwrap();
        assert!(matches!(
            g.touch(9, Touch::Down, mid(row).0, mid(row).1, R),
            Some(EventKind::Select(_))
        ));
        assert_eq!(g.button_label(1), "List", "the lens showing");
    }

    /// The Pressure lane (plan 3A.1): the picked notes' pressure as a
    /// line of narrow columns along them, the highest where picked notes
    /// sound together; a stroke sends each column it drew by its first
    /// tick, then its end, once, as the finger lifts. Nothing picked:
    /// no columns, nothing to draw, a hint in their place.
    #[test]
    fn the_pressure_lane_draws_a_line_along_the_picked_notes() {
        use mpfree_seq::TouchPoint;
        let mut g = grid();
        g.lane.kind = LaneKind::Pressure;
        // Note 1 (960, 480) picked, its pressure 40 then 100 from 240.
        g.touch = vec![
            TouchPoint { at: 0, value: 40 },
            TouchPoint {
                at: 240,
                value: 100,
            },
        ];
        g.notes[1].touch = mpfree_seq::Touch { first: 0, len: 2 };
        let geo = g.geo(R);
        let body = lane_body(&geo);
        let (lane, cols) = g.pressure_lane(&geo);
        assert_eq!(lane.style, LaneStyle::Line);
        assert_eq!(lane.bars.len(), cols.len());
        let (x0, x1) = (g.x_of(&geo, 960), g.x_of(&geo, 1440));
        assert!(
            lane.bars.iter().all(|b| b.x + b.w > x0 && b.x < x1),
            "along it"
        );
        assert!((lane.bars[0].x - x0).abs() <= PRESSURE_COL);
        assert_eq!(lane.bars[0].value, 40);
        assert_eq!(lane.bars.last().unwrap().value, 100);
        assert!(cols.windows(2).all(|w| w[0].1 == w[1].0), "end to end");
        // A stroke along the first part at the top: each column it
        // drew, then where it ends.
        let (a, b) = (lane.bars[1].x + 1, lane.bars[4].x + 1);
        let top = body.y + 14;
        assert_eq!(g.touch(1, Touch::Down, a, top, R), None);
        g.touch(1, Touch::Move, b, top, R);
        let Some(EventKind::Values { index, set }) = g.touch(1, Touch::Up, b, top, R) else {
            panic!("a stroke sends its line");
        };
        assert_eq!(index, PRESSURE_LANE);
        let ticks: Vec<usize> = cols[1..=4].iter().map(|c| c.0 as usize).collect();
        let sent: Vec<usize> = set[..4].iter().map(|s| s.0).collect();
        assert_eq!(sent, ticks);
        assert!(set[..4].iter().all(|s| s.1 == 127), "{set:?}");
        assert_eq!(set[4], (cols[4].1 as usize, PRESSURE_END));
        // Two picked notes sounding together: the higher pressure.
        g.notes[0].selected = true;
        g.notes[0].tick = 960;
        g.touch.push(TouchPoint { at: 0, value: 90 });
        g.notes[0].touch = mpfree_seq::Touch { first: 2, len: 1 };
        let (lane, _) = g.pressure_lane(&g.geo(R));
        assert_eq!(lane.bars[0].value, 90);
        // Nothing picked: no columns and no stroke, a hint to pick.
        g.notes.iter_mut().for_each(|n| n.selected = false);
        assert!(g.pressure_lane(&g.geo(R)).0.bars.is_empty());
        g.touch(2, Touch::Down, a, top, R);
        assert_eq!(g.stroke, None);
        g.touch(2, Touch::Up, a, top, R);
        // The hint: ink across the middle of the empty lane.
        let c = painted(&g);
        let (cx, cy) = (body.x + body.w / 2, body.y + body.h / 2);
        let empty = c.pixel(body.x + 2, cy);
        assert!(
            (cx - 60..cx + 60).any(|x| (cy - 4..cy + 4).any(|y| c.pixel(x, y) != empty)),
            "the hint to pick notes"
        );
        // The chip steps on from Pressure to Velocity.
        let chip = mid(geo.chip);
        g.touch(3, Touch::Down, chip.0, chip.1, R);
        g.touch(3, Touch::Up, chip.0, chip.1, R);
        assert_eq!(g.lane.kind, LaneKind::Value(Field::Velocity));
    }

    /// A lock lane (plan 3A.2): a bar where the notes play the setting
    /// (a step and a row), as high as its lock or faint at what the step
    /// plays; a stroke sends each bar it set by its first note, with the
    /// place of its value; the notes a lock reaches are marked, and a tap
    /// on a note's marks opens their lane. The chip steps through the
    /// lock lanes after Pressure.
    #[test]
    fn a_lock_lane_shows_and_sets_each_steps_lock() {
        let addr = RelAddr::new(OnTrack::Inst, 7);
        let mut g = grid();
        // A second note on step 0, a fifth up.
        g.notes.push(NoteBox::of(&Note::new(0, 240, 55, 90), false));
        g.lock_settings = Arc::new(vec![LockSetting {
            addr,
            name: "Cutoff".into(),
            own: 5.0,
            values: (0..=10).map(|k| (k as f32, format!("{k} k"))).collect(),
            centred: false,
        }]);
        g.locks = vec![LockMark {
            tick: 0,
            sound: None,
            addr,
            value: 8.0,
        }];
        g.lane.kind = LaneKind::Lock(addr);
        let geo = g.geo(R);
        let (lane, notes) = g.lock_lane(&geo, 0);
        assert_eq!(lane.bars.len(), 2, "step 1's and step 5's");
        assert_eq!((lane.bars[0].value, lane.bars[0].dim), (8, false));
        assert_eq!(
            (lane.bars[1].value, lane.bars[1].dim),
            (5, true),
            "no lock: what the step plays"
        );
        assert_eq!(notes, [vec![0, 2], vec![1]], "step 1's notes share it");
        assert!(lane.bars[1].picked && !lane.bars[0].picked);
        assert_eq!(
            g.lock_card(&lane, &notes, addr).as_deref(),
            Some("5 k · not locked")
        );
        // Both of step 1's notes are marked; picked, it says they share.
        assert_eq!(
            (g.marks(0), g.marks(1), g.marks(2)),
            ((true, false), (false, false), (true, false))
        );
        g.notes[0].selected = true;
        g.notes[1].selected = false;
        let (lane, notes) = g.lock_lane(&geo, 0);
        assert_eq!(
            g.lock_card(&lane, &notes, addr).as_deref(),
            Some("8 k · shared by 2 notes")
        );
        // A stroke from step 1's foot to step 5's top: both, by their
        // first notes, the faint one at the top too.
        let body = lane_body(&geo);
        let (a, b) = (lane.bars[0].x + 1, lane.bars[1].x + 1);
        g.touch(1, Touch::Down, a, body.bottom() - 1, R);
        g.touch(1, Touch::Move, b, body.y + 14, R);
        let Some(EventKind::Values { index, set }) = g.touch(1, Touch::Up, b, body.y + 14, R)
        else {
            panic!("a stroke sends the bars it set");
        };
        assert_eq!(index, LOCK_LANES);
        assert_eq!((set.len(), set[0].0, set[1]), (2, 0, (1, 10)), "{set:?}");
        // A tap on note 2's marks (a lock on step 5 now) opens the lane,
        // unfolded, from Velocity, and picks the note.
        g.locks.push(LockMark {
            tick: 960,
            sound: None,
            addr,
            value: 2.0,
        });
        g.lane = LaneShow {
            folded: true,
            ..LaneShow::default()
        };
        let marks = |g: &NoteGrid| mid(g.mark_zone(&g.geo(R), 1).expect("note 2 is wide enough"));
        let at = marks(&g);
        g.touch(2, Touch::Down, at.0, at.1, R);
        assert_eq!(
            g.touch(2, Touch::Up, at.0, at.1, R),
            Some(EventKind::Select(1))
        );
        assert_eq!((g.lane.kind, g.lane.folded), (LaneKind::Lock(addr), false));
        // A chance under 100 % with no lock: its marks open Chance.
        g.locks.clear();
        g.notes[1].chance = 50;
        assert_eq!(g.marks(1), (false, true));
        let at = marks(&g);
        g.touch(3, Touch::Down, at.0, at.1, R);
        g.touch(3, Touch::Up, at.0, at.1, R);
        assert_eq!(g.lane.kind, LaneKind::Value(Field::Chance));
        // Elsewhere on the note, a tap is a tap.
        let nr = g.note_rect_in(&g.geo(R), 1).unwrap();
        g.touch(4, Touch::Down, nr.x + 8, at.1, R);
        assert_eq!(
            g.touch(4, Touch::Up, nr.x + 8, at.1, R),
            Some(EventKind::Toggle(1))
        );
        // The chip: Pressure, then the lock lane, then Velocity again.
        g.lane.kind = LaneKind::Pressure;
        let chip = mid(geo.chip);
        let chip_tap = |g: &mut NoteGrid| {
            g.touch(5, Touch::Down, chip.0, chip.1, R);
            g.touch(5, Touch::Up, chip.0, chip.1, R);
            g.lane.kind
        };
        assert_eq!(chip_tap(&mut g), LaneKind::Lock(addr));
        assert_eq!(chip_tap(&mut g), LaneKind::Value(Field::Velocity));
        // A setting the app no longer offers: the lane shows Velocity.
        g.lane.kind = LaneKind::Lock(addr);
        g.lock_settings = Arc::default();
        assert_eq!(g.lane_kind(), LaneKind::Value(Field::Velocity));
    }

    /// A setting either side of 0 draws from the middle: its lane runs
    /// from below 0 to above it, a step without a lock at 0.
    #[test]
    fn a_centred_lock_lane_draws_from_the_middle() {
        let addr = RelAddr::new(OnTrack::Pad(50), 1);
        let mut g = grid();
        g.lock_settings = Arc::new(vec![LockSetting {
            addr,
            name: "Tune".into(),
            own: 0.0,
            values: (-2..=2)
                .map(|k| (k as f32 * 100.0, format!("{k:+} st")))
                .collect(),
            centred: true,
        }]);
        let (lane, notes) = g.lock_lane(&g.geo(R), 0);
        assert_eq!(
            (lane.min, lane.max, lane.style),
            (-2, 2, LaneStyle::Centred)
        );
        // Pad 50's setting: only the note on pad 50 plays it.
        assert_eq!(notes, [vec![0]]);
        assert_eq!(lane.bars[0].value, 0);
    }

    #[test]
    fn it_paints_every_lens() {
        use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};
        let mut g = grid();
        g.playhead = Some(1000);
        g.locks = vec![LockMark {
            tick: 960,
            sound: Some(52),
            addr: RelAddr::new(OnTrack::Inst, 3),
            value: 0.5,
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

/// The clip's scale on the note editor (piano roll plan 2B.1).
#[cfg(test)]
mod scale_tests {
    use super::*;
    use mpfree_seq::mpfree_scale::{DORIAN, MAJOR, MINOR};
    use mpfree_seq::Holds;

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 830,
        h: 640,
    };

    fn clip(notes: &[(Tick, u8)]) -> Clip {
        let mut c = Clip::new("Keys", 3840);
        c.holds = Holds::Notes;
        for &(t, n) in notes {
            c.add_note(Note::new(t, 240, n, 100)).unwrap();
        }
        c.holds = mpfree_seq::Holds::Notes;
        c
    }

    fn run(ed: &mut Editor, c: &mut Clip, a: EditAction) -> Done {
        let done = ed.act(a, c);
        c.apply(&done.edit).unwrap();
        if let Some(k) = done.scale {
            c.scale = k;
        }
        ed.applied(c, &done);
        done
    }

    fn pitches(c: &Clip) -> Vec<u8> {
        let mut p: Vec<u8> = c.notes.iter().map(|n| n.note).collect();
        p.sort_unstable();
        p
    }

    /// The Scale menu offers Off, the pads' scale first, every built-in
    /// scale but Chromatic, then the owner's; a scale keeps the clip's
    /// root (the pads' with none); the pads' choice is lit when the clip
    /// has their scale, root and all.
    #[test]
    fn the_scale_menu_offers_the_pads_scale_first() {
        let none = ScaleOffers::default();
        assert_eq!(none.len(), 1 + SCALES.len() - 1);
        assert_eq!(none.choice_name(0).as_deref(), Some("Off"));
        assert_eq!(none.choice_name(1).as_deref(), Some("Major"));
        assert_eq!(none.choice_name(none.len()), None);
        let offers = ScaleOffers {
            pads: Some(Scale::new(2, MINOR)),
            own: vec![("Mine".into(), 0b1001_0001, 0b1001_0001)],
        };
        assert_eq!(offers.len(), 2 + SCALES.len());
        assert_eq!(offers.choice_name(1).as_deref(), Some("Pads: D Minor"));
        assert_eq!(offers.choice_name(2).as_deref(), Some("Major"));
        assert_eq!(
            offers.choice_name(offers.len() - 1).as_deref(),
            Some("Mine")
        );
        assert_eq!(offers.scale_of(0, None), Some(None));
        assert_eq!(offers.scale_of(1, None), Some(Some(Scale::new(2, MINOR))));
        assert_eq!(
            offers.scale_of(2, None),
            Some(Some(Scale::new(2, MAJOR))),
            "the pads' root"
        );
        let on_a = Some(Scale::new(9, DORIAN));
        assert_eq!(
            offers.scale_of(2, on_a),
            Some(Some(Scale::new(9, MAJOR))),
            "the clip's root"
        );
        assert_eq!(offers.scale_of(offers.len(), on_a), None);
        assert_eq!(offers.index_of(Some(Scale::new(2, MINOR))), Some(1));
        assert_eq!(
            offers.index_of(Some(Scale::new(4, MINOR))),
            offers.index_of(Some(Scale::new(0, MINOR)))
        );
        assert_ne!(offers.index_of(Some(Scale::new(4, MINOR))), Some(1));
        assert_eq!(
            offers.index_of(Some(Scale::new(0, 0b11))),
            None,
            "no choice names it"
        );
        assert_eq!(offers.label(on_a), "A Dorian");
        assert_eq!(offers.label(Some(Scale::new(0, 0b1001_0001))), "C Mine");
        assert_eq!(offers.label(Some(Scale::new(0, 0b11))), "C Own scale");
        assert_eq!(offers.label(None), "Off");
    }

    /// The menu's picks change the clip's scale, a setting the app keeps
    /// (no note moves); a root needs a scale; a clip of pads has none.
    #[test]
    fn the_scale_and_its_root_are_the_clips_settings() {
        let mut c = clip(&[(0, 60), (240, 61)]);
        let mut ed = Editor::new(0, 0, &c);
        let done = ed.act(EditAction::Root(2), &c);
        assert_eq!(
            (done.scale, done.line.as_str()),
            (None, "edit: pick a scale first")
        );
        let done = run(
            &mut ed,
            &mut c,
            EditAction::Scale(Some(Scale::new(0, MAJOR))),
        );
        assert!(done.edit.is_empty());
        assert_eq!(done.line, "edit: scale C Major");
        assert_eq!(c.scale, Some(Scale::new(0, MAJOR)));
        assert_eq!(pitches(&c), [60, 61], "no note moves");
        let done = run(&mut ed, &mut c, EditAction::Root(9));
        assert_eq!(done.line, "edit: scale A Major");
        assert_eq!(c.scale, Some(Scale::new(9, MAJOR)));
        run(&mut ed, &mut c, EditAction::Scale(None));
        assert_eq!(c.scale, None);
        let bad = Scale {
            root: 12,
            ..Scale::new(0, MAJOR)
        };
        assert_eq!(ed.act(EditAction::Scale(Some(bad)), &c).scale, None);
        // A clip of pad hits has none, whatever pads it hits (B1 is 16),
        // and lights none it was given.
        for notes in [&[(0, 3)][..], &[(0, 3), (240, 16)], &[]] {
            let mut pads = clip(notes);
            pads.holds = Holds::Hits;
            for a in [
                EditAction::Scale(Some(Scale::new(0, MAJOR))),
                EditAction::Root(2),
            ] {
                let done = Editor::new(0, 0, &pads).act(a, &pads);
                assert_eq!(
                    (done.scale, done.line.as_str()),
                    (None, "edit: a clip of pads has no scale"),
                    "{notes:?}"
                );
            }
            pads.scale = Some(Scale::new(0, MAJOR));
            assert_eq!(lit_scale(&pads), None, "{notes:?}");
        }
        // An empty clip of notes takes one.
        let empty = clip(&[]);
        let done =
            Editor::new(0, 0, &empty).act(EditAction::Scale(Some(Scale::new(0, MAJOR))), &empty);
        assert_eq!(done.scale, Some(Some(Scale::new(0, MAJOR))));
    }

    /// With the clip's scale on, a drag and Q3 move by its notes (a chord
    /// keeps to the scale), SHIFT on Q3 by semitones; Fit to scale brings
    /// the picks, or every note, onto it; Select's Off the scale picks
    /// the notes the roll marks.
    #[test]
    fn moves_go_by_the_clips_scale_and_fit_brings_notes_onto_it() {
        let mut c = clip(&[(0, 60), (0, 64), (0, 67), (960, 61)]);
        c.scale = Some(Scale::new(0, MAJOR));
        let mut ed = Editor::new(0, 0, &c);
        ed.sel = vec![0, 1, 2];
        run(
            &mut ed,
            &mut c,
            EditAction::Drop {
                index: 0,
                rev: 0,
                ticks: 0,
                notes: 1,
                scale: true,
            },
        );
        assert_eq!(pitches(&c), [61, 62, 65, 69], "C E G a step up: D F A");
        let turn = |k, ticks, fine| EditAction::Knob {
            k,
            ticks,
            fine,
            turn: 1,
        };
        run(&mut ed, &mut c, turn(2, -1, false));
        assert_eq!(pitches(&c), [60, 61, 64, 67], "and back");
        run(&mut ed, &mut c, turn(2, 1, true));
        assert_eq!(pitches(&c), [61, 61, 65, 68], "SHIFT: a semitone");
        run(&mut ed, &mut c, turn(2, -1, true));
        // Off the scale: the C#.
        ed.act(EditAction::Choose(13), &c);
        assert_eq!(
            ed.sel.iter().map(|&i| c.notes[i].note).collect::<Vec<_>>(),
            [61]
        );
        let rows = edit_rows(&ed, &c);
        assert_eq!(rows[FIT_ROW].name, "Fit to scale");
        assert_eq!(rows[FIT_ROW].value, "1 off");
        let done = run(
            &mut ed,
            &mut c,
            EditAction::Row {
                row: FIT_ROW,
                delta: 1,
            },
        );
        assert_eq!(done.line, "edit: 1 note fitted to C Major");
        assert_eq!(pitches(&c), [60, 60, 64, 67]);
        assert_eq!(edit_rows(&ed, &c)[FIT_ROW].value, "none off");
        // A drag without the scale's flag (SHIFT as it went down) goes by
        // semitones.
        ed.sel = vec![];
        let i = c.notes.iter().position(|n| n.tick == 960).unwrap();
        run(
            &mut ed,
            &mut c,
            EditAction::Drop {
                index: i,
                rev: 0,
                ticks: 0,
                notes: 1,
                scale: false,
            },
        );
        assert_eq!(pitches(&c), [60, 61, 64, 67]);
        c.scale = None;
        assert_eq!(edit_rows(&ed, &c)[FIT_ROW].value, "no scale");
        assert_eq!(
            ed.act(EditAction::Choose(13), &c).line,
            "edit: the clip has no scale: pick one with Scale"
        );
        assert_eq!(knob_name(2, true), "Scale steps");
        assert_eq!(knob_name(2, false), "Semitones");
    }

    fn roll(notes: &[(u8, bool)], scale: Option<Scale>) -> NoteGrid {
        NoteGrid {
            notes: notes
                .iter()
                .enumerate()
                .map(|(k, &(n, picked))| {
                    NoteBox::of(&Note::new(k as Tick * 480, 240, n, 100), picked)
                })
                .collect(),
            length: 3840,
            bar: 3840,
            view: RollView {
                lift: 57 * 40,
                row_h: 40,
                ..RollView::default()
            },
            scale,
            scale_name: ScaleOffers::default().label(scale),
            ..NoteGrid::default()
        }
    }

    fn target(g: &NoteGrid, id: &str) -> Option<Rect> {
        g.targets(R)
            .into_iter()
            .find(|t| t.id == id)
            .map(|t| t.rect)
    }

    /// A finger down and up on `r`: what either sent.
    fn tap(g: &mut NoteGrid, f: u64, r: Rect) -> Option<EventKind> {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        let down = g.touch(f, Touch::Down, x, y, R);
        let up = g.touch(f, Touch::Up, x, y, R);
        assert!(down.is_none() || up.is_none(), "one event a tap");
        down.or(up)
    }

    /// A drag with the clip's scale on goes by its notes: the finger two
    /// rows up from C lands on D, one scale step, sent with the scale's
    /// flag; with SHIFT as it went down, by semitones.
    #[test]
    fn a_drag_on_the_roll_goes_by_the_scales_notes() {
        for (fine, want) in [(false, (1, true)), (true, (2, false))] {
            let mut g = roll(&[(60, false)], Some(Scale::new(0, MAJOR)));
            g.fine = fine;
            let nr = g.note_rect(R, 0).unwrap();
            let (x, y) = (nr.x + 10, nr.y + nr.h / 2);
            g.touch(1, Touch::Down, x, y, R);
            g.touch(1, Touch::Move, x, y - 80, R);
            let Some(EventKind::Marker { index: 0, frame }) = g.touch(1, Touch::Up, x, y - 80, R)
            else {
                panic!("a drag drops");
            };
            let (ticks, notes, scale) = drop_of(frame);
            assert_eq!((ticks, (notes, scale)), (0, want), "fine {fine}");
        }
    }

    /// A note drawn on a row off the scale goes on its nearest scale note
    /// (the lower on a tie); with SHIFT, on the row.
    #[test]
    fn a_note_drawn_off_the_scale_lands_on_it() {
        for (fine, want) in [(false, 60), (true, 61)] {
            let mut g = roll(&[(64, false)], Some(Scale::new(0, MAJOR)));
            g.tool = Tool::Draw;
            g.fine = fine;
            let nr = g.note_rect(R, 0).unwrap();
            // The C# row, three rows under the E, past the note's end.
            let (x, y) = (nr.right() + 60, nr.y + nr.h / 2 + 3 * 40);
            g.touch(1, Touch::Down, x, y, R);
            match g.touch(1, Touch::Up, x, y, R) {
                Some(EventKind::Cell { row, .. }) => assert_eq!(row, want, "fine {fine}"),
                e => panic!("a cell: {e:?}"),
            }
        }
    }

    /// Fold: rows with notes, or the scale's rows, a row with a picked
    /// note never folding away; a chip counts the notes it hides and a
    /// tap on it shows every row; the Fold chip steps through them,
    /// Scale rows only while the clip has a scale.
    #[test]
    fn fold_hides_rows_and_a_chip_counts_the_notes_hidden() {
        let major = Scale::new(0, MAJOR);
        let mut g = roll(&[(60, false), (61, false), (64, false)], Some(major));
        assert_eq!(g.rows(), 128);
        assert_eq!(target(&g, "hidden-chip"), None);
        g.fold = Fold::Notes;
        assert_eq!(g.rows(), 3);
        g.fold = Fold::Scale;
        assert_eq!(
            g.rows(),
            (0..128).filter(|&n| major.contains(n)).count() as u8
        );
        let hidden = target(&g, "hidden-chip").expect("1 note hidden");
        assert_eq!(g.row_map().of(61), None, "the C# has no row");
        assert_eq!(tap(&mut g, 1, hidden), Some(g.view_event()));
        assert_eq!((g.fold, g.rows()), (Fold::All, 128));
        // Picked, the C# keeps its row.
        let mut picked = roll(&[(60, false), (61, true), (64, false)], Some(major));
        picked.fold = Fold::Scale;
        assert!(picked.row_map().of(61).is_some());
        assert_eq!(target(&picked, "hidden-chip"), None);
        // Folding keeps the note in the roll's middle there.
        let mut fold = roll(&[(60, false), (61, false), (64, false)], Some(major));
        let g0 = fold.geo(R);
        let at = fold.note_at(&g0, g0.roll.y + g0.roll.h / 2).unwrap();
        fold.refold(R, Fold::Scale);
        let g1 = fold.geo(R);
        let now = fold.note_at(&g1, g1.roll.y + g1.roll.h / 2).unwrap();
        assert!(major.contains(i16::from(now)) && (i16::from(now) - i16::from(at)).abs() <= 2);
        // The chip: All, Notes, Scale, All; with no scale, All, Notes, All.
        let chip = target(&g, "fold-chip").unwrap();
        for want in [Fold::Notes, Fold::Scale, Fold::All] {
            tap(&mut g, 2, chip);
            assert_eq!(g.fold, want);
        }
        let mut plain = roll(&[(60, false)], None);
        for want in [Fold::Notes, Fold::All] {
            tap(&mut plain, 3, chip);
            assert_eq!(plain.fold, want);
        }
        plain.fold = Fold::Scale;
        assert_eq!(plain.rows(), 128, "no scale: every row");
    }

    /// The Scale chip opens the Scale menu over the roll: a root (with a
    /// scale on) and a scale are its taps, and it stays open for the
    /// other; a touch anywhere else closes it and does nothing more.
    #[test]
    fn the_scale_chip_opens_the_scale_menu() {
        let mut g = roll(&[(60, false)], None);
        let chip = target(&g, "scale-chip").unwrap();
        assert_eq!(tap(&mut g, 1, chip), None);
        assert!(g.scale_menu);
        assert_eq!(target(&g, "scale-chip"), None, "the menu covers the roll");
        let pick = |g: &NoteGrid, id: &str, k: usize| {
            g.targets(R)
                .into_iter()
                .find(|t| t.id == id && t.index == k)
                .map(|t| t.rect)
                .unwrap()
        };
        let root = pick(&g, "scale-root", 2);
        assert_eq!(tap(&mut g, 2, root), None, "no scale: no root");
        let dorian = pick(&g, "scale-pick", 2);
        assert_eq!(tap(&mut g, 3, dorian), Some(EventKind::Tab(SCALE_MENU + 2)));
        g.scale = Some(Scale::new(0, DORIAN));
        assert_eq!(tap(&mut g, 4, root), Some(EventKind::Tab(ROOT_MENU + 2)));
        assert!(g.scale_menu);
        let tools = g.button_rects(R)[0];
        assert_eq!(
            tap(&mut g, 5, tools),
            None,
            "a touch elsewhere only closes it"
        );
        assert!(!g.scale_menu);
        // A clip of pads has no scale.
        let mut pads = roll(&[(3, false)], None);
        pads.pads = true;
        assert_eq!(target(&pads, "scale-chip"), None);
    }
}

#[cfg(test)]
mod chord_tests {
    use super::*;
    use mpfree_seq::mpfree_scale::{MAJOR, MINOR};
    use mpfree_seq::Holds;

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 830,
        h: 640,
    };

    fn keys(notes: &[(Tick, Tick, u8)]) -> Clip {
        let mut c = Clip::new("Keys", 3840);
        c.holds = Holds::Notes;
        for &(t, l, n) in notes {
            c.add_note(Note::new(t, l, n, 100)).unwrap();
        }
        c
    }

    /// As the time engine does: an edit that changes something goes in
    /// and the picks follow it.
    fn run(ed: &mut Editor, c: &mut Clip, a: EditAction) -> Done {
        let done = ed.act(a, c);
        if !done.edit.is_empty() {
            c.apply(&done.edit).unwrap();
            ed.applied(c, &done);
        }
        done
    }

    fn picked(ed: &Editor, c: &Clip) -> Vec<(Tick, u8)> {
        let mut p: Vec<(Tick, u8)> = ed
            .picked(c)
            .iter()
            .map(|&i| (c.notes[i].tick, c.notes[i].note))
            .collect();
        p.sort_unstable();
        p
    }

    fn chord(shape: ChordShape, minor: bool) -> EditAction {
        EditAction::Chord { shape, minor }
    }

    /// A note drawn puts in the Chord chip's chord, picked after, as one
    /// take: a triad in the clip's scale, major or minor with none, the
    /// pads' triad, one note with the chip Off or on a clip of pads; an
    /// empty clip of notes takes chords too. The chip changes nothing in
    /// the clip.
    #[test]
    fn a_note_drawn_puts_in_the_chord_chips_chord() {
        let mut c = keys(&[]);
        c.scale = Some(Scale::new(0, MAJOR));
        assert!(!pads(&c), "an empty clip of notes shows the keys");
        let mut ed = Editor::new(0, 0, &c);
        ed.tool = Tool::Draw;
        let done = run(&mut ed, &mut c, chord(ChordShape::Triad, false));
        assert_eq!(done.line, "edit: drawing chords: Triad");
        assert!(done.edit.add.is_empty() && c.notes.is_empty());
        let done = run(&mut ed, &mut c, EditAction::Empty { tick: 0, note: 62 });
        assert_eq!(done.line, "edit: chord Dm drawn at tick 0");
        assert_eq!(picked(&ed, &c), [(0, 62), (0, 65), (0, 69)]);
        assert_eq!(done.join, None, "its own take");
        // No scale: Triad and Seventh are major or minor; a drag draws
        // the chord as long as it.
        c.scale = None;
        run(&mut ed, &mut c, chord(ChordShape::Seventh, true));
        let done = run(
            &mut ed,
            &mut c,
            EditAction::Draw {
                tick: 960,
                note: 60,
                len: 480,
            },
        );
        assert_eq!(done.line, "edit: chord Cm7 drawn at tick 960");
        assert_eq!(
            picked(&ed, &c),
            [(960, 60), (960, 63), (960, 67), (960, 70)]
        );
        assert!(c.notes[ed.picked(&c)[0]].len == 480);
        // Pads: the triad the Chord pads play, in the pads' scale.
        ed.offers = Arc::new(ScaleOffers {
            pads: Some(Scale::new(2, MINOR)),
            own: Vec::new(),
        });
        run(&mut ed, &mut c, chord(ChordShape::Pads, false));
        run(
            &mut ed,
            &mut c,
            EditAction::Empty {
                tick: 1920,
                note: 65,
            },
        );
        assert_eq!(
            picked(&ed, &c),
            [(1920, 65), (1920, 69), (1920, 72)],
            "F A C"
        );
        // Off: one note.
        run(&mut ed, &mut c, chord(ChordShape::Off, false));
        run(
            &mut ed,
            &mut c,
            EditAction::Empty {
                tick: 2880,
                note: 64,
            },
        );
        assert_eq!(picked(&ed, &c), [(2880, 64)]);
        // On a clip of pads, one hit whatever the chip.
        let mut drums = Clip::new("Drums", 3840);
        drums.add_note(Note::new(0, 240, 3, 100)).unwrap();
        let mut ed = Editor::new(0, 0, &drums);
        ed.tool = Tool::Draw;
        ed.chord = ChordShape::Triad;
        run(
            &mut ed,
            &mut drums,
            EditAction::Empty { tick: 480, note: 5 },
        );
        assert_eq!(picked(&ed, &drums), [(480, 5)]);
    }

    /// The tools on chords (plans 2B.2 and 2B.3): a row opens its tool on
    /// the picks; Q1 to Q4 (and the wheel on the row, Q1) change its
    /// settings and the preview, never the clip; other edits wait; Done
    /// makes it one take with the notes it made picked, and Cancel leaves
    /// the clip; another tool's row opens that one in its place.
    #[test]
    fn strum_invert_and_spread_preview_until_done() {
        let mut c = keys(&[
            (0, 960, 60),
            (0, 960, 64),
            (0, 960, 67),
            (960, 960, 65),
            (960, 960, 69),
            (960, 960, 72),
        ]);
        let was = c.clone();
        let mut ed = Editor::new(0, 0, &c);
        let value = |ed: &Editor, c: &Clip, row: usize| edit_rows(ed, c)[row].value.clone();
        let row = |row, delta| EditAction::Row { row, delta };
        let knob = |k, ticks| EditAction::Knob {
            k,
            ticks,
            fine: false,
            turn: 1,
        };
        assert_eq!(EDIT_ROWS[STRUM_ROW], "Strum");
        assert_eq!(value(&ed, &c, STRUM_ROW), "pick notes");
        let done = run(&mut ed, &mut c, row(STRUM_ROW, 1));
        assert_eq!(done.line, "edit: pick notes first");
        assert_eq!(ed.open_tool, None);
        ed.sel = vec![0, 3];
        let done = run(&mut ed, &mut c, row(STRUM_ROW, 1));
        assert_eq!(done.line, "edit: pick a chord: notes that sound together");
        ed.sel = (0..6).collect();
        assert_eq!(value(&ed, &c, STRUM_ROW), "none");
        assert_eq!(value(&ed, &c, INVERT_ROW), "C");
        assert_eq!(value(&ed, &c, SPREAD_ROW), "close");
        // Strum opens at no change: no preview.
        let done = run(&mut ed, &mut c, row(STRUM_ROW, 1));
        assert_eq!(
            done.line,
            "edit: Strum: turn the Q-Links, then Done or Cancel"
        );
        assert_eq!(ed.open_tool.map(|t| t.kind), Some(ToolKind::Strum));
        assert_eq!(value(&ed, &c, STRUM_ROW), "on");
        assert_eq!(ed.preview(&c), None);
        let tool = ed.open_tool.unwrap();
        assert_eq!(tool.why_not(&c, &ed.sel), "nothing changes yet");
        // Q1 two detents, the wheel on the row one more: 30 ticks apart,
        // on the preview only.
        assert!(run(&mut ed, &mut c, knob(0, 2)).edit.is_empty());
        assert!(run(&mut ed, &mut c, row(STRUM_ROW, 1)).edit.is_empty());
        assert_eq!(c, was, "the clip is as it was");
        let g = 3 * STRUM_TICKS;
        assert_eq!(ed.open_tool.unwrap().cards()[0].1, format!("+{g}t"));
        let starts = |c: &Clip| -> Vec<Tick> {
            let mut s: Vec<(u8, Tick)> = c.notes.iter().map(|n| (n.note, n.tick)).collect();
            s.sort_unstable();
            s.into_iter().map(|(_, t)| t).collect()
        };
        let (p, picks) = ed.preview(&c).unwrap();
        assert_eq!(starts(&p), [0, g, 960, 2 * g, 960 + g, 960 + 2 * g]);
        assert_eq!(picks.len(), 6, "the notes it makes are picked");
        // Q2 to Down: the highest first.
        run(&mut ed, &mut c, knob(1, 2));
        assert_eq!(ed.open_tool.unwrap().cards()[1].1, "Down");
        let (p, _) = ed.preview(&c).unwrap();
        assert_eq!(starts(&p), [2 * g, g, 960 + 2 * g, 0, 960 + g, 960]);
        // Other edits wait for Done or Cancel.
        let done = run(&mut ed, &mut c, EditAction::Button(6));
        assert_eq!(done.line, "edit: Done or Cancel Strum first");
        assert_eq!(c, was);
        // Done: one take of its own, picked after; the tool closes.
        let done = run(&mut ed, &mut c, EditAction::Tool(ToolAct::Done));
        assert_eq!(done.line, "edit: 2 chords strummed");
        assert_eq!(done.join, None);
        assert_eq!(starts(&c), [2 * g, g, 960 + 2 * g, 0, 960 + g, 960]);
        assert_eq!(ed.open_tool, None);
        assert_eq!(ed.picked(&c).len(), 6);
        assert_eq!(value(&ed, &c, STRUM_ROW), format!("↓ {g}t"));
        // Invert: Q1 up one previews C/E; Cancel leaves the clip.
        let strummed = c.clone();
        run(&mut ed, &mut c, row(INVERT_ROW, -1));
        run(&mut ed, &mut c, knob(0, 1));
        let (p, picks) = ed.preview(&c).unwrap();
        assert_eq!(value(&ed, &p, INVERT_ROW), "on");
        let notes: Vec<u8> = picks.iter().map(|&i| p.notes[i].note).collect();
        assert!(notes.contains(&72) && !notes.contains(&60), "{notes:?}");
        let done = run(&mut ed, &mut c, EditAction::Tool(ToolAct::Cancel));
        assert_eq!(done.line, "edit: Invert cancelled: the clip is as it was");
        assert_eq!(c, strummed);
        assert_eq!(ed.open_tool, None);
        // Down one, then Spread's row opens Spread in its place.
        run(&mut ed, &mut c, row(INVERT_ROW, 1));
        run(&mut ed, &mut c, knob(0, -1));
        assert_eq!(ed.open_tool.unwrap().cards()[0].1, "Down 1");
        run(&mut ed, &mut c, row(SPREAD_ROW, 1));
        assert_eq!(ed.open_tool.map(|t| t.kind), Some(ToolKind::Spread));
        run(&mut ed, &mut c, knob(0, 1));
        assert_eq!(ed.open_tool.unwrap().cards()[0].1, "Open 1");
        let done = run(&mut ed, &mut c, EditAction::Tool(ToolAct::Done));
        assert_eq!(done.line, "edit: 2 chords spread");
        assert_eq!(value(&ed, &c, SPREAD_ROW), "open");
        // Closed: as close as they go, then no closer (refused at Done,
        // and said why).
        run(&mut ed, &mut c, row(SPREAD_ROW, 1));
        run(&mut ed, &mut c, knob(0, -3));
        let done = run(&mut ed, &mut c, EditAction::Tool(ToolAct::Done));
        assert_eq!(done.line, "edit: 2 chords closed");
        assert_eq!(value(&ed, &c, SPREAD_ROW), "close");
        run(&mut ed, &mut c, row(SPREAD_ROW, 1));
        run(&mut ed, &mut c, knob(0, -1));
        assert_eq!(ed.preview(&c), None);
        let before = c.clone();
        let done = run(&mut ed, &mut c, EditAction::Tool(ToolAct::Done));
        assert_eq!(c, before);
        assert_eq!(
            done.line,
            "edit: Spread: the chords are as close as they go, so nothing changed"
        );
        // Settings past their ends stop there.
        let mut t = NoteTool::new(ToolKind::Invert);
        assert!(t.turn(0, 100, false));
        assert_eq!(t.q[0], MAX_TURNS);
        assert!(!t.turn(0, 1, false));
        assert!(!t.turn(1, 1, false), "Invert has no Q2");
        // One note picked: no chord.
        ed.sel = vec![0];
        assert_eq!(value(&ed, &c, STRUM_ROW), "no chord");
        let done = run(&mut ed, &mut c, row(SPREAD_ROW, 1));
        assert_eq!(done.line, "edit: pick a chord of three notes or more");
    }

    /// Chop, Legato and Humanize (plan 2B.3): their rows open them on the
    /// picks; Chop's Reset pressure and Humanize's Again are the tools' own
    /// button; Legato fills a share of the gap; the same dice give the same
    /// humanize. Something else taking a pick closes the tool.
    #[test]
    fn chop_legato_and_humanize_preview_until_done() {
        let mut c = keys(&[(0, 960, 60), (1920, 480, 62)]);
        c.notes[0].touch = c
            .add_touch(&[
                TouchPoint { at: 0, value: 10 },
                TouchPoint { at: 600, value: 90 },
            ])
            .unwrap();
        let was = c.clone();
        let mut ed = Editor::new(0, 0, &c);
        let value = |ed: &Editor, c: &Clip, row: usize| edit_rows(ed, c)[row].value.clone();
        let row = |row| EditAction::Row { row, delta: 1 };
        let knob = |k, ticks| EditAction::Knob {
            k,
            ticks,
            fine: false,
            turn: 1,
        };
        assert_eq!(value(&ed, &c, CHOP_ROW), "pick notes");
        assert_eq!(
            run(&mut ed, &mut c, row(CHOP_ROW)).line,
            "edit: pick notes first"
        );
        ed.sel = vec![0];
        assert_eq!(value(&ed, &c, CHOP_ROW), "open");
        // Chop opens in two; Q1 makes it four; the pieces share the
        // pressure.
        run(&mut ed, &mut c, row(CHOP_ROW));
        let (p, picks) = ed.preview(&c).unwrap();
        assert_eq!((p.notes.len(), picks.len()), (3, 2));
        run(&mut ed, &mut c, knob(0, 2));
        let (p, picks) = ed.preview(&c).unwrap();
        assert_eq!(picks.len(), 4);
        let last = picks
            .iter()
            .map(|&i| p.notes[i])
            .max_by_key(|n| n.tick)
            .unwrap();
        assert_eq!(p.touch_of(&last), [TouchPoint { at: 0, value: 90 }]);
        // Reset pressure: none on the pieces, lit on the bar.
        run(&mut ed, &mut c, EditAction::Tool(ToolAct::Option));
        assert_eq!(
            ed.open_tool.unwrap().option(),
            Some(("Reset pressure", true))
        );
        let (p, picks) = ed.preview(&c).unwrap();
        assert!(picks.iter().all(|&i| p.notes[i].touch.is_none()));
        assert_eq!(c, was);
        let done = run(&mut ed, &mut c, EditAction::Tool(ToolAct::Done));
        assert_eq!(done.line, "edit: 1 note chopped in 4");
        assert_eq!((c.notes.len(), ed.picked(&c).len()), (5, 4));
        // Legato on every note: half the gap to the next start.
        ed.sel = (0..c.notes.len()).collect();
        run(&mut ed, &mut c, row(LEGATO_ROW));
        assert_eq!(
            ed.open_tool.unwrap().cards()[0],
            ("Fill", "100%".to_string())
        );
        run(&mut ed, &mut c, knob(0, -10));
        let (p, _) = ed.preview(&c).unwrap();
        let mut lens: Vec<(Tick, Tick)> = p.notes.iter().map(|n| (n.tick, n.len)).collect();
        lens.sort_unstable();
        assert_eq!(
            lens,
            [(0, 240), (240, 240), (480, 240), (720, 720), (1920, 480)]
        );
        run(&mut ed, &mut c, EditAction::Tool(ToolAct::Cancel));
        // Humanize: a little at once; Again rolls again; the same dice,
        // the same result.
        run(&mut ed, &mut c, row(HUMANIZE_ROW));
        let first = ed.preview(&c).unwrap();
        assert_eq!(ed.preview(&c).unwrap(), first);
        run(&mut ed, &mut c, EditAction::Tool(ToolAct::Option));
        assert_ne!(ed.preview(&c).unwrap(), first);
        let cards = ed.open_tool.unwrap().cards();
        assert_eq!(
            cards.clone().map(|(name, _)| name),
            ["Timing", "Velocity", "Length", "Ahead/behind"]
        );
        assert_eq!(cards[3].1, "Even");
        run(&mut ed, &mut c, knob(3, -3));
        assert_eq!(ed.open_tool.unwrap().cards()[3].1, "Ahead 30");
        assert!(ed.open_tool.unwrap().bipolar(3));
        // A recorded note leaves the picks; UNDO taking one closes the
        // tool with the picks.
        let now = keys(&[(0, 960, 60)]);
        ed.follow(&c, &now);
        assert_eq!(ed.open_tool, None);
        assert!(ed.cleared);
    }

    fn roll(chord: ChordShape, minor: bool, scale: Option<Scale>) -> NoteGrid {
        NoteGrid {
            notes: vec![NoteBox::of(&Note::new(0, 240, 60, 100), false)],
            length: 3840,
            bar: 3840,
            view: RollView {
                lift: 57 * 40,
                row_h: 40,
                ..RollView::default()
            },
            scale,
            chord,
            minor,
            ..NoteGrid::default()
        }
    }

    fn target(g: &NoteGrid, id: &str, index: usize) -> Option<Rect> {
        g.targets(R)
            .into_iter()
            .find(|t| t.id == id && t.index == index)
            .map(|t| t.rect)
    }

    fn tap(g: &mut NoteGrid, f: u64, r: Rect) -> Option<EventKind> {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        let down = g.touch(f, Touch::Down, x, y, R);
        let up = g.touch(f, Touch::Up, x, y, R);
        down.or(up)
    }

    /// The Chord chip opens the Chord menu over the roll: Major and Minor
    /// (only with no scale) keep it open, a shape closes it, Pads shows
    /// only while the pads have a scale, a touch elsewhere closes it with
    /// nothing sent; no chip on a clip of pads. A note being drawn shows
    /// the whole chord.
    #[test]
    fn the_chord_chip_opens_the_chord_menu() {
        let mut g = roll(ChordShape::Off, false, None);
        let chip = target(&g, "chord-chip", 0).unwrap();
        assert_eq!(tap(&mut g, 1, chip), None);
        assert!(g.chord_menu);
        assert_eq!(
            target(&g, "chord-chip", 0),
            None,
            "the menu covers the roll"
        );
        assert_eq!(
            target(&g, "chord-pick", 7),
            None,
            "no Pads: the pads have no scale"
        );
        let minor = target(&g, "chord-pick", CHORD_MINOR).unwrap();
        assert_eq!(
            tap(&mut g, 2, minor),
            Some(EventKind::Tab(CHORD_MENU + CHORD_MINOR))
        );
        assert!(g.chord_menu, "Minor keeps it open for a shape");
        let triad = target(&g, "chord-pick", 1).unwrap();
        assert_eq!(tap(&mut g, 3, triad), Some(EventKind::Tab(CHORD_MENU + 1)));
        assert!(!g.chord_menu, "a shape closes it");
        g.chord_menu = true;
        let tools = g.button_rects(R)[0];
        assert_eq!(
            tap(&mut g, 4, tools),
            None,
            "a touch elsewhere only closes it"
        );
        assert!(!g.chord_menu);
        // With a scale, no Major or Minor; Pads with the pads' scale.
        let mut g = roll(ChordShape::Triad, false, Some(Scale::new(0, MAJOR)));
        g.offers = Arc::new(ScaleOffers {
            pads: Some(Scale::new(2, MINOR)),
            own: Vec::new(),
        });
        g.chord_menu = true;
        assert_eq!(target(&g, "chord-pick", CHORD_MAJOR), None);
        assert!(target(&g, "chord-pick", 7).is_some());
        // Drawing shows the chord: C E G on the C row.
        g.chord_menu = false;
        g.tool = Tool::Draw;
        assert_eq!(g.chord_on(60), [60, 64, 67]);
        // No chip on pads.
        let mut pads = roll(ChordShape::Triad, false, None);
        pads.pads = true;
        assert_eq!(target(&pads, "chord-chip", 0), None);
        assert_eq!(pads.chord_on(3), [3]);
    }

    /// A tool's bar (plan 2B.3) takes the buttons' place: Cancel, its own
    /// button and Done send the bar's tabs; the notes, their handles, the
    /// lane, the Scale and Chord chips and the gaps of the bar do nothing,
    /// but a second finger on the roll still moves the view; it closes
    /// the menus over the roll.
    #[test]
    fn a_tools_bar_takes_the_buttons_and_the_roll_only_moves_the_view() {
        let mut g = roll(ChordShape::Off, false, None);
        g.notes[0].selected = true;
        assert!(target(&g, "handle", 0).is_some());
        g.menu = true;
        let bar = ToolBar {
            name: "Chop",
            hint: "Turn the Q-Links: the roll shows the result".into(),
            option: Some(("Reset pressure", false)),
        };
        g.show(NoteGrid {
            tool_bar: Some(bar.clone()),
            ..g.clone()
        });
        assert!(!g.menu, "the bar closes the menus");
        let ids: Vec<&str> = g.targets(R).iter().map(|t| t.id).collect();
        for gone in [
            "button",
            "note",
            "handle",
            "lane",
            "roll",
            "scale-chip",
            "chord-chip",
        ] {
            assert!(!ids.contains(&gone), "{gone} in {ids:?}");
        }
        for id in [
            "tool-cancel",
            "tool-option",
            "tool-done",
            "fit",
            "lane-chip",
        ] {
            assert!(ids.contains(&id), "{id} not in {ids:?}");
        }
        assert_eq!(
            (g.button_label(0), g.button_label(1), g.button_label(7)),
            ("Cancel".into(), "Reset pressure".into(), "Done".into())
        );
        let tab = |k| Some(EventKind::Tab(TOOL_BUTTONS + k));
        let cancel = target(&g, "tool-cancel", 0).unwrap();
        assert_eq!(tap(&mut g, 1, cancel), tab(TOOL_CANCEL));
        let option = target(&g, "tool-option", 0).unwrap();
        assert_eq!(tap(&mut g, 2, option), tab(TOOL_OPTION));
        let done = target(&g, "tool-done", 0).unwrap();
        assert_eq!(tap(&mut g, 3, done), tab(TOOL_DONE));
        // Between them, nothing; the note and the handle, nothing.
        let gap = g.button_rects(R)[4];
        assert_eq!(tap(&mut g, 4, gap), None);
        let note = g.note_rect(R, 0).unwrap();
        assert_eq!(tap(&mut g, 5, note), None);
        let (x, y) = (note.right() + HANDLE_W / 2, note.y + note.h / 2);
        assert_eq!(g.touch(6, Touch::Down, x, y, R), None);
        assert_eq!(g.touch(6, Touch::Move, x + 80, y, R), None);
        assert_eq!(g.touch(6, Touch::Up, x + 80, y, R), None);
        // A second finger on the roll: a pinch.
        let roll = g.roll_rect(R);
        let (x, y) = (roll.x + 200, roll.y + 100);
        g.touch(7, Touch::Down, x, y, R);
        g.touch(8, Touch::Down, x + 100, y, R);
        assert!(matches!(g.hand, Hand::Pinch(_)));
        g.touch(8, Touch::Move, x + 300, y, R);
        let up = g
            .touch(8, Touch::Up, x + 300, y, R)
            .or(g.touch(7, Touch::Up, x, y, R));
        assert!(matches!(up, Some(EventKind::View { .. })), "{up:?}");
        // No option button without one: the hint takes its room.
        g.tool_bar = Some(ToolBar {
            option: None,
            ..bar
        });
        assert_eq!(target(&g, "tool-option", 0), None);
        let gap = g.button_rects(R)[1];
        assert_eq!(tap(&mut g, 9, gap), None);
    }
}

#[cfg(test)]
mod ghost_tests {
    use super::*;

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 1100,
        h: 640,
    };

    fn roll(draw: bool) -> NoteGrid {
        NoteGrid {
            notes: vec![NoteBox::of(&Note::new(0, 240, 60, 100), false)],
            length: 3840,
            bar: 3840,
            view: RollView {
                lift: 57 * 40,
                row_h: 40,
                ..RollView::default()
            },
            tool: if draw { Tool::Draw } else { Tool::Select },
            ghosts: vec![GhostLayer {
                track: 3,
                slot: 1,
                name: "Bass · Line".into(),
                with: true,
                color: crate::view::track_color(3),
                phase: 0.0,
                notes: vec![Note::new(1920, 480, 60, 90), Note::new(0, 240, 60, 90)],
            }],
            show_offers: (0..40)
                .map(|k| ShowOffer {
                    track: k / 4,
                    slot: k % 4,
                    name: format!("T{} · C{}", k / 4, k % 4),
                    with: k < 3,
                    shown: k == 1,
                })
                .collect(),
            ..NoteGrid::default()
        }
    }

    fn target(g: &NoteGrid, id: &str, index: usize) -> Option<Rect> {
        g.targets(R)
            .into_iter()
            .find(|t| t.id == id && t.index == index)
            .map(|t| t.rect)
    }

    fn tap(g: &mut NoteGrid, f: u64, r: Rect) -> Option<EventKind> {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        let down = g.touch(f, Touch::Down, x, y, R);
        let up = g.touch(f, Touch::Up, x, y, R);
        down.or(up)
    }

    /// Show picks up to three other clips (plan 4.1), never the clip
    /// itself; a pick again takes it off, and None takes them all off.
    #[test]
    fn show_picks_up_to_three_other_clips_and_none_takes_them_off() {
        let c = Clip::new("Keys", 3840);
        let mut ed = Editor::new(1, 0, &c);
        for (track, slot) in [(0, 0), (2, 0), (3, 1)] {
            ed.act(EditAction::Show { track, slot }, &c);
        }
        assert_eq!(ed.show, [(0, 0), (2, 0), (3, 1)]);
        let d = ed.act(EditAction::Show { track: 4, slot: 0 }, &c);
        assert!(d.line.contains("3 clips show already"), "{}", d.line);
        assert_eq!(ed.show.len(), 3);
        ed.act(EditAction::Show { track: 2, slot: 0 }, &c);
        assert_eq!(ed.show, [(0, 0), (3, 1)]);
        ed.act(EditAction::Show { track: 1, slot: 0 }, &c);
        assert_eq!(ed.show.len(), 2, "never the clip itself");
        assert!(ed.act(EditAction::ShowNone, &c).edit.is_empty());
        assert!(ed.show.is_empty());
    }

    /// The Show chip opens its menu over the roll: an offer is picked by
    /// its place in the list and the menu stays open; More pages; None;
    /// a touch off it closes it.
    #[test]
    fn the_show_menu_picks_pages_and_closes() {
        let mut g = roll(false);
        let chip = target(&g, "show-chip", 0).expect("Show on the overview row");
        assert_eq!(tap(&mut g, 1, chip), None);
        assert!(g.show_menu);
        let first = target(&g, "show-pick", 0).unwrap();
        assert_eq!(tap(&mut g, 2, first), Some(EventKind::Tab(SHOW_MENU)));
        assert!(g.show_menu, "it stays open for more");
        let more = target(&g, "show-more", 0).expect("40 offers don't fit");
        let on_page = g.targets(R).iter().filter(|t| t.id == "show-pick").count();
        assert_eq!(tap(&mut g, 3, more), None);
        let next = target(&g, "show-pick", on_page).expect("the next page's first");
        assert_eq!(
            tap(&mut g, 4, next),
            Some(EventKind::Tab(SHOW_MENU + on_page))
        );
        let none = target(&g, "show-none", 0).unwrap();
        assert_eq!(tap(&mut g, 5, none), Some(EventKind::Tab(SHOW_NONE)));
        let tools = g.button_rects(R)[0];
        assert_eq!(tap(&mut g, 6, tools), None, "a touch off it only closes it");
        assert!(!g.show_menu);
    }

    /// With Draw off, a tap on another clip's faint note opens that clip,
    /// keeping where it was tapped for COPY; one of this clip's notes on
    /// top wins; with Draw on, the tap draws as on empty roll.
    #[test]
    fn a_tap_on_a_faint_note_opens_its_clip_with_draw_off() {
        let mut g = roll(false);
        let faint = target(&g, "ghost-note", 0).expect("the faint note in sight");
        assert_eq!(tap(&mut g, 1, faint), Some(EventKind::Tab(GHOST_TAP)));
        assert_eq!(g.ghost_cell.0, 60);
        assert!((1920..2400).contains(&g.ghost_cell.1), "{:?}", g.ghost_cell);
        // The faint note under this clip's own: the own note is tapped.
        let geo = g.geo(R);
        let own = g.ghost_rect(&geo, &Note::new(0, 240, 60, 90)).unwrap();
        assert_eq!(tap(&mut g, 2, own), Some(EventKind::Toggle(0)));
        let mut d = roll(true);
        assert_eq!(target(&d, "ghost-note", 0), None, "Draw on: it draws");
        assert!(matches!(
            tap(&mut d, 3, faint),
            Some(EventKind::Cell { row: 60, .. })
        ));
    }
}

/// Drum clips (piano roll plan 4.2): a row a pad, named, and PAD BANK.
#[cfg(test)]
mod pad_row_tests {
    use super::*;

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 1100,
        h: 640,
    };

    fn drums(pads: &[u8]) -> NoteGrid {
        let mut names = vec![String::new(); 128];
        names[0] = "Kick".into();
        names[1] = "Snare".into();
        NoteGrid {
            notes: pads
                .iter()
                .map(|&p| NoteBox::of(&Note::new(0, 240, p, 100), false))
                .collect(),
            length: 3840,
            bar: 3840,
            pads: true,
            pad_names: Arc::new(names),
            ..NoteGrid::default()
        }
    }

    /// The bottom row the roll shows.
    fn lowest(g: &NoteGrid) -> u8 {
        let geo = g.geo(R);
        let row = *geo.v.notes_shown(geo.roll, geo.rows.n).start();
        geo.rows.note_of(row).unwrap()
    }

    /// Clip::holds alone says a clip plays the pads: a kit clip with a
    /// note past the first bank still does, a clip of notes on low notes
    /// doesn't.
    #[test]
    fn a_clip_of_pad_hits_is_drums_whatever_its_notes() {
        let mut c = Clip::new("Beat", 3840);
        c.add_note(Note::new(0, 240, 20, 100)).unwrap();
        assert!(pads(&c), "pad B5 is a pad");
        c.holds = mpfree_seq::Holds::Notes;
        c.notes[0].note = 3;
        assert!(!pads(&c), "a note low down is a note");
        assert_eq!(
            (pad_place(0), pad_place(20), pad_place(127)),
            ("A1".into(), "B5".into(), "H16".into())
        );
        assert_eq!(sound_name(20, true), "Pad B5");
        assert_eq!(one_named(&c, 60), "note C3");
    }

    /// Each of the kit's 128 pads has a row, named by its place and the
    /// kit's name for it; the key column is wider for the names.
    #[test]
    fn a_drum_clip_has_a_named_row_for_each_pad() {
        let g = drums(&[0, 1, 20]);
        assert_eq!(g.rows(), 128);
        assert_eq!(
            (g.row_name(0), g.row_name(1), g.row_name(20)),
            ("A1 Kick".into(), "A2 Snare".into(), "B5".into())
        );
        let keys = |g: &NoteGrid| g.geo(R).keys.w;
        let notes = NoteGrid {
            pads: false,
            ..drums(&[60])
        };
        assert!(keys(&g) > keys(&notes), "room for the names");
        // No scale and no chords on pads: Fold alone.
        let geo = g.geo(R);
        assert!(geo.scale_chip.is_empty() && geo.chord_chip.is_empty());
        assert!(!geo.fold_chip.is_empty());
        assert_eq!(notes.row_name(60), "C3");
    }

    /// PAD BANK scrolls a roll of pad rows to the bank's first pad, the
    /// zoom kept; with Fold on Rows with notes, to the first row there
    /// from there up. An empty drum clip opens on the bank the pads play;
    /// a clip of notes doesn't move.
    #[test]
    fn pad_bank_scrolls_to_its_banks_first_pad() {
        let mut g = drums(&[0, 1, 20]);
        g.view = g.open_view(R);
        let row_h = g.view.row_h;
        assert_eq!(lowest(&g), 0, "the notes from A1");
        let v = g.show_bank(R, 1).expect("a roll of pad rows");
        assert_eq!((lowest(&g), v.row_h, g.bank), (16, row_h, 1));
        g.show_bank(R, 7);
        assert!(lowest(&g) > 16, "as far up as the rows go");
        // Folded to 24 rows with notes, more than show.
        let pads: Vec<u8> = (0..13).chain(20..31).collect();
        let mut g = NoteGrid {
            fold: Fold::Notes,
            ..drums(&pads)
        };
        g.view = g.open_view(R);
        g.show_bank(R, 1);
        assert_eq!(lowest(&g), 20, "B5, the first pad with notes from B1");
        // Empty, on bank C.
        let empty = NoteGrid {
            bank: 2,
            ..drums(&[])
        };
        let open = NoteGrid {
            view: empty.open_view(R),
            ..empty
        };
        assert_eq!(lowest(&open), 32);
        // Notes and the list stay where they are.
        let mut keys = NoteGrid {
            pads: false,
            ..drums(&[60])
        };
        assert_eq!(keys.show_bank(R, 1), None);
        let mut list = NoteGrid {
            lens: Lens::List,
            ..drums(&[0])
        };
        assert_eq!(list.show_bank(R, 1), None);
    }
}
