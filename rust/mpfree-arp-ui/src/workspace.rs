//! The pattern workspace: a full screen for making and changing patterns.
//!
//! It opens from the Arp panel's Edit pattern or Record on pads, and Back
//! returns to the page you came from, with the pattern playing all the
//! while. The grid has a column per step and a row per part of a step:
//!
//! - **Step**: rest, note or tie. A tap cycles rest, note and accent.
//! - **Note**: which held note plays: the next in the order (`x`), the 1st
//!   to 9th lowest, all of them, a random one, the lowest or the highest.
//! - **Octave**, **Ratchet** (hits in the step), **Chance** and **Gate**.
//!
//! Drag a cell up or down, or turn the wheel, to change its value. Mutate
//! and Rotate change a copy; Original puts back the pattern the workspace
//! opened with.
//!
//! **Lanes** switches the grid to the pattern's lanes: octave, degree,
//! velocity, gate and nudge, a row each. Every lane loops at its own length,
//! so a 3-value velocity lane over 4 steps makes a polymeter. Drag a value
//! or turn the wheel; drag the `+` cell at a lane's end to add a value;
//! Length −/+ and Random −/+ work on the selected lane.
//!
//! **Record** writes the pattern on the pads. In Step time each press writes
//! the next step. In Free time the pattern plays over the latched notes and
//! each press lands on the nearest step as it goes by.
//!
//! [`Editor`] is the model and has no drawing, so it is tested on its own.
//! [`PatternWorkspace`] adds the widgets and two layouts to the app's `Ui`:
//! one for editing, and one with the pads for recording on them.

use crate::view::pattern_cell;
use crate::widgets::{Button, ButtonRow, CellKind, LaneGrid, StepCell};
use mpfree_arp::{Lane, Pattern, Pick, RecordMode, Step, MAX_STEPS};
use mpfree_ui::widgets::Label;
use mpfree_ui::{Align, EventKind, Tone, Ui, UiEvent};

/// Widget names the workspace adds. The app's own are `header`, `pads` and
/// `hint`.
pub const TITLE: &str = "arp-ws-title";
pub const GRID: &str = "arp-ws-grid";
pub const TOOLS: &str = "arp-ws-tools";

/// Editing: the grid takes the whole screen.
pub const EDIT_LAYOUT: &str = "col(header:60, arp-ws-title:44, arp-ws-grid:*, arp-ws-tools:72, hint:26)";
/// Recording on the pads: the pads, named for what each one writes, beside
/// the grid.
pub const RECORD_LAYOUT: &str =
    "col(header:60, arp-ws-title:44, row(pads:*, arp-ws-grid:780):*, arp-ws-tools:72, hint:26)";

/// Steps shown at once while editing; longer patterns page with the cursor.
pub const COLUMNS: usize = 16;
/// Steps shown at once while recording, when the pads take half the screen.
pub const RECORD_COLUMNS: usize = 8;

/// The grid's rows, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Step,
    Note,
    Octave,
    Ratchet,
    Chance,
    Gate,
}

impl Row {
    pub const ALL: [Row; 6] = [Row::Step, Row::Note, Row::Octave, Row::Ratchet, Row::Chance, Row::Gate];

    pub const fn label(self) -> &'static str {
        match self {
            Row::Step => "Step",
            Row::Note => "Note",
            Row::Octave => "Octave",
            Row::Ratchet => "Ratchet",
            Row::Chance => "Chance",
            Row::Gate => "Gate",
        }
    }

    fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// The lanes view's rows, top to bottom: the pattern's lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneRow {
    Octave,
    Degree,
    Velocity,
    Gate,
    Nudge,
}

impl LaneRow {
    pub const ALL: [LaneRow; 5] = [LaneRow::Octave, LaneRow::Degree, LaneRow::Velocity, LaneRow::Gate, LaneRow::Nudge];

