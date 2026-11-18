//! The Looper tab as an instrument (the looper redesign, approved by Jay
//! on 17 Nov): the deck of loops across the top ([`LoopDeck`]), the
//! picked loop under it at full width ([`LoopEditor`]), and the Q-Links on
//! the right. Q1 to Q4 are loops 1 to 4's levels; Q-LINK steps to loops 5
//! to 8, then to the picked loop (Feedback, the picked layer's level and
//! pan, the seam fade). And the Play page's loop strip (mockups F and H),
//! which shows the picked loop.
//!
//! The panels are [`crate::looper_deck`]'s and [`crate::looper_tab`]'s;
//! this fills them from the looper's view. REC, OVERDUB and UNDO on the
//! MPC work the picked loop while this tab shows (`App::looper_tap`), as
//! do the strip's buttons on Play.

use crate::app::Settings;
use crate::looper_deck::{Channel, DeckHead, LoopDeck, RingState, DECK};
use crate::looper_tab::{
    level_frac, level_text, Chip, ChipKind, Ctl, DropSpot, DropTo, EmptyLook, HistoryList,
    LayerDrag, LayerRow, LoopEditor, Strip, BAR_CHOICES, EDITOR, LOOP_COLOURS, QLINKS, SEND_SHOW,
};
use crate::looping::{LoopSource, LoopWave, LooperView};
use mpfree_looper::{
    After, Command, Edit, History, LayerCommand, LayerInfo, Phase, Status, Step, MAX_LAYERS,
    MAX_LOOPS, MAX_SEAM,
};
use mpfree_time_ui::view::{LoopPlace, TransportInfo};
use mpfree_transport::State;
use mpfree_ui::draw;
use mpfree_ui::qlink::{QLinkCard, QLinkColumn};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Tone, Touch, Ui, Widget};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const LOOPER_PAGE: &str = "Looper";
/// The deck across the top, the picked loop under it, the Q-Links on the
/// right (the redesign's Main board).
pub const LOOPER_LAYOUT: &str =
    "col(header:60, row(col(ldeck:240, leditor:*):*, lqlinks:128):*, hint:26)";

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
    /// The Arm pill at the left of the heading: the loop as a REC target.
    pub arm: Option<crate::play_widgets::ArmPill>,
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
    arm: Option<crate::play_widgets::ArmPill>,
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
/// The layers' colours, layer 1 first: the loops' (none red, amber or
/// orange, which mean recording and overdubbing), less the grey.
pub const LAYER_COLOURS: [Color; 7] = [
    LOOP_COLOURS[0],
    LOOP_COLOURS[1],
    LOOP_COLOURS[2],
    LOOP_COLOURS[3],
    LOOP_COLOURS[4],
    LOOP_COLOURS[5],
    LOOP_COLOURS[6],
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
            arm: None,
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
            arm: self.arm,
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
        // The Arm pill takes the heading's left, the title after it.
        let title_x = match self.arm {
            Some(_) => {
                draw::card(cx, r, "", right);
                let (h, _) = draw::card_head(r);
                let x = h.x + crate::play_widgets::ARM_PILL_W + 10;
                cx.text(
                    FontId::Bold,
                    draw::HEAD_PX,
                    Rect::new(x, h.y, 80, h.h),
                    Align::Left,
                    t.text_dim,
                    "LOOPER",
                );
                x
            }
            None => {
                draw::card(cx, r, "Looper", right);
                draw::card_head(r).0.x
            }
        };
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
        let mut lx = title_x + cx.measure(FontId::Bold, draw::HEAD_PX, "LOOPER") + 24;
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
            // The name and its speaker are one mute button: outlined, and
            // answering from the card's top edge to below the heading
            // (Jay after live test 3: bigger mutes).
            let button = Rect::new(lx - 6, head.y - 5, sx + 14 - (lx - 6), head.h + 10);
            cx.canvas.stroke_round_rect(button, 8, 1, t.outline);
            draw::speaker(cx, sx, cy, t.text_dim, muted);
            speakers.push(Rect::new(
                button.x,
                r.y,
                button.w,
                button.bottom() + 12 - r.y,
            ));
            lx = sx + 12 + 18;
        }
        *self.speakers.borrow_mut() = speakers;
        if let Some(pill) = self.arm {
            crate::play_widgets::paint_arm(cx, r, pill);
        }
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
        if let Some(pill) = self.arm {
            out.push(Target::tap(
                "arm",
                0,
                pill.label(),
                crate::play_widgets::arm_pill_hit(r),
            ));
        }
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
        if self.arm.is_some() && crate::play_widgets::arm_pill_hit(r).contains(x, y) {
            return Some(EventKind::Select(0));
        }
        let row = LoopStrip::parts(r).1?;
        self.button_rects(row)
            .iter()
            .position(|b| b.contains(x, y))
            .map(EventKind::Tab)
    }
}

/// The trim grips' range, as a command while the loop can be edited.
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
        // Dragged here from another loop, or from here to another.
        Step::Added(id, from) => name(id).map_or_else(
            || format!("From loop {}", from + 1),
            |n| format!("From loop {} · {n}", from + 1),
        ),
        Step::Moved(_, to) => format!("Moved to loop {}", to + 1),
    }
}

/// The undo history as the panel lists it: Start (the first pass), the
/// steps Undo takes back, oldest first, then the ones Redo brings back.
pub fn history_list(h: &History, layers: &[Option<LayerInfo>], rate: u32) -> HistoryList {
    let mut rows = vec!["Start: the first pass".to_string()];
    rows.extend(h.done().iter().map(|&s| step_text(s, layers, rate)));
    let now = rows.len() - 1;
    rows.extend(h.redo().iter().map(|&s| step_text(s, layers, rate)));
    HistoryList { rows, now }
}

/// A seam fade as text: "10 ms", "cut".
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

/// What the panel's control `c` asks of the picked loop, for the ones
/// that are a looper command; None while it can't work.
pub fn strip_command(c: Ctl, s: &Status) -> Option<Command> {
    match c {
        Ctl::Reverse if editable(s) => Some(Command::Edit(Edit::Reverse)),
        Ctl::Half if editable(s) => Some(Command::Edit(Edit::Half)),
        Ctl::Double if editable(s) => Some(Command::Edit(Edit::Double)),
        Ctl::Merge if editable(s) && s.layers > 1 => Some(Command::Merge),
        Ctl::Then(i) => Some(Command::SetAfter(if i == 0 {
            After::Play
        } else {
            After::Overdub
        })),
        _ => None,
    }
}

/// Why the panel's control `c` can't work now, for the screen to say in
/// place when its faint button is tapped (Jay, 12 Nov).
pub fn why_faint(c: Ctl, s: &Status) -> &'static str {
    let phase = match s.phase {
        Phase::Empty => "Record a loop first: this changes the loop that plays",
        Phase::Armed | Phase::Recording => "This works once the first pass is recorded and plays",
        Phase::Overdubbing => "This works while the loop plays, not while it overdubs",
        Phase::Playing if s.merging.is_some() => "Wait for the merge to finish",
        Phase::Playing => "That can't change this loop now",
    };
    match c {
        Ctl::Send => "Record a loop first: Send sends the loop that plays",
        Ctl::Clear => "Nothing to clear: the loop is empty",
        Ctl::Undo => "Nothing to undo on this loop yet",
        Ctl::Redo => "Nothing to redo: Undo first",
        Ctl::History => "No history yet: overdub a layer first",
        Ctl::Merge if s.phase == Phase::Playing && s.merging.is_none() => {
            "Merge needs two layers or more: overdub one first"
        }
        _ => phase,
    }
}

