//! The Looper tab as an instrument (the looper redesign, approved by Jay
//! on 17 Nov): the deck of loops across the top ([`crate::looper_deck`]),
//! the picked loop under it at full width, the Q-Links on the right.
//!
//! The picked loop ([`LoopEditor`]) has a header (its colour, name, where
//! it lives, the input it records with a live meter, Send and Clear), a
//! ruler of its bars, its layers as lanes a fingertip tall, newest on
//! top, and under them the controls the moment needs ([`Strip`]): a loop
//! that plays shows how it can change (Feedback, Reverse, its length,
//! Undo, Redo and Merge with the history as chips), an empty one how it
//! will record (from the pads, the line input or the other loops, the
//! monitor, the length, what it does after). The rare settings live in
//! MENU.
//!
//! Each panel keeps what a touch asked for as [`LoopAct`]s and answers the
//! touch with a plain event; the page collects them with [`take_actions`]
//! and the app turns them into looper commands. The panels only draw what
//! they are given ([`crate::looper_page::LooperPage::update`] fills them).
//!
//! A lane's wave: a tap picks the layer (it gets an outline and its ×), a
//! swipe scrolls the lanes, and a press held still for [`LIFT_HOLD_S`]
//! lifts the layer to drag it onto a loop on the deck ([`LayerDrag`], drag
//! and drop as approved 14 Nov; Jay, 17 Nov: a swipe must scroll). A tap
//! on a lane's head mutes the layer, its S solos it. A finger resting on
//! a lane while the wheel turns sets the layer's level (SHIFT: its pan).
//! The ruler drags sideways to nudge the loop by 1/16 notes, and its grips
//! trim it.

use crate::looper_deck::{LoopDeck, DECK};
use crate::looping::{Edge, LoopWave, MoveAct, MoveStep, Side, POINT_BARS};
use mpfree_looper::{LayerCommand, Snap};
use mpfree_ui::draw::speaker_on;
use mpfree_ui::{
    Align, Color, EventKind, FontId, PaintCx, Rect, Shape, Target, Theme, Touch, Ui, Widget,
};
use std::cell::RefCell;
use std::sync::Arc;

/// The panels' widget names.
pub const EDITOR: &str = "leditor";
pub const QLINKS: &str = "lqlinks";
/// Snap anyway and Not now, floated at the hint line's end while a snap
/// asks ([`AskButtons`]).
pub const ASK: &str = "lask";

/// The loops' colours, loop 1 first: none red, amber or orange, which
/// mean recording and overdubbing.
pub const LOOP_COLOURS: [Color; 8] = [
    Color::rgb(0x6EA8FE),
    Color::rgb(0x3DDC97),
    Color::rgb(0xD46CFF),
    Color::rgb(0x4FD1E8),
    Color::rgb(0xF26BA8),
    Color::rgb(0xA3E05B),
    Color::rgb(0xB0A4FF),
    Color::rgb(0xA0A8B8),
];

/// A control of the picked loop's panel: the header's, and the strip's
/// under the lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctl {
    /// The deck's Records from chip (a tap changes the input).
    Input,
    /// The header's Listen: the picked loop alone while the music is
    /// stopped (again: stop).
    Listen,
    /// A free loop's header, in Listen's place: Stop the free loops while
    /// they play, Play while they are stopped (again while it waits for
    /// the song's bar line: call it off).
    FreeGo,
    Send,
    Clear,
    /// A loop that plays.
    Feedback,
    /// Overdubs: 0 Add, 1 Replace, 2 Once.
    Overdubs(u8),
    Reverse,
    /// Speed: 0 half, 1 as recorded, 2 double ([`SPEED_CHOICES`]).
    Speed(u8),
    Half,
    Double,
    Undo,
    /// The history's chips: a tap opens the undo history in the lanes.
    History,
    Redo,
    Merge,
    /// An empty loop: Records from (0 Pads, 1 Line in, 2 Loops).
    Source(u8),
    /// Sync: 0 Song, 1 Free (the loop keeps its own time on the free
    /// clock).
    Sync(u8),
    Monitor,
    /// Length: 0 Pattern, 1 Until tap, then 2 to 5: 1, 2, 4 and 8 bars.
    Bars(u8),
    /// Starts: 0 On the bar, 1 When I play.
    Starts(u8),
    /// Then: 0 Play, 1 Overdub.
    Then(u8),
}

/// The Starts choices: on the next bar line, or when you play (the first
/// pad hit, or the line input's first note).
pub const STARTS_CHOICES: [&str; 2] = ["On the bar", "When I play"];

/// The Sync choices: the loop keeps time with the song, or on its own
/// with the other free loops (the free clock).
pub const SYNC_CHOICES: [&str; 2] = ["Song", "Free"];

/// The Length choices as the strip names them, [`BAR_CHOICES`] in order:
/// Until tap records until you tap again (free while nothing plays).
pub const LENGTH_CHOICES: [&str; 6] = ["Pattern", "Until tap", "1", "2", "4", "8 bars"];

/// The Overdubs choices: on top until Overdub again, in place of what was
/// there, or one pass.
pub const OVERDUB_CHOICES: [&str; 3] = ["Add", "Replace", "Once"];

/// The Speed choices, like tape: half (an octave down, twice the bars),
/// as recorded, double.
pub const SPEED_CHOICES: [&str; 3] = ["½×", "1×", "2×"];

/// Merge's width at the strip's right end.
const MERGE_W: i32 = 100;

/// The Length choices' bars: the pattern's length
/// ([`crate::looping::PATTERN_BARS`], the default), free (None), or a
/// number of bars.
pub const BAR_CHOICES: [Option<u32>; 6] = [
    Some(crate::looping::PATTERN_BARS),
    None,
    Some(1),
    Some(2),
    Some(4),
    Some(8),
];

/// What a touch on the Looper tab asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopAct {
    /// Pick loop `n` (its name on the deck).
    Pick(usize),
    /// Loop `n`'s ring: its next step (record, play, overdub, play; armed,
    /// cancel; muted, unmute).
    Tap(usize),
    /// Loop `n`'s speaker: mute it or let it play, without picking it. A
    /// muted loop stays silent when PLAY starts the others (Jay, 13 Nov).
    Mute(usize),
    /// Loop `n`'s S: solo it or let go, without picking it.
    Solo(usize),
    /// Clear loop `n`: every layer, the first pass too (its ring held, or
    /// Clear on the picked loop).
    Clear(usize),
    /// Loop `n`'s level, 0 to 2.
    Level(usize, f32),
    /// The deck's Pads switch: the pads as loop keys, or playing again.
    PadKeys,
    /// Capture (the deck's, What you played's): the last bars heard
    /// become a loop.
    Capture,
    /// What you played's window moved or resized: the bars Capture takes,
    /// ending on the bar line that starts bar `.0` (None: the newest, so
    /// it keeps up), `.1` of them.
    CaptureSpan(Option<i64>, u32),
    /// Show layer `k` (play order) on the Q-Links: it is picked.
    Layer(usize),
    /// A control of layer `k` (play order).
    LayerCmd(usize, LayerCommand),
    /// Layer `k`'s ▶ (play order): that layer alone while the music is
    /// stopped (again: stop).
    LayerListen(usize),
    /// One of the panel's controls tapped.
    Strip(Ctl),
    /// A control drawn faint (it can't work now) was tapped: the screen
    /// says why in place.
    Faint(Ctl),
    /// Feedback turned: the share of the older layers kept a pass.
    Feedback(f32),
    /// The ruler dragged: move the loop this many 1/16 notes.
    Nudge(i32),
    /// Go to row `r` of the undo history ([`HistoryList::rows`]): back or
    /// forward to just after that step.
    Step(usize),
    /// Keep ticks `from..to` of the picked loop: its trim grips.
    Trim(i64, i64),
    /// Send's choice `i` ([`SEND_CHOICES`]).
    Send(usize),
    /// A Send choice not built yet was tapped: the screen says so.
    FaintSend(usize),
    /// Layer `k` (play order) of the picked loop dragged onto a loop (an
    /// empty one: a loop of its own; SHIFT held as it lifts: a copy).
    DropLayer(usize, DropTo),
    /// A layer let go on a loop that can't take it: the screen says why
    /// ([`DropSpot::refused`]).
    NoDrop(DropTo),
    /// Smart loop end's chip's button, or the loop point view's Read as.
    Chip(ChipAct),
    /// The loop point view's Snap, Use my tap, Keep my tap or Keep the
    /// snap: that side, and whether the chip is put away there.
    Side(Side, bool),
    /// The loop point view's A/B, on or off.
    Ab(bool),
    /// The loop point view's Move start / Move end: a finger or a hit on
    /// a window, a step, the step's size, Move or Cancel.
    Move(MoveAct),
    /// The loop point view opened (true) or closed: the chip's words, ×.
    PointView(bool),
    /// The hint line's Snap anyway (true) or Not now ([`AskButtons`]).
    Answer(bool),
}

/// What smart loop end's chip's button does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChipAct {
    /// Snap, Use my tap, Use my press: that side.
    Side(Side),
    /// Read as: a snap of its own.
    Snap(Snap),
}

/// Smart loop end's chip on the picked loop's header, after its length,
/// while there is something to do: what it found or did, and its action
/// as a button inside it (Jay's OK on the drawings, 22 Nov). A tap on
/// the words opens the loop point view.
#[derive(Clone, Debug, PartialEq)]
pub struct SmartChip {
    /// "48 ms late", "Snapped 48 ms earlier", "Close call".
    pub words: String,
    /// The button's label, what it does, and whether it is filled with
    /// the accent (Snap) or outlined (Use my tap, Use my press, Read as).
    pub action: (String, ChipAct, bool),
}

/// What a button under the loop point view's windows does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    /// Snap, Use my tap, Keep my tap or Keep the snap: that side, and
    /// whether the chip is put away there.
    Side(Side, bool),
    /// Move: the move set up lands on the loop's next downbeat.
    Move,
    /// Cancel: the move set up is put away.
    Cancel,
}

/// One side of the loop point view: the start's or the end's window.
#[derive(Clone, Debug, PartialEq)]
pub struct PointPane {
    /// "Start", "End".
    pub title: String,
    /// "your first hit came 80 ms after REC", "on your tap".
    pub note: String,
    /// The audio's peaks, 0 to 255, left to right.
    pub peaks: [u8; POINT_BARS],
    /// Where the loop is now, 0 to 1 across, and whether the shade is
    /// before it (the start) or after it (the end): outside the loop.
    pub edge: (f32, bool),
    /// Your press or tap (grey, dashed) and its label.
    pub you: (f32, String),
    /// Where smart loop end found or put it (the accent), its label, and
    /// how far that is from you ("−48 ms"), when it isn't you.
    pub found: Option<(f32, String, String)>,
    /// Where the window begins and how long it is, in frames from your
    /// press (the start's) or your tap (the end's): a finger on it puts
    /// the edge there (Move start / Move end).
    pub span: (i64, i64),
    /// The hits heard in it, 0 to 1 across: a tap by one puts the edge
    /// on it.
    pub hits: Vec<f32>,
    /// A move set up there: where, and how far that is from you.
    pub moving: Option<(f32, String)>,
}

/// The loop point view, in the lanes' place: the start and the end side
/// by side with your press and tap and what was found, what Snap makes
/// of it, the choice, A/B, and Read as.
#[derive(Clone, Debug, PartialEq)]
pub struct PointLook {
    /// "Now: your tap, 1 bar at 96.00 BPM".
    pub now: String,
    pub start: PointPane,
    pub end: PointPane,
    /// "Snap makes it 1 bar at 97.87 BPM, from the loop's next downbeat."
    pub line: String,
    /// The buttons, the first filled: label and what it does (Snap and
    /// Keep my tap; Use my tap and Keep the snap; Keep the snap and Keep
    /// my tap during A/B; Keep my tap alone when nothing was found; Move
    /// and Cancel while a move is set up).
    pub pick: Vec<(String, Pick)>,
    /// Move start / Move end's step, lit.
    pub step: MoveStep,
    /// A/B, on or off; None when there aren't two sides to swap.
    pub ab: Option<bool>,
    /// Read as: each choice's words and bars, the loop's bars now among
    /// them.
    pub read_as: Vec<(String, u32)>,
    pub bars_now: u32,
}

/// The loop point view's title rows: the title, ◀ ▶ and the note.
const TITLE_H: i32 = 30;
/// Where a window's ◀ ▶ start, after its title.
const NUDGE_X: i32 = 52;
/// A finger this near a hit on a loop point window puts the edge on it.
const HIT_REACH: i32 = 10;

/// A place across a loop point window, 0 to 1 over its peaks (as
/// [`point_pane`] draws them), at screen `x`.
fn point_frac(win: Rect, x: i32) -> f32 {
    let step = ((win.w - 26) / POINT_BARS as i32).max(1);
    let span = (step * POINT_BARS as i32).max(1);
    (x - (win.x + 13)) as f32 / span as f32
}

/// Where a finger at `x` on a loop point window puts the edge, in frames
/// from your press or tap: on a hit within reach, else where it is.
fn point_to(win: Rect, pane: &PointPane, x: i32) -> i64 {
    let step = ((win.w - 26) / POINT_BARS as i32).max(1);
    let span = (step * POINT_BARS as i32).max(1) as f32;
    let at = |f: f32| (f * span) as i32 + win.x + 13;
    let f = pane
        .hits
        .iter()
        .copied()
        .filter(|&h| (at(h) - x).abs() <= HIT_REACH)
        .min_by_key(|&h| (at(h) - x).abs())
        .unwrap_or_else(|| point_frac(win, x).clamp(0.0, 1.0));
    pane.span.0 + (f64::from(f) * pane.span.1 as f64).round() as i64
}

/// A finger held on a layer's lane moves this far before the lane lifts
/// out of the panel (less is a tap).
pub const LIFT_PX: i32 = 12;
/// How long a finger stays still on a layer's lane before it can lift out
/// (a press and hold); a finger that moves sooner swipes the lanes.
pub const LIFT_HOLD_S: f32 = 0.4;
/// A finger moves this far (either way) before the hold to swipe the lanes.
const SWIPE_PX: i32 = mpfree_ui::qlink::DRAG_DETENT;
/// The card a lifted layer shows under the finger.
const CARD_W: i32 = 280;
const CARD_H: i32 = 58;

/// Where a layer dragged out of the lanes can go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTo {
    /// Onto loop `n` (an empty one: the layer becomes a loop of its own).
    Loop(usize),
}

/// A place on the screen a dragged layer can be let go: what it is, where
/// (screen coordinates), and None if it takes the layer, else why not.
#[derive(Clone, Debug, PartialEq)]
pub struct DropSpot {
    pub to: DropTo,
    pub rect: Rect,
    pub refused: Option<String>,
}

/// A layer lifted out of the lanes, as the deck draws it over itself: the
/// card (its lane, where it is now, what letting go there does), where it
/// would land and whether that takes it, and the places that do, lit.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerDrag {
    pub row: LayerRow,
    pub card: Rect,
    /// The finger (or the loop the wheel points at): the panel under it
    /// draws the card.
    pub at: (i32, i32),
    /// "Move to Loop 2", "Loop 3 is 2 bars and this layer 4" ...
    pub says: String,
    pub landing: Option<(DropTo, bool)>,
    pub lit: Vec<DropTo>,
}

/// A layer's card while it is dragged: its colour, name and tag, and what
/// letting go now does, raised above the screen.
pub(crate) fn paint_card(cx: &mut PaintCx, d: &LayerDrag) {
    let t = cx.theme.clone();
    let r = d.card;
    cx.canvas
        .fill_round_rect(r.inset(-2), t.round(12), t.accent);
    cx.canvas.fill_round_rect(r, t.round(10), t.raised);
    let line = Rect::new(r.x + 12, r.y + 8, r.w - 24, 20);
    cx.canvas
        .fill_round_rect(Rect::new(line.x, line.y + 5, 10, 10), 2, d.row.colour);
    let nw = cx.measure(FontId::Bold, 15.0, &d.row.name);
    cx.text(
        FontId::Bold,
        15.0,
        Rect::new(line.x + 18, line.y, nw + 4, line.h),
        Align::Left,
        t.text,
        &d.row.name,
    );
    cx.text(
        FontId::Sans,
        12.0,
        Rect::new(line.x + 18 + nw + 10, line.y, line.w - nw - 28, line.h),
        Align::Left,
        t.text_faint,
        &d.row.tag,
    );
    let ink = match d.landing {
        Some((_, true)) => t.accent,
        Some((_, false)) => t.bad,
        None => t.text_dim,
    };
    cx.text(
        FontId::Bold,
        12.5,
        Rect::new(line.x, line.bottom() + 2, line.w, 18),
        Align::Left,
        ink,
        &d.says,
    );
}

/// The places a dragged layer can go, drawn over a panel: lit where it
/// would be taken, outlined bold where it lands (red where it would be
/// refused). `spots` are the drag's places in this panel.
pub(crate) fn paint_spots(cx: &mut PaintCx, d: &LayerDrag, spots: &[(DropTo, Rect)]) {
    let t = cx.theme.clone();
    for &(to, rr) in spots {
        match d.landing {
            Some((at, ok)) if at == to => {
                cx.canvas
                    .stroke_round_rect(rr, t.round(12), 3, if ok { t.accent } else { t.bad });
            }
            _ if d.lit.contains(&to) => cx.canvas.stroke_round_rect(rr, t.round(12), 2, t.accent),
            _ => {}
        }
    }
}

/// The fill under a place the dragged layer would land on now (painted
/// before the place's own words, so they stay readable).
pub(crate) fn landing_fill(t: &Theme, d: Option<&LayerDrag>, to: DropTo) -> Option<Color> {
    let (at, ok) = d?.landing?;
    (at == to).then(|| if ok { t.accent } else { t.bad }.mix(t.bg, 0.84))
}

/// The card fitted into panel `r` for a finger at `at`: as wide as fits,
/// above the finger (below it near the top) so the place it points at
/// stays in sight.
pub(crate) fn card_in(r: Rect, at: (i32, i32)) -> Rect {
    let w = CARD_W.min(r.w - 16);
    let x = (at.0 - w / 2).clamp(r.x + 8, r.x + r.w - 8 - w);
    let above = at.1 - 30 - CARD_H;
    let y = if above >= r.y + 8 { above } else { at.1 + 30 };
    Rect::new(x, y.min(r.y + r.h - 8 - CARD_H), w, CARD_H)
}

/// Every [`LoopAct`] the Looper tab's panels have kept since the last call.
pub fn take_actions(ui: &mut Ui) -> Vec<LoopAct> {
    let mut acts = Vec::new();
    if let Some(w) = ui.get_mut::<LoopDeck>(DECK) {
        acts.append(&mut w.take());
    }
    if let Some(w) = ui.get_mut::<LoopEditor>(EDITOR) {
        acts.append(&mut w.acts);
    }
    if let Some(w) = ui.get_mut::<crate::loop_row::LoopRow>(crate::loop_row::LOOP_ROW) {
        acts.append(&mut w.take());
    }
    if let Some(w) = ui.get_mut::<AskButtons>(ASK) {
        acts.append(&mut w.take());
    }
    acts
}

// ------------------------------------------------------------- levels

/// The level bar's range: -24 to +6 dB.
const LEVEL_FLOOR_DB: f32 = -24.0;
const LEVEL_TOP_DB: f32 = 6.0;

/// Where a level (0 to 2) sits along a level bar, 0 to 1.
pub fn level_frac(level: f32) -> f32 {
    let db = 20.0 * level.max(1e-6).log10();
    ((db - LEVEL_FLOOR_DB) / (LEVEL_TOP_DB - LEVEL_FLOOR_DB)).clamp(0.0, 1.0)
}

/// The level at a place along a level bar (the inverse of [`level_frac`];
/// the bottom is silence).
pub(crate) fn level_of_frac(f: f32) -> f32 {
    if f <= 0.0 {
        return 0.0;
    }
    let db = LEVEL_FLOOR_DB + f.clamp(0.0, 1.0) * (LEVEL_TOP_DB - LEVEL_FLOOR_DB);
    10f32.powf(db / 20.0)
}

/// A level as the screen prints it: "-3.5 dB", "-inf dB".
pub fn level_text(level: f32) -> String {
    if level <= 1e-6 {
        "-inf dB".to_string()
    } else {
        let db = 20.0 * level.log10();
        format!("{db:.1} dB").replace("-0.0", "0.0")
    }
}

// ------------------------------------------------------------- the panel

/// One layer's lane.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerRow {
    /// Listen plays this layer alone now.
    pub listening: bool,
    /// "Drums", "Layer 2".
    pub name: String,
    /// "dub 5 · -2.0 dB", "first · at 49 %", "dub 3 · muted".
    pub tag: String,
    pub colour: Color,
    pub muted: bool,
    pub solo: bool,
    /// Heard: not muted, not left out by a solo.
    pub heard: bool,
    /// 0 to 2 (1 is as recorded).
    pub level: f32,
    /// -1 (left) to 1 (right).
    pub pan: f32,
    /// How far Feedback has turned it down, 0 to 1 (1: not at all).
    pub faded: f32,
    pub recording: bool,
}

impl Default for LayerRow {
    fn default() -> Self {
        LayerRow {
            name: String::new(),
            tag: String::new(),
            colour: LOOP_COLOURS[0],
            muted: false,
            solo: false,
            heard: true,
            level: 1.0,
            pan: 0.0,
            faded: 1.0,
            recording: false,
            listening: false,
        }
    }
}