    pub const fn label(self) -> &'static str {
        match self {
            LaneRow::Octave => "Octave",
            LaneRow::Degree => "Degree",
            LaneRow::Velocity => "Velocity",
            LaneRow::Gate => "Gate",
            LaneRow::Nudge => "Nudge",
        }
    }

    /// (lowest value, highest value, one wheel click or drag step, the most
    /// random), as the pattern text allows them.
    pub const fn range(self) -> (i32, i32, i32, i32) {
        match self {
            LaneRow::Octave => (-3, 3, 1, 3),
            LaneRow::Degree => (-24, 24, 1, 24),
            LaneRow::Velocity => (0, 127, 8, 127),
            LaneRow::Gate => (0, 100, 10, 100),
            LaneRow::Nudge => (-50, 50, 5, 50),
        }
    }

    fn index(self) -> usize {
        LaneRow::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// A lane's values and its random amount.
pub fn lane_values(p: &Pattern, lane: LaneRow) -> (Vec<i32>, u8) {
    fn get<T: Copy + Default + Into<i32>>(l: &Lane<T>) -> (Vec<i32>, u8) {
        (l.values().iter().map(|&v| v.into()).collect(), l.random())
    }
    let l = &p.lanes;
    match lane {
        LaneRow::Octave => get(&l.octave),
        LaneRow::Degree => get(&l.degree),
        LaneRow::Velocity => get(&l.velocity),
        LaneRow::Gate => get(&l.gate),
        LaneRow::Nudge => get(&l.nudge),
    }
}

/// `p` with one lane replaced. Values are clamped to the lane's range.
pub fn with_lane(p: &Pattern, lane: LaneRow, values: &[i32], random: u8) -> Option<Pattern> {
    let (lo, hi, _, most) = lane.range();
    let v: Vec<i32> = values.iter().map(|x| (*x).clamp(lo, hi)).collect();
    let random = random.min(most as u8);
    let mut q = *p;
    let small = |v: &[i32]| -> Vec<i8> { v.iter().map(|&x| x as i8).collect() };
    let byte = |v: &[i32]| -> Vec<u8> { v.iter().map(|&x| x as u8).collect() };
    match lane {
        LaneRow::Octave => q.lanes.octave = Lane::new(&small(&v))?.with_random(random),
        LaneRow::Degree => q.lanes.degree = Lane::new(&small(&v))?.with_random(random),
        LaneRow::Velocity => q.lanes.velocity = Lane::new(&byte(&v))?.with_random(random),
        LaneRow::Gate => q.lanes.gate = Lane::new(&byte(&v))?.with_random(random),
        LaneRow::Nudge => q.lanes.nudge = Lane::new(&small(&v))?.with_random(random),
    }
    Some(q)
}

/// How a lane value reads in its cell.
pub fn lane_label(lane: LaneRow, v: i32) -> String {
    let signed = |v: i32| if v > 0 { format!("+{v}") } else { format!("{v}") };
    match lane {
        LaneRow::Octave | LaneRow::Degree => signed(v),
        LaneRow::Velocity | LaneRow::Gate if v == 0 => "auto".into(),
        LaneRow::Velocity => format!("{v}"),
        LaneRow::Gate => format!("{v}%"),
        LaneRow::Nudge => format!("{}%", signed(v)),
    }
}

/// The note picks in the order the wheel and drags move through them.
const PICKS: [Pick; 14] = [
    Pick::Next,
    Pick::Index(1),
    Pick::Index(2),
    Pick::Index(3),
    Pick::Index(4),
    Pick::Index(5),
    Pick::Index(6),
    Pick::Index(7),
    Pick::Index(8),
    Pick::Index(9),
    Pick::All,
    Pick::Random,
    Pick::Lowest,
    Pick::Highest,
];

/// The pattern being edited, the one it started from, and the cursor.
#[derive(Clone, Debug)]
pub struct Editor {
    original: Pattern,
    pattern: Pattern,
    /// The selected row and step.
    pub cursor: (Row, usize),
    /// The selected lane and value in the lanes view.
    pub lane_cursor: (LaneRow, usize),
    mutations: u32,
}

impl Editor {
    pub fn new(pattern: Pattern) -> Editor {
        Editor { original: pattern, pattern, cursor: (Row::Step, 0), lane_cursor: (LaneRow::Velocity, 0), mutations: 0 }
    }

    pub fn pattern(&self) -> &Pattern {
        &self.pattern
    }

    pub fn original(&self) -> &Pattern {
        &self.original
    }

    /// The pattern differs from the one the workspace opened with.
    pub fn changed(&self) -> bool {
        self.pattern != self.original
    }

    /// Selects a cell. Steps past the end are refused.
    pub fn select(&mut self, row: Row, step: usize) -> bool {
        if step < self.pattern.len() {
            self.cursor = (row, step);
            true
        } else {
            false
        }
    }

    /// A tap on the Step row: rest, then note, then accent, then rest
    /// again. A tie becomes a rest. A note keeps its pick and the rest of
    /// its settings when it gains or loses its accent.
    pub fn tap(&mut self, step: usize) -> bool {
        let Some(&s) = self.pattern.steps().get(step) else { return false };
        let next = match s {
            Step::Tie => Step::Rest,
            Step::Rest => Step::play(Pick::Next),
            Step::Play { accent: false, pick, octave, gate, ratchet, chance } => {
                Step::Play { pick, octave, accent: true, gate, ratchet, chance }
            }
            Step::Play { accent: true, .. } => Step::Rest,
        };
        self.pattern.set_step(step, next)
    }

    /// Moves the value in `row` at `step` by `delta` (a wheel click or a
    /// drag). Values stop at their ends rather than wrap. Rows other than
    /// Step do nothing on a rest or a tie.
    pub fn turn(&mut self, row: Row, step: usize, delta: i32) -> bool {
        let Some(&s) = self.pattern.steps().get(step) else { return false };
        let next = match (row, s) {
            (Row::Step, _) => {
                // Rest, tie, note.
                let now = match s {
                    Step::Rest => 0,
                    Step::Tie => 1,
                    Step::Play { .. } => 2,
                };
                let to = (now + delta).clamp(0, 2);
                match (to, s) {
                    (0, _) => Step::Rest,
                    (1, _) => Step::Tie,
                    (_, Step::Play { .. }) => s,
                    _ => Step::play(Pick::Next),
                }
            }
            (_, Step::Rest | Step::Tie) => return false,
            (_, Step::Play { pick, octave, accent, gate, ratchet, chance }) => {
                let mut p = (pick, octave, accent, gate, ratchet, chance);
                match row {
                    Row::Note => {
                        let at = PICKS.iter().position(|x| *x == pick).unwrap_or(0) as i32;
                        p.0 = PICKS[(at + delta).clamp(0, PICKS.len() as i32 - 1) as usize];
                    }
                    Row::Octave => p.1 = (i32::from(octave) + delta).clamp(-3, 3) as i8,
                    Row::Ratchet => p.4 = (i32::from(ratchet) + delta).clamp(1, 8) as u8,
                    Row::Chance => p.5 = ((i32::from(chance) + 9) / 10 * 10 + 10 * delta).clamp(10, 100) as u8,
                    // 0 is the arp's own gate setting ("auto").
                    Row::Gate => p.3 = ((i32::from(gate) + 9) / 10 * 10 + 10 * delta).clamp(0, 100) as u8,
                    Row::Step => {}
                }
                Step::Play { pick: p.0, octave: p.1, accent: p.2, gate: p.3, ratchet: p.4, chance: p.5 }
            }
        };
        next != s && self.pattern.set_step(step, next)
    }

    /// Makes the pattern `len` steps long (1 to [`MAX_STEPS`]). New steps are
    /// rests; the lanes are kept.
    pub fn set_len(&mut self, len: usize) -> bool {
        if len == 0 || len > MAX_STEPS || len == self.pattern.len() {
            return false;
        }
        let mut steps = [Step::Rest; MAX_STEPS];
        let have = self.pattern.len().min(len);
        steps[..have].copy_from_slice(&self.pattern.steps()[..have]);
        let Some(mut p) = Pattern::from_steps(&steps[..len]) else { return false };
        p.lanes = self.pattern.lanes;
        self.pattern = p;
        self.cursor.1 = self.cursor.1.min(len - 1);
        true
    }

    pub fn rotate(&mut self, n: i32) {
        self.pattern = self.pattern.rotated(n);
    }

    /// A new variation of the pattern as it is now. Each press uses the
    /// next seed, so the same presses give the same variations.
    pub fn mutate(&mut self) {
        self.mutations += 1;
        self.pattern = self.pattern.mutated(self.mutations, 35);
    }

    /// Back to the pattern the workspace opened with.
    pub fn revert(&mut self) {
        self.pattern = self.original;
        self.cursor.1 = self.cursor.1.min(self.pattern.len() - 1);
        self.clamp_lane_cursor();
    }

    /// Selects a lane value, or the empty cell just past the lane's end.
    pub fn select_lane(&mut self, lane: LaneRow, pos: usize) -> bool {
        let len = lane_values(&self.pattern, lane).0.len();
        if pos <= len && pos < MAX_STEPS {
            self.lane_cursor = (lane, pos);
            true
        } else {
            false
        }
    }

    fn set_lane(&mut self, lane: LaneRow, values: &[i32], random: u8) -> bool {
        match with_lane(&self.pattern, lane, values, random) {
            Some(p) if p != self.pattern => {
                self.pattern = p;
                true
            }
            _ => false,
        }
    }

    fn clamp_lane_cursor(&mut self) {
        let (lane, pos) = self.lane_cursor;
        let len = lane_values(&self.pattern, lane).0.len();
        self.lane_cursor.1 = pos.min(len).min(MAX_STEPS - 1);
    }

    /// Moves a lane value by `delta` clicks of the lane's step, stopping at
    /// its ends. Turning the empty cell just past the end adds a value
    /// there, starting from the one before it.
    pub fn turn_lane(&mut self, lane: LaneRow, pos: usize, delta: i32) -> bool {
        let (mut v, random) = lane_values(&self.pattern, lane);
        if pos > v.len() || pos >= MAX_STEPS {
            return false;
        }
        if pos == v.len() {
            v.push(v.last().copied().unwrap_or(0));
        }
        let (lo, hi, step, _) = lane.range();
        v[pos] = (v[pos] + step * delta).clamp(lo, hi);
        let changed = self.set_lane(lane, &v, random);
        if changed {
            self.lane_cursor = (lane, pos);
        }
        changed
    }

    /// Makes a lane `len` values long (0 turns it off). New values repeat
    /// the last one.
    pub fn set_lane_len(&mut self, lane: LaneRow, len: usize) -> bool {
        if len > MAX_STEPS {
            return false;
        }
        let (mut v, random) = lane_values(&self.pattern, lane);
        let fill = v.last().copied().unwrap_or(0);
        v.resize(len, fill);
        let changed = self.set_lane(lane, &v, random);
        self.clamp_lane_cursor();
        changed
    }

    /// Moves a lane's random amount (how far each value wanders as it
    /// plays) by `delta` clicks of the lane's step.
    pub fn turn_random(&mut self, lane: LaneRow, delta: i32) -> bool {
        let (v, random) = lane_values(&self.pattern, lane);
        let (_, _, step, most) = lane.range();
        let r = (i32::from(random) + step * delta).clamp(0, most);
        self.set_lane(lane, &v, r as u8)
    }

    /// Turns a lane off: no values and no random amount.
    pub fn clear_lane(&mut self, lane: LaneRow) -> bool {
        let changed = self.set_lane(lane, &[], 0);
        self.clamp_lane_cursor();
        changed
    }
}

/// One cell of the grid: what `row` shows for `step`.
pub fn grid_cell(row: Row, step: Step) -> StepCell {
    let quiet = |label: &str, kind| StepCell { kind, label: label.to_string(), ..StepCell::default() };
    let Step::Play { pick, octave, accent, gate, ratchet, chance } = step else {
        return match (row, step) {
            (Row::Step, Step::Tie) => quiet("—", CellKind::Tie),
            (Row::Step, _) => quiet("·", CellKind::Rest),
            _ => quiet("", CellKind::Rest),
        };
    };
    let kind = match pick {
        Pick::All => CellKind::Chord,
        Pick::Random => CellKind::Random,
        _ => CellKind::Note,
    };
    let plain = |label: String| StepCell { kind, label, ..StepCell::default() };
    match row {
        Row::Step => StepCell { kind: CellKind::Note, label: "●".into(), accent, ..StepCell::default() },
        Row::Note => {
            let c = pattern_cell(Step::play(pick));
            StepCell { kind, label: c.label, ..StepCell::default() }
        }
        Row::Octave => plain(match octave {
            0 => "0".into(),
            o if o > 0 => format!("+{o}"),
            o => format!("{o}"),
        }),
        Row::Ratchet => plain(if ratchet > 1 { format!("×{ratchet}") } else { "1".into() }),
        Row::Chance => plain(format!("{chance}%")),
        Row::Gate => plain(if gate == 0 { "auto".into() } else { format!("{gate}%") }),
    }
}

/// What each pad writes while recording on the pads (the decided layout:
/// pads 1 to 9 the 1st to 9th held note, 13 the next note, 14 random, 15 a
/// tie, 16 a rest; 10 to 12 do nothing yet).
pub fn record_pad_labels() -> [&'static str; 16] {
    [
        "1st note", "2nd note", "3rd note", "4th note", "5th note", "6th note", "7th note", "8th note", "9th note", "",
        "", "", "Next", "Random", "Tie", "Rest",
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Back,
    RotateLeft,
    RotateRight,
    Mutate,
    Original,
    Shorter,
    Longer,
    Record,
    Save,
    /// The lanes view, and back to the steps.
    Lanes,
    Steps,
    LaneShorter,
    LaneLonger,
    RandomLess,
    RandomMore,
    ClearLane,
    /// While recording: Step time or Free time, Undo and Clear.
    StepTime,
    FreeTime,
    Undo,
    Clear,
}

/// The toolbar while editing steps.
pub const STEP_TOOLS: [(Tool, &str, Tone); 10] = [
    (Tool::Back, "Back", Tone::Normal),
    (Tool::RotateLeft, "Rotate ←", Tone::Normal),
    (Tool::RotateRight, "Rotate →", Tone::Normal),
    (Tool::Mutate, "Mutate", Tone::Normal),
    (Tool::Original, "Original", Tone::Normal),
    (Tool::Shorter, "Length −", Tone::Normal),
    (Tool::Longer, "Length +", Tone::Normal),
    (Tool::Record, "Record", Tone::Bad),
    (Tool::Save, "Save", Tone::Accent),
    (Tool::Lanes, "Lanes", Tone::Normal),
];

/// The toolbar while editing lanes.
pub const LANE_TOOLS: [(Tool, &str, Tone); 9] = [
    (Tool::Back, "Back", Tone::Normal),
    (Tool::Steps, "Steps", Tone::Normal),
    (Tool::LaneShorter, "Length −", Tone::Normal),
    (Tool::LaneLonger, "Length +", Tone::Normal),
    (Tool::RandomLess, "Random −", Tone::Normal),
    (Tool::RandomMore, "Random +", Tone::Normal),
    (Tool::ClearLane, "Clear lane", Tone::Normal),
    (Tool::Original, "Original", Tone::Normal),
    (Tool::Save, "Save", Tone::Accent),
];

/// The toolbar while recording on the pads.
pub const RECORD_TOOLS: [(Tool, &str, Tone); 7] = [
    (Tool::Back, "Back", Tone::Normal),
    (Tool::StepTime, "Step time", Tone::Normal),
    (Tool::FreeTime, "Free time", Tone::Normal),
    (Tool::Undo, "Undo", Tone::Normal),
    (Tool::Clear, "Clear", Tone::Normal),
    (Tool::Record, "Stop recording", Tone::Bad),
    (Tool::Save, "Save", Tone::Accent),
];

/// Which grid the workspace shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WsView {
    #[default]
    Steps,
    Lanes,
}

/// What the app does after a touch in the workspace.
#[derive(Clone, Debug, PartialEq)]
pub enum WsAction {
    /// The pattern changed: give it to the engine, which plays it from its
    /// next step.
    Changed(Pattern),
    /// Leave the workspace for the page it was opened from.
    Back,
    /// Save the pattern to the library under the workspace's name.
    Save(Pattern),
    /// Recording on the pads turned on or off: the app feeds pad hits to
    /// `mpfree_arp::Recorder` and labels the pads with [`record_pad_labels`].
    Record(bool),
    /// Record in Step time or Free time (from now on, and at once if
    /// recording).
    RecordMode(RecordMode),
    /// While recording: take back the last step written (Step time).
    Undo,
    /// While recording: start again from an empty pattern (Step time) or
    /// rest every step (Free time).
    Clear,
}

/// The workspace's widgets and their meaning.
pub struct PatternWorkspace {
    /// The page names for editing and for recording on the pads.
    pub edit_page: String,
    pub record_page: String,
    /// The pattern's name in the library.
    pub name: String,
    pub editor: Editor,
    pub recording: bool,
    /// Recording in Free time rather than Step time (the app says which).
    pub free_time: bool,
    /// Steps or lanes.
    pub view: WsView,
    /// The engine's step count for the step playing now (`Engine::step`
    /// minus one) while the pattern plays, so each lane can mark its own
    /// position.
    pub played: Option<u64>,
}

impl PatternWorkspace {
    /// Adds the widgets and both pages to `ui` (the app adds `header`,
    /// `pads` and `hint` first), opened on `pattern`.
    pub fn add(ui: &mut Ui, edit_page: &str, record_page: &str, name: &str, pattern: Pattern)
        -> Result<PatternWorkspace, String> {
        ui.add(TITLE, Label::new("", Tone::Normal, Align::Left));
        let rows: Vec<&str> = Row::ALL.iter().map(|r| r.label()).collect();
        ui.add(GRID, LaneGrid::new(&rows, COLUMNS));
        ui.add(TOOLS, ButtonRow::one_line(&[]));
        ui.add_page(edit_page, EDIT_LAYOUT)?;
        ui.add_page(record_page, RECORD_LAYOUT)?;
        let ws = PatternWorkspace {
            edit_page: edit_page.to_string(),
            record_page: record_page.to_string(),
            name: name.to_string(),
            editor: Editor::new(pattern),
            recording: false,
            free_time: false,
            view: WsView::Steps,
            played: None,
        };
        ws.update(ui, None);
        Ok(ws)
    }

    /// The page to show now.
    pub fn page(&self) -> &str {
        if self.recording { &self.record_page } else { &self.edit_page }
    }

    /// Steps the grid shows at once.
    pub fn columns(&self) -> usize {
        if self.recording { RECORD_COLUMNS } else { COLUMNS }
    }

    /// The first step the grid shows: the page of steps that holds the
    /// cursor.
    pub fn first(&self) -> usize {
        self.editor.cursor.1 / self.columns() * self.columns()
    }

    /// The lanes are on screen (never while recording).
    pub fn showing_lanes(&self) -> bool {
        self.view == WsView::Lanes && !self.recording
    }

    /// The first lane value the lanes grid shows.
    fn lane_first(&self) -> usize {
        self.editor.lane_cursor.1 / COLUMNS * COLUMNS
    }

    /// The toolbar showing now.
    pub fn tools(&self) -> &'static [(Tool, &'static str, Tone)] {
        if self.recording {
            &RECORD_TOOLS
        } else if self.showing_lanes() {
            &LANE_TOOLS
        } else {
            &STEP_TOOLS
        }
    }

    /// The hint line for what the workspace shows.
    pub fn hint(&self) -> &'static str {
        if self.recording && self.free_time {
            "Free time: each press lands on the nearest step as the pattern plays · hold a pad for ties"
        } else if self.showing_lanes() {
            "Each lane loops at its own length · drag a value or turn the wheel · drag + to add one"
        } else {
            hint(self.recording)
        }
    }

    /// Shows the editor's pattern. `playing` is the pattern step the engine
    /// is on, if it is playing.
    pub fn update(&self, ui: &mut Ui, playing: Option<usize>) {
        self.update_tools(ui);
        if self.showing_lanes() {
            self.update_lanes(ui);
            return;
        }
        let p = self.editor.pattern();
        let (first, columns) = (self.first(), self.columns());
        let last = p.len().min(first + columns);
        if let Some(t) = ui.get_mut::<Label>(TITLE) {
            let star = if self.editor.changed() { " (changed)" } else { "" };
            let pages = p.len().div_ceil(columns);
            let page = if pages > 1 { format!(" · steps {}–{} of {}", first + 1, last, p.len()) } else { String::new() };
            let lanes = lane_names(p);
            let rec = if self.recording { "Recording on the pads · " } else { "" };
            t.text = format!("{rec}Pattern: {}{star} · {} steps{page}{lanes}", self.name, p.len());
        }
        if let Some(g) = ui.get_mut::<LaneGrid>(GRID) {
            g.rows = Row::ALL.iter().map(|r| r.label().to_string()).collect();
            g.playing_rows.clear();
            g.first = first;
            g.columns = columns;
            g.cells = Row::ALL
                .iter()
                .map(|row| p.steps()[first..last].iter().map(|s| grid_cell(*row, *s)).collect())
                .collect();
            let (row, step) = self.editor.cursor;
            g.cursor = (step >= first && step < last).then(|| (row.index(), step - first));
            g.playing = playing.filter(|s| *s >= first && *s < last).map(|s| s - first);
        }
    }

    fn update_lanes(&self, ui: &mut Ui) {
        let p = self.editor.pattern();
        let first = self.lane_first();
        let (lane, pos) = self.editor.lane_cursor;
        if let Some(t) = ui.get_mut::<Label>(TITLE) {
            let star = if self.editor.changed() { " (changed)" } else { "" };
            let (v, random) = lane_values(p, lane);
            let state = match (v.len(), random) {
                (0, 0) => "off".to_string(),
                (n, 0) => format!("{n} values"),
                (n, r) => format!("{n} values, random ±{r}"),
            };
            t.text = format!("Pattern: {}{star} · {} steps · lanes loop at their own lengths · {}: {state}",
                             self.name, p.len(), lane.label());
        }
        if let Some(g) = ui.get_mut::<LaneGrid>(GRID) {
            g.first = first;
            g.columns = COLUMNS;
            g.rows.clear();
            g.cells.clear();
            g.playing_rows.clear();
            g.playing = None;
            for row in LaneRow::ALL {
                let (v, random) = lane_values(p, row);
                g.rows.push(if random > 0 { format!("{} ±{random}", row.label()) } else { row.label().to_string() });
                let mut cells: Vec<StepCell> = v
                    .iter()
                    .skip(first)
                    .take(COLUMNS)
                    .map(|x| StepCell { kind: CellKind::Note, label: lane_label(row, *x), ..StepCell::default() })
                    .collect();
                if v.len() >= first && v.len() < first + COLUMNS && v.len() < MAX_STEPS {
                    cells.push(StepCell { kind: CellKind::Rest, label: "+".into(), ..StepCell::default() });
                }
                g.cells.push(cells);
                let at = self.played.filter(|_| !v.is_empty()).map(|n| (n % v.len() as u64) as usize);
                g.playing_rows.push(at.filter(|a| *a >= first && *a < first + COLUMNS).map(|a| a - first));
            }
            g.cursor = (pos >= first && pos < first + COLUMNS).then(|| (lane.index(), pos - first));
        }
    }

    fn update_tools(&self, ui: &mut Ui) {
        let p = self.editor.pattern();
        let (lane, _) = self.editor.lane_cursor;
        let (v, random) = lane_values(p, lane);
        let (_, _, _, most) = lane.range();
        let buttons: Vec<Button> = self
            .tools()
            .iter()
            .map(|(tool, label, tone)| {
                let disabled = match tool {
                    Tool::Original => !self.editor.changed(),
                    Tool::Shorter => p.len() <= 1,
                    Tool::Longer => p.len() >= MAX_STEPS,
                    Tool::LaneShorter => v.is_empty(),
                    Tool::LaneLonger => v.len() >= MAX_STEPS,
                    Tool::RandomLess => random == 0,
                    Tool::RandomMore => i32::from(random) >= most,
                    Tool::ClearLane => v.is_empty() && random == 0,
                    Tool::Undo => self.free_time,
                    _ => false,
                };
                let chosen = matches!((tool, self.free_time), (Tool::StepTime, false) | (Tool::FreeTime, true));
                Button { label: label.to_string(), tone: if chosen { Tone::Accent } else { *tone }, disabled }
            })
            .collect();
        if let Some(b) = ui.get_mut::<ButtonRow>(TOOLS) {
            if b.buttons != buttons {
                b.per_line = buttons.len();
                b.buttons = buttons;
            }
        }
    }

    /// What a touch on the workspace asks for; None for touches elsewhere or
    /// that change nothing. Call [`PatternWorkspace::update`] after it.
    pub fn action(&mut self, ev: &UiEvent) -> Option<WsAction> {
        let changed = |e: &Editor| Some(WsAction::Changed(*e.pattern()));
        let lane = self.editor.lane_cursor.0;
        match (ev.widget.as_str(), &ev.kind) {
            (GRID, EventKind::Select(i)) if self.showing_lanes() => {
                let row = LaneRow::ALL.get(i / COLUMNS).copied()?;
                self.editor.select_lane(row, self.lane_first() + i % COLUMNS);
                None
            }
            (GRID, EventKind::Step { index, delta }) if self.showing_lanes() => {
                let row = LaneRow::ALL.get(index / COLUMNS).copied()?;
                let pos = self.lane_first() + index % COLUMNS;
                self.editor.turn_lane(row, pos, *delta).then(|| changed(&self.editor))?
            }
            (GRID, EventKind::Select(i)) => {
                let (row, step) = (Row::ALL.get(i / self.columns()).copied()?, self.first() + i % self.columns());
                if !self.editor.select(row, step) {
                    return None;
                }
                (row == Row::Step && self.editor.tap(step)).then(|| changed(&self.editor))?
            }
            (GRID, EventKind::Step { index, delta }) => {
                let (row, step) = (Row::ALL.get(index / self.columns()).copied()?, self.first() + index % self.columns());
                // The Step row already changed on the tap; a drag there does nothing.
                (row != Row::Step && self.editor.turn(row, step, *delta)).then(|| changed(&self.editor))?
            }
            (TOOLS, EventKind::Tab(i)) => match self.tools().get(*i)?.0 {
                Tool::Back => Some(WsAction::Back),
                Tool::RotateLeft => {
                    self.editor.rotate(-1);
                    changed(&self.editor)
                }
                Tool::RotateRight => {
                    self.editor.rotate(1);
                    changed(&self.editor)
                }
                Tool::Mutate => {
                    self.editor.mutate();
                    changed(&self.editor)
                }
                Tool::Original => {
                    self.editor.revert();
                    changed(&self.editor)
                }
                Tool::Shorter => {
                    let n = self.editor.pattern().len();
                    self.editor.set_len(n.saturating_sub(1)).then(|| changed(&self.editor))?
                }
                Tool::Longer => {
                    let n = self.editor.pattern().len();
                    self.editor.set_len(n + 1).then(|| changed(&self.editor))?
                }
                Tool::Record => {
                    self.recording = !self.recording;
                    Some(WsAction::Record(self.recording))
                }
                Tool::Save => Some(WsAction::Save(*self.editor.pattern())),
                Tool::Lanes => {
                    self.view = WsView::Lanes;
                    None
                }
                Tool::Steps => {
                    self.view = WsView::Steps;
                    None
                }
                Tool::LaneShorter => {
                    let n = lane_values(self.editor.pattern(), lane).0.len();
                    self.editor.set_lane_len(lane, n.saturating_sub(1)).then(|| changed(&self.editor))?
                }
                Tool::LaneLonger => {
                    let n = lane_values(self.editor.pattern(), lane).0.len();
                    self.editor.set_lane_len(lane, n + 1).then(|| changed(&self.editor))?
                }
                Tool::RandomLess => self.editor.turn_random(lane, -1).then(|| changed(&self.editor))?,
                Tool::RandomMore => self.editor.turn_random(lane, 1).then(|| changed(&self.editor))?,
                Tool::ClearLane => self.editor.clear_lane(lane).then(|| changed(&self.editor))?,
                Tool::StepTime => Some(WsAction::RecordMode(RecordMode::Step)),
                Tool::FreeTime => Some(WsAction::RecordMode(RecordMode::FreeTime)),
                Tool::Undo => Some(WsAction::Undo),
                Tool::Clear => Some(WsAction::Clear),
            },
            _ => None,
        }
    }

    /// The data wheel: turns the selected cell's value.
    pub fn wheel(&mut self, delta: i32) -> Option<WsAction> {
        if self.showing_lanes() {
            let (lane, pos) = self.editor.lane_cursor;
            return self.editor.turn_lane(lane, pos, delta).then(|| WsAction::Changed(*self.editor.pattern()));
        }
        let (row, step) = self.editor.cursor;
        self.editor.turn(row, step, delta).then(|| WsAction::Changed(*self.editor.pattern()))
    }

    /// A pattern that was recorded on the pads: it replaces the one being
    /// edited (Original still goes back to the one the workspace opened with).
    pub fn recorded(&mut self, pattern: Pattern) {
        self.editor.pattern = pattern;
        self.editor.cursor.1 = self.editor.cursor.1.min(pattern.len() - 1);
        self.editor.clamp_lane_cursor();
    }
}

