//! The Looper tab's panels (mockup J): the loops with the memory they
//! share, the picked loop's editor with a row per layer, how the looper
//! records, and what to do with the loop.
//!
//! Each panel keeps what a touch asked for as [`LoopAct`]s and answers the
//! touch with a plain event; the page collects them with [`take_actions`]
//! and the app turns them into looper commands. The panels only draw what
//! they are given ([`crate::looper_page::LooperPage::update`] fills them).

use crate::looping::LoopWave;
use mpfree_looper::LayerCommand;
use mpfree_ui::draw::{self, card, card_head, dot, speaker, speaker_hit};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Tone, Touch, Ui, Widget};
use std::cell::RefCell;
use std::sync::Arc;

/// The panels' widget names.
pub const LOOPS: &str = "lloops";
pub const EDITOR: &str = "leditor";
pub const RECORDING: &str = "lrecording";
pub const USE: &str = "luse";
pub const QLINKS: &str = "lqlinks";

/// The loops' colours, loop 1 first (as mockup J).
pub const LOOP_COLOURS: [Color; 8] = [
    draw::KNOB[1],
    draw::KNOB[0],
    draw::KNOB[2],
    draw::KNOB[3],
    draw::PERC,
    draw::KICK,
    draw::VOX,
    draw::FX,
];

/// What a touch on the Looper tab asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopAct {
    /// Pick loop `n`.
    Pick(usize),
    /// Pick the first empty loop.
    NewLoop,
    /// Loop `n`'s speaker: mute it or let it play, without picking it. A
    /// muted loop stays silent when PLAY starts the others (Jay, 13 Nov).
    Mute(usize),
    /// Clear the picked loop: every layer, the first pass too.
    Clear,
    /// Show layer `k` (play order) on the Q-Links.
    Layer(usize),
    /// A control of layer `k` (play order).
    LayerCmd(usize, LayerCommand),
    /// Row `k` of the Recording and input panel, stepped by `delta`.
    Setting(usize, i32),
    /// One of the editor's tools ([`LoopEditor::tools`]).
    Tool(usize),
    /// A tool drawn faint (it can't work now) was tapped: the screen says
    /// why in place.
    FaintTool(usize),
    /// Go to row `r` of the undo history ([`HistoryList::rows`]): back or
    /// forward to just after that step.
    Step(usize),
    /// Keep ticks `from..to` of the picked loop: its trim handles.
    Trim(i64, i64),
    /// One of Use the loop's rows.
    Use(usize),
    /// A row drawn faint (it can't work now) was tapped: the screen says
    /// why in place.
    FaintUse(usize),
}

/// Every [`LoopAct`] the Looper tab's panels have kept since the last call.
pub fn take_actions(ui: &mut Ui) -> Vec<LoopAct> {
    let mut acts = Vec::new();
    if let Some(w) = ui.get_mut::<LoopList>(LOOPS) {
        acts.append(&mut w.acts);
    }
    if let Some(w) = ui.get_mut::<LoopEditor>(EDITOR) {
        acts.append(&mut w.acts);
    }
    if let Some(w) = ui.get_mut::<LoopSettings>(RECORDING) {
        acts.append(&mut w.acts);
    }
    if let Some(w) = ui.get_mut::<UseLoop>(USE) {
        acts.append(&mut w.acts);
    }
    acts
}

// ------------------------------------------------------------- loops

/// One loop in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopRow {
    pub name: String,
    /// "4 bars · 3 layers", "empty".
    pub detail: String,
    /// "PLAYING", "MUTED"; None when empty.
    pub state: Option<(String, Tone)>,
    pub colour: Color,
    /// The speaker on the card: Some(muted) once the loop has sound.
    pub speaker: Option<bool>,
}

/// The loops, the picked one outlined, a button for a new one, and the
/// memory they share. A tap on a loop picks it.
#[derive(Default)]
pub struct LoopList {
    pub rows: Vec<LoopRow>,
    pub picked: usize,
    /// "+ New loop" shows while an empty loop is left.
    pub new_loop: bool,
    /// "Clear loop N" shows while the picked loop has sound.
    pub clear: bool,
    /// "16 s of 5 min" and how full, 0 to 1.
    pub memory: (String, f32),
    acts: Vec<LoopAct>,
}

impl LoopList {
    /// Each loop's card, the New loop button, the Clear loop button and
    /// the memory gauge's area.
    pub(crate) fn parts(&self, r: Rect) -> ListParts {
        let (_, body) = card_head(r);
        let (gauge, body) = body.take_bottom(44);
        let n = self.rows.len().max(1) as i32;
        let button = 42;
        let gap = 8;
        let buttons = i32::from(self.new_loop) + i32::from(self.clear);
        let room = body.h - buttons * (button + gap);
        let h = ((room - (n - 1) * gap) / n).clamp(30, 62);
        let rows: Vec<Rect> = (0..self.rows.len() as i32)
            .map(|i| Rect::new(body.x, body.y + i * (h + gap), body.w, h))
            .collect();
        let mut below = rows.last().map_or(body.y, |l| l.bottom() + gap);
        let new = self.new_loop.then(|| {
            let b = Rect::new(body.x, below, body.w, button);
            below = b.bottom() + gap;
            b
        });
        let clear = self.clear.then(|| Rect::new(body.x, below, body.w, button));
        ListParts {
            rows,
            new,
            clear,
            gauge,
        }
    }

    /// Where loop card `rr`'s speaker sits (bottom right).
    pub(crate) fn speaker_at(rr: Rect) -> (i32, i32) {
        (rr.right() - 22, rr.bottom() - 18)
    }

    /// Where loop card `rr` answers a tap to mute: its bottom right, 64 px
    /// wide and up to the state line (Jay after live test 3: bigger
    /// mutes).
    pub(crate) fn mute_hit(rr: Rect) -> Rect {
        let top = (rr.y + 24).min(rr.bottom() - 30).max(rr.y);
        Rect::new(rr.right() - 64, top, 64, rr.bottom() - top)
    }
}

/// [`LoopList::parts`]: the cards, the buttons and the gauge.
pub(crate) struct ListParts {
    pub rows: Vec<Rect>,
    pub new: Option<Rect>,
    pub clear: Option<Rect>,
    pub gauge: Rect,
}