/// Why Send's choice `i` can't be used now.
pub fn why_no_send(i: usize) -> &'static str {
    if i == SEND_SHOW {
        return "This loop is in no cell yet: once it has a first pass it shows on a Loop track";
    }
    "Not built yet: Show in Clips works now (the bounce to a pad, a track or a file comes next)"
}

/// Feedback as the panel and the Q-Links move it: 50 % to 100 % kept a
/// pass, in steps of 1 % (SHIFT: 0.2 %).
const FADE_OLD_MIN: f32 = 0.5;

/// How far round Feedback's ring is lit, 0 to 1.
fn fade_old_pos(k: f32) -> f32 {
    ((k - FADE_OLD_MIN) / (1.0 - FADE_OLD_MIN)).clamp(0.0, 1.0)
}

/// A Q-Link turn on the Looper tab: what it asks of the looper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopTurn {
    /// Loop `n`'s level (the first two banks).
    LoopLevel(usize, f32),
    /// The picked layer's level or pan, by play order.
    Layer(usize, LayerCommand),
    /// The picked loop's Feedback, 0.5 to 1 kept a pass.
    FadeOld(f32),
    /// The picked loop's seam fade, in frames.
    Seam(u32),
}

/// The Q-Link banks on the Looper tab, in Q-LINK's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopBank {
    /// Loops 1 to 4's levels.
    #[default]
    First,
    /// Loops 5 to 8's levels.
    Second,
    /// The picked loop: Feedback, the picked layer's level and pan, the
    /// seam fade.
    Picked,
}

impl LoopBank {
    pub fn next(self) -> LoopBank {
        match self {
            LoopBank::First => LoopBank::Second,
            LoopBank::Second => LoopBank::Picked,
            LoopBank::Picked => LoopBank::First,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LoopBank::First => "Loops 1-4",
            LoopBank::Second => "Loops 5-8",
            LoopBank::Picked => "the picked loop",
        }
    }
}

