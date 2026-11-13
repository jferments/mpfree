//! The Looper tab (mockup J): the loops and the memory they share, the
//! picked loop's editor with a row per layer, how the looper records and
//! what to do with the loop, with the picked layer on the Q-Links. And the
//! Play page's loop strip (mockups F and H), which shows the same loop.
//!
//! The panels are [`crate::looper_tab`]'s; this fills them from the
//! looper's view. REC, OVERDUB and UNDO on the MPC work the looper while
//! this tab shows (`App::looper_tap`), as do the strip's buttons on Play.

use crate::app::{Item, Settings};
use crate::looper_tab::{
    level_frac, level_text, HistoryList, LayerRow, LoopEditor, LoopList, LoopRow, LoopSettings,
    SettingRow, UseLoop, EDITOR, LOOPS, LOOP_COLOURS, QLINKS, RECORDING, TOOLS, USE,
};
use crate::looping::{LoopSource, LoopWave, LooperView};
use mpfree_looper::{
    After, Command, Edit, History, LayerCommand, LayerInfo, Phase, Status, Step, MAX_LAYERS,
    MAX_LOOPS, MAX_SEAM,
};
use mpfree_ui::draw;
use mpfree_ui::qlink::{QLinkCard, QLinkColumn};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Tone, Touch, Ui, Widget};
use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const LOOPER_PAGE: &str = "Looper";
/// Mockup J: the loops on the left, the editor over the recording set-up
/// and Use the loop, the Q-Links on the right.
pub const LOOPER_LAYOUT: &str = "col(header:60, row(lloops:220, col(leditor:*, \
     row(lrecording:*, luse:330):206):*, lqlinks:168):*, hint:26)";

/// The looper's presses, as `App::looper_tap` takes them.
pub const B_RECORD: usize = 0;
pub const B_OVERDUB: usize = 1;
pub const B_UNDO: usize = 2;
pub const B_REDO: usize = 3;
pub const B_CLEAR: usize = 4;
pub const B_MUTE: usize = 5;
pub const B_MONITOR: usize = 6;

/// The Play page's looper panel (the All and Loop views), as mockups F and
/// H draw it: "LOOPER" with a chip per layer, the length on the right, and
/// under them the loop with each layer's peaks stacked in its colour (full
/// behind the playhead, dimmer ahead), the bar lines and numbers, the
/// playhead and the state in a pill. When the panel is tall (the Loop
/// view) a row of the looper's buttons sits under the loop; a tap on one
/// gives `Tab(i)` for [`STRIP_BUTTONS`]`[i]`. The Looper tab is where the
/// loop is edited; this is where it is played.
pub struct LoopStrip {
    pub wave: Option<Arc<LoopWave>>,
    /// The heading's right-hand text: "4 bars · 3 layers".
    pub right: String,
    /// The same with the input, for the Loop view's wider heading:
    /// "4 bars · input: pads and synth · 3 layers".
    pub right_wide: String,
    /// The layers' names, layer 1 first ("Drums", "Bass").
    pub names: Vec<String>,
    /// Each layer's speaker: muted (crossed out), layer 1 first.
    pub muted: Vec<bool>,
    /// Each layer heard (not muted, and not left out by another's solo);
    /// the others are drawn grey.
    pub heard: Vec<bool>,
    /// The pill: "PLAYING", "OVERDUB"; None while empty.
    pub state: Option<(String, Tone)>,
    /// Shown in the loop's place while it is empty.
    pub note: String,
    /// The playhead, 0 to 1 along the loop.
    pub position: Option<f32>,
    /// Bars in the loop, for the bar lines.
    pub bars: usize,
    pub layers: usize,
    /// The buttons' labels and whether each is lit (the Loop view).
    pub buttons: Vec<(String, bool)>,
    /// The playhead's x and everything else as last drawn, so a playhead
    /// move redraws only the columns it passed.
    drawn: RefCell<Option<(Rect, StripLook, i32)>>,
    /// Where each layer's speaker was drawn, layer 1 first, for a tap.
    speakers: RefCell<Vec<Rect>>,
}

/// What the strip shows apart from the playhead.
#[derive(Clone, Debug, PartialEq)]
struct StripLook {
    right: String,
    right_wide: String,
    names: Vec<String>,
    muted: Vec<bool>,
    heard: Vec<bool>,
    state: Option<(String, Tone)>,
    note: String,
    bars: usize,
    layers: usize,
    buttons: Vec<(String, bool)>,
    wave: Option<u64>,
}

/// The strip's widget name.
pub const LOOP_STRIP: &str = "play-loop";
/// The buttons under the loop in the Loop view, in tap order. "Play" and
/// "Stop" are the transport's; the others are the Looper tab's presses.
pub const STRIP_BUTTONS: [&str; 8] = [
    "Record", "Overdub", "Play", "Stop", "Undo", "Redo", "Mute", "Clear",
];
/// What a tap on one of [`STRIP_BUTTONS`] does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripPress {
    /// One of the looper's presses (`B_RECORD`…), as `App::looper_tap` takes them.
    Looper(usize),
    /// The transport's PLAY.
    Play,
    /// The transport's STOP.
    Stop,
}
/// Each of [`STRIP_BUTTONS`]' job.
pub const STRIP_PRESSES: [StripPress; 8] = [
    StripPress::Looper(B_RECORD),
    StripPress::Looper(B_OVERDUB),
    StripPress::Play,
    StripPress::Stop,
    StripPress::Looper(B_UNDO),
    StripPress::Looper(B_REDO),
    StripPress::Looper(B_MUTE),
    StripPress::Looper(B_CLEAR),
];
/// Each of [`STRIP_BUTTONS`]' symbol. The Record button keeps its dot
/// whatever it says (Cancel, End), since it is still the record button.
pub const STRIP_SYMBOLS: [Option<draw::Symbol>; 8] = [
    Some(draw::Symbol::Record),
    Some(draw::Symbol::Overdub),
    Some(draw::Symbol::Play),
    Some(draw::Symbol::Stop),
    None,
    None,
    None,
    None,
];
/// The layers' colours, layer 1 first.
pub const LAYER_COLOURS: [Color; 8] = [
    draw::KNOB[0],
    draw::KNOB[1],
    draw::CHOP,
    draw::KNOB[2],
    draw::VOX,
    draw::PERC,
    draw::KICK,
    draw::FX,
];
/// Panels at least this tall show the buttons.
const BUTTONS_FROM_H: i32 = 300;

impl Default for LoopStrip {
    fn default() -> Self {
        LoopStrip {
            wave: None,
            right: String::new(),
            right_wide: String::new(),
            names: Vec::new(),
            muted: Vec::new(),
            heard: Vec::new(),
            state: None,
            note: String::new(),
            position: None,
            bars: 0,
            layers: 0,
            buttons: STRIP_BUTTONS
                .iter()
                .map(|b| (b.to_string(), false))
                .collect(),
            drawn: RefCell::new(None),
            speakers: RefCell::new(Vec::new()),
        }
    }
}

impl LoopStrip {
    /// The loop's panel and the buttons' row in the strip's rectangle.
    fn parts(r: Rect) -> (Rect, Option<Rect>) {
        let (_, body) = draw::card_head(r);
        if r.h >= BUTTONS_FROM_H {
            let (btns, wave) = body.take_bottom(44);
            (wave.take_bottom(8).1, Some(btns))
        } else {
            (body, None)
        }
    }

    /// The loop's drawing area inside its panel.
    fn wave_area(r: Rect) -> Rect {
        LoopStrip::parts(r).0.inset_xy(10, 8)
    }

    fn button_rects(&self, row: Rect) -> Vec<Rect> {
        let n = self.buttons.len().max(1) as i32;
        let gap = 8;
        let bw = (row.w - gap * (n - 1)) / n;
        (0..n)
            .map(|i| Rect::new(row.x + i * (bw + gap), row.y, bw, row.h))
            .collect()
    }

    fn look(&self) -> StripLook {
        StripLook {
            right: self.right.clone(),
            right_wide: self.right_wide.clone(),
            names: self.names.clone(),
            muted: self.muted.clone(),
            heard: self.heard.clone(),
            state: self.state.clone(),
            note: self.note.clone(),
            bars: self.bars,
            layers: self.layers,
            buttons: self.buttons.clone(),
            wave: self.wave.as_ref().map(|w| Arc::as_ptr(w) as usize as u64),
        }
    }

    /// Layer `k` is heard (as far as the strip knows).
    fn is_heard(&self, k: usize) -> bool {
        self.heard.get(k).copied().unwrap_or(true)
    }

    fn play_x(&self, w: Rect) -> Option<i32> {
        self.position
            .filter(|_| self.wave.as_ref().is_some_and(|w| w.frames > 0))
            .map(|p| w.x + (p.clamp(0.0, 1.0) * w.w as f32) as i32)
    }
}