impl Widget for LoopList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, "Loops", "");
        let ListParts {
            rows,
            new,
            clear,
            gauge,
        } = self.parts(r);
        for (i, (rr, row)) in rows.iter().zip(&self.rows).enumerate() {
            let picked = i == self.picked;
            cx.canvas.fill_round_rect(
                *rr,
                10,
                if picked {
                    t.raised
                } else {
                    t.bg.mix(t.surface, 0.5)
                },
            );
            if picked {
                cx.canvas.stroke_round_rect(*rr, 10, 2, draw::KNOB[0]);
            }
            cx.canvas.fill_round_rect(
                Rect::new(rr.x + 10, rr.y + 10, 4, (rr.h - 20).max(4)),
                2,
                row.colour,
            );
            let inner = Rect::new(rr.x + 22, rr.y, rr.w - 32, rr.h);
            let two = rr.h >= 50;
            let (top, bottom) = if two {
                let (a, b) = inner.take_top(rr.h / 2 + 2);
                (
                    a.take_top(8).1,
                    b.take_bottom(rr.h / 2 - 2).0.take_top(rr.h / 2 - 12).0,
                )
            } else {
                (inner, inner)
            };
            cx.text(FontId::Bold, 15.0, top, Align::Left, t.text, &row.name);
            if let Some((state, tone)) = &row.state {
                let c = match tone {
                    Tone::Normal | Tone::Dim => t.text_faint,
                    k => t.tone(*k),
                };
                cx.text(FontId::Bold, 11.0, top, Align::Right, c, state);
            }
            if two {
                cx.text(
                    FontId::Sans,
                    12.5,
                    bottom,
                    Align::Left,
                    t.text_dim,
                    &row.detail,
                );
            }
            if let Some(muted) = row.speaker {
                let (x, y) = LoopList::speaker_at(*rr);
                if rr.h >= 44 {
                    let b = Rect::new(x - 18, y - 14, 38, 28);
                    cx.canvas.stroke_round_rect(b, 7, 1, t.outline);
                }
                speaker(
                    cx,
                    x,
                    y,
                    if muted { t.text_faint } else { t.text_dim },
                    muted,
                );
            }
        }
        if let Some(b) = new {
            cx.canvas.stroke_round_rect(b, 10, 1, t.outline);
            cx.text(
                FontId::Bold,
                14.0,
                b,
                Align::Center,
                t.text_dim,
                "+ New loop",
            );
        }
        if let Some(b) = clear {
            cx.canvas.stroke_round_rect(b, 10, 1, t.bad);
            let text = format!("Clear loop {}", self.picked + 1);
            cx.text(FontId::Bold, 14.0, b, Align::Center, t.bad, &text);
        }
        // The memory every loop shares.
        let (label, bar) = gauge.take_top(26);
        cx.text(
            FontId::Bold,
            11.0,
            label,
            Align::Left,
            t.text_faint,
            "LOOP MEMORY",
        );
        cx.text(
            FontId::Bold,
            11.5,
            label,
            Align::Right,
            t.text_dim,
            &self.memory.0,
        );
        let track = Rect::new(bar.x, bar.y + 6, bar.w, 8);
        cx.canvas.fill_round_rect(track, 4, t.raised);
        let full = self.memory.1.clamp(0.0, 1.0);
        if full > 0.0 {
            let w = ((track.w as f32 * full) as i32).max(8);
            let c = if full > 0.9 { t.bad } else { draw::KNOB[0] };
            cx.canvas
                .fill_round_rect(Rect::new(track.x, track.y, w, track.h), 4, c);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let ListParts {
            rows, new, clear, ..
        } = self.parts(r);
        let mut out = Vec::new();
        for (i, rr) in rows.iter().enumerate() {
            let name = &self.rows[i].name;
            if self.rows[i].speaker.is_some() {
                let (sx, sy) = LoopList::speaker_at(*rr);
                let hit = speaker_hit(sx, sy);
                out.push(Target::tap("loop-mute", i, format!("{name} speaker"), hit));
                // The row less its speaker picks the loop.
                out.push(Target::tap(
                    "loop",
                    i,
                    name.clone(),
                    Rect::new(rr.x, rr.y, hit.x - rr.x, rr.h),
                ));
            } else {
                out.push(Target::tap("loop", i, name.clone(), *rr));
            }
        }
        if let Some(b) = new {
            out.push(Target::tap("new-loop", 0, "+ New loop", b));
        }
        if let Some(b) = clear {
            out.push(Target::tap("clear", 0, "Clear", b));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let ListParts {
            rows, new, clear, ..
        } = self.parts(r);
        if let Some(i) = rows.iter().position(|rr| rr.contains(x, y)) {
            if self.rows[i].speaker.is_some() && LoopList::mute_hit(rows[i]).contains(x, y) {
                self.acts.push(LoopAct::Mute(i));
                return Some(EventKind::Toggle(i));
            }
            self.acts.push(LoopAct::Pick(i));
            return Some(EventKind::Select(i));
        }
        if new.is_some_and(|b| b.contains(x, y)) {
            self.acts.push(LoopAct::NewLoop);
            return Some(EventKind::Tab(0));
        }
        if clear.is_some_and(|b| b.contains(x, y)) {
            self.acts.push(LoopAct::Clear);
            return Some(EventKind::Tab(1));
        }
        None
    }
}

// ------------------------------------------------------------- editor

/// One layer's row.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerRow {
    /// "Layer 2".
    pub name: String,
    /// "overdub · bass".
    pub detail: String,
    pub colour: Color,
    pub muted: bool,
    pub solo: bool,
    /// Heard: not muted, not left out by a solo.
    pub heard: bool,
    /// 0 to 2 (1 is as recorded).
    pub level: f32,
    pub recording: bool,
}

/// The editor's tools, in order (they work while the loop plays; Undo
/// history is still to come).
pub const TOOLS: [&str; 8] = [
    "½ length",
    "× 2",
    "Reverse",
    "← 1/16",
    "1/16 →",
    "Seam fade 10 ms",
    "Merge layers",
    "Undo history",
];

/// The level bar's range: -24 to +6 dB.
const LEVEL_FLOOR_DB: f32 = -24.0;
const LEVEL_TOP_DB: f32 = 6.0;

/// Where a level (0 to 2) sits along the level bar, 0 to 1.
pub fn level_frac(level: f32) -> f32 {
    let db = 20.0 * level.max(1e-6).log10();
    ((db - LEVEL_FLOOR_DB) / (LEVEL_TOP_DB - LEVEL_FLOOR_DB)).clamp(0.0, 1.0)
}

fn level_of_frac(f: f32) -> f32 {
    if f <= 0.0 {
        return 0.0;
    }
    let db = LEVEL_FLOOR_DB + f.clamp(0.0, 1.0) * (LEVEL_TOP_DB - LEVEL_FLOOR_DB);
    10f32.powf(db / 20.0)
}

/// A level as the row prints it: "-3.5 dB", "-inf dB".
pub fn level_text(level: f32) -> String {
    if level <= 1e-6 {
        "-inf dB".to_string()
    } else {
        let db = 20.0 * level.log10();
        format!("{db:.1} dB").replace("-0.0", "0.0")
    }
}

/// The undo history as the editor lists it: "Start" (the first pass),
/// then each step, oldest first, and the row that is now. The rows after
/// it are the steps Redo would bring back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryList {
    pub rows: Vec<String>,
    pub now: usize,
}

/// The picked loop: its tools, its wave with the bar lines and the
/// playhead, and a row per layer with its speaker, solo, level, wave and
/// delete. A tap on a row shows that layer on the Q-Links. With the undo
/// history open, the history is listed in the layers' place: a tap on a
/// step goes back (or forward) to it, and a drag scrolls the list.
#[derive(Default)]
pub struct LoopEditor {
    /// "Loop 1 · Editor".
    pub title: String,
    /// "4 bars at 92 BPM · 10.4 s".
    pub right: String,
    /// Each tool's label and whether it works now.
    pub tools: Vec<(String, bool)>,
    pub wave: Option<Arc<LoopWave>>,
    /// The playhead, 0 to 1.
    pub position: Option<f32>,
    pub bars: usize,
    pub layers: Vec<LayerRow>,
    /// The layer on the Q-Links.
    pub picked: usize,
    /// Shown in the wave's place while the loop is empty.
    pub note: String,
    /// The tool drawn lit (the undo history while it is open).
    pub lit: Option<usize>,
    /// The loop's length in ticks while it can be trimmed: the wave has a
    /// handle at each end, to drag in by 1/16 notes.
    pub trim_ticks: Option<i64>,
    /// A finger on a trim handle (true: the end), and the range kept so
    /// far.
    trimming: Option<(u64, bool, i64, i64)>,
    /// The undo history, in the layers' place while it is open.
    history: Option<HistoryList>,
    /// The history's first row shown, once a drag has scrolled it; until
    /// then the list follows now.
    history_top: Option<usize>,
    /// A finger on the history: where it went down, the first row shown
    /// then, and whether it has moved enough to be a drag.
    history_press: Option<(u64, i32, usize, bool)>,
    /// The first row shown, when there are more layers than fit.
    top: usize,
    /// A finger on a level bar: its layer.
    dragging: Option<(u64, usize)>,
    /// The tools' widths as last drawn (a touch has no canvas to measure).
    tool_widths: RefCell<Vec<i32>>,
    /// The playhead's x and the rest as last drawn.
    drawn: RefCell<Option<(Rect, EditorLook, i32)>>,
    acts: Vec<LoopAct>,
}