/// A take being recorded: its loop, its layer, and how many hits of each
/// group of sounds went into it.
type Take = (usize, usize, Vec<(&'static str, u32)>);

/// The input's level history the empty loop shows: a value every few
/// views, about five seconds of them.
const INPUT_HISTORY: usize = 120;
const INPUT_EVERY: u32 = 2;

/// A level in dBFS along the input meter, 0 to 1 (-48 dB to 0).
fn input_frac(db: f32) -> f32 {
    if !db.is_finite() {
        return 0.0;
    }
    ((db + 48.0) / 48.0).clamp(0.0, 1.0)
}

pub struct LooperPage {
    /// The input's peak, held for about 2 s of updates.
    hold: f32,
    hold_age: u32,
    /// The input's level, oldest first, for an empty loop's look.
    input: VecDeque<f32>,
    input_age: u32,
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
    /// A loop's level set from the screen a moment ago (its bar, its
    /// Q-Link): the deck shows it until the looper has caught up.
    levelled: [Option<(f32, Instant)>; MAX_LOOPS],
    /// The undo history is open in the lanes' place.
    history_open: bool,
    /// REC records the loop ([`crate::rec_arm`]): the strip's empty text
    /// says what REC does.
    rec_loop: bool,
    /// Each loop's place on the Clips grid (Loop tracks), for its name.
    places: [Option<LoopPlace>; MAX_LOOPS],
    /// The song's clock, for the deck's header and a waiting loop's bar.
    transport: TransportInfo,
    /// What the Q-Links turn.
    bank: LoopBank,
    /// The pads are the loop keys: the deck's Pads switch is lit.
    pads_keys: bool,
}

impl LooperPage {
    pub fn add(ui: &mut Ui) -> Result<LooperPage, String> {
        ui.add(DECK, LoopDeck::default());
        ui.add(EDITOR, LoopEditor::default());
        ui.add(QLINKS, QLinkColumn::new());
        ui.add(LOOP_STRIP, LoopStrip::default());
        ui.add_page(LOOPER_PAGE, LOOPER_LAYOUT)?;
        let mut p = LooperPage {
            hold: f32::NEG_INFINITY,
            hold_age: 0,
            input: VecDeque::with_capacity(INPUT_HISTORY),
            input_age: 0,
            names: vec![Vec::new(); MAX_LOOPS],
            take: None,
            layers: [None; MAX_LAYERS],
            selected: 0,
            layer: 0,
            view: LooperView::default(),
            turned: None,
            levelled: [None; MAX_LOOPS],
            history_open: false,
            rec_loop: false,
            places: Default::default(),
            transport: TransportInfo::default(),
            bank: LoopBank::default(),
            pads_keys: false,
        };
        p.update(ui, &LooperView::default(), &Settings::default());
        Ok(p)
    }

    /// Each loop's place on the Clips grid, from the time engine's state;
    /// true when one changed (the page is then refreshed).
    pub fn set_places(&mut self, places: [Option<LoopPlace>; MAX_LOOPS]) -> bool {
        if self.places == places {
            return false;
        }
        self.places = places;
        true
    }

    /// Loop `n` as the screens name it: by its place on the Clips grid,
    /// "Loops B · Loop 2 · Verse", or "Loop 2" while it is in no cell.
    pub fn loop_name(&self, n: usize) -> String {
        match self.places.get(n).and_then(Option::as_ref) {
            Some(p) => p.full(n),
            None => format!("Loop {}", n + 1),
        }
    }

    /// Where loop `n` sits on the Clips grid, if it is in a cell.
    pub fn place(&self, n: usize) -> Option<&LoopPlace> {
        self.places.get(n).and_then(Option::as_ref)
    }

    /// Whether REC records the loop (it is armed) or the notes: the
    /// strip's empty text follows.
    pub fn set_rec_loop(&mut self, ui: &mut Ui, on: bool) {
        if self.rec_loop != on {
            self.rec_loop = on;
            let v = self.view.clone();
            self.update_strip(ui, &v);
        }
    }

    /// The song's clock moved: the deck's header, and a waiting loop's
    /// bar.
    pub fn set_transport(&mut self, ui: &mut Ui, t: TransportInfo) {
        if self.transport == t {
            return;
        }
        self.transport = t;
        let v = self.view.clone();
        self.refresh_deck(ui, &v);
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

    /// The name of the picked loop's layer `k`, as the lanes show it.
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

    /// Loop `n`'s level was set from the screen (its level bar): the deck
    /// keeps it while the looper catches up.
    pub fn level_set(&mut self, n: usize, level: f32) {
        if let Some(l) = self.levelled.get_mut(n) {
            *l = Some((level, Instant::now()));
        }
    }

    /// Whether the pads are the loop keys (the app's status): the deck's
    /// Pads switch follows.
    pub fn set_pads_keys(&mut self, ui: &mut Ui, on: bool) {
        if self.pads_keys == on {
            return;
        }
        self.pads_keys = on;
        let v = self.view.clone();
        self.refresh_deck(ui, &v);
    }

    /// The Q-Links' bank now.
    pub fn bank(&self) -> LoopBank {
        self.bank
    }

    /// Q-LINK: the next bank.
    pub fn next_bank(&mut self, ui: &mut Ui) -> LoopBank {
        self.bank = self.bank.next();
        let v = self.view.clone();
        self.refresh_qlinks(ui, &v);
        self.bank
    }

    /// Where a layer dragged out of the lanes can be let go: the other
    /// loops on the deck. An empty one takes it as a loop of its own; one
    /// that plays takes it when it is as long (another length, or a loop
    /// recording, says why not).
    fn drop_spots(&self, ui: &Ui, v: &LooperView) -> Vec<DropSpot> {
        let (Some(dr), Some(deck)) = (ui.rect_on(LOOPER_PAGE, DECK), ui.get::<LoopDeck>(DECK))
        else {
            return Vec::new();
        };
        let src = v.selected;
        let s = &v.status;
        let len = |l: &Status| {
            let b = bars_of(l.ticks, v.beats_per_bar);
            format!("{b} bar{}", if b == 1 { "" } else { "s" })
        };
        let busy = |l: &Status| l.merging.is_some() || l.copying.is_some() || l.fitting.is_some();
        // The picked loop's layers drag while it plays with nothing busy.
        let held = match s.phase {
            Phase::Playing if !busy(s) => None,
            Phase::Playing => Some(format!(
                "Loop {} is busy: drag its layers once it is done",
                src + 1
            )),
            _ => Some(format!(
                "Loop {} is recording: drag its layers once it plays",
                src + 1
            )),
        };
        let mut out = Vec::new();
        for (n, card) in deck.cards(dr).into_iter().enumerate() {
            let Some(l) = v.loops.get(n).filter(|_| n != src) else {
                continue;
            };
            let refused = held.clone().or_else(|| match l.phase {
                Phase::Empty if l.loading.is_some() => {
                    Some(format!("Loop {} is busy: try again in a moment", n + 1))
                }
                Phase::Empty => None,
                Phase::Playing if l.ticks != s.ticks => Some(format!(
                    "Loop {} is {} and this layer {}: a layer goes only onto a loop of its length",
                    n + 1,
                    len(l),
                    len(s)
                )),
                Phase::Playing if l.layers >= MAX_LAYERS - 1 => {
                    Some(format!("Loop {} has all the layers it can take", n + 1))
                }
                Phase::Playing if busy(l) => {
                    Some(format!("Loop {} is busy: try again in a moment", n + 1))
                }
                Phase::Playing => None,
                _ => Some(format!("Loop {} is recording", n + 1)),
            });
            out.push(DropSpot {
                to: DropTo::Loop(n),
                rect: card,
                refused,
            });
        }
        out
    }

    /// Why a dragged layer can't go to `to`, as the screen says it.
    pub fn why_no_drop(&self, ui: &Ui, to: DropTo) -> String {
        ui.get::<LoopEditor>(EDITOR)
            .and_then(|e| e.drops.iter().find(|d| d.to == to))
            .and_then(|d| d.refused.clone())
            .unwrap_or_else(|| "That loop can't take the layer".to_string())
    }

    /// What letting a dragged layer go on `to` does, as its card says it
    /// (`copy`: SHIFT is held).
    fn drop_words(&self, to: Option<&DropSpot>, copy: bool) -> String {
        let only = self.view.status.layers <= 1;
        match to {
            None => "Let go here: it stays where it is".to_string(),
            Some(DropSpot {
                refused: Some(why), ..
            }) => why.clone(),
            Some(d) => {
                let way = if copy || only { "Copy" } else { "Move" };
                let DropTo::Loop(n) = d.to;
                let empty = self
                    .view
                    .loops
                    .get(n)
                    .is_some_and(|l| l.phase == Phase::Empty);
                if empty {
                    format!("{way} into Loop {}: a loop of its own", n + 1)
                } else {
                    format!("{way} to Loop {}", n + 1)
                }
            }
        }
    }

    /// Keep the deck and the card in step with the lanes' drag: called
    /// after every touch on the Looper tab and every turn of the wheel
    /// (`shift`: SHIFT is held, so letting go copies).
    pub fn sync_drag(&mut self, ui: &mut Ui, shift: bool) {
        // A layer the wheel picked up goes back once nothing holds it: the
        // page changed, or an overlay took the wheel.
        if !ui.focus_grabbed() && ui.get::<LoopEditor>(EDITOR).is_some_and(|e| e.wheel_held()) {
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.wheel_cancel();
            }
        }
        let Some(e) = ui.get::<LoopEditor>(EDITOR) else {
            return;
        };
        let drag = e.drag().and_then(|(k, card)| {
            let landing = e.landing();
            Some(LayerDrag {
                row: e.layers.get(k).cloned()?,
                card,
                at: e.finger()?,
                says: self.drop_words(landing, shift),
                landing: landing.map(|d| (d.to, d.refused.is_none())),
                lit: e
                    .drops
                    .iter()
                    .filter(|d| d.refused.is_none())
                    .map(|d| d.to)
                    .collect(),
            })
        });
        let says = drag.as_ref().map(|d| d.says.clone()).unwrap_or_default();
        if e.says != says {
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.says = says;
            }
        }
        if ui.get::<LoopDeck>(DECK).is_some_and(|d| d.drag != drag) {
            if let Some(d) = ui.get_mut::<LoopDeck>(DECK) {
                d.drag = drag;
            }
        }
    }

    /// The history chips: open the undo history in the lanes' place, or
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

    /// The picked loop's undo history for the list that holding UNDO
    /// opens: the loop's name, and its rows as the panel lists them.
    pub fn history_rows(&self) -> (String, HistoryList) {
        let v = &self.view;
        (
            self.loop_name(v.selected),
            history_list(&v.history, &self.layers, v.rate),
        )
    }

    /// A tap on a layer's lane: that layer goes on the Q-Links.
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
        let base = if v.resample && s.resampling {
            "Loops"
        } else {
            match v.source {
                LoopSource::Line => "Line in",
                LoopSource::Pads => match counts.iter().max_by_key(|(_, c)| *c) {
                    Some((g, _)) => g,
                    None => "",
                },
            }
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

    /// The new loop waveform (empty when the loop was cleared), in the
    /// lanes and on the Play page's strip. While the first pass records,
    /// the wave grows with it and the playhead follows its end.
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
    /// a loop's level in the first two banks; the picked loop's Feedback,
    /// the picked layer's level and pan, and the seam fade in the third.
    /// The card follows at once.
    pub fn qlink(&mut self, ui: &mut Ui, k: usize, ticks: i32, fine: bool) -> Option<LoopTurn> {
        if ticks == 0 || k > 3 {
            return None;
        }
        let d = ticks as f32 * if fine { 0.1 } else { 0.5 };
        let step_db = |level: f32| {
            let db = (20.0 * level.max(1e-6).log10()).max(-60.0);
            let db = (db + d).clamp(-60.0, 6.0);
            if db <= -60.0 {
                0.0
            } else {
                10f32.powf(db / 20.0)
            }
        };
        let turn = match self.bank {
            LoopBank::First | LoopBank::Second => {
                let n = k + if self.bank == LoopBank::Second { 4 } else { 0 };
                let l = self.view.loops.get(n)?;
                if l.phase == Phase::Empty {
                    return None;
                }
                let now = self.levelled[n].map_or(l.level, |(v, _)| v);
                let level = step_db(now);
                self.levelled[n] = Some((level, Instant::now()));
                LoopTurn::LoopLevel(n, level)
            }
            LoopBank::Picked => match k {
                0 => {
                    let s = &mut self.view.status;
                    if s.phase == Phase::Empty {
                        return None;
                    }
                    let step = if fine { 0.002 } else { 0.01 };
                    s.fade_old = (s.fade_old + ticks as f32 * step).clamp(FADE_OLD_MIN, 1.0);
                    self.turned = Some(Instant::now());
                    LoopTurn::FadeOld(s.fade_old)
                }
                1 | 2 => {
                    let info = self.layers.get_mut(self.layer)?.as_mut()?;
                    let cmd = if k == 1 {
                        info.level = step_db(info.level);
                        LayerCommand::Level(info.level)
                    } else {
                        let step = if fine { 0.01 } else { 0.02 };
                        info.pan = (info.pan + ticks as f32 * step).clamp(-1.0, 1.0);
                        LayerCommand::Pan(info.pan)
                    };
                    let info = *info;
                    if let Some(slot) = self.view.layers.get_mut(self.layer) {
                        *slot = Some(info);
                    }
                    self.turned = Some(Instant::now());
                    LoopTurn::Layer(self.layer, cmd)
                }
                _ => {
                    let rate = self.view.rate;
                    let s = &mut self.view.status;
                    if !editable(s) {
                        return None;
                    }
                    // A millisecond a detent (SHIFT: a tenth).
                    let step = (rate as f32 / if fine { 10_000.0 } else { 1000.0 }).max(1.0);
                    let seam = (s.seam as f32 + ticks as f32 * step).clamp(0.0, MAX_SEAM as f32);
                    s.seam = seam.round() as usize;
                    self.turned = Some(Instant::now());
                    LoopTurn::Seam(s.seam as u32)
                }
            },
        };
        let v = self.view.clone();
        self.refresh_qlinks(ui, &v);
        self.refresh_deck(ui, &v);
        self.refresh_editor(ui, &v);
        Some(turn)
    }

    /// Loop `n`'s level as the screen shows it: what the screen set a
    /// moment ago, else the looper's.
    fn level_of(&self, n: usize, l: &Status) -> f32 {
        match self.levelled.get(n).copied().flatten() {
            Some((v, at)) if at.elapsed() < Duration::from_millis(400) => v,
            _ => l.level,
        }
    }

    fn refresh_qlinks(&mut self, ui: &mut Ui, v: &LooperView) {
        let loop_card = |n: usize| {
            let l = &v.loops[n];
            let name = format!("Loop {}", n + 1);
            let mut c = if l.phase == Phase::Empty {
                QLinkCard::new(&name, "empty", "-", 0.0)
            } else {
                let level = self.level_of(n, l);
                QLinkCard::new(&name, "level", &level_text(level), level_frac(level))
            };
            c.color = Some(LOOP_COLOURS[n % LOOP_COLOURS.len()]);
            c
        };
        let cards = match self.bank {
            LoopBank::First => [loop_card(0), loop_card(1), loop_card(2), loop_card(3)],
            LoopBank::Second => [loop_card(4), loop_card(5), loop_card(6), loop_card(7)],
            LoopBank::Picked => {
                let n = v.selected;
                let loop_scope = format!("Loop {}", n + 1);
                let s = &v.status;
                let layer = self.layers.get(self.layer).copied().flatten();
                let scope = self.layer_name(self.layer);
                let (c1, c2) = match layer {
                    Some(l) => {
                        let mut pan =
                            QLinkCard::new("Pan", &scope, &pan_text(l.pan), (l.pan + 1.0) / 2.0);
                        pan.bipolar = true;
                        (
                            QLinkCard::new(
                                "Layer level",
                                &scope,
                                &level_text(l.level),
                                level_frac(l.level),
                            ),
                            pan,
                        )
                    }
                    None => {
                        let mut pan = QLinkCard::new("Pan", "No layer", "-", 0.5);
                        pan.bipolar = true;
                        (QLinkCard::new("Layer level", "No layer", "-", 0.0), pan)
                    }
                };
                let c0 = if s.phase == Phase::Empty {
                    QLinkCard::new("Feedback", &loop_scope, "-", 0.0)
                } else {
                    QLinkCard::new(
                        "Feedback",
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
                [c0, c1, c2, c3]
            }
        };
        let title = match self.bank {
            LoopBank::Picked => format!("Loop {}", v.selected + 1),
            b => b.name().to_string(),
        };
        if let Some(q) = ui.get_mut::<QLinkColumn>(QLINKS) {
            if q.cards != cards || q.title != title {
                q.cards = cards;
                q.title = title;
            }
        }
    }

    /// The deck: each loop's channel, the clock and the memory.
    fn refresh_deck(&mut self, ui: &mut Ui, v: &LooperView) {
        let t = &self.transport;
        let beats = usize::from(t.meter.beats.max(1));
        let bar_ticks = mpfree_transport::time::PPQ as i64 * beats as i64;
        let running = matches!(t.state, State::Playing | State::Recording);
        let pos = t.position.max(0);
        let beat = (pos / mpfree_transport::time::PPQ as i64) as usize % beats;
        let bar = pos / bar_ticks.max(1);
        let head = DeckHead {
            beat: running.then_some((beat, beats)),
            clock: match t.state {
                State::Stopped => "Stopped".to_string(),
                State::CountIn => "Count-in".to_string(),
                _ => format!("Bar {} · beat {}", bar + 1, beat + 1),
            },
            tempo: format!("{:.0} BPM", t.bpm),
            memory: memory(v),
            pads: self.pads_keys,
        };
        // An armed loop starts on the next bar line (on the bar PLAY
        // starts while stopped).
        let next_bar = running.then_some(bar + 2);
        let foot = source_word(v).to_string();
        let bar_frames = v.frames_per_beat * f64::from(v.beats_per_bar.max(1));
        let channels: Vec<Channel> = (0..MAX_LOOPS)
            .map(|n| {
                let l = &v.loops[n];
                let bars = bars_of(l.ticks, v.beats_per_bar);
                let length = if bars == 0 || l.phase == Phase::Empty {
                    String::new()
                } else {
                    bars_word(bars)
                };
                let state = RingState::of(l, next_bar);
                let level = self.level_of(n, l);
                let (position, ring_bars, centre) = match state {
                    RingState::Empty => (0.0, 0, ("+".to_string(), String::new())),
                    RingState::Waiting(b) => (
                        0.0,
                        0,
                        match b {
                            Some(b) => (b.to_string(), "REC on bar".to_string()),
                            None => ("▶".to_string(), "REC on PLAY".to_string()),
                        },
                    ),
                    RingState::Recording => {
                        let done = l.recorded as f64 / bar_frames.max(1.0);
                        match l.bars {
                            Some(b) => (
                                (done / f64::from(b.max(1))).clamp(0.0, 1.0) as f32,
                                b as usize,
                                (
                                    ((done.floor() as u32).min(b.max(1) - 1) + 1).to_string(),
                                    format!("of {b}"),
                                ),
                            ),
                            None => (
                                done.fract() as f32,
                                1,
                                ((done.floor() as u64 + 1).to_string(), "REC".to_string()),
                            ),
                        }
                    }
                    RingState::Off => (l.position, bars, ("off".to_string(), String::new())),
                    _ => {
                        let at = ((l.position * bars as f32) as usize).min(bars.saturating_sub(1));
                        (
                            l.position,
                            bars,
                            ((at + 1).to_string(), format!("of {bars}")),
                        )
                    }
                };
                let wave = if state.has_sound() {
                    v.ring_waves[n].to_vec()
                } else {
                    Vec::new()
                };
                let meter = match state {
                    RingState::Playing | RingState::Overdubbing if !wave.is_empty() => {
                        let i = ((position * wave.len() as f32) as usize).min(wave.len() - 1);
                        level_frac(f32::from(wave[i]) / 255.0 * level)
                    }
                    _ => 0.0,
                };
                Channel {
                    name: format!("Loop {}", n + 1),
                    length,
                    colour: LOOP_COLOURS[n % LOOP_COLOURS.len()],
                    state,
                    position,
                    bars: ring_bars,
                    centre,
                    wave,
                    level,
                    meter: (meter * 20.0).round() / 20.0,
                    foot: if state.has_sound() {
                        String::new()
                    } else {
                        foot.clone()
                    },
                }
            })
            .collect();
        if let Some(d) = ui.get_mut::<LoopDeck>(DECK) {
            if d.head != head || d.channels != channels || d.picked != v.selected {
                d.head = head;
                d.channels = channels;
                d.picked = v.selected;
            }
        }
    }

    fn refresh_editor(&mut self, ui: &mut Ui, v: &LooperView) {
        let s = &v.status;
        let n = v.selected;
        let empty = s.phase == Phase::Empty;
        let has_sound = matches!(s.phase, Phase::Playing | Phase::Overdubbing);
        let title = format!("Loop {}", n + 1);
        let source = source_word(v);
        let records = match (v.resample, v.source) {
            (true, _) => "records the other loops",
            (false, LoopSource::Pads) => "records the pads",
            (false, LoopSource::Line) => "records the line input",
        };
        let sub = {
            let mut parts = Vec::new();
            if let Some(p) = self.place(n) {
                parts.push(format!("{} · {}", p.track_name, p.row));
            }
            match s.phase {
                Phase::Empty => parts.push(format!("empty · {records}")),
                Phase::Armed => parts.push(format!("waits for the bar line · {records}")),
                Phase::Recording => parts.push("recording the first pass".to_string()),
                _ => {
                    let bars = bars_of(s.ticks, v.beats_per_bar);
                    if bars > 0 {
                        parts.push(bars_word(bars));
                    }
                    parts.push(format!(
                        "{} layer{}",
                        s.layers,
                        if s.layers == 1 { "" } else { "s" }
                    ));
                }
            }
            parts.join(" · ")
        };
        let input = (source.to_string(), input_frac(self.hold));
        let names: Vec<String> = (0..s.layers)
            .map(|k| self.names[n].get(k).cloned().unwrap_or_default())
            .collect();
        let layers: Vec<LayerRow> = (0..s.layers)
            .map(|k| {
                let info = self.layers.get(k).copied().flatten();
                // A layer dragged in from another loop says where from.
                let from = info.and_then(|l| {
                    v.history.done().iter().find_map(|s| match *s {
                        Step::Added(id, o) if id == l.id => Some(o),
                        _ => None,
                    })
                });
                let what = match from {
                    Some(o) => format!("from loop {}", o + 1),
                    None if k == 0 => "first".to_string(),
                    None => format!("dub {k}"),
                };
                let how = match info {
                    Some(l) if l.recording => "recording".to_string(),
                    Some(l) if l.muted => "muted".to_string(),
                    // Feedback has turned it down under the overdubs since.
                    Some(l) if l.faded < 0.995 => format!("at {:.0} %", 100.0 * l.faded),
                    Some(l) => level_text(l.level),
                    None => String::new(),
                };
                let tag = if how.is_empty() {
                    what
                } else {
                    format!("{what} · {how}")
                };
                LayerRow {
                    name: names
                        .get(k)
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| format!("Layer {}", k + 1)),
                    tag,
                    colour: LAYER_COLOURS[k % LAYER_COLOURS.len()],
                    muted: info.is_some_and(|l| l.muted),
                    solo: info.is_some_and(|l| l.solo),
                    heard: info.is_none_or(|l| l.audible),
                    level: info.map_or(1.0, |l| l.level),
                    pan: info.map_or(0.0, |l| l.pan),
                    faded: info.map_or(1.0, |l| l.faded),
                    recording: info.is_some_and(|l| l.recording),
                }
            })
            .collect();
        let h = &v.history;
        let strip = if has_sound {
            let layer_colour = |id: u8| {
                v.layers
                    .iter()
                    .position(|l| l.is_some_and(|l| l.id == id))
                    .map(|k| LAYER_COLOURS[k % LAYER_COLOURS.len()])
            };
            let grey = LOOP_COLOURS[7];
            let colour = |s: &Step| match *s {
                Step::Overdub(id) | Step::Added(id, _) => layer_colour(id).unwrap_or(grey),
                _ => grey,
            };
            let mut chips: Vec<Chip> = h
                .done()
                .iter()
                .map(|s| Chip {
                    colour: colour(s),
                    kind: ChipKind::Done,
                })
                .collect();
            if s.phase == Phase::Overdubbing {
                chips.push(Chip {
                    colour: LAYER_COLOURS[s.layers.saturating_sub(1) % LAYER_COLOURS.len()],
                    kind: ChipKind::Recording,
                });
            }
            chips.extend(h.redo().iter().map(|s| Chip {
                colour: colour(s),
                kind: ChipKind::Undone,
            }));
            let reversed = h
                .done()
                .iter()
                .filter(|s| matches!(s, Step::Edit(Edit::Reverse)))
                .count()
                % 2
                == 1;
            let merge = match s.merging {
                Some(f) => (format!("Merging {:.0} %", 100.0 * f), false),
                None => ("Merge".to_string(), editable(s) && s.layers > 1),
            };
            Strip::Plays {
                feedback: s.fade_old,
                reversed,
                length: bars_word(bars_of(s.ticks, v.beats_per_bar)),
                edits: editable(s),
                chips,
                undo: !h.done().is_empty() || s.phase == Phase::Overdubbing,
                redo: !h.redo().is_empty(),
                merge,
            }
        } else {
            Strip::Records {
                source: source_index(v),
                monitor: (!v.resample && v.source == LoopSource::Line && v.line_on)
                    .then_some(v.monitor),
                bars: BAR_CHOICES.iter().position(|&b| b == s.bars).unwrap_or(0) as u8,
                then: match s.after {
                    After::Play => 0,
                    After::Overdub => 1,
                },
            }
        };
        let empty_look = (layers.is_empty()).then(|| EmptyLook {
            title: format!("INPUT · {}", source.to_uppercase()),
            says: match s.phase {
                Phase::Armed => format!("Loop {} records from the next bar line", n + 1),
                _ => format!("Tap Loop {}'s ring or press REC: it {records}", n + 1),
            },
            waits: match s.phase {
                Phase::Armed => Some(match self.transport.state {
                    State::Playing | State::Recording => {
                        let beats = i64::from(self.transport.meter.beats.max(1));
                        let bar = self.transport.position.max(0)
                            / (mpfree_transport::time::PPQ as i64 * beats);
                        format!("REC starts on bar {}", bar + 2)
                    }
                    _ => "REC starts when PLAY does".to_string(),
                }),
                _ => None,
            },
            input: self.input.iter().copied().collect(),
        });
        let history = self
            .history_open
            .then(|| history_list(h, &self.layers, v.rate));
        let trim_ticks = (editable(s) && s.ticks > 0).then_some(s.ticks);
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
        let send_built = [false, false, false, self.place(n).is_some()];
        let colour = LOOP_COLOURS[n % LOOP_COLOURS.len()];
        let r = ui.rect_on(LOOPER_PAGE, EDITOR);
        if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
            if e.title != title
                || e.sub != sub
                || e.colour != colour
                || e.input != input
                || e.has_sound != has_sound
                || e.layers != layers
                || e.empty != empty_look
                || e.position != position
                || e.bars != bars
                || e.picked != picked
                || e.strip != strip
                || e.history() != history.as_ref()
                || e.trim_ticks != trim_ticks
                || e.send_built != send_built
            {
                e.title = title;
                e.sub = sub;
                e.colour = colour;
                e.input = input;
                e.has_sound = has_sound;
                e.layers = layers;
                e.empty = empty_look;
                e.position = position;
                e.bars = bars;
                e.picked = picked;
                e.strip = strip;
                e.set_history(history);
                e.trim_ticks = trim_ticks;
                e.send_built = send_built;
                if !has_sound && e.send_open() {
                    e.close_send();
                }
                if let Some(r) = r {
                    e.follow_in(r);
                }
            }
        }
    }

    pub fn update(&mut self, ui: &mut Ui, v: &LooperView, _settings: &Settings) {
        let mut v = v.clone();
        v.selected = v.selected.min(MAX_LOOPS - 1);
        v.loops[v.selected] = v.status;
        self.follow_takes(&v);
        if self.selected != v.selected {
            self.layer = 0;
            self.history_open = false;
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.close_send();
            }
        }
        self.selected = v.selected;
        // A Q-Link turned a moment ago: keep what it set until the looper
        // has caught up. Only the values it sets, on the same loop: the
        // layers themselves come from the looper, so a layer an overdub
        // adds, or one a move or delete takes away, shows at once, and a
        // lane's × names the layer the looper has there. (Found 16 Nov by
        // the Looper smoke test in a release build: × on a new overdub,
        // within 400 ms of a turn, sent the id of a layer moved away
        // before, and nothing was deleted.)
        let fresh = self
            .turned
            .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
            && v.selected == self.view.selected;
        if fresh {
            for l in v.layers.iter_mut().flatten() {
                if let Some(set) = self.view.layers.iter().flatten().find(|s| s.id == l.id) {
                    l.level = set.level;
                    l.pan = set.pan;
                }
            }
            v.status.fade_old = self.view.status.fade_old;
            v.status.seam = self.view.status.seam;
            v.loops[v.selected] = v.status;
        }
        self.layers = v.layers;
        self.layer = self.layer.min(v.status.layers.saturating_sub(1));
        let v = &v;
        // The input's peak, held for about 2 s of updates, and its level
        // over the last few seconds.
        self.hold_age += 1;
        let peak = v.meter_db[0].max(v.meter_db[1]);
        if self.hold_age > 8 || peak > self.hold {
            self.hold = peak;
            self.hold_age = 0;
        }
        self.input_age += 1;
        if self.input_age >= INPUT_EVERY {
            self.input_age = 0;
            if self.input.len() >= INPUT_HISTORY {
                self.input.pop_front();
            }
            self.input
                .push_back((input_frac(peak) * 16.0).round() / 16.0);
        }
        self.refresh_deck(ui, v);
        self.refresh_editor(ui, v);
        self.refresh_qlinks(ui, v);
        let drops = self.drop_spots(ui, v);
        if ui
            .get::<LoopEditor>(EDITOR)
            .is_some_and(|e| e.drops != drops)
        {
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.drops = drops;
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
            } else {
                format!("{} · ", bars_word(n_bars))
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
        // On a Loop track, the loop's place first: "Loops B · Loop 2 ·
        // Verse · 4 bars · 3 layers".
        let (right, right_wide) = match self.place(v.selected) {
            Some(p) if !empty => {
                let at = p.full(v.selected);
                (format!("{at} · {right}"), format!("{at} · {right_wide}"))
            }
            _ => (right, right_wide),
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
        let note = if empty && !self.rec_loop {
            // The notes are armed: REC records them, not this loop.
            "Empty: REC records the notes now. Open the Loop view to record a loop here".to_string()
        } else if empty {
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

/// What the Play strip's Record button does now.
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

/// What the looper records from, as the input chip and an empty ring's
/// foot say it.
fn source_word(v: &LooperView) -> &'static str {
    match (v.resample, v.source) {
        (true, _) => "Loops",
        (false, LoopSource::Pads) => "Pads",
        (false, LoopSource::Line) => "Line in",
    }
}

/// Records from, as the panel's choices count it: 0 Pads, 1 Line in, 2
/// Loops.
pub fn source_index(v: &LooperView) -> u8 {
    match (v.resample, v.source) {
        (true, _) => 2,
        (false, LoopSource::Pads) => 0,
        (false, LoopSource::Line) => 1,
    }
}

/// "1 bar", "4 bars".
fn bars_word(bars: usize) -> String {
    format!("{bars} bar{}", if bars == 1 { "" } else { "s" })
}

/// The memory every loop shares: "37 s of 5 min", and how full, 0 to 1.
fn memory(v: &LooperView) -> (String, f32) {
    let rate = f64::from(v.rate.max(1));
    (
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
    )
}

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

/// Bars in a loop of `ticks`, rounded (0 before it has a length).
pub(crate) fn bars_of(ticks: i64, beats_per_bar: u32) -> usize {
    if ticks <= 0 {
        return 0;
    }
    (ticks as f64 / (960.0 * f64::from(beats_per_bar.max(1))))
        .round()
        .max(1.0) as usize
}

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
    use crate::looper_deck::RingState;
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
        s.on = true;
        s.position = 0.3;
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
    fn the_page_fits_the_deck_four_lanes_and_the_q_links() {
        let (ui, _) = page();
        let deck = ui.rect_of(DECK).unwrap();
        let editor = ui.rect_of(EDITOR).unwrap();
        let q = ui.rect_of(QLINKS).unwrap();
        assert_eq!((deck.y, deck.w, deck.h), (84, 1116, 240), "{deck:?}");
        assert_eq!((editor.y, editor.h), (336, 414), "{editor:?}");
        assert_eq!(q.w, 128);
        let mut e = LoopEditor::default();
        e.layers = vec![LayerRow::default(); 6];
        let lanes = e.lane_rects(LoopEditor::parts(editor).lanes);
        assert_eq!(lanes.len(), 4, "four lanes show");
        assert!(lanes.iter().all(|(_, r)| r.h >= 60), "{lanes:?}");
    }

    #[test]
    fn the_deck_shows_every_loop_and_the_picked_loops_lanes() {
        let (mut ui, mut page) = page();
        page.update(&mut ui, &two_loops(), &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        let rows: Vec<(&str, &str, RingState)> = d
            .channels
            .iter()
            .map(|c| (c.name.as_str(), c.length.as_str(), c.state))
            .collect();
        assert_eq!(rows.len(), 8, "every loop, always");
        assert_eq!(rows[0], ("Loop 1", "4 bars", RingState::Playing));
        assert_eq!(rows[1], ("Loop 2", "2 bars", RingState::Muted));
        assert_eq!(rows[2], ("Loop 3", "", RingState::Empty));
        // Loop 1 is 30 % through: in its second bar of four.
        assert_eq!(d.channels[0].centre, ("2".to_string(), "of 4".to_string()));
        assert_eq!(d.channels[0].bars, 4);
        assert_eq!(d.channels[2].centre.0, "+");
        assert_eq!(d.channels[2].foot, "Pads", "what an empty one records");
        assert_eq!(d.picked, 0);
        assert_eq!(d.head.memory.0, "16 s of 1 min");
        assert!((d.head.memory.1 - 16.0 / 60.0).abs() < 1e-4);
        assert_eq!(d.head.clock, "Stopped");
        // Above a minute, minutes and seconds, and the size in minutes.
        assert_eq!(super::memory_time(102.4, false), "1:42");
        assert_eq!(super::memory_time(300.4, true), "5 min");
        assert_eq!(super::memory_time(1200.9, true), "20 min");
        assert_eq!(super::memory_time(59.4, true), "59 s");
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.title, "Loop 1");
        assert_eq!(e.sub, "4 bars · 3 layers");
        assert_eq!(e.input.0, "Pads");
        assert!(e.has_sound);
        let tags: Vec<&str> = e.layers.iter().map(|l| l.tag.as_str()).collect();
        assert_eq!(tags, ["first · 0.0 dB", "dub 1 · 0.0 dB", "dub 2 · muted"]);
        assert!(e.layers[2].muted && !e.layers[2].heard && e.layers[0].heard);
        let Strip::Plays {
            length,
            edits,
            chips,
            undo,
            merge,
            ..
        } = &e.strip
        else {
            panic!("a loop that plays shows how it can change");
        };
        assert_eq!((length.as_str(), *edits, *undo), ("4 bars", true, false));
        assert!(chips.is_empty() && merge.1);
        assert_eq!(e.send_built, [false, false, false, false]);
        // An empty loop picked: how it will record.
        let mut v = two_loops();
        v.selected = 5;
        v.status = v.loops[5];
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.title, "Loop 6");
        assert_eq!(e.sub, "empty · records the pads");
        assert!(e.layers.is_empty() && !e.has_sound);
        let look = e.empty.as_ref().unwrap();
        assert_eq!(look.title, "INPUT · PADS");
        assert!(look.says.contains("ring"), "{}", look.says);
        assert!(
            matches!(
                e.strip,
                Strip::Records {
                    source: 0,
                    monitor: None,
                    then: 0,
                    ..
                }
            ),
            "{:?}",
            e.strip
        );
        assert_eq!(ui.get::<LoopDeck>(DECK).unwrap().picked, 5);
        ui.paint(&mut SoftCanvas::new(1280, 800));
    }

    #[test]
    fn a_waiting_loop_says_the_bar_it_starts_on() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.selected = 4;
        v.status = v.loops[4];
        v.status.phase = Phase::Armed;
        v.status.bars = Some(2);
        v.source = LoopSource::Line;
        v.line_on = true;
        page.update(&mut ui, &v, &Settings::default());
        // Bar 12, beat 2.
        let t = TransportInfo {
            state: State::Playing,
            position: (11 * 4 + 1) * 960,
            bpm: 92.0,
            ..TransportInfo::default()
        };
        page.set_transport(&mut ui, t);
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert_eq!(d.channels[4].state, RingState::Waiting(Some(13)));
        assert_eq!(d.channels[4].centre.0, "13");
        assert_eq!(d.channels[4].foot, "Line in");
        assert_eq!(d.head.clock, "Bar 12 · beat 2");
        assert_eq!(d.head.beat, Some((1, 4)));
        assert_eq!(d.head.tempo, "92 BPM");
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let look = e.empty.as_ref().unwrap();
        assert_eq!(look.waits.as_deref(), Some("REC starts on bar 13"));
        assert_eq!(look.title, "INPUT · LINE IN");
        assert_eq!(
            e.strip,
            Strip::Records {
                source: 1,
                monitor: Some(false),
                bars: 2,
                then: 0
            },
            "the line in shows its monitor; 2 bars is the third length"
        );
        // Records from the loops.
        v.resample = true;
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.input.0, "Loops");
        assert!(matches!(
            e.strip,
            Strip::Records {
                source: 2,
                monitor: None,
                ..
            }
        ));
    }

    #[test]
    fn a_ring_shows_the_first_pass_recording_through_its_bars() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.selected = 2;
        v.status = v.loops[2];
        v.status.phase = Phase::Recording;
        v.status.bars = Some(4);
        let bar = v.frames_per_beat * 4.0;
        v.status.recorded = (bar * 2.5) as usize;
        page.update(&mut ui, &v, &Settings::default());
        let ch = &ui.get::<LoopDeck>(DECK).unwrap().channels[2];
        assert_eq!(ch.state, RingState::Recording);
        assert_eq!(ch.centre, ("3".to_string(), "of 4".to_string()));
        assert!((ch.position - 0.625).abs() < 0.01, "{}", ch.position);
        // Free: round each bar, counting them.
        v.status.bars = None;
        page.update(&mut ui, &v, &Settings::default());
        let ch = &ui.get::<LoopDeck>(DECK).unwrap().channels[2];
        assert_eq!(ch.centre, ("3".to_string(), "REC".to_string()));
        assert!((ch.position - 0.5).abs() < 0.01, "{}", ch.position);
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
        let lanes: Vec<&str> = e.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(lanes, names);
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
    fn the_undo_history_names_its_steps_and_opens_in_the_lanes() {
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
        let (mut ui, mut page) = page();
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(e.history().is_none());
        // The loop plays, so the ruler's grips trim it.
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
    fn the_controls_ask_for_the_looper_edits_while_the_loop_plays() {
        let v = two_loops();
        let mut s = v.status;
        assert_eq!(
            strip_command(Ctl::Half, &s),
            Some(Command::Edit(Edit::Half))
        );
        assert_eq!(
            strip_command(Ctl::Double, &s),
            Some(Command::Edit(Edit::Double))
        );
        assert_eq!(
            strip_command(Ctl::Reverse, &s),
            Some(Command::Edit(Edit::Reverse))
        );
        assert_eq!(strip_command(Ctl::Merge, &s), Some(Command::Merge));
        assert_eq!(
            strip_command(Ctl::Then(1), &s),
            Some(Command::SetAfter(After::Overdub))
        );
        assert_eq!(strip_command(Ctl::Undo, &s), None, "UNDO's own");
        // One layer has nothing to merge, and says so.
        s.layers = 1;
        assert_eq!(strip_command(Ctl::Merge, &s), None);
        assert!(why_faint(Ctl::Merge, &s).starts_with("Merge needs two layers"));
        // While overdubbing, or merging, no edit works.
        s.phase = Phase::Overdubbing;
        assert_eq!(strip_command(Ctl::Half, &s), None);
        assert!(why_faint(Ctl::Half, &s).contains("not while it overdubs"));
        s.phase = Phase::Playing;
        s.merging = Some(0.4);
        assert_eq!(strip_command(Ctl::Reverse, &s), None);
        assert_eq!(why_faint(Ctl::Reverse, &s), "Wait for the merge to finish");
        // The panel's Merge says how far it has got.
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.status.merging = Some(0.4);
        page.update(&mut ui, &v, &Settings::default());
        let Strip::Plays { merge, edits, .. } = &ui.get::<LoopEditor>(EDITOR).unwrap().strip else {
            panic!("plays");
        };
        assert_eq!(
            (merge.0.as_str(), merge.1, *edits),
            ("Merging 40 %", false, false)
        );
    }

    #[test]
    fn the_q_links_turn_the_loops_levels_then_the_picked_loop() {
        let (mut ui, mut page) = page();
        let v = two_loops();
        page.update(&mut ui, &v, &Settings::default());
        let card = |ui: &Ui, k: usize| {
            let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
            (q.cards[k].name.clone(), q.cards[k].value.clone())
        };
        assert_eq!(page.bank(), LoopBank::First);
        assert_eq!(card(&ui, 0), ("Loop 1".to_string(), "0.0 dB".to_string()));
        assert_eq!(card(&ui, 2), ("Loop 3".to_string(), "-".to_string()));
        // Q1: loop 1's level, half a dB a detent; the deck follows.
        let Some(LoopTurn::LoopLevel(0, l)) = page.qlink(&mut ui, 0, -4, false) else {
            panic!("Q1 turns loop 1's level");
        };
        assert!((l - 10f32.powf(-2.0 / 20.0)).abs() < 1e-4);
        assert_eq!(card(&ui, 0).1, "-2.0 dB");
        assert!((ui.get::<LoopDeck>(DECK).unwrap().channels[0].level - l).abs() < 1e-4);
        // The looper's next view (from before the turn) does not undo it.
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(card(&ui, 0).1, "-2.0 dB");
        // An empty loop has no level to turn.
        assert_eq!(page.qlink(&mut ui, 2, 1, false), None);
        // Q-LINK: loops 5 to 8, then the picked loop.
        assert_eq!(page.next_bank(&mut ui), LoopBank::Second);
        assert_eq!(card(&ui, 0).0, "Loop 5");
        assert_eq!(page.next_bank(&mut ui), LoopBank::Picked);
        page.pick_layer(&mut ui, 1);
        assert_eq!(ui.get::<LoopEditor>(EDITOR).unwrap().picked, 1);
        assert_eq!(card(&ui, 0), ("Feedback".to_string(), "100 %".to_string()));
        assert_eq!(
            card(&ui, 1),
            ("Layer level".to_string(), "0.0 dB".to_string())
        );
        // Q2: the layer's level; SHIFT in tenths.
        let Some(LoopTurn::Layer(1, LayerCommand::Level(l))) = page.qlink(&mut ui, 1, 2, false)
        else {
            panic!("Q2 turns the picked layer's level");
        };
        assert!((l - 10f32.powf(1.0 / 20.0)).abs() < 1e-4);
        page.qlink(&mut ui, 1, -3, true);
        assert_eq!(card(&ui, 1).1, "0.7 dB");
        // Q3: its pan.
        assert_eq!(
            page.qlink(&mut ui, 2, -6, false),
            Some(LoopTurn::Layer(1, LayerCommand::Pan(-0.12)))
        );
        assert_eq!(card(&ui, 2).1, "L 6");
        // Q1: Feedback, a percent a detent; Q4: the seam fade, a
        // millisecond a detent.
        let Some(LoopTurn::FadeOld(k)) = page.qlink(&mut ui, 0, -2, false) else {
            panic!("Q1 turns the loop's Feedback");
        };
        assert!((k - 0.98).abs() < 1e-4, "{k}");
        assert_eq!(card(&ui, 0).1, "98 %");
        let before = v.status.seam;
        let Some(LoopTurn::Seam(f)) = page.qlink(&mut ui, 3, 10, false) else {
            panic!("Q4 turns the loop's seam fade");
        };
        assert_eq!(f as usize, before + 441);
        assert_eq!(card(&ui, 3).1, seam_text(before + 441, 44_100));
        assert_eq!(page.qlink(&mut ui, 0, 0, false), None);
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(card(&ui, 1).1, "0.7 dB");
        assert_eq!(card(&ui, 2).1, "L 6");
        assert_eq!(page.layer_info(1).map(|l| l.id), Some(11));
        // And round to loops 1 to 4.
        assert_eq!(page.next_bank(&mut ui), LoopBank::First);
    }

    #[test]
    fn a_turn_holds_what_it_set_but_the_layers_follow_the_looper() {
        let (mut ui, mut page) = page();
        let v = two_loops();
        page.update(&mut ui, &v, &Settings::default());
        page.next_bank(&mut ui);
        page.next_bank(&mut ui);
        page.pick_layer(&mut ui, 0);
        page.qlink(&mut ui, 1, 2, false);
        let turned = 10f32.powf(1.0 / 20.0);
        // Within the hold, the looper's view: layer 11 moved to another
        // loop and an overdub added layer 13.
        let mut w = v.clone();
        w.layers = [None; MAX_LAYERS];
        for (k, id) in [10, 12, 13].into_iter().enumerate() {
            w.layers[k] = Some(layer(id));
        }
        page.update(&mut ui, &w, &Settings::default());
        let ids: Vec<u8> = (0..MAX_LAYERS)
            .filter_map(|k| page.layer_info(k).map(|l| l.id))
            .collect();
        assert_eq!(ids, [10, 12, 13], "each lane's × names the looper's layer");
        let level = page.layer_info(0).map_or(0.0, |l| l.level);
        assert!(
            (level - turned).abs() < 1e-4,
            "the turned level holds: {level}"
        );
        // Loop 2 picked: its layer 10 is its own, nothing held from loop 1.
        let mut x = w.clone();
        x.selected = 1;
        x.status = x.loops[1];
        x.layers = [None; MAX_LAYERS];
        x.layers[0] = Some(layer(10));
        page.update(&mut ui, &x, &Settings::default());
        assert_eq!(page.layer_info(0).map(|l| l.level), Some(1.0));
    }

    #[test]
    fn a_dragged_layer_goes_onto_a_loop_of_its_length_or_an_empty_one() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.loops[3] = v.status;
        v.loops[3].layers = 2;
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let spot = |n: usize| e.drops.iter().find(|d| d.to == DropTo::Loop(n)).cloned();
        assert!(spot(0).is_none(), "not its own loop");
        let two = spot(1).unwrap();
        assert!(
            two.refused
                .as_deref()
                .unwrap()
                .contains("2 bars and this layer 4 bars"),
            "{two:?}"
        );
        assert_eq!(spot(2).unwrap().refused, None, "an empty loop: its own");
        assert_eq!(spot(3).unwrap().refused, None, "as long");
        let cards = ui
            .get::<LoopDeck>(DECK)
            .unwrap()
            .cards(ui.rect_of(DECK).unwrap());
        assert_eq!(spot(2).unwrap().rect, cards[2]);
        let words = page.drop_words(spot(2).as_ref(), false);
        assert_eq!(words, "Move into Loop 3: a loop of its own");
        assert_eq!(page.drop_words(spot(3).as_ref(), true), "Copy to Loop 4");
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
