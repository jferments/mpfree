//! Continue in the note editor (composing tools, phase 1 step 2): a tool
//! of the Edit options ([`ToolKind::Continue`](super::ToolKind)) that
//! carries the clip on by some bars. It opens on the whole clip, no picks
//! needed, and asks at once; the time engine makes the new bars off the
//! music's threads and plays them in place of the clip from the next bar
//! line (`TimeState::continuing`), and the roll shows them faintly after
//! yours. The Q-Links turn Similarity, Variation, Activity and Range
//! ([`super::NoteTool`]'s `q`); while it is open the drawer's rows are
//! [`CONTINUE_ROWS`] ([`ContinueSet`]), and its takes area holds the
//! tries (‹ ›). Every change asks again, the last result playing on
//! until the next is ready; Try again asks with the same settings; Done
//! keeps the result as one take, Cancel goes back to the clip as it was.
//!
//! Step 3 adds [`Picks`]: new bars picked on the ruler or new notes picked
//! on the roll make Try again work on them alone, and new bars locked stay
//! through every try.
//!
//! The design is docs/dev/design/compose.md.

use mpfree_compose::{Ending, Keeps, RangeSetting, Settings, STYLES};
use mpfree_transport::time::Tick;
use mpfree_ui::widgets::OptionRow;

use crate::continuing::Scope;

/// The drawer's rows while Continue is open, top to bottom: the style,
/// the bars it adds, how the last bar closes, then what stays exactly as
/// yours (the design's Keeps: Shape is the tune's shape, each step going
/// the way yours did), each "yours" or "free".
pub const CONTINUE_ROWS: [&str; 7] = [
    "Style", "Bars", "Ending", "Rhythm", "Shape", "Chords", "Accents",
];
/// The first of the Keeps in [`CONTINUE_ROWS`].
pub const KEEP_ROW: usize = 3;
/// The bars Continue adds, as the Bars row steps them.
pub const BARS: [u8; 5] = [1, 2, 4, 8, 16];
/// The endings, as the Ending row steps them.
pub const ENDINGS: [Ending; 3] = [Ending::Home, Ending::Open, Ending::LoopsBack];
/// The Range card's settings, as Q4 steps them.
pub const RANGES: [RangeSetting; 3] = [
    RangeSetting::Narrow,
    RangeSetting::Medium,
    RangeSetting::Wide,
];

/// Continue's settings that the drawer's rows hold (the Q-Links' are the
/// tool's own): kept by the editor from one Continue to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContinueSet {
    /// An index into [`STYLES`].
    pub style: usize,
    /// One of [`BARS`].
    pub bars: u8,
    pub ending: Ending,
    pub keeps: Keeps,
    /// The Q-Links' settings ([`opening_q`]'s order), as the tool last
    /// left them.
    pub q: [i32; 4],
    /// The row picked (the wheel steps it).
    pub row: usize,
}

impl Default for ContinueSet {
    fn default() -> Self {
        let s = Settings::default();
        ContinueSet {
            style: STYLES
                .iter()
                .position(|k| k.name == s.style.name)
                .unwrap_or(0),
            bars: s.bars,
            ending: s.ending,
            keeps: s.keeps,
            q: opening_q(),
            row: 0,
        }
    }
}

/// The Q-Links' settings as Continue opens the first time: Similarity,
/// Variation and Activity in %, and Range (0 Narrow to 2 Wide), from
/// [`Settings::default`].
pub fn opening_q() -> [i32; 4] {
    let s = Settings::default();
    let pct = |v: f32| (v * 100.0).round() as i32;
    [
        pct(s.similarity),
        pct(s.variation),
        pct(s.activity),
        RANGES.iter().position(|&r| r == s.range).unwrap_or(1) as i32,
    ]
}

/// What a touch on Continue asks of the time engine (the editor's
/// [`super::Done::compose`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Compose {
    /// A result with these settings: Continue opening, a setting changed,
    /// or Try again (on everything, or on what is picked). The time engine
    /// picks the seed.
    Ask(Box<Asking>),
    /// Done: the result shown, as one take.
    Keep,
    /// Cancel: the clip as it was.
    Back,
}

/// An ask from the note editor's Continue.
#[derive(Clone, Debug, PartialEq)]
pub struct Asking {
    pub settings: Settings,
    pub scope: Scope,
    /// The new bars locked (from 0).
    pub locked: Vec<u16>,
}