/// The undo history as the panel lists it: "Start" (the first pass),
/// then each step, oldest first, and the row that is now. The rows after
/// it are the steps Redo would bring back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryList {
    pub rows: Vec<String>,
    pub now: usize,
}

/// One step of the picked loop's history as a chip: its colour, and how
/// it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chip {
    pub colour: Color,
    pub kind: ChipKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipKind {
    /// A step Undo takes back.
    Done,
    /// The overdub recording now (outlined amber).
    Recording,
    /// A step Redo brings back (hollow).
    Undone,
}

/// The controls under the lanes: what the moment needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Strip {
    /// A loop that plays (or overdubs): how it can change.
    Plays {
        /// Feedback: the share of the older layers kept a pass, 0.5 to 1.
        feedback: f32,
        /// How overdubs record: 0 Add, 1 Replace, 2 Once.
        overdubs: u8,
        reversed: bool,
        /// Into [`SPEED_CHOICES`]: 0 half, 1 as recorded, 2 double.
        speed: u8,
        /// "4 bars".
        length: String,
        /// ½, × 2 and Reverse work (the loop plays, nothing renders).
        edits: bool,
        chips: Vec<Chip>,
        undo: bool,
        redo: bool,
        /// Merge's label ("Merge", "Merging 40 %") and whether it works.
        merge: (String, bool),
    },
    /// An empty loop (or one recording its first pass): how it records.
    Records {
        /// 0 Pads, 1 Line in, 2 Loops.
        source: u8,
        /// Into [`SYNC_CHOICES`], and whether it can change: an empty
        /// loop's as last set (every empty loop records so), changeable;
        /// once the loop records, its own clock, fixed (Jay's review,
        /// 23 Nov, 5.2).
        sync: (u8, bool),
        /// The monitor switch, shown for the line input.
        monitor: Option<bool>,
        /// Into [`BAR_CHOICES`].
        bars: u8,
        /// Into [`STARTS_CHOICES`], and whether it can change (not for
        /// Loops: a resample starts on the bar).
        starts: (u8, bool),
        /// 0 Play, 1 Overdub.
        then: u8,
    },
}

impl Default for Strip {
    fn default() -> Self {
        Strip::Records {
            source: 0,
            sync: (0, true),
            monitor: None,
            bars: 0,
            starts: (0, true),
            then: 0,
        }
    }
}

/// While the picked loop has no layers, the lanes' space shows What you
/// played: the input's last 8 bars, and what Capture takes from it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmptyLook {
    /// "WHAT YOU PLAYED · LINE IN".
    pub title: String,
    /// What a tap on its ring or REC does now.
    pub says: String,
    /// "REC starts on bar 13", while it waits for its bar.
    pub waits: Option<String>,
    /// What the looper heard over the last 8 bars, oldest first, 0 to 1.
    pub input: Vec<f32>,
    /// The bar lines over it: each bar's number and where it starts, 0
    /// to 1.
    pub bars: Vec<(i64, f32)>,
    /// What Capture takes now: where it starts and ends (0 to 1), and
    /// its button's words ("Capture bars 10 and 11"); None when there is
    /// nothing to take.
    pub take: Option<(f32, f32, String)>,
    /// The bars it takes: the first one's number and how many.
    pub window: Option<(i64, u32)>,
    /// The bars the window grows and shrinks by (the pattern's length,
    /// `pattern` true; or the Length's bars; or 1).
    pub unit: u32,
    pub pattern: bool,
}

impl EmptyLook {
    /// Where bar `bar` starts on the wave (0 to 1), if its line shows; the
    /// bar after the newest line ends at "now".
    fn bar_x(&self, bar: i64) -> Option<f32> {
        let newest = self.bars.last()?.0;
        if bar == newest + 1 {
            return Some(1.0);
        }
        self.bars.iter().find(|b| b.0 == bar).map(|b| b.1)
    }

    /// The button's words for a window of `bars` from bar `first`, with
    /// how many patterns it holds.
    fn words(&self, first: i64, bars: u32) -> String {
        capture_words(first, bars, self.unit, self.pattern)
    }
}

/// What you played's button for `bars` from bar `first`: "Capture bars 5
/// to 10", and "· 3 × pattern" when it holds whole patterns of `unit`
/// bars, more than one.
pub fn capture_words(first: i64, bars: u32, unit: u32, pattern: bool) -> String {
    let w = format!("Capture {}", crate::looping::bars_words(first, bars));
    let unit = unit.max(1);
    match (pattern, bars / unit) {
        (true, k) if k > 1 && bars.is_multiple_of(unit) => format!("{w} · {k} × pattern"),
        _ => w,
    }
}

/// A finger on What you played's window: what it holds (the window, or
/// one of its ends), where it went down, the window then and as it is now
/// (first bar and bars), and whether it went down on the button.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowDrag {
    finger: u64,
    grip: Grip,
    x0: i32,
    from: (i64, u32),
    now: (i64, u32),
    button: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grip {
    Move,
    Start,
    End,
}

/// How near (pixels) to the window's end a finger takes that end.
const GRIP_REACH: i32 = 22;

/// The Capture button for a window from `a` to `b` (0 to 1) with its
/// words `label`: centred over it, as wide as its words need.
fn capture_button(lanes: Rect, a: f32, b: f32, label: &str) -> Rect {
    let w = empty_wave_rect(lanes);
    let mid = w.x + ((a + b) / 2.0 * w.w as f32) as i32;
    let bw = ((60.0 + 8.6 * label.chars().count() as f32) as i32)
        .max(200)
        .min(w.w);
    let x = (mid - bw / 2).clamp(w.x, w.right() - bw);
    Rect::new(x, w.y + w.h / 2 - 22, bw, 44)
}

/// What you played's window from `a` to `b` (0 to 1): the wave's height
/// over the bars it takes.
fn window_rect(lanes: Rect, a: f32, b: f32) -> Rect {
    let w = empty_wave_rect(lanes);
    let at = |f: f32| w.x + (f.clamp(0.0, 1.0) * w.w as f32) as i32;
    Rect::new(at(a), w.y, (at(b) - at(a)).max(1), w.h)
}

/// The window `from` (first bar, bars) after a finger on `grip` moved
/// `steps` bars: moved, it keeps its bars; an end moved, it grows or
/// shrinks by whole units (the pattern), the other end staying. Always
/// whole bars that have been played and still show: from the oldest bar
/// line to the newest.
fn window_after(e: &EmptyLook, grip: Grip, from: (i64, u32), steps: i64) -> (i64, u32) {
    let (Some(oldest), Some(newest)) = (e.bars.first().map(|b| b.0), e.bars.last().map(|b| b.0))
    else {
        return from;
    };
    let unit = i64::from(e.unit.max(1));
    let (first, bars) = (from.0, i64::from(from.1));
    let units = |n: i64| (n as f32 / unit as f32).round().max(1.0) as i64;
    let (f, n) = match grip {
        Grip::Move => ((first + steps).min(newest - bars).max(oldest), bars),
        Grip::Start => {
            let end = (first + bars).min(newest);
            let k = units(bars - steps).min((end - oldest) / unit);
            (end - k * unit, k * unit)
        }
        Grip::End => {
            let k = units(bars + steps).min((newest - first) / unit);
            (first, k * unit)
        }
    };
    if n < 1 || f < oldest || f + n > newest + 1 {
        return from;
    }
    (f, n as u32)
}

/// Where What you played draws the input, in the lanes' space.
fn empty_wave_rect(lanes: Rect) -> Rect {
    let inner = lanes.inset_xy(14, 10);
    Rect::new(inner.x, inner.y + 22, inner.w, inner.h - 22 - 46)
}

/// Send's choices: title and what it does, and whether it is built.
pub const SEND_CHOICES: [(&str, &str); 4] = [
    ("To a pad", "the loop as a sample on the Sample tab"),
    ("To an audio track", "the loop as a take on a track"),
    ("Save WAV", "one pass, as heard, on the stick"),
    ("Show in Clips", "its cell on the Arrange page"),
];
/// Send's Show in Clips.
pub const SEND_SHOW: usize = 3;

/// Records from's choices, as its picker shows them in the lanes' place
/// (the deck's Records from opens it): title and what it records. The
/// mixes follow them, from index 3 ([`LoopEditor::set_mixes`]).
pub const SOURCE_CHOICES: [(&str, &str); 3] = [
    ("Pads", "what the pads play, and clips on that sound"),
    ("Line in", "the INPUTS on the back of the MPC"),
    ("Loops", "the other loops as you hear them"),
];

/// Where the panel's parts are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorParts {
    pub head: Rect,
    /// Listen, left of Send.
    pub listen: Rect,
    pub send: Rect,
    pub clear: Rect,
    pub ruler: Rect,
    /// The lanes' space (the history, the empty look and Send's choices
    /// show here too).
    pub lanes: Rect,
    /// The layer map down the right, while more layers than fit.
    pub map: Rect,
    pub strip: Rect,
    /// The loop point view: the ruler's, the lanes' and the map's place.
    pub point: Rect,
}

/// The loop point view's parts ([`LoopEditor::point_parts`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PointParts {
    /// "LOOP POINT" and what the loop is now.
    pub head: Rect,
    /// ×: close the view.
    pub close: Rect,
    /// "STEP" and Move start / Move end's step sizes ([`MoveStep::ALL`]).
    pub step_label: Rect,
    pub steps: Vec<Rect>,
    /// Each window's ◀ and ▶: a step earlier and later (the start's,
    /// then the end's).
    pub nudge: [[Rect; 2]; 2],
    /// The start's and the end's titles and notes.
    pub titles: (Rect, Rect),
    /// The start's and the end's windows.
    pub start: Rect,
    pub end: Rect,
    /// What the other side makes of the loop.
    pub line: Rect,
    /// Snap and Keep my tap (or Use my tap and Keep the snap).
    pub pick: Vec<Rect>,
    /// A/B, when it shows.
    pub ab: Option<Rect>,
    /// The rule between A/B and Read as.
    pub rule: Rect,
    pub read_label: Rect,
    /// Read as's choices.
    pub read_as: Vec<Rect>,
}

/// A lane's parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneParts {
    pub head: Rect,
    /// ▶: the layer alone while the music is stopped.
    pub listen: Rect,
    pub solo: Rect,
    pub wave: Rect,
    pub delete: Rect,
}

/// A finger on a layer's lane ([`LoopEditor`]): the layer, where the
/// finger went down and is now, where it holds the card, how long it has
/// been held still, whether the hold is long enough to lift the lane,
/// whether it has lifted, and, once it moved before the hold, the first
/// lane shown when it went down (a swipe).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    finger: u64,
    k: usize,
    from: (i32, i32),
    at: (i32, i32),
    grip: (i32, i32),
    held: f32,
    ready: bool,
    lifted: bool,
    swipe: Option<usize>,
}

/// What the panel shows apart from the playhead.
#[derive(Clone, Debug, PartialEq)]
struct EditorLook {
    title: String,
    sub: String,
    colour: Color,
    listen: (bool, bool),
    free: Option<(bool, bool)>,
    has_sound: bool,
    wave: Option<usize>,
    bars: usize,
    layers: Vec<LayerRow>,
    picked: usize,
    top: usize,
    empty: Option<EmptyLook>,
    strip: Strip,
    send_open: bool,
    sources: Option<usize>,
    mixes: Vec<String>,
    history: Option<(HistoryList, usize)>,
    trim: Option<i64>,
    trimming: Option<(i64, i64)>,
    nudging: Option<i32>,
    window: Option<(i64, u32)>,
    feedback_held: bool,
    drag: Option<(usize, Rect, String, Option<DropSpot>)>,
    ready: Option<usize>,
    chip: Option<SmartChip>,
    point: Option<PointLook>,
}

/// The picked loop: header, ruler, lanes, layer map and the controls
/// under them.
pub struct LoopEditor {
    /// "Loop 1".
    pub title: String,
    /// "Loops A · Verse · 4 bars · 6 layers", "empty · records the pads".
    pub sub: String,
    pub colour: Color,
    /// Listen: whether it can play now (the loop has sound and the music
    /// is stopped), and whether it plays the whole loop now (lit: Stop).
    pub listen: (bool, bool),
    /// A free loop with sound: the free loops' Stop and Play in Listen's
    /// place. Whether they play (Stop), and whether a Play waits for the
    /// song's bar line (Play lit).
    pub free: Option<(bool, bool)>,
    /// Send and Clear work: the loop has sound.
    pub has_sound: bool,
    pub wave: Option<Arc<LoopWave>>,
    /// The playhead, 0 to 1.
    pub position: Option<f32>,
    pub bars: usize,
    /// The layers in play order (the first pass first): drawn newest on
    /// top.
    pub layers: Vec<LayerRow>,
    /// The layer on the Q-Links (play order).
    pub picked: usize,
    /// The lanes' space while the loop has no layers.
    pub empty: Option<EmptyLook>,
    /// The loop's length in ticks while it can be trimmed and nudged.
    pub trim_ticks: Option<i64>,
    pub strip: Strip,
    /// Send's built choices (the rest show faint).
    pub send_built: [bool; 4],
    /// Send's choices show in the lanes' place.
    send_open: bool,
    /// Records from's choices show in the lanes' place, the one in use
    /// (into [`SOURCE_CHOICES`], then the mixes) lit.
    sources: Option<usize>,
    /// Records from's mixes, by name: the master, the tracks and the
    /// submixes (the page's [`crate::looper_page::LooperPage::set_mixes`]).
    mixes: Vec<String>,
    /// A finger on a trim grip (true: the end), and the range kept so far.
    trimming: Option<(u64, bool, i64, i64)>,
    /// A finger on the ruler: where it went down and the 1/16 notes it
    /// has moved.
    nudging: Option<(u64, i32, i32)>,
    /// A finger on What you played's window (moving or resizing it).
    window_drag: Option<WindowDrag>,
    /// A finger on a loop point window (Move start / Move end): which
    /// edge it moves.
    point_drag: Option<(u64, Edge)>,
    /// A finger on the Feedback knob: where it went down and the value
    /// then.
    feedback: Option<(u64, i32, f32)>,
    /// The undo history, in the lanes' place while it is open.
    history: Option<HistoryList>,
    history_top: Option<usize>,
    history_press: Option<(u64, i32, usize, bool)>,
    /// The first lane shown (0: the newest layer).
    top: usize,
    /// The lanes were swiped while this layer was picked: they stay where
    /// the swipe left them until another layer is picked.
    swiped: Option<usize>,
    /// Where a dragged layer can be let go, on the screen.
    pub drops: Vec<DropSpot>,
    /// What letting the dragged layer go now does, as its card says it.
    pub says: String,
    press: Option<Press>,
    /// The wheel has picked layer `k` up and points at `drops[at]` (None:
    /// back in its place).
    wheel: Option<(usize, Option<usize>)>,
    /// Smart loop end's chip, after the loop's length.
    pub chip: Option<SmartChip>,
    /// The loop point view's contents, while there is something to show.
    pub point: Option<PointLook>,
    /// The loop point view shows in the lanes' place.
    point_open: bool,
    /// Where the chip's words and button were drawn (they follow the
    /// title and length, which only drawing can measure).
    chip_at: RefCell<Option<(Rect, Rect)>>,
    drawn: RefCell<Option<(Rect, EditorLook, i32)>>,
    acts: Vec<LoopAct>,
}

impl Default for LoopEditor {
    fn default() -> Self {
        LoopEditor {
            title: String::new(),
            sub: String::new(),
            colour: LOOP_COLOURS[0],
            listen: (false, false),
            free: None,
            has_sound: false,
            wave: None,
            position: None,
            bars: 0,
            layers: Vec::new(),
            picked: 0,
            empty: None,
            trim_ticks: None,
            strip: Strip::default(),
            send_built: [false; 4],
            send_open: false,
            sources: None,
            mixes: vec!["Master".to_string()],
            trimming: None,
            nudging: None,
            window_drag: None,
            point_drag: None,
            feedback: None,
            history: None,
            history_top: None,
            history_press: None,
            top: 0,
            swiped: None,
            drops: Vec::new(),
            says: String::new(),
            press: None,
            wheel: None,
            chip: None,
            point: None,
            point_open: false,
            chip_at: RefCell::new(None),
            drawn: RefCell::new(None),
            acts: Vec::new(),
        }
    }
}

/// Lanes are at least a fingertip tall (about 7 mm at the MPC One's
/// 216 pixels an inch), four in view.
pub const LANE_H: i32 = 60;
const LANE_GAP: i32 = 4;
const LANES_SHOWN: usize = 4;
/// A lane's head: its speaker, name, tag and S.
const LANE_HEAD: i32 = 196;
/// The undo history's rows: a fingertip tall, as the lanes.
const STEP_H: i32 = 56;
const STEP_GAP: i32 = 4;
/// The strip's controls.
const CTRL_H: i32 = 46;
/// A 1/16 note in ticks: the trim and nudge step.
const SIXTEENTH: i64 = mpfree_transport::time::PPQ as i64 / 4;

impl LoopEditor {
    pub fn parts(r: Rect) -> EditorParts {
        let head = Rect::new(r.x + 12, r.y + 8, r.w - 24, 36);
        let clear = Rect::new(head.right() - 88, head.y + 1, 88, 34);
        let send = Rect::new(clear.x - 8 - 80, head.y + 1, 80, 34);
        let listen = Rect::new(send.x - 8 - 112, head.y + 1, 112, 34);
        let strip = Rect::new(
            r.x + 12,
            r.bottom() - 10 - (18 + CTRL_H),
            r.w - 24,
            18 + CTRL_H,
        );
        let map_w = 14;
        let lanes_top = r.y + 72;
        let lanes = Rect::new(
            r.x + 10,
            lanes_top,
            r.w - 20 - map_w - 8,
            (strip.y - 10 - lanes_top).max(LANE_H),
        );
        let map = Rect::new(lanes.right() + 8, lanes.y, map_w, lanes.h);
        let wave_x = lanes.x + LANE_HEAD + 8;
        let ruler = Rect::new(wave_x, r.y + 50, lanes.right() - wave_x, 18);
        let point = Rect::new(
            lanes.x,
            ruler.y - 2,
            map.right() - lanes.x,
            lanes.bottom() - (ruler.y - 2),
        );
        EditorParts {
            head,
            listen,
            send,
            clear,
            ruler,
            lanes,
            map,
            strip,
            point,
        }
    }

    /// How many lanes show, and how tall each is.
    fn lane_fit(lanes: Rect) -> (usize, i32) {
        let n = LANES_SHOWN as i32;
        let h = ((lanes.h - (n - 1) * LANE_GAP) / n).clamp(LANE_H.min(lanes.h), 76);
        let fit = ((lanes.h + LANE_GAP) / (h + LANE_GAP)).max(1) as usize;
        (fit, h)
    }

    /// The layer a lane `d` from the top shows (0: the newest).
    fn layer_at(&self, d: usize) -> Option<usize> {
        self.layers.len().checked_sub(d + 1)
    }

    /// The lanes shown: (layer, its rectangle), newest on top.
    pub fn lane_rects(&self, lanes: Rect) -> Vec<(usize, Rect)> {
        let (fit, h) = LoopEditor::lane_fit(lanes);
        (self.top..self.layers.len().min(self.top + fit))
            .enumerate()
            .filter_map(|(i, d)| {
                let k = self.layer_at(d)?;
                let y = lanes.y + i as i32 * (h + LANE_GAP);
                Some((k, Rect::new(lanes.x, y, lanes.w, h)))
            })
            .collect()
    }

    pub fn lane_parts(rr: Rect) -> LaneParts {
        let head = Rect::new(rr.x, rr.y, LANE_HEAD, rr.h);
        let solo = Rect::new(head.right() - 42, rr.y + (rr.h - 36) / 2, 34, 36);
        let listen = Rect::new(solo.x - 40, solo.y, 34, 36);
        let wave = Rect::new(rr.x + LANE_HEAD + 8, rr.y, rr.w - LANE_HEAD - 8, rr.h);
        let delete = Rect::new(wave.right() - 44, rr.y + (rr.h - 36) / 2, 36, 36);
        LaneParts {
            head,
            listen,
            solo,
            wave,
            delete,
        }
    }

    /// The trim grips at the ruler's ends.
    pub(crate) fn trim_grips(ruler: Rect) -> (Rect, Rect) {
        (
            Rect::new(ruler.x - 5, ruler.y - 1, 10, ruler.h + 2),
            Rect::new(ruler.right() - 5, ruler.y - 1, 10, ruler.h + 2),
        )
    }

    /// The tick a finger at `x` on the ruler points to, to the nearest 1/16.
    fn tick_at(ruler: Rect, x: i32, ticks: i64) -> i64 {
        let f = ((x - ruler.x) as f64 / ruler.w.max(1) as f64).clamp(0.0, 1.0);
        let t = (f * ticks as f64 / SIXTEENTH as f64).round() as i64 * SIXTEENTH;
        t.clamp(0, ticks)
    }

    /// The ruler's width of a 1/16 note.
    fn sixteenth_px(ruler: Rect, ticks: i64) -> f32 {
        ruler.w as f32 * SIXTEENTH as f32 / ticks.max(SIXTEENTH) as f32
    }

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

    /// Whether Send's choices show.
    pub fn send_open(&self) -> bool {
        self.send_open
    }

    /// Close Send's choices (the loop emptied, or another was picked).
    pub fn close_send(&mut self) {
        self.send_open = false;
    }