/// What the editor shows apart from the playhead.
#[derive(Clone, Debug, PartialEq)]
struct EditorLook {
    title: String,
    right: String,
    tools: Vec<(String, bool)>,
    wave: Option<usize>,
    bars: usize,
    layers: Vec<LayerRow>,
    picked: usize,
    top: usize,
    note: String,
    lit: Option<usize>,
    history: Option<(HistoryList, usize)>,
    trim: Option<i64>,
    trimming: Option<(i64, i64)>,
}

/// Where a layer row's parts are.
pub(crate) struct RowParts {
    pub speaker: (i32, i32),
    pub solo: Rect,
    pub level: Rect,
    pub wave: Rect,
    pub delete: Rect,
}

impl LoopEditor {
    const ROW_H: i32 = 72;
    const ROW_GAP: i32 = 8;
    /// The undo history's rows.
    const STEP_H: i32 = 30;
    const STEP_GAP: i32 = 4;

    /// Open the undo history on `h`, or close it (None). The list follows
    /// now again when now moves.
    pub fn set_history(&mut self, h: Option<HistoryList>) {
        if h.as_ref().map(|h| h.now) != self.history.as_ref().map(|h| h.now) {
            self.history_top = None;
        }
        self.history = h;
    }

    pub fn history(&self) -> Option<&HistoryList> {
        self.history.as_ref()
    }

    /// A 1/16 note in ticks: the trim handles' step.
    const SIXTEENTH: i64 = mpfree_transport::time::PPQ as i64 / 4;

    /// The trim handles' grips on the wave `w` (inside the wave's box):
    /// the start's and the end's.
    pub(crate) fn trim_grips(w: Rect) -> (Rect, Rect) {
        let h = (w.h / 3).max(20);
        let y = w.y + (w.h - h) / 2;
        (
            Rect::new(w.x - 6, y, 12, h),
            Rect::new(w.right() - 6, y, 12, h),
        )
    }

    /// The tick a finger at `x` on the wave `w` points to, to the nearest
    /// 1/16 note.
    fn tick_at(w: Rect, x: i32, ticks: i64) -> i64 {
        let f = ((x - w.x) as f64 / w.w.max(1) as f64).clamp(0.0, 1.0);
        let t = (f * ticks as f64 / Self::SIXTEENTH as f64).round() as i64 * Self::SIXTEENTH;
        t.clamp(0, ticks)
    }

    /// How many history rows fit in `rows`.
    fn steps_fit(rows: Rect) -> usize {
        ((rows.h + Self::STEP_GAP) / (Self::STEP_H + Self::STEP_GAP)).max(1) as usize
    }

    /// The history's first row shown: where a drag left it, or with now
    /// and up to two steps after it in view.
    fn history_top(&self, rows: Rect) -> usize {
        let Some(h) = &self.history else { return 0 };
        let fit = LoopEditor::steps_fit(rows);
        let last = h.rows.len().saturating_sub(fit);
        self.history_top
            .unwrap_or_else(|| (h.now + 3).saturating_sub(fit))
            .min(last)
    }

    /// The history rows shown: (row, its rectangle).
    pub(crate) fn step_rects(&self, rows: Rect) -> Vec<(usize, Rect)> {
        let Some(h) = &self.history else {
            return Vec::new();
        };
        let top = self.history_top(rows);
        let fit = LoopEditor::steps_fit(rows);
        (top..h.rows.len().min(top + fit))
            .enumerate()
            .map(|(i, k)| {
                let y = rows.y + i as i32 * (Self::STEP_H + Self::STEP_GAP);
                (k, Rect::new(rows.x, y, rows.w, Self::STEP_H))
            })
            .collect()
    }

    /// The tools' row, the wave and the layer rows' area.
    pub(crate) fn parts(r: Rect) -> (Rect, Rect, Rect) {
        let (_, body) = card_head(r);
        let (tools, rest) = body.take_top(32);
        let (wave, rows) = rest.take_top(14).1.take_top(100);
        (tools, wave, rows.take_top(14).1)
    }

    fn tool_rects(&self, row: Rect) -> Vec<Rect> {
        let mut x = row.x;
        let mut out = Vec::new();
        for &w in self.tool_widths.borrow().iter() {
            if x + w > row.right() {
                break;
            }
            out.push(Rect::new(x, row.y, w, row.h));
            x += w + 8;
        }
        out
    }

    /// How many rows fit in `rows`.
    fn fit(rows: Rect) -> usize {
        ((rows.h + Self::ROW_GAP) / (Self::ROW_H + Self::ROW_GAP)).max(1) as usize
    }

    /// The rows shown: (layer, its rectangle).
    pub(crate) fn row_rects(&self, rows: Rect) -> Vec<(usize, Rect)> {
        let fit = LoopEditor::fit(rows);
        let h = ((rows.h - (fit as i32 - 1) * Self::ROW_GAP) / fit as i32).min(Self::ROW_H);
        (self.top..self.layers.len().min(self.top + fit))
            .enumerate()
            .map(|(i, k)| {
                (
                    k,
                    Rect::new(rows.x, rows.y + i as i32 * (h + Self::ROW_GAP), rows.w, h),
                )
            })
            .collect()
    }

    /// Where layer row `rr` answers a tap to mute: the speaker's whole
    /// column under the name, up to the S button.
    pub(crate) fn mute_hit(rr: Rect) -> Rect {
        let top = (rr.y + 30).min(rr.bottom() - 32).max(rr.y);
        Rect::new(rr.x, top, 42, rr.bottom() - top)
    }

    pub(crate) fn row_parts(rr: Rect) -> RowParts {
        let low = rr.y + rr.h * 2 / 3 + 1;
        RowParts {
            speaker: (rr.x + 24, low),
            solo: Rect::new(rr.x + 46, low - 13, 30, 26),
            level: Rect::new(rr.x + 86, low - 3, 90, 6),
            wave: Rect::new(rr.x + 250, rr.y + 10, rr.w - 250 - 50, rr.h - 20),
            delete: Rect::new(rr.right() - 40, rr.y + rr.h / 2 - 16, 32, 32),
        }
    }

    /// Keep the picked layer in view, with the editor in `r`.
    pub fn follow_in(&mut self, r: Rect) {
        let fit = LoopEditor::fit(LoopEditor::parts(r).2);
        let n = self.layers.len();
        if self.picked < self.top {
            self.top = self.picked;
        } else if self.picked >= self.top + fit {
            self.top = self.picked + 1 - fit;
        }
        self.top = self.top.min(n.saturating_sub(fit));
    }

    fn play_x(&self, w: Rect) -> Option<i32> {
        self.position
            .filter(|_| self.wave.as_ref().is_some_and(|w| w.frames > 0))
            .map(|p| w.x + (p.clamp(0.0, 1.0) * w.w as f32) as i32)
    }

    /// Everything but the playhead, for knowing when only it moved.
    fn look(&self) -> EditorLook {
        EditorLook {
            title: self.title.clone(),
            right: self.right.clone(),
            tools: self.tools.clone(),
            wave: self.wave.as_ref().map(|w| Arc::as_ptr(w) as usize),
            bars: self.bars,
            layers: self.layers.clone(),
            picked: self.picked,
            top: self.top,
            note: self.note.clone(),
            lit: self.lit,
            history: self
                .history
                .clone()
                .map(|h| (h, self.history_top.unwrap_or(usize::MAX))),
            trim: self.trim_ticks,
            trimming: self.trimming.map(|t| (t.2, t.3)),
        }
    }
}