impl Widget for LoopStrip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let (panel, btns) = LoopStrip::parts(r);
        let right = if btns.is_some() {
            &self.right_wide
        } else {
            &self.right
        };
        draw::card(cx, r, "Looper", right);
        cx.canvas
            .fill_round_rect(panel, 10, t.bg.mix(t.surface, 0.4));
        let w = LoopStrip::wave_area(r);
        let play = self.play_x(w);
        match self.wave.as_ref().filter(|wv| wv.frames > 0) {
            Some(wave) => {
                crate::looper_tab::paint_layers(cx, w, wave, play, &self.heard);
                // Bar lines and numbers.
                let bars = self.bars.max(1);
                for b in 0..bars {
                    let bx = w.x + (b as f32 / bars as f32 * w.w as f32) as i32;
                    if b > 0 {
                        cx.canvas.fill_rect(Rect::new(bx, w.y, 1, w.h), t.outline);
                    }
                    cx.text(
                        FontId::Bold,
                        11.0,
                        Rect::new(bx + 5, w.y, 24, 14),
                        Align::Left,
                        t.text_faint,
                        &format!("{}", b + 1),
                    );
                }
                if let Some(px) = play {
                    cx.canvas
                        .fill_rect(Rect::new(px - 1, panel.y + 2, 2, panel.h - 4), t.text);
                }
            }
            None => {
                cx.text(
                    FontId::Sans,
                    14.0,
                    panel,
                    Align::Center,
                    t.text_dim,
                    &self.note,
                );
            }
        }
        // The state, top right of the loop.
        if let Some((state, tone)) = &self.state {
            let sw = draw::pill_width(cx, state, true);
            let c = match tone {
                Tone::Dim | Tone::Normal => t.text_faint,
                k => t.tone(*k),
            };
            draw::pill(cx, panel.right() - sw - 10, panel.y + 8, 26, state, c, true);
        }
        // The layers beside the title, each with its speaker, as many as
        // fit before the heading's right-hand text.
        let (head, _) = draw::card_head(r);
        let right_w = if right.is_empty() {
            0
        } else {
            cx.measure(FontId::Sans, draw::HEAD_PX, right) + 20
        };
        let end = head.right() - right_w;
        let mut lx = head.x + cx.measure(FontId::Bold, draw::HEAD_PX, "LOOPER") + 24;
        let cy = head.y + head.h / 2;
        let mut speakers = Vec::new();
        for li in 0..self.layers {
            let name = self
                .names
                .get(li)
                .cloned()
                .unwrap_or_else(|| format!("Layer {}", li + 1));
            let tw = cx.measure(FontId::Sans, draw::HEAD_PX, &name);
            // The dot, the name, a gap, the speaker.
            let sx = lx + 12 + tw + 18;
            let more = self.layers - li - 1;
            let room = if more > 0 { 36 } else { 0 };
            if sx + 12 + room > end {
                cx.text(
                    FontId::Sans,
                    draw::HEAD_PX,
                    Rect::new(lx, head.y, 60, head.h),
                    Align::Left,
                    t.text_faint,
                    &format!("+{}", self.layers - li),
                );
                break;
            }
            let heard = self.is_heard(li);
            let colour = LAYER_COLOURS[li % LAYER_COLOURS.len()];
            draw::dot(cx, lx + 4, cy, 4, if heard { colour } else { t.text_faint });
            cx.text(
                FontId::Sans,
                draw::HEAD_PX,
                Rect::new(lx + 12, head.y, tw + 4, head.h),
                Align::Left,
                if heard { t.text_dim } else { t.text_faint },
                &name,
            );
            let muted = self.muted.get(li).copied().unwrap_or(false);
            draw::speaker(cx, sx, cy, t.text_dim, muted);
            speakers.push(draw::speaker_hit(sx, cy));
            lx = sx + 12 + 18;
        }
        *self.speakers.borrow_mut() = speakers;
        if let Some(row) = btns {
            for (i, (br, (label, on))) in
                self.button_rects(row).iter().zip(&self.buttons).enumerate()
            {
                let lit = if label == "Overdub" || label == "Stop dub" {
                    t.warn
                } else if label == "Mute" || label == "Unmute" {
                    t.text_dim
                } else if label == "Play" {
                    t.good
                } else {
                    t.bad
                };
                cx.canvas
                    .fill_round_rect(*br, 10, if *on { lit } else { t.raised });
                let ink = if *on {
                    t.bg
                } else if label == "Clear" {
                    t.bad
                } else {
                    t.text
                };
                let sym = STRIP_SYMBOLS.get(i).copied().flatten();
                let sym_ink = match sym {
                    Some(s) if !*on => s.colour(&t),
                    _ => ink,
                };
                draw::symbol_label(cx, *br, sym, FontId::Bold, 14.0, ink, sym_ink, label);
            }
        }
        *self.drawn.borrow_mut() = Some((r, self.look(), play.unwrap_or(-1)));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, look, old_x) = drawn.as_ref()?;
        if *at != r || *look != self.look() {
            return None;
        }
        let w = LoopStrip::wave_area(r);
        let (panel, _) = LoopStrip::parts(r);
        let new_x = self.play_x(w).unwrap_or(-1);
        if new_x == *old_x {
            return Some(Vec::new());
        }
        if *old_x < 0 || new_x < 0 || new_x < *old_x {
            // Round the loop's end: the columns behind the old place
            // turn dim again.
            return Some(vec![panel]);
        }
        Some(vec![Rect::new(
            old_x - 4,
            panel.y,
            new_x - old_x + 8,
            panel.h,
        )])
    }

    fn targets(&self, r: Rect) -> Vec<mpfree_ui::Target> {
        use mpfree_ui::Target;
        let mut out: Vec<Target> = self
            .speakers
            .borrow()
            .iter()
            .enumerate()
            .map(|(k, h)| {
                let name = self.names.get(k).cloned().unwrap_or_default();
                Target::tap("layer-mute", k, format!("{name} speaker"), *h)
            })
            .collect();
        if let Some(row) = LoopStrip::parts(r).1 {
            for (i, b) in self.button_rects(row).into_iter().enumerate() {
                let label = self.buttons.get(i).map_or(String::new(), |b| b.0.clone());
                out.push(Target::tap("button", i, label, b));
            }
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if let Some(k) = self.speakers.borrow().iter().position(|h| h.contains(x, y)) {
            return Some(EventKind::Toggle(k));
        }
        let row = LoopStrip::parts(r).1?;
        self.button_rects(row)
            .iter()
            .position(|b| b.contains(x, y))
            .map(EventKind::Tab)
    }
}

/// The Recording and input panel's rows, left column then right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetRow {
    RecordFrom,
    FirstPass,
    Start,
    After,
    InputPeak,
    LineInput,
    Monitor,
    Latency,
    Tempo,
}

/// The rows in order: the first five in the left column, the line
/// input's four in the right.
pub const SET_ROWS: [SetRow; 9] = [
    SetRow::RecordFrom,
    SetRow::FirstPass,
    SetRow::Start,
    SetRow::After,
    SetRow::Tempo,
    SetRow::LineInput,
    SetRow::InputPeak,
    SetRow::Monitor,
    SetRow::Latency,
];

/// The setting row `i` steps, and by how much for a tap at `delta`
/// (on and off rows and the source flip whichever half is tapped).
pub fn setting_item(i: usize, delta: i32, set: &Settings) -> Option<(Item, i32)> {
    let flip = |on: bool| if on { -1 } else { 1 };
    Some(match SET_ROWS.get(i)? {
        SetRow::RecordFrom => (Item::LoopSource, flip(set.loop_source == LoopSource::Line)),
        SetRow::FirstPass => (Item::LoopBars, delta),
        SetRow::LineInput => (Item::LineInput, flip(set.line_input)),
        SetRow::Monitor => (Item::LoopMonitor, flip(set.loop_monitor)),
        SetRow::Latency => (Item::LoopLatency, delta),
        _ => return None,
    })
}

/// The setting row `i` that is the picked loop's own (After recording):
/// the looper command a tap asks for. None for the other rows.
pub fn setting_command(i: usize, s: &Status) -> Option<Command> {
    match SET_ROWS.get(i)? {
        SetRow::After => Some(Command::SetAfter(match s.after {
            After::Play => After::Overdub,
            After::Overdub => After::Play,
        })),
        _ => None,
    }
}

/// Use the loop's rows: title and what it does.
const USE_ROWS: [(&str, &str); 4] = [
    ("To a pad", "a sample on the Sample tab"),
    ("To a clip", "audio on the Arrange page"),
    ("Save loop", "on the stick, with its layers"),
    ("Resample", "the loops as heard, into a new loop"),
];

/// Use the loop's Resample row.
pub const USE_RESAMPLE: usize = 3;

/// The seam fades the tool steps through, in milliseconds (0: a straight
/// cut); the last is the looper's longest ([`MAX_SEAM`]).
const SEAM_STEPS_MS: [f32; 4] = [0.0, 10.0, 50.0, 1000.0];