/// What Try again works on while Continue is open, and the new bars
/// locked: the editor's own, from when Continue opens until it closes
/// (phase 1 step 3). Bars or notes are picked, never both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picks {
    /// New bars picked on the ruler (from 0, the clip's first bar), in
    /// the order picked.
    pub bars: Vec<u16>,
    /// New notes picked on the roll: where each starts, in the clip's
    /// ticks.
    pub notes: Vec<Tick>,
    /// New bars locked: they stay exactly as they are through every try.
    pub locked: Vec<u16>,
}

impl Picks {
    /// A tap on new bar `b` on the ruler: picked, or let go. The line.
    pub fn bar(&mut self, b: u16) -> String {
        self.notes.clear();
        if let Some(i) = self.bars.iter().position(|&x| x == b) {
            self.bars.remove(i);
            return format!("continue: bar {} let go", b + 1);
        }
        self.bars.push(b);
        if self.locked.contains(&b) {
            format!(
                "continue: bar {} picked: it's locked, Unlock lets it change",
                b + 1
            )
        } else {
            format!("continue: bar {} picked: Try again remakes it alone", b + 1)
        }
    }

    /// A tap on a new note starting at `t`: picked, or let go. The line.
    pub fn note(&mut self, t: Tick) -> String {
        self.bars.clear();
        match self.notes.iter().position(|&x| x == t) {
            Some(i) => {
                self.notes.remove(i);
            }
            None => self.notes.push(t),
        }
        match self.notes.len() {
            0 => "continue: no notes picked".to_string(),
            1 => "continue: 1 note picked: Try again gives it another".to_string(),
            n => format!("continue: {n} notes picked: Try again gives them others"),
        }
    }

    /// Lock, on the drawer: the picked bars locked, or unlocked when they
    /// all are; the picks let go. The line.
    pub fn lock(&mut self) -> String {
        if self.bars.is_empty() {
            return "continue: pick new bars on the ruler to lock them".to_string();
        }
        let unlock = self.bars.iter().all(|b| self.locked.contains(b));
        let which = bars_named(&self.bars);
        for b in std::mem::take(&mut self.bars) {
            match unlock {
                true => self.locked.retain(|&x| x != b),
                false if !self.locked.contains(&b) => self.locked.push(b),
                false => {}
            }
        }
        self.locked.sort_unstable();
        match unlock {
            true => format!("continue: {which} unlocked"),
            false => format!("continue: {which} locked: it stays through every try"),
        }
    }

    /// What Try again works on: the picked bars that aren't locked, the
    /// picked notes, or everything.
    pub fn scope(&self) -> Scope {
        let mut bars: Vec<u16> = self
            .bars
            .iter()
            .copied()
            .filter(|b| !self.locked.contains(b))
            .collect();
        bars.sort_unstable();
        if !bars.is_empty() {
            Scope::Bars(bars)
        } else if !self.notes.is_empty() {
            let mut notes = self.notes.clone();
            notes.sort_unstable();
            Scope::Notes(notes)
        } else {
            Scope::All
        }
    }

    /// Only locked bars are picked: Try again has nothing it may change.
    pub fn only_locked(&self) -> bool {
        !self.bars.is_empty() && self.bars.iter().all(|b| self.locked.contains(b))
    }

    /// The tool bar's Try again, saying what it works on.
    pub fn try_label(&self) -> String {
        match self.scope() {
            Scope::All => "Try again".to_string(),
            Scope::Bars(b) if b.len() == 1 => format!("Try bar {} again", b[0] + 1),
            Scope::Bars(b) => format!("Try {} bars again", b.len()),
            Scope::Notes(n) if n.len() == 1 => "Try 1 note again".to_string(),
            Scope::Notes(n) => format!("Try {} notes again", n.len()),
        }
    }

    /// The drawer's lock button: "Lock bar 6", "Unlock bar 4", "Lock 2
    /// bars", or "Lock picked bars" with none picked.
    pub fn lock_label(&self) -> String {
        if self.bars.is_empty() {
            return "Lock picked bars".to_string();
        }
        let unlock = self.bars.iter().all(|b| self.locked.contains(b));
        let which = bars_named(&self.bars);
        match unlock {
            true => format!("Unlock {which}"),
            false => format!("Lock {which}"),
        }
    }
}

/// "bar 6", "2 bars".
fn bars_named(bars: &[u16]) -> String {
    match bars {
        [b] => format!("bar {}", b + 1),
        v => format!("{} bars", v.len()),
    }
}

