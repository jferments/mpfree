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
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Tone, Touch, Ui, Widget};
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
    /// Show layer `k` (play order) on the Q-Links.
    Layer(usize),
    /// A control of layer `k` (play order).
    LayerCmd(usize, LayerCommand),
    /// Row `k` of the Recording and input panel, stepped by `delta`.
    Setting(usize, i32),
    /// One of the editor's tools ([`LoopEditor::tools`]).
    Tool(usize),
    /// One of Use the loop's rows.
    Use(usize),
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
}

/// The loops, the picked one outlined, a button for a new one, and the
/// memory they share. A tap on a loop picks it.
#[derive(Default)]
pub struct LoopList {
    pub rows: Vec<LoopRow>,
    pub picked: usize,
    /// "+ New loop" shows while an empty loop is left.
    pub new_loop: bool,
    /// "16 s of 60 s" and how full, 0 to 1.
    pub memory: (String, f32),
    acts: Vec<LoopAct>,
}

impl LoopList {
    /// Each loop's card, the New loop button and the memory gauge's area.
    pub(crate) fn parts(&self, r: Rect) -> (Vec<Rect>, Option<Rect>, Rect) {
        let (_, body) = card_head(r);
        let (gauge, body) = body.take_bottom(44);
        let n = self.rows.len().max(1) as i32;
        let button = 42;
        let gap = 8;
        let room = body.h - if self.new_loop { button + gap } else { 0 };
        let h = ((room - (n - 1) * gap) / n).clamp(30, 62);
        let rows: Vec<Rect> = (0..self.rows.len() as i32)
            .map(|i| Rect::new(body.x, body.y + i * (h + gap), body.w, h))
            .collect();
        let below = rows.last().map_or(body.y, |l| l.bottom() + gap);
        let new = self
            .new_loop
            .then(|| Rect::new(body.x, below, body.w, button));
        (rows, new, gauge)
    }
}

impl Widget for LoopList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, "Loops", "");
        let (rows, new, gauge) = self.parts(r);
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

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let (rows, new, _) = self.parts(r);
        if let Some(i) = rows.iter().position(|rr| rr.contains(x, y)) {
            self.acts.push(LoopAct::Pick(i));
            return Some(EventKind::Select(i));
        }
        if new.is_some_and(|b| b.contains(x, y)) {
            self.acts.push(LoopAct::NewLoop);
            return Some(EventKind::Tab(0));
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

/// The picked loop: its tools, its wave with the bar lines and the
/// playhead, and a row per layer with its speaker, solo, level, wave and
/// delete. A tap on a row shows that layer on the Q-Links.
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
        for (tr, (label, on)) in self.tool_rects(tool_row).iter().zip(&self.tools) {
            cx.canvas.fill_round_rect(*tr, 8, t.raised);
            cx.text(
                FontId::Bold,
                13.0,
                *tr,
                Align::Center,
                if *on { t.text } else { t.text_faint },
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
                    return None;
                }
                let (k, rr) = self
                    .row_rects(rows)
                    .into_iter()
                    .find(|(_, rr)| rr.contains(x, y))?;
                let l = self.layers[k].clone();
                let p = LoopEditor::row_parts(rr);
                let act = if speaker_hit(p.speaker.0, p.speaker.1).contains(x, y) {
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
                None
            }
        }
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
/// cannot work yet are drawn faint and do nothing.
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
                if *on { t.text } else { t.text_faint },
                title,
            );
            cx.text(FontId::Sans, 12.0, inner, Align::Right, t.text_faint, what);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = self.row_rects(r).iter().position(|rr| rr.contains(x, y))?;
        if !self.rows[i].2 {
            return None;
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

    #[test]
    fn only_working_tools_answer_a_tap() {
        let mut ui = page_of(EDITOR, editor(1));
        let r = ui.rect_of(EDITOR).unwrap();
        let tools = ui
            .get::<LoopEditor>(EDITOR)
            .unwrap()
            .tool_rects(LoopEditor::parts(r).0);
        assert_eq!(tools.len(), TOOLS.len(), "every tool fits");
        assert_eq!(tap(&mut ui, tools[0].x + 4, tools[0].y + 4), None);
        assert_eq!(
            tap(&mut ui, tools[2].x + 4, tools[2].y + 4),
            Some(EventKind::Tab(2))
        );
        assert_eq!(take_actions(&mut ui), [LoopAct::Tool(2)]);
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
        };
        let list = LoopList {
            rows: (0..4).map(row).collect(),
            new_loop: true,
            ..LoopList::default()
        };
        let mut ui = page_of(LOOPS, list);
        let r = ui.rect_of(LOOPS).unwrap();
        let (rows, new, _) = ui.get::<LoopList>(LOOPS).unwrap().parts(r);
        assert_eq!(
            tap(&mut ui, rows[1].x + 10, rows[1].y + 10),
            Some(EventKind::Select(1))
        );
        let new = new.expect("an empty loop is left");
        tap(&mut ui, new.x + 10, new.y + 10);
        assert_eq!(take_actions(&mut ui), [LoopAct::Pick(1), LoopAct::NewLoop]);
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
        assert_eq!(tap(&mut ui, rects[0].x + 5, rects[0].y + 5), None);
        tap(&mut ui, rects[1].x + 5, rects[1].y + 5);
        assert_eq!(take_actions(&mut ui), [LoopAct::Use(1)]);
    }
}