    /// Open Records from's choices in the lanes' place, `now` (into
    /// [`SOURCE_CHOICES`]) lit; open, close them.
    pub fn toggle_sources(&mut self, now: usize) {
        self.send_open = false;
        self.sources = if self.sources.is_some() {
            None
        } else {
            Some(now)
        };
    }

    /// Records from's choices show, and the one lit.
    #[cfg(test)]
    pub fn sources(&self) -> Option<usize> {
        self.sources
    }

    /// Records from's mixes, by name, as the Mix tab names them.
    pub fn set_mixes(&mut self, mixes: Vec<String>) {
        self.mixes = mixes;
    }

    /// Acts are waiting for [`take_actions`] (the loop point view closed
    /// itself).
    pub fn has_acts(&self) -> bool {
        !self.acts.is_empty()
    }

    /// Whether the loop point view is open (it shows while there is
    /// something in it, [`LoopEditor::point`]).
    #[cfg(test)]
    pub fn point_open(&self) -> bool {
        self.point_open
    }

    /// Open the loop point view, as a tap on the chip's words does.
    pub fn open_point(&mut self) {
        if !self.point_open {
            self.point_open = true;
            self.acts.push(LoopAct::PointView(true));
        }
    }

    /// Close the loop point view, and tell the looper it can stop reading
    /// the audio for it. Nothing if it is closed.
    pub fn close_point(&mut self) {
        if self.point_open {
            self.point_open = false;
            self.acts.push(LoopAct::PointView(false));
        }
    }

    /// The loop point view, while it shows in the lanes' place: open, with
    /// something in it, and none of Records from, Send or the undo
    /// history over it.
    fn point_shown(&self) -> Option<&PointLook> {
        self.point.as_ref().filter(|_| {
            self.point_open && self.sources.is_none() && !self.send_open && self.history.is_none()
        })
    }

    /// The loop point view's parts in `a` ([`EditorParts::point`]) for
    /// `look`: its buttons left to right, then Read as's choices.
    pub fn point_parts(a: Rect, look: &PointLook) -> PointParts {
        let reads = look.read_as.len();
        let head = Rect::new(a.x + 16, a.y + 8, a.w - 32, 36);
        let close = Rect::new(head.right() - 36, head.y, 36, 36);
        let seg_w = 60;
        let n_steps = MoveStep::ALL.len() as i32;
        let steps_x = close.x - 16 - n_steps * seg_w - 4;
        let steps = (0..n_steps)
            .map(|i| Rect::new(steps_x + i * seg_w, head.y + 4, seg_w, 28))
            .collect();
        let step_label = Rect::new(steps_x - 12 - 38, head.y, 38, head.h);
        let pane_w = (head.w - 16) / 2;
        let titles_y = head.bottom() + 6;
        // What isn't the windows: the head, the titles, the line, the
        // buttons and the gaps between them.
        let win_h = (a.h - 8 - 36 - 6 - TITLE_H - 4 - 6 - 22 - 6 - 44 - 10).clamp(40, 112);
        let win_y = titles_y + TITLE_H + 4;
        let start = Rect::new(head.x, win_y, pane_w, win_h);
        let end = Rect::new(head.right() - pane_w, win_y, pane_w, win_h);
        let line = Rect::new(head.x, start.bottom() + 6, head.w, 22);
        let row_y = line.bottom() + 6;
        let mut x = head.x;
        let mut next = |w: i32| {
            let r = Rect::new(x, row_y, w, 44);
            x += w + 10;
            r
        };
        let pick = (0..look.pick.len())
            .map(|i| next(if i == 0 { 140 } else { 148 }))
            .collect();
        let ab = look.ab.map(|_| next(72));
        let rule = Rect::new(x + 8, row_y + 6, 1, 32);
        let read_label = Rect::new(rule.right() + 18, row_y, 64, 44);
        let well_x = read_label.right() + 10;
        let n = reads.max(1) as i32;
        let w = ((head.right() - well_x - 8 - 4 * (n - 1)) / n).clamp(0, 196);
        let read_as = (0..reads as i32)
            .map(|i| Rect::new(well_x + 4 + i * (w + 4), row_y + 4, w, 36))
            .collect();
        let nudge = |x: i32| {
            [
                Rect::new(x + NUDGE_X, titles_y, 40, TITLE_H),
                Rect::new(x + NUDGE_X + 44, titles_y, 40, TITLE_H),
            ]
        };
        PointParts {
            head,
            close,
            step_label,
            steps,
            nudge: [nudge(start.x), nudge(end.x)],
            titles: (
                Rect::new(start.x, titles_y, pane_w, TITLE_H),
                Rect::new(end.x, titles_y, pane_w, TITLE_H),
            ),
            start,
            end,
            line,
            pick,
            ab,
            rule,
            read_label,
            read_as,
        }
    }

    /// Records from's choices in the lanes' space: the three side by side,
    /// then `mixes` chips in rows under them, as many as fit.
    fn source_rects(lanes: Rect, mixes: usize) -> Vec<Rect> {
        let n = SOURCE_CHOICES.len() as i32;
        let inner = lanes.inset_xy(24, 24);
        let w = (inner.w - 16 * (n - 1)) / n;
        let h = 84;
        let mut out: Vec<Rect> = (0..n)
            .map(|i| Rect::new(inner.x + i * (w + 16), inner.y, w, h))
            .collect();
        let gap = 10;
        let per_row = ((inner.w + gap) / (128 + gap)).max(1);
        let cw = (inner.w - gap * (per_row - 1)) / per_row;
        let top = inner.y + h + 36;
        for k in 0..mixes as i32 {
            let b = Rect::new(
                inner.x + (k % per_row) * (cw + gap),
                top + (k / per_row) * (40 + gap),
                cw,
                40,
            );
            if b.bottom() > lanes.bottom() - 4 {
                break;
            }
            out.push(b);
        }
        out
    }

    fn paint_sources(&self, cx: &mut PaintCx, lanes: Rect, now: usize) {
        let t = cx.theme.clone();
        cx.canvas
            .fill_round_rect(lanes, t.round(10), t.bg.mix(t.surface, 0.45));
        group_label(cx, lanes.x + 24, lanes.y + 4, "RECORDS FROM");
        let rects = LoopEditor::source_rects(lanes, self.mixes.len());
        let fixed = SOURCE_CHOICES.len();
        if let Some(first) = rects.get(fixed) {
            cx.text(
                FontId::Bold,
                11.5,
                Rect::new(first.x, first.y - 22, lanes.w - 48, 14),
                Align::Left,
                t.text_dim,
                "OR A MIX: THE MASTER (ALL BUT THE LOOPS), A TRACK OR A SUBMIX",
            );
        }
        for (i, (b, name)) in rects.iter().skip(fixed).zip(&self.mixes).enumerate() {
            let lit = fixed + i == now;
            cx.canvas
                .fill_round_rect(*b, t.round(10), if lit { t.picked() } else { t.raised });
            cx.text(
                FontId::Bold,
                15.0,
                b.inset_xy(8, 0),
                Align::Center,
                if lit { t.on_picked() } else { t.text },
                name,
            );
        }
        for (i, (b, (title, what))) in rects.into_iter().zip(SOURCE_CHOICES).enumerate() {
            let lit = i == now;
            cx.canvas
                .fill_round_rect(b, t.round(12), if lit { t.picked() } else { t.raised });
            let (ink, dim) = if lit {
                (t.on_picked(), t.on_picked())
            } else {
                (t.text, t.text_dim)
            };
            cx.text(
                FontId::Bold,
                18.0,
                Rect::new(b.x, b.y + b.h / 2 - 24, b.w, 26),
                Align::Center,
                ink,
                title,
            );
            cx.text(
                FontId::Sans,
                13.0,
                Rect::new(b.x + 10, b.y + b.h / 2 + 4, b.w - 20, 18),
                Align::Center,
                dim,
                what,
            );
        }
    }

    fn steps_fit(rows: Rect) -> usize {
        ((rows.h + STEP_GAP) / (STEP_H + STEP_GAP)).max(1) as usize
    }

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
                let y = rows.y + i as i32 * (STEP_H + STEP_GAP);
                (k, Rect::new(rows.x, y, rows.w, STEP_H))
            })
            .collect()
    }

    /// Send's choices, two by two in the lanes' space.
    fn send_rects(lanes: Rect) -> Vec<Rect> {
        let inner = lanes.inset_xy(24, 20);
        let w = (inner.w - 16) / 2;
        let h = ((inner.h - 16) / 2).min(96);
        let y0 = inner.y + (inner.h - (2 * h + 16)) / 2;
        (0..4)
            .map(|i| {
                let (c, r) = (i % 2, i / 2);
                Rect::new(inner.x + c * (w + 16), y0 + r * (h + 16), w, h)
            })
            .collect()
    }

    /// The strip's controls as drawn now: each one, its rectangle, its
    /// label, and whether it works.
    pub fn strip_controls(&self, s: Rect) -> Vec<(Ctl, Rect, String, bool)> {
        let y = s.y + 18;
        let mut out = Vec::new();
        match &self.strip {
            Strip::Plays {
                reversed,
                length,
                edits,
                chips,
                undo,
                redo,
                merge,
                ..
            } => {
                let x = s.x;
                out.push((
                    Ctl::Feedback,
                    Rect::new(x, s.y + 14, 64, s.h - 14),
                    "Feedback".to_string(),
                    true,
                ));
                let xo = x + 84;
                for (i, l) in OVERDUB_CHOICES.iter().enumerate() {
                    out.push((
                        Ctl::Overdubs(i as u8),
                        Rect::new(xo + 3 + i as i32 * 70, y + 3, 66, CTRL_H - 6),
                        l.to_string(),
                        true,
                    ));
                }
                let x2 = xo + OVERDUB_CHOICES.len() as i32 * 70 + 26;
                let rev = if *reversed { "Reverse on" } else { "Reverse" };
                out.push((
                    Ctl::Reverse,
                    Rect::new(x2, y, 104, CTRL_H),
                    rev.to_string(),
                    *edits,
                ));
                let xs = x2 + 104 + 16;
                for (i, l) in SPEED_CHOICES.iter().enumerate() {
                    out.push((
                        Ctl::Speed(i as u8),
                        Rect::new(xs + 3 + i as i32 * 44, y + 3, 40, CTRL_H - 6),
                        format!("Speed {l}"),
                        *edits,
                    ));
                }
                let x3 = xs + SPEED_CHOICES.len() as i32 * 44 + 16;
                out.push((
                    Ctl::Half,
                    Rect::new(x3, y, 46, CTRL_H),
                    format!("½ ({length})"),
                    *edits,
                ));
                out.push((
                    Ctl::Double,
                    Rect::new(x3 + 46 + 60, y, 46, CTRL_H),
                    format!("× 2 ({length})"),
                    *edits,
                ));
                let x4 = x3 + 46 + 60 + 46 + 28;
                out.push((
                    Ctl::Undo,
                    Rect::new(x4, y, 48, CTRL_H),
                    "Undo".to_string(),
                    *undo,
                ));
                // The chips take the room between Undo and Redo, Redo
                // sitting clear of Merge.
                let room = (s.right() - MERGE_W - 12 - 48 - 6) - (x4 + 54) - 6;
                let shown = LoopEditor::chips_shown(chips.len(), room);
                let cw = (shown as i32 * 17).max(17) + 6;
                out.push((
                    Ctl::History,
                    Rect::new(x4 + 54, y, cw, CTRL_H),
                    "History".to_string(),
                    !chips.is_empty(),
                ));
                out.push((
                    Ctl::Redo,
                    Rect::new(x4 + 54 + cw + 6, y, 48, CTRL_H),
                    "Redo".to_string(),
                    *redo,
                ));
                out.push((
                    Ctl::Merge,
                    Rect::new(s.right() - MERGE_W, y, MERGE_W, CTRL_H),
                    merge.0.clone(),
                    merge.1,
                ));
            }
            Strip::Records {
                monitor,
                starts,
                sync,
                ..
            } => {
                // Records from is over the loops (the deck's header), for
                // every loop; here, how this one records. Sync first: the
                // clock it keeps time with.
                let x = s.x;
                for (i, l) in SYNC_CHOICES.iter().enumerate() {
                    out.push((
                        Ctl::Sync(i as u8),
                        Rect::new(x + 3 + [0, 68][i], y + 3, [64, 56][i], CTRL_H - 6),
                        l.to_string(),
                        sync.1,
                    ));
                }
                let mut x2 = x + 146;
                if let Some(on) = monitor {
                    out.push((
                        Ctl::Monitor,
                        Rect::new(x2, y + 2, 74, CTRL_H - 4),
                        format!("Monitor {}", if *on { "on" } else { "off" }),
                        true,
                    ));
                    x2 += 100;
                }
                for (i, l) in LENGTH_CHOICES.iter().enumerate() {
                    let w = [70, 84, 36, 36, 36, 62][i];
                    let bx = x2 + 3 + [0, 74, 162, 202, 242, 282][i];
                    out.push((
                        Ctl::Bars(i as u8),
                        Rect::new(bx, y + 3, w, CTRL_H - 6),
                        l.to_string(),
                        true,
                    ));
                }
                let x3 = x2 + 364;
                for (i, l) in STARTS_CHOICES.iter().enumerate() {
                    out.push((
                        Ctl::Starts(i as u8),
                        Rect::new(x3 + 3 + i as i32 * 110, y + 3, 106, CTRL_H - 6),
                        l.to_string(),
                        starts.1,
                    ));
                }
                let x4 = x3 + 232;
                for (i, l) in ["Play", "Overdub"].iter().enumerate() {
                    out.push((
                        Ctl::Then(i as u8),
                        Rect::new(x4 + 3 + i as i32 * 78, y + 3, 74, CTRL_H - 6),
                        l.to_string(),
                        true,
                    ));
                }
            }
        }
        out
    }

    /// How many history chips fit in `room` pixels (the newest are
    /// shown).
    fn chips_shown(n: usize, room: i32) -> usize {
        n.min((room.max(0) / 17) as usize)
    }

    /// The layer being dragged (play order) and where its card is now:
    /// under the finger, or on the place the wheel points at.
    pub fn drag(&self) -> Option<(usize, Rect)> {
        if let Some((k, _)) = self.wheel {
            let (x, y) = self.finger()?;
            return Some((k, Rect::new(x - CARD_W / 2, y - CARD_H / 2, CARD_W, CARD_H)));
        }
        let p = self.press.filter(|p| p.lifted)?;
        Some((
            p.k,
            Rect::new(p.at.0 - p.grip.0, p.at.1 - p.grip.1, CARD_W, CARD_H),
        ))
    }

    /// Where the drag points: the finger, or the loop the wheel points at
    /// (the layer's own lane before it points anywhere).
    pub fn finger(&self) -> Option<(i32, i32)> {
        if let Some((k, at)) = self.wheel {
            return Some(match at.and_then(|i| self.drops.get(i)) {
                Some(d) => (d.rect.x + d.rect.w / 2, d.rect.y + d.rect.h / 2),
                None => self.drawn_lane(k).map_or((0, 0), |r| (r.x + 140, r.y + 30)),
            });
        }
        self.press.filter(|p| p.lifted).map(|p| p.at)
    }

    /// Where the dragged layer would land if let go now.
    pub fn landing(&self) -> Option<&DropSpot> {
        if let Some((_, at)) = self.wheel {
            return at.and_then(|i| self.drops.get(i));
        }
        let p = self.press.filter(|p| p.lifted)?;
        self.drops.iter().find(|d| d.rect.contains(p.at.0, p.at.1))
    }

    /// Layer `k`'s lane as last drawn, if it shows.
    fn drawn_lane(&self, k: usize) -> Option<Rect> {
        let (r, _, _) = self.drawn.borrow().as_ref()?.clone();
        self.lane_rects(LoopEditor::parts(r).lanes)
            .into_iter()
            .find(|(kk, _)| *kk == k)
            .map(|(_, rr)| rr)
    }

    /// The wheel's push on layer `k`: pick it up (true), to point the
    /// wheel at the places it can go. False when nothing can take it.
    pub fn wheel_grab(&mut self, k: usize) -> bool {
        if k >= self.layers.len() || self.drops.is_empty() {
            return false;
        }
        self.press = None;
        self.wheel = Some((k, None));
        true
    }

    /// The wheel's push again: put the picked-up layer down where it
    /// points, as a finger letting go there would.
    pub fn wheel_drop(&mut self) -> Option<LoopAct> {
        let (k, at) = self.wheel.take()?;
        let d = self.drops.get(at?)?;
        let act = match d.refused {
            None => LoopAct::DropLayer(k, d.to),
            Some(_) => LoopAct::NoDrop(d.to),
        };
        self.acts.push(act);
        Some(act)
    }

    /// The outline moved away: the wheel's layer goes back to its place.
    pub fn wheel_cancel(&mut self) {
        self.wheel = None;
    }

    /// Whether the wheel holds a layer picked up.
    pub fn wheel_held(&self) -> bool {
        self.wheel.is_some()
    }

    /// Time passed: a finger held still on a layer's lane for
    /// [`LIFT_HOLD_S`] can now lift it (the lane is outlined to say so).
    /// True when that just happened.
    pub fn held(&mut self, dt: f32) -> bool {
        let Some(p) = self.press.as_mut() else {
            return false;
        };
        if p.ready || p.lifted || p.swipe.is_some() {
            return false;
        }
        p.held += dt.max(0.0);
        p.ready = p.held >= LIFT_HOLD_S;
        p.ready
    }

    /// Whether a finger is held on a layer's lane, waiting to lift it (so
    /// time passing matters to the panel).
    pub fn holding(&self) -> bool {
        self.press
            .is_some_and(|p| !p.ready && !p.lifted && p.swipe.is_none())
    }

    /// Keep the picked layer in view, with the panel in `r`; lanes a
    /// swipe moved stay where it left them until another layer is picked.
    pub fn follow_in(&mut self, r: Rect) {
        let (fit, _) = LoopEditor::lane_fit(LoopEditor::parts(r).lanes);
        let n = self.layers.len();
        let last = n.saturating_sub(fit);
        if self.swiped == Some(self.picked) {
            self.top = self.top.min(last);
            return;
        }
        self.swiped = None;
        // The picked layer's place from the top.
        let d = n.saturating_sub(self.picked + 1);
        if d < self.top {
            self.top = d;
        } else if d >= self.top + fit {
            self.top = d + 1 - fit;
        }
        self.top = self.top.min(last);
    }

    fn play_x(&self, w: Rect) -> Option<i32> {
        self.position
            .filter(|_| self.wave.as_ref().is_some_and(|w| w.frames > 0))
            .map(|p| w.x + (p.clamp(0.0, 1.0) * w.w as f32) as i32)
    }

    fn look(&self) -> EditorLook {
        EditorLook {
            title: self.title.clone(),
            sub: self.sub.clone(),
            colour: self.colour,
            listen: self.listen,
            free: self.free,
            has_sound: self.has_sound,
            wave: self.wave.as_ref().map(|w| Arc::as_ptr(w) as usize),
            bars: self.bars,
            layers: self.layers.clone(),
            picked: self.picked,
            top: self.top,
            empty: self.empty.clone(),
            strip: self.strip.clone(),
            send_open: self.send_open,
            sources: self.sources,
            mixes: self.mixes.clone(),
            history: self
                .history
                .clone()
                .map(|h| (h, self.history_top.unwrap_or(usize::MAX))),
            trim: self.trim_ticks,
            trimming: self.trimming.map(|t| (t.2, t.3)),
            nudging: self.nudging.map(|n| n.2),
            window: self.window_drag.map(|d| d.now),
            feedback_held: self.feedback.is_some(),
            drag: self
                .drag()
                .map(|(k, card)| (k, card, self.says.clone(), self.landing().cloned())),
            ready: self.press.filter(|p| p.ready && !p.lifted).map(|p| p.k),
            chip: self.chip.clone(),
            point: self.point.clone().filter(|_| self.point_open),
        }
    }

    /// The Feedback value a finger on the knob points to: up turns it up,
    /// 150 px the whole way.
    fn feedback_at(&self, y: i32) -> Option<f32> {
        let (_, y0, v0) = self.feedback?;
        Some((v0 + (y0 - y) as f32 / 150.0 * 0.5).clamp(0.5, 1.0))
    }
}

// ------------------------------------------------------------- painting

/// A button: raised (or lit), its label centred, faint when it can't work.
/// A small triangle pointing left (`left`) or right, centred in `b`: a
/// loop point window's ◀ and ▶ (the UI font has neither).
fn step_icon(cx: &mut PaintCx, b: Rect, left: bool, c: Color) {
    let h = 12.0f32;
    let w = h * 0.8;
    let x = b.x as f32 + (b.w as f32 - w) / 2.0;
    let y = b.y as f32 + b.h as f32 / 2.0;
    let mut s = Shape::new(b);
    let rows = (h * 2.0) as i32;
    for i in 0..=rows {
        let f = i as f32 / rows as f32;
        let yy = y - h / 2.0 + f * h;
        let len = (w * (1.0 - (2.0 * f - 1.0).abs())).max(0.5);
        let (a, z) = if left {
            (x + w - len, x + w)
        } else {
            (x, x + len)
        };
        s.segment((a, yy), (z, yy), 1.2);
    }
    s.draw(cx.canvas, c);
}