impl ContinueSet {
    /// The settings an ask carries, with the Q-Links' `q`.
    pub fn settings(&self, q: [i32; 4]) -> Settings {
        let pct = |v: i32| v.clamp(0, 100) as f32 / 100.0;
        Settings {
            style: STYLES[self.style.min(STYLES.len() - 1)],
            similarity: pct(q[0]),
            variation: pct(q[1]),
            activity: pct(q[2]),
            range: RANGES[q[3].clamp(0, 2) as usize],
            keeps: self.keeps,
            bars: self.bars,
            ending: self.ending,
            ..Settings::default()
        }
    }

    /// Keep `k` of [`CONTINUE_ROWS`]' four, on or off.
    fn keep(&mut self, k: usize) -> Option<&mut bool> {
        let keeps = &mut self.keeps;
        Some(match k {
            0 => &mut keeps.rhythm,
            1 => &mut keeps.tune_shape,
            2 => &mut keeps.chords,
            3 => &mut keeps.accents,
            _ => return None,
        })
    }

    /// Row `row` stepped `delta`: true when a setting changed. Style,
    /// Bars and Ending stop at their ends; a Keep is "yours" with a step
    /// up and "free" with a step down.
    pub fn step(&mut self, row: usize, delta: i32) -> bool {
        let by = |i: usize, n: usize| (i as i64 + i64::from(delta)).clamp(0, n as i64 - 1) as usize;
        match row {
            0 => {
                let was = self.style;
                self.style = by(self.style, STYLES.len());
                self.style != was
            }
            1 => {
                let at = BARS.iter().position(|&b| b == self.bars).unwrap_or(2);
                let was = self.bars;
                self.bars = BARS[by(at, BARS.len())];
                self.bars != was
            }
            2 => {
                let at = ENDINGS.iter().position(|&e| e == self.ending).unwrap_or(0);
                let was = self.ending;
                self.ending = ENDINGS[by(at, ENDINGS.len())];
                self.ending != was
            }
            r if delta != 0 => match self.keep(r.wrapping_sub(KEEP_ROW)) {
                Some(k) => {
                    let was = *k;
                    *k = delta > 0;
                    *k != was
                }
                None => false,
            },
            _ => false,
        }
    }

    /// Row `row`'s value as the drawer shows it (short, to fit the 250
    /// px drawer).
    pub fn value(&self, row: usize) -> String {
        let on = |b: bool| if b { "yours" } else { "free" }.to_string();
        let k = self.keeps;
        match row {
            0 => STYLES[self.style.min(STYLES.len() - 1)].name.to_string(),
            1 => format!(
                "+{} bar{}",
                self.bars,
                if self.bars == 1 { "" } else { "s" }
            ),
            2 => match self.ending {
                Ending::Home => "Home",
                Ending::Open => "Open",
                Ending::LoopsBack => "Loop",
            }
            .to_string(),
            3 => on(k.rhythm),
            4 => on(k.tune_shape),
            5 => on(k.chords),
            6 => on(k.accents),
            _ => String::new(),
        }
    }

    /// The drawer's rows while Continue is open.
    pub fn rows(&self) -> Vec<OptionRow> {
        CONTINUE_ROWS
            .iter()
            .enumerate()
            .map(|(r, name)| OptionRow {
                name: name.to_string(),
                value: self.value(r),
            })
            .collect()
    }

    /// Row `row` as the log says it once stepped: "style Groove", "2
    /// more bars", "leaves it open", "rhythm kept as yours".
    pub fn said(&self, row: usize) -> String {
        match row {
            0 => format!("style {}", self.value(0)),
            1 => bars_words(self.bars),
            2 => self.ending.words().to_lowercase(),
            r => {
                let name = CONTINUE_ROWS.get(r).copied().unwrap_or("").to_lowercase();
                match self.value(r).as_str() {
                    "yours" => format!("{name} kept as yours"),
                    _ => format!("{name} free"),
                }
            }
        }
    }
}

/// "1 more bar", "4 more bars".
pub fn bars_words(bars: u8) -> String {
    match bars {
        1 => "1 more bar".to_string(),
        n => format!("{n} more bars"),
    }
}