impl Widget for LoopEditor {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, &self.title, &self.right);
        let (tool_row, wave_r, rows) = LoopEditor::parts(r);
        // The tools.
        let widths: Vec<i32> = self
            .tools
            .iter()
            .map(|(l, _)| cx.measure(FontId::Bold, 13.0, l) + 26)
            .collect();
        *self.tool_widths.borrow_mut() = widths;
        for (i, (tr, (label, on))) in self
            .tool_rects(tool_row)
            .iter()
            .zip(&self.tools)
            .enumerate()
        {
            let lit = self.lit == Some(i);
            cx.canvas
                .fill_round_rect(*tr, 8, if lit { t.accent } else { t.raised });
            cx.text(
                FontId::Bold,
                13.0,
                *tr,
                Align::Center,
                if lit { t.bg } else { t.button_ink(*on) },
                label,
            );
        }
        // The loop.
        cx.canvas
            .fill_round_rect(wave_r, 10, t.bg.mix(t.surface, 0.4));
        let w = wave_r.inset_xy(10, 6);
        let play = self.play_x(w);
        match self.wave.as_ref().filter(|wv| wv.frames > 0) {
            Some(wave) => {
                let heard: Vec<bool> = self.layers.iter().map(|l| l.heard).collect();
                paint_layers(cx, w, wave, play, &heard);
                let bars = self.bars.max(1);
                for b in 1..bars {
                    let bx = w.x + (b as f32 / bars as f32 * w.w as f32) as i32;
                    cx.canvas.fill_rect(Rect::new(bx, w.y, 1, w.h), t.outline);
                }
                if let Some(px) = play {
                    cx.canvas
                        .fill_rect(Rect::new(px - 1, wave_r.y + 2, 2, wave_r.h - 4), t.text);
                }
                if let Some(ticks) = self.trim_ticks.filter(|&t| t > 0) {
                    paint_trim(cx, wave_r, w, ticks, self.trimming.map(|t| (t.2, t.3)));
                }
            }
            None => {
                cx.text(
                    FontId::Sans,
                    14.0,
                    wave_r,
                    Align::Center,
                    t.text_dim,
                    &self.note,
                );
            }
        }
        if let Some(h) = &self.history {
            paint_history(cx, rows, h, &self.step_rects(rows));
            *self.drawn.borrow_mut() = Some((r, self.look(), play.unwrap_or(-1)));
            return;
        }
        // The layers.
        for (k, rr) in self.row_rects(rows) {
            let l = &self.layers[k];
            let picked = k == self.picked;
            cx.canvas.fill_round_rect(
                rr,
                10,
                if picked {
                    t.raised
                } else {
                    t.bg.mix(t.surface, 0.6)
                },
            );
            if picked {
                cx.canvas.stroke_round_rect(rr, 10, 2, draw::KNOB[0]);
            }
            let c = if l.heard { l.colour } else { t.text_faint };
            let ink = if l.heard { t.text } else { t.text_faint };
            let top = Rect::new(rr.x + 12, rr.y + 10, rr.w / 3, 18);
            cx.canvas
                .fill_round_rect(Rect::new(top.x, top.y + 5, 9, 9), 2, c);
            let nw = cx.measure(FontId::Bold, 14.0, &l.name);
            cx.text(
                FontId::Bold,
                14.0,
                Rect::new(top.x + 16, top.y, nw + 4, top.h),
                Align::Left,
                ink,
                &l.name,
            );
            cx.text(
                FontId::Sans,
                12.0,
                Rect::new(top.x + 16 + nw + 12, top.y, 160, top.h),
                Align::Left,
                t.text_faint,
                &l.detail,
            );
            let p = LoopEditor::row_parts(rr);
            let b = Rect::new(p.speaker.0 - 18, p.speaker.1 - 13, 38, 26);
            cx.canvas.stroke_round_rect(b, 7, 1, t.outline);
            speaker(cx, p.speaker.0, p.speaker.1, t.text_dim, l.muted);
            cx.canvas.fill_round_rect(
                p.solo,
                7,
                if l.solo {
                    t.warn
                } else {
                    t.bg.mix(t.surface, 0.2)
                },
            );
            cx.text(
                FontId::Bold,
                13.0,
                p.solo,
                Align::Center,
                if l.solo { t.bg } else { t.text },
                "S",
            );
            cx.canvas
                .fill_round_rect(p.level, 3, t.bg.mix(t.surface, 0.2));
            let f = level_frac(l.level);
            if f > 0.0 {
                cx.canvas.fill_round_rect(
                    Rect::new(
                        p.level.x,
                        p.level.y,
                        ((p.level.w as f32 * f) as i32).max(6),
                        p.level.h,
                    ),
                    3,
                    if l.heard { l.colour } else { t.text_faint },
                );
            }
            cx.text(
                FontId::Bold,
                12.5,
                Rect::new(p.level.right() + 10, p.level.y - 8, 70, 22),
                Align::Left,
                t.text_dim,
                &level_text(l.level),
            );
            if let Some(wave) = self.wave.as_ref().filter(|wv| wv.frames > 0) {
                if let Some(peaks) = wave.layers.get(k) {
                    paint_one_layer(cx, p.wave, peaks, c);
                }
            }
            if l.recording {
                dot(cx, p.wave.x - 12, rr.y + rr.h / 2, 4, t.bad);
            }
            cx.text(
                FontId::Sans,
                16.0,
                p.delete,
                Align::Center,
                t.text_faint,
                "×",
            );
        }
        if self.layers.len() > LoopEditor::fit(rows) {
            let more = format!(
                "layers {} to {} of {}",
                self.top + 1,
                (self.top + LoopEditor::fit(rows)).min(self.layers.len()),
                self.layers.len()
            );
            cx.text(
                FontId::Sans,
                11.5,
                Rect::new(rows.x, rows.bottom() - 2, rows.w, 14),
                Align::Right,
                t.text_faint,
                &more,
            );
        }
        *self.drawn.borrow_mut() = Some((r, self.look(), play.unwrap_or(-1)));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, look, old_x) = drawn.as_ref()?;
        if *at != r || *look != self.look() {
            return None;
        }
        let (_, wave_r, _) = LoopEditor::parts(r);
        let new_x = self.play_x(wave_r.inset_xy(10, 6)).unwrap_or(-1);
        if new_x == *old_x {
            return Some(Vec::new());
        }
        if *old_x < 0 || new_x < 0 || new_x < *old_x {
            return Some(vec![wave_r]);
        }
        Some(vec![Rect::new(
            old_x - 4,
            wave_r.y,
            new_x - old_x + 8,
            wave_r.h,
        )])
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (tool_row, wave_r, rows) = LoopEditor::parts(r);
        let mut out: Vec<Target> = self
            .tool_rects(tool_row)
            .into_iter()
            .enumerate()
            .map(|(i, b)| {
                let (label, on) = self
                    .tools
                    .get(i)
                    .map_or((String::new(), false), |t| (t.0.clone(), t.1));
                Target::tap("tool", i, label, b).disabled_if(!on)
            })
            .collect();
        if self.trim_ticks.is_some_and(|t| t > 0) {
            let (start, end) = LoopEditor::trim_grips(wave_r.inset_xy(10, 6));
            out.push(Target::drag("trim-start", 0, "trim start", start));
            // The end grip moves left: a drag right would leave it whole.
            out.push(Target::drag("trim-end", 0, "trim end", end).towards(-60, 0));
        }
        if let Some(h) = &self.history {
            for (k, sr) in self.step_rects(rows) {
                if k != h.now {
                    out.push(Target::tap("history-step", k, h.rows[k].clone(), sr));
                }
            }
            return out;
        }
        for (k, rr) in self.row_rects(rows) {
            let p = LoopEditor::row_parts(rr);
            let name = &self.layers[k].name;
            out.push(Target::tap(
                "layer-mute",
                k,
                format!("{name} speaker"),
                speaker_hit(p.speaker.0, p.speaker.1),
            ));
            out.push(Target::tap("layer-solo", k, format!("{name} S"), p.solo));
            out.push(Target::tap(
                "layer-delete",
                k,
                format!("{name} delete"),
                p.delete,
            ));
            out.push(Target::drag(
                "layer-level",
                k,
                format!("{name} level"),
                p.level,
            ));
            out.push(Target::tap("layer", k, name.clone(), p.wave));
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let (tool_row, _, rows) = LoopEditor::parts(r);
        match phase {
            Touch::Down => {
                if let Some(i) = self
                    .tool_rects(tool_row)
                    .iter()
                    .position(|b| b.contains(x, y))
                {
                    if self.tools.get(i).is_some_and(|t| t.1) {
                        self.acts.push(LoopAct::Tool(i));
                        return Some(EventKind::Tab(i));
                    }
                    self.acts.push(LoopAct::FaintTool(i));
                    return Some(EventKind::Tab(i));
                }
                let (_, wave_r, _) = LoopEditor::parts(r);
                let w = wave_r.inset_xy(10, 6);
                if let Some(ticks) = self.trim_ticks.filter(|&t| t > 0) {
                    let (start, end) = LoopEditor::trim_grips(w);
                    let near = |g: Rect| g.inset_xy(-14, -12).contains(x, y);
                    if near(start) || near(end) {
                        // The nearer end, when both are in reach.
                        let is_end = near(end) && (!near(start) || x > w.x + w.w / 2);
                        self.trimming = Some((finger, is_end, 0, ticks));
                        return None;
                    }
                }
                if self.history.is_some() {
                    if rows.contains(x, y) {
                        self.history_press = Some((finger, y, self.history_top(rows), false));
                    }
                    return None;
                }
                let (k, rr) = self
                    .row_rects(rows)
                    .into_iter()
                    .find(|(_, rr)| rr.contains(x, y))?;
                let l = self.layers[k].clone();
                let p = LoopEditor::row_parts(rr);
                let act = if LoopEditor::mute_hit(rr).contains(x, y) {
                    LoopAct::LayerCmd(k, LayerCommand::Mute(!l.muted))
                } else if p.solo.inset(-4).contains(x, y) {
                    LoopAct::LayerCmd(k, LayerCommand::Solo(!l.solo))
                } else if p.delete.contains(x, y) {
                    LoopAct::LayerCmd(k, LayerCommand::Delete)
                } else if p.level.inset_xy(-4, -14).contains(x, y) {
                    self.dragging = Some((finger, k));
                    let v = level_of_frac((x - p.level.x) as f32 / p.level.w.max(1) as f32);
                    self.layers[k].level = v;
                    LoopAct::LayerCmd(k, LayerCommand::Level(v))
                } else {
                    LoopAct::Layer(k)
                };
                if self.picked != k {
                    self.picked = k;
                    if !matches!(act, LoopAct::Layer(_)) {
                        self.acts.push(LoopAct::Layer(k));
                    }
                }
                self.acts.push(act);
                Some(EventKind::Select(k))
            }
            Touch::Move if self.trimming.is_some_and(|t| t.0 == finger) => {
                let (f, is_end, from, to) = self.trimming?;
                let ticks = self.trim_ticks.filter(|&t| t > 0)?;
                let (_, wave_r, _) = LoopEditor::parts(r);
                let t = LoopEditor::tick_at(wave_r.inset_xy(10, 6), x, ticks);
                let (from, to) = if is_end {
                    (from, t.max(from + Self::SIXTEENTH))
                } else {
                    (t.min(to - Self::SIXTEENTH), to)
                };
                self.trimming = Some((f, is_end, from, to));
                None
            }
            Touch::Move if self.history_press.is_some_and(|p| p.0 == finger) => {
                let (f, y0, top0, moved) = self.history_press?;
                let dy = y - y0;
                if !moved && dy.abs() < mpfree_ui::qlink::DRAG_DETENT {
                    return None;
                }
                let step = Self::STEP_H + Self::STEP_GAP;
                let top = (top0 as i32 - dy / step).max(0) as usize;
                self.history_press = Some((f, y0, top0, true));
                self.history_top = Some(top);
                let top = self.history_top(rows);
                self.history_top = Some(top);
                None
            }
            Touch::Move => {
                let (f, k) = self.dragging?;
                if f != finger {
                    return None;
                }
                let (_, rr) = self.row_rects(rows).into_iter().find(|(kk, _)| *kk == k)?;
                let p = LoopEditor::row_parts(rr);
                let v = level_of_frac((x - p.level.x) as f32 / p.level.w.max(1) as f32);
                if (v - self.layers[k].level).abs() < 1e-4 {
                    return None;
                }
                self.layers[k].level = v;
                self.acts.push(LoopAct::LayerCmd(k, LayerCommand::Level(v)));
                Some(EventKind::Select(k))
            }
            Touch::Up | Touch::Cancel => {
                if self.dragging.is_some_and(|(f, _)| f == finger) {
                    self.dragging = None;
                }
                if let Some((f, _, from, to)) = self.trimming.filter(|t| t.0 == finger) {
                    self.trimming = None;
                    let whole = self.trim_ticks.is_some_and(|t| from == 0 && to == t);
                    if phase == Touch::Cancel || whole || f != finger {
                        return None;
                    }
                    self.acts.push(LoopAct::Trim(from, to));
                    return Some(EventKind::Value(0.0));
                }
                let (f, _, _, moved) = self.history_press?;
                if f != finger {
                    return None;
                }
                self.history_press = None;
                if moved || phase == Touch::Cancel {
                    return None;
                }
                let (k, _) = self
                    .step_rects(rows)
                    .into_iter()
                    .find(|(_, sr)| sr.contains(x, y))?;
                if self.history.as_ref().is_some_and(|h| h.now == k) {
                    return None;
                }
                self.acts.push(LoopAct::Step(k));
                Some(EventKind::Select(k))
            }
        }
    }
}