/// A play triangle, `h` pixels tall, its left edge at `x` and centred on
/// `y` (the UI font has no ▶).
pub(crate) fn play_icon(cx: &mut PaintCx, x: f32, y: f32, h: f32, c: Color) {
    let w = h * 0.87;
    let bbox = Rect::new(
        x as i32 - 2,
        (y - h / 2.0) as i32 - 2,
        w as i32 + 4,
        h as i32 + 4,
    );
    let mut s = Shape::new(bbox);
    let rows = (h * 2.0) as i32;
    for i in 0..=rows {
        let f = i as f32 / rows as f32;
        let yy = y - h / 2.0 + f * h;
        let len = w * (1.0 - (2.0 * f - 1.0).abs());
        s.segment((x, yy), (x + len.max(0.5), yy), 1.2);
    }
    s.draw(cx.canvas, c);
}

/// Listen's button: ▶ Listen, or Stop lit while it plays.
fn listen_button(cx: &mut PaintCx, b: Rect, works: bool, on: bool) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(b, t.round(10), if on { t.picked() } else { t.raised });
    let ink = if on {
        t.on_picked()
    } else {
        t.button_ink(works)
    };
    let label = if on { "Stop" } else { "Listen" };
    let lw = cx.measure(FontId::Bold, 14.0, label);
    let total = 12 + 8 + lw;
    let x0 = b.x + (b.w - total) / 2;
    let cy = b.y as f32 + b.h as f32 / 2.0;
    if on {
        cx.canvas
            .fill_rect(Rect::new(x0, cy as i32 - 5, 11, 11), ink);
    } else {
        play_icon(cx, x0 as f32 + 1.0, cy, 12.0, ink);
    }
    cx.text(
        FontId::Bold,
        14.0,
        Rect::new(x0 + 20, b.y, lw + 2, b.h),
        Align::Left,
        ink,
        label,
    );
}

/// A free loop's Stop free or Play free, in Listen's place: ■ Stop free
/// while the free loops play; ▶ Play free while they are stopped, lit
/// while a Play waits for the song's bar line. "free": they stop and
/// start the free loops together, not this loop alone (Jay's review,
/// 23 Nov, 5.3).
fn free_button(cx: &mut PaintCx, b: Rect, runs: bool, cued: bool) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(b, t.round(10), if cued { t.picked() } else { t.raised });
    let ink = if cued { t.on_picked() } else { t.text };
    let label = if runs { "Stop free" } else { "Play free" };
    let lw = cx.measure(FontId::Bold, 14.0, label);
    let total = 12 + 8 + lw;
    let x0 = b.x + (b.w - total) / 2;
    let cy = b.y as f32 + b.h as f32 / 2.0;
    if runs {
        cx.canvas
            .fill_rect(Rect::new(x0, cy as i32 - 5, 11, 11), ink);
    } else {
        play_icon(cx, x0 as f32 + 1.0, cy, 12.0, ink);
    }
    cx.text(
        FontId::Bold,
        14.0,
        Rect::new(x0 + 20, b.y, lw + 2, b.h),
        Align::Left,
        ink,
        label,
    );
}

fn button(cx: &mut PaintCx, b: Rect, label: &str, on: bool, lit: bool, px: f32) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(b, t.round(10), if lit { t.picked() } else { t.raised });
    let ink = if lit { t.on_picked() } else { t.button_ink(on) };
    cx.text(FontId::Bold, px, b, Align::Center, ink, label);
}

/// A segmented switch's well behind `items`, the chosen one lit.
fn segments(cx: &mut PaintCx, items: &[(Rect, &str, bool)]) {
    let t = cx.theme.clone();
    if let (Some(a), Some(b)) = (items.first(), items.last()) {
        let well = Rect::new(
            a.0.x - 3,
            a.0.y - 3,
            b.0.right() + 3 - (a.0.x - 3),
            a.0.h + 6,
        );
        cx.canvas
            .fill_round_rect(well, 11, t.bg.mix(t.surface, 0.35));
    }
    for (r, label, chosen) in items {
        if *chosen {
            cx.canvas.fill_round_rect(*r, t.round(8), t.picked());
        }
        let ink = if *chosen { t.on_picked() } else { t.text };
        cx.text(FontId::Bold, 14.0, *r, Align::Center, ink, label);
    }
}

/// [`segments`] drawn faint: they can't change now (a tap says why).
fn faint_segments(cx: &mut PaintCx, items: &[(Rect, &str, bool)]) {
    let t = cx.theme.clone();
    if let (Some(a), Some(b)) = (items.first(), items.last()) {
        let well = Rect::new(
            a.0.x - 3,
            a.0.y - 3,
            b.0.right() + 3 - (a.0.x - 3),
            a.0.h + 6,
        );
        cx.canvas
            .fill_round_rect(well, 11, t.bg.mix(t.surface, 0.35));
    }
    for (r, label, chosen) in items {
        if *chosen {
            cx.canvas.stroke_round_rect(*r, t.round(8), 1, t.text_dim);
        }
        cx.text(FontId::Bold, 14.0, *r, Align::Center, t.text_dim, label);
    }
}

/// A small label over a group of controls.
fn group_label(cx: &mut PaintCx, x: i32, y: i32, s: &str) {
    let t = cx.theme.clone();
    cx.text(
        FontId::Bold,
        11.5,
        Rect::new(x, y, 160, 14),
        Align::Left,
        t.text_dim,
        s,
    );
}

/// One of the loop point view's sides: its title and note over `title`,
/// and in `win` the audio's peaks, the shade outside the loop, your press
/// or tap (grey, dashed) and what was found (the accent), with how far
/// apart they are.
fn point_pane(cx: &mut PaintCx, pane: &PointPane, title: Rect, win: Rect) {
    let t = cx.theme.clone();
    let tw = cx.measure(FontId::Bold, 13.0, &pane.title);
    cx.text(
        FontId::Bold,
        13.0,
        Rect::new(title.x, title.y, tw + 2, title.h),
        Align::Left,
        t.text,
        &pane.title,
    );
    let lit_ink = t.text.mix(t.accent, 0.25);
    let note_x = title.x + NUDGE_X + 88 + 12;
    cx.text(
        FontId::Sans,
        13.0,
        Rect::new(note_x, title.y, title.right() - note_x, title.h),
        Align::Right,
        if pane.found.is_some() {
            lit_ink
        } else {
            t.text_dim
        },
        &pane.note,
    );
    cx.canvas.fill_round_rect(win, 6, t.bg);
    let old = cx.canvas.clip();
    cx.canvas.set_clip(old.intersect(win));
    // The peaks: a slice every `step` pixels, from 13 in.
    let step = ((win.w - 26) / POINT_BARS as i32).max(1);
    let x0 = win.x + 13;
    let span = step * POINT_BARS as i32;
    let tall = (win.h - 14).max(2) as f32;
    for (i, &p) in pane.peaks.iter().enumerate() {
        let h = ((f32::from(p) / 255.0 * tall).round() as i32).max(2);
        let r = Rect::new(
            x0 + i as i32 * step,
            win.y + (win.h - h) / 2,
            (step - 1).max(1),
            h,
        );
        cx.canvas.fill_rect(r, t.accent);
    }
    let at = |f: f32| x0 + (f.clamp(-0.1, 1.1) * span as f32).round() as i32;
    let (edge, before) = pane.edge;
    let xe = at(edge);
    let shade = if before {
        Rect::new(win.x, win.y, xe - win.x, win.h)
    } else {
        Rect::new(xe, win.y, win.right() - xe, win.h)
    };
    cx.canvas.fill_rect(shade, t.bg.with_alpha(140));
    let xy = at(pane.you.0);
    let you_ink = t.text.mix(t.text_dim, 0.4);
    fn label(cx: &mut PaintCx, x: i32, y: i32, left: bool, ink: Color, s: &str) {
        let w = cx.measure(FontId::Bold, 12.0, s);
        let x = if left { x - 6 - w } else { x + 6 };
        // On a backing, to read over the peaks.
        let back = cx.theme.bg.with_alpha(170);
        cx.canvas
            .fill_round_rect(Rect::new(x - 3, y - 1, w + 6, 16), 4, back);
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(x, y, w + 2, 14),
            Align::Left,
            ink,
            s,
        );
    }
    if let Some((f, name, gap)) = &pane.found {
        let xf = at(*f);
        let (l, r) = (xf.min(xy), xf.max(xy));
        cx.canvas
            .fill_rect(Rect::new(l, win.y, r - l, win.h), t.accent.with_alpha(41));
        cx.canvas
            .fill_rect(Rect::new(xf - 1, win.y, 2, win.h), t.accent);
        // Each label on the side away from the other line.
        label(cx, xf, win.y + 6, xf < xy, t.accent, name);
        if !gap.is_empty() {
            let gw = cx.measure(FontId::Bold, 12.0, gap);
            let back = t.bg.with_alpha(170);
            cx.canvas.fill_round_rect(
                Rect::new((l + r - gw) / 2 - 3, win.bottom() - 25, gw + 6, 16),
                4,
                back,
            );
            cx.text(
                FontId::Bold,
                12.0,
                Rect::new((l + r - gw) / 2, win.bottom() - 24, gw + 2, 14),
                Align::Left,
                lit_ink,
                gap,
            );
        }
    }
    // Your press or tap: dashed.
    let mut y = win.y;
    while y < win.bottom() {
        cx.canvas
            .fill_rect(Rect::new(xy - 1, y, 2, 5.min(win.bottom() - y)), t.text_dim);
        y += 9;
    }
    let left = pane.found.as_ref().is_some_and(|(f, _, _)| at(*f) > xy);
    label(cx, xy, win.y + 6, left, you_ink, &pane.you.1);
    // The hits heard: short marks along the foot, a tap puts the edge on
    // one.
    let hit_ink = t.text.mix(t.accent, 0.5);
    for &h in &pane.hits {
        let x = at(h);
        cx.canvas
            .fill_round_rect(Rect::new(x - 2, win.bottom() - 9, 4, 7), 2, hit_ink);
    }
    // A move set up: a solid line with a grip, "your move" over it and
    // how far from you under it.
    if let Some((f, gap)) = &pane.moving {
        let x = at(*f);
        cx.canvas
            .fill_rect(Rect::new(x - 1, win.y, 3, win.h), t.text);
        cx.canvas
            .fill_round_rect(Rect::new(x - 6, win.y + win.h / 2 - 12, 13, 24), 4, t.text);
        for k in [-4, 0, 4] {
            cx.canvas
                .fill_rect(Rect::new(x - 3, win.y + win.h / 2 + k - 1, 7, 1), t.bg);
        }
        let left = x > win.x + win.w / 2;
        label(cx, x, win.y + 24, left, t.text, "your move");
        label(cx, x, win.bottom() - 42, left, t.text, gap);
    }
    cx.canvas.set_clip(old);
}

/// A knob: the 270-degree track, the value's arc and its pointer, the
/// value in the middle.
fn knob(cx: &mut PaintCx, centre: (i32, i32), d: i32, v: f32, lit: Color, value: &str) {
    use std::f32::consts::PI;
    let t = cx.theme.clone();
    let (x, y) = (centre.0 as f32, centre.1 as f32);
    let rad = d as f32 / 2.0 - 3.0;
    let bbox = Rect::new(centre.0 - d / 2 - 3, centre.1 - d / 2 - 3, d + 6, d + 6);
    let (a0, a1) = (0.75 * PI, 2.25 * PI);
    let mut s = Shape::new(bbox);
    s.arc(x, y, rad, a0, a1, 5.0);
    s.draw(cx.canvas, t.raised);
    let at = a0 + (a1 - a0) * v.clamp(0.0, 1.0);
    let mut s = Shape::new(bbox);
    if at - a0 > 0.01 {
        s.arc(x, y, rad, a0, at, 5.0);
    }
    s.draw(cx.canvas, lit);
    let mut s = Shape::new(bbox);
    s.disc(x + rad * at.cos(), y + rad * at.sin(), 3.5);
    s.draw(cx.canvas, t.text);
    cx.text(
        FontId::Bold,
        12.5,
        Rect::new(centre.0 - d / 2, centre.1 - 9, d, 18),
        Align::Center,
        t.text,
        value,
    );
}

/// A lane's wave: the layer's peaks across `w` as bars round the middle,
/// at its level, the part not yet recorded left out while it records.
fn paint_lane_wave(
    cx: &mut PaintCx,
    w: Rect,
    peaks: &[f32],
    c: Color,
    gain: f32,
    upto: Option<i32>,
) {
    if peaks.is_empty() || w.w <= 0 {
        return;
    }
    let mid = w.y + w.h / 2;
    let half = (w.h - 10) as f32 / 2.0;
    let mut x = 0;
    while x < w.w {
        if upto.is_some_and(|u| w.x + x > u) {
            break;
        }
        let col = (x as usize * peaks.len() / w.w as usize).min(peaks.len() - 1);
        let a = (peaks[col] * gain).clamp(0.0, 1.0).powf(0.6);
        let h = ((a * half) as i32).max(1);
        cx.canvas
            .fill_rect(Rect::new(w.x + x, mid - h, 2, 2 * h), c);
        x += 3;
    }
}

impl LoopEditor {
    fn paint_head(&self, cx: &mut PaintCx, p: &EditorParts) {
        let t = cx.theme.clone();
        let h = p.head;
        cx.canvas
            .fill_round_rect(Rect::new(h.x, h.y + 6, 22, 22), t.round(6), self.colour);
        let nw = cx.measure(FontId::Bold, 19.0, &self.title);
        cx.text(
            FontId::Bold,
            19.0,
            Rect::new(h.x + 32, h.y, nw + 4, h.h),
            Align::Left,
            t.text,
            &self.title,
        );
        let sub_x = h.x + 32 + nw + 16;
        let sub_w = (p.listen.x - 12 - sub_x).max(0);
        cx.text(
            FontId::Sans,
            14.0,
            Rect::new(sub_x, h.y + 1, sub_w, h.h),
            Align::Left,
            t.text_dim,
            &self.sub,
        );
        // Smart loop end's chip, after the length.
        let sw = cx.measure(FontId::Sans, 14.0, &self.sub).min(sub_w);
        let at = self
            .chip
            .as_ref()
            .and_then(|c| self.paint_chip(cx, c, sub_x + sw + 10, p.listen.x - 12, h));
        *self.chip_at.borrow_mut() = at;
        // Listen: the loop alone while the music is stopped. A free loop
        // has the free loops' Stop or Play there instead.
        match self.free {
            Some((runs, cued)) => free_button(cx, p.listen, runs, cued),
            None => {
                let (works, on) = self.listen;
                listen_button(cx, p.listen, works || on, on);
            }
        }
        button(cx, p.send, "Send", self.has_sound, self.send_open, 14.0);
        let ink = if self.has_sound {
            t.bad
        } else {
            t.bad.mix(t.surface, 0.55)
        };
        cx.canvas.stroke_round_rect(p.clear, t.round(10), 2, ink);
        cx.text(FontId::Bold, 14.0, p.clear, Align::Center, ink, "Clear");
    }

    /// Smart loop end's chip from `x`, ending by `right`, on the header
    /// `h`: its words and its button, the words cut to fit (nothing if the
    /// button doesn't). Where the two went, for a finger.
    fn paint_chip(
        &self,
        cx: &mut PaintCx,
        c: &SmartChip,
        x: i32,
        right: i32,
        h: Rect,
    ) -> Option<(Rect, Rect)> {
        let t = cx.theme.clone();
        let (label, _, filled) = &c.action;
        let bw = cx.measure(FontId::Bold, 14.0, label) + 28;
        let room = right - x - (4 + 10 + 4 + 10 + 3) - bw;
        if room < 24 {
            return None;
        }
        let ww = cx.measure(FontId::Bold, 14.0, &c.words).min(room);
        let chip = Rect::new(x, h.y + 2, 4 + 10 + ww + 4 + 10 + bw + 3, 32);
        let open = self.point_shown().is_some();
        let fill = t.accent.mix(t.bg, if open { 0.6 } else { 0.8 });
        cx.canvas.fill_round_rect(chip, 16, fill);
        cx.canvas.stroke_round_rect(chip, 16, 1, t.accent);
        // Words on the chip's tint, moved only as far as it takes to read
        // where the accent is dark (Slate's tint is a mid blue-grey).
        let lit_ink = t.readable(t.text.mix(t.accent, 0.25), fill);
        let words = Rect::new(chip.x + 4, chip.y + 3, 10 + ww + 4, 26);
        cx.text(
            FontId::Bold,
            14.0,
            Rect::new(words.x + 10, words.y, ww + 2, words.h),
            Align::Left,
            if open {
                t.readable(t.text, fill)
            } else {
                lit_ink
            },
            &c.words,
        );
        let b = Rect::new(words.right() + 10, chip.y + 3, bw, 26);
        if *filled {
            cx.canvas.fill_round_rect(b, 13, t.accent);
            cx.text(FontId::Bold, 14.0, b, Align::Center, t.on_accent, label);
        } else {
            cx.canvas.stroke_round_rect(b, 13, 1, t.accent);
            cx.text(FontId::Bold, 14.0, b, Align::Center, lit_ink, label);
        }
        // A finger has the header's height, split between the two.
        let split = words.right() + 5;
        Some((
            Rect::new(chip.x, h.y, split - chip.x, h.h),
            Rect::new(split, h.y, chip.right() - split, h.h),
        ))
    }

    /// The loop point view in `a`: the start's and the end's windows
    /// with your press and tap, what was found, what the other side
    /// makes of the loop, the two choices, A/B and Read as (Jay's OK on
    /// the drawings, 22 Nov).
    fn paint_point(&self, cx: &mut PaintCx, a: Rect, look: &PointLook) {
        let t = cx.theme.clone();
        let pp = LoopEditor::point_parts(a, look);
        cx.canvas.fill_round_rect(a, 10, t.surface);
        cx.canvas.stroke_round_rect(a, 10, 1, t.outline);
        let lw = cx.measure(FontId::Bold, 11.5, "LOOP POINT");
        cx.text(
            FontId::Bold,
            11.5,
            Rect::new(pp.head.x, pp.head.y, lw + 2, pp.head.h),
            Align::Left,
            t.text_dim,
            "LOOP POINT",
        );
        cx.text(
            FontId::Sans,
            14.0,
            Rect::new(
                pp.head.x + lw + 14,
                pp.head.y,
                pp.step_label.x - 12 - (pp.head.x + lw + 14),
                pp.head.h,
            ),
            Align::Left,
            t.text_dim,
            &look.now,
        );
        cx.text(
            FontId::Bold,
            11.5,
            pp.step_label,
            Align::Right,
            t.text_dim,
            "STEP",
        );
        let steps: Vec<(Rect, &str, bool)> = pp
            .steps
            .iter()
            .zip(MoveStep::ALL)
            .map(|(r, st)| (*r, st.label(), st == look.step))
            .collect();
        segments(cx, &steps);
        cx.canvas.fill_round_rect(pp.close, 8, t.raised);
        cx.text(FontId::Sans, 18.0, pp.close, Align::Center, t.text, "×");
        for (pane, title, win) in [
            (&look.start, pp.titles.0, pp.start),
            (&look.end, pp.titles.1, pp.end),
        ] {
            point_pane(cx, pane, title, win);
        }
        for [earlier, later] in pp.nudge {
            button(cx, earlier, "", true, false, 14.0);
            step_icon(cx, earlier, true, t.text);
            button(cx, later, "", true, false, 14.0);
            step_icon(cx, later, false, t.text);
        }
        cx.text(
            FontId::Sans,
            13.0,
            pp.line,
            Align::Left,
            t.text_dim,
            &look.line,
        );
        for (i, (b, (label, _))) in pp.pick.iter().zip(&look.pick).enumerate() {
            // The first is what the view is for, filled; alone (nothing
            // to snap to), it isn't.
            button(cx, *b, label, true, i == 0 && look.ab.is_some(), 15.0);
        }
        if let (Some(b), Some(on)) = (pp.ab, look.ab) {
            button(cx, b, "A/B", true, on, 15.0);
        }
        cx.canvas.fill_rect(pp.rule, t.outline);
        cx.text(
            FontId::Bold,
            11.5,
            pp.read_label,
            Align::Left,
            t.text_dim,
            "READ AS",
        );
        let items: Vec<(Rect, &str, bool)> = pp
            .read_as
            .iter()
            .zip(&look.read_as)
            .map(|(r, (label, bars))| (*r, label.as_str(), *bars == look.bars_now))
            .collect();
        segments(cx, &items);
    }