/// " · lanes: oct, vel" for the lanes a pattern has, or nothing.
fn lane_names(p: &Pattern) -> String {
    let l = &p.lanes;
    let names: Vec<&str> = [
        ("oct", l.octave.is_off()),
        ("deg", l.degree.is_off()),
        ("vel", l.velocity.is_off()),
        ("gate", l.gate.is_off()),
        ("nudge", l.nudge.is_off()),
    ]
    .iter()
    .filter(|(_, off)| !off)
    .map(|(n, _)| *n)
    .collect();
    if names.is_empty() { String::new() } else { format!(" · lanes: {}", names.join(", ")) }
}

/// The hint line for the workspace.
pub fn hint(recording: bool) -> &'static str {
    if recording {
        "Pads 1-9: held notes · 13 next · 14 random · 15 tie · 16 rest · hit hard for an accent"
    } else {
        "Tap a step: rest, note, accent · drag a cell or turn the wheel to change it · Back keeps it playing"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme, Touch};

    fn steps(p: &Pattern) -> String {
        p.to_text()
    }

    #[test]
    fn a_tap_cycles_rest_note_accent_and_keeps_the_pick() {
        let mut e = Editor::new(Pattern::parse("2+ . _").unwrap());
        assert!(e.tap(0));
        assert_eq!(steps(e.pattern()), "2+! . _");
        assert!(e.tap(0));
        assert_eq!(steps(e.pattern()), ". . _");
        assert!(e.tap(0));
        assert_eq!(steps(e.pattern()), "x . _");
        // A tie becomes a rest; past the end nothing happens.
        assert!(e.tap(2));
        assert_eq!(steps(e.pattern()), "x . .");
        assert!(!e.tap(3));
        assert!(e.changed());
        e.revert();
        assert_eq!(steps(e.pattern()), "2+ . _");
        assert!(!e.changed());
    }

    #[test]
    fn turning_moves_each_value_and_stops_at_its_ends() {
        let mut e = Editor::new(Pattern::parse("x").unwrap());
        assert!(e.turn(Row::Note, 0, 2));
        assert_eq!(steps(e.pattern()), "2");
        assert!(e.turn(Row::Note, 0, 99));
        assert_eq!(steps(e.pattern()), "h");
        assert!(!e.turn(Row::Note, 0, 1));
        assert!(e.turn(Row::Octave, 0, 5));
        assert!(e.turn(Row::Ratchet, 0, 2));
        assert!(e.turn(Row::Chance, 0, -3));
        assert!(e.turn(Row::Gate, 0, 4));
        let Step::Play { octave, ratchet, chance, gate, .. } = e.pattern().steps()[0] else { panic!() };
        assert_eq!((octave, ratchet, chance, gate), (3, 3, 70, 40));
        assert!(e.turn(Row::Gate, 0, -9));
        let Step::Play { gate, .. } = e.pattern().steps()[0] else { panic!() };
        assert_eq!(gate, 0, "0 is the arp's own gate");
        // The Step row: note, tie, rest.
        assert!(e.turn(Row::Step, 0, -1));
        assert_eq!(e.pattern().steps()[0], Step::Tie);
        assert!(!e.turn(Row::Octave, 0, 1), "a tie has no octave");
        assert!(e.turn(Row::Step, 0, -1));
        assert_eq!(e.pattern().steps()[0], Step::Rest);
        assert!(e.turn(Row::Step, 0, 2));
        assert_eq!(e.pattern().steps()[0], Step::play(Pick::Next));
    }

    #[test]
    fn length_rotate_and_mutate_keep_the_original() {
        let start = Pattern::parse("1 2 3 | vel 100 60").unwrap();
        let mut e = Editor::new(start);
        assert!(e.set_len(5));
        assert_eq!(steps(e.pattern()), "1 2 3 . . | vel 100 60");
        assert!(e.set_len(2));
        assert_eq!(steps(e.pattern()), "1 2 | vel 100 60");
        assert!(!e.set_len(0) && !e.set_len(MAX_STEPS + 1));
        e.rotate(1);
        assert_eq!(steps(e.pattern()), "2 1 | vel 100 60");
        e.mutate();
        let once = *e.pattern();
        assert!(once.steps().iter().any(|s| matches!(s, Step::Play { .. })));
        e.revert();
        assert_eq!(*e.pattern(), start);
        // The same presses give the same variations.
        let mut again = Editor::new(start);
        again.set_len(2);
        again.rotate(1);
        again.mutate();
        assert_eq!(*again.pattern(), once);
    }

    #[test]
    fn grid_cells_show_each_row() {
        let s = Pattern::parse("2+!:3%40'").unwrap().steps()[0];
        let row = |r| grid_cell(r, s).label;
        assert_eq!(
            [row(Row::Step), row(Row::Note), row(Row::Octave), row(Row::Ratchet), row(Row::Chance), row(Row::Gate)],
            ["●", "2", "+1", "×3", "40%", "25%"].map(String::from)
        );
        assert!(grid_cell(Row::Step, s).accent);
        assert_eq!(grid_cell(Row::Step, Step::Tie).label, "—");
        assert_eq!(grid_cell(Row::Octave, Step::Rest).label, "");
        assert_eq!(record_pad_labels()[12..], ["Next", "Random", "Tie", "Rest"]);
    }

    fn workspace(pattern: &str) -> (Ui, PatternWorkspace) {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", mpfree_ui::widgets::Header::new(&["Play", "Debug"]));
        ui.add("pads", mpfree_ui::widgets::PadGrid::new(16, 4));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let ws = PatternWorkspace::add(&mut ui, "Pattern", "Pattern, recording", "Test", Pattern::parse(pattern).unwrap())
            .unwrap();
        ui.show(ws.page()).unwrap();
        (ui, ws)
    }

    fn tap(ui: &mut Ui, r: mpfree_ui::Rect) -> Option<UiEvent> {
        let ev = ui.touch(1, Touch::Down, r.x + r.w / 2, r.y + r.h / 2);
        ui.touch(1, Touch::Up, r.x + r.w / 2, r.y + r.h / 2);
        ev
    }

    #[test]
    fn taps_and_drags_on_the_grid_edit_the_pattern() {
        let (mut ui, mut ws) = workspace("x . x");
        let r = ui.rect_of(GRID).unwrap();
        // Tap step 2 (a rest) on the Step row: it becomes a note.
        let cell = ui.get::<LaneGrid>(GRID).unwrap().cell(r, 0, 1);
        let ev = tap(&mut ui, cell).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Changed(Pattern::parse("x x x").unwrap())));
        ws.update(&mut ui, Some(1));
        // Drag step 3's octave up two steps.
        let cell = ui.get::<LaneGrid>(GRID).unwrap().cell(r, Row::Octave.index(), 2);
        let (x, y) = (cell.x + 5, cell.y + 5);
        let down = ui.touch(2, Touch::Down, x, y).unwrap();
        assert_eq!(ws.action(&down), None, "selecting a value cell changes nothing");
        let mut last = None;
        for dy in [10, 30, 60] {
            if let Some(ev) = ui.touch(2, Touch::Move, x, y - dy) {
                last = ws.action(&ev);
            }
        }
        ui.touch(2, Touch::Up, x, y - 60);
        assert_eq!(last, Some(WsAction::Changed(Pattern::parse("x x x++").unwrap())));
        assert_eq!(ws.editor.cursor, (Row::Octave, 2));
        // The wheel turns the selected cell.
        assert_eq!(ws.wheel(-1), Some(WsAction::Changed(Pattern::parse("x x x+").unwrap())));
        // Cells past the end are not there to tap.
        let past = ui.get::<LaneGrid>(GRID).unwrap().cell(r, 0, 5);
        assert_eq!(tap(&mut ui, past), None);
    }

    #[test]
    fn the_toolbar_rotates_mutates_restores_records_and_saves() {
        let (mut ui, mut ws) = workspace("1 2 3 .");
        let r = ui.rect_of(TOOLS).unwrap();
        let button = |ui: &Ui, i: usize| ui.get::<ButtonRow>(TOOLS).unwrap().button_rect(r, i);
        // Original is greyed out until something changes.
        let orig = button(&ui, 4);
        assert_eq!(tap(&mut ui, orig), None);
        let b = button(&ui, 2);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Changed(Pattern::parse(". 1 2 3").unwrap())));
        ws.update(&mut ui, None);
        let ev = tap(&mut ui, orig).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Changed(Pattern::parse("1 2 3 .").unwrap())));
        let b = button(&ui, 6);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Changed(Pattern::parse("1 2 3 . .").unwrap())));
        let b = button(&ui, 7);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Record(true)));
        ws.update(&mut ui, None);
        ui.show(ws.page()).unwrap();
        assert_eq!(ws.page(), "Pattern, recording");
        assert!(ui.rect_of("pads").is_some());
        let labels: Vec<String> = ui.get::<ButtonRow>(TOOLS).unwrap().buttons.iter().map(|b| b.label.clone()).collect();
        assert_eq!(labels, RECORD_TOOLS.map(|t| t.1.to_string()));
        // The pads take half the screen, so the grid shows 8 steps at a time.
        assert_eq!((ws.columns(), ui.get::<LaneGrid>(GRID).unwrap().columns), (RECORD_COLUMNS, RECORD_COLUMNS));
        let r = ui.rect_of(TOOLS).unwrap();
        // Free time, Undo and Clear go to the app, which owns the recorder.
        let button = |ui: &Ui, i: usize| ui.get::<ButtonRow>(TOOLS).unwrap().button_rect(r, i);
        let b = button(&ui, 2);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::RecordMode(RecordMode::FreeTime)));
        ws.free_time = true;
        ws.update(&mut ui, None);
        let b = ui.get::<ButtonRow>(TOOLS).unwrap();
        assert!(b.buttons[3].disabled, "Undo is for Step time");
        assert_eq!((b.buttons[1].tone, b.buttons[2].tone), (Tone::Normal, Tone::Accent));
        assert!(ws.hint().starts_with("Free time"));
        let b = button(&ui, 4);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Clear));
        let save = button(&ui, 6);
        let ev = tap(&mut ui, save).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Save(Pattern::parse("1 2 3 . .").unwrap())));
        let back = ui.get::<ButtonRow>(TOOLS).unwrap().button_rect(r, 0);
        let ev = tap(&mut ui, back).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Back));
    }

    #[test]
    fn lanes_take_values_lengths_and_random_amounts() {
        let mut e = Editor::new(Pattern::parse("x x x x").unwrap());
        // A drag on the + cell of an empty lane starts it.
        assert!(e.turn_lane(LaneRow::Velocity, 0, 13));
        assert_eq!(steps(e.pattern()), "x x x x | vel 104");
        assert!(!e.turn_lane(LaneRow::Velocity, 2, 1), "only the cell just past the end adds one");
        assert!(e.turn_lane(LaneRow::Velocity, 1, -3));
        assert!(e.set_lane_len(LaneRow::Velocity, 3));
        assert_eq!(steps(e.pattern()), "x x x x | vel 104 80 80");
        // Values stop at the lane's ends.
        assert!(e.turn_lane(LaneRow::Velocity, 0, 50));
        assert!(!e.turn_lane(LaneRow::Velocity, 0, 1));
        assert!(e.turn_lane(LaneRow::Octave, 0, -9));
        assert!(e.turn_lane(LaneRow::Nudge, 0, 1));
        assert!(e.turn_lane(LaneRow::Gate, 0, 4));
        assert_eq!(steps(e.pattern()), "x x x x | oct -3 | vel 127 80 80 | gate 40 | nudge 5");
        // Random amounts move by the lane's step and stop at its most.
        assert!(e.turn_random(LaneRow::Velocity, 2));
        assert!(e.turn_random(LaneRow::Octave, 9));
        assert!(!e.turn_random(LaneRow::Octave, 1));
        assert!(!e.turn_random(LaneRow::Degree, -1));
        assert_eq!(lane_values(e.pattern(), LaneRow::Velocity), (vec![127, 80, 80], 16));
        // A random amount alone is a lane too; Clear turns it off.
        assert!(e.turn_random(LaneRow::Degree, 2));
        assert_eq!(steps(e.pattern()), "x x x x | oct~3 -3 | deg~2 | vel~16 127 80 80 | gate 40 | nudge 5");
        assert!(e.clear_lane(LaneRow::Degree) && e.clear_lane(LaneRow::Octave));
        assert!(!e.clear_lane(LaneRow::Octave));
        assert!(e.set_lane_len(LaneRow::Nudge, 0));
        assert_eq!(steps(e.pattern()), "x x x x | vel~16 127 80 80 | gate 40");
        // Every change reads back the same through the text form.
        assert_eq!(Pattern::parse(&e.pattern().to_text()).unwrap(), *e.pattern());
        e.revert();
        assert_eq!(steps(e.pattern()), "x x x x");
        assert_eq!(lane_label(LaneRow::Velocity, 0), "auto");
        assert_eq!(lane_label(LaneRow::Nudge, -10), "-10%");
        assert_eq!(lane_label(LaneRow::Degree, 3), "+3");
    }

    #[test]
    fn the_lanes_view_edits_by_touch_and_marks_each_lane_playing() {
        let (mut ui, mut ws) = workspace("x x x x | vel 100 60 30");
        let r = ui.rect_of(TOOLS).unwrap();
        let lanes = ui.get::<ButtonRow>(TOOLS).unwrap().button_rect(r, 9);
        let ev = tap(&mut ui, lanes).unwrap();
        assert_eq!(ws.action(&ev), None);
        assert!(ws.showing_lanes());
        ws.played = Some(4);
        ws.update(&mut ui, Some(0));
        let g = ui.get::<LaneGrid>(GRID).unwrap();
        assert_eq!(g.rows, ["Octave", "Degree", "Velocity", "Gate", "Nudge"]);
        let labels: Vec<&str> = g.cells[2].iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["100", "60", "30", "+"]);
        assert_eq!(g.cells[0].len(), 1, "an empty lane shows only its + cell");
        // Step 5 played: the 3-value velocity lane is on its 2nd value.
        assert_eq!((g.playing_rows[2], g.playing_rows[0]), (Some(1), None));
        assert!(ui.get::<Label>(TITLE).unwrap().text.ends_with("Velocity: 3 values"));
        // Drag the + cell up: a 4th value, starting from the 3rd.
        let r = ui.rect_of(GRID).unwrap();
        let plus = ui.get::<LaneGrid>(GRID).unwrap().cell(r, 2, 3);
        let (x, y) = (plus.x + 5, plus.y + 5);
        let down = ui.touch(3, Touch::Down, x, y).unwrap();
        assert_eq!(ws.action(&down), None);
        let ev = ui.touch(3, Touch::Move, x, y - 30).unwrap();
        ui.touch(3, Touch::Up, x, y - 30);
        assert_eq!(ws.action(&ev), Some(WsAction::Changed(Pattern::parse("x x x x | vel 100 60 30 38").unwrap())));
        // The wheel turns the selected value; Random + and Length − work on its lane.
        assert_eq!(ws.wheel(1), Some(WsAction::Changed(Pattern::parse("x x x x | vel 100 60 30 46").unwrap())));
        ws.update(&mut ui, None);
        let tools = ui.rect_of(TOOLS).unwrap();
        let button = |ui: &Ui, i: usize| ui.get::<ButtonRow>(TOOLS).unwrap().button_rect(tools, i);
        let b = button(&ui, 5);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Changed(Pattern::parse("x x x x | vel~8 100 60 30 46").unwrap())));
        ws.update(&mut ui, None);
        let b = button(&ui, 2);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Changed(Pattern::parse("x x x x | vel~8 100 60 30").unwrap())));
        assert_eq!(ws.editor.lane_cursor, (LaneRow::Velocity, 3), "the cursor stays on the lane's end");
        ws.update(&mut ui, None);
        assert!(ws.hint().starts_with("Each lane loops"));
        // Steps goes back; recording always shows the steps.
        let b = button(&ui, 1);
        let ev = tap(&mut ui, b).unwrap();
        assert_eq!(ws.action(&ev), None);
        assert!(!ws.showing_lanes());
        ws.view = WsView::Lanes;
        ws.recording = true;
        assert!(!ws.showing_lanes());
        ws.update(&mut ui, None);
        assert_eq!(ui.get::<LaneGrid>(GRID).unwrap().rows.len(), Row::ALL.len());
    }

    #[test]
    fn long_patterns_page_with_the_cursor_and_both_pages_draw() {
        let text = vec!["x"; 20].join(" ");
        let (mut ui, mut ws) = workspace(&text);
        assert!(ws.editor.select(Row::Note, 17));
        ws.update(&mut ui, Some(18));
        let g = ui.get::<LaneGrid>(GRID).unwrap();
        assert_eq!((g.first, g.cells[0].len(), g.cursor, g.playing), (16, 4, Some((1, 1)), Some(2)));
        let title = &ui.get::<Label>(TITLE).unwrap().text;
        assert!(title.contains("steps 17–20 of 20"), "{title}");
        for page in ["Pattern", "Pattern, recording"] {
            ui.show(page).unwrap();
            let mut canvas = SoftCanvas::new(1280, 800);
            ui.paint(&mut canvas);
            for name in [TITLE, GRID, TOOLS] {
                let r = ui.rect_of(name).unwrap();
                assert!(r.w > 0 && r.h > 0 && r.right() <= 1280 && r.bottom() <= 800, "{page}: {name} {r:?}");
            }
        }
    }
}