/// A seam fade in frames at `rate`, up to the looper's longest.
fn seam_frames(ms: f32, rate: u32) -> u32 {
    ((ms / 1000.0 * rate as f32).round() as u32).min(MAX_SEAM as u32)
}

/// A seam fade as text: "10 ms", "cut".
/// The editor's Undo history tool: it opens the history in the layers'
/// place.
pub const TOOL_HISTORY: usize = 7;

/// The trim handles' range, as a command while the loop can be edited.
pub fn trim_command(from: i64, to: i64, s: &Status) -> Option<Command> {
    (editable(s) && 0 <= from && from < to && to <= s.ticks)
        .then_some(Command::Edit(Edit::Trim { from, to }))
}

/// An edit as the undo history names it.
fn edit_text(e: Edit, rate: u32) -> String {
    let sixteenths = |n: i32| {
        if n.abs() == 1 {
            "1/16".to_string()
        } else {
            format!("{} × 1/16", n.abs())
        }
    };
    match e {
        Edit::Half => "½ length".to_string(),
        Edit::Double => "× 2".to_string(),
        Edit::Reverse => "Reverse".to_string(),
        Edit::Nudge(n) if n < 0 => format!("Nudge ← {}", sixteenths(n)),
        Edit::Nudge(n) => format!("Nudge {} →", sixteenths(n)),
        Edit::Trim { .. } => "Trim".to_string(),
        Edit::SeamFade(f) => format!("Seam fade {}", seam_text(f as usize, rate)),
    }
}

/// A step as the undo history names it: layers by their place in the
/// loop now ("Layer 3"), when they are in it.
fn step_text(s: Step, layers: &[Option<LayerInfo>], rate: u32) -> String {
    let name = |id: u8| {
        layers
            .iter()
            .position(|l| l.is_some_and(|l| l.id == id))
            .map(|k| format!("Layer {}", k + 1))
    };
    match s {
        Step::Overdub(id) => name(id).map_or("Overdub".to_string(), |n| format!("Overdub · {n}")),
        Step::Delete(id) => {
            name(id).map_or("Delete a layer".to_string(), |n| format!("Delete · {n}"))
        }
        Step::Edit(e) => edit_text(e, rate),
        Step::Merge(_) => "Merge layers".to_string(),
        // Overdub or Record over a print fits the loop to the tempo.
        Step::Fit(_) => "Fit to the tempo".to_string(),
    }
}

/// The undo history as the editor lists it: Start (the first pass), the
/// steps Undo takes back, oldest first, then the ones Redo brings back.
pub fn history_list(h: &History, layers: &[Option<LayerInfo>], rate: u32) -> HistoryList {
    let mut rows = vec!["Start: the first pass".to_string()];
    rows.extend(h.done().iter().map(|&s| step_text(s, layers, rate)));
    let now = rows.len() - 1;
    rows.extend(h.redo().iter().map(|&s| step_text(s, layers, rate)));
    HistoryList { rows, now }
}

pub fn seam_text(frames: usize, rate: u32) -> String {
    if frames == 0 {
        "cut".to_string()
    } else {
        format!("{:.0} ms", frames as f64 * 1000.0 / f64::from(rate.max(1)))
    }
}

/// Whether the picked loop takes edits now: it plays (not recording or
/// overdubbing) and no merge is rendering.
fn editable(s: &Status) -> bool {
    s.phase == Phase::Playing && s.merging.is_none()
}

/// What the editor's tool `i` ([`TOOLS`]) asks of the picked loop; None
/// while it can't be used (the loop isn't playing, or the tool is not
/// built yet).
pub fn tool_command(i: usize, s: &Status, rate: u32) -> Option<Command> {
    if !editable(s) {
        return None;
    }
    Some(match i {
        0 => Command::Edit(Edit::Half),
        1 => Command::Edit(Edit::Double),
        2 => Command::Edit(Edit::Reverse),
        3 => Command::Edit(Edit::Nudge(-1)),
        4 => Command::Edit(Edit::Nudge(1)),
        5 => {
            // The next step up from the fade now, then back to a cut.
            let next = SEAM_STEPS_MS
                .iter()
                .map(|&ms| seam_frames(ms, rate))
                .find(|&f| f as usize > s.seam)
                .unwrap_or(0);
            Command::Edit(Edit::SeamFade(next))
        }
        6 if s.layers > 1 => Command::Merge,
        _ => return None,
    })
}

/// Why the editor's tool `i` can't be used now, for the screen to say in
/// place when its faint button is tapped.
pub fn why_no_tool(i: usize, s: &Status) -> &'static str {
    match s.phase {
        Phase::Empty => "Record a loop first: the tools edit the loop that plays",
        Phase::Armed | Phase::Recording => {
            "The tools work once the first pass is recorded and plays"
        }
        Phase::Overdubbing => "The tools work while the loop plays, not while it overdubs",
        Phase::Playing if s.merging.is_some() => "Wait for the merge to finish",
        Phase::Playing if i == 6 => "Merge needs two layers or more: overdub one first",
        Phase::Playing => "That tool can't change this loop now",
    }
}

/// Why Use the loop's row `i` can't be used now, for the screen to say in
/// place when its faint row is tapped.
pub fn why_no_use(i: usize, v: &LooperView) -> &'static str {
    if i != USE_RESAMPLE {
        return "Not built yet: Resample works now (the bounce to a pad, a clip or a file comes next)";
    }
    if !v.loops.iter().any(|l| l.phase == Phase::Empty) {
        return "Every loop is in use: clear one to resample into it";
    }
    "Resample needs a loop that plays and isn't muted"
}

/// The tools as the editor shows them: each one's name (the seam fade's
/// with its length, the merge's progress) and whether it works now.
pub fn tools(s: &Status, rate: u32) -> Vec<(String, bool)> {
    TOOLS
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let name = match i {
                5 => format!("Seam fade {}", seam_text(s.seam, rate)),
                6 if s.merging.is_some() => {
                    format!("Merging {:.0} %", 100.0 * s.merging.unwrap_or(0.0))
                }
                _ => t.to_string(),
            };
            (name, tool_command(i, s, rate).is_some())
        })
        .collect()
}

/// Fade old as a Q-Link moves it: 50 % to 100 % kept a pass, in steps of
/// 1 % (SHIFT: 0.2 %).
const FADE_OLD_MIN: f32 = 0.5;

/// How far round Fade old's ring is lit, 0 to 1.
fn fade_old_pos(k: f32) -> f32 {
    ((k - FADE_OLD_MIN) / (1.0 - FADE_OLD_MIN)).clamp(0.0, 1.0)
}

/// A Q-Link turn on the Looper tab: what it asks of the looper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopTurn {
    /// The picked layer's level or pan, by play order.
    Layer(usize, LayerCommand),
    /// Q-Link 3: the picked loop's Fade old, 0.5 to 1 kept a pass.
    FadeOld(f32),
    /// Q-Link 4: the picked loop's seam fade, in frames.
    Seam(u32),
}