    fn paint_ruler(&self, cx: &mut PaintCx, p: &EditorParts) {
        let t = cx.theme.clone();
        let r = p.ruler;
        if self.layers.is_empty() {
            return;
        }
        let bars = self.bars.max(1);
        for b in 0..bars {
            let x = r.x + (b as f32 / bars as f32 * r.w as f32) as i32;
            cx.text(
                FontId::Bold,
                12.0,
                Rect::new(x + 7, r.y, 40, r.h),
                Align::Left,
                t.text_faint,
                &format!("{}", b + 1),
            );
        }
        if let Some(n) = self.nudging.map(|n| n.2).filter(|&n| n != 0) {
            let s = if n.abs() == 1 { "" } else { "s" };
            let label = format!(
                "nudge {} {} 1/16{s}",
                if n < 0 { "←" } else { "→" },
                n.abs()
            );
            cx.text(FontId::Bold, 12.5, r, Align::Center, t.accent, &label);
        }
        if let Some(ticks) = self.trim_ticks.filter(|&t| t > 0) {
            let (from, to) = self.trimming.map_or((0, ticks), |t| (t.2, t.3));
            let x_of = |tick: i64| r.x + (tick as f64 / ticks as f64 * r.w as f64) as i32;
            let (start, end) = LoopEditor::trim_grips(r);
            for (g, x) in [(start, x_of(from)), (end, x_of(to))] {
                let g = Rect::new(x - g.w / 2, g.y, g.w, g.h);
                cx.canvas.fill_round_rect(g, 3, t.accent);
            }
            if self.trimming.is_some() {
                let shade = t.bg.mix(t.surface, 0.15).with_alpha(200);
                let (a, b) = (x_of(from), x_of(to));
                let lanes = p.lanes;
                if a > r.x {
                    cx.canvas
                        .fill_rect(Rect::new(r.x, lanes.y, a - r.x, lanes.h), shade);
                }
                if b < r.right() {
                    cx.canvas
                        .fill_rect(Rect::new(b, lanes.y, r.right() - b, lanes.h), shade);
                }
                let beats = |tk: i64| {
                    let b = tk as f64 / mpfree_transport::time::PPQ as f64;
                    format!("{b:.2}")
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .to_string()
                };
                let label = format!("keep {} of {} beats", beats(to - from), beats(ticks));
                cx.text(FontId::Bold, 12.5, r, Align::Center, t.text, &label);
            }
        }
    }

    fn paint_lane(&self, cx: &mut PaintCx, k: usize, rr: Rect, play: Option<i32>) {
        let t = cx.theme.clone();
        let l = &self.layers[k];
        let p = LoopEditor::lane_parts(rr);
        let picked = k == self.picked;
        cx.canvas
            .fill_round_rect(rr, t.round(10), t.bg.mix(t.surface, 0.45));
        // The head: a tap mutes, S solos.
        let hb = if l.muted {
            t.bad.mix(t.bg, 0.82)
        } else {
            t.surface.mix(t.raised, 0.4)
        };
        cx.canvas
            .fill_round_rect(p.head.inset_xy(2, 2), t.round(9), hb);
        let strip_c = if l.heard { l.colour } else { t.text_faint };
        cx.canvas.fill_round_rect(
            Rect::new(p.head.x + 3, p.head.y + 8, 4, p.head.h - 16),
            2,
            strip_c,
        );
        let top = rr.y + (rr.h - 46) / 2;
        speaker_on(
            cx,
            p.head.x + 24,
            top + 11,
            if l.muted { t.bad } else { t.text_dim },
            l.muted,
            hb,
        );
        let name_w = p.listen.x - (p.head.x + 40) - 4;
        cx.text(
            FontId::Bold,
            16.0,
            Rect::new(p.head.x + 40, top, name_w, 22),
            Align::Left,
            if l.heard { t.text } else { t.text_dim },
            &l.name,
        );
        cx.text(
            FontId::Sans,
            12.0,
            Rect::new(p.head.x + 14, top + 26, p.listen.x - p.head.x - 18, 18),
            Align::Left,
            if l.recording { t.warn } else { t.text_dim },
            &l.tag,
        );
        // ▶: this layer alone while the music is stopped (lit while it
        // plays; faint while the music plays).
        cx.canvas.fill_round_rect(
            p.listen,
            t.round(8),
            if l.listening { t.picked() } else { t.raised },
        );
        let ink = if l.listening {
            t.on_picked()
        } else {
            t.button_ink(self.listen.0)
        };
        let cy = p.listen.y as f32 + p.listen.h as f32 / 2.0;
        if l.listening {
            let c = p.listen.x + p.listen.w / 2;
            cx.canvas
                .fill_rect(Rect::new(c - 5, cy as i32 - 5, 10, 10), ink);
        } else {
            play_icon(
                cx,
                (p.listen.x + p.listen.w / 2) as f32 - 4.0,
                cy,
                12.0,
                ink,
            );
        }
        cx.canvas.fill_round_rect(
            p.solo,
            t.round(8),
            if l.solo { t.picked() } else { t.raised },
        );
        cx.text(
            FontId::Bold,
            14.0,
            p.solo,
            Align::Center,
            if l.solo { t.on_picked() } else { t.text },
            "S",
        );
        // The wave, with the bar lines and the playhead.
        let w = p.wave;
        let bars = self.bars.max(1);
        for b in 1..bars {
            let bx = w.x + (b as f32 / bars as f32 * w.w as f32) as i32;
            cx.canvas
                .fill_rect(Rect::new(bx, w.y, 1, w.h), t.outline.with_alpha(160));
        }
        if l.recording {
            if let Some(px) = play {
                cx.canvas.fill_rect(
                    Rect::new(w.x, w.y + 2, (px - w.x).max(0), w.h - 4),
                    t.warn.mix(t.bg, 0.9),
                );
            }
        }
        let ink = if !l.heard {
            t.text_faint.mix(t.surface, 0.3)
        } else {
            l.colour
                .mix(t.surface, (1.0 - l.faded.clamp(0.0, 1.0)) * 0.6)
        };
        if let Some(wave) = self.wave.as_ref().filter(|wv| wv.frames > 0) {
            if let Some(peaks) = wave.layers.get(k) {
                let gain = (l.level * l.faded).clamp(0.0, 2.0) * 1.6;
                let upto = l.recording.then_some(play).flatten();
                paint_lane_wave(cx, w.inset_xy(4, 0), peaks, ink, gain, upto);
            }
        }
        if let Some(px) = play {
            cx.canvas
                .fill_rect(Rect::new(px - 1, rr.y + 2, 2, rr.h - 4), t.text);
        }
        if l.recording {
            cx.canvas.stroke_round_rect(rr, t.round(10), 2, t.warn);
        } else if picked {
            cx.canvas.stroke_round_rect(rr, t.round(10), 2, t.accent);
            cx.canvas
                .fill_round_rect(p.delete, t.round(8), t.raised.with_alpha(235));
            cx.text(FontId::Bold, 18.0, p.delete, Align::Center, t.text, "×");
        }
        // Held long enough to lift: raised, ready to drag.
        if self.press.is_some_and(|q| q.k == k && q.ready && !q.lifted) {
            cx.canvas
                .stroke_round_rect(rr.inset(-2), t.round(12), 3, t.accent);
        }
        // A lifted layer's old place.
        if self.drag().is_some_and(|(dk, _)| dk == k) {
            cx.canvas
                .fill_round_rect(rr, t.round(10), t.bg.mix(t.surface, 0.3));
            cx.canvas
                .stroke_round_rect(rr, t.round(10), 2, t.text_faint);
        }
    }

    fn paint_map(&self, cx: &mut PaintCx, p: &EditorParts, fit: usize) {
        let t = cx.theme.clone();
        let n = self.layers.len();
        if n <= fit {
            return;
        }
        let m = p.map;
        cx.canvas.fill_round_rect(m, 7, t.bg.mix(t.surface, 0.35));
        let tick = (m.h - 8) as f32 / n as f32;
        for d in 0..n {
            let Some(k) = self.layer_at(d) else { continue };
            let l = &self.layers[k];
            let c = if l.recording {
                t.warn
            } else if l.heard {
                l.colour
            } else {
                t.text_faint
            };
            let y = m.y + 4 + (d as f32 * tick) as i32;
            cx.canvas
                .fill_round_rect(Rect::new(m.x + 3, y + 1, 8, (tick as i32 - 3).max(2)), 2, c);
        }
        let y = m.y + 2 + (self.top as f32 * tick) as i32;
        let h = (fit as f32 * tick) as i32 + 4;
        cx.canvas
            .stroke_round_rect(Rect::new(m.x - 1, y, 16, h), t.round(6), 2, t.text);
        // The older layers continue below.
        let older = n.saturating_sub(self.top + fit);
        if older > 0 {
            let s = if older == 1 { "" } else { "s" };
            let label = format!("↓ {older} older layer{s}: swipe up");
            let lanes = p.lanes;
            let wx = lanes.x + LANE_HEAD + 18;
            let r = Rect::new(wx, lanes.bottom() - 16, 260, 16);
            let w = cx.measure(FontId::Bold, 12.0, &label);
            cx.canvas.fill_round_rect(
                Rect::new(r.x - 6, r.y - 1, w + 12, r.h + 2),
                t.round(6),
                t.bg.with_alpha(220),
            );
            cx.text(FontId::Bold, 12.0, r, Align::Left, t.text, &label);
        }
    }

    /// What you played's window as it shows: where a finger has it while
    /// it is moved or resized, else what Capture takes now.
    fn shown_take(&self) -> Option<(f32, f32, String)> {
        let e = self.empty.as_ref()?;
        match self.window_drag {
            Some(d) => {
                let (first, bars) = d.now;
                let a = e.bar_x(first)?;
                let b = e.bar_x(first + i64::from(bars))?;
                Some((a, b, e.words(first, bars)))
            }
            None => e.take.clone(),
        }
    }

    /// The window's edges, with grips to take them by (above the button,
    /// where a finger finds them), and a line where each repeat of the
    /// pattern starts inside it.
    fn paint_window(&self, cx: &mut PaintCx, lanes: Rect, e: &EmptyLook, a: f32, b: f32) {
        let t = cx.theme.clone();
        let r = window_rect(lanes, a, b);
        let w = empty_wave_rect(lanes);
        let at = |f: f32| w.x + (f.clamp(0.0, 1.0) * w.w as f32) as i32;
        let window = self.window_drag.map(|d| d.now).or(e.window);
        if let Some((first, bars)) = window.filter(|_| e.unit > 1) {
            let unit = i64::from(e.unit);
            let mut k = unit;
            while k < i64::from(bars) {
                if let Some(x) = e.bar_x(first + k) {
                    let x = at(x);
                    let mut y = r.y;
                    while y < r.bottom() {
                        cx.canvas
                            .fill_rect(Rect::new(x - 1, y, 2, 6.min(r.bottom() - y)), t.accent);
                        y += 10;
                    }
                }
                k += unit;
            }
        }
        for x in [r.x, r.right() - 2] {
            cx.canvas.fill_rect(Rect::new(x, r.y, 2, r.h), t.accent);
        }
        for x in [r.x - 5, r.right() - 7] {
            cx.canvas
                .fill_round_rect(Rect::new(x, r.y + 14, 12, 34), 6, t.accent);
        }
    }

