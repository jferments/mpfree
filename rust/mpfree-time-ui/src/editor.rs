//! The note editor of the Arrange page (the MPC's Grid View and List
//! Edit): one clip's notes on a piano roll or as a list, the tools along
//! the bottom, and beside it the Edit options: Timing Correct, and the
//! selection's nudge, transpose, velocity and length.
//!
//! [`Editor`] is the editor's state (the clip, the selection, the view,
//! the tool, the snap and the Timing Correct settings). [`Editor::act`]
//! turns an [`EditAction`] into an [`Edit`] from [`mpfree_seq::notes`],
//! which the app applies as one undoable take, then calls
//! [`Editor::applied`] so the selection follows the notes.
//!
//! [`NoteGrid`] draws the clip and reports touches:
//!
//! - a note (on the roll or a row of the list): `Select(index)`;
//! - empty space on the roll: `Cell { row: note, col: tick }`, the tick
//!   under the finger snapped down to the snap's line;
//! - a note dragged: when the finger lifts, `Marker { index, frame }`,
//!   the move packed into `frame` ([`drop_frame`], [`drop_of`]);
//! - the velocity lane: `Value(v)`, 0 to 1, as the finger goes down and
//!   moves;
//! - the buttons along the bottom: `Tab(k)` for [`EDIT_BUTTONS`]`[k]`.
//!
//! [`edit_rows`] fills the Edit options list; its `Step` and `Select`
//! become [`EditAction::Row`] and [`EditAction::PickRow`].

use mpfree_seq::notes::{self, Snap, Tc, TcKind, Which, DIVISIONS};
use mpfree_seq::steps::Edit;
use mpfree_seq::{Clip, Note};
use mpfree_transport::time::{Tick, PPQ};
use mpfree_ui::widgets::OptionRow;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Touch, Widget};