/// A take being recorded: its loop, its layer, and how many hits of each
/// group of sounds went into it.
type Take = (usize, usize, Vec<(&'static str, u32)>);

pub struct LooperPage {
    /// The input's peak, held for about 2 s of updates.
    hold: f32,
    hold_age: u32,
    /// Each loop's layers' names, by what was played into each ("Drums",
    /// "Bass"); kept past an undo, so a redo finds its name again.
    names: Vec<Vec<String>>,
    /// The take recording now: its loop and layer, and how many hits of
    /// each group of sounds went into it.
    take: Option<Take>,
    /// The picked loop's layers' ids and mix as of the last view.
    layers: [Option<LayerInfo>; MAX_LAYERS],
    /// The loop the controls act on.
    selected: usize,
    /// The layer on the Q-Links, play order.
    layer: usize,
    /// The last view, for the Q-Links.
    view: LooperView,
    /// A Q-Link was turned: the cards show what it set, not the view, for
    /// a moment (the looper takes a period or two to answer).
    turned: Option<Instant>,
    /// The undo history is open in the editor.
    history_open: bool,
}

impl LooperPage {
    pub fn add(ui: &mut Ui) -> Result<LooperPage, String> {
        ui.add(LOOPS, LoopList::default());
        ui.add(EDITOR, LoopEditor::default());
        ui.add(RECORDING, LoopSettings::default());
        ui.add(USE, UseLoop::default());
        ui.add(QLINKS, QLinkColumn::new());
        ui.add(LOOP_STRIP, LoopStrip::default());
        ui.add_page(LOOPER_PAGE, LOOPER_LAYOUT)?;
        let mut p = LooperPage {
            hold: f32::NEG_INFINITY,
            hold_age: 0,
            names: vec![Vec::new(); MAX_LOOPS],
            take: None,
            layers: [None; MAX_LAYERS],
            selected: 0,
            layer: 0,
            view: LooperView::default(),
            turned: None,
            history_open: false,
        };
        p.update(ui, &LooperView::default(), &Settings::default());
        Ok(p)
    }

    /// A pad played a sound of `group` ("Drums", "Bass", "Synth"): it
    /// names the layer recording now, if one is.
    pub fn hit(&mut self, group: &'static str) {
        if let Some((_, _, counts)) = &mut self.take {
            match counts.iter_mut().find(|(g, _)| *g == group) {
                Some((_, n)) => *n += 1,
                None => counts.push((group, 1)),
            }
        }
    }

    /// The picked loop's layers' names as they are (the previews name
    /// theirs).
    pub fn set_names(&mut self, names: &[&str]) {
        self.names[self.selected] = names.iter().map(|n| n.to_string()).collect();
    }

    /// The name of the picked loop's layer `k`, as the strip shows it.
    pub fn layer_name(&self, k: usize) -> String {
        self.names[self.selected]
            .get(k)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Layer {}", k + 1))
    }

    /// Layer `k`'s id and mix as of the last view (for its controls).
    pub fn layer_info(&self, k: usize) -> Option<LayerInfo> {
        self.layers.get(k).copied().flatten()
    }

    /// The first empty loop, for New loop.
    pub fn first_empty(&self) -> Option<usize> {
        self.view.loops.iter().position(|l| l.phase == Phase::Empty)
    }

    /// The Undo history tool: open the history in the layers' place, or
    /// close it.
    pub fn toggle_history(&mut self, ui: &mut Ui) {
        self.history_open = !self.history_open;
        let v = self.view.clone();
        self.refresh_editor(ui, &v);
    }

    pub fn history_open(&self) -> bool {
        self.history_open
    }

    /// A tap on row `row` of the undo history: how many steps back (below
    /// 0) or forward that is. None for now itself.
    pub fn history_steps(&self, row: usize) -> Option<i32> {
        let now = self.view.history.done().len();
        let rows = now + self.view.history.redo().len();
        (row != now && row <= rows).then(|| row as i32 - now as i32)
    }

    /// A tap on a layer's row: that layer goes on the Q-Links.
    pub fn pick_layer(&mut self, ui: &mut Ui, k: usize) {
        self.layer = k;
        let v = self.view.clone();
        self.refresh_editor(ui, &v);
        self.refresh_qlinks(ui, &v);
    }

    /// Follows the looper's takes: a new one starts counting, one that
    /// ends is named by what was played most into it ("Drums", "Bass 2"),
    /// or "Line in" when the line was the source.
    fn follow_takes(&mut self, v: &LooperView) {
        let s = &v.status;
        let n = v.selected.min(MAX_LOOPS - 1);
        let now = match s.phase {
            Phase::Recording => Some(0),
            Phase::Overdubbing => Some(s.layers.saturating_sub(1)),
            _ => None,
        };
        for (i, l) in v.loops.iter().enumerate() {
            let phase = if i == n { s.phase } else { l.phase };
            if phase == Phase::Empty {
                self.names[i].clear();
            }
        }
        if self.take.as_ref().map(|t| (t.0, t.1)) != now.map(|k| (n, k)) {
            self.take = now.map(|k| (n, k, Vec::new()));
        }
        let Some((n, k, counts)) = &self.take else {
            return;
        };
        let base = match v.source {
            LoopSource::Line => "Line in",
            LoopSource::Pads => match counts.iter().max_by_key(|(_, c)| *c) {
                Some((g, _)) => g,
                None => "",
            },
        };
        let (n, k) = (*n, *k);
        let names = &self.names[n];
        let name = if base.is_empty() {
            String::new()
        } else {
            let same = (0..k)
                .filter(|&i| {
                    names
                        .get(i)
                        .is_some_and(|m| m == base || m.starts_with(&format!("{base} ")))
                })
                .count();
            if same == 0 {
                base.to_string()
            } else {
                format!("{base} {}", same + 1)
            }
        };
        let names = &mut self.names[n];
        if names.len() <= k {
            names.resize(k + 1, String::new());
        }
        names[k] = name;
    }

    /// The new loop waveform (empty when the loop was cleared), on the
    /// Looper tab and on the Play page's strip.
    /// While the first pass records, the wave grows with it and the
    /// playhead follows its end.
    pub fn set_wave(&mut self, ui: &mut Ui, wave: Arc<LoopWave>) {
        let wave = Some(wave).filter(|w| w.frames > 0);
        let live = live_place(&self.view, wave.as_ref());
        if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
            e.wave = wave.clone();
            if let Some((at, bars)) = live {
                (e.position, e.bars) = (Some(at), bars);
            }
        }
        if let Some(s) = ui.get_mut::<LoopStrip>(LOOP_STRIP) {
            s.wave = wave;
            if let Some((at, bars)) = live {
                (s.position, s.bars) = (Some(at), bars);
            }
        }
    }

    /// A Q-Link turned `ticks` detents on the Looper tab (SHIFT: `fine`):
    /// the picked layer's level and pan, the loop's Fade old and its seam
    /// fade (mockup J). The card follows at once.
    pub fn qlink(&mut self, ui: &mut Ui, k: usize, ticks: i32, fine: bool) -> Option<LoopTurn> {
        if ticks == 0 {
            return None;
        }
        let d = ticks as f32 * if fine { 0.1 } else { 0.5 };
        let turn = match k {
            0 | 1 => {
                let info = self.layers.get_mut(self.layer)?.as_mut()?;
                if k == 0 {
                    let db = (20.0 * info.level.max(1e-6).log10()).max(-60.0);
                    let db = (db + d).clamp(-60.0, 6.0);
                    info.level = if db <= -60.0 {
                        0.0
                    } else {
                        10f32.powf(db / 20.0)
                    };
                    LoopTurn::Layer(self.layer, LayerCommand::Level(info.level))
                } else {
                    let step = if fine { 0.01 } else { 0.02 };
                    info.pan = (info.pan + ticks as f32 * step).clamp(-1.0, 1.0);
                    LoopTurn::Layer(self.layer, LayerCommand::Pan(info.pan))
                }
            }
            2 => {
                let s = &mut self.view.status;
                if s.phase == Phase::Empty {
                    return None;
                }
                let step = if fine { 0.002 } else { 0.01 };
                s.fade_old = (s.fade_old + ticks as f32 * step).clamp(FADE_OLD_MIN, 1.0);
                LoopTurn::FadeOld(s.fade_old)
            }
            3 => {
                let rate = self.view.rate;
                let s = &mut self.view.status;
                if !editable(s) {
                    return None;
                }
                // A millisecond a detent (SHIFT: a tenth).
                let step = (rate as f32 / if fine { 10_000.0 } else { 1000.0 }).max(1.0);
                let seam = (s.seam as f32 + ticks as f32 * step).clamp(0.0, MAX_SEAM as f32);
                s.seam = seam.round() as usize;
                LoopTurn::Seam(s.seam as u32)
            }
            _ => return None,
        };
        self.turned = Some(Instant::now());
        if let LoopTurn::Layer(l, _) = turn {
            if let (Some(Some(info)), Some(slot)) =
                (self.layers.get(l), self.view.layers.get_mut(l))
            {
                *slot = Some(*info);
            }
        }
        let v = self.view.clone();
        self.refresh_qlinks(ui, &v);
        self.refresh_editor(ui, &v);
        Some(turn)
    }

    fn refresh_qlinks(&mut self, ui: &mut Ui, v: &LooperView) {
        let n = v.selected;
        let loop_scope = format!("Loop {}", n + 1);
        let layer = self.layers.get(self.layer).copied().flatten();
        let scope = format!("Layer {}", self.layer + 1);
        let (c0, c1) = match layer {
            Some(l) => {
                let mut pan = QLinkCard::new("Pan", &scope, &pan_text(l.pan), (l.pan + 1.0) / 2.0);
                pan.bipolar = true;
                (
                    QLinkCard::new("Level", &scope, &level_text(l.level), level_frac(l.level)),
                    pan,
                )
            }
            None => {
                let mut pan = QLinkCard::new("Pan", "No layer", "-", 0.5);
                pan.bipolar = true;
                (QLinkCard::new("Level", "No layer", "-", 0.0), pan)
            }
        };
        let s = &v.status;
        let c2 = if s.phase == Phase::Empty {
            QLinkCard::new("Fade old", &loop_scope, "-", 0.0)
        } else {
            QLinkCard::new(
                "Fade old",
                &loop_scope,
                &format!("{:.0} %", 100.0 * s.fade_old),
                fade_old_pos(s.fade_old),
            )
        };
        let c3 = if s.phase == Phase::Empty {
            QLinkCard::new("Seam fade", &loop_scope, "-", 0.0)
        } else {
            QLinkCard::new(
                "Seam fade",
                &loop_scope,
                &seam_text(s.seam, v.rate),
                s.seam as f32 / MAX_SEAM as f32,
            )
        };
        if let Some(q) = ui.get_mut::<QLinkColumn>(QLINKS) {
            let cards = [c0, c1, c2, c3];
            if q.cards != cards {
                q.cards = cards;
            }
            q.title = "Looper".to_string();
        }
    }

    fn refresh_editor(&mut self, ui: &mut Ui, v: &LooperView) {
        let s = &v.status;
        let n = v.selected;
        let empty = s.phase == Phase::Empty;
        let title = format!("Loop {} · Editor", n + 1);
        let right = if empty {
            "empty".to_string()
        } else {
            let mut parts = Vec::new();
            let bars = bars_of(s.ticks, v.beats_per_bar);
            if bars > 0 {
                let bpm = 60.0 * f64::from(v.rate) / v.frames_per_beat.max(1.0);
                parts.push(format!(
                    "{bars} bar{} at {bpm:.0} BPM",
                    if bars == 1 { "" } else { "s" }
                ));
            }
            if s.frames > 0 {
                parts.push(format!(
                    "{:.1} s",
                    s.frames as f64 / f64::from(v.rate.max(1))
                ));
            }
            parts.push(format!("input: {}", v.source.name().to_lowercase()));
            parts.join(" · ")
        };
        let names: Vec<String> = (0..s.layers)
            .map(|k| self.names[n].get(k).cloned().unwrap_or_default())
            .collect();
        let layers: Vec<LayerRow> = (0..s.layers)
            .map(|k| {
                let info = self.layers.get(k).copied().flatten();
                let what = if k == 0 { "first pass" } else { "overdub" };
                let mut detail = match names.get(k).filter(|n| !n.is_empty()) {
                    Some(name) => format!("{what} · {}", name.to_lowercase()),
                    None => what.to_string(),
                };
                // Fade old has turned it down under the overdubs since.
                if let Some(l) = info.filter(|l| l.faded < 0.995) {
                    detail.push_str(&format!(" · faded to {:.0} %", 100.0 * l.faded));
                }
                LayerRow {
                    name: format!("Layer {}", k + 1),
                    detail,
                    colour: LAYER_COLOURS[k % LAYER_COLOURS.len()],
                    muted: info.is_some_and(|l| l.muted),
                    solo: info.is_some_and(|l| l.solo),
                    heard: info.is_none_or(|l| l.audible),
                    level: info.map_or(1.0, |l| l.level),
                    recording: info.is_some_and(|l| l.recording),
                }
            })
            .collect();
        let mut tools = tools(s, v.rate);
        let h = &v.history;
        if let Some(t) = tools.get_mut(TOOL_HISTORY) {
            t.1 = self.history_open || !h.done().is_empty() || !h.redo().is_empty();
        }
        let history = self
            .history_open
            .then(|| history_list(h, &self.layers, v.rate));
        let lit = self.history_open.then_some(TOOL_HISTORY);
        let trim_ticks = (editable(s) && s.ticks > 0).then_some(s.ticks);
        let note = empty_note(v);
        let live = live_place(
            v,
            ui.get::<LoopEditor>(EDITOR).and_then(|e| e.wave.as_ref()),
        );
        let position = match live {
            Some((at, _)) => Some(at),
            None => (!empty && s.frames > 0).then_some(s.position),
        };
        let bars = live.map_or_else(|| bars_of(s.ticks, v.beats_per_bar), |(_, b)| b);
        let picked = self.layer.min(s.layers.saturating_sub(1));
        let r = ui.rect_on(LOOPER_PAGE, EDITOR);
        if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
            if e.title != title
                || e.right != right
                || e.tools != tools
                || e.layers != layers
                || e.note != note
                || e.position != position
                || e.bars != bars
                || e.picked != picked
                || e.lit != lit
                || e.history() != history.as_ref()
                || e.trim_ticks != trim_ticks
            {
                e.title = title;
                e.right = right;
                e.tools = tools;
                e.layers = layers;
                e.note = note;
                e.position = position;
                e.bars = bars;
                e.picked = picked;
                e.lit = lit;
                e.set_history(history);
                e.trim_ticks = trim_ticks;
                if let Some(r) = r {
                    e.follow_in(r);
                }
            }
        }
    }

    pub fn update(&mut self, ui: &mut Ui, v: &LooperView, settings: &Settings) {
        let mut v = v.clone();
        v.selected = v.selected.min(MAX_LOOPS - 1);
        v.loops[v.selected] = v.status;
        self.follow_takes(&v);
        if self.selected != v.selected {
            self.layer = 0;
            self.history_open = false;
        }
        self.selected = v.selected;
        // A Q-Link turned a moment ago: keep what it set until the looper
        // has caught up.
        let fresh = self
            .turned
            .is_some_and(|t| t.elapsed() < Duration::from_millis(400));
        if fresh {
            v.layers = self.view.layers;
            v.status.level = self.view.status.level;
            v.status.fade_old = self.view.status.fade_old;
            v.status.seam = self.view.status.seam;
        }
        self.layers = v.layers;
        self.layer = self.layer.min(v.status.layers.saturating_sub(1));
        let v = &v;
        // The input's peak, held for about 2 s of updates.
        self.hold_age += 1;
        let peak = v.meter_db[0].max(v.meter_db[1]);
        if self.hold_age > 8 || peak > self.hold {
            self.hold = peak;
            self.hold_age = 0;
        }
        // The loops.
        let last = v
            .loops
            .iter()
            .rposition(|l| l.phase != Phase::Empty)
            .map_or(0, |i| i + 1);
        let shown = last.max(v.selected + 1).clamp(4, MAX_LOOPS);
        let rows: Vec<LoopRow> = (0..shown)
            .map(|i| {
                let l = &v.loops[i];
                LoopRow {
                    name: format!("Loop {}", i + 1),
                    detail: loop_detail(l, v.beats_per_bar),
                    state: state_of(l).map(|(a, b)| (a.to_string(), b)),
                    colour: LOOP_COLOURS[i % LOOP_COLOURS.len()],
                    speaker: (l.phase != Phase::Empty).then_some(l.muted),
                }
            })
            .collect();
        let rate = f64::from(v.rate.max(1));
        let memory = (
            format!(
                "{} of {}",
                memory_time(v.memory.used as f64 / rate, false),
                memory_time(v.memory.capacity as f64 / rate, true)
            ),
            if v.memory.capacity == 0 {
                0.0
            } else {
                v.memory.used as f32 / v.memory.capacity as f32
            },
        );
        let new_loop = v.loops.iter().any(|l| l.phase == Phase::Empty);
        let clear = v
            .loops
            .get(v.selected)
            .is_some_and(|l| l.phase != Phase::Empty);
        if let Some(l) = ui.get_mut::<LoopList>(LOOPS) {
            if l.rows != rows
                || l.picked != v.selected
                || l.memory != memory
                || l.new_loop != new_loop
                || l.clear != clear
            {
                l.rows = rows;
                l.picked = v.selected;
                l.memory = memory;
                l.new_loop = new_loop;
                l.clear = clear;
            }
        }
        // The editor and the Q-Links.
        self.refresh_editor(ui, v);
        self.refresh_qlinks(ui, v);
        // How it records.
        let ready = v.source == LoopSource::Pads || v.line_on;
        let set_rows: Vec<SettingRow> = SET_ROWS
            .iter()
            .map(|r| setting_row(*r, v, settings, self.hold, ready))
            .collect();
        if let Some(w) = ui.get_mut::<LoopSettings>(RECORDING) {
            let right = "the Play strip uses these".to_string();
            if w.rows != set_rows || w.right != right {
                w.rows = set_rows;
                w.right = right;
            }
        }
        // What to do with it: Resample works while a loop plays and one is
        // empty; the bounce to a pad, a clip or a file comes next.
        let resample = v.loops.iter().any(|l| l.phase == Phase::Empty)
            && v.loops
                .iter()
                .any(|l| l.phase == Phase::Playing && !l.muted);
        let use_rows: Vec<(String, String, bool)> = USE_ROWS
            .iter()
            .enumerate()
            .map(|(i, (a, b))| (a.to_string(), b.to_string(), i == USE_RESAMPLE && resample))
            .collect();
        if let Some(w) = ui.get_mut::<UseLoop>(USE) {
            if w.rows != use_rows {
                w.rows = use_rows;
            }
        }
        self.view = v.clone();
        self.update_strip(ui, v);
    }

    /// The Play page's strip: the same loop, its layers and its state.
    fn update_strip(&mut self, ui: &mut Ui, v: &LooperView) {
        let s = &v.status;
        let empty = s.phase == Phase::Empty;
        let n_bars = if s.frames == 0 {
            0
        } else {
            bars_of(s.ticks, v.beats_per_bar)
        };
        let right = if empty {
            String::new()
        } else {
            let len = if n_bars == 0 {
                String::new()
            } else if n_bars == 1 {
                "1 bar · ".to_string()
            } else {
                format!("{n_bars} bars · ")
            };
            format!(
                "{len}{} layer{}",
                s.layers,
                if s.layers == 1 { "" } else { "s" }
            )
        };
        let right_wide = if empty {
            String::new()
        } else {
            let input = match v.source {
                LoopSource::Pads => "pads and synth",
                LoopSource::Line => "line in",
            };
            match right.split_once(" · ") {
                Some((len, layers)) => format!("{len} · input: {input} · {layers}"),
                None => format!("input: {input} · {right}"),
            }
        };
        let names: Vec<String> = (0..s.layers).map(|k| self.layer_name(k)).collect();
        let info = |k: usize| v.layers.get(k).copied().flatten();
        let muted: Vec<bool> = (0..s.layers)
            .map(|k| info(k).is_some_and(|l| l.muted))
            .collect();
        let heard: Vec<bool> = (0..s.layers)
            .map(|k| info(k).is_none_or(|l| l.audible))
            .collect();
        let state = match s.phase {
            Phase::Empty => None,
            Phase::Armed => Some(("ARMED", Tone::Warn)),
            Phase::Recording => Some(("RECORDING", Tone::Bad)),
            Phase::Overdubbing => Some(("OVERDUB", Tone::Warn)),
            Phase::Playing if s.muted => Some(("MUTED", Tone::Dim)),
            Phase::Playing => Some(("PLAYING", Tone::Good)),
        }
        .map(|(a, b)| (a.to_string(), b));
        let note = if empty {
            format!(
                "Empty: REC records the {}",
                match v.source {
                    LoopSource::Pads => "pads and the synth",
                    LoopSource::Line => "line input",
                }
            )
        } else {
            String::new()
        };
        // While the loop plays, REC overdubs (as on the Looper tab), but
        // the strip has its own Overdub button, so this one stays "Record".
        let strip_record = match s.phase {
            Phase::Playing | Phase::Overdubbing => "Record",
            _ => record_label(s),
        };
        let buttons: Vec<(String, bool)> = vec![
            (
                strip_record.to_string(),
                matches!(s.phase, Phase::Armed | Phase::Recording),
            ),
            ("Overdub".to_string(), s.phase == Phase::Overdubbing),
            ("Play".to_string(), false),
            ("Stop".to_string(), false),
            ("Undo".to_string(), false),
            ("Redo".to_string(), false),
            (if s.muted { "Unmute" } else { "Mute" }.to_string(), s.muted),
            ("Clear".to_string(), false),
        ];
        let live = live_place(
            v,
            ui.get::<LoopStrip>(LOOP_STRIP)
                .and_then(|st| st.wave.as_ref()),
        );
        let position = match live {
            Some((at, _)) => Some(at),
            None => (!empty && s.frames > 0).then_some(s.position),
        };
        // The bar lines: the loop's, or while the first pass records, the
        // ones its wave is drawn against.
        let n_bars = live.map_or(n_bars, |(_, b)| b);
        if let Some(st) = ui.get_mut::<LoopStrip>(LOOP_STRIP) {
            if st.right != right
                || st.right_wide != right_wide
                || st.names != names
                || st.muted != muted
                || st.heard != heard
                || st.state != state
                || st.note != note
                || st.position != position
                || st.bars != n_bars
                || st.layers != s.layers
                || st.buttons != buttons
            {
                st.right = right;
                st.right_wide = right_wide;
                st.names = names;
                st.muted = muted;
                st.heard = heard;
                st.state = state;
                st.note = note;
                st.position = position;
                st.bars = n_bars;
                st.layers = s.layers;
                st.buttons = buttons;
            }
        }
    }
}