/// The trim handles on the wave `w` (in the box `wave_r`) of a loop
/// `ticks` long; while one is dragged, the parts it cuts off are dimmed,
/// with what stays as a label.
fn paint_trim(cx: &mut PaintCx, wave_r: Rect, w: Rect, ticks: i64, kept: Option<(i64, i64)>) {
    let t = cx.theme.clone();
    let x_of = |tick: i64| w.x + (tick as f64 / ticks as f64 * w.w as f64) as i32;
    let (from, to) = kept.unwrap_or((0, ticks));
    let (a, b) = (x_of(from), x_of(to));
    if kept.is_some() {
        let shade = t.bg.mix(t.surface, 0.15);
        if a > wave_r.x {
            cx.canvas
                .fill_rect(Rect::new(wave_r.x, wave_r.y, a - wave_r.x, wave_r.h), shade);
        }
        if b < wave_r.right() {
            cx.canvas
                .fill_rect(Rect::new(b, wave_r.y, wave_r.right() - b, wave_r.h), shade);
        }
        let beats = |tk: i64| {
            let b = tk as f64 / mpfree_transport::time::PPQ as f64;
            format!("{b:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        let label = format!("keep {} of {} beats", beats(to - from), beats(ticks));
        cx.text(
            FontId::Bold,
            12.5,
            Rect::new(wave_r.x + 12, wave_r.y + 4, wave_r.w - 24, 16),
            Align::Center,
            t.text,
            &label,
        );
    }
    let (start, end) = LoopEditor::trim_grips(w);
    let start = Rect::new(a - 6, start.y, start.w, start.h);
    let end = Rect::new(b - 6, end.y, end.w, end.h);
    for g in [start, end] {
        cx.canvas.fill_round_rect(g, 4, t.text_dim);
        cx.canvas
            .fill_rect(Rect::new(g.x + g.w / 2 - 1, g.y + 6, 2, g.h - 12), t.bg);
    }
}

/// The undo history in `rows`: each step shown in `shown`, now outlined,
/// the steps Redo would bring back faint, and how many more there are.
fn paint_history(cx: &mut PaintCx, rows: Rect, h: &HistoryList, shown: &[(usize, Rect)]) {
    let t = cx.theme.clone();
    for &(k, sr) in shown {
        let now = k == h.now;
        let after = k > h.now;
        cx.canvas.fill_round_rect(
            sr,
            8,
            if now {
                t.raised
            } else {
                t.bg.mix(t.surface, 0.6)
            },
        );
        if now {
            cx.canvas.stroke_round_rect(sr, 8, 2, draw::KNOB[0]);
        }
        let ink = if after { t.text_faint } else { t.text };
        let label = Rect::new(sr.x + 14, sr.y, sr.w - 120, sr.h);
        cx.text(FontId::Bold, 13.0, label, Align::Left, ink, &h.rows[k]);
        let tag = if now {
            "now"
        } else if after {
            "redo"
        } else {
            ""
        };
        cx.text(
            FontId::Sans,
            12.0,
            Rect::new(sr.right() - 100, sr.y, 86, sr.h),
            Align::Right,
            if now { t.text_dim } else { t.text_faint },
            tag,
        );
    }
    let first = shown.first().map_or(0, |s| s.0);
    let last = shown.last().map_or(0, |s| s.0 + 1);
    if first > 0 || last < h.rows.len() {
        let more = format!("steps {} to {} of {}", first + 1, last, h.rows.len());
        cx.text(
            FontId::Sans,
            11.5,
            Rect::new(rows.x, rows.bottom() - 2, rows.w, 14),
            Align::Right,
            t.text_faint,
            &more,
        );
    }
}

/// The layers' peaks stacked in their colours across `w`, the part before
/// the playhead brighter; a layer not heard is grey.
pub fn paint_layers(cx: &mut PaintCx, w: Rect, wave: &LoopWave, play: Option<i32>, heard: &[bool]) {
    let t = cx.theme.clone();
    let mid = w.y + w.h / 2;
    let half = w.h as f32 / 2.0 - 2.0;
    let cols = crate::looping::WAVE_COLUMNS;
    let mut x = 0;
    while x < w.w {
        let col = (x as usize * cols / w.w.max(1) as usize).min(cols - 1);
        let past = play.is_some_and(|p| w.x + x < p);
        let mut acc = 0.0f32;
        for (li, layer) in wave.layers.iter().enumerate() {
            let amp = layer.get(col).copied().unwrap_or(0.0);
            if amp <= 0.001 {
                continue;
            }
            let y0 = (acc * half) as i32;
            acc += amp;
            let y1 = (acc * half).min(half) as i32;
            let c = if heard.get(li).copied().unwrap_or(true) {
                crate::looper_page::LAYER_COLOURS[li % crate::looper_page::LAYER_COLOURS.len()]
            } else {
                t.text_faint.mix(t.surface, 0.45)
            };
            let c = if past { c } else { c.mix(t.surface, 0.35) };
            if y1 > y0 {
                cx.canvas
                    .fill_rect(Rect::new(w.x + x, mid - y1, 2, y1 - y0), c);
                cx.canvas
                    .fill_rect(Rect::new(w.x + x, mid + y0, 2, y1 - y0), c);
            }
        }
        x += 3;
    }
}

/// One layer's peaks across `w` in `c`, its quiet parts left out.
fn paint_one_layer(cx: &mut PaintCx, w: Rect, peaks: &[f32], c: Color) {
    if peaks.is_empty() || w.w <= 0 {
        return;
    }
    let loudest = peaks.iter().copied().fold(0.0f32, f32::max).max(0.05);
    let mid = w.y + w.h / 2;
    let half = w.h as f32 / 2.0;
    let mut x = 0;
    while x < w.w {
        let col = (x as usize * peaks.len() / w.w as usize).min(peaks.len() - 1);
        let a = (peaks[col] / loudest).min(1.0);
        let h = (a * half) as i32;
        if a > 0.04 && h > 0 {
            cx.canvas
                .fill_rect(Rect::new(w.x + x, mid - h, 2, 2 * h), c);
        }
        x += 4;
    }
}

// ------------------------------------------------------------- recording

/// One setting: its label, value, how the value is drawn, and whether a tap
/// steps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingRow {
    pub label: String,
    pub value: String,
    pub tone: Tone,
    pub steps: bool,
}

/// How the looper records, in two columns: a tap on a row's right half
/// steps it up, on its left half down.
#[derive(Default)]
pub struct LoopSettings {
    pub rows: Vec<SettingRow>,
    pub right: String,
    acts: Vec<LoopAct>,
}

impl LoopSettings {
    pub(crate) fn row_rects(&self, r: Rect) -> Vec<Rect> {
        let (_, body) = card_head(r);
        let per = self.rows.len().div_ceil(2).max(1) as i32;
        let gap = 6;
        let colw = (body.w - 12) / 2;
        let h = ((body.h - (per - 1) * gap) / per).min(30);
        (0..self.rows.len() as i32)
            .map(|i| {
                let (c, k) = (i / per, i % per);
                Rect::new(body.x + c * (colw + 12), body.y + k * (h + gap), colw, h)
            })
            .collect()
    }
}

impl Widget for LoopSettings {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, "Recording and input", &self.right);
        for (rr, row) in self.row_rects(r).iter().zip(&self.rows) {
            cx.canvas.fill_round_rect(*rr, 7, t.raised);
            let inner = rr.inset_xy(10, 0);
            cx.text(
                FontId::Sans,
                13.0,
                inner,
                Align::Left,
                t.text_dim,
                &row.label,
            );
            let c = match row.tone {
                Tone::Normal => t.text,
                Tone::Dim => t.text_dim,
                k => t.tone(k),
            };
            cx.text(FontId::Bold, 13.0, inner, Align::Right, c, &row.value);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = Vec::new();
        for (i, rr) in self.row_rects(r).into_iter().enumerate() {
            // Rows that only show a value (the input's peak) are no controls.
            if !self.rows[i].steps {
                continue;
            }
            let label = &self.rows[i].label;
            let (less, more) = rr.take_left(rr.w / 2);
            out.push(Target::tap(
                "setting-less",
                i,
                format!("{label} (left half)"),
                less,
            ));
            out.push(Target::tap(
                "setting-more",
                i,
                format!("{label} (right half)"),
                more,
            ));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let rects = self.row_rects(r);
        let i = rects.iter().position(|rr| rr.contains(x, y))?;
        if !self.rows[i].steps {
            return None;
        }
        let delta = if x >= rects[i].x + rects[i].w / 2 {
            1
        } else {
            -1
        };
        self.acts.push(LoopAct::Setting(i, delta));
        Some(EventKind::Step { index: i, delta })
    }
}

// ------------------------------------------------------------- use

/// What to do with the loop: each row a title and what it does; rows that
/// cannot work yet are drawn faint, and a tap on one says why.
#[derive(Default)]
pub struct UseLoop {
    pub rows: Vec<(String, String, bool)>,
    acts: Vec<LoopAct>,
}

impl UseLoop {
    pub(crate) fn row_rects(&self, r: Rect) -> Vec<Rect> {
        let (_, body) = card_head(r);
        let n = self.rows.len().max(1) as i32;
        let gap = 6;
        let h = ((body.h - (n - 1) * gap) / n).min(34);
        (0..self.rows.len() as i32)
            .map(|i| Rect::new(body.x, body.y + i * (h + gap), body.w, h))
            .collect()
    }
}

impl Widget for UseLoop {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, "Use the loop", "");
        for (rr, (title, what, on)) in self.row_rects(r).iter().zip(&self.rows) {
            cx.canvas.fill_round_rect(*rr, 7, t.raised);
            let inner = rr.inset_xy(12, 0);
            cx.text(
                FontId::Bold,
                14.0,
                inner,
                Align::Left,
                t.button_ink(*on),
                title,
            );
            cx.text(FontId::Sans, 12.0, inner, Align::Right, t.text_dim, what);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.row_rects(r)
            .into_iter()
            .enumerate()
            .map(|(i, rr)| {
                Target::tap("use", i, self.rows[i].0.clone(), rr).disabled_if(!self.rows[i].2)
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = self.row_rects(r).iter().position(|rr| rr.contains(x, y))?;
        if !self.rows[i].2 {
            self.acts.push(LoopAct::FaintUse(i));
            return Some(EventKind::Select(i));
        }
        self.acts.push(LoopAct::Use(i));
        Some(EventKind::Select(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme};

    const W: i32 = 844;
    const H: i32 = 480;

    /// A page of just `id`, drawn once (the editor measures its tools as it
    /// draws them).
    fn page_of(id: &str, w: impl Widget + 'static) -> Ui {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), W, H);
        ui.add(id, w);
        ui.add_page("t", &format!("col({id}:*)")).unwrap();
        ui.show("t").unwrap();
        ui.paint(&mut SoftCanvas::new(W, H));
        ui
    }

    fn tap(ui: &mut Ui, x: i32, y: i32) -> Option<EventKind> {
        let e = ui.touch(1, Touch::Down, x, y).map(|e| e.kind);
        ui.touch(1, Touch::Up, x, y);
        e
    }

    fn layer(k: usize, muted: bool) -> LayerRow {
        LayerRow {
            name: format!("Layer {}", k + 1),
            detail: String::new(),
            colour: LOOP_COLOURS[k % LOOP_COLOURS.len()],
            muted,
            solo: false,
            heard: !muted,
            level: 1.0,
            recording: false,
        }
    }

    fn editor(layers: usize) -> LoopEditor {
        LoopEditor {
            title: "Loop 1 · Editor".into(),
            // Only Reverse works.
            tools: TOOLS
                .iter()
                .enumerate()
                .map(|(i, t)| (t.to_string(), i == 2))
                .collect(),
            layers: (0..layers).map(|k| layer(k, k == 2)).collect(),
            ..LoopEditor::default()
        }
    }

    /// Layer `k`'s parts, as drawn.
    fn parts_of(ui: &Ui, k: usize) -> RowParts {
        let r = ui.rect_of(EDITOR).unwrap();
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let (_, rr) = e
            .row_rects(LoopEditor::parts(r).2)
            .into_iter()
            .find(|(kk, _)| *kk == k)
            .expect("the layer is shown");
        LoopEditor::row_parts(rr)
    }

    #[test]
    fn a_layers_controls_ask_for_their_commands_and_pick_it() {
        let mut ui = page_of(EDITOR, editor(3));
        let r = ui.rect_of(EDITOR).unwrap();
        assert_eq!(
            ui.get::<LoopEditor>(EDITOR)
                .unwrap()
                .row_rects(LoopEditor::parts(r).2)
                .len(),
            3,
            "three layers fit"
        );
        // The third layer's speaker unmutes it, and picks it.
        let p = parts_of(&ui, 2);
        assert_eq!(
            tap(&mut ui, p.speaker.0, p.speaker.1),
            Some(EventKind::Select(2))
        );
        assert_eq!(
            take_actions(&mut ui),
            [
                LoopAct::Layer(2),
                LoopAct::LayerCmd(2, LayerCommand::Mute(false))
            ]
        );
        // The first's S solos it; its × deletes it.
        let p = parts_of(&ui, 0);
        tap(&mut ui, p.solo.x + 5, p.solo.y + 5);
        tap(&mut ui, p.delete.x + 5, p.delete.y + 5);
        assert_eq!(
            take_actions(&mut ui),
            [
                LoopAct::Layer(0),
                LoopAct::LayerCmd(0, LayerCommand::Solo(true)),
                LoopAct::LayerCmd(0, LayerCommand::Delete)
            ]
        );
        // A tap elsewhere on a row only picks it.
        let p = parts_of(&ui, 1);
        tap(&mut ui, p.wave.x + 20, p.wave.y + 5);
        assert_eq!(take_actions(&mut ui), [LoopAct::Layer(1)]);
        assert_eq!(ui.get::<LoopEditor>(EDITOR).unwrap().picked, 1);
    }

    #[test]
    fn a_level_bar_follows_the_finger() {
        let mut ui = page_of(EDITOR, editor(2));
        let p = parts_of(&ui, 1);
        let y = p.level.y + 3;
        ui.touch(7, Touch::Down, p.level.right(), y);
        ui.touch(7, Touch::Move, p.level.x + p.level.w / 2, y);
        // Another finger does not move it.
        ui.touch(8, Touch::Move, p.level.x, y);
        ui.touch(7, Touch::Up, p.level.x + p.level.w / 2, y);
        let acts = take_actions(&mut ui);
        let levels: Vec<f32> = acts
            .iter()
            .filter_map(|a| match a {
                LoopAct::LayerCmd(1, LayerCommand::Level(v)) => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(acts[0], LoopAct::Layer(1));
        assert_eq!(levels.len(), 2, "{acts:?}");
        assert_eq!(level_text(levels[0]), "6.0 dB", "the bar's end is +6 dB");
        assert!((levels[1] - level_of_frac(0.5)).abs() < 0.02);
        assert_eq!(level_frac(level_of_frac(0.25)), 0.25);
        assert_eq!(level_text(0.0), "-inf dB");
        assert_eq!(
            ui.get::<LoopEditor>(EDITOR).unwrap().layers[1].level,
            levels[1]
        );
    }

    /// A working tool acts; a faint one asks the screen to say why.
    #[test]
    fn working_tools_act_and_faint_ones_say_why() {
        let mut ui = page_of(EDITOR, editor(1));
        let r = ui.rect_of(EDITOR).unwrap();
        let tools = ui
            .get::<LoopEditor>(EDITOR)
            .unwrap()
            .tool_rects(LoopEditor::parts(r).0);
        assert_eq!(tools.len(), TOOLS.len(), "every tool fits");
        tap(&mut ui, tools[0].x + 4, tools[0].y + 4);
        assert_eq!(take_actions(&mut ui), [LoopAct::FaintTool(0)]);
        assert_eq!(
            tap(&mut ui, tools[2].x + 4, tools[2].y + 4),
            Some(EventKind::Tab(2))
        );
        assert_eq!(take_actions(&mut ui), [LoopAct::Tool(2)]);
    }

    #[test]
    fn the_undo_history_takes_the_layers_place_and_a_tap_goes_to_a_step() {
        let mut e = editor(3);
        let rows: Vec<String> = std::iter::once("Start: the first pass".to_string())
            .chain((1..40).map(|k| format!("Step {k}")))
            .collect();
        e.set_history(Some(HistoryList { rows, now: 30 }));
        let mut ui = page_of(EDITOR, e);
        let r = ui.rect_of(EDITOR).unwrap();
        let rows_r = LoopEditor::parts(r).2;
        let shown = ui.get::<LoopEditor>(EDITOR).unwrap().step_rects(rows_r);
        assert_eq!(shown.len(), LoopEditor::steps_fit(rows_r));
        assert_eq!(shown.last().unwrap().0, 32, "now and two steps after it");
        // A tap on a step goes to it as the finger lifts; on now, nothing;
        // no layer answers.
        let lift = |ui: &mut Ui, x: i32, y: i32| {
            assert_eq!(ui.touch(1, Touch::Down, x, y).map(|e| e.kind), None);
            ui.touch(1, Touch::Up, x, y).map(|e| e.kind)
        };
        let (k, sr) = shown[0];
        assert_eq!(
            lift(&mut ui, sr.x + 20, sr.y + 10),
            Some(EventKind::Select(k))
        );
        assert_eq!(take_actions(&mut ui), [LoopAct::Step(k)]);
        let now = shown.iter().find(|s| s.0 == 30).unwrap().1;
        assert_eq!(lift(&mut ui, now.x + 20, now.y + 10), None);
        assert!(take_actions(&mut ui).is_empty());
        // A drag down shows earlier steps, and is no tap.
        let (x, y) = (sr.x + 20, sr.y + 10);
        let step = LoopEditor::STEP_H + LoopEditor::STEP_GAP;
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Move, x, y + 3 * step);
        assert!(ui.touch(1, Touch::Up, x, y + 3 * step).is_none());
        assert!(take_actions(&mut ui).is_empty());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.step_rects(rows_r)[0].0, k - 3);
        // When now moves, the list follows it again.
        let mut h = e.history().unwrap().clone();
        h.now = 12;
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.set_history(Some(h));
        let shown = e.step_rects(rows_r);
        assert!(shown.iter().any(|s| s.0 == 12), "{shown:?}");
        ui.paint(&mut SoftCanvas::new(W, H));
        // Closed, the layers are back.
        ui.get_mut::<LoopEditor>(EDITOR).unwrap().set_history(None);
        assert_eq!(
            ui.get::<LoopEditor>(EDITOR)
                .unwrap()
                .row_rects(rows_r)
                .len(),
            3
        );
    }

    #[test]
    fn the_trim_handles_keep_a_range_by_sixteenths() {
        let mut e = editor(1);
        e.trim_ticks = Some(16 * 240);
        let mut ui = page_of(EDITOR, e);
        let r = ui.rect_of(EDITOR).unwrap();
        let w = LoopEditor::parts(r).1.inset_xy(10, 6);
        let (start, end) = LoopEditor::trim_grips(w);
        // The start dragged a quarter in: four 1/16 notes.
        let (x, y) = (start.x + 6, start.y + 4);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Move, w.x + w.w / 4, y);
        ui.paint(&mut SoftCanvas::new(W, H));
        let up = ui.touch(1, Touch::Up, w.x + w.w / 4, y).map(|e| e.kind);
        assert_eq!(up, Some(EventKind::Value(0.0)));
        assert_eq!(take_actions(&mut ui), [LoopAct::Trim(960, 3840)]);
        // The end can't pass the start: a 1/16 note stays.
        let (x, y) = (end.x + 6, end.y + 4);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Move, w.x - 50, y);
        ui.touch(1, Touch::Up, w.x - 50, y);
        assert_eq!(take_actions(&mut ui), [LoopAct::Trim(0, 240)]);
        // Let go where it started: nothing to trim.
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
        assert!(take_actions(&mut ui).is_empty());
        // While the loop can't be edited, the ends don't answer.
        ui.get_mut::<LoopEditor>(EDITOR).unwrap().trim_ticks = None;
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Move, w.x, y);
        ui.touch(1, Touch::Up, w.x, y);
        assert!(take_actions(&mut ui).is_empty());
    }

    #[test]
    fn the_picked_layer_stays_in_view() {
        let mut e = editor(9);
        e.picked = 7;
        let r = Rect::new(0, 0, W, H);
        e.follow_in(r);
        let shown: Vec<usize> = e
            .row_rects(LoopEditor::parts(r).2)
            .iter()
            .map(|(k, _)| *k)
            .collect();
        assert!(shown.contains(&7), "{shown:?}");
        e.picked = 0;
        e.follow_in(r);
        assert_eq!(e.row_rects(LoopEditor::parts(r).2)[0].0, 0);
    }

    #[test]
    fn the_list_picks_a_loop_or_a_new_one() {
        let row = |i: usize| LoopRow {
            name: format!("Loop {}", i + 1),
            detail: "empty".into(),
            state: None,
            colour: LOOP_COLOURS[i],
            speaker: (i == 0).then_some(false),
        };
        let list = LoopList {
            rows: (0..4).map(row).collect(),
            new_loop: true,
            clear: true,
            ..LoopList::default()
        };
        let mut ui = page_of(LOOPS, list);
        let r = ui.rect_of(LOOPS).unwrap();
        let parts = ui.get::<LoopList>(LOOPS).unwrap().parts(r);
        let rows = parts.rows;
        assert_eq!(
            tap(&mut ui, rows[1].x + 10, rows[1].y + 10),
            Some(EventKind::Select(1))
        );
        let new = parts.new.expect("an empty loop is left");
        tap(&mut ui, new.x + 10, new.y + 10);
        // Loop 1's speaker mutes it without picking it; loop 2 has none.
        let (sx, sy) = LoopList::speaker_at(rows[0]);
        tap(&mut ui, sx, sy);
        let (sx, sy) = LoopList::speaker_at(rows[1]);
        tap(&mut ui, sx, sy);
        let clear = parts.clear.expect("the picked loop has sound");
        assert!(clear.y >= new.bottom());
        tap(&mut ui, clear.x + 10, clear.y + 10);
        assert_eq!(
            take_actions(&mut ui),
            [
                LoopAct::Pick(1),
                LoopAct::NewLoop,
                LoopAct::Mute(0),
                LoopAct::Pick(1),
                LoopAct::Clear
            ]
        );
    }

    #[test]
    fn a_setting_steps_by_the_half_tapped_and_use_waits_for_its_tools() {
        let row = |label: &str, steps: bool| SettingRow {
            label: label.into(),
            value: "x".into(),
            tone: Tone::Normal,
            steps,
        };
        let settings = LoopSettings {
            rows: vec![
                row("First pass", true),
                row("Start", false),
                row("Line input", true),
            ],
            ..LoopSettings::default()
        };
        let mut ui = page_of(RECORDING, settings);
        let r = ui.rect_of(RECORDING).unwrap();
        let rects = ui.get::<LoopSettings>(RECORDING).unwrap().row_rects(r);
        // Two columns: two rows on the left, one on the right.
        assert_eq!(rects[0].x, rects[1].x);
        assert!(rects[2].x > rects[0].right());
        tap(&mut ui, rects[0].right() - 5, rects[0].y + 5);
        tap(&mut ui, rects[0].x + 5, rects[0].y + 5);
        assert_eq!(tap(&mut ui, rects[1].right() - 5, rects[1].y + 5), None);
        tap(&mut ui, rects[2].right() - 5, rects[2].y + 5);
        assert_eq!(
            take_actions(&mut ui),
            [
                LoopAct::Setting(0, 1),
                LoopAct::Setting(0, -1),
                LoopAct::Setting(2, 1)
            ]
        );
        let use_loop = UseLoop {
            rows: vec![
                ("To a pad".into(), String::new(), false),
                ("Save loop".into(), String::new(), true),
            ],
            ..UseLoop::default()
        };
        let mut ui = page_of(USE, use_loop);
        let r = ui.rect_of(USE).unwrap();
        let rects = ui.get::<UseLoop>(USE).unwrap().row_rects(r);
        tap(&mut ui, rects[0].x + 5, rects[0].y + 5);
        assert_eq!(take_actions(&mut ui), [LoopAct::FaintUse(0)]);
        tap(&mut ui, rects[1].x + 5, rects[1].y + 5);
        assert_eq!(take_actions(&mut ui), [LoopAct::Use(1)]);
    }
}