/// The buttons along the bottom, left to right. Some show a state in
/// place of their name (see [`NoteGrid::button_label`]).
pub const EDIT_BUTTONS: [&str; 8] = [
    "‹ Clips",
    "List",
    "Tool",
    "Snap",
    "All",
    "Duplicate",
    "Delete",
    "TC",
];
/// The Edit options, top to bottom. The first six are Timing Correct's
/// settings (TC applies them); the last four change the selection at once.
pub const EDIT_ROWS: [&str; 10] = [
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
];
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
    /// The list in place of the roll.
    pub list: bool,
    pub tool: Tool,
    /// Index into the divisions, or [`DIVISIONS`]`.len()` for no snap.
    pub snap: usize,
    pub tc: Tc,
    /// The Edit option picked (the wheel steps it).
    pub row: usize,
    /// The roll's lowest note.
    pub low: u8,
    /// The list's first row.
    pub first: usize,
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
        Editor {
            track,
            slot,
            sel: Vec::new(),
            list: false,
            tool: Tool::Select,
            snap: DEFAULT_SNAP,
            tc: Tc::default(),
            row: 0,
            low,
            first: 0,
        }
    }

    /// The snap in ticks, or 0 for none.
    pub fn snap_ticks(&self) -> Tick {
        DIVISIONS.get(self.snap).map_or(0, |d| d.0)
    }

    pub fn snap_name(&self) -> &'static str {
        DIVISIONS.get(self.snap).map_or("Off", |d| d.1)
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
                self.list = !self.list;
                return Done::line(
                    if self.list {
                        "edit: list"
                    } else {
                        "edit: grid"
                    }
                    .into(),
                );
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
                // moves it; on another note it selects that one.
                _ => {
                    if !sel.contains(&i) {
                        self.sel = vec![i];
                    }
                    return Done::default();
                }
            },
            EditAction::Note(_) => return Done::default(),
            EditAction::Empty { tick, note } => match self.tool {
                Tool::Draw if tick < clip.length => {
                    let n = Note::new(tick, self.step(), note.min(127), DRAW_VELOCITY);
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
                    _ => return Done::default(),
                }
            }
            EditAction::Scroll(d) => {
                if self.list {
                    let last = clip.notes.len().saturating_sub(1) as i64;
                    self.first = (self.first as i64 + i64::from(d)).clamp(0, last.max(0)) as usize;
                } else {
                    self.low = (i32::from(self.low) + d).clamp(0, 127) as u8;
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
        }
    }

    /// After the app applied `done`'s edit to `clip`: the selection is the
    /// notes the edit put in (when it selects them) and the selected notes
    /// it left alone.
    pub fn applied(&mut self, clip: &Clip, done: &Done) {
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
            None => "select notes".into(),
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
            "select notes".into()
        } else {
            format!("by {}", ed.snap_name())
        },
        of_sel(&|n| i64::from(n.note), &|v| note_name(v as u8)),
        of_sel(&|n| i64::from(n.velocity), &|v| v.to_string()),
        of_sel(&|n| n.len, &length_name),
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
    pub selected: bool,
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
    pub list: bool,
    /// The list's first row.
    pub first: usize,
    pub color: Color,
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
            list: false,
            first: 0,
            color: Color::rgb(0x6EA8FE),
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
            && (self.snap, self.tool, self.list, self.first)
                == (other.snap, other.tool, other.list, other.first)
            && self.snap_name == other.snap_name
            && self.color == other.color
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
        let (vel, rest) = if self.list {
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

    /// Where row `k` of the list is (0 is the heading), if it shows.
    pub fn list_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let g = self.geo(r);
        let area = Rect::new(
            g.keys.x,
            g.ruler.y,
            g.keys.w + g.roll.w,
            g.roll.bottom() - g.ruler.y,
        );
        let y = area.y + k as i32 * LIST_H;
        (y + LIST_H <= area.bottom()).then(|| Rect::new(area.x, y, area.w, LIST_H))
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
            1 => if self.list { "Grid" } else { "List" }.into(),
            2 => self.tool.name().into(),
            3 => format!("Snap {}", self.snap_name),
            4 => if any { "None" } else { "All" }.into(),
            7 => if any { "TC sel." } else { "TC all" }.into(),
            _ => EDIT_BUTTONS.get(k).copied().unwrap_or("").into(),
        }
    }

    fn row_name(&self, note: u8) -> String {
        if self.pads {
            format!("Pad {}", note + 1)
        } else {
            note_name(note)
        }
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
                String::new(),
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
        if self.list {
            self.paint_list(cx, r);
        } else {
            self.paint_roll(cx, r, &g);
        }
        for (k, br) in self.button_rects(r).iter().enumerate() {
            let lit = (k == 1 && self.list) || (k == 2 && self.tool != Tool::Select);
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

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = self.geo(r);
        match phase {
            Touch::Down => {
                if let Some(k) = self.button_rects(r).iter().position(|b| b.contains(x, y)) {
                    return Some(EventKind::Tab(k));
                }
                if self.list {
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
        assert_eq!(edit_rows(&ed, &c)[8].value, "select notes");
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
        ed.act(EditAction::Button(1), &c);
        assert!(ed.list);
        ed.act(EditAction::Scroll(5), &c);
        assert_eq!(ed.first, 0, "one note: the list cannot scroll");
        ed.act(EditAction::Button(1), &c);
        ed.act(EditAction::Scroll(-12), &c);
        assert_eq!(ed.low, 48);
        assert!(ed.act(EditAction::Button(0), &c).close);
        ed.act(EditAction::Row { row: 0, delta: -1 }, &c);
        assert_eq!(ed.tc.kind, TcKind::Legato);
        ed.act(EditAction::Row { row: 2, delta: 1 }, &c);
        assert_eq!(ed.tc.swing, 51);
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
                NoteBox {
                    tick: 0,
                    len: 240,
                    note: 50,
                    velocity: 100,
                    selected: false,
                },
                NoteBox {
                    tick: 960,
                    len: 480,
                    note: 52,
                    velocity: 64,
                    selected: true,
                },
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
        g.list = true;
        // Row 1 is the first note in time order.
        let row = g.list_rect(R, 2).unwrap();
        assert_eq!(
            g.touch(2, Touch::Down, mid(row).0, mid(row).1, R),
            Some(EventKind::Select(1))
        );
        assert_eq!(g.button_label(1), "Grid");
    }

    #[test]
    fn it_paints_both_views() {
        use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};
        let mut g = grid();
        g.playhead = Some(1000);
        for list in [false, true] {
            g.list = list;
            let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), R.w, R.h);
            ui.add("notes", g.clone());
            ui.add_page("Edit", "col(notes:*)").unwrap();
            let mut c = SoftCanvas::new(R.w, R.h);
            assert!(!ui.paint(&mut c).is_empty());
        }
    }
}