/// What the Record button does now.
fn record_label(s: &mpfree_looper::Status) -> &'static str {
    match s.phase {
        Phase::Empty => "Record",
        Phase::Armed => "Cancel",
        Phase::Recording if s.ending => "Ending",
        Phase::Recording => "End",
        Phase::Playing => "Overdub",
        Phase::Overdubbing => "Stop dub",
    }
}

/// What the panel shows of `row`.
fn setting_row(row: SetRow, v: &LooperView, set: &Settings, peak: f32, ready: bool) -> SettingRow {
    let on_off = |b: bool| if b { "on" } else { "off" }.to_string();
    let (label, value, tone, steps) = match row {
        SetRow::RecordFrom => (
            "Record from",
            format!("{}{}", v.source.name(), if ready { "" } else { " (off)" }),
            if ready { Tone::Normal } else { Tone::Warn },
            true,
        ),
        SetRow::FirstPass => (
            "First pass",
            match set.loop_bars {
                Some(1) => "1 bar".to_string(),
                Some(n) => format!("{n} bars"),
                None => "free".to_string(),
            },
            Tone::Normal,
            true,
        ),
        SetRow::Start => ("Start", "next bar line".to_string(), Tone::Dim, false),
        SetRow::After => (
            "After recording",
            match v.status.after {
                After::Play => "play",
                After::Overdub => "overdub",
            }
            .to_string(),
            Tone::Normal,
            true,
        ),
        SetRow::InputPeak => (
            "Input peak",
            if peak.is_finite() {
                format!("{peak:.1} dBFS")
            } else {
                "silent".to_string()
            },
            if peak > -1.0 {
                Tone::Bad
            } else if peak > -6.0 {
                Tone::Warn
            } else {
                Tone::Normal
            },
            false,
        ),
        SetRow::LineInput => ("Line input", on_off(set.line_input), Tone::Normal, true),
        SetRow::Monitor => (
            "Monitor",
            on_off(v.monitor),
            if set.line_input {
                Tone::Normal
            } else {
                Tone::Dim
            },
            set.line_input,
        ),
        SetRow::Latency => match v.source {
            LoopSource::Pads => ("Latency", "none (pads)".to_string(), Tone::Dim, false),
            LoopSource::Line => (
                "Latency",
                format!(
                    "{} frames{}",
                    v.status.latency,
                    if set.loop_latency_set {
                        ""
                    } else {
                        " (estimate)"
                    }
                ),
                if set.loop_latency_set {
                    Tone::Normal
                } else {
                    Tone::Warn
                },
                true,
            ),
        },
        SetRow::Tempo => (
            "At another tempo",
            "like tape".to_string(),
            Tone::Dim,
            false,
        ),
    };
    SettingRow {
        label: label.to_string(),
        value,
        tone,
        steps,
    }
}