    fn paint_empty(&self, cx: &mut PaintCx, lanes: Rect, e: &EmptyLook) {
        let t = cx.theme.clone();
        cx.canvas
            .fill_round_rect(lanes, t.round(10), t.bg.mix(t.surface, 0.45));
        let inner = lanes.inset_xy(14, 10);
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(inner.x, inner.y, inner.w, 16),
            Align::Left,
            t.text_dim,
            &e.title,
        );
        // What Capture takes (its window), behind the input.
        let w = empty_wave_rect(lanes);
        let at = |f: f32| w.x + (f.clamp(0.0, 1.0) * w.w as f32) as i32;
        let shown = self.shown_take();
        if let Some((a, b, _)) = &shown {
            let (a, b) = (at(*a), at(*b));
            cx.canvas.fill_rect(
                Rect::new(a, w.y, (b - a).max(1), w.h),
                t.accent.mix(t.bg, 0.78),
            );
        }
        // The bar lines and their numbers.
        for (bar, f) in &e.bars {
            let x = at(*f);
            cx.canvas.fill_rect(Rect::new(x, w.y, 1, w.h), t.outline);
            cx.text(
                FontId::Bold,
                12.0,
                Rect::new(x + 5, w.y + 2, 40, 14),
                Align::Left,
                t.text_dim,
                &bar.to_string(),
            );
        }
        // The input's level over the last 8 bars, now at the right.
        let n = e.input.len().max(1);
        let mid = w.y + w.h / 2;
        let half = w.h as f32 / 2.0 - 2.0;
        let cw = (w.w / n as i32).max(2);
        for (i, v) in e.input.iter().enumerate() {
            let x = w.x + (i as f32 / n as f32 * w.w as f32) as i32;
            let h = ((v.clamp(0.0, 1.0) * half) as i32).max(1);
            cx.canvas.fill_rect(
                Rect::new(x, mid - h, (cw - 1).max(1), 2 * h),
                self.colour.mix(t.surface, 0.2),
            );
        }
        if let Some((a, b, _)) = &shown {
            self.paint_window(cx, lanes, e, *a, *b);
        }
        if let Some((a, b, label)) = &shown {
            let b = capture_button(lanes, *a, *b, label);
            let label = label.as_str();
            cx.canvas.fill_round_rect(b, t.round(10), t.accent);
            let tw = cx.measure(FontId::Bold, 15.0, label);
            let icon_x = b.x + (b.w - tw - 26) / 2;
            let icon = Rect::new(icon_x, b.y + b.h / 2 - 9, 18, 18);
            crate::looper_deck::capture_icon(cx, icon, t.on_accent);
            cx.text(
                FontId::Bold,
                15.0,
                Rect::new(icon.right() + 8, b.y, tw + 4, b.h),
                Align::Left,
                t.on_accent,
                label,
            );
        }
        cx.canvas
            .fill_rect(Rect::new(w.right() - 2, w.y, 2, w.h), t.text);
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(w.right() - 40, inner.y, 36, 16),
            Align::Right,
            t.text,
            "now",
        );
        let says = Rect::new(inner.x, inner.bottom() - 40, inner.w, 20);
        cx.text(FontId::Bold, 15.0, says, Align::Center, t.text, &e.says);
        if let Some(waits) = &e.waits {
            let r = Rect::new(inner.x, inner.bottom() - 16, inner.w, 16);
            cx.text(FontId::Bold, 13.0, r, Align::Right, t.bad, waits);
        }
    }

    fn paint_history(&self, cx: &mut PaintCx, rows: Rect) {
        let t = cx.theme.clone();
        let Some(h) = &self.history else { return };
        let shown = self.step_rects(rows);
        for &(k, sr) in &shown {
            let now = k == h.now;
            let after = k > h.now;
            cx.canvas.fill_round_rect(
                sr,
                t.round(8),
                if now {
                    t.raised
                } else {
                    t.bg.mix(t.surface, 0.6)
                },
            );
            if now {
                cx.canvas.stroke_round_rect(sr, t.round(8), 2, t.accent);
            }
            let ink = if after { t.text_faint } else { t.text };
            let label = Rect::new(sr.x + 14, sr.y, sr.w - 120, sr.h);
            cx.text(FontId::Bold, 15.0, label, Align::Left, ink, &h.rows[k]);
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

    fn paint_send(&self, cx: &mut PaintCx, lanes: Rect) {
        let t = cx.theme.clone();
        cx.canvas
            .fill_round_rect(lanes, t.round(10), t.bg.mix(t.surface, 0.45));
        for (i, (b, (title, what))) in LoopEditor::send_rects(lanes)
            .into_iter()
            .zip(SEND_CHOICES)
            .enumerate()
        {
            let on = self.send_built[i];
            cx.canvas.fill_round_rect(b, t.round(12), t.raised);
            cx.text(
                FontId::Bold,
                18.0,
                Rect::new(b.x, b.y + b.h / 2 - 24, b.w, 26),
                Align::Center,
                t.button_ink(on),
                title,
            );
            let what = if on {
                what.to_string()
            } else {
                format!("{what} · not built yet")
            };
            cx.text(
                FontId::Sans,
                13.0,
                Rect::new(b.x + 10, b.y + b.h / 2 + 4, b.w - 20, 18),
                Align::Center,
                t.text_dim,
                &what,
            );
        }
    }

    fn paint_strip(&self, cx: &mut PaintCx, s: Rect) {
        let t = cx.theme.clone();
        let ctls = self.strip_controls(s);
        let rect = |c: Ctl| ctls.iter().find(|x| x.0 == c).map(|x| x.1);
        match &self.strip {
            Strip::Plays {
                feedback,
                overdubs,
                reversed,
                speed,
                length,
                chips,
                merge,
                ..
            } => {
                group_label(cx, s.x, s.y, "FEEDBACK");
                let od: Vec<(Rect, &str, bool)> = ctls
                    .iter()
                    .filter_map(|c| match c.0 {
                        Ctl::Overdubs(i) => Some((c.1, c.2.as_str(), i == *overdubs)),
                        _ => None,
                    })
                    .collect();
                if let Some(first) = od.first() {
                    cx.canvas
                        .fill_rect(Rect::new(first.0.x - 17, s.y + 2, 1, s.h - 4), t.outline);
                    group_label(cx, first.0.x - 3, s.y, "OVERDUBS");
                }
                segments(cx, &od);
                if let Some(k) = rect(Ctl::Feedback) {
                    let v = (feedback - 0.5) / 0.5;
                    let lit = if *feedback < 0.999 { t.warn } else { t.accent };
                    let d = 46;
                    knob(
                        cx,
                        (k.x + 4 + d / 2, k.y + 4 + d / 2),
                        d,
                        v,
                        lit,
                        &format!("{:.0} %", 100.0 * feedback),
                    );
                    if self.feedback.is_some() {
                        cx.canvas.stroke_round_rect(k, t.round(10), 2, t.accent);
                    }
                }
                let on = |c: Ctl| ctls.iter().find(|x| x.0 == c).is_some_and(|x| x.3);
                if let Some(b) = rect(Ctl::Reverse) {
                    group_label(cx, b.x, s.y, "PLAY IT");
                    cx.canvas.fill_round_rect(b, t.round(10), t.raised);
                    let mut d = Shape::new(Rect::new(b.x + 12, b.y + 16, 16, 16));
                    d.disc(b.x as f32 + 20.0, b.y as f32 + b.h as f32 / 2.0, 4.5);
                    d.draw(cx.canvas, if *reversed { t.accent } else { t.outline });
                    cx.text(
                        FontId::Bold,
                        15.0,
                        Rect::new(b.x + 30, b.y, b.w - 34, b.h),
                        Align::Center,
                        t.button_ink(on(Ctl::Reverse)),
                        "Reverse",
                    );
                }
                let sp: Vec<(Rect, &str, bool)> = ctls
                    .iter()
                    .filter_map(|c| match c.0 {
                        Ctl::Speed(i) => Some((c.1, SPEED_CHOICES[i as usize], i == *speed)),
                        _ => None,
                    })
                    .collect();
                if let Some(first) = sp.first() {
                    group_label(cx, first.0.x - 3, s.y, "SPEED");
                }
                segments(cx, &sp);
                if let (Some(h), Some(d)) = (rect(Ctl::Half), rect(Ctl::Double)) {
                    group_label(cx, h.x, s.y, "LENGTH");
                    button(cx, h, "½", on(Ctl::Half), false, 17.0);
                    cx.text(
                        FontId::Bold,
                        14.0,
                        Rect::new(h.right(), h.y, d.x - h.right(), h.h),
                        Align::Center,
                        t.text,
                        length,
                    );
                    button(cx, d, "×2", on(Ctl::Double), false, 16.0);
                }
                if let (Some(u), Some(hi), Some(re)) =
                    (rect(Ctl::Undo), rect(Ctl::History), rect(Ctl::Redo))
                {
                    let div = u.x - 16;
                    cx.canvas
                        .fill_rect(Rect::new(div, s.y + 2, 1, s.h - 4), t.outline);
                    group_label(cx, u.x, s.y, "HISTORY");
                    button(cx, u, "Undo", on(Ctl::Undo), false, 14.0);
                    button(cx, re, "Redo", on(Ctl::Redo), false, 14.0);
                    let shown = LoopEditor::chips_shown(chips.len(), hi.w - 6);
                    let first = chips.len() - shown;
                    for (i, c) in chips[first..].iter().enumerate() {
                        let b = Rect::new(hi.x + 3 + i as i32 * 17, hi.y + 8, 12, hi.h - 16);
                        match c.kind {
                            ChipKind::Done => cx.canvas.fill_round_rect(b, 3, c.colour),
                            ChipKind::Recording => cx.canvas.stroke_round_rect(b, 3, 2, t.warn),
                            ChipKind::Undone => {
                                cx.canvas
                                    .stroke_round_rect(b, 3, 2, c.colour.mix(t.surface, 0.4))
                            }
                        }
                    }
                    if chips.is_empty() {
                        cx.text(FontId::Sans, 12.0, hi, Align::Center, t.text_faint, "–");
                    }
                    if self.history.is_some() {
                        cx.canvas.stroke_round_rect(hi, t.round(8), 2, t.accent);
                    }
                }
                if let Some(m) = rect(Ctl::Merge) {
                    button(cx, m, &merge.0, merge.1, false, 15.0);
                }
                let first_div = rect(Ctl::Reverse).map_or(s.x, |b| b.x - 14);
                cx.canvas
                    .fill_rect(Rect::new(first_div, s.y + 2, 1, s.h - 4), t.outline);
            }
            Strip::Records {
                monitor,
                bars,
                starts,
                then,
                sync,
                ..
            } => {
                let sy: Vec<(Rect, &str, bool)> = ctls
                    .iter()
                    .filter_map(|c| match c.0 {
                        Ctl::Sync(i) => Some((c.1, c.2.as_str(), i == sync.0)),
                        _ => None,
                    })
                    .collect();
                if let Some(first) = sy.first() {
                    group_label(cx, first.0.x - 3, s.y, "SYNC");
                }
                if sync.1 {
                    segments(cx, &sy);
                } else {
                    faint_segments(cx, &sy);
                }
                if let (Some(on), Some(m)) = (monitor, rect(Ctl::Monitor)) {
                    cx.canvas
                        .fill_rect(Rect::new(m.x - 18, s.y + 2, 1, s.h - 4), t.outline);
                    group_label(cx, m.x, s.y, "MONITOR");
                    let sw = Rect::new(m.x, m.y + (m.h - 34) / 2, 62, 34);
                    cx.canvas
                        .fill_round_rect(sw, 17, if *on { t.picked() } else { t.raised });
                    let kx = if *on { sw.x + 32 } else { sw.x + 4 };
                    cx.canvas
                        .fill_round_rect(Rect::new(kx, sw.y + 4, 26, 26), 13, t.text);
                }
                let len: Vec<(Rect, &str, bool)> = ctls
                    .iter()
                    .filter_map(|c| match c.0 {
                        Ctl::Bars(i) => Some((c.1, c.2.as_str(), i == *bars)),
                        _ => None,
                    })
                    .collect();
                if let Some(first) = len.first() {
                    if first.0.x - 18 > s.x {
                        cx.canvas
                            .fill_rect(Rect::new(first.0.x - 18, s.y + 2, 1, s.h - 4), t.outline);
                    }
                    group_label(cx, first.0.x - 3, s.y, "LENGTH");
                }
                segments(cx, &len);
                let st: Vec<(Rect, &str, bool)> = ctls
                    .iter()
                    .filter_map(|c| match c.0 {
                        Ctl::Starts(i) => Some((c.1, c.2.as_str(), i == starts.0)),
                        _ => None,
                    })
                    .collect();
                if let Some(first) = st.first() {
                    group_label(cx, first.0.x - 3, s.y, "STARTS");
                }
                if starts.1 {
                    segments(cx, &st);
                } else {
                    faint_segments(cx, &st);
                }
                let th: Vec<(Rect, &str, bool)> = ctls
                    .iter()
                    .filter_map(|c| match c.0 {
                        Ctl::Then(i) => Some((c.1, c.2.as_str(), i == *then)),
                        _ => None,
                    })
                    .collect();
                if let Some(first) = th.first() {
                    group_label(cx, first.0.x - 3, s.y, "THEN");
                }
                segments(cx, &th);
            }
        }
    }
}

impl Widget for LoopEditor {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let p = LoopEditor::parts(r);
        self.paint_head(cx, &p);
        let (fit, _) = LoopEditor::lane_fit(p.lanes);
        let wave_r = Rect::new(p.ruler.x, p.lanes.y, p.ruler.w, p.lanes.h);
        let play = self.play_x(wave_r.inset_xy(4, 0));
        if let Some(now) = self.sources {
            self.paint_sources(cx, p.lanes, now);
        } else if self.send_open {
            self.paint_send(cx, p.lanes);
        } else if self.history.is_some() {
            self.paint_history(cx, p.lanes);
        } else if self.layers.is_empty() {
            if let Some(e) = &self.empty {
                self.paint_empty(cx, p.lanes, e);
            }
        } else if let Some(look) = self.point_shown() {
            self.paint_point(cx, p.point, look);
        } else {
            self.paint_ruler(cx, &p);
            for (k, rr) in self.lane_rects(p.lanes) {
                self.paint_lane(cx, k, rr, play);
            }
            self.paint_map(cx, &p, fit);
            if self.trimming.is_some() {
                self.paint_ruler(cx, &p);
            }
        }
        self.paint_strip(cx, p.strip);
        let over = self.finger().filter(|&(x, y)| r.contains(x, y));
        if let Some(((k, card), at)) = self.drag().zip(over) {
            let x = card.x.clamp(r.x + 8, r.x + r.w - 8 - card.w);
            let card = Rect::new(x, card.y, card.w, card.h);
            let d = LayerDrag {
                row: self.layers[k.min(self.layers.len().saturating_sub(1))].clone(),
                card,
                at,
                says: self.says.clone(),
                landing: self.landing().map(|d| (d.to, d.refused.is_none())),
                lit: Vec::new(),
            };
            paint_card(cx, &d);
        }
        *self.drawn.borrow_mut() = Some((r, self.look(), play.unwrap_or(-1)));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, look, old_x) = drawn.as_ref()?;
        if *at != r || *look != self.look() {
            return None;
        }
        let p = LoopEditor::parts(r);
        let wave_r = Rect::new(p.ruler.x, p.lanes.y, p.ruler.w, p.lanes.h);
        let new_x = self.play_x(wave_r.inset_xy(4, 0)).unwrap_or(-1);
        // The loop point view has no playhead.
        if new_x == *old_x || self.layers.is_empty() || self.point_shown().is_some() {
            return Some(Vec::new());
        }
        // A lane recording grows with the playhead: its lanes whole.
        if *old_x < 0 || new_x < 0 || new_x < *old_x || self.layers.iter().any(|l| l.recording) {
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
        let p = LoopEditor::parts(r);
        let listen = match self.free {
            Some((runs, cued)) => Target::tap(
                "free-loops",
                0,
                match (runs, cued) {
                    (true, _) => "Stop the free loops",
                    (false, true) => "Call off the free loops' Play",
                    (false, false) => "Play the free loops",
                },
                p.listen,
            ),
            None => Target::tap(
                "listen",
                0,
                if self.listen.1 {
                    "Stop listening"
                } else {
                    "Listen"
                },
                p.listen,
            )
            .disabled_if(!self.listen.0 && !self.listen.1),
        };
        let mut out = vec![
            listen,
            Target::tap("send", 0, "Send", p.send).disabled_if(!self.has_sound),
            Target::tap("clear", 0, "Clear", p.clear).disabled_if(!self.has_sound),
        ];
        if let (Some(c), Some((words, act))) = (&self.chip, *self.chip_at.borrow()) {
            out.push(Target::tap("chip", 0, c.words.clone(), words));
            out.push(Target::tap("chip-act", 0, c.action.0.clone(), act));
        }
        for (c, b, label, on) in self.strip_controls(p.strip) {
            let t = match c {
                Ctl::Feedback => Target::drag("feedback", 0, label, b),
                Ctl::Overdubs(i) => Target::tap("overdubs", i as usize, label, b),
                Ctl::Reverse => Target::tap("reverse", 0, label, b),
                Ctl::Speed(i) => Target::tap("speed", i as usize, label, b),
                Ctl::Half => Target::tap("half", 0, label, b),
                Ctl::Double => Target::tap("double", 0, label, b),
                Ctl::Undo => Target::tap("undo", 0, label, b),
                Ctl::History => Target::tap("history", 0, label, b),
                Ctl::Redo => Target::tap("redo", 0, label, b),
                Ctl::Merge => Target::tap("merge", 0, label, b),
                Ctl::Source(i) => Target::tap("source", i as usize, label, b),
                Ctl::Sync(i) => Target::tap("sync", i as usize, label, b),
                Ctl::Monitor => Target::tap("monitor", 0, label, b),
                Ctl::Bars(i) => Target::tap("bars", i as usize, label, b),
                Ctl::Starts(i) => Target::tap("starts", i as usize, label, b),
                Ctl::Then(i) => Target::tap("then", i as usize, label, b),
                Ctl::Input | Ctl::Listen | Ctl::FreeGo | Ctl::Send | Ctl::Clear => continue,
            };
            let chosen = match (&self.strip, c) {
                (Strip::Records { source, .. }, Ctl::Source(i)) => *source == i,
                (Strip::Records { sync, .. }, Ctl::Sync(i)) => sync.0 == i,
                (Strip::Records { bars, .. }, Ctl::Bars(i)) => *bars == i,
                (Strip::Records { starts, .. }, Ctl::Starts(i)) => starts.0 == i,
                (Strip::Records { then, .. }, Ctl::Then(i)) => *then == i,
                (Strip::Plays { overdubs, .. }, Ctl::Overdubs(i)) => *overdubs == i,
                (Strip::Plays { speed, .. }, Ctl::Speed(i)) => *speed == i,
                _ => false,
            };
            out.push(t.disabled_if(!on || chosen));
        }
        if let Some(now) = self.sources {
            let rects = LoopEditor::source_rects(p.lanes, self.mixes.len());
            for (i, b) in rects.into_iter().enumerate() {
                let label = SOURCE_CHOICES
                    .get(i)
                    .map(|c| c.0)
                    .or_else(|| self.mixes.get(i - SOURCE_CHOICES.len()).map(String::as_str))
                    .unwrap_or("");
                out.push(Target::tap("source", i, label, b).disabled_if(i == now));
            }
            return out;
        }
        if self.send_open {
            for (i, b) in LoopEditor::send_rects(p.lanes).into_iter().enumerate() {
                out.push(
                    Target::tap("send-to", i, SEND_CHOICES[i].0, b)
                        .disabled_if(!self.send_built[i]),
                );
            }
            return out;
        }
        if let Some(h) = &self.history {
            for (k, sr) in self.step_rects(p.lanes) {
                if k != h.now {
                    out.push(Target::tap("history-step", k, h.rows[k].clone(), sr));
                }
            }
            return out;
        }
        if self.layers.is_empty() {
            if let Some(e) = &self.empty {
                if let Some((a, b, label)) = self.shown_take().filter(|_| e.take.is_some()) {
                    let b = capture_button(p.lanes, a, b, &label);
                    out.push(Target::tap("capture", 0, label, b));
                }
            }
            return out;
        }
        if let Some(look) = self.point_shown() {
            let pp = LoopEditor::point_parts(p.point, look);
            out.push(Target::tap(
                "point-close",
                0,
                "Close the loop point view",
                pp.close,
            ));
            for (i, (b, (label, _))) in pp.pick.iter().zip(&look.pick).enumerate() {
                out.push(Target::tap("point-pick", i, label.clone(), *b));
            }
            for (i, (b, st)) in pp.steps.iter().zip(MoveStep::ALL).enumerate() {
                out.push(Target::tap(
                    "point-step",
                    i,
                    format!("Step: {}", st.label()),
                    *b,
                ));
            }
            let nudges = [
                "Move the start earlier",
                "Move the start later",
                "Move the end earlier",
                "Move the end later",
            ];
            for (i, b) in pp.nudge.iter().flatten().enumerate() {
                out.push(Target::tap("point-nudge", i, nudges[i], *b));
            }
            out.push(Target::tap(
                "point-window",
                0,
                "Move the start here",
                pp.start,
            ));
            out.push(Target::tap("point-window", 1, "Move the end here", pp.end));
            if let Some(b) = pp.ab {
                out.push(Target::tap("point-ab", 0, "A/B", b));
            }
            for (i, (r, (label, bars))) in pp.read_as.iter().zip(&look.read_as).enumerate() {
                out.push(
                    Target::tap("read-as", i, label.clone(), *r)
                        .disabled_if(*bars == look.bars_now),
                );
            }
            return out;
        }
        if let Some(ticks) = self.trim_ticks.filter(|&t| t > 0) {
            let (start, end) = LoopEditor::trim_grips(p.ruler);
            out.push(Target::drag("trim-start", 0, "trim start", start));
            // The end grip moves left: a drag right would leave it whole.
            out.push(Target::drag("trim-end", 0, "trim end", end).towards(-60, 0));
            // The ruler between them nudges: a 1/16 note at a time.
            let px = LoopEditor::sixteenth_px(p.ruler, ticks).ceil() as i32 + 2;
            out.push(Target::drag("nudge", 0, "nudge", p.ruler.inset_xy(14, 0)).towards(px, 0));
        }
        let (fit, _) = LoopEditor::lane_fit(p.lanes);
        if self.layers.len() > fit {
            out.push(Target::tap("layer-map", 0, "layer map", p.map));
        }
        for (k, rr) in self.lane_rects(p.lanes) {
            let lp = LoopEditor::lane_parts(rr);
            let name = &self.layers[k].name;
            let label = format!("{name} {}", self.layers[k].tag);
            out.push(Target::tap(
                "layer-mute",
                k,
                format!("{name} speaker"),
                Rect::new(lp.head.x, lp.head.y, lp.listen.x - lp.head.x - 2, lp.head.h),
            ));
            out.push(
                Target::tap("layer-listen", k, format!("{name} listen"), lp.listen)
                    .disabled_if(!self.listen.0 && !self.layers[k].listening),
            );
            out.push(Target::tap("layer-solo", k, format!("{name} S"), lp.solo));
            if k == self.picked && !self.layers[k].recording {
                out.push(Target::tap(
                    "layer-delete",
                    k,
                    format!("{name} delete"),
                    lp.delete,
                ));
            }
            // The wheel's push picks the layer up to drop on another loop.
            out.push(Target::tap("layer", k, label, lp.wave).moves_if(!self.drops.is_empty()));
        }
        out
    }

    fn steps(&self, t: &Target) -> bool {
        (t.id == "layer" && !self.drops.is_empty()) || t.id == "feedback"
    }

    fn step(&mut self, t: &Target, delta: i32, r: Rect) -> Option<EventKind> {
        if t.id == "feedback" {
            return self.held_step(t, delta, false, r);
        }
        if t.id != "layer" || self.drops.is_empty() {
            return None;
        }
        if self.wheel.is_none_or(|(k, _)| k != t.index) {
            self.wheel_grab(t.index);
        }
        let (k, at) = self.wheel?;
        // Round the places, and back to its own place between the ends.
        let n = self.drops.len() as i32 + 1;
        let now = at.map_or(0, |i| i as i32 + 1);
        let next = (now + delta).rem_euclid(n);
        self.wheel = Some((k, (next > 0).then(|| next as usize - 1)));
        Some(EventKind::Select(k))
    }

    fn held(&self, x: i32, y: i32, r: Rect) -> Option<Target> {
        self.targets(r)
            .into_iter()
            .find(|t| t.rect.contains(x, y) && (t.id == "layer" || t.id == "feedback"))
    }

    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, _r: Rect) -> Option<EventKind> {
        match t.id {
            "feedback" => {
                let Strip::Plays { feedback, .. } = &mut self.strip else {
                    return None;
                };
                let step = if fine { 0.002 } else { 0.01 };
                let v = (*feedback + delta as f32 * step).clamp(0.5, 1.0);
                if (v - *feedback).abs() < 1e-6 {
                    return None;
                }
                *feedback = v;
                self.acts.push(LoopAct::Feedback(v));
                Some(EventKind::Value(v))
            }
            "layer" => {
                let k = t.index;
                let l = self.layers.get_mut(k)?;
                // A resting finger and a turn: no lift, no pick.
                self.press = None;
                let cmd = if fine {
                    // SHIFT: the pan, 1 % of the way across a detent.
                    l.pan = (l.pan + delta as f32 * 0.02).clamp(-1.0, 1.0);
                    LayerCommand::Pan(l.pan)
                } else {
                    let db = (20.0 * l.level.max(1e-6).log10()).max(-60.0);
                    let db = (db + delta as f32 * 0.5).clamp(-60.0, 6.0);
                    l.level = if db <= -60.0 {
                        0.0
                    } else {
                        10f32.powf(db / 20.0)
                    };
                    LayerCommand::Level(l.level)
                };
                if self.picked != k {
                    self.picked = k;
                    self.acts.push(LoopAct::Layer(k));
                }
                self.acts.push(LoopAct::LayerCmd(k, cmd));
                Some(EventKind::Select(k))
            }
            _ => None,
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let p = LoopEditor::parts(r);
        match phase {
            Touch::Down => {
                // The header.
                if p.listen.contains(x, y) && self.free.is_some() {
                    self.acts.push(LoopAct::Strip(Ctl::FreeGo));
                    return Some(EventKind::Tab(0));
                }
                if p.listen.contains(x, y) {
                    let (works, on) = self.listen;
                    self.acts.push(if works || on {
                        LoopAct::Strip(Ctl::Listen)
                    } else {
                        LoopAct::Faint(Ctl::Listen)
                    });
                    return Some(EventKind::Tab(0));
                }
                if p.send.contains(x, y) {
                    if !self.has_sound {
                        self.acts.push(LoopAct::Faint(Ctl::Send));
                        return Some(EventKind::Tab(1));
                    }
                    self.send_open = !self.send_open;
                    self.sources = None;
                    return Some(EventKind::Tab(1));
                }
                if p.clear.contains(x, y) {
                    let act = if self.has_sound {
                        LoopAct::Strip(Ctl::Clear)
                    } else {
                        LoopAct::Faint(Ctl::Clear)
                    };
                    self.acts.push(act);
                    return Some(EventKind::Tab(2));
                }
                // Smart loop end's chip: the words open or close the loop
                // point view, the button does what it says.
                let chip_at = *self.chip_at.borrow();
                if let (Some(c), Some((words, act))) = (&self.chip, chip_at) {
                    if words.contains(x, y) {
                        self.point_open = !self.point_open;
                        self.acts.push(LoopAct::PointView(self.point_open));
                        return Some(EventKind::Tab(5));
                    }
                    if act.contains(x, y) {
                        self.acts.push(LoopAct::Chip(c.action.1));
                        return Some(EventKind::Tab(6));
                    }
                }
                // The controls under the lanes.
                if let Some((c, _, _, on)) = self
                    .strip_controls(p.strip)
                    .into_iter()
                    .find(|c| c.1.contains(x, y))
                {
                    if c == Ctl::Feedback {
                        if let Strip::Plays { feedback, .. } = self.strip {
                            self.feedback = Some((finger, y, feedback));
                        }
                        return None;
                    }
                    self.acts.push(if on {
                        LoopAct::Strip(c)
                    } else {
                        LoopAct::Faint(c)
                    });
                    return Some(EventKind::Tab(3));
                }
                if self.sources.is_some() {
                    if let Some(i) = LoopEditor::source_rects(p.lanes, self.mixes.len())
                        .iter()
                        .position(|b| b.contains(x, y))
                    {
                        self.sources = None;
                        self.acts.push(LoopAct::Strip(Ctl::Source(i as u8)));
                        return Some(EventKind::Select(i));
                    }
                    // Anywhere else closes the choices.
                    self.sources = None;
                    return None;
                }
                if self.send_open {
                    if let Some(i) = LoopEditor::send_rects(p.lanes)
                        .iter()
                        .position(|b| b.contains(x, y))
                    {
                        self.acts.push(if self.send_built[i] {
                            self.send_open = false;
                            LoopAct::Send(i)
                        } else {
                            LoopAct::FaintSend(i)
                        });
                        return Some(EventKind::Select(i));
                    }
                    // Anywhere else closes the choices.
                    self.send_open = false;
                    return None;
                }
                if self.history.is_some() {
                    if p.lanes.contains(x, y) {
                        self.history_press = Some((finger, y, self.history_top(p.lanes), false));
                    }
                    return None;
                }
                if self.layers.is_empty() {
                    // What you played's window: its button captures (on
                    // the finger's lift, if it didn't move); a finger on
                    // it moves it, and one by an end resizes it.
                    let e = self.empty.as_ref()?;
                    let (a, b, label) = e.take.clone()?;
                    let win = window_rect(p.lanes, a, b);
                    let on_button = capture_button(p.lanes, a, b, &label).contains(x, y);
                    let in_wave = y >= win.y && y < win.bottom();
                    let grip = if on_button {
                        Some(Grip::Move)
                    } else if in_wave && (x - win.x).abs() <= GRIP_REACH {
                        Some(Grip::Start)
                    } else if in_wave && (x - win.right()).abs() <= GRIP_REACH {
                        Some(Grip::End)
                    } else if win.contains(x, y) {
                        Some(Grip::Move)
                    } else {
                        None
                    };
                    if let (Some(grip), Some(from)) = (grip, e.window) {
                        self.window_drag = Some(WindowDrag {
                            finger,
                            grip,
                            x0: x,
                            from,
                            now: from,
                            button: on_button,
                        });
                    } else if on_button {
                        self.acts.push(LoopAct::Capture);
                        return Some(EventKind::Tab(4));
                    }
                    return None;
                }
                if let Some(look) = self.point_shown().cloned() {
                    let pp = LoopEditor::point_parts(p.point, &look);
                    if pp.close.contains(x, y) {
                        self.close_point();
                        return Some(EventKind::Tab(5));
                    }
                    if let Some(i) = pp.pick.iter().position(|b| b.contains(x, y)) {
                        self.acts.push(match look.pick[i].1 {
                            Pick::Side(side, keep) => LoopAct::Side(side, keep),
                            Pick::Move => LoopAct::Move(MoveAct::Keep),
                            Pick::Cancel => LoopAct::Move(MoveAct::Cancel),
                        });
                        return Some(EventKind::Select(i));
                    }
                    if let Some(i) = pp.steps.iter().position(|b| b.contains(x, y)) {
                        self.acts
                            .push(LoopAct::Move(MoveAct::Scale(MoveStep::ALL[i])));
                        return Some(EventKind::Tab(9));
                    }
                    for (k, edge) in [Edge::Start, Edge::End].into_iter().enumerate() {
                        for (j, d) in [-1i8, 1].into_iter().enumerate() {
                            if pp.nudge[k][j].contains(x, y) {
                                self.acts.push(LoopAct::Move(MoveAct::Step(edge, d)));
                                return Some(EventKind::Tab(10));
                            }
                        }
                        let (win, pane) = match edge {
                            Edge::Start => (pp.start, &look.start),
                            Edge::End => (pp.end, &look.end),
                        };
                        if win.contains(x, y) {
                            let f = point_to(win, pane, x);
                            self.acts.push(LoopAct::Move(MoveAct::To(edge, f)));
                            self.point_drag = Some((finger, edge));
                            return Some(EventKind::Tab(11));
                        }
                    }
                    if let (Some(b), Some(on)) = (pp.ab, look.ab) {
                        if b.contains(x, y) {
                            self.acts.push(LoopAct::Ab(!on));
                            return Some(EventKind::Tab(7));
                        }
                    }
                    let read = pp.read_as.iter().position(|b| b.contains(x, y));
                    if let Some((_, bars)) = read.and_then(|i| look.read_as.get(i)) {
                        if *bars != look.bars_now {
                            let act = LoopAct::Chip(ChipAct::Snap(Snap::Bars(*bars)));
                            self.acts.push(act);
                            return Some(EventKind::Tab(8));
                        }
                    }
                    // The rest of the view is the view.
                    return None;
                }
                if let Some(ticks) = self.trim_ticks.filter(|&t| t > 0) {
                    let (start, end) = LoopEditor::trim_grips(p.ruler);
                    let near = |g: Rect| g.inset_xy(-14, -8).contains(x, y);
                    if near(start) || near(end) {
                        // The nearer end, when both are in reach.
                        let is_end = near(end) && (!near(start) || x > p.ruler.x + p.ruler.w / 2);
                        self.trimming = Some((finger, is_end, 0, ticks));
                        return None;
                    }
                    if p.ruler.inset_xy(0, -6).contains(x, y) {
                        self.nudging = Some((finger, x, 0));
                        return None;
                    }
                }
                let (fit, _) = LoopEditor::lane_fit(p.lanes);
                if self.layers.len() > fit && p.map.inset_xy(-8, 0).contains(x, y) {
                    // The map: jump there, that place in the middle.
                    let n = self.layers.len();
                    let f = (y - p.map.y) as f32 / p.map.h.max(1) as f32;
                    let d = ((f * n as f32) as usize).min(n - 1);
                    self.top = d.saturating_sub(fit / 2).min(n - fit);
                    self.swiped = Some(self.picked);
                    return Some(EventKind::Value(self.top as f32));
                }
                let (k, rr) = self
                    .lane_rects(p.lanes)
                    .into_iter()
                    .find(|(_, rr)| rr.contains(x, y))?;
                let l = self.layers[k].clone();
                let lp = LoopEditor::lane_parts(rr);
                let act = if lp.listen.inset(-3).contains(x, y) {
                    if self.listen.0 || l.listening {
                        LoopAct::LayerListen(k)
                    } else {
                        LoopAct::Faint(Ctl::Listen)
                    }
                } else if lp.solo.inset(-3).contains(x, y) {
                    LoopAct::LayerCmd(k, LayerCommand::Solo(!l.solo))
                } else if lp.head.contains(x, y) {
                    LoopAct::LayerCmd(k, LayerCommand::Mute(!l.muted))
                } else if k == self.picked && !l.recording && lp.delete.contains(x, y) {
                    LoopAct::LayerCmd(k, LayerCommand::Delete)
                } else {
                    // Picked on the lift, unless it moves first: a swipe,
                    // or after a hold, a drag.
                    self.wheel = None;
                    self.press = Some(Press {
                        finger,
                        k,
                        from: (x, y),
                        at: (x, y),
                        grip: (
                            (x - rr.x).clamp(24, CARD_W - 24),
                            (y - rr.y).clamp(12, CARD_H - 12),
                        ),
                        held: 0.0,
                        ready: false,
                        lifted: false,
                        swipe: None,
                    });
                    return None;
                };
                self.acts.push(act);
                Some(EventKind::Select(k))
            }
            Touch::Move if self.feedback.is_some_and(|f| f.0 == finger) => {
                let v = self.feedback_at(y)?;
                let Strip::Plays { feedback, .. } = &mut self.strip else {
                    return None;
                };
                if (v - *feedback).abs() < 0.005 {
                    return None;
                }
                *feedback = v;
                self.acts.push(LoopAct::Feedback(v));
                Some(EventKind::Value(v))
            }
            Touch::Move if self.window_drag.is_some_and(|d| d.finger == finger) => {
                let d = self.window_drag?;
                let e = self.empty.as_ref()?;
                let w = empty_wave_rect(p.lanes);
                let (first, last) = (e.bars.first()?, e.bars.last()?);
                let lines = e.bars.len().checked_sub(1).filter(|&n| n > 0)? as f32;
                let px = ((last.1 - first.1) / lines * w.w as f32).max(1.0);
                let steps = ((x - d.x0) as f32 / px).round() as i64;
                let now = window_after(e, d.grip, d.from, steps);
                if now == d.now {
                    return None;
                }
                self.window_drag = Some(WindowDrag { now, ..d });
                Some(EventKind::Value(now.1 as f32))
            }
            Touch::Move if self.point_drag.is_some_and(|d| d.0 == finger) => {
                let (_, edge) = self.point_drag?;
                let look = self.point_shown()?.clone();
                let pp = LoopEditor::point_parts(p.point, &look);
                let (win, pane) = match edge {
                    Edge::Start => (pp.start, &look.start),
                    Edge::End => (pp.end, &look.end),
                };
                let f = point_to(win, pane, x);
                self.acts.push(LoopAct::Move(MoveAct::To(edge, f)));
                Some(EventKind::Tab(11))
            }
            Touch::Move if self.trimming.is_some_and(|t| t.0 == finger) => {
                let (f, is_end, from, to) = self.trimming?;
                let ticks = self.trim_ticks.filter(|&t| t > 0)?;
                let t = LoopEditor::tick_at(p.ruler, x, ticks);
                let (from, to) = if is_end {
                    (from, t.max(from + SIXTEENTH))
                } else {
                    (t.min(to - SIXTEENTH), to)
                };
                self.trimming = Some((f, is_end, from, to));
                None
            }
            Touch::Move if self.nudging.is_some_and(|n| n.0 == finger) => {
                let (f, x0, _) = self.nudging?;
                let ticks = self.trim_ticks.filter(|&t| t > 0)?;
                let px = LoopEditor::sixteenth_px(p.ruler, ticks).max(1.0);
                let n = ((x - x0) as f32 / px).round() as i32;
                self.nudging = Some((f, x0, n));
                None
            }
            Touch::Move if self.history_press.is_some_and(|p| p.0 == finger) => {
                let (f, y0, top0, moved) = self.history_press?;
                let dy = y - y0;
                if !moved && dy.abs() < mpfree_ui::qlink::DRAG_DETENT {
                    return None;
                }
                let step = STEP_H + STEP_GAP;
                let top = (top0 as i32 - dy / step).max(0) as usize;
                self.history_press = Some((f, y0, top0, true));
                self.history_top = Some(top);
                let top = self.history_top(p.lanes);
                self.history_top = Some(top);
                None
            }
            Touch::Move if self.press.is_some_and(|q| q.finger == finger) => {
                let top = self.top;
                let q = self.press.as_mut()?;
                let (dx, dy) = (x - q.from.0, y - q.from.1);
                q.at = (x, y);
                if q.ready {
                    q.lifted |= dx * dx + dy * dy >= LIFT_PX * LIFT_PX;
                    // The page follows the drag (the deck lights its loops).
                    return q.lifted.then_some(EventKind::Xy {
                        x: x as f32,
                        y: y as f32,
                        down: true,
                    });
                }
                // Moved before the hold: a swipe, the lanes following the
                // finger a lane at a time (up shows the older layers).
                if q.swipe.is_none() && dx.abs().max(dy.abs()) < SWIPE_PX {
                    return None;
                }
                let top0 = *q.swipe.get_or_insert(top);
                let (fit, h) = LoopEditor::lane_fit(p.lanes);
                let last = self.layers.len().saturating_sub(fit);
                let step = h + LANE_GAP;
                let to = (top0 as i32 - dy / step.max(1)).clamp(0, last as i32) as usize;
                self.swiped = Some(self.picked);
                if to == self.top {
                    return None;
                }
                self.top = to;
                Some(EventKind::Value(to as f32))
            }
            Touch::Move => None,
            Touch::Up | Touch::Cancel => {
                if self.feedback.is_some_and(|f| f.0 == finger) {
                    self.feedback = None;
                    return None;
                }
                if self.point_drag.is_some_and(|d| d.0 == finger) {
                    self.point_drag = None;
                    return None;
                }
                if let Some(d) = self.window_drag.filter(|d| d.finger == finger) {
                    self.window_drag = None;
                    if phase == Touch::Cancel {
                        return None;
                    }
                    if d.now != d.from {
                        let newest = self.empty.as_ref().and_then(|e| e.bars.last()).map(|b| b.0);
                        let end = d.now.0 + i64::from(d.now.1);
                        self.acts.push(LoopAct::CaptureSpan(
                            (Some(end) != newest).then_some(end),
                            d.now.1,
                        ));
                        return Some(EventKind::Value(d.now.1 as f32));
                    }
                    if d.button {
                        self.acts.push(LoopAct::Capture);
                        return Some(EventKind::Tab(4));
                    }
                    return None;
                }
                if let Some(q) = self.press.filter(|q| q.finger == finger) {
                    let landing = self.landing().cloned();
                    self.press = None;
                    let gone = Some(EventKind::Xy {
                        x: x as f32,
                        y: y as f32,
                        down: false,
                    });
                    if phase == Touch::Cancel {
                        return q.lifted.then_some(gone?);
                    }
                    if q.swipe.is_some() {
                        return None;
                    }
                    if !q.lifted {
                        // A tap: the layer is picked, onto the Q-Links.
                        self.picked = q.k;
                        self.acts.push(LoopAct::Layer(q.k));
                        return Some(EventKind::Select(q.k));
                    }
                    // Let go anywhere else (its own place too): nothing.
                    match landing {
                        Some(d) if d.refused.is_none() => {
                            self.acts.push(LoopAct::DropLayer(q.k, d.to))
                        }
                        Some(d) => self.acts.push(LoopAct::NoDrop(d.to)),
                        None => {}
                    }
                    return gone;
                }
                if let Some((f, _, n)) = self.nudging.filter(|n| n.0 == finger) {
                    self.nudging = None;
                    if phase == Touch::Cancel || n == 0 || f != finger {
                        return None;
                    }
                    self.acts.push(LoopAct::Nudge(n));
                    return Some(EventKind::Value(n as f32));
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
                    .step_rects(p.lanes)
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

/// Snap anyway and Not now's float: its size, and how far it is from the
/// screen's right and bottom edges (the approved drawing: the hint line's
/// right end, 20 px in, its top 43 px from the bottom).
pub const ASK_W: i32 = 240;
pub const ASK_H: i32 = 36;
pub const ASK_RIGHT: i32 = 20;
pub const ASK_BOTTOM: i32 = 43;

/// Snap anyway (or Do it anyway) and Not now at the hint line's end,
/// floated over the Looper tab ([`ASK`], [`Ui::set_float`]) while a snap
/// over recorded audio asks, or Read as and Discard while a first pass
/// waits on its Length; the hint line says what it asks.
pub struct AskButtons {
    /// The yes and the no buttons' words.
    pub yes: String,
    pub no: String,
    acts: Vec<LoopAct>,
}

impl Default for AskButtons {
    fn default() -> AskButtons {
        AskButtons {
            yes: String::new(),
            no: "Not now".to_string(),
            acts: Vec::new(),
        }
    }
}

impl AskButtons {
    /// The yes and the Not now buttons in `r`.
    pub fn parts(r: Rect) -> (Rect, Rect) {
        (
            Rect::new(r.x, r.y, 132, r.h),
            Rect::new(r.right() - 100, r.y, 100, r.h),
        )
    }

    pub fn take(&mut self) -> Vec<LoopAct> {
        std::mem::take(&mut self.acts)
    }
}

impl Widget for AskButtons {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (yes, no) = AskButtons::parts(r);
        // The gap between them is the hint line's.
        cx.canvas.fill_rect(r, t.bg);
        cx.canvas.fill_round_rect(yes, 8, t.accent);
        cx.text(
            FontId::Bold,
            14.0,
            yes,
            Align::Center,
            t.on_accent,
            &self.yes,
        );
        cx.canvas.fill_round_rect(no, 8, t.raised);
        cx.text(FontId::Bold, 14.0, no, Align::Center, t.text, &self.no);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (yes, no) = AskButtons::parts(r);
        vec![
            Target::tap("ask-yes", 0, self.yes.clone(), yes),
            Target::tap("ask-no", 0, self.no.clone(), no),
        ]
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let (yes, no) = AskButtons::parts(r);
        let answer = if yes.contains(x, y) {
            true
        } else if no.contains(x, y) {
            false
        } else {
            return None;
        };
        self.acts.push(LoopAct::Answer(answer));
        Some(EventKind::Toggle(usize::from(answer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas};

    const W: i32 = 1116;
    const H: i32 = 452;

    /// A page of just `id`, drawn once.
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
            tag: format!("dub {k}"),
            colour: LOOP_COLOURS[k % LOOP_COLOURS.len()],
            muted,
            heard: !muted,
            ..LayerRow::default()
        }
    }

    fn plays() -> Strip {
        Strip::Plays {
            feedback: 1.0,
            overdubs: 0,
            reversed: false,
            speed: 1,
            length: "4 bars".into(),
            edits: true,
            chips: vec![
                Chip {
                    colour: LOOP_COLOURS[0],
                    kind: ChipKind::Done,
                },
                Chip {
                    colour: LOOP_COLOURS[1],
                    kind: ChipKind::Undone,
                },
            ],
            undo: true,
            redo: true,
            merge: ("Merge".into(), false),
        }
    }

    fn editor(layers: usize) -> LoopEditor {
        LoopEditor {
            title: "Loop 1".into(),
            has_sound: true,
            layers: (0..layers).map(|k| layer(k, k == 2)).collect(),
            strip: plays(),
            trim_ticks: Some(4 * 4 * 960),
            bars: 4,
            ..LoopEditor::default()
        }
    }

    fn hold(ui: &mut Ui, s: f32) -> bool {
        ui.get_mut::<LoopEditor>(EDITOR).unwrap().held(s)
    }

    fn acts(ui: &mut Ui) -> Vec<LoopAct> {
        take_actions(ui)
    }

    fn lanes(ui: &Ui) -> Vec<(usize, Rect)> {
        let r = ui.rect_of(EDITOR).unwrap();
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        e.lane_rects(LoopEditor::parts(r).lanes)
    }

    /// Loop 1 with smart loop end's chip (48 ms late, Snap) and the loop
    /// point view's contents, the view closed.
    fn smart_editor() -> LoopEditor {
        let pane = |title: &str, found: Option<(f32, String, String)>| PointPane {
            title: title.into(),
            note: "on your tap".into(),
            peaks: [128; POINT_BARS],
            edge: (2.0 / 3.0, false),
            you: (2.0 / 3.0, "your tap".into()),
            found,
            span: (-13_230, 13_230),
            hits: vec![0.5],
            moving: None,
        };
        LoopEditor {
            chip: Some(SmartChip {
                words: "48 ms late".into(),
                action: ("Snap".into(), ChipAct::Side(Side::Snap), true),
            }),
            point: Some(PointLook {
                now: "Now: your tap, 4 bars at 92.00 BPM".into(),
                start: pane("Start", None),
                end: pane(
                    "End",
                    Some((0.5, "end found".into(), "\u{2212}48 ms".into())),
                ),
                line: "Snap makes it 4 bars at 92.43 BPM, from the loop's next downbeat.".into(),
                pick: vec![
                    ("Snap".into(), Pick::Side(Side::Snap, false)),
                    ("Keep my tap".into(), Pick::Side(Side::Tap, true)),
                ],
                ab: Some(false),
                read_as: vec![
                    ("2 bars".into(), 2),
                    ("4 bars".into(), 4),
                    ("8 bars".into(), 8),
                ],
                bars_now: 4,
                step: MoveStep::Sixteenth,
            }),
            ..editor(3)
        }
    }

    fn target(ui: &Ui, id: &str, index: usize) -> Option<Target> {
        ui.targets()
            .into_iter()
            .map(|t| t.target)
            .find(|t| t.id == id && t.index == index)
    }

    fn tap_target(ui: &mut Ui, id: &str, index: usize) -> Vec<LoopAct> {
        let t = target(ui, id, index).unwrap_or_else(|| panic!("no {id} {index}"));
        let (x, y) = t.centre();
        tap(ui, x, y);
        ui.paint(&mut SoftCanvas::new(W, H));
        acts(ui)
    }

    #[test]
    fn the_chips_button_does_what_it_says_and_its_words_open_the_loop_point_view() {
        let mut ui = page_of(EDITOR, smart_editor());
        assert!(target(&ui, "point-close", 0).is_none(), "closed at first");
        let chip = target(&ui, "chip", 0).expect("the chip's words");
        let act = target(&ui, "chip-act", 0).expect("the chip's button");
        assert_eq!(
            (chip.label.as_str(), act.label.as_str()),
            ("48 ms late", "Snap")
        );
        assert!(chip.rect.right() <= act.rect.x, "words, then the button");
        let p = LoopEditor::parts(ui.rect_of(EDITOR).unwrap());
        assert!(act.rect.right() <= p.listen.x - 12, "clear of Listen");
        assert_eq!(
            tap_target(&mut ui, "chip-act", 0),
            vec![LoopAct::Chip(ChipAct::Side(Side::Snap))]
        );
        // The words: the view in the lanes' place, the lanes gone.
        assert_eq!(
            tap_target(&mut ui, "chip", 0),
            vec![LoopAct::PointView(true)]
        );
        assert!(target(&ui, "point-close", 0).is_some());
        assert!(target(&ui, "layer", 0).is_none(), "the lanes are under it");
        assert!(target(&ui, "trim-start", 0).is_none());
        assert_eq!(
            tap_target(&mut ui, "point-pick", 0),
            vec![LoopAct::Side(Side::Snap, false)]
        );
        assert_eq!(
            tap_target(&mut ui, "point-pick", 1),
            vec![LoopAct::Side(Side::Tap, true)]
        );
        assert_eq!(tap_target(&mut ui, "point-ab", 0), vec![LoopAct::Ab(true)]);
        // Read as: another reading snaps; the one it is now can't.
        assert_eq!(
            tap_target(&mut ui, "read-as", 2),
            vec![LoopAct::Chip(ChipAct::Snap(Snap::Bars(8)))]
        );
        assert!(!target(&ui, "read-as", 1).unwrap().enabled);
        assert_eq!(tap_target(&mut ui, "read-as", 1), Vec::new());
        // ×: closed, the lanes back.
        assert_eq!(
            tap_target(&mut ui, "point-close", 0),
            vec![LoopAct::PointView(false)]
        );
        assert!(target(&ui, "layer", 0).is_some());
        assert!(!ui.get::<LoopEditor>(EDITOR).unwrap().point_open());
    }

    /// Free loops (step F2): a free loop's header has Stop in Listen's
    /// place while the free loops play, Play while they are stopped, and
    /// "Call off" while a Play waits; each taps Ctl::FreeGo. An empty
    /// loop's strip starts with Sync, Song or Free, the one lit can't be
    /// tapped, and Length's second choice is Until tap.
    #[test]
    fn a_free_loops_header_stops_and_plays_them_and_the_strip_starts_with_sync() {
        let mut ui = page_of(
            EDITOR,
            LoopEditor {
                free: Some((true, false)),
                ..editor(3)
            },
        );
        assert!(target(&ui, "listen", 0).is_none(), "Listen's place");
        let go = target(&ui, "free-loops", 0).expect("Stop");
        assert_eq!(go.label, "Stop the free loops");
        let p = LoopEditor::parts(ui.rect_of(EDITOR).unwrap());
        assert_eq!(go.rect, p.listen);
        assert_eq!(
            tap_target(&mut ui, "free-loops", 0),
            vec![LoopAct::Strip(Ctl::FreeGo)]
        );
        for (free, label) in [
            ((false, false), "Play the free loops"),
            ((false, true), "Call off the free loops' Play"),
        ] {
            ui.get_mut::<LoopEditor>(EDITOR).unwrap().free = Some(free);
            ui.paint(&mut SoftCanvas::new(W, H));
            assert_eq!(target(&ui, "free-loops", 0).unwrap().label, label);
            assert_eq!(
                tap_target(&mut ui, "free-loops", 0),
                vec![LoopAct::Strip(Ctl::FreeGo)]
            );
        }
        // An empty loop: Sync first, Free lit.
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.free = None;
        e.layers.clear();
        e.has_sound = false;
        e.strip = Strip::Records {
            source: 0,
            sync: (1, true),
            monitor: None,
            bars: 1,
            starts: (0, true),
            then: 0,
        };
        ui.paint(&mut SoftCanvas::new(W, H));
        let (song, free) = (
            target(&ui, "sync", 0).unwrap(),
            target(&ui, "sync", 1).unwrap(),
        );
        assert_eq!((song.label.as_str(), free.label.as_str()), ("Song", "Free"));
        assert!(song.enabled && !free.enabled, "Free is lit");
        assert!(free.rect.right() < target(&ui, "bars", 0).unwrap().rect.x);
        assert_eq!(target(&ui, "bars", 1).unwrap().label, "Until tap");
        assert_eq!(
            tap_target(&mut ui, "sync", 0),
            vec![LoopAct::Strip(Ctl::Sync(0))]
        );
        // Once the loop records, Sync shows its own clock, fixed: a tap
        // only says why (Jay's review, 5.2).
        if let Strip::Records { sync, .. } = &mut ui.get_mut::<LoopEditor>(EDITOR).unwrap().strip {
            *sync = (1, false);
        }
        ui.paint(&mut SoftCanvas::new(W, H));
        assert_eq!(
            tap_target(&mut ui, "sync", 0),
            vec![LoopAct::Faint(Ctl::Sync(0))]
        );
        assert_eq!(target(&ui, "listen", 0).unwrap().label, "Listen");
    }

    #[test]
    fn the_loop_point_view_fits_its_place_and_keeps_its_buttons_apart() {
        let r = LoopEditor::parts(Rect::new(14, 336, 1112, 412)).point;
        assert_eq!(r, Rect::new(24, 384, 1092, 280), "the drawing's place");
        let look = smart_editor().point.unwrap();
        let pp = LoopEditor::point_parts(r, &look);
        let mut rects = pp.pick.clone();
        rects.extend(pp.ab);
        rects.extend(pp.read_as.iter().copied());
        for w in rects.windows(2) {
            assert!(w[0].right() <= w[1].x, "{:?} then {:?}", w[0], w[1]);
        }
        for b in &rects {
            assert!(
                b.h >= 36 && r.contains(b.x, b.y) && b.right() <= r.right() - 16,
                "{b:?}"
            );
            assert!(b.bottom() <= r.bottom(), "{b:?}");
        }
        assert!(pp.start.right() < pp.end.x && pp.end.right() <= r.right() - 16);
        // The titles hold ◀ ▶ (Move start / Move end), so the windows are
        // 108 px; STEP's choices sit in the head, left of ×.
        assert_eq!(pp.start.h, 108);
        let mut head = pp.steps.clone();
        head.push(pp.close);
        for w in head.windows(2) {
            assert!(w[0].right() <= w[1].x, "{:?} then {:?}", w[0], w[1]);
        }
        assert!(pp.steps.iter().all(|b| pp.head.contains(b.x, b.y)));
        for (k, win) in [pp.start, pp.end].into_iter().enumerate() {
            let [a, b] = pp.nudge[k];
            assert!(
                a.right() <= b.x && b.right() < win.x + win.w / 2,
                "{a:?} {b:?}"
            );
            assert!(a.y >= pp.head.bottom() && a.bottom() <= win.y, "{a:?}");
        }
        // Nothing found: one button and no A/B, Read as moves up.
        let alone = PointLook {
            pick: vec![("Keep my tap".into(), Pick::Side(Side::Tap, true))],
            ab: None,
            ..look
        };
        let pa = LoopEditor::point_parts(r, &alone);
        assert!(pa.ab.is_none() && pa.read_as[0].x < pp.read_as[0].x);
    }

    /// Move start / Move end's touches (the looper refactor's R2): STEP's
    /// choices pick the step, ◀ ▶ move each edge a step, a finger on a
    /// window puts that edge there (on a hit within reach) and follows a
    /// drag, and while a move is set up the row is Move and Cancel.
    #[test]
    fn the_loop_point_views_steps_arrows_and_windows_move_an_edge() {
        let mut ui = page_of(EDITOR, smart_editor());
        assert_eq!(
            tap_target(&mut ui, "chip", 0),
            vec![LoopAct::PointView(true)]
        );
        for (i, st) in MoveStep::ALL.into_iter().enumerate() {
            assert_eq!(
                tap_target(&mut ui, "point-step", i),
                vec![LoopAct::Move(MoveAct::Scale(st))]
            );
        }
        let nudges = [
            (Edge::Start, -1),
            (Edge::Start, 1),
            (Edge::End, -1),
            (Edge::End, 1),
        ];
        for (i, (edge, d)) in nudges.into_iter().enumerate() {
            assert_eq!(
                tap_target(&mut ui, "point-nudge", i),
                vec![LoopAct::Move(MoveAct::Step(edge, d))]
            );
        }
        // The middle of the end's window is by its hit, half way across
        // its 13 230 frames: the end goes on the hit.
        assert_eq!(
            tap_target(&mut ui, "point-window", 1),
            vec![LoopAct::Move(MoveAct::To(Edge::End, -6_615))]
        );
        // A finger on the start's window at its first peak, then 40 px
        // on: the start follows it, nowhere near the hit.
        let w = target(&ui, "point-window", 0).unwrap().rect;
        let y = w.y + w.h / 2;
        ui.touch(1, Touch::Down, w.x + 13, y);
        ui.touch(1, Touch::Move, w.x + 53, y);
        ui.touch(1, Touch::Up, w.x + 53, y);
        let got = acts(&mut ui);
        let to: Vec<i64> = got
            .iter()
            .map(|a| match a {
                LoopAct::Move(MoveAct::To(Edge::Start, f)) => *f,
                a => panic!("{a:?}"),
            })
            .collect();
        assert_eq!(to.len(), 2, "{got:?}");
        assert_eq!(to[0], -13_230);
        assert!(to[0] < to[1] && to[1] < -6_615 - 1_000, "{to:?}");
        // A move set up: Move keeps it, Cancel puts it away.
        let look = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        look.point.as_mut().unwrap().pick =
            vec![("Move".into(), Pick::Move), ("Cancel".into(), Pick::Cancel)];
        ui.paint(&mut SoftCanvas::new(W, H));
        assert_eq!(target(&ui, "point-pick", 0).unwrap().label, "Move");
        assert_eq!(
            tap_target(&mut ui, "point-pick", 0),
            vec![LoopAct::Move(MoveAct::Keep)]
        );
        assert_eq!(
            tap_target(&mut ui, "point-pick", 1),
            vec![LoopAct::Move(MoveAct::Cancel)]
        );
    }

    #[test]
    fn snap_anyway_and_not_now_answer() {
        let mut ui = page_of(
            ASK,
            AskButtons {
                yes: "Snap anyway".into(),
                ..AskButtons::default()
            },
        );
        let yes = target(&ui, "ask-yes", 0).unwrap();
        assert_eq!(yes.label, "Snap anyway");
        assert!(yes.rect.h >= 36);
        let (x, y) = yes.centre();
        tap(&mut ui, x, y);
        assert_eq!(take_actions(&mut ui), vec![LoopAct::Answer(true)]);
        let no = target(&ui, "ask-no", 0).unwrap().centre();
        tap(&mut ui, no.0, no.1);
        assert_eq!(take_actions(&mut ui), vec![LoopAct::Answer(false)]);
    }

    #[test]
    fn four_lanes_a_fingertip_tall_show_newest_on_top() {
        let ui = page_of(EDITOR, editor(6));
        let l = lanes(&ui);
        assert_eq!(l.len(), 4);
        assert_eq!(
            l.iter().map(|x| x.0).collect::<Vec<_>>(),
            vec![5, 4, 3, 2],
            "the newest layer first"
        );
        assert!(l.iter().all(|(_, r)| r.h >= LANE_H), "{l:?}");
        // The map shows while two older layers are out of view.
        let r = ui.rect_of(EDITOR).unwrap();
        let ts = ui.get::<LoopEditor>(EDITOR).unwrap().targets(r);
        assert!(ts.iter().any(|t| t.id == "layer-map"));
    }

    /// Jay, 17 Nov: with more layers than fit, a swipe on the lanes
    /// scrolls them; up shows the older layers. Nothing is picked or
    /// lifted, and the lanes stay where the swipe left them until another
    /// layer is picked.
    #[test]
    fn a_swipe_up_shows_the_older_layers_and_lifts_nothing() {
        let mut ui = page_of(EDITOR, editor(7));
        let (_, first) = lanes(&ui)[0];
        let (x, y) = (first.x + LANE_HEAD + 100, first.y + first.h / 2);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Move, x, y - 2 * (first.h + LANE_GAP));
        ui.touch(1, Touch::Up, x, y - 2 * (first.h + LANE_GAP));
        assert_eq!(lanes(&ui)[0].0, 4, "two lanes along: layer 5 on top");
        assert!(acts(&mut ui).is_empty(), "nothing picked");
        // Past the end it stops on the oldest four.
        ui.touch(2, Touch::Down, x, y);
        ui.touch(2, Touch::Move, x, y - 10 * (first.h + LANE_GAP));
        ui.touch(2, Touch::Up, x, y);
        assert_eq!(
            lanes(&ui).last().unwrap().0,
            0,
            "the first pass at the bottom"
        );
        // A swipe down comes back to the newest.
        ui.touch(3, Touch::Down, x, y);
        ui.touch(3, Touch::Move, x, y + 10 * (first.h + LANE_GAP));
        ui.touch(3, Touch::Up, x, y);
        assert_eq!(lanes(&ui)[0].0, 6);
    }

    #[test]
    fn a_lane_lifts_after_a_hold_and_drops_on_a_loop() {
        let mut ui = page_of(EDITOR, editor(3));
        let spot = Rect::new(20, 10, 120, 120);
        ui.get_mut::<LoopEditor>(EDITOR).unwrap().drops = vec![DropSpot {
            to: DropTo::Loop(3),
            rect: spot,
            refused: None,
        }];
        let (k, rr) = lanes(&ui)[1];
        let (x, y) = (rr.x + LANE_HEAD + 60, rr.y + rr.h / 2);
        ui.touch(1, Touch::Down, x, y);
        assert!(hold(&mut ui, LIFT_HOLD_S));
        ui.touch(1, Touch::Move, x, y - 40);
        ui.touch(1, Touch::Move, 60, 60);
        ui.touch(1, Touch::Up, 60, 60);
        assert_eq!(acts(&mut ui), vec![LoopAct::DropLayer(k, DropTo::Loop(3))]);
        // A quick tap still picks it.
        tap(&mut ui, x, y);
        assert_eq!(acts(&mut ui), vec![LoopAct::Layer(k)]);
    }

    #[test]
    fn a_lanes_head_mutes_its_s_solos_and_the_picked_one_deletes() {
        let mut ui = page_of(EDITOR, editor(3));
        let (k, rr) = lanes(&ui)[0];
        let lp = LoopEditor::lane_parts(rr);
        // The newest (layer 3) is muted: its head lets it play.
        assert!(ui.get::<LoopEditor>(EDITOR).unwrap().layers[k].muted);
        tap(&mut ui, lp.head.x + 30, lp.head.y + 20);
        tap(&mut ui, lp.solo.x + 10, lp.solo.y + 10);
        assert_eq!(
            acts(&mut ui),
            vec![
                LoopAct::LayerCmd(k, LayerCommand::Mute(false)),
                LoopAct::LayerCmd(k, LayerCommand::Solo(true)),
            ]
        );
        // Layer 1 is picked: its × is on its lane, at the bottom.
        let (k0, rr0) = *lanes(&ui).last().unwrap();
        assert_eq!(k0, 0);
        let lp = LoopEditor::lane_parts(rr0);
        tap(&mut ui, lp.delete.x + 10, lp.delete.y + 10);
        assert_eq!(
            acts(&mut ui),
            vec![LoopAct::LayerCmd(0, LayerCommand::Delete)]
        );
    }

    #[test]
    fn the_ruler_nudges_by_sixteenths_and_its_grips_trim() {
        let mut ui = page_of(EDITOR, editor(2));
        let r = ui.rect_of(EDITOR).unwrap();
        let p = LoopEditor::parts(r);
        let px = LoopEditor::sixteenth_px(p.ruler, 4 * 4 * 960);
        let (x, y) = (p.ruler.x + p.ruler.w / 2, p.ruler.y + p.ruler.h / 2);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Move, x + (3.0 * px) as i32 + 1, y);
        ui.touch(1, Touch::Up, x + (3.0 * px) as i32 + 1, y);
        assert_eq!(acts(&mut ui), vec![LoopAct::Nudge(3)]);
        let (_, end) = LoopEditor::trim_grips(p.ruler);
        ui.touch(2, Touch::Down, end.x + 4, y);
        ui.touch(2, Touch::Move, p.ruler.x + p.ruler.w / 2, y);
        ui.touch(2, Touch::Up, p.ruler.x + p.ruler.w / 2, y);
        assert_eq!(acts(&mut ui), vec![LoopAct::Trim(0, 2 * 4 * 960)]);
    }

    #[test]
    fn the_strip_shows_what_the_moment_needs() {
        let mut ui = page_of(EDITOR, editor(2));
        let r = ui.rect_of(EDITOR).unwrap();
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let ids: Vec<&str> = e.targets(r).iter().map(|t| t.id).collect();
        for id in [
            "feedback", "reverse", "half", "double", "undo", "redo", "merge",
        ] {
            assert!(ids.contains(&id), "{id} in {ids:?}");
        }
        assert!(!ids.contains(&"source"));
        // Merge is faint with one step to merge: it says why.
        let p = LoopEditor::parts(r);
        let m = e
            .strip_controls(p.strip)
            .into_iter()
            .find(|c| c.0 == Ctl::Merge)
            .unwrap();
        tap(&mut ui, m.1.x + 10, m.1.y + 10);
        assert_eq!(acts(&mut ui), vec![LoopAct::Faint(Ctl::Merge)]);
        // An empty loop: how it records.
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.layers.clear();
        e.strip = Strip::Records {
            source: 1,
            sync: (0, true),
            monitor: Some(false),
            bars: 2,
            starts: (0, true),
            then: 0,
        };
        let ts = e.targets(r);
        let ids: Vec<&str> = ts.iter().map(|t| t.id).collect();
        assert!(!ids.contains(&"feedback"));
        let four = ts
            .iter()
            .find(|t| t.id == "bars" && t.index == 3)
            .unwrap()
            .rect;
        let mon = ts.iter().find(|t| t.id == "monitor").unwrap().rect;
        tap(&mut ui, four.x + 5, four.y + 5);
        tap(&mut ui, mon.x + 5, mon.y + 5);
        assert_eq!(
            acts(&mut ui),
            vec![LoopAct::Strip(Ctl::Bars(3)), LoopAct::Strip(Ctl::Monitor)]
        );
        // What you played: Capture over the bars it takes, while it has
        // some to take.
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.empty = Some(EmptyLook {
            title: "WHAT YOU PLAYED · LINE IN".into(),
            bars: vec![(9, 0.1), (10, 0.35), (11, 0.6), (12, 0.85)],
            take: Some((0.35, 0.85, "Capture bars 10 and 11".into())),
            ..EmptyLook::default()
        });
        let ts = e.targets(r);
        let c = ts.iter().find(|t| t.id == "capture").expect("Capture");
        assert_eq!(c.label, "Capture bars 10 and 11");
        assert!(
            c.rect.h >= 44 && c.rect.w >= 200,
            "a fingertip: {:?}",
            c.rect
        );
        let lanes = LoopEditor::parts(r).lanes;
        let mid = lanes.x + 14 + (0.6 * (lanes.w - 28) as f32) as i32;
        assert!(
            (c.rect.x + c.rect.w / 2 - mid).abs() <= 2,
            "over bars 10 and 11"
        );
        let (x, y) = (c.rect.x + 10, c.rect.y + 10);
        tap(&mut ui, x, y);
        assert_eq!(acts(&mut ui), vec![LoopAct::Capture]);
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.empty.as_mut().unwrap().take = None;
        assert!(!e.targets(r).iter().any(|t| t.id == "capture"));
        ui.paint(&mut mpfree_ui::SoftCanvas::new(1280, 800));
    }

    /// What you played's window: a finger in it moves it by whole bars,
    /// one by an end grows or shrinks it by whole patterns, never past
    /// the bars played; let go, Capture is asked to take that; at the
    /// newest bar line it keeps up (None). The button still captures.
    #[test]
    fn the_capture_window_moves_and_resizes_by_the_pattern() {
        let mut ui = page_of(EDITOR, editor(0));
        let r = ui.rect_of(EDITOR).unwrap();
        let lanes = LoopEditor::parts(r).lanes;
        let x_of = |bar: i64| (bar - 4) as f32 / 9.0;
        let look = EmptyLook {
            title: "WHAT YOU PLAYED · PADS".into(),
            bars: (5..=12).map(|b| (b, x_of(b))).collect(),
            take: Some((x_of(9), x_of(11), "Capture bars 9 and 10".into())),
            window: Some((9, 2)),
            unit: 2,
            pattern: true,
            ..EmptyLook::default()
        };
        ui.get_mut::<LoopEditor>(EDITOR).unwrap().empty = Some(look.clone());
        let win = window_rect(lanes, x_of(9), x_of(11));
        let w = empty_wave_rect(lanes);
        let bar = (w.w as f32 / 9.0) as i32;
        let drag = |ui: &mut Ui, x: i32, y: i32, dx: i32| {
            ui.touch(1, Touch::Down, x, y);
            ui.touch(1, Touch::Move, x + dx / 2, y);
            ui.touch(1, Touch::Move, x + dx, y);
            ui.touch(1, Touch::Up, x + dx, y);
        };
        // Moved two bars back (by its top, clear of the button).
        let top = win.y + 6;
        drag(&mut ui, win.x + win.w / 2, top, -2 * bar);
        assert_eq!(acts(&mut ui), vec![LoopAct::CaptureSpan(Some(9), 2)]);
        // Its start pulled back two bars: a pattern more, bars 7 to 10.
        drag(&mut ui, win.x, top, -2 * bar);
        assert_eq!(acts(&mut ui), vec![LoopAct::CaptureSpan(Some(11), 4)]);
        // Its end can't go past the bars played (bar 12 is still playing).
        drag(&mut ui, win.right(), top, 3 * bar);
        assert_eq!(acts(&mut ui), vec![], "no whole pattern more to take");
        // Moved up to the newest bar line: it keeps up.
        drag(&mut ui, win.x + win.w / 2, top, 3 * bar);
        assert_eq!(acts(&mut ui), vec![LoopAct::CaptureSpan(None, 2)]);
        // While a finger has it, it shows where, with its patterns.
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.window_drag = Some(WindowDrag {
            finger: 9,
            grip: Grip::Start,
            x0: 0,
            from: (9, 2),
            now: (5, 6),
            button: false,
        });
        let shown = e.shown_take().unwrap();
        assert_eq!(shown.2, "Capture bars 5 to 10 · 3 × pattern");
        assert_eq!((shown.0, shown.1), (x_of(5), x_of(11)));
        ui.paint(&mut mpfree_ui::SoftCanvas::new(1280, 800));
        ui.get_mut::<LoopEditor>(EDITOR).unwrap().window_drag = None;
        // A tap on the button, unmoved, captures.
        let b = capture_button(lanes, x_of(9), x_of(11), "Capture bars 9 and 10");
        tap(&mut ui, b.x + b.w / 2, b.y + b.h / 2);
        assert_eq!(acts(&mut ui), vec![LoopAct::Capture]);
    }

    #[test]
    fn feedback_follows_a_finger_up_and_the_wheel() {
        let mut ui = page_of(EDITOR, editor(2));
        let r = ui.rect_of(EDITOR).unwrap();
        let p = LoopEditor::parts(r);
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let k = e.strip_controls(p.strip)[0].1;
        let (x, y) = (k.x + 20, k.y + 20);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Move, x, y + 60);
        ui.touch(1, Touch::Up, x, y + 60);
        let a = acts(&mut ui);
        assert!(
            matches!(a.last(), Some(LoopAct::Feedback(v)) if (*v - 0.8).abs() < 0.01),
            "{a:?}"
        );
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        let t = e
            .targets(r)
            .into_iter()
            .find(|t| t.id == "feedback")
            .unwrap();
        e.held_step(&t, 5, false, r);
        assert!(
            matches!(acts(&mut ui).last(), Some(LoopAct::Feedback(v)) if (*v - 0.85).abs() < 0.01)
        );
    }

    #[test]
    fn send_opens_its_choices_and_only_built_ones_work() {
        let mut ui = page_of(EDITOR, editor(2));
        ui.get_mut::<LoopEditor>(EDITOR).unwrap().send_built = [false, false, false, true];
        let r = ui.rect_of(EDITOR).unwrap();
        let p = LoopEditor::parts(r);
        tap(&mut ui, p.send.x + 10, p.send.y + 10);
        assert!(ui.get::<LoopEditor>(EDITOR).unwrap().send_open());
        let b = LoopEditor::send_rects(p.lanes);
        tap(&mut ui, b[0].x + 10, b[0].y + 10);
        tap(&mut ui, b[SEND_SHOW].x + 10, b[SEND_SHOW].y + 10);
        assert_eq!(
            acts(&mut ui),
            vec![LoopAct::FaintSend(0), LoopAct::Send(SEND_SHOW)]
        );
        assert!(!ui.get::<LoopEditor>(EDITOR).unwrap().send_open());
    }

    #[test]
    fn the_undo_history_takes_the_lanes_place_and_a_tap_goes_to_a_step() {
        let mut ui = page_of(EDITOR, editor(2));
        ui.get_mut::<LoopEditor>(EDITOR)
            .unwrap()
            .set_history(Some(HistoryList {
                rows: (0..5).map(|i| format!("Step {i}")).collect(),
                now: 3,
            }));
        let r = ui.rect_of(EDITOR).unwrap();
        let rows = ui
            .get::<LoopEditor>(EDITOR)
            .unwrap()
            .step_rects(LoopEditor::parts(r).lanes);
        let (k, sr) = rows[1];
        tap(&mut ui, sr.x + 20, sr.y + 10);
        assert_eq!(acts(&mut ui), vec![LoopAct::Step(k)]);
    }

    #[test]
    fn the_picked_layer_stays_in_view() {
        let mut ui = page_of(EDITOR, editor(8));
        let r = ui.rect_of(EDITOR).unwrap();
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.picked = 0;
        e.follow_in(r);
        assert!(
            lanes(&ui).iter().any(|l| l.0 == 0),
            "the first pass in view"
        );
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        e.picked = 7;
        e.follow_in(r);
        assert_eq!(lanes(&ui)[0].0, 7, "the newest on top again");
    }
}
