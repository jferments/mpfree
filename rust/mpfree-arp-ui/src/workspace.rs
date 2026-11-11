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
//! opened with. The pattern's lanes (their own lengths, for polymeter) are
//! kept as they are; editing them here comes later.
//!
//! [`Editor`] is the model and has no drawing, so it is tested on its own.
//! [`PatternWorkspace`] adds the widgets and two layouts to the app's `Ui`:
//! one for editing, and one with the pads for recording on them.

use crate::view::pattern_cell;
use crate::widgets::{ButtonRow, CellKind, LaneGrid, StepCell};
use mpfree_arp::{Pattern, Pick, Step, MAX_STEPS};
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
    mutations: u32,
}

impl Editor {
    pub fn new(pattern: Pattern) -> Editor {
        Editor { original: pattern, pattern, cursor: (Row::Step, 0), mutations: 0 }
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

/// The toolbar, in [`Tool`] order.
pub const TOOL_BUTTONS: [(&str, Tone); 9] = [
    ("Back", Tone::Normal),
    ("Rotate ←", Tone::Normal),
    ("Rotate →", Tone::Normal),
    ("Mutate", Tone::Normal),
    ("Original", Tone::Normal),
    ("Length −", Tone::Normal),
    ("Length +", Tone::Normal),
    ("Record", Tone::Bad),
    ("Save", Tone::Accent),
];

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
}

const TOOLS_ALL: [Tool; 9] = [
    Tool::Back,
    Tool::RotateLeft,
    Tool::RotateRight,
    Tool::Mutate,
    Tool::Original,
    Tool::Shorter,
    Tool::Longer,
    Tool::Record,
    Tool::Save,
];

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
}

impl PatternWorkspace {
    /// Adds the widgets and both pages to `ui` (the app adds `header`,
    /// `pads` and `hint` first), opened on `pattern`.
    pub fn add(ui: &mut Ui, edit_page: &str, record_page: &str, name: &str, pattern: Pattern)
        -> Result<PatternWorkspace, String> {
        ui.add(TITLE, Label::new("", Tone::Normal, Align::Left));
        let rows: Vec<&str> = Row::ALL.iter().map(|r| r.label()).collect();
        ui.add(GRID, LaneGrid::new(&rows, COLUMNS));
        ui.add(TOOLS, ButtonRow::one_line(&TOOL_BUTTONS));
        ui.add_page(edit_page, EDIT_LAYOUT)?;
        ui.add_page(record_page, RECORD_LAYOUT)?;
        let ws = PatternWorkspace {
            edit_page: edit_page.to_string(),
            record_page: record_page.to_string(),
            name: name.to_string(),
            editor: Editor::new(pattern),
            recording: false,
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

    /// Shows the editor's pattern. `playing` is the pattern step the engine
    /// is on, if it is playing.
    pub fn update(&self, ui: &mut Ui, playing: Option<usize>) {
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
        if let Some(b) = ui.get_mut::<ButtonRow>(TOOLS) {
            b.buttons[4].disabled = !self.editor.changed();
            b.buttons[5].disabled = p.len() <= 1;
            b.buttons[6].disabled = p.len() >= MAX_STEPS;
            b.buttons[7].label = if self.recording { "Stop recording".into() } else { "Record".into() };
        }
    }

    /// What a touch on the workspace asks for; None for touches elsewhere or
    /// that change nothing. Call [`PatternWorkspace::update`] after it.
    pub fn action(&mut self, ev: &UiEvent) -> Option<WsAction> {
        let changed = |e: &Editor| Some(WsAction::Changed(*e.pattern()));
        match (ev.widget.as_str(), &ev.kind) {
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
            (TOOLS, EventKind::Tab(i)) => match TOOLS_ALL.get(*i)? {
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
            },
            _ => None,
        }
    }

    /// The data wheel: turns the selected cell's value.
    pub fn wheel(&mut self, delta: i32) -> Option<WsAction> {
        let (row, step) = self.editor.cursor;
        self.editor.turn(row, step, delta).then(|| WsAction::Changed(*self.editor.pattern()))
    }

    /// A pattern that was recorded on the pads: it replaces the one being
    /// edited (Original still goes back to the one the workspace opened with).
    pub fn recorded(&mut self, pattern: Pattern) {
        self.editor.pattern = pattern;
        self.editor.cursor.1 = self.editor.cursor.1.min(pattern.len() - 1);
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
        assert_eq!(ui.get::<ButtonRow>(TOOLS).unwrap().buttons[7].label, "Stop recording");
        // The pads take half the screen, so the grid shows 8 steps at a time.
        assert_eq!((ws.columns(), ui.get::<LaneGrid>(GRID).unwrap().columns), (RECORD_COLUMNS, RECORD_COLUMNS));
        let r = ui.rect_of(TOOLS).unwrap();
        let save = ui.get::<ButtonRow>(TOOLS).unwrap().button_rect(r, 8);
        let ev = tap(&mut ui, save).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Save(Pattern::parse("1 2 3 . .").unwrap())));
        let back = ui.get::<ButtonRow>(TOOLS).unwrap().button_rect(r, 0);
        let ev = tap(&mut ui, back).unwrap();
        assert_eq!(ws.action(&ev), Some(WsAction::Back));
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