/// The Q-Links' cards while Continue is open: their names and values.
pub fn cards(q: [i32; 4]) -> [(&'static str, String); 4] {
    let activity = match q[2] {
        50 => "As yours".to_string(),
        v if v < 50 => format!("Sparser {v}%"),
        v => format!("Busier {v}%"),
    };
    [
        ("Similarity", format!("{}%", q[0])),
        ("Variation", format!("{}%", q[1])),
        ("Activity", activity),
        (
            "Range",
            RANGES[q[3].clamp(0, 2) as usize].name().to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::continuing::Scope;

    #[test]
    fn the_settings_open_as_the_composers_own_and_go_back_whole() {
        let set = ContinueSet::default();
        let s = set.settings(opening_q());
        let d = Settings::default();
        assert_eq!(s.style.name, d.style.name);
        assert_eq!(
            (s.bars, s.ending, s.keeps, s.range),
            (d.bars, d.ending, d.keeps, d.range)
        );
        assert!((s.similarity - d.similarity).abs() < 1e-6);
        assert!((s.variation - d.variation).abs() < 1e-6);
        assert!((s.activity - d.activity).abs() < 1e-6);
    }

    #[test]
    fn rows_step_and_stop_at_their_ends() {
        let mut set = ContinueSet::default();
        assert_eq!(set.value(0), "Minimal");
        assert!(set.step(0, 1));
        assert_eq!(set.value(0), STYLES[1].name);
        assert!(set.step(0, 9));
        assert!(!set.step(0, 1), "the last style stays");
        assert_eq!(set.value(1), "+4 bars");
        assert!(set.step(1, -1));
        assert_eq!(set.bars, 2);
        assert_eq!(set.said(1), "2 more bars");
        assert!(set.step(1, -5));
        assert_eq!(set.value(1), "+1 bar");
        assert!(!set.step(1, -1));
        assert!(set.step(2, 1));
        assert_eq!(set.value(2), "Open");
        assert_eq!(set.said(2), "leaves it open");
        assert_eq!(
            set.said(0),
            format!("style {}", STYLES.last().unwrap().name)
        );
        // A Keep is yours with a step up and free with a step down.
        assert_eq!(set.value(KEEP_ROW), "free");
        assert!(set.step(KEEP_ROW, 1));
        assert!(!set.step(KEEP_ROW, 1));
        assert!(set.settings(opening_q()).keeps.rhythm);
        assert_eq!(set.said(KEEP_ROW), "rhythm kept as yours");
        assert!(set.step(KEEP_ROW, -1));
        assert!(!set.step(9, 1));
        assert_eq!(set.rows().len(), CONTINUE_ROWS.len());
    }

    /// Bars and notes are picked one at a time, never both; Lock locks
    /// the picked bars (or unlocks them when they all are) and lets them
    /// go; Try again works on the picked bars that aren't locked, else
    /// the picked notes, else everything, and its label says which.
    #[test]
    fn picks_say_what_try_again_works_on() {
        let mut p = Picks::default();
        assert_eq!((p.scope(), p.try_label()), (Scope::All, "Try again".into()));
        assert_eq!(p.lock_label(), "Lock picked bars");
        assert_eq!(
            p.lock(),
            "continue: pick new bars on the ruler to lock them"
        );
        assert_eq!(
            p.bar(5),
            "continue: bar 6 picked: Try again remakes it alone"
        );
        assert_eq!(p.try_label(), "Try bar 6 again");
        assert_eq!(p.lock_label(), "Lock bar 6");
        p.bar(4);
        assert_eq!(p.scope(), Scope::Bars(vec![4, 5]));
        assert_eq!(p.try_label(), "Try 2 bars again");
        assert_eq!(p.bar(4), "continue: bar 5 let go");
        // A note picked lets the bars go.
        assert_eq!(
            p.note(7680),
            "continue: 1 note picked: Try again gives it another"
        );
        assert!(p.bars.is_empty());
        assert_eq!(
            p.note(7200),
            "continue: 2 notes picked: Try again gives them others"
        );
        assert_eq!(p.scope(), Scope::Notes(vec![7200, 7680]));
        assert_eq!(p.try_label(), "Try 2 notes again");
        p.note(7200);
        assert_eq!(p.try_label(), "Try 1 note again");
        assert_eq!(p.note(7680), "continue: no notes picked");
        // Locked: the picks go, and Try again leaves the bar alone.
        p.bar(5);
        assert_eq!(
            p.lock(),
            "continue: bar 6 locked: it stays through every try"
        );
        assert_eq!((p.bars.clone(), p.locked.clone()), (vec![], vec![5]));
        assert_eq!(p.scope(), Scope::All);
        assert_eq!(
            p.bar(5),
            "continue: bar 6 picked: it's locked, Unlock lets it change"
        );
        assert!(p.only_locked());
        assert_eq!(p.lock_label(), "Unlock bar 6");
        p.bar(3);
        assert!(!p.only_locked());
        assert_eq!(p.scope(), Scope::Bars(vec![3]), "the locked bar stays out");
        assert_eq!(p.lock_label(), "Lock 2 bars");
        assert_eq!(
            p.lock(),
            "continue: 2 bars locked: it stays through every try"
        );
        assert_eq!(p.locked, vec![3, 5]);
        p.bar(3);
        assert_eq!(p.lock(), "continue: bar 4 unlocked");
        assert_eq!(p.locked, vec![5]);
    }

    #[test]
    fn the_cards_say_the_settings_in_words() {
        let c = cards(opening_q());
        assert_eq!(c[0], ("Similarity", "60%".to_string()));
        assert_eq!(c[2].1, "As yours");
        assert_eq!(c[3].1, "Medium");
        assert_eq!(cards([0, 0, 20, 2])[2].1, "Sparser 20%");
        assert_eq!(cards([0, 0, 80, 0])[3].1, "Narrow");
    }
}

/// The tool in the note editor: what each touch asks of the time engine.
#[cfg(test)]
mod editor_tests {
    use super::{Compose, Picks, CONTINUE_ROWS};
    use crate::continuing::Scope;
    use crate::editor::{edit_rows, EditAction, Editor, ToolAct, CONTINUE_ROW};
    use mpfree_seq::{Clip, Holds, Note};

    fn riff() -> Clip {
        let mut c = Clip::new("Riff", 3840);
        c.holds = Holds::Notes;
        for (k, n) in [60u8, 63, 67, 65].into_iter().enumerate() {
            c.add_note(Note::new(k as i64 * 960, 480, n, 100)).unwrap();
        }
        c
    }

    fn row(row: usize, delta: i32) -> EditAction {
        EditAction::Row { row, delta }
    }

    fn asked(d: &crate::editor::Done) -> mpfree_compose::Settings {
        match d.compose {
            Some(Compose::Ask(ref s)) => s.settings,
            ref other => panic!("no ask: {other:?} ({})", d.line),
        }
    }

    /// Continue opens on the whole clip with nothing picked and asks at
    /// once; the drawer's rows become its settings and the Q-Links its
    /// own, each change asking again; Try again asks with the same; Done
    /// and Cancel ask for Keep and Back, the tool staying open until the
    /// time engine closes it; it opens again as it was left.
    #[test]
    fn continue_opens_on_the_whole_clip_and_each_change_asks_again() {
        let c = riff();
        let mut ed = Editor::new(0, 0, &c);
        assert!(ed.sel.is_empty());
        assert_eq!(edit_rows(&ed, &c)[CONTINUE_ROW].value, "+4 bars");
        let d = ed.act(row(CONTINUE_ROW, 1), &c);
        assert!(ed.continuing_open());
        assert!(d.edit.is_empty(), "nothing goes into the clip");
        assert_eq!(asked(&d).bars, 4);
        let rows = edit_rows(&ed, &c);
        assert_eq!(rows.len(), CONTINUE_ROWS.len());
        assert_eq!(
            (rows[0].name.as_str(), rows[0].value.as_str()),
            ("Style", "Minimal")
        );
        // A row picked, then stepped: a new ask, said.
        assert!(ed.act(EditAction::PickRow(1), &c).compose.is_none());
        assert_eq!(ed.row_picked(), 1);
        let d = ed.act(row(1, -1), &c);
        assert_eq!(asked(&d).bars, 2);
        assert_eq!(d.line, "continue: 2 more bars");
        // At its end, nothing to ask.
        ed.act(row(1, -1), &c);
        assert!(ed.act(row(1, -1), &c).compose.is_none());
        let d = ed.act(row(3, 1), &c);
        assert!(asked(&d).keeps.rhythm);
        // The Q-Links: Similarity up two detents.
        let knob = EditAction::Knob {
            k: 0,
            ticks: 2,
            fine: false,
            turn: 1,
        };
        let s = asked(&ed.act(knob, &c));
        assert!((s.similarity - 0.7).abs() < 1e-6);
        assert_eq!(ed.continuing.q[0], 70);
        // Try again: the same settings.
        let again = asked(&ed.act(EditAction::Tool(ToolAct::Option), &c));
        assert_eq!(again, s);
        // Anything else waits for Done or Cancel.
        let d = ed.act(EditAction::Correct, &c);
        assert!(d.compose.is_none() && d.edit.is_empty());
        assert!(
            d.line.contains("Done or Cancel Continue first"),
            "{}",
            d.line
        );
        // Done and Cancel ask; the tool stays open until the time engine
        // has done it.
        let d = ed.act(EditAction::Tool(ToolAct::Done), &c);
        assert_eq!(d.compose, Some(Compose::Keep));
        assert!(ed.continuing_open());
        let d = ed.act(EditAction::Tool(ToolAct::Cancel), &c);
        assert_eq!(d.compose, Some(Compose::Back));
        ed.open_tool = None;
        assert_eq!(ed.row_picked(), ed.row);
        // Opened again: as it was left.
        let s = asked(&ed.act(row(CONTINUE_ROW, 1), &c));
        assert_eq!((s.bars, s.keeps.rhythm), (1, true));
        assert!((s.similarity - 0.7).abs() < 1e-6);
    }

    fn scope_of(d: &crate::editor::Done) -> (Scope, Vec<u16>) {
        match d.compose {
            Some(Compose::Ask(ref s)) => (s.scope.clone(), s.locked.clone()),
            ref other => panic!("no ask: {other:?} ({})", d.line),
        }
    }

    /// Phase 1 step 3: bars picked on the ruler and notes on the roll go
    /// with Try again; a locked bar goes with every ask, a setting changed
    /// too; only locked bars picked, Try again says why it can't; the
    /// picks start empty each time Continue opens.
    #[test]
    fn continue_tries_again_on_what_is_picked_and_keeps_locks() {
        let c = riff();
        let mut ed = Editor::new(0, 0, &c);
        ed.act(row(CONTINUE_ROW, 1), &c);
        let try_again = EditAction::Tool(ToolAct::Option);
        let d = ed.act(EditAction::ContinueBar(2), &c);
        assert_eq!(d.line, "continue: bar 3 picked: Try again remakes it alone");
        assert!(d.compose.is_none() && d.edit.is_empty());
        assert_eq!(
            scope_of(&ed.act(try_again.clone(), &c)),
            (Scope::Bars(vec![2]), vec![])
        );
        let d = ed.act(EditAction::ContinueLock, &c);
        assert_eq!(d.line, "continue: bar 3 locked: it stays through every try");
        assert!(d.compose.is_none());
        assert_eq!(
            scope_of(&ed.act(try_again.clone(), &c)),
            (Scope::All, vec![2])
        );
        assert_eq!(scope_of(&ed.act(row(1, -1), &c)), (Scope::All, vec![2]));
        ed.act(EditAction::ContinueNote(4000), &c);
        assert_eq!(
            scope_of(&ed.act(try_again.clone(), &c)),
            (Scope::Notes(vec![4000]), vec![2])
        );
        ed.act(EditAction::ContinueBar(2), &c);
        let d = ed.act(try_again, &c);
        assert!(d.compose.is_none());
        assert_eq!(
            d.line,
            "continue: the picked bars are locked: Unlock them to try them again"
        );
        // Closed and opened again: nothing picked, nothing locked.
        ed.act(EditAction::Tool(ToolAct::Cancel), &c);
        ed.open_tool = None;
        assert_eq!(
            scope_of(&ed.act(row(CONTINUE_ROW, 1), &c)),
            (Scope::All, vec![])
        );
        assert_eq!(ed.continue_picks, Picks::default());
    }

    /// A clip of pads has no tune, and an empty clip nothing to carry on:
    /// Continue says so and stays closed.
    #[test]
    fn continue_says_why_it_cant_open() {
        let empty = {
            let mut c = Clip::new("New", 3840);
            c.holds = Holds::Notes;
            c
        };
        let mut ed = Editor::new(0, 0, &empty);
        assert_eq!(edit_rows(&ed, &empty)[CONTINUE_ROW].value, "no notes");
        let d = ed.act(row(CONTINUE_ROW, 1), &empty);
        assert!(d.compose.is_none() && !ed.continuing_open());
        assert_eq!(
            d.line,
            "continue: play or draw some notes to carry on first"
        );
        let mut pads = riff();
        pads.holds = Holds::Hits;
        let mut ed = Editor::new(0, 0, &pads);
        assert_eq!(edit_rows(&ed, &pads)[CONTINUE_ROW].value, "no tune");
        let d = ed.act(row(CONTINUE_ROW, 1), &pads);
        assert!(d.compose.is_none() && !ed.continuing_open());
        assert!(d.line.contains("plays pads"), "{}", d.line);
    }
}