/// Bars in a loop of `ticks`, rounded (0 before it has a length).
/// While the first pass records and `wave` is its wave so far: the
/// playhead (how far the pass has got along the wave, 0 to 1) and the bars
/// the wave is drawn against.
fn live_place(v: &LooperView, wave: Option<&Arc<LoopWave>>) -> Option<(f32, usize)> {
    if v.status.phase != Phase::Recording {
        return None;
    }
    let w = wave?;
    let at = w.live()?;
    let bar = v.frames_per_beat * f64::from(v.beats_per_bar.max(1));
    let bars = if bar > 0.0 {
        (w.frames as f64 / bar).round().max(1.0) as usize
    } else {
        0
    };
    Some((at, bars))
}

fn bars_of(ticks: i64, beats_per_bar: u32) -> usize {
    if ticks <= 0 {
        return 0;
    }
    (ticks as f64 / (960.0 * f64::from(beats_per_bar.max(1))))
        .round()
        .max(1.0) as usize
}

/// A loop's line in the list: "4 bars · 3 layers", "empty".
/// The memory gauge's times: "37 s" under a minute, "1:42" above, and the
/// whole size in minutes ("5 min"; it is a little over, whole blocks).
fn memory_time(seconds: f64, whole: bool) -> String {
    let s = seconds.max(0.0);
    if s < 60.0 {
        format!("{s:.0} s")
    } else if whole {
        format!("{:.0} min", (s / 60.0).floor())
    } else {
        let t = s.round() as u64;
        format!("{}:{:02}", t / 60, t % 60)
    }
}

fn loop_detail(l: &mpfree_looper::Status, beats_per_bar: u32) -> String {
    match l.phase {
        Phase::Empty => "empty".to_string(),
        Phase::Armed => "waits for the bar line".to_string(),
        Phase::Recording => "recording the first pass".to_string(),
        _ => {
            let bars = bars_of(l.ticks, beats_per_bar);
            format!(
                "{bars} bar{} · {} layer{}",
                if bars == 1 { "" } else { "s" },
                l.layers,
                if l.layers == 1 { "" } else { "s" }
            )
        }
    }
}

/// A loop's state as a word and a tone; None when empty.
fn state_of(l: &mpfree_looper::Status) -> Option<(&'static str, Tone)> {
    match l.phase {
        Phase::Empty => None,
        Phase::Armed => Some(("ARMED", Tone::Warn)),
        Phase::Recording => Some(("RECORDING", Tone::Bad)),
        Phase::Overdubbing => Some(("OVERDUB", Tone::Warn)),
        Phase::Playing if l.muted => Some(("MUTED", Tone::Dim)),
        Phase::Playing => Some(("PLAYING", Tone::Good)),
    }
}

/// What an empty loop says in its wave's place.
fn empty_note(v: &LooperView) -> String {
    if v.status.phase != Phase::Empty {
        return String::new();
    }
    format!(
        "Empty: REC records the {}",
        match v.source {
            LoopSource::Pads => "pads and the synth",
            LoopSource::Line => "line input",
        }
    )
}

/// A pan as the card prints it: "L 12", "C", "R 30".
pub fn pan_text(pan: f32) -> String {
    let p = (pan * 50.0).round() as i32;
    match p {
        0 => "C".to_string(),
        p if p < 0 => format!("L {}", -p),
        p => format!("R {p}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looper_tab::{LoopEditor, LoopList, LoopSettings, UseLoop};
    use mpfree_ui::{Fonts, SoftCanvas, Theme};

    fn page() -> (Ui, LooperPage) {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add(
            "header",
            mpfree_ui::widgets::Header::new(&["Play", "Looper"]),
        );
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new("", Tone::Dim, Align::Left),
        );
        let page = LooperPage::add(&mut ui).unwrap();
        ui.show(LOOPER_PAGE).unwrap();
        ui.paint(&mut SoftCanvas::new(1280, 800));
        (ui, page)
    }

    fn layer(id: u8) -> LayerInfo {
        LayerInfo {
            id,
            level: 1.0,
            pan: 0.0,
            muted: false,
            solo: false,
            audible: true,
            recording: false,
            faded: 1.0,
        }
    }

    /// Loop 1 playing four bars at 92 BPM with three layers (the third
    /// muted), loop 2 two bars and muted, 16 s of a minute used.
    fn two_loops() -> LooperView {
        let mut v = LooperView {
            frames_per_beat: 44_100.0 * 60.0 / 92.0,
            ..LooperView::default()
        };
        let s = &mut v.status;
        s.phase = Phase::Playing;
        s.layers = 3;
        s.frames = 460_174;
        s.ticks = 4 * 4 * 960;
        s.level = 1.0;
        s.muted = false;
        v.loops[0] = v.status;
        let mut second = v.status;
        second.layers = 1;
        second.ticks = 2 * 4 * 960;
        second.muted = true;
        v.loops[1] = second;
        for k in 0..3 {
            v.layers[k] = Some(layer(10 + k as u8));
        }
        if let Some(l) = v.layers[2].as_mut() {
            l.muted = true;
            l.audible = false;
        }
        v.memory = mpfree_looper::Memory {
            used: 16 * 44_100,
            capacity: 60 * 44_100,
        };
        v
    }

    #[test]
    fn the_tab_lists_the_loops_and_the_picked_loops_layers() {
        let (mut ui, mut page) = page();
        page.update(&mut ui, &two_loops(), &Settings::default());
        let list = ui.get::<LoopList>(LOOPS).unwrap();
        let rows: Vec<(&str, &str, Option<&str>)> = list
            .rows
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.detail.as_str(),
                    r.state.as_ref().map(|s| s.0.as_str()),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("Loop 1", "4 bars · 3 layers", Some("PLAYING")),
                ("Loop 2", "2 bars · 1 layer", Some("MUTED")),
                ("Loop 3", "empty", None),
                ("Loop 4", "empty", None),
            ]
        );
        assert_eq!(list.memory.0, "16 s of 1 min");
        // Above a minute, minutes and seconds, and the size in minutes.
        assert_eq!(super::memory_time(102.4, false), "1:42");
        assert_eq!(super::memory_time(300.4, true), "5 min");
        assert_eq!(super::memory_time(1200.9, true), "20 min");
        assert_eq!(super::memory_time(59.4, true), "59 s");
        assert!((list.memory.1 - 16.0 / 60.0).abs() < 1e-4);
        assert!(list.new_loop && list.picked == 0);
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.title, "Loop 1 · Editor");
        assert!(
            e.right.starts_with("4 bars at 92 BPM · 10.4 s"),
            "{}",
            e.right
        );
        assert_eq!(e.layers.len(), 3);
        assert_eq!(
            (e.layers[0].detail.as_str(), e.layers[1].detail.as_str()),
            ("first pass", "overdub")
        );
        assert!(e.layers[2].muted && !e.layers[2].heard && e.layers[0].heard);
        // The loop plays, so its edits are on; Undo history is to come.
        let on: Vec<bool> = e.tools.iter().map(|t| t.1).collect();
        assert_eq!(on, [true, true, true, true, true, true, true, false]);
        // As on mockup J, the three layers fit under the wave.
        let r = ui.rect_of(EDITOR).unwrap();
        assert_eq!(e.row_rects(LoopEditor::parts(r).2).len(), 3);
        let set = ui.get::<LoopSettings>(RECORDING).unwrap();
        let labels: Vec<&str> = set.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Record from",
                "First pass",
                "Start",
                "After recording",
                "At another tempo",
                "Line input",
                "Input peak",
                "Monitor",
                "Latency"
            ]
        );
        assert_eq!(set.rows[0].value, "Pads and synth");
        assert!(
            !set.rows[7].steps,
            "monitoring needs the line input on first"
        );
        let use_loop = ui.get::<UseLoop>(USE).unwrap();
        assert_eq!(use_loop.rows.len(), 4);
        // Only Resample is built: Loop 3 is empty and Loop 1 plays.
        let on: Vec<bool> = use_loop.rows.iter().map(|r| r.2).collect();
        assert_eq!(on, [false, false, false, true]);
        // A loop picked further down shows the list down to it.
        let mut v = two_loops();
        v.selected = 5;
        v.status = v.loops[5];
        page.update(&mut ui, &v, &Settings::default());
        let list = ui.get::<LoopList>(LOOPS).unwrap();
        assert_eq!((list.rows.len(), list.picked), (6, 5));
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.title, "Loop 6 · Editor");
        assert_eq!((e.layers.len(), e.right.as_str()), (0, "empty"));
        assert!(e.note.starts_with("Empty: REC records"), "{}", e.note);
        assert_eq!(page.first_empty(), Some(2));
        ui.paint(&mut SoftCanvas::new(1280, 800));
    }

    #[test]
    fn layers_are_named_by_what_was_played_into_them_in_each_loop() {
        let (mut ui, mut page) = page();
        let mut v = LooperView::default();
        fn step(
            ui: &mut Ui,
            v: &mut LooperView,
            page: &mut LooperPage,
            phase: Phase,
            layers: usize,
            hits: &[&'static str],
        ) {
            v.status.phase = phase;
            v.status.layers = layers;
            v.status.frames = if phase == Phase::Empty { 0 } else { 1000 };
            page.update(ui, v, &Settings::default());
            for h in hits {
                page.hit(h);
            }
            page.update(ui, v, &Settings::default());
        }
        let drums = ["Drums", "Drums", "Bass"];
        step(&mut ui, &mut v, &mut page, Phase::Recording, 1, &drums);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 1, &["Bass"]);
        let bass = ["Bass", "Drums", "Bass"];
        step(&mut ui, &mut v, &mut page, Phase::Overdubbing, 2, &bass);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 2, &[]);
        step(
            &mut ui,
            &mut v,
            &mut page,
            Phase::Overdubbing,
            3,
            &["Drums"],
        );
        step(&mut ui, &mut v, &mut page, Phase::Playing, 3, &[]);
        step(&mut ui, &mut v, &mut page, Phase::Overdubbing, 4, &[]);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 4, &[]);
        let names: Vec<String> = (0..4).map(|k| page.layer_name(k)).collect();
        assert_eq!(names, ["Drums", "Bass", "Drums 2", "Layer 4"]);
        let strip = ui.get::<LoopStrip>(LOOP_STRIP).unwrap();
        assert_eq!(strip.names, names);
        assert_eq!(
            strip.buttons[0].0, "Record",
            "the strip has its own Overdub"
        );
        assert!(
            strip.right_wide.contains("input: pads and synth"),
            "{}",
            strip.right_wide
        );
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.layers[1].detail, "overdub · bass");
        // Loop 2 has names of its own; loop 1 keeps its.
        let first = v.status;
        v.loops[0] = first;
        v.selected = 1;
        v.status = v.loops[1];
        step(&mut ui, &mut v, &mut page, Phase::Recording, 1, &["Synth"]);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 1, &[]);
        assert_eq!(page.layer_name(0), "Synth");
        v.loops[1] = v.status;
        v.selected = 0;
        v.status = first;
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(page.layer_name(2), "Drums 2");
        // A clear forgets them.
        step(&mut ui, &mut v, &mut page, Phase::Empty, 0, &[]);
        assert_eq!(page.layer_name(0), "Layer 1");
    }

    #[test]
    fn a_setting_row_steps_its_option() {
        let set = Settings::default();
        assert_eq!(set.loop_source, LoopSource::Pads);
        assert!(!set.line_input);
        let item = |i: usize, d: i32, s: &Settings| setting_item(i, d, s);
        // The source and the on-off rows flip whichever half is tapped.
        assert_eq!(item(0, -1, &set), Some((Item::LoopSource, 1)));
        assert_eq!(item(1, -1, &set), Some((Item::LoopBars, -1)));
        assert_eq!(item(2, 1, &set), None, "Start waits for the looper");
        assert_eq!(item(4, 1, &set), None);
        assert_eq!(item(5, -1, &set), Some((Item::LineInput, 1)));
        assert_eq!(item(6, 1, &set), None, "the peak is a meter");
        assert_eq!(item(7, 1, &set), Some((Item::LoopMonitor, 1)));
        assert_eq!(item(8, -1, &set), Some((Item::LoopLatency, -1)));
        assert_eq!(item(9, 1, &set), None);
        let on = Settings {
            loop_source: LoopSource::Line,
            line_input: true,
            loop_monitor: true,
            ..Settings::default()
        };
        assert_eq!(item(0, 1, &on), Some((Item::LoopSource, -1)));
        assert_eq!(item(5, 1, &on), Some((Item::LineInput, -1)));
        assert_eq!(item(7, 1, &on), Some((Item::LoopMonitor, -1)));
        assert_eq!(SET_ROWS.len(), 9);
    }

    /// The editor's tools work while the loop plays and isn't recording:
    /// each asks for its looper command; seam fade steps up and round.
    #[test]
    fn the_undo_history_names_its_steps_and_opens_in_the_editor() {
        let v = two_loops();
        let layers = v.layers;
        let id = |k: usize| layers[k].unwrap().id;
        let text = |s: Step| step_text(s, &layers, 44_100);
        assert_eq!(text(Step::Overdub(id(1))), "Overdub · Layer 2");
        assert_eq!(text(Step::Delete(id(2))), "Delete · Layer 3");
        assert_eq!(text(Step::Delete(99)), "Delete a layer");
        assert_eq!(text(Step::Edit(Edit::Nudge(-1))), "Nudge ← 1/16");
        assert_eq!(text(Step::Edit(Edit::Nudge(2))), "Nudge 2 × 1/16 →");
        assert_eq!(text(Step::Edit(Edit::SeamFade(441))), "Seam fade 10 ms");
        assert_eq!(text(Step::Edit(Edit::Half)), "½ length");
        assert_eq!(text(Step::Merge(id(0))), "Merge layers");
        // With no steps the tool is faint; open, it is lit and lists Start.
        let (mut ui, mut page) = page();
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(!e.tools[TOOL_HISTORY].1 && e.history().is_none());
        // The loop plays, so the wave's ends trim it.
        assert_eq!(e.trim_ticks, Some(4 * 4 * 960));
        let s = &v.status;
        assert_eq!(
            trim_command(960, 3840, s),
            Some(Command::Edit(Edit::Trim {
                from: 960,
                to: 3840
            }))
        );
        assert_eq!(trim_command(960, 960, s), None);
        assert_eq!(trim_command(0, 4 * 4 * 960 + 240, s), None);
        page.toggle_history(&mut ui);
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.lit, Some(TOOL_HISTORY));
        assert!(e.tools[TOOL_HISTORY].1, "open, it can be closed");
        let h = e.history().unwrap();
        assert_eq!(
            (h.rows.as_slice(), h.now),
            (&["Start: the first pass".to_string()][..], 0)
        );
        assert_eq!(page.history_steps(0), None, "now is no step");
        ui.paint(&mut SoftCanvas::new(1280, 800));
        // Picking another loop closes it.
        let mut other = v.clone();
        other.selected = 1;
        other.status = v.loops[1];
        page.update(&mut ui, &other, &Settings::default());
        assert!(!page.history_open());
        assert!(ui.get::<LoopEditor>(EDITOR).unwrap().history().is_none());
    }

    #[test]
    fn the_tools_ask_for_the_looper_edits_while_the_loop_plays() {
        let v = two_loops();
        let mut s = v.status;
        s.seam = mpfree_looper::FADE;
        let cmd = |i: usize, s: &Status| tool_command(i, s, 44_100);
        assert_eq!(cmd(0, &s), Some(Command::Edit(Edit::Half)));
        assert_eq!(cmd(1, &s), Some(Command::Edit(Edit::Double)));
        assert_eq!(cmd(2, &s), Some(Command::Edit(Edit::Reverse)));
        assert_eq!(cmd(3, &s), Some(Command::Edit(Edit::Nudge(-1))));
        assert_eq!(cmd(4, &s), Some(Command::Edit(Edit::Nudge(1))));
        assert_eq!(cmd(6, &s), Some(Command::Merge));
        assert_eq!(cmd(7, &s), None, "the undo history is the page's own");
        // Seam fade: 10 ms, 50 ms, the longest, then a straight cut.
        let mut seams = Vec::new();
        for _ in 0..4 {
            let Some(Command::Edit(Edit::SeamFade(f))) = cmd(5, &s) else {
                panic!("seam fade");
            };
            seams.push(f);
            s.seam = f as usize;
        }
        assert_eq!(seams, [441, 2205, MAX_SEAM as u32, 0]);
        let names = tools(&s, 44_100);
        assert_eq!(names[5], ("Seam fade cut".to_string(), true));
        // One layer has nothing to merge; while overdubbing, or merging,
        // nothing works.
        s.layers = 1;
        assert_eq!(cmd(6, &s), None);
        s.phase = Phase::Overdubbing;
        assert!(tools(&s, 44_100).iter().all(|(_, on)| !on));
        s.phase = Phase::Playing;
        s.merging = Some(0.4);
        assert_eq!(tools(&s, 44_100)[6], ("Merging 40 %".to_string(), false));
        // After recording flips between play and overdub.
        let after = SET_ROWS.iter().position(|r| *r == SetRow::After).unwrap();
        assert_eq!(
            setting_command(after, &s),
            Some(Command::SetAfter(After::Overdub))
        );
        assert_eq!(setting_command(0, &s), None);
    }

    #[test]
    fn the_q_links_turn_the_picked_layer_and_the_loop() {
        let (mut ui, mut page) = page();
        let v = two_loops();
        page.update(&mut ui, &v, &Settings::default());
        page.pick_layer(&mut ui, 1);
        assert_eq!(ui.get::<LoopEditor>(EDITOR).unwrap().picked, 1);
        let card = |ui: &Ui, k: usize| {
            let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
            (q.cards[k].scope.clone(), q.cards[k].value.clone())
        };
        assert_eq!(card(&ui, 0), ("Layer 2".to_string(), "0.0 dB".to_string()));
        // Q1: the layer's level, half a dB a detent.
        let Some(LoopTurn::Layer(1, LayerCommand::Level(l))) = page.qlink(&mut ui, 0, 2, false)
        else {
            panic!("Q1 turns the picked layer's level");
        };
        assert!((l - 10f32.powf(1.0 / 20.0)).abs() < 1e-4);
        assert_eq!(card(&ui, 0).1, "1.0 dB");
        // SHIFT turns it in tenths.
        page.qlink(&mut ui, 0, -3, true);
        assert_eq!(card(&ui, 0).1, "0.7 dB");
        // Q2: its pan.
        assert_eq!(
            page.qlink(&mut ui, 1, -6, false),
            Some(LoopTurn::Layer(1, LayerCommand::Pan(-0.12)))
        );
        assert_eq!(card(&ui, 1).1, "L 6");
        // Q3: the loop's Fade old, a percent a detent; Q4: its seam fade,
        // a millisecond a detent (mockup J).
        let Some(LoopTurn::FadeOld(k)) = page.qlink(&mut ui, 2, -2, false) else {
            panic!("Q3 turns the loop's Fade old");
        };
        assert!((k - 0.98).abs() < 1e-4, "{k}");
        assert_eq!(card(&ui, 2), ("Loop 1".to_string(), "98 %".to_string()));
        let before = v.status.seam;
        let Some(LoopTurn::Seam(f)) = page.qlink(&mut ui, 3, 10, false) else {
            panic!("Q4 turns the loop's seam fade");
        };
        assert_eq!(f as usize, before + 441);
        assert_eq!(card(&ui, 3).1, seam_text(before + 441, 44_100));
        assert_eq!(page.qlink(&mut ui, 0, 0, false), None);
        // The looper's next view (from before the turn) does not undo the
        // cards while it catches up.
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(card(&ui, 0).1, "0.7 dB");
        assert_eq!(card(&ui, 1).1, "L 6");
        assert_eq!(page.layer_info(1).map(|l| l.id), Some(11));
        // An empty loop has no layer or level to turn.
        let (mut ui, mut page) = self::page();
        page.update(&mut ui, &LooperView::default(), &Settings::default());
        assert_eq!(page.qlink(&mut ui, 0, 1, false), None);
        assert_eq!(page.qlink(&mut ui, 2, 1, false), None);
        assert_eq!(card(&ui, 0).0, "No layer");
    }

    #[test]
    fn a_pan_reads_as_the_cards_print_it() {
        assert_eq!(pan_text(0.0), "C");
        assert_eq!(pan_text(-0.24), "L 12");
        assert_eq!(pan_text(0.6), "R 30");
        assert_eq!(bars_of(0, 4), 0);
        assert_eq!(bars_of(4 * 960, 4), 1);
        assert_eq!(bars_of(7 * 960, 4), 2);
    }
}
