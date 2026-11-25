//! The mixer screen's widget: a row of channel strips with the master on
//! the right, the strips' views and pages along the top, and an output
//! picker that opens over the strips.
//!
//! Each strip, top to bottom: its name over a colour stripe, four insert
//! rows (a tap opens the slot's page, or the effect list on the empty
//! slot), sends A to D (drag to set; a tap on the letter takes it before or
//! after the fader), pan (drag; a tap leaves it where it is), the fader beside a
//! stereo meter (drag; it moves relative to where the finger lands, with a
//! detent at 0 dB), the level (a tap let go on it opens the number pad;
//! SHIFT + the wheel's push sets 0 dB), Mute and Solo (and Arm on an
//! input), and the output (a tap opens the picker). An input strip trades
//! its last two insert rows for its Source and Monitor.
//!
//! Once the app offers takes ([`MixState::takes`]), a tap on the selected
//! strip's name opens its menu: Freeze or Thaw (a track or a pad), Print
//! to a new sample, or Cancel while a take is under way. A frozen strip shows "Frozen" over its inserts, and a take
//! under way shows there with its progress.
//!
//! The widget keeps a copy of what it shows and moves it under the finger
//! at once; what the touches mean goes into a queue of [`MixAction`]s
//! (see [`crate::take_actions`]) for the app to send to the mixer.

use std::cell::{Cell, RefCell};

use mpfree_fx::ids::mixer::{self as mx, output};
use mpfree_fx::{StripId, BUSES, RETURNS, SILENT_DB};
use mpfree_ui::touch::Class;
use mpfree_ui::{
    Align, Color, Commit, Consequence, Context, EventKind, Fingers, FontId, PaintCx, Rect, Target,
    Theme, Timing, Tone, Touch, Widget,
};

use crate::eq_look::{self, Dot};
use crate::io::{self, IoFrame, IoInput, IoKnob, IoReply, IoSection};
use crate::state::{choice, output_label, peak_db, MixState, MixView, StripView, TakeView};

/// What a touch on the mixer meant, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum MixAction {
    /// Set a strip parameter (an id from `mpfree_fx::ids::mixer`); send it
    /// to the mixer as an event.
    Set { strip: StripId, id: u16, value: f32 },
    /// A strip's name was tapped: it becomes the selected strip (the one
    /// the knobs and the data wheel work on).
    Select(StripId),
    /// An insert slot was tapped: open its effect's page.
    OpenInsert { strip: StripId, slot: usize },
    /// An effect's "SC from ..." or "Nothing picked" was tapped on the
    /// Effects look: open its page with the sidechain list up.
    OpenSidechain { strip: StripId, slot: usize },
    /// The empty slot was tapped: open the list of effects to add.
    AddInsert { strip: StripId },
    /// An effect was tapped on the Effects look: it is picked, and its
    /// first four knobs go on the Q-Links (a tap again opens its chain).
    PickInsert { strip: StripId, slot: usize },
    /// An effect was dragged along its row on the Effects look: move it
    /// (`Mixer` `Edit::Move`).
    MoveInsert {
        strip: StripId,
        from: usize,
        to: usize,
    },
    /// A view was picked: show its strips.
    View(MixView),
    /// Freeze the strip: take one pass of the loop through its inserts
    /// (`mpfree_fx::Print` with `PrintJob::freeze`), then
    /// `Mixer::freeze` it with the take.
    Freeze(StripId),
    /// Back to the strip's live inserts (`Mixer::thaw`).
    Thaw(StripId),
    /// Print the strip, with its inserts and their tail, to a new sample.
    Print(StripId),
    /// Stop the strip's freeze or print under way; nothing changes.
    CancelTake(StripId),
    /// The look was switched (Faders or Effects): the app's hint line
    /// follows. The mixer has nothing to do.
    Look(Look),
    /// The app saw the pick go (the look switched, the effect gone, the
    /// tab left by TRACK MIX): the Q-Links go back to the selected strip.
    /// The widget never queues this; the app sends it.
    Unpick,
    /// "+ EQ" on the EQ look: add a parametric EQ in the strip's first
    /// free slot.
    AddEq { strip: StripId },
    /// A band picked on the EQ look (its dot tapped or dragged): the
    /// picked EQ's Q-Links follow parameter `id`'s band, as the chain
    /// editor's focus does.
    FocusInsert {
        strip: StripId,
        slot: usize,
        id: u16,
    },
    /// A dot dragged on the EQ look: set parameter `id` of insert `slot`,
    /// as the chain editor's knobs set one.
    SetInsert {
        strip: StripId,
        slot: usize,
        id: u16,
        value: f32,
    },
    /// The finger left the dot: the drag ends (one UNDO step).
    ReleaseInsert {
        strip: StripId,
        slot: usize,
        id: u16,
    },
    /// The level under a strip's fader was tapped (let go on it): the
    /// app opens the number pad on the strip's level, `db` now, and sends
    /// the number typed back as a [`MixAction::Set`] of [`mx::VOLUME`].
    /// Nothing changes until Enter. (The tap used to put the fader at
    /// 0 dB at once, a jump of 20 dB or more from a finger that missed M
    /// or S by a few pixels; SHIFT + the push still does that, on
    /// purpose.)
    TypeLevel {
        strip: StripId,
        db: f32,
        name: String,
    },
}

/// The Mix tab's four looks of the same strips (layout spec section
/// 3a): Faders, the strips side by side; Sends, each strip's pan, sends
/// and output as a row (the touch plan's A7); Effects, each strip's chain
/// as a row of slots; and EQ, each strip's EQ as a curve. On Sends,
/// Effects and EQ the master's row is pinned under the strips'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    #[default]
    Faders,
    Sends,
    Effects,
    Eq,
}

impl Look {
    pub const ALL: [Look; 4] = [Look::Faders, Look::Sends, Look::Effects, Look::Eq];

    pub fn label(self) -> &'static str {
        match self {
            Look::Faders => "Faders",
            Look::Sends => "Sends",
            Look::Effects => "Effects",
            Look::Eq => "EQ",
        }
    }

    /// A look of rows, a strip a row, rather than strips side by side.
    pub fn rows(self) -> bool {
        self != Look::Faders
    }
}

/// What a strip's menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripMenu {
    Freeze,
    Thaw,
    Print,
    Cancel,
}

impl StripMenu {
    /// What the strip `s`'s menu offers, in order.
    pub fn items(s: &StripView) -> Vec<StripMenu> {
        if s.take.is_some() {
            return vec![StripMenu::Cancel];
        }
        let mut v = Vec::new();
        if s.can_freeze() {
            v.push(if s.frozen {
                StripMenu::Thaw
            } else {
                StripMenu::Freeze
            });
        }
        v.push(StripMenu::Print);
        v
    }

    /// The button's label and the line under it.
    pub fn text(self, s: &StripView) -> (&'static str, &'static str) {
        match self {
            StripMenu::Freeze => (
                "Freeze",
                "Takes the next pass through its effects and plays that instead, to free CPU",
            ),
            StripMenu::Thaw => ("Thaw", "Back to its live effects"),
            StripMenu::Print => (
                "Print to a new sample",
                "Records the next pass with its effects and their tail",
            ),
            StripMenu::Cancel => (
                if s.take.is_some_and(|t| t.print) {
                    "Cancel the print"
                } else {
                    "Cancel the freeze"
                },
                "Stops the take; nothing changes",
            ),
        }
    }

    fn action(self, id: StripId) -> MixAction {
        match self {
            StripMenu::Freeze => MixAction::Freeze(id),
            StripMenu::Thaw => MixAction::Thaw(id),
            StripMenu::Print => MixAction::Print(id),
            StripMenu::Cancel => MixAction::CancelTake(id),
        }
    }
}

/// The panel's own top row.
pub const HEAD_H: i32 = 44;
/// A channel strip's width and the gap between strips.
pub const STRIP_W: i32 = 104;
pub const MASTER_W: i32 = 150;
const GAP: i32 = 6;
const CHIP_W: i32 = 150;
/// The narrowest a strip view's chip gets ("Submixes" and its count).
const MIN_CHIP_W: i32 = 124;
/// The I/O view's chip, after the four strip views.
const IO_CHIP_W: i32 = 76;
const ARROW_W: i32 = 52;
const PAGE_W: i32 = 120;
/// The Faders, Sends, Effects and EQ toggle.
const LOOK_W: i32 = 246;
/// The view chips without their counts, when the full ones don't fit
/// beside the toggle (the Mix tab beside the Q-Link column): Tracks,
/// Pads, Inputs, Submixes.
const COMPACT_CHIP_W: [i32; 4] = [110, 90, 96, 120];
/// The Effects look: a strip's name, then its slots, a row a strip.
const FX_NAME_W: i32 = 176;
/// Slots a row shows (the most a strip in the app has).
pub const FX_COLS: usize = 8;
const FX_ROW_H: i32 = 52;
const FX_ROW_GAP: i32 = 6;

/// Fader positions (0 to 1) against levels (dB): +6 at the top, 0 dB four
/// fifths up, -80 (silent) at the bottom.
const LAW: [(f32, f32); 8] = [
    (0.0, -80.0),
    (0.1, -60.0),
    (0.22, -40.0),
    (0.38, -24.0),
    (0.52, -12.0),
    (0.66, -6.0),
    (0.8, 0.0),
    (1.0, 6.0),
];
/// The marks beside a fader.
const MARKS: [(f32, &str); 7] = [
    (6.0, "+6"),
    (0.0, "0"),
    (-6.0, "-6"),
    (-12.0, "-12"),
    (-24.0, "-24"),
    (-40.0, "-40"),
    (-60.0, "-60"),
];

/// Where a level sits on a fader (0 to 1).
pub fn db_to_pos(db: f32) -> f32 {
    if db.is_nan() || db <= LAW[0].1 {
        return 0.0;
    }
    for w in LAW.windows(2) {
        let ((p0, d0), (p1, d1)) = (w[0], w[1]);
        if db <= d1 {
            return p0 + (p1 - p0) * (db - d0) / (d1 - d0);
        }
    }
    1.0
}

/// The level at a fader position, to 0.1 dB, with a detent at 0 dB.
pub fn pos_to_db(pos: f32) -> f32 {
    let pos = pos.clamp(0.0, 1.0);
    let mut db = LAW[LAW.len() - 1].1;
    for w in LAW.windows(2) {
        let ((p0, d0), (p1, d1)) = (w[0], w[1]);
        if pos <= p1 {
            db = d0 + (d1 - d0) * (pos - p0) / (p1 - p0);
            break;
        }
    }
    if db <= SILENT_DB + 0.05 {
        return SILENT_DB;
    }
    if db.abs() < 0.3 {
        return 0.0;
    }
    (db * 10.0).round() / 10.0
}

/// A level as the strip writes it: "-3.0 dB", "+2.5 dB", "off".
pub fn db_text(db: f32) -> String {
    if db <= SILENT_DB {
        "off".into()
    } else if db > 0.04 {
        format!("+{db:.1} dB")
    } else {
        format!("{:.1} dB", if db.abs() < 0.05 { 0.0 } else { db })
    }
}

/// A pan as the strip writes it: "C", "L40", "R25".
pub fn pan_text(p: f32) -> String {
    let v = p.round() as i32;
    match v {
        0 => "C".into(),
        v if v < 0 => format!("L{}", -v),
        v => format!("R{v}"),
    }
}

/// The strips' colours, from the theme (one per track number).
/// A small wave mark in `r`, four bars of different heights: an audio
/// track's, as on the Clips grid and the Song view.
/// A small play mark (the ▶ the fonts don't have) filling `r`, a row of
/// pixels at a time.
fn play_mark(cx: &mut PaintCx, r: Rect, color: Color) {
    let h = r.h.max(2);
    for y in 0..h {
        let half = (h - 1) as f32 / 2.0;
        let w = ((1.0 - (y as f32 - half).abs() / (half + 0.5)) * r.w as f32).round() as i32;
        if w > 0 {
            cx.canvas.fill_rect(Rect::new(r.x, r.y + y, w, 1), color);
        }
    }
}

fn wave_mark(cx: &mut PaintCx, r: Rect, color: Color) {
    for (k, h) in [5, 12, 8, 3].iter().enumerate() {
        let x = r.x + k as i32 * 4;
        cx.canvas
            .fill_round_rect(Rect::new(x, r.y + (r.h - h) / 2, 2, *h), 1, color);
    }
}

fn strip_color(t: &Theme, s: StripId) -> Color {
    let palette = [
        t.accent,
        t.good,
        t.warn,
        t.bad,
        t.accent.mix(t.bad, 0.5),
        t.good.mix(t.accent, 0.5),
        t.warn.mix(t.bad, 0.4),
        t.accent.mix(t.text, 0.5),
    ];
    match s {
        StripId::Track(i) | StripId::Pad(i) => palette[i % palette.len()],
        StripId::Input(_) => t.bad.mix(t.warn, 0.5),
        StripId::Bus(_) => t.text_dim,
        StripId::Return(_) => t.accent.mix(t.text_dim, 0.6),
        StripId::Master => t.text,
    }
}

/// Where everything in the panel goes, from its rectangle.
struct Layout {
    chips: [Rect; 5],
    prev: Rect,
    next: Rect,
    page: Rect,
    /// The Faders, Effects and EQ toggle (where the mixer's CPU chip
    /// was).
    look: Rect,
    strips: Rect,
    master: Rect,
    per_page: usize,
    /// The view chips are the narrow ones, without counts.
    compact: bool,
}

fn layout(r: Rect) -> Layout {
    let (head, body) = r.take_top(HEAD_H);
    let body = body.take_top(8).1;
    let (master, rest) = body.take_right(MASTER_W);
    let strips = rest.take_left((rest.w - 12).max(0)).0;
    let per_page = ((strips.w + GAP) / (STRIP_W + GAP)).max(1) as usize;
    let ch = 36.min(head.h);
    let cy = head.y + (head.h - ch) / 2;
    let next = Rect::new(head.right() - ARROW_W, cy, ARROW_W, ch);
    let page = Rect::new(next.x - PAGE_W, cy, PAGE_W, ch);
    let prev = Rect::new(page.x - ARROW_W, cy, ARROW_W, ch);
    let look = Rect::new(prev.x - 14 - LOOK_W, cy, LOOK_W, ch);
    // The view chips share what is left of the toggle: narrower when the
    // panel is (the Q-Link column beside it), never under MIN_CHIP_W.
    let room = look.x - 12 - head.x - 4 * 8 - IO_CHIP_W;
    let compact = room / 4 < MIN_CHIP_W;
    let chip_w = (room / 4).clamp(MIN_CHIP_W, CHIP_W);
    let mut x = head.x;
    let chips = std::array::from_fn(|i| {
        let w = if MixView::ALL[i] == MixView::Io {
            IO_CHIP_W
        } else if compact {
            COMPACT_CHIP_W[i.min(3)]
        } else {
            chip_w
        };
        let c = Rect::new(x, cy, w, ch);
        x += w + 8;
        c
    });
    Layout {
        chips,
        prev,
        next,
        page,
        look,
        strips,
        master,
        per_page,
        compact,
    }
}

/// The toggle's parts: Faders, Sends, Effects, EQ, each as wide as its
/// word needs (EQ the narrowest).
fn look_halves(r: Rect) -> [Rect; 4] {
    const WEIGHTS: [i32; 4] = [6, 6, 7, 4];
    let inner = r.inset(4);
    let room = inner.w - 3 * 4;
    let total: i32 = WEIGHTS.iter().sum();
    let mut x = inner.x;
    std::array::from_fn(|i| {
        let w = if i == 3 {
            inner.right() - x
        } else {
            room * WEIGHTS[i] / total
        };
        let h = Rect::new(x, inner.y, w, inner.h);
        x += w + 4;
        h
    })
}

/// The Effects look: the panel over the strips and the master, a title
/// line, the slot numbers, a row a strip and the master's row pinned at
/// the bottom.
struct Grid {
    panel: Rect,
    title: Rect,
    numbers: Rect,
    /// Where the strips' rows start.
    top: i32,
    width: i32,
    left: i32,
    master: Rect,
    rows: usize,
}

fn grid(l: &Layout) -> Grid {
    let panel = Rect::new(
        l.strips.x,
        l.strips.y,
        l.master.right() - l.strips.x,
        l.strips.h,
    );
    let inner = panel.inset(14);
    let (title, rest) = inner.take_top(22);
    let rest = rest.take_top(8).1;
    let (numbers, rest) = rest.take_top(20);
    let rest = rest.take_top(8).1;
    let (master, rest) = rest.take_bottom(FX_ROW_H);
    let room = rest.h - 12;
    let rows = ((room + FX_ROW_GAP) / (FX_ROW_H + FX_ROW_GAP)).max(1) as usize;
    Grid {
        panel,
        title,
        numbers,
        top: rest.y,
        width: inner.w,
        left: inner.x,
        master,
        rows,
    }
}

impl Grid {
    /// Row `i` of the page (0 at the top).
    fn row(&self, i: usize) -> Rect {
        Rect::new(
            self.left,
            self.top + i as i32 * (FX_ROW_H + FX_ROW_GAP),
            self.width,
            FX_ROW_H,
        )
    }
}

/// A row's name cell and its slots' cells.
fn row_cells(row: Rect) -> (Rect, [Rect; FX_COLS]) {
    let name = row.take_left(FX_NAME_W).0;
    let n = FX_COLS as i32;
    let w = ((row.w - FX_NAME_W - n * 8) / n).max(24);
    let cells = std::array::from_fn(|k| {
        Rect::new(row.x + FX_NAME_W + 8 + k as i32 * (w + 8), row.y, w, row.h)
    });
    (name, cells)
}

/// Where slot `s` ends up when the effect in `from` moves to `to`.
fn moved_slot(s: usize, from: usize, to: usize) -> usize {
    if s == from {
        to
    } else if from < s && s <= to {
        s - 1
    } else if to <= s && s < from {
        s + 1
    } else {
        s
    }
}

/// What a strip's row says under its name: which strip it is, unless the
/// name says so, and how full its chain is.
fn row_sub(s: &StripView) -> String {
    let id = match s.id {
        StripId::Master => "Out 1+2".to_string(),
        StripId::Bus(b) => format!("Submix {}", b + 1),
        StripId::Input(i) => format!("Input {}", i + 1),
        id => crate::state::default_name(id),
    };
    let fx = if s.frozen {
        "frozen".to_string()
    } else if s.inserts.is_empty() {
        "no effects".to_string()
    } else {
        format!("{} of {}", s.inserts.len(), s.slots)
    };
    let mut parts = Vec::new();
    if s.name != id {
        parts.push(id);
    }
    if s.mute {
        parts.push("muted".to_string());
    }
    parts.push(fx);
    parts.join(" · ")
}

/// The parts of one strip (layout spec 3a; the touch plan's A7). In
/// Make: the name, the chain's summary (a tap opens the Effects look) and
/// the signal's (sends, pan and output; a tap opens the Sends look), the
/// fader, the level, and M and S stacked full width (and Arm on an input).
/// In Perform: the name, a pan row that only drags, a longer fader, the
/// level shown, and two big slots (MUTE and SOLO). The master has no
/// buttons: its Limit (Make) or its safety (Perform) where they go.
struct Parts {
    name: Rect,
    /// Make: the chain's summary.
    fx: Rect,
    /// Make: the sends, pan and output (the master: its safety).
    path: Rect,
    /// Perform: the pan row.
    pan: Rect,
    fader: Rect,
    value: Rect,
    /// The buttons, top first ([`Parts::n`] of them).
    buttons: [Rect; 3],
    n: usize,
    /// Where the buttons go, all of them.
    button_area: Rect,
}

impl Parts {
    /// The buttons there are: M and S, and Arm on an input in Make.
    fn buttons(&self) -> &[Rect] {
        &self.buttons[..self.n]
    }
}

/// A strip's name block, its colour stripe at the top.
const NAME_H: i32 = 44;
/// Make: the chain's summary (Dense, a row of one kind across the page).
const FX_H: i32 = 44;
/// Make: the sends, pan and output (Normal).
const PATH_H: i32 = 64;
/// Make: M and S, full width and stacked (Normal), and an input's Arm
/// under them (Dense).
const MAKE_BUTTON_H: i32 = 64;
/// MUTE and SOLO are easily confused: 12 px apart (layout spec 1a).
const MAKE_BUTTON_GAP: i32 = 12;
const ARM_H: i32 = 44;
/// Perform: the pan row, and the two slots with a dead gap between
/// (MUTE and SOLO, 92 x 85 px: 10.8 x 10 mm).
const PAN_ROW_H: i32 = 44;
const SLOT_H: i32 = 85;
const SLOT_GAP: i32 = 12;
/// The level under the fader.
const VALUE_H: i32 = 24;
/// Between a strip's parts: independent actions (layout spec 1a).
const PART_GAP: i32 = 8;

/// A strip's tag under its name (phase 4, board 6): two lines.
const TAG_H: i32 = 34;

/// The parts of strip `v` at `s`, in Make or (`perform`) in Perform: a
/// tagged strip's name block is taller, its fader shorter.
fn parts_of(s: Rect, v: &StripView, perform: bool) -> Parts {
    let tag = if v.tag.is_some() { TAG_H } else { 0 };
    let arm = v.input.is_some() && !perform;
    parts_with(s, tag, perform, arm)
}

fn parts_with(s: Rect, tag: i32, perform: bool, arm: bool) -> Parts {
    let inner = s.inset_xy(6, 6);
    let (name, rest) = inner.take_top(NAME_H + tag);
    let rest = rest.take_top(PART_GAP).1;
    let mut buttons = [Rect::default(); 3];
    if perform {
        let (pan, rest) = rest.take_top(PAN_ROW_H);
        // The pan and the fader are both played: 12 px apart.
        let rest = rest.take_top(SLOT_GAP).1;
        let (button_area, rest) = rest.take_bottom(2 * SLOT_H + SLOT_GAP);
        let (mute, solo) = button_area.take_top(SLOT_H);
        buttons[0] = mute;
        buttons[1] = solo.take_bottom(SLOT_H).0;
        let rest = rest.take_bottom(6).1;
        let (value, rest) = rest.take_bottom(VALUE_H);
        let fader = rest.take_bottom(4).1;
        return Parts {
            name,
            fx: Rect::default(),
            path: Rect::default(),
            pan,
            fader,
            value,
            buttons,
            n: 2,
            button_area,
        };
    }
    let (fx, rest) = rest.take_top(FX_H);
    let rest = rest.take_top(PART_GAP).1;
    let (path, rest) = rest.take_top(PATH_H);
    let rest = rest.take_top(PART_GAP).1;
    let area_h = 2 * MAKE_BUTTON_H + MAKE_BUTTON_GAP + if arm { PART_GAP + ARM_H } else { 0 };
    let (button_area, rest) = rest.take_bottom(area_h);
    let (m, more) = button_area.take_top(MAKE_BUTTON_H);
    let more = more.take_top(MAKE_BUTTON_GAP).1;
    let (solo, more) = more.take_top(MAKE_BUTTON_H);
    buttons[0] = m;
    buttons[1] = solo;
    if arm {
        buttons[2] = more.take_top(PART_GAP).1.take_top(ARM_H).0;
    }
    // A level and MUTE are easily confused: 12 px apart (layout spec 1a).
    let rest = rest.take_bottom(MAKE_BUTTON_GAP).1;
    let (value, rest) = rest.take_bottom(VALUE_H);
    let fader = rest.take_bottom(PART_GAP).1;
    Parts {
        name,
        fx,
        path,
        pan: Rect::default(),
        fader,
        value,
        buttons,
        n: if arm { 3 } else { 2 },
        button_area,
    }
}

/// Row `k`'s cells on the Sends look of a panel at `r` (the samples and
/// tests aim at them).
pub(crate) fn sends_cells(r: Rect, k: usize) -> crate::sends_look::Row {
    crate::sends_look::row_parts(grid(&layout(r)).row(k), FX_NAME_W)
}

/// The fader's travel inside its area: (top, bottom) of the cap's centre.
fn travel(f: Rect) -> (i32, i32) {
    (f.y + 10, f.bottom() - 10)
}

fn pos_y(f: Rect, pos: f32) -> i32 {
    let (top, bot) = travel(f);
    bot - ((bot - top) as f32 * pos.clamp(0.0, 1.0)).round() as i32
}

/// Inside a fader area: the scale, the track and the meter.
fn fader_columns(f: Rect, master: bool) -> (Rect, Rect, Rect) {
    let (scale, rest) = f.take_left(26);
    let meter_w = if master { 30 } else { 16 };
    let (meter, track) = rest.take_right(meter_w);
    (scale, track, meter)
}

/// What a finger is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Fader {
        id: StripId,
        y0: i32,
        pos0: f32,
        travel: f32,
    },
    Pan {
        id: StripId,
        x0: i32,
        v0: f32,
        w: f32,
        moved: bool,
    },
    Send {
        id: StripId,
        r: usize,
        x0: i32,
        pos0: f32,
        w: f32,
    },
    /// An effect on the Effects look: a tap picks it (or opens it), a
    /// drag along its row moves it to slot `to`.
    Fx {
        id: StripId,
        from: usize,
        x0: i32,
        to: Option<usize>,
        /// It went down on the effect's sidechain line ("SC from Kick",
        /// "Nothing picked"): a tap opens the sidechain list.
        line: bool,
    },
    /// The level under a fader: let go on it, the number pad opens
    /// ([`MixAction::TypeLevel`]); slid off first, nothing.
    Level { id: StripId, at: Rect },
    /// A button that acts on the lift inside it (layout spec 1a): slid
    /// off first, nothing.
    Tap { id: StripId, what: Tap, at: Rect },
    /// A curve on the EQ look, the dot under the finger if any: a tap
    /// picks the EQ (and the band), a tap again opens it; a dot that moves
    /// past the lift follows the finger.
    Eq {
        id: StripId,
        slot: usize,
        dot: Option<Dot>,
        x0: i32,
        y0: i32,
        moved: bool,
    },
}

/// What a [`Grab::Tap`] does when the finger lifts on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tap {
    /// A Make strip's chain summary: the Effects look on its row.
    Effects,
    /// A Make strip's sends, pan and output: the Sends look on its row.
    Sends,
    Mute,
    Solo,
    Arm,
    /// Send `k`'s pre toggle on the Sends look.
    Pre(usize),
    /// The output picker.
    Output,
}

/// The choices of the output picker: (OUTPUT value, track), in order.
pub fn output_choices(s: StripId, tracks: usize) -> Vec<(u8, u8)> {
    let mut v = vec![(output::MASTER as u8, 0)];
    let first_bus = match s {
        StripId::Bus(b) => b + 1,
        _ => 0,
    };
    v.extend((first_bus..BUSES).map(|b| (output::bus(b) as u8, 0)));
    if matches!(s, StripId::Input(_) | StripId::Pad(_)) {
        v.extend((0..tracks.min(64)).map(|t| (output::TRACK as u8, t as u8)));
    }
    v.push((output::NONE as u8, 0));
    v
}

/// What needs drawing again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dirty {
    Clean,
    Meters,
    /// The meters and the I/O view's panel (a section's live values).
    Panel,
    All,
}

/// The mixer panel. See the module docs.
pub struct MixerView {
    state: MixState,
    /// Faders, Sends, Effects or EQ.
    look: Look,
    /// The project is in Perform: Faders draws the Perform strip (big
    /// MUTE and SOLO, a longer fader, nothing else to hit by mistake).
    perform: bool,
    /// The effect picked on the Effects look, or the EQ on the EQ look:
    /// (strip, slot).
    pick: Option<(StripId, usize)>,
    /// The band picked on the EQ look.
    eq_dot: Option<Dot>,
    /// The EQ look's curves as last worked out: (values, curve).
    eq_curves: RefCell<Vec<(Vec<f32>, Vec<f32>)>>,
    /// The first strip shown.
    first: usize,
    /// The strip whose output picker is open.
    picker: Option<StripId>,
    /// The strip whose menu is open.
    menu: Option<StripId>,
    grabs: Fingers<Grab>,
    actions: Vec<MixAction>,
    dirty: Cell<Dirty>,
    /// Strips per page at the last paint, to keep the selection in view.
    per_page: Cell<usize>,
    /// The I/O view's sections, the one showing and its fingers.
    io: IoFrame,
    /// Audio rows per page at the last paint.
    io_per_page: Cell<usize>,
    /// The outside section last told it shows.
    io_live: Option<usize>,
    /// Outside sections that changed a setting on a touch, in order.
    io_changed: Vec<usize>,
    /// What a section's last touch, key or Q-Link couldn't do, and why
    /// ([`IoSection::take_note`]), until the app takes it.
    io_note: Option<String>,
}

impl Default for MixerView {
    fn default() -> MixerView {
        MixerView::new()
    }
}

impl MixerView {
    /// The strip a fader, pan or send target (the Faders or Sends look) is
    /// on, and which send (0 for a fader or a pan).
    fn strip_of(&self, t: &Target, r: Rect) -> Option<(StripId, usize)> {
        let l = layout(r);
        let strips: Vec<StripId> = if self.look == Look::Sends {
            self.sends_rows(&l).into_iter().map(|(id, _)| id).collect()
        } else {
            let mut v: Vec<StripId> = self
                .shown(&l)
                .into_iter()
                .map(|(i, _)| self.state.strips[i].id)
                .collect();
            v.push(self.state.master.id);
            v
        };
        let (k, j) = match t.id {
            "send" => (t.index / 4, t.index % 4),
            _ => (t.index, 0),
        };
        Some((*strips.get(k)?, j))
    }

    /// The strip parameter a fader, pan, send, M or S target sets (for
    /// SHIFT + a touch's MIDI Learn): its strip and the parameter's id.
    /// None for other targets, and for a send the strip doesn't have.
    pub fn param_of(&self, t: &Target, r: Rect) -> Option<(StripId, u16)> {
        let (strip, j) = self.strip_of(t, r)?;
        let s = self.state.strip(strip)?;
        match t.id {
            "fader" => Some((strip, mx::VOLUME)),
            "pan" => Some((strip, mx::PAN)),
            "mute" => Some((strip, mx::MUTE)),
            "solo" => Some((strip, mx::SOLO)),
            "send" => {
                s.sends?.get(j)?;
                Some((strip, mx::send(j)))
            }
            _ => None,
        }
    }
}

impl MixerView {
    pub fn new() -> MixerView {
        MixerView {
            state: MixState::default(),
            look: Look::Faders,
            perform: false,
            pick: None,
            eq_dot: None,
            eq_curves: RefCell::new(Vec::new()),
            first: 0,
            picker: None,
            menu: None,
            grabs: Fingers::new(),
            actions: Vec::new(),
            dirty: Cell::new(Dirty::All),
            per_page: Cell::new(10),
            io: IoFrame::new(),
            io_per_page: Cell::new(12),
            io_live: None,
            io_changed: Vec::new(),
            io_note: None,
        }
    }

    pub fn state(&self) -> &MixState {
        &self.state
    }

    /// Shows a new state. Only the meters are drawn again when nothing else
    /// changed. A strip the fingers are holding keeps the finger's value.
    pub fn update(&mut self, mut st: MixState) {
        for (_, g) in self.grabs.iter() {
            let (id, ids): (StripId, Vec<u16>) = match *g {
                Grab::Fader { id, .. } => (id, vec![mx::VOLUME]),
                Grab::Pan { id, .. } => (id, vec![mx::PAN]),
                Grab::Send { id, r, .. } => (id, vec![mx::send(r)]),
                Grab::Fx { .. } | Grab::Level { .. } | Grab::Tap { .. } => continue,
                Grab::Eq {
                    id,
                    slot,
                    moved: true,
                    ..
                } => {
                    // The dot under the finger stays where the finger has it.
                    let old = self
                        .state
                        .strip(id)
                        .and_then(|s| s.inserts.get(slot))
                        .and_then(|i| i.eq.clone());
                    if let Some(ins) = st.strip_mut(id).and_then(|s| s.inserts.get_mut(slot)) {
                        if ins.eq.is_some() && old.is_some() {
                            ins.eq = old;
                        }
                    }
                    continue;
                }
                Grab::Eq { .. } => continue,
            };
            if let (Some(old), Some(new)) = (self.state.strip(id).cloned(), st.strip_mut(id)) {
                for p in ids {
                    if p == mx::VOLUME {
                        new.volume_db = old.volume_db;
                    }
                    if p == mx::PAN {
                        new.pan = old.pan;
                    }
                    if let (Some(a), Some(b)) = (old.sends.as_ref(), new.sends.as_mut()) {
                        if (6..10).contains(&p) {
                            b[(p - 6) as usize].db = a[(p - 6) as usize].db;
                        }
                    }
                }
            }
        }
        let kind = if strip_meters(&self.state) == strip_meters(&st) {
            Dirty::Meters
        } else {
            Dirty::All
        };
        if st.view != self.state.view || !st.takes {
            self.menu = None;
        }
        if st.view != self.state.view {
            self.first = 0;
            self.io.top = 0;
            self.picker = None;
            self.pick = None;
        }
        if let Some((id, slot)) = self.pick {
            let gone = st.strip(id).is_none_or(|s| slot >= s.inserts.len());
            let not_eq = self.look == Look::Eq
                && st
                    .strip(id)
                    .is_none_or(|s| s.inserts.get(slot).is_none_or(|i| i.eq.is_none()));
            if gone || not_eq {
                self.pick = None;
                self.eq_dot = None;
            }
        }
        let follow = st.selected != self.state.selected;
        self.state = st;
        if follow {
            self.follow_selection();
        }
        self.first = self.first.min(self.state.strips.len().saturating_sub(1));
        self.io.top = self.io.top.min(self.state.strips.len().saturating_sub(1));
        self.sync_shown();
        self.mark(kind);
    }

    /// Pages to the selected strip if it is off the page.
    fn follow_selection(&mut self) {
        let Some(i) = self
            .state
            .selected
            .and_then(|s| self.state.strips.iter().position(|v| v.id == s))
        else {
            return;
        };
        if self.state.view == MixView::Io {
            let n = self.io_per_page.get().max(1);
            if i < self.io.top || i >= self.io.top + n {
                self.io.top = i / n * n;
            }
        } else {
            let n = self.per_page.get().max(1);
            if i < self.first || i >= self.first + n {
                self.first = i / n * n;
            }
        }
    }

    /// The actions the touches made since the last call.
    pub fn take_actions(&mut self) -> Vec<MixAction> {
        std::mem::take(&mut self.actions)
    }

    /// Shows a change made away from the screen (a Q-Link turned) at once,
    /// before the next state comes. It is not queued: the caller sends it.
    pub fn show(&mut self, a: &MixAction) {
        if let MixAction::Set { strip, id, value } = *a {
            if let Some(s) = self.state.strip_mut(strip) {
                apply(s, id, value);
                self.mark(Dirty::All);
            }
        }
    }

    /// The look showing: Faders, Sends, Effects or EQ.
    pub fn look(&self) -> Look {
        self.look
    }

    /// Whether Faders draws the Perform strip.
    pub fn perform(&self) -> bool {
        self.perform
    }

    /// The project went to Perform (`on`) or back to Make: Faders draws
    /// its strips for it. The same mixer state drawn another way: no
    /// value, selection or routing changes. A menu or the output picker
    /// closes, and a finger on a strip lets go.
    pub fn set_perform(&mut self, on: bool) {
        if on != self.perform {
            self.perform = on;
            self.menu = None;
            self.picker = None;
            self.grabs.clear();
            self.mark(Dirty::All);
        }
    }

    /// Shows look `l` (TRACK MIX opens on Faders), as a tap on the toggle
    /// does but without telling the app. A menu or the output picker
    /// closes.
    pub fn set_look(&mut self, l: Look) {
        if l != self.look {
            self.look = l;
            self.menu = None;
            self.picker = None;
            self.pick = None;
            self.eq_dot = None;
            self.grabs.clear();
            self.mark(Dirty::All);
        }
    }

    /// The band picked on the EQ look, with its EQ ([`MixerView::pick`]).
    pub fn eq_dot(&self) -> Option<Dot> {
        self.eq_dot.filter(|_| self.pick.is_some())
    }

    /// The effect picked on the Effects look, whose knobs the Q-Links
    /// turn: (strip, slot).
    pub fn pick(&self) -> Option<(StripId, usize)> {
        self.pick
    }

    /// Strips a page shows in the look showing (the selection pages by
    /// it, and so do < and >).
    fn page_size(&self, l: &Layout) -> usize {
        match self.look {
            Look::Faders => l.per_page,
            Look::Sends | Look::Effects | Look::Eq => grid(l).rows.min(l.per_page.max(1)),
        }
    }

    /// The Effects look's rows on this page: (strip, row).
    fn rows(&self, l: &Layout) -> Vec<(usize, Rect)> {
        if self.state.view == MixView::Io || self.look == Look::Faders {
            return Vec::new();
        }
        let g = grid(l);
        let n = self.state.strips.len();
        (self.first..n.min(self.first + self.page_size(l)))
            .enumerate()
            .map(|(k, i)| (i, g.row(k)))
            .collect()
    }

    /// The first strip shown (pages move it by a page of strips).
    pub fn first(&self) -> usize {
        self.first
    }

    pub fn picker(&self) -> Option<StripId> {
        self.picker
    }

    /// The strip whose menu is open.
    pub fn menu(&self) -> Option<StripId> {
        self.menu
    }

    fn mark(&self, d: Dirty) {
        self.dirty.set(self.dirty.get().max(d));
    }

    fn push(&mut self, a: MixAction) {
        if let MixAction::Set { strip, id, value } = a {
            if let Some(s) = self.state.strip_mut(strip) {
                apply(s, id, value);
            }
        }
        if let MixAction::SetInsert {
            strip,
            slot,
            id,
            value,
        } = a
        {
            let vals = self
                .state
                .strip_mut(strip)
                .and_then(|s| s.inserts.get_mut(slot))
                .and_then(|i| i.eq.as_mut());
            if let Some(v) = vals.and_then(|v| v.get_mut(id as usize)) {
                *v = value;
            }
        }
        self.actions.push(a);
    }

    fn set(&mut self, strip: StripId, id: u16, value: f32) {
        self.push(MixAction::Set { strip, id, value });
    }

    /// The strips shown and their rectangles (none in the I/O view, which
    /// shows rows).
    fn shown(&self, l: &Layout) -> Vec<(usize, Rect)> {
        if self.state.view == MixView::Io || self.look != Look::Faders {
            return Vec::new();
        }
        let n = self.state.strips.len();
        (self.first..n.min(self.first + l.per_page))
            .enumerate()
            .map(|(col, i)| {
                (
                    i,
                    Rect::new(
                        l.strips.x + col as i32 * (STRIP_W + GAP),
                        l.strips.y,
                        STRIP_W,
                        l.strips.h,
                    ),
                )
            })
            .collect()
    }

    fn picker_rect(l: &Layout) -> Rect {
        l.strips.inset_xy(24, 40)
    }

    fn picker_buttons(&self, l: &Layout, s: StripId) -> Vec<((u8, u8), Rect)> {
        let area = MixerView::picker_rect(l).inset(18).take_top(40).1;
        let (bw, bh) = (122, 40);
        let cols = ((area.w + 8) / (bw + 8)).max(1);
        output_choices(s, self.state.tracks)
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let (col, row) = (i as i32 % cols, i as i32 / cols);
                (
                    c,
                    Rect::new(area.x + col * (bw + 8), area.y + row * (bh + 8), bw, bh),
                )
            })
            .collect()
    }

    /// Where the open picker shows an output choice (OUTPUT value, track).
    pub fn picker_button(&self, r: Rect, choice: (u8, u8)) -> Option<Rect> {
        let s = self.picker?;
        self.picker_buttons(&layout(r), s)
            .into_iter()
            .find(|(c, _)| *c == choice)
            .map(|(_, b)| b)
    }

    fn menu_rect(&self, l: &Layout, s: StripId) -> Rect {
        let n = self.state.strip(s).map_or(1, |v| StripMenu::items(v).len()) as i32;
        let (w, h) = (620.min(l.strips.w), 18 + 30 + 10 + n * 64 + 8);
        Rect::new(
            l.strips.x + (l.strips.w - w) / 2,
            l.strips.y + (l.strips.h - h) / 3,
            w,
            h,
        )
    }

    fn menu_buttons(&self, l: &Layout, s: StripId) -> Vec<(StripMenu, Rect)> {
        let Some(v) = self.state.strip(s) else {
            return Vec::new();
        };
        let area = self.menu_rect(l, s).inset(18).take_top(40).1;
        StripMenu::items(v)
            .into_iter()
            .enumerate()
            .map(|(i, m)| (m, Rect::new(area.x, area.y + i as i32 * 64, area.w, 56)))
            .collect()
    }

    /// Where the open menu shows `item`.
    pub fn menu_button(&self, r: Rect, item: StripMenu) -> Option<Rect> {
        let s = self.menu?;
        self.menu_buttons(&layout(r), s)
            .into_iter()
            .find(|(m, _)| *m == item)
            .map(|(_, b)| b)
    }

    fn down(&mut self, finger: u64, x: i32, y: i32, r: Rect) {
        let l = layout(r);
        if let Some(s) = self.menu {
            let pick = self
                .menu_buttons(&l, s)
                .into_iter()
                .find(|(_, b)| b.contains(x, y))
                .map(|(m, _)| m);
            if let Some(m) = pick {
                self.push(m.action(s));
                self.menu = None;
            } else if !self.menu_rect(&l, s).contains(x, y) {
                self.menu = None;
            }
            return;
        }
        if let Some(s) = self.picker {
            let pick = self
                .picker_buttons(&l, s)
                .into_iter()
                .find(|(_, b)| b.contains(x, y))
                .map(|(c, _)| c);
            if let Some((out, track)) = pick {
                if out == output::TRACK as u8 {
                    self.set(s, mx::OUTPUT_TRACK, f32::from(track));
                }
                self.set(s, mx::OUTPUT, f32::from(out));
                self.picker = None;
            } else if !MixerView::picker_rect(&l).contains(x, y) {
                self.picker = None;
            }
            return;
        }
        if let Some(i) = l.chips.iter().position(|c| c.contains(x, y)) {
            let v = MixView::ALL[i];
            if v != self.state.view {
                self.state.view = v;
                self.first = 0;
                self.io.top = 0;
                self.pick = None;
                self.eq_dot = None;
                self.push(MixAction::View(v));
            }
            return;
        }
        if let Some(look) = look_halves(l.look)
            .into_iter()
            .zip(Look::ALL)
            .find(|(h, _)| h.contains(x, y))
            .map(|(_, look)| look)
        {
            if look != self.look {
                self.set_look(look);
                self.push(MixAction::Look(look));
            }
            return;
        }
        let n = self.state.strips.len();
        if l.prev.contains(x, y) || l.next.contains(x, y) {
            let d = if l.prev.contains(x, y) { -1 } else { 1 };
            let per_page = self.page_size(&l);
            if self.state.view == MixView::Io {
                self.io_page(d);
            } else if d < 0 {
                self.first = self.first.saturating_sub(per_page);
            } else if self.first + per_page < n {
                self.first += per_page;
            }
            return;
        }
        if self.state.view == MixView::Io && l.strips.contains(x, y) {
            self.io_down(finger, x, y, &l);
            return;
        }
        if self.look == Look::Effects {
            self.fx_down(finger, x, y, &l);
            return;
        }
        if self.look == Look::Eq {
            self.eq_down(finger, x, y, &l);
            return;
        }
        if self.look == Look::Sends {
            self.sends_down(finger, x, y, &l);
            return;
        }
        let mut hits: Vec<(StripId, Rect)> = self
            .shown(&l)
            .into_iter()
            .map(|(i, sr)| (self.state.strips[i].id, sr))
            .collect();
        hits.push((StripId::Master, l.master));
        let Some((id, sr)) = hits.into_iter().find(|(_, sr)| sr.contains(x, y)) else {
            return;
        };
        let Some(s) = self.state.strip(id).cloned() else {
            return;
        };
        let p = parts_of(sr, &s, self.perform);
        if p.name.contains(x, y) {
            self.name_down(id);
            return;
        }
        let tap = |what: Tap, at: Rect| Grab::Tap { id, what, at };
        if !self.perform && p.fx.contains(x, y) {
            self.grabs.put(finger, tap(Tap::Effects, p.fx));
            return;
        }
        if !self.perform && p.path.contains(x, y) && id != StripId::Master {
            self.grabs.put(finger, tap(Tap::Sends, p.path));
            return;
        }
        if self.perform && p.pan.contains(x, y) {
            // The pan row only drags: a tap leaves it.
            self.grabs.put(
                finger,
                Grab::Pan {
                    id,
                    x0: x,
                    v0: s.pan,
                    w: p.pan.w.max(1) as f32,
                    moved: false,
                },
            );
            return;
        }
        if p.fader.contains(x, y) {
            let (top, bot) = travel(p.fader);
            self.grabs.put(
                finger,
                Grab::Fader {
                    id,
                    y0: y,
                    pos0: db_to_pos(s.volume_db),
                    travel: (bot - top).max(1) as f32,
                },
            );
            return;
        }
        if !self.perform && p.value.contains(x, y) {
            self.grabs.put(finger, Grab::Level { id, at: p.value });
            return;
        }
        if id == StripId::Master {
            return;
        }
        let what = [Tap::Mute, Tap::Solo, Tap::Arm];
        let Some(k) = p.buttons().iter().position(|b| b.contains(x, y)) else {
            return;
        };
        if self.perform {
            // Perform's slots act as the finger lands: timing matters.
            self.tap(id, what[k]);
        } else {
            // Make's act on the lift, so a slip slides off (Jay's second
            // review: a studio mute is release-inside).
            self.grabs.put(finger, tap(what[k], p.buttons[k]));
        }
    }

    /// A strip's name pressed (Faders, Sends, Effects, EQ): it is selected
    /// for the Q-Links; the selected one's opens its menu, with takes.
    fn name_down(&mut self, id: StripId) {
        if self.state.selected == Some(id) {
            if self.state.takes {
                self.menu = Some(id);
            }
        } else {
            self.state.selected = Some(id);
            self.push(MixAction::Select(id));
        }
    }

    /// What a [`Tap`] does on strip `id`.
    fn tap(&mut self, id: StripId, what: Tap) {
        let Some(s) = self.state.strip(id).cloned() else {
            return;
        };
        match what {
            Tap::Effects | Tap::Sends => {
                if self.state.selected != Some(id) {
                    self.state.selected = Some(id);
                    self.push(MixAction::Select(id));
                }
                let look = if what == Tap::Effects {
                    Look::Effects
                } else {
                    Look::Sends
                };
                self.set_look(look);
                self.push(MixAction::Look(look));
            }
            Tap::Mute => self.set(id, mx::MUTE, if s.mute { 0.0 } else { 1.0 }),
            Tap::Solo => self.set(id, mx::SOLO, if s.solo { 0.0 } else { 1.0 }),
            Tap::Arm => {
                let arm = s.input.is_some_and(|i| i.arm);
                self.set(id, mx::ARM, if arm { 0.0 } else { 1.0 });
            }
            Tap::Pre(k) => {
                if let Some(sends) = s.sends {
                    self.set(id, mx::send_pre(k), if sends[k].pre { 0.0 } else { 1.0 });
                }
            }
            Tap::Output => {
                if s.has_output() {
                    self.picker = Some(id);
                }
            }
        }
    }

    /// The Sends look's rows on this page and the master's, pinned:
    /// (strip, row).
    fn sends_rows(&self, l: &Layout) -> Vec<(StripId, Rect)> {
        let mut rows: Vec<(StripId, Rect)> = self
            .rows(l)
            .into_iter()
            .map(|(i, row)| (self.state.strips[i].id, row))
            .collect();
        rows.push((StripId::Master, grid(l).master));
        rows
    }

    /// A finger on the Sends look: a strip's name selects it (again: its
    /// menu, with takes); the pan and a send's level drag from where the
    /// finger lands; a send's pre and the output act on the lift.
    fn sends_down(&mut self, finger: u64, x: i32, y: i32, l: &Layout) {
        let Some((id, row)) = self
            .sends_rows(l)
            .into_iter()
            .find(|(_, row)| row.contains(x, y))
        else {
            return;
        };
        let Some(s) = self.state.strip(id).cloned() else {
            return;
        };
        let r = crate::sends_look::row_parts(row, FX_NAME_W);
        if r.name.contains(x, y) {
            self.name_down(id);
            return;
        }
        if r.pan.contains(x, y) {
            self.grabs.put(
                finger,
                Grab::Pan {
                    id,
                    x0: x,
                    v0: s.pan,
                    w: r.pan.w.max(1) as f32,
                    moved: false,
                },
            );
            return;
        }
        if r.output.contains(x, y) && s.has_output() {
            self.grabs.put(
                finger,
                Grab::Tap {
                    id,
                    what: Tap::Output,
                    at: r.output,
                },
            );
            return;
        }
        let Some(sends) = s.sends else {
            return;
        };
        for k in 0..RETURNS {
            if r.pre[k].contains(x, y) {
                self.grabs.put(
                    finger,
                    Grab::Tap {
                        id,
                        what: Tap::Pre(k),
                        at: r.pre[k],
                    },
                );
                return;
            }
            if r.send[k].contains(x, y) {
                self.grabs.put(
                    finger,
                    Grab::Send {
                        id,
                        r: k,
                        x0: x,
                        pos0: db_to_pos(sends[k].db),
                        w: r.send[k].w.max(1) as f32,
                    },
                );
                return;
            }
        }
    }

    /// A finger on the Effects look: a strip's name selects it (again: its
    /// menu, with takes), + FX opens the effect picker for the strip's
    /// first free slot, and an effect is held until the finger lifts (a
    /// tap picks it or opens it, a drag moves it).
    fn fx_down(&mut self, finger: u64, x: i32, y: i32, l: &Layout) {
        let mut rows: Vec<(StripId, Rect)> = self
            .rows(l)
            .into_iter()
            .map(|(i, row)| (self.state.strips[i].id, row))
            .collect();
        rows.push((StripId::Master, grid(l).master));
        let Some((id, row)) = rows.into_iter().find(|(_, row)| row.contains(x, y)) else {
            return;
        };
        let Some(s) = self.state.strip(id) else {
            return;
        };
        let (name, cells) = row_cells(row);
        if name.contains(x, y) {
            // The Q-Links go back to the strip.
            self.pick = None;
            if self.state.selected == Some(id) {
                if self.state.takes {
                    self.menu = Some(id);
                }
            } else {
                self.state.selected = Some(id);
                self.push(MixAction::Select(id));
            }
            return;
        }
        let (len, slots) = (s.inserts.len(), s.slots.min(FX_COLS));
        if let Some(k) = cells.iter().position(|c| c.contains(x, y)) {
            if k < len {
                let ins = &s.inserts[k];
                // The second line, as `paint_row` places it.
                let line = ins.on
                    && (ins.key.is_some() || ins.nothing_picked)
                    && y >= cells[k].y + 6 + (cells[k].h - 12) / 2 + 2;
                self.grabs.put(
                    finger,
                    Grab::Fx {
                        id,
                        from: k,
                        x0: x,
                        to: None,
                        line,
                    },
                );
            } else if k == len && len < slots {
                self.push(MixAction::AddInsert { strip: id });
            }
        }
    }

    /// A tap on effect `slot` of strip `id` on the Effects look: picked,
    /// or opened when it already was.
    fn fx_tap(&mut self, id: StripId, slot: usize) {
        if self.pick == Some((id, slot)) {
            self.push(MixAction::OpenInsert { strip: id, slot });
        } else {
            self.pick = Some((id, slot));
            self.push(MixAction::PickInsert { strip: id, slot });
        }
    }

    /// Effect `from` of strip `id` dragged to slot `to`: it moves on the
    /// screen at once (the pick with it) and the app moves it in the mixer.
    fn fx_move(&mut self, id: StripId, from: usize, to: usize) {
        if let Some(s) = self.state.strip_mut(id) {
            if from < s.inserts.len() && to < s.inserts.len() {
                let fx = s.inserts.remove(from);
                s.inserts.insert(to, fx);
            }
        }
        if let Some((pid, slot)) = self.pick.filter(|(pid, _)| *pid == id) {
            self.pick = Some((pid, moved_slot(slot, from, to)));
        }
        self.push(MixAction::MoveInsert {
            strip: id,
            from,
            to,
        });
    }

    /// The slot a finger at `x` is over on strip `id`'s row (one of its
    /// effects), while it drags one.
    fn fx_slot_at(&self, id: StripId, x: i32, r: Rect) -> Option<usize> {
        let len = self.state.strip(id)?.inserts.len();
        let (_, cells) = row_cells(Rect::new(0, 0, grid(&layout(r)).width, FX_ROW_H));
        let left = grid(&layout(r)).left;
        let k = cells
            .iter()
            .position(|c| x < left + c.right() + 4)
            .unwrap_or(FX_COLS - 1);
        Some(k.min(len.saturating_sub(1)))
    }

    /// A finger on the EQ look: a strip's name selects it (as on
    /// Effects), "+ EQ" adds one, and a curve is held until the finger
    /// lifts or its dot moves.
    fn eq_down(&mut self, finger: u64, x: i32, y: i32, l: &Layout) {
        let mut rows: Vec<(StripId, Rect)> = self
            .rows(l)
            .into_iter()
            .map(|(i, row)| (self.state.strips[i].id, row))
            .collect();
        rows.push((StripId::Master, grid(l).master));
        let Some((id, row)) = rows.into_iter().find(|(_, row)| row.contains(x, y)) else {
            return;
        };
        let Some(s) = self.state.strip(id) else {
            return;
        };
        let (name, graph, _) = eq_look::row_parts(row, FX_NAME_W);
        if name.contains(x, y) {
            self.pick = None;
            self.eq_dot = None;
            if self.state.selected == Some(id) {
                if self.state.takes {
                    self.menu = Some(id);
                }
            } else {
                self.state.selected = Some(id);
                self.push(MixAction::Select(id));
            }
            return;
        }
        if !graph.contains(x, y) {
            return;
        }
        let Some(slot) = s.eq_slot() else {
            if s.inserts.len() < s.slots {
                self.push(MixAction::AddEq { strip: id });
            }
            return;
        };
        let vals = s.inserts[slot].eq.clone().unwrap_or_default();
        let curve = self.eq_curve(&vals);
        let dot = eq_look::hit(&vals, &curve, eq_look::plot(graph), x, y);
        self.grabs.put(
            finger,
            Grab::Eq {
                id,
                slot,
                dot,
                x0: x,
                y0: y,
                moved: false,
            },
        );
    }

    /// Picks strip `id`'s EQ in `slot` and band `dot` for the Q-Links.
    fn eq_pick(&mut self, id: StripId, slot: usize, dot: Option<Dot>) {
        if self.pick != Some((id, slot)) {
            self.pick = Some((id, slot));
            self.eq_dot = None;
            self.push(MixAction::PickInsert { strip: id, slot });
        }
        if let Some(d) = dot.filter(|d| self.eq_dot != Some(*d)) {
            self.eq_dot = Some(d);
            self.push(MixAction::FocusInsert {
                strip: id,
                slot,
                id: d.main_id(),
            });
        }
    }

    /// A tap on a curve: its EQ (and the band tapped) picked, or opened
    /// when the tap lands on what is already picked.
    fn eq_tap(&mut self, id: StripId, slot: usize, dot: Option<Dot>) {
        let same = self.pick == Some((id, slot)) && (dot.is_none() || dot == self.eq_dot);
        if same {
            self.push(MixAction::OpenInsert { strip: id, slot });
        } else {
            self.eq_pick(id, slot, dot);
        }
    }

    /// Dot `dot` of strip `id`'s EQ under a finger at `(x, y)`.
    fn eq_drag(&mut self, id: StripId, slot: usize, dot: Dot, x: i32, y: i32, r: Rect) {
        let l = layout(r);
        let row = self
            .rows(&l)
            .into_iter()
            .find(|(i, _)| self.state.strips[*i].id == id)
            .map(|(_, row)| row)
            .or((id == StripId::Master).then(|| grid(&l).master));
        let Some(row) = row else {
            return;
        };
        let Some(vals) = self
            .state
            .strip(id)
            .and_then(|s| s.inserts.get(slot))
            .and_then(|i| i.eq.clone())
        else {
            return;
        };
        let (_, graph, _) = eq_look::row_parts(row, FX_NAME_W);
        for (p, value) in eq_look::drag(&vals, eq_look::plot(graph), dot, x, y) {
            if vals.get(p as usize) != Some(&value) {
                self.push(MixAction::SetInsert {
                    strip: id,
                    slot,
                    id: p,
                    value,
                });
            }
        }
    }

    /// Where dot `dot` of strip `id`'s EQ is drawn on the EQ look, in the
    /// widget's rectangle `r` (None off the page or without an EQ).
    pub fn eq_dot_at(&self, r: Rect, id: StripId, dot: Dot) -> Option<(i32, i32)> {
        let l = layout(r);
        let row = self
            .rows(&l)
            .into_iter()
            .find(|(i, _)| self.state.strips[*i].id == id)
            .map(|(_, row)| row)
            .or((id == StripId::Master).then(|| grid(&l).master))?;
        let s = self.state.strip(id)?;
        let vals = s.inserts.get(s.eq_slot()?)?.eq.clone()?;
        let (_, graph, _) = eq_look::row_parts(row, FX_NAME_W);
        let (x, y) = eq_look::dot_at(&vals, &self.eq_curve(&vals), eq_look::plot(graph), dot);
        Some((x.round() as i32, y.round() as i32))
    }

    /// An EQ's curve, worked out again only when its values change.
    fn eq_curve(&self, vals: &[f32]) -> Vec<f32> {
        let mut cache = self.eq_curves.borrow_mut();
        if let Some((_, c)) = cache.iter().find(|(v, _)| v.as_slice() == vals) {
            return c.clone();
        }
        let c = eq_look::curve(vals);
        if cache.len() >= 24 {
            cache.remove(0);
        }
        cache.push((vals.to_vec(), c.clone()));
        c
    }

    fn drag(&mut self, finger: u64, x: i32, y: i32, r: Rect) {
        if self.io_finger(finger, Touch::Move, x, y, r) {
            return;
        }
        let Some(g) = self.grabs.get(finger).copied() else {
            return;
        };
        match g {
            Grab::Eq {
                id,
                slot,
                dot: Some(dot),
                x0,
                y0,
                moved,
            } => {
                if !moved && (x - x0).abs().max((y - y0).abs()) < eq_look::LIFT {
                    return;
                }
                if !moved {
                    self.grabs.put(
                        finger,
                        Grab::Eq {
                            id,
                            slot,
                            dot: Some(dot),
                            x0,
                            y0,
                            moved: true,
                        },
                    );
                    self.eq_pick(id, slot, Some(dot));
                }
                self.eq_drag(id, slot, dot, x, y, r);
            }
            Grab::Eq { .. } | Grab::Level { .. } | Grab::Tap { .. } => {}
            Grab::Fx {
                id,
                from,
                x0,
                to,
                line,
            } => {
                if to.is_none() && (x - x0).abs() < 12 {
                    return;
                }
                let at = self.fx_slot_at(id, x, r).or(Some(from));
                self.grabs.put(
                    finger,
                    Grab::Fx {
                        id,
                        from,
                        x0,
                        to: at,
                        line,
                    },
                );
            }
            Grab::Fader {
                id,
                y0,
                pos0,
                travel,
            } => {
                let db = pos_to_db(pos0 + (y0 - y) as f32 / travel);
                if self.state.strip(id).is_some_and(|s| s.volume_db != db) {
                    self.set(id, mx::VOLUME, db);
                }
            }
            Grab::Pan {
                id,
                x0,
                v0,
                w,
                moved,
            } => {
                if !moved && (x - x0).abs() < 6 {
                    return;
                }
                self.grabs.put(
                    finger,
                    Grab::Pan {
                        id,
                        x0,
                        v0,
                        w,
                        moved: true,
                    },
                );
                let v = (v0 + (x - x0) as f32 * 200.0 / w)
                    .clamp(-100.0, 100.0)
                    .round();
                if self.state.strip(id).is_some_and(|s| s.pan != v) {
                    self.set(id, mx::PAN, v);
                }
            }
            Grab::Send { id, r, x0, pos0, w } => {
                let db = pos_to_db(pos0 + (x - x0) as f32 / w);
                if self
                    .state
                    .strip(id)
                    .and_then(|s| s.sends)
                    .is_some_and(|s| s[r].db != db)
                {
                    self.set(id, mx::send(r), db);
                }
            }
        }
    }

    fn lift(&mut self, finger: u64, cancel: bool, x: i32, y: i32, r: Rect) {
        let phase = if cancel { Touch::Cancel } else { Touch::Up };
        if self.io_finger(finger, phase, x, y, r) {
            return;
        }
        let Some(g) = self.grabs.take(finger) else {
            return;
        };
        if let Grab::Tap { id, what, at } = g {
            if !cancel && at.contains(x, y) {
                self.tap(id, what);
            }
        }
        if let Grab::Level { id, at } = g {
            let s = self
                .state
                .strip(id)
                .filter(|_| !cancel && at.contains(x, y));
            if let Some((db, name)) = s.map(|s| (s.volume_db, s.name.clone())) {
                self.push(MixAction::TypeLevel {
                    strip: id,
                    db,
                    name,
                });
            }
        }
        if let Grab::Eq {
            id,
            slot,
            dot,
            moved,
            ..
        } = g
        {
            match (moved, dot) {
                (true, Some(d)) => {
                    let ids: &[u16] = match d {
                        Dot::Band(b) => &[mpfree_fx::ids::eq::freq(b), mpfree_fx::ids::eq::gain(b)],
                        Dot::Low => &[mpfree_fx::ids::eq::LOW_CUT],
                        Dot::High => &[mpfree_fx::ids::eq::HIGH_CUT],
                    };
                    for &p in ids {
                        self.push(MixAction::ReleaseInsert {
                            strip: id,
                            slot,
                            id: p,
                        });
                    }
                }
                _ if cancel => {}
                _ => self.eq_tap(id, slot, dot),
            }
        }
        if let Grab::Fx {
            id, from, to, line, ..
        } = g
        {
            match to {
                _ if cancel => {}
                None if line => {
                    self.pick = Some((id, from));
                    self.push(MixAction::OpenSidechain {
                        strip: id,
                        slot: from,
                    });
                }
                None => self.fx_tap(id, from),
                Some(to) if to != from => self.fx_move(id, from, to),
                Some(_) => {}
            }
        }
    }

    // ------------------------------------------------------------ painting

    fn paint_head(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        for (i, v) in MixView::ALL.iter().enumerate() {
            let c = l.chips[i];
            let on = *v == self.state.view;
            cx.canvas
                .fill_round_rect(c, t.round(10), if on { t.raised } else { t.surface });
            if on {
                cx.canvas.fill_round_rect(
                    Rect::new(c.x + 20, c.bottom() - 4, c.w - 40, 3),
                    1,
                    t.accent,
                );
            }
            let count = self.state.counts.get(i).filter(|_| !l.compact);
            let Some(&count) = count else {
                // The I/O view, or a narrow chip: no count.
                cx.text(
                    if on { FontId::Bold } else { FontId::Sans },
                    t.size_body,
                    c,
                    Align::Center,
                    if on { t.text } else { t.text_dim },
                    v.label(),
                );
                continue;
            };
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body,
                c.inset_xy(14, 0),
                Align::Left,
                if on { t.text } else { t.text_dim },
                v.label(),
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                c.inset_xy(14, 0).take_right(34).0,
                Align::Right,
                t.text_faint,
                &count.to_string(),
            );
        }
        let [back, on] = self.arrows(l);
        for (b, s, live) in [(l.prev, "<", back), (l.next, ">", on)] {
            cx.canvas
                .fill_round_rect(b, t.round(10), if live { t.raised } else { t.surface });
            cx.text(
                FontId::Bold,
                t.size_title,
                b,
                Align::Center,
                if live { t.text } else { t.text_faint },
                s,
            );
        }
        let (first, shown, n) = self.span(l);
        let page = if self.state.view == MixView::Io && self.io.section().is_some() {
            String::new()
        } else if n == 0 {
            "none".to_string()
        } else {
            format!("{}-{} of {n}", first + 1, first + shown)
        };
        cx.text(
            FontId::Sans,
            t.size_small,
            l.page,
            Align::Center,
            t.text_dim,
            &page,
        );
        // The three looks, where the mixer's CPU chip was.
        cx.canvas.fill_round_rect(l.look, t.round(12), t.surface);
        for (half, look) in look_halves(l.look).into_iter().zip(Look::ALL) {
            let on = look == self.look;
            if on {
                cx.canvas.fill_round_rect(half, t.round(8), t.picked());
            }
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_small,
                half,
                Align::Center,
                if on { t.on_picked() } else { t.text_dim },
                look.label(),
            );
        }
    }

    /// The whole mixer's cost, inserts included, against the mix and fx
    /// budgets together (1 % and 10 % of a period on the MPC), while
    /// profiling: on the master's name row (Faders) or the title line
    /// (Effects).
    fn paint_cost(&self, cx: &mut PaintCx, r: Rect, long: bool) {
        let Some(c) = self.state.cost_pct else {
            return;
        };
        let t = cx.theme.clone();
        let tone = if c <= 5.0 {
            Tone::Good
        } else if c <= 11.0 {
            Tone::Warn
        } else {
            Tone::Bad
        };
        let text = if long {
            format!("Mix + FX {c:.1} %")
        } else {
            format!("{c:.1} %")
        };
        cx.text(
            FontId::Sans,
            t.size_small,
            r,
            Align::Right,
            t.tone(tone),
            &text,
        );
    }

    fn paint_strip(&self, cx: &mut PaintCx, s: &StripView, sr: Rect) {
        let t = cx.theme.clone();
        let p = parts_of(sr, s, self.perform);
        let master = s.id == StripId::Master;
        let selected = self.state.selected == Some(s.id);
        cx.canvas.fill_round_rect(sr, t.round(12), t.surface);
        if selected {
            cx.canvas.stroke_round_rect(sr, t.round(12), 2, t.accent);
        }
        let ink = if s.audible { t.text } else { t.text_faint };
        // Name over a colour stripe.
        let color = s.color.unwrap_or_else(|| strip_color(&t, s.id));
        cx.canvas.fill_round_rect(
            Rect::new(p.name.x, p.name.y, p.name.w, 4),
            2,
            if s.audible {
                color
            } else {
                color.mix(t.surface, 0.6)
            },
        );
        let mut label = p.name.take_top(6).1;
        if let Some((what, name)) = &s.tag {
            // What the track plays now (phase 4, board 6), under its name.
            let (top, tag) = label.take_top(24);
            label = top;
            let tag = Rect::new(tag.x, tag.y + 2, tag.w, tag.h - 4);
            cx.canvas.fill_round_rect(tag, t.round(6), t.raised);
            let line = Rect::new(tag.x + 6, tag.y + 1, tag.w - 10, 14);
            play_mark(cx, Rect::new(line.x, line.y + 3, 7, 8), color);
            cx.text(
                FontId::Bold,
                t.size_small - 3.0,
                Rect::new(line.x + 11, line.y, line.w - 11, line.h),
                Align::Left,
                t.readable(color, t.raised),
                what,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                Rect::new(line.x, line.y + 14, line.w, 14),
                Align::Left,
                t.text_dim,
                name,
            );
        }
        // An audio track's input at the right of the name, after a wave
        // mark (layout spec 3a): "∿ ST".
        if let Some(input) = &s.audio {
            let w = cx.measure(FontId::Sans, t.size_small - 2.0, input);
            let chip = Rect::new(label.right() - w, label.y, w, label.h);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                chip,
                Align::Right,
                t.text_dim,
                input,
            );
            let mark = Rect::new(chip.x - 18, label.y + label.h / 2 - 6, 14, 12);
            wave_mark(cx, mark, color);
            label.w = (mark.x - 4 - label.x).max(0);
        }
        cx.text(FontId::Bold, t.size_small, label, Align::Left, ink, &s.name);
        if master {
            self.paint_cost(cx, label, false);
        }
        if self.perform {
            self.paint_perform_strip(cx, s, &p);
            return;
        }
        // The chain's summary: a tap opens the Effects look.
        self.paint_fx_summary(cx, s, p.fx);
        self.paint_freeze(cx, s, p.fx);
        // The signal's: the sends, the pan and the output (a tap opens
        // the Sends look), or the master's safety.
        if master {
            self.paint_safety(cx, p.path);
        } else {
            self.paint_path(cx, s, p.path);
        }
        self.paint_fader(cx, s, p.fader, master);
        self.paint_level(cx, s, p.value, true);
        if master {
            // The master's limiting, where a strip's M goes, and where
            // it plays.
            let b = p.buttons[0];
            let red = self.state.safety.reduction_db;
            let tone = if red < -6.0 { t.warn } else { t.text_dim };
            cx.canvas.fill_round_rect(b, t.round(8), t.raised);
            cx.text(
                FontId::Sans,
                t.size_small,
                b.inset_xy(10, 0),
                Align::Left,
                t.text_dim,
                "Limit",
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                b.inset_xy(10, 0),
                Align::Right,
                tone,
                &format!("{red:.1} dB"),
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                p.buttons[1],
                Align::Center,
                t.text_faint,
                &format!("Pan {} · Out 1+2", pan_text(s.pan)),
            );
            return;
        }
        let arm = s.input.map(|i| i.arm);
        let spec = [
            ("M", s.mute, t.warn),
            ("S", s.solo, t.picked()),
            ("Arm", arm.unwrap_or(false), t.bad),
        ];
        for (b, (txt, on, c)) in p.buttons().iter().zip(spec) {
            self.paint_button(cx, *b, txt, on, c, t.size_body + 2.0);
        }
    }

    /// The Perform strip below its name: the pan row, the fader, the
    /// level (shown, not typed) and the two slots; the master's safety
    /// where they go.
    fn paint_perform_strip(&self, cx: &mut PaintCx, s: &StripView, p: &Parts) {
        let t = cx.theme.clone();
        let master = s.id == StripId::Master;
        let ink = if s.audible { t.text } else { t.text_faint };
        // The pan row: a track with the pan's bar and a mark, L and R at
        // its ends; it only drags.
        let r = p.pan;
        cx.canvas.stroke_round_rect(r, t.round(8), 1, t.outline);
        let track = r.inset_xy(14, 0);
        let mid = track.x + track.w / 2;
        let px = mid + ((track.w / 2) as f32 * (s.pan / 100.0)) as i32;
        let (a, b) = if px < mid { (px, mid) } else { (mid, px) };
        let bar_y = r.y + r.h / 2 - 2;
        cx.canvas
            .fill_rect(Rect::new(track.x, bar_y + 1, track.w, 2), t.raised);
        if b > a {
            cx.canvas.fill_rect(
                Rect::new(a, bar_y, b - a, 4),
                strip_color(&t, s.id).mix(t.surface, 0.6),
            );
        }
        cx.canvas
            .fill_round_rect(Rect::new(px - 2, r.y + 8, 4, r.h - 16), 2, ink);
        cx.text(
            FontId::Sans,
            t.size_small - 4.0,
            r.inset_xy(5, 2).take_top(12).0,
            Align::Left,
            t.text_faint,
            "L",
        );
        cx.text(
            FontId::Sans,
            t.size_small - 4.0,
            r.inset_xy(5, 2).take_top(12).0,
            Align::Right,
            t.text_faint,
            "R",
        );
        cx.text(
            FontId::Sans,
            t.size_small - 4.0,
            r.inset_xy(5, 2).take_bottom(12).0,
            Align::Right,
            t.text_dim,
            &pan_text(s.pan),
        );
        self.paint_fader(cx, s, p.fader, master);
        self.paint_level(cx, s, p.value, false);
        if master {
            let (safety, rest) = p.button_area.take_top(80);
            self.paint_safety(cx, safety);
            let red = self.state.safety.reduction_db;
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                rest.take_top(12).1.take_top(22).0.inset_xy(8, 0),
                Align::Left,
                if red < -6.0 { t.warn } else { t.text_dim },
                &format!("Limit {red:.1} dB"),
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                rest.take_bottom(22).0.inset_xy(8, 0),
                Align::Left,
                t.text_faint,
                "Out 1+2",
            );
            return;
        }
        let spec = [("MUTE", s.mute, t.warn), ("SOLO", s.solo, t.picked())];
        for (b, (txt, on, c)) in p.buttons().iter().zip(spec) {
            self.paint_button(cx, *b, txt, on, c, t.size_body + 2.0);
        }
    }

    /// A strip's button: lit in `c` while on.
    fn paint_button(&self, cx: &mut PaintCx, b: Rect, txt: &str, on: bool, c: Color, size: f32) {
        let t = cx.theme.clone();
        cx.canvas
            .fill_round_rect(b, t.round(10), if on { c } else { t.raised });
        cx.text(
            FontId::Bold,
            size,
            b,
            Align::Center,
            if on { t.ink_on(c) } else { t.text_dim },
            txt,
        );
    }

    /// The level under a fader; `typed`: with the dotted line every number
    /// that can be typed has (Make: a tap opens the number pad).
    fn paint_level(&self, cx: &mut PaintCx, s: &StripView, r: Rect, typed: bool) {
        let t = cx.theme.clone();
        let ink = if s.audible { t.text } else { t.text_faint };
        let level = db_text(s.volume_db);
        cx.text(FontId::Bold, t.size_small, r, Align::Center, ink, &level);
        if !typed {
            return;
        }
        let w = cx.measure(FontId::Bold, t.size_small, &level).min(r.w);
        let (x0, y) = (r.x + (r.w - w) / 2, r.bottom() - 3);
        let mut x = x0;
        while x + 4 <= x0 + w {
            cx.canvas.fill_rect(Rect::new(x, y, 4, 2), t.text_dim);
            x += 8;
        }
    }

    /// A Make strip's chain, read only: "FX 2" and the effects' names
    /// ("FX" and "none" with none; red while a sidechain has nothing
    /// picked), and an arrow: a tap opens the Effects look.
    fn paint_fx_summary(&self, cx: &mut PaintCx, s: &StripView, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(r, t.round(8), t.raised);
        let inner = r.inset_xy(8, 4);
        let (arrow, words) = inner.take_right(12);
        let (top, bottom) = words.take_top(words.h / 2);
        let title = if s.inserts.is_empty() {
            "FX".to_string()
        } else {
            format!("FX {}", s.inserts.len())
        };
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            top,
            Align::Left,
            t.text,
            &title,
        );
        let unpicked = s.inserts.iter().any(|i| i.nothing_picked);
        let names = if s.inserts.is_empty() {
            "none".to_string()
        } else {
            s.inserts
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        cx.text(
            FontId::Sans,
            t.size_small - 3.0,
            bottom,
            Align::Left,
            if unpicked {
                t.bad
            } else if s.inserts.is_empty() {
                t.text_faint
            } else {
                t.text_dim
            },
            &names,
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            arrow,
            Align::Right,
            t.text_dim,
            "›",
        );
    }

    /// A Make strip's signal, read only, in three lines: the sends that
    /// are on (the first two, "+1" for more; one before the fader in
    /// amber), the pan (an input: its source and monitor) and where it
    /// goes. A tap opens the Sends look.
    fn paint_path(&self, cx: &mut PaintCx, s: &StripView, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(r, t.round(8), t.raised);
        let inner = r.inset_xy(8, 3);
        let h = inner.h / 3;
        let line = |i: i32| Rect::new(inner.x, inner.y + i * h, inner.w, h);
        let size = t.size_small - 3.0;
        match s.sends {
            None => {
                cx.text(
                    FontId::Sans,
                    size,
                    line(0),
                    Align::Left,
                    t.text_faint,
                    "No sends",
                );
            }
            Some(sends) => {
                let on: Vec<usize> = (0..RETURNS).filter(|&k| sends[k].db > SILENT_DB).collect();
                let shown: Vec<usize> = if on.is_empty() {
                    vec![0, 1]
                } else {
                    on.iter().copied().take(2).collect()
                };
                let mut x = line(0).x;
                for k in shown {
                    let letter = ["A", "B", "C", "D"][k];
                    let (words, ink) = if sends[k].db <= SILENT_DB {
                        (format!("{letter} off"), t.text_faint)
                    } else if sends[k].pre {
                        (format!("{letter} {:.0}", sends[k].db), t.warn)
                    } else {
                        (format!("{letter} {:.0}", sends[k].db), t.text)
                    };
                    let w = cx.measure(FontId::Sans, size, &words);
                    let l0 = line(0);
                    cx.text(
                        FontId::Sans,
                        size,
                        Rect::new(x, l0.y, (l0.right() - x).max(0), l0.h),
                        Align::Left,
                        ink,
                        &words,
                    );
                    x += w + 8;
                }
                if on.len() > 2 {
                    cx.text(
                        FontId::Sans,
                        size,
                        line(0),
                        Align::Right,
                        t.text_dim,
                        &format!("+{}", on.len() - 2),
                    );
                }
            }
        }
        let second = match s.input {
            Some(inp) => format!(
                "{} · {}",
                choice(mx::SOURCE, inp.source),
                choice(mx::MONITOR, inp.monitor)
            ),
            None => format!("Pan {}", pan_text(s.pan)),
        };
        cx.text(
            FontId::Sans,
            size,
            line(1),
            Align::Left,
            t.text_dim,
            &second,
        );
        let to = if s.has_output() {
            s.output_label()
        } else {
            "Master".to_string()
        };
        let none = s.has_output() && s.output == output::NONE as u8;
        cx.text(
            FontId::Sans,
            size,
            line(2),
            Align::Left,
            if none { t.warn } else { t.text_dim },
            &format!("→ {to}"),
        );
    }

    /// The Effects look: each strip's chain as a row of slots, in order
    /// left to right, with the master's row pinned under them.
    fn paint_effects(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        let g = grid(l);
        cx.canvas.fill_round_rect(g.panel, 14, t.surface);
        let rows = self.rows(l);
        let title = match (rows.first(), rows.last()) {
            (Some((a, _)), Some((b, _))) => format!(
                "{} {}-{} · EACH CHAIN IN ORDER, LEFT TO RIGHT",
                self.state.view.label().to_uppercase(),
                a + 1,
                b + 1
            ),
            _ => self.state.view.label().to_uppercase(),
        };
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            g.title,
            Align::Left,
            t.text_dim,
            &title,
        );
        let picked = self.pick.and_then(|(id, slot)| {
            let s = self.state.strip(id)?;
            Some(format!(
                "{} · {} picked: its knobs are on the Q-Links",
                s.name,
                s.inserts.get(slot)?.name
            ))
        });
        match picked {
            Some(p) => {
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    g.title,
                    Align::Right,
                    t.text_dim,
                    &p,
                );
            }
            None => self.paint_cost(cx, g.title, true),
        }
        let (name, cells) = row_cells(g.numbers);
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            name,
            Align::Left,
            t.text_faint,
            "Strip",
        );
        for (k, c) in cells.iter().enumerate() {
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                *c,
                Align::Center,
                t.text_faint,
                &(k + 1).to_string(),
            );
        }
        for (i, row) in rows {
            self.paint_row(cx, &self.state.strips[i], row);
        }
        let line = Rect::new(g.left, g.master.y - 6, g.width, 1);
        cx.canvas.fill_rect(line, t.raised);
        self.paint_row(cx, &self.state.master, g.master);
    }

    /// The Sends look: each strip's pan, sends and output as a row, with
    /// the master's row pinned under them.
    fn paint_sends(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        let g = grid(l);
        cx.canvas.fill_round_rect(g.panel, 14, t.surface);
        let rows = self.rows(l);
        let title = match (rows.first(), rows.last()) {
            (Some((a, _)), Some((b, _))) => format!(
                "{} {}-{} · PAN, SENDS AND WHERE EACH GOES",
                self.state.view.label().to_uppercase(),
                a + 1,
                b + 1
            ),
            _ => self.state.view.label().to_uppercase(),
        };
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            g.title,
            Align::Left,
            t.text_dim,
            &title,
        );
        let head = crate::sends_look::row_parts(g.numbers, FX_NAME_W);
        let small = t.size_small - 2.0;
        cx.text(
            FontId::Sans,
            small,
            head.name,
            Align::Left,
            t.text_faint,
            "Strip",
        );
        cx.text(
            FontId::Sans,
            small,
            head.pan,
            Align::Center,
            t.text_faint,
            "Pan",
        );
        for k in 0..RETURNS {
            let both = Rect::new(
                head.pre[k].x,
                head.pre[k].y,
                head.send[k].right() - head.pre[k].x,
                head.pre[k].h,
            );
            cx.text(
                FontId::Sans,
                small,
                both,
                Align::Center,
                t.text_faint,
                &format!("Send {}", ["A", "B", "C", "D"][k]),
            );
        }
        cx.text(
            FontId::Sans,
            small,
            head.output,
            Align::Center,
            t.text_faint,
            "Output",
        );
        for (i, row) in rows {
            self.paint_sends_row(cx, &self.state.strips[i], row);
        }
        let line = Rect::new(g.left, g.master.y - 6, g.width, 1);
        cx.canvas.fill_rect(line, t.raised);
        self.paint_sends_row(cx, &self.state.master, g.master);
    }

    /// One strip's row on the Sends look.
    fn paint_sends_row(&self, cx: &mut PaintCx, s: &StripView, row: Rect) {
        let t = cx.theme.clone();
        let r = crate::sends_look::row_parts(row, FX_NAME_W);
        self.paint_row_name(cx, s, r.name);
        let ink = if s.audible { t.text } else { t.text_faint };
        let color = s.color.unwrap_or_else(|| strip_color(&t, s.id));
        // The pan: its bar from the middle and a mark.
        let p = r.pan;
        cx.canvas.fill_round_rect(p, t.round(8), t.raised);
        let track = p.inset_xy(12, 0);
        let mid = track.x + track.w / 2;
        let px = mid + ((track.w / 2) as f32 * (s.pan / 100.0)) as i32;
        let (a, b) = if px < mid { (px, mid) } else { (mid, px) };
        if b > a {
            cx.canvas.fill_rect(
                Rect::new(a, p.y + p.h / 2 - 2, b - a, 4),
                color.mix(t.raised, 0.6),
            );
        }
        cx.canvas
            .fill_rect(Rect::new(mid, p.y + 8, 1, p.h - 16), t.outline);
        cx.canvas
            .fill_round_rect(Rect::new(px - 2, p.y + 7, 4, p.h - 14), 2, ink);
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            p.inset_xy(8, 0),
            Align::Left,
            t.text_dim,
            &pan_text(s.pan),
        );
        // The sends: pre, and the level as a bar from the left. What is off
        // (pre unlit, a send at off) is faded, never under 3:1 (`faded`).
        if let Some(sends) = s.sends {
            for k in 0..RETURNS {
                let pre = sends[k].pre;
                cx.canvas.fill_round_rect(
                    r.pre[k],
                    t.round(8),
                    if pre { t.warn } else { t.raised },
                );
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    r.pre[k],
                    Align::Center,
                    if pre {
                        t.ink_on(t.warn)
                    } else {
                        t.faded(t.text, t.raised)
                    },
                    "pre",
                );
                let bar = r.send[k];
                cx.canvas.fill_round_rect(bar, t.round(8), t.raised);
                let on = sends[k].db > SILENT_DB;
                if on {
                    let w = ((bar.w as f32) * db_to_pos(sends[k].db)) as i32;
                    let fill = if s.audible { t.accent } else { t.text_faint };
                    cx.canvas.fill_round_rect(
                        Rect::new(bar.x, bar.y, w.max(3), bar.h),
                        t.round(8),
                        fill.mix(t.raised, 0.4),
                    );
                    cx.canvas
                        .fill_rect(Rect::new(bar.x + w.max(3) - 3, bar.y, 3, bar.h), fill);
                }
                let v = if on {
                    format!("{:.0} dB", sends[k].db)
                } else {
                    "off".to_string()
                };
                cx.text(
                    if on { FontId::Bold } else { FontId::Sans },
                    t.size_small - 1.0,
                    bar.inset_xy(8, 0),
                    Align::Right,
                    if on { ink } else { t.faded(t.text, t.raised) },
                    &v,
                );
            }
        } else if s.id != StripId::Master {
            let all = Rect::new(
                r.pre[0].x,
                r.pre[0].y,
                r.send[RETURNS - 1].right() - r.pre[0].x,
                r.pre[0].h,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                all,
                Align::Center,
                t.text_faint,
                "No sends",
            );
        }
        // The output.
        if s.has_output() {
            let open = self.picker == Some(s.id);
            cx.canvas.fill_round_rect(
                r.output,
                t.round(8),
                if open { t.picked() } else { t.raised },
            );
            let none = s.output == output::NONE as u8;
            let c = if open {
                t.on_picked()
            } else if none {
                t.warn
            } else {
                t.text
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                r.output,
                Align::Center,
                c,
                &s.output_label(),
            );
        } else {
            let to = if s.id == StripId::Master {
                "Out 1+2"
            } else {
                "Master"
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                r.output,
                Align::Center,
                t.text_faint,
                to,
            );
        }
    }

    /// One strip's row on the Effects look.
    fn paint_row(&self, cx: &mut PaintCx, s: &StripView, row: Rect) {
        let t = cx.theme.clone();
        let (name, cells) = row_cells(row);
        self.paint_row_name(cx, s, name);
        let slots = s.slots.min(FX_COLS);
        let drop = self.grabs.iter().find_map(|(_, g)| match *g {
            Grab::Fx {
                id,
                from,
                to: Some(to),
                ..
            } if id == s.id => Some((from, to)),
            _ => None,
        });
        for (k, c) in cells.into_iter().enumerate() {
            if k >= slots {
                break;
            }
            if let Some(ins) = s.inserts.get(k) {
                cx.canvas.fill_round_rect(
                    c,
                    t.round(10),
                    if ins.on {
                        t.raised
                    } else {
                        t.surface.mix(t.raised, 0.4)
                    },
                );
                let dot = Rect::new(c.right() - 16, c.y + 9, 8, 8);
                let tone = if !ins.on {
                    t.text_faint
                } else if ins.nothing_picked {
                    t.bad
                } else if ins.key.is_some() {
                    t.accent
                } else {
                    t.good
                };
                cx.canvas.fill_round_rect(dot, 4, tone);
                if let Some(cost) = ins.cost_pct {
                    let w = ((c.w as f32) * (cost / 10.0).clamp(0.0, 1.0)) as i32;
                    cx.canvas.fill_round_rect(
                        Rect::new(c.x, c.bottom() - 3, w.max(2), 3),
                        1,
                        if cost <= 2.0 {
                            t.good
                        } else if cost <= 5.0 {
                            t.warn
                        } else {
                            t.bad
                        },
                    );
                }
                let inner = Rect::new(c.x + 8, c.y + 6, c.w - 28, c.h - 12);
                let second = match (&ins.key, ins.on) {
                    (_, false) => Some("Off".to_string()),
                    (Some(k), true) => Some(format!("SC from {k}")),
                    (None, true) if ins.nothing_picked => Some("Nothing picked".to_string()),
                    (None, true) => None,
                };
                let warn = ins.on && ins.nothing_picked;
                let (a, b) = match second {
                    Some(_) => inner.take_top(inner.h / 2 + 2),
                    None => (inner, Rect::default()),
                };
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    a,
                    Align::Left,
                    t.button_ink(ins.on),
                    &ins.name,
                );
                if let Some(line) = second {
                    cx.text(
                        if warn { FontId::Bold } else { FontId::Sans },
                        t.size_small - 4.0,
                        Rect::new(b.x, b.y, c.w - 16, b.h),
                        Align::Left,
                        if warn { t.bad } else { t.text_dim },
                        &line,
                    );
                }
                if self.pick == Some((s.id, k)) && drop.is_none() {
                    cx.canvas.stroke_round_rect(c, t.round(10), 2, t.accent);
                }
                match drop {
                    Some((from, _)) if from == k => {
                        cx.canvas.stroke_round_rect(c, t.round(10), 1, t.text_dim);
                    }
                    Some((_, to)) if to == k => {
                        cx.canvas.stroke_round_rect(c, t.round(10), 2, t.accent);
                    }
                    _ => {}
                }
            } else if k == s.inserts.len() {
                cx.canvas.fill_round_rect(c, t.round(10), t.raised);
                cx.text(FontId::Bold, t.size_small, c, Align::Center, t.text, "+ FX");
            } else {
                cx.canvas
                    .stroke_round_rect(c, t.round(10), 1, t.outline.mix(t.surface, 0.5));
            }
        }
    }

    /// The EQ look: the panel, its title line, the frequencies over the
    /// curves, a row a strip and the master's row.
    fn paint_eq(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        let g = grid(l);
        cx.canvas.fill_round_rect(g.panel, 14, t.surface);
        let rows = self.rows(l);
        let title = match (rows.first(), rows.last()) {
            (Some((a, _)), Some((b, _))) => format!(
                "{} {}-{} · EACH STRIP'S EQ · TAP A CURVE TO SHAPE IT",
                self.state.view.label().to_uppercase(),
                a + 1,
                b + 1
            ),
            _ => self.state.view.label().to_uppercase(),
        };
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            g.title,
            Align::Left,
            t.text_dim,
            &title,
        );
        let picked = self.pick.and_then(|(id, _)| {
            let s = self.state.strip(id)?;
            Some(match self.eq_dot() {
                Some(d) => format!("{} · EQ · {} picked", s.name.to_uppercase(), d.name()),
                None => format!("{} · EQ picked", s.name.to_uppercase()),
            })
        });
        match picked {
            Some(p) => {
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    g.title,
                    Align::Right,
                    t.text_dim,
                    &p,
                );
            }
            None => self.paint_cost(cx, g.title, true),
        }
        let (name, graph, summary) = eq_look::row_parts(g.numbers, FX_NAME_W);
        let faint = |cx: &mut PaintCx, r: Rect, a: Align, s: &str| {
            cx.text(FontId::Sans, t.size_small - 2.0, r, a, t.text_faint, s);
        };
        faint(cx, name, Align::Left, "Strip");
        for (hz, label) in [
            (20.0, "20 Hz"),
            (100.0, "100"),
            (1_000.0, "1k"),
            (10_000.0, "10k"),
            (20_000.0, "20 kHz"),
        ] {
            let x = graph.x + (eq_look::freq_x(hz) * graph.w as f32) as i32;
            let (r, a) = match label {
                "20 Hz" => (Rect::new(x, graph.y, 60, graph.h), Align::Left),
                "20 kHz" => (Rect::new(x - 60, graph.y, 60, graph.h), Align::Right),
                _ => (Rect::new(x - 30, graph.y, 60, graph.h), Align::Center),
            };
            faint(cx, r, a, label);
        }
        faint(cx, summary, Align::Left, "Where it sits");
        for (i, row) in rows {
            self.paint_eq_row(cx, &self.state.strips[i], row);
        }
        let line = Rect::new(g.left, g.master.y - 6, g.width, 1);
        cx.canvas.fill_rect(line, t.raised);
        self.paint_eq_row(cx, &self.state.master, g.master);
    }

    /// One strip's row on the EQ look: its name, its EQ's curve and dots
    /// (or "+ EQ"), and where the EQ sits.
    fn paint_eq_row(&self, cx: &mut PaintCx, s: &StripView, row: Rect) {
        let t = cx.theme.clone();
        let (name, graph, summary) = eq_look::row_parts(row, FX_NAME_W);
        self.paint_row_name(cx, s, name);
        let Some(slot) = s.eq_slot() else {
            cx.canvas
                .stroke_round_rect(graph, t.round(8), 1, t.outline.mix(t.surface, 0.3));
            let full = s.inserts.len() >= s.slots;
            let note = if full {
                "the chain is full".to_string()
            } else if s.inserts.is_empty() {
                "it goes in slot 1".to_string()
            } else {
                format!(
                    "it goes in slot {}, the first free slot",
                    s.inserts.len() + 1
                )
            };
            let bw = 76;
            let nw = cx.measure(FontId::Sans, t.size_small - 3.0, &note);
            let x0 = graph.x + (graph.w - bw - 10 - nw) / 2;
            if !full {
                let b = Rect::new(x0, graph.y + (graph.h - 32) / 2, bw, 32);
                cx.canvas.fill_round_rect(b, t.round(8), t.raised);
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    b,
                    Align::Center,
                    t.text,
                    "+ EQ",
                );
            }
            let nr = Rect::new(x0 + bw + 10, graph.y, nw + 4, graph.h);
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                nr,
                Align::Left,
                t.text_dim,
                &note,
            );
            return;
        };
        let ins = &s.inserts[slot];
        let vals = ins.eq.clone().unwrap_or_default();
        let picked = self.pick == Some((s.id, slot));
        let on = ins.on && s.audible;
        cx.canvas
            .fill_round_rect(graph, t.round(8), t.bg.mix(t.surface, 0.4));
        let p = eq_look::plot(graph);
        let mid = eq_look::db_y(p, 0.0);
        cx.canvas
            .fill_rect(Rect::new(p.x, mid as i32, p.w, 1), t.outline);
        for hz in [100.0, 1_000.0, 10_000.0] {
            let x = p.x + (eq_look::freq_x(hz) * p.w as f32) as i32;
            cx.canvas
                .fill_rect(Rect::new(x, graph.y, 1, graph.h), t.outline.mix(t.bg, 0.6));
        }
        let color = if on {
            strip_color(&t, s.id)
        } else {
            t.text_faint
        };
        let curve = self.eq_curve(&vals);
        let pts: Vec<(f32, f32)> = curve
            .iter()
            .enumerate()
            .map(|(k, &d)| {
                (
                    p.x as f32 + k as f32 / (curve.len() - 1) as f32 * p.w as f32,
                    eq_look::db_y(p, d),
                )
            })
            .collect();
        let clip = cx.canvas.clip();
        cx.canvas.set_clip(clip.intersect(graph));
        let mut fill = mpfree_ui::Shape::new(graph);
        fill.fill_under(&pts, mid, if picked { 0.26 } else { 0.16 });
        fill.draw(cx.canvas, color);
        let mut line = mpfree_ui::Shape::new(graph);
        line.polyline(&pts, 2.0);
        line.draw(cx.canvas, color);
        for d in eq_look::dots(&vals) {
            let (x, y) = eq_look::dot_at(&vals, &curve, p, d);
            let sel = picked && self.eq_dot == Some(d);
            let rad = if sel { 7.0 } else { 5.0 };
            if sel {
                let mut ring = mpfree_ui::Shape::new(graph);
                ring.disc(x, y, rad + 2.0);
                ring.draw(cx.canvas, t.text);
            }
            let mut m = mpfree_ui::Shape::new(graph);
            m.disc(x, y, rad);
            let ink = if on && eq_look::heard(&vals, d) {
                d.color(&t)
            } else {
                t.text_faint
            };
            m.draw(cx.canvas, ink);
        }
        cx.canvas.set_clip(clip);
        if picked {
            cx.canvas.stroke_round_rect(graph, t.round(8), 2, t.accent);
        }
        let (a, b) = eq_look::summary(&vals, slot, ins.on);
        let (top, bottom) = summary.inset_xy(0, 6).take_top(20);
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            top,
            Align::Left,
            if on { t.text } else { t.text_dim },
            &a,
        );
        cx.text(
            FontId::Sans,
            t.size_small - 3.0,
            bottom,
            Align::Left,
            t.text_dim,
            &b,
        );
    }

    /// A row's name cell on Effects and EQ: the strip's colour, its name
    /// and the line under it.
    fn paint_row_name(&self, cx: &mut PaintCx, s: &StripView, name: Rect) {
        let t = cx.theme.clone();
        if self.state.selected == Some(s.id) {
            cx.canvas.fill_round_rect(name, t.round(10), t.raised);
            cx.canvas.stroke_round_rect(name, t.round(10), 2, t.accent);
        }
        let color = strip_color(&t, s.id);
        let stripe = Rect::new(name.x + 6, name.y + (name.h - 36) / 2, 4, 36);
        cx.canvas.fill_round_rect(
            stripe,
            2,
            if s.audible {
                color
            } else {
                color.mix(t.surface, 0.6)
            },
        );
        let text = Rect::new(
            stripe.right() + 10,
            name.y + 6,
            name.right() - stripe.right() - 16,
            name.h - 12,
        );
        let (top, bottom) = text.take_top(text.h / 2 + 2);
        cx.text(
            FontId::Bold,
            t.size_body - 2.0,
            top,
            Align::Left,
            if s.audible { t.text } else { t.text_dim },
            &s.name,
        );
        let sub = match s.take {
            Some(TakeView { print, done }) => {
                let what = if print { "printing" } else { "freezing" };
                match done {
                    Some(d) => format!("{what} {:.0} %", (d * 100.0).clamp(0.0, 100.0)),
                    None => format!("{what} next loop"),
                }
            }
            None => row_sub(s),
        };
        cx.text(
            FontId::Sans,
            t.size_small - 3.0,
            bottom,
            Align::Left,
            if s.take.is_some() || s.frozen {
                t.accent
            } else {
                t.text_dim
            },
            &sub,
        );
    }

    fn paint_safety(&self, cx: &mut PaintCx, area: Rect) {
        let t = cx.theme.clone();
        let sf = &self.state.safety;
        cx.canvas
            .fill_round_rect(area, t.round(8), t.raised.mix(t.surface, 0.5));
        // Four rows, or three in the Make strip's 64 px (the mode as the
        // title).
        let rows = area.inset_xy(8, 4);
        let short = area.h < 76;
        let n = if short { 3 } else { 4 };
        let h = rows.h / n;
        let row = |i: i32| Rect::new(rows.x, rows.y + i * h, rows.w, h);
        if short {
            cx.text(
                FontId::Bold,
                t.size_small - 3.0,
                row(0),
                Align::Left,
                t.text_dim,
                &sf.mode.to_uppercase(),
            );
        } else {
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                row(0),
                Align::Left,
                t.text_dim,
                "MASTER SAFETY",
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                row(1),
                Align::Left,
                t.text,
                &sf.mode,
            );
        }
        let size = t.size_small - if short { 3.0 } else { 1.0 };
        cx.text(
            FontId::Sans,
            size,
            row(n - 2),
            Align::Left,
            t.text_dim,
            &format!("Ceiling {:.1} dB", sf.ceiling_db),
        );
        let tone = if sf.overs > 0 { t.warn } else { t.text_dim };
        cx.text(
            FontId::Sans,
            size,
            row(n - 1),
            Align::Left,
            tone,
            &format!("Caught {}", sf.overs),
        );
    }

    fn paint_fader(&self, cx: &mut PaintCx, s: &StripView, f: Rect, master: bool) {
        let t = cx.theme.clone();
        let (scale, track_area, meter) = fader_columns(f, master);
        for (db, label) in MARKS {
            let y = pos_y(f, db_to_pos(db));
            let c = if db == 0.0 { t.text_dim } else { t.text_faint };
            cx.text(
                FontId::Sans,
                t.size_small - 4.0,
                Rect::new(scale.x, y - 6, scale.w - 4, 12),
                Align::Right,
                c,
                label,
            );
            cx.canvas
                .fill_rect(Rect::new(scale.right() - 2, y, 4, 1), t.outline);
        }
        let (top, bot) = travel(f);
        let tx = track_area.x + track_area.w / 2;
        cx.canvas
            .fill_round_rect(Rect::new(tx - 3, top, 6, bot - top), 3, t.raised);
        let pos = db_to_pos(s.volume_db);
        let cy = pos_y(f, pos);
        let color = if s.audible {
            strip_color(&t, s.id)
        } else {
            t.text_faint
        };
        cx.canvas.fill_round_rect(
            Rect::new(tx - 3, cy, 6, bot - cy),
            3,
            color.mix(t.raised, 0.35),
        );
        let capw = (track_area.w - 6).min(40);
        let cap = Rect::new(tx - capw / 2, cy - 10, capw, 20);
        cx.canvas
            .fill_round_rect(cap, 5, if s.audible { t.text } else { t.text_dim });
        cx.canvas
            .fill_rect(Rect::new(cap.x + 5, cy, cap.w - 10, 2), t.surface);
        // The meter: one bar per side on the fader's scale, red above 0 dBFS.
        let mw = if master { 9 } else { 5 };
        let gap = 2;
        let mx0 = meter.right() - 2 * mw - gap - 2;
        let zero = pos_y(f, db_to_pos(0.0));
        for side in 0..2 {
            let bar = Rect::new(mx0 + side * (mw + gap), top, mw, bot - top);
            cx.canvas.fill_rect(bar, t.bg.mix(t.surface, 0.3));
            let db = peak_db(s.peak[side as usize]);
            if db > -80.0 {
                let y = pos_y(f, db_to_pos(db.min(6.0)));
                let lit = Rect::new(bar.x, y, bar.w, bot - y);
                let warn_y = pos_y(f, db_to_pos(-6.0));
                let (c_low, c_mid) = if s.audible {
                    (t.good, t.warn)
                } else {
                    (t.text_faint, t.text_faint)
                };
                cx.canvas.fill_rect(
                    lit.intersect(Rect::new(bar.x, warn_y, bar.w, bot - warn_y)),
                    c_low,
                );
                cx.canvas.fill_rect(
                    lit.intersect(Rect::new(bar.x, zero, bar.w, warn_y - zero)),
                    c_mid,
                );
                cx.canvas.fill_rect(
                    lit.intersect(Rect::new(bar.x, top, bar.w, zero - top)),
                    t.bad,
                );
            }
        }
        if master {
            // Master safety's gain reduction, down from the top.
            let red = (-self.state.safety.reduction_db).clamp(0.0, 12.0);
            let gr = Rect::new(
                meter.x + 2,
                top,
                4,
                ((bot - top) as f32 * red / 12.0) as i32,
            );
            if gr.h > 0 {
                cx.canvas.fill_rect(gr, t.warn);
            }
        }
    }

    fn paint_picker(&self, cx: &mut PaintCx, l: &Layout, s: StripId) {
        let t = cx.theme.clone();
        let pr = MixerView::picker_rect(l);
        cx.canvas.fill_round_rect(pr.inset(-2), 16, t.outline);
        cx.canvas.fill_round_rect(pr, 14, t.surface);
        let name = self
            .state
            .strip(s)
            .map_or(String::new(), |v| v.name.clone());
        let head = pr.inset(18).take_top(30).0;
        cx.text(
            FontId::Bold,
            t.size_title,
            head,
            Align::Left,
            t.text,
            &format!("Output of {name}"),
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            head,
            Align::Right,
            t.text_dim,
            "Tap outside to close",
        );
        let cur = self.state.strip(s).map(|v| {
            (
                v.output,
                if v.output == output::TRACK as u8 {
                    v.output_track
                } else {
                    0
                },
            )
        });
        for ((out, track), b) in self.picker_buttons(l, s) {
            let on = cur == Some((out, track));
            cx.canvas
                .fill_round_rect(b, t.round(10), if on { t.picked() } else { t.raised });
            cx.text(
                FontId::Sans,
                t.size_body,
                b,
                Align::Center,
                if on { t.on_picked() } else { t.text },
                &output_label(out, track),
            );
        }
    }

    fn paint_menu(&self, cx: &mut PaintCx, l: &Layout, s: StripId) {
        let t = cx.theme.clone();
        let Some(v) = self.state.strip(s) else {
            return;
        };
        let mr = self.menu_rect(l, s);
        cx.canvas.fill_round_rect(mr.inset(-2), 16, t.outline);
        cx.canvas.fill_round_rect(mr, 14, t.surface);
        let head = mr.inset(18).take_top(30).0;
        cx.text(
            FontId::Bold,
            t.size_title,
            head,
            Align::Left,
            t.text,
            &v.name,
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            head,
            Align::Right,
            t.text_dim,
            "Tap outside to close",
        );
        for (m, b) in self.menu_buttons(l, s) {
            let (label, line) = m.text(v);
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            let inner = b.inset_xy(16, 7);
            let (top, bottom) = inner.take_top(24);
            cx.text(FontId::Bold, t.size_body, top, Align::Left, t.text, label);
            cx.text(
                FontId::Sans,
                t.size_small,
                bottom,
                Align::Left,
                t.text_dim,
                line,
            );
        }
    }

    /// Over a strip's insert rows: "Frozen", or a take under way.
    fn paint_freeze(&self, cx: &mut PaintCx, s: &StripView, area: Rect) {
        let t = cx.theme.clone();
        if s.take.is_none() && !s.frozen {
            return;
        }
        cx.canvas
            .fill_round_rect(area, t.round(8), t.surface.mix(t.accent, 0.12));
        cx.canvas.stroke_round_rect(area, t.round(8), 1, t.accent);
        if area.h < 60 {
            // The Make strip's chain summary (44 px): what it is, how far
            // (or that it waits for the loop), and the bar.
            let inner = area.inset_xy(8, 4);
            let (top, rest) = inner.take_top(17);
            let (words, line, done) = match s.take {
                Some(TakeView { print, done }) => (
                    if print { "Print" } else { "Freeze" },
                    match done {
                        None => "next loop".to_string(),
                        Some(d) => format!("{:.0} %", (d * 100.0).clamp(0.0, 100.0)),
                    },
                    Some(done),
                ),
                None => ("Frozen", format!("FX {}", s.inserts.len()), None),
            };
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                top,
                Align::Left,
                t.accent,
                words,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                rest.take_top(15).0,
                Align::Left,
                t.text_dim,
                &line,
            );
            if let Some(done) = done {
                let bar = Rect::new(inner.x, inner.bottom() - 3, inner.w, 3);
                cx.canvas.fill_round_rect(bar, 1, t.raised);
                if let Some(d) = done {
                    let w = ((bar.w as f32) * d.clamp(0.0, 1.0)) as i32;
                    if w > 0 {
                        cx.canvas
                            .fill_round_rect(Rect::new(bar.x, bar.y, w, bar.h), 1, t.accent);
                    }
                }
            }
            return;
        }
        let inner = area.inset_xy(8, 8);
        let (title, rest) = inner.take_top(24);
        match s.take {
            Some(TakeView { print, done }) => {
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    title,
                    Align::Left,
                    t.accent,
                    if print { "Printing" } else { "Freezing" },
                );
                let line = match done {
                    None => "Next loop".to_string(),
                    Some(d) => format!("{:.0} %", (d * 100.0).clamp(0.0, 100.0)),
                };
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    rest.take_top(22).0,
                    Align::Left,
                    t.text_dim,
                    &line,
                );
                let bar = Rect::new(inner.x, inner.bottom() - 6, inner.w, 6);
                cx.canvas.fill_round_rect(bar, 3, t.raised);
                if let Some(d) = done {
                    let w = ((bar.w as f32) * d.clamp(0.0, 1.0)) as i32;
                    if w > 0 {
                        cx.canvas
                            .fill_round_rect(Rect::new(bar.x, bar.y, w, bar.h), 3, t.accent);
                    }
                }
            }
            None => {
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    title,
                    Align::Left,
                    t.accent,
                    "Frozen",
                );
                let n = s.inserts.len();
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    rest.take_top(22).0,
                    Align::Left,
                    t.text_dim,
                    &format!("{n} effect{}", if n == 1 { "" } else { "s" }),
                );
            }
        }
    }

    /// The rectangles the meters (and, while profiling, the costs) fill.
    fn meter_parts(&self, r: Rect) -> Vec<Rect> {
        let l = layout(r);
        let profiling = self.state.cost_pct.is_some();
        let mut v: Vec<Rect> = Vec::new();
        let perform = self.perform;
        let mut add = |sr: Rect, master: bool, s: &StripView| {
            let p = parts_of(sr, s, perform);
            v.push(fader_columns(p.fader, master).2);
            if s.take.is_some() && !perform {
                // A take's progress.
                v.push(p.fx);
            }
            if master {
                // Its safety and its limiting.
                v.push(p.path);
                v.push(p.button_area);
            }
            if profiling {
                v.push(p.fx);
            }
        };
        if self.look == Look::Sends && self.state.view != MixView::Io {
            // No meters here, nor costs or takes.
        } else if self.look == Look::Effects && self.state.view != MixView::Io {
            // No meters here: a take's progress, and the costs while
            // profiling.
            let g = grid(&l);
            let mut rows: Vec<(&StripView, Rect)> = self
                .rows(&l)
                .into_iter()
                .map(|(i, row)| (&self.state.strips[i], row))
                .collect();
            rows.push((&self.state.master, g.master));
            for (s, row) in rows {
                if profiling {
                    v.push(row);
                } else if s.take.is_some() {
                    v.push(row_cells(row).0);
                }
            }
            if profiling {
                v.push(g.title);
            }
        } else {
            for (i, sr) in self.shown(&l) {
                add(sr, false, &self.state.strips[i]);
            }
            add(l.master, true, &self.state.master);
            if profiling {
                v.push(parts_of(l.master, &self.state.master, perform).name);
            }
        }
        if self.picker.is_some() {
            // The picker covers the strips: draw it again over them.
            v.push(MixerView::picker_rect(&l).inset(-2));
        }
        if let Some(s) = self.menu {
            v.push(self.menu_rect(&l, s).inset(-2));
        }
        v
    }
}

impl MixerView {
    /// The wheel on a strip's fader, pan or send by `delta` detents,
    /// `fine` with SHIFT ([`Widget::step`], [`Widget::held_step`]).
    fn turn(&mut self, t: &Target, delta: i32, fine: bool, r: Rect) -> Option<EventKind> {
        let (strip, j) = self.strip_of(t, r)?;
        let s = self.state.strip(strip)?;
        let (id, from, to) = match t.id {
            "fader" => (
                mx::VOLUME,
                s.volume_db,
                crate::qlinks::level_turn(s.volume_db, delta, fine),
            ),
            "pan" => (mx::PAN, s.pan, crate::qlinks::pan_turn(s.pan, delta, fine)),
            "send" => {
                let db = s.sends?.get(j)?.db;
                (mx::send(j), db, crate::qlinks::level_turn(db, delta, fine))
            }
            _ => return None,
        };
        if to == from {
            return None;
        }
        self.set(strip, id, to);
        self.mark(Dirty::All);
        Some(EventKind::Select(self.actions.len()))
    }

    // ------------------------------------------------------------ I/O view

    /// Adds a section to the I/O view's list, after Audio and the sections
    /// added before it.
    pub fn add_io_section(&mut self, s: Box<dyn IoSection>) {
        self.io.sections.push(s);
        self.mark(Dirty::All);
    }

    /// The I/O view's section showing: 0 is Audio, then the added sections
    /// in order.
    pub fn io_section(&self) -> usize {
        self.io.sel
    }

    /// Shows section `i` of the I/O view (0 is Audio); false if there is
    /// none.
    pub fn show_io_section(&mut self, i: usize) -> bool {
        if i > self.io.sections.len() {
            return false;
        }
        if i != self.io.sel {
            if let Some(s) = self.io.section_mut() {
                s.back();
            }
            self.io.sel = i;
            self.io.fingers.clear();
        }
        self.sync_shown();
        self.mark(Dirty::All);
        true
    }

    /// Outside sections (by index in add order) that replied
    /// [`IoReply::Redraw`] to a touch, a key ([`MixerView::io_input`]) or a
    /// Q-Link ([`MixerView::io_qlink_turn`]) since the last call: read
    /// their state. One place, whatever moved the setting.
    pub fn take_io_changes(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.io_changed)
    }

    /// What a section's last touch, key or Q-Link couldn't do, and why
    /// ([`IoSection::take_note`]), once.
    pub fn take_io_note(&mut self) -> Option<String> {
        self.grab_io_note();
        self.io_note.take()
    }

    /// Keeps a section's note until the app takes it.
    fn grab_io_note(&mut self) {
        if let Some(n) = self.io.sections.iter_mut().find_map(|s| s.take_note()) {
            self.io_note = Some(n);
        }
    }

    /// The data wheel, - and + or the page arrows while the I/O view shows.
    /// On Audio the wheel moves the selected row and - and + step its
    /// output (both queue actions); an outside section gets them itself.
    pub fn io_input(&mut self, ev: IoInput) -> IoReply {
        if self.state.view != MixView::Io {
            return IoReply::Ignored;
        }
        let outside = self.io.sel.checked_sub(1);
        let r = if let Some(s) = self.io.section_mut() {
            let r = s.input(ev);
            if r == IoReply::Redraw {
                self.io_changed.extend(outside);
            }
            r
        } else {
            match ev {
                IoInput::Wheel(d) => self.io_wheel(d),
                IoInput::Step(d) => self.io_step(d),
                IoInput::Page(d) => {
                    self.io_page(d);
                    IoReply::Redraw
                }
            }
        };
        if r == IoReply::Redraw {
            self.mark(Dirty::All);
        }
        r
    }

    /// Each frame: the outside section showing updates its live values.
    pub fn io_tick(&mut self, now: std::time::Duration) -> IoReply {
        if self.state.view != MixView::Io {
            return IoReply::Ignored;
        }
        let r = self
            .io
            .section_mut()
            .map_or(IoReply::Ignored, |s| s.tick(now));
        if r == IoReply::Redraw {
            self.mark(Dirty::Panel);
        }
        r
    }

    /// The app's Back while the I/O view shows: closes the output picker,
    /// the strip menu, or the showing section's detail. True if it closed
    /// something.
    pub fn io_back(&mut self) -> bool {
        if self.state.view != MixView::Io {
            return false;
        }
        let closed = if self.picker.take().is_some() || self.menu.take().is_some() {
            true
        } else {
            self.io.section_mut().is_some_and(|s| s.back())
        };
        if closed {
            self.mark(Dirty::All);
        }
        closed
    }

    /// The hint line for the I/O view as it shows now: the section's own,
    /// or [`crate::MIXER_HINT_IO`]; `None` away from the I/O view.
    pub fn io_hint(&self) -> Option<String> {
        if self.state.view != MixView::Io {
            return None;
        }
        Some(
            self.io
                .section()
                .and_then(|s| s.hint())
                .unwrap_or(crate::MIXER_HINT_IO)
                .to_string(),
        )
    }

    /// The Q-Link cards of the outside section showing, if it has its own.
    pub fn io_qlinks(&self) -> Option<[IoKnob; 4]> {
        if self.state.view != MixView::Io {
            return None;
        }
        self.io.section().and_then(|s| s.qlinks())
    }

    /// A Q-Link turned while an outside section with its own cards shows.
    pub fn io_qlink_turn(&mut self, knob: usize, detents: i32, fine: bool) -> IoReply {
        if self.io_qlinks().is_none() {
            return IoReply::Ignored;
        }
        let r = self
            .io
            .section_mut()
            .map_or(IoReply::Ignored, |s| s.qlink_turn(knob, detents, fine));
        if r == IoReply::Redraw {
            self.io_changed.extend(self.io.sel.checked_sub(1));
            self.mark(Dirty::Panel);
        }
        r
    }

    /// Tells the sections which one shows, when that changes.
    fn sync_shown(&mut self) {
        let now = (self.state.view == MixView::Io)
            .then(|| self.io.sel.checked_sub(1))
            .flatten()
            .filter(|&i| i < self.io.sections.len());
        if now == self.io_live {
            return;
        }
        if let Some(s) = self.io_live.and_then(|i| self.io.sections.get_mut(i)) {
            s.shown(false);
        }
        if let Some(s) = now.and_then(|i| self.io.sections.get_mut(i)) {
            s.shown(true);
        }
        self.io_live = now;
    }

    /// What the head's page arrows and count show: the first item, how many
    /// show, how many there are, and whether an outside section has the
    /// arrows (then they are always lit and there is no count).
    fn span(&self, l: &Layout) -> (usize, usize, usize) {
        let n = self.state.strips.len();
        if self.state.view != MixView::Io {
            let shown = match self.look {
                Look::Faders => self.shown(l).len(),
                Look::Sends | Look::Effects | Look::Eq => self.rows(l).len(),
            };
            return (self.first, shown, n);
        }
        let rows = io::rows_per_page(self.io_panel(l));
        let top = self.io.top.min(n);
        (top, rows.min(n - top), n)
    }

    /// Whether the page arrows (< and >) have somewhere to go: the strips'
    /// pages, or an open I/O section's own ([`IoSection::can_page`]).
    fn arrows(&self, l: &Layout) -> [bool; 2] {
        if self.state.view == MixView::Io {
            if let Some(s) = self.io.section() {
                return [s.can_page(-1), s.can_page(1)];
            }
        }
        let (first, shown, n) = self.span(l);
        [first > 0, first + shown < n]
    }

    fn io_panel(&self, l: &Layout) -> Rect {
        io::io_layout(l.strips, 0).panel
    }

    /// The Audio rows shown and their rectangles, under the column heads.
    fn io_rows(&self, panel: Rect) -> Vec<(usize, Rect)> {
        let n = self.state.strips.len();
        let per = io::rows_per_page(panel);
        let top = self.io.top.min(n);
        (top..n.min(top + per))
            .enumerate()
            .map(|(k, i)| {
                let y = panel.y + io::HEAD_ROW_H + k as i32 * io::ROW_H;
                (i, Rect::new(panel.x, y, panel.w, io::ROW_H - 4))
            })
            .collect()
    }

    fn io_page(&mut self, d: i32) {
        let outside = self.io.sel.checked_sub(1);
        if let Some(s) = self.io.section_mut() {
            if s.input(IoInput::Page(d)) == IoReply::Redraw {
                self.io_changed.extend(outside);
            }
            return;
        }
        let per = self.io_per_page.get().max(1);
        let n = self.state.strips.len();
        if d < 0 {
            self.io.top = self.io.top.saturating_sub(per);
        } else if self.io.top + per < n {
            self.io.top += per;
        }
    }

    fn io_select(&mut self, id: StripId) {
        if self.state.selected != Some(id) {
            self.state.selected = Some(id);
            self.push(MixAction::Select(id));
        }
    }

    fn io_wheel(&mut self, d: i32) -> IoReply {
        let n = self.state.strips.len();
        if n == 0 || d == 0 {
            return IoReply::Handled;
        }
        let cur = self
            .state
            .selected
            .and_then(|s| self.state.strips.iter().position(|v| v.id == s));
        let i = match cur {
            Some(c) => (c as i64 + i64::from(d)).clamp(0, n as i64 - 1) as usize,
            None if d > 0 => 0,
            None => n - 1,
        };
        if Some(i) == cur {
            return IoReply::Handled;
        }
        self.io_select(self.state.strips[i].id);
        self.follow_selection();
        IoReply::Redraw
    }

    fn io_step(&mut self, d: i32) -> IoReply {
        let Some(s) = self
            .state
            .selected
            .and_then(|id| self.state.strips.iter().find(|v| v.id == id))
            .filter(|s| s.has_output())
            .cloned()
        else {
            return IoReply::Handled;
        };
        let choices = output_choices(s.id, self.state.tracks);
        let cur = (
            s.output,
            if s.output == output::TRACK as u8 {
                s.output_track
            } else {
                0
            },
        );
        let at = choices.iter().position(|c| *c == cur).unwrap_or(0);
        let to = (at as i64 + i64::from(d.signum())).clamp(0, choices.len() as i64 - 1) as usize;
        if to == at && choices.get(at) == Some(&cur) {
            return IoReply::Handled;
        }
        let (out, track) = choices[to];
        if out == output::TRACK as u8 {
            self.set(s.id, mx::OUTPUT_TRACK, f32::from(track));
        }
        self.set(s.id, mx::OUTPUT, f32::from(out));
        IoReply::Redraw
    }

    /// A finger down in the I/O view's area (left of the master).
    fn io_down(&mut self, finger: u64, x: i32, y: i32, l: &Layout) {
        let il = io::io_layout(l.strips, self.io.sections.len() + 1);
        if let Some(i) = il.list.iter().position(|b| b.contains(x, y)) {
            // The list: back out of the section's detail, or show another.
            if let Some(s) = self.io.section_mut() {
                s.back();
            }
            if i != self.io.sel {
                self.io.sel = i;
                self.io.fingers.clear();
            }
            return;
        }
        if !il.panel.contains(x, y) {
            return;
        }
        if let Some(k) = self.io.sel.checked_sub(1) {
            if k < self.io.sections.len() {
                self.io.fingers.retain(|(f, _)| *f != finger);
                self.io.fingers.push((finger, k));
                let r = self.io.sections[k].touch(finger, Touch::Down, x, y, il.panel);
                if r == IoReply::Redraw {
                    self.io_changed.push(k);
                }
            }
            return;
        }
        let Some((i, row)) = self
            .io_rows(il.panel)
            .into_iter()
            .find(|(_, r)| r.contains(x, y))
        else {
            return;
        };
        let s = self.state.strips[i].clone();
        let p = io::row_parts(row);
        if p.output.contains(x, y) && s.has_output() {
            self.picker = Some(s.id);
        } else if let Some(inp) = s.input.filter(|_| p.source.contains(x, y)) {
            self.set(s.id, mx::SOURCE, f32::from((inp.source + 1) % 4));
        } else if let Some(inp) = s.input.filter(|_| p.monitor.contains(x, y)) {
            self.set(s.id, mx::MONITOR, f32::from((inp.monitor + 1) % 3));
        } else if p.name.contains(x, y) && self.state.selected == Some(s.id) {
            if self.state.takes {
                self.menu = Some(s.id);
            }
        } else {
            self.io_select(s.id);
        }
    }

    /// A finger an outside section holds moved or lifted. True if it was
    /// one.
    fn io_finger(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> bool {
        let Some(at) = self.io.fingers.iter().position(|(f, _)| *f == finger) else {
            return false;
        };
        let k = self.io.fingers[at].1;
        if matches!(phase, Touch::Up | Touch::Cancel) {
            self.io.fingers.remove(at);
        }
        let panel = self.io_panel(&layout(r));
        if let Some(s) = self.io.sections.get_mut(k) {
            if s.touch(finger, phase, x, y, panel) == IoReply::Redraw {
                self.io_changed.push(k);
            }
        }
        true
    }

    fn paint_io(&self, cx: &mut PaintCx, l: &Layout) {
        let t = cx.theme.clone();
        let il = io::io_layout(l.strips, self.io.sections.len() + 1);
        self.io_per_page.set(io::rows_per_page(il.panel));
        cx.canvas.fill_rect(l.strips, t.bg);
        for (i, (b, name)) in il.list.iter().zip(self.io.labels()).enumerate() {
            let on = i == self.io.sel;
            cx.canvas
                .fill_round_rect(*b, t.round(10), if on { t.raised } else { t.surface });
            if on {
                cx.canvas
                    .fill_round_rect(Rect::new(b.x, b.y + 10, 4, b.h - 20), 2, t.accent);
            }
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body,
                b.inset_xy(16, 0),
                Align::Left,
                if on { t.text } else { t.text_dim },
                &name,
            );
        }
        if let Some(s) = self.io.section() {
            s.paint(cx, il.panel);
            return;
        }
        self.paint_audio(cx, il.panel);
    }

    /// The Audio section: one row per strip with an output.
    fn paint_audio(&self, cx: &mut PaintCx, panel: Rect) {
        let t = cx.theme.clone();
        let head = Rect::new(panel.x, panel.y, panel.w, io::HEAD_ROW_H);
        let hp = io::row_parts(Rect::new(head.x, head.y - 4, head.w, head.h));
        for (r, s) in [
            (hp.name, "Strip"),
            (hp.output, "Output"),
            (hp.source, "Source (inputs)"),
            (hp.monitor, "Monitor (inputs)"),
        ] {
            cx.text(FontId::Sans, t.size_small, r, Align::Left, t.text_faint, s);
        }
        if self.state.strips.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_body,
                panel,
                Align::Center,
                t.text_dim,
                "No strips",
            );
            return;
        }
        for (i, row) in self.io_rows(panel) {
            let s = &self.state.strips[i];
            let selected = self.state.selected == Some(s.id);
            cx.canvas.fill_round_rect(row, t.round(10), t.surface);
            if selected {
                cx.canvas.stroke_round_rect(row, t.round(10), 2, t.accent);
            }
            let p = io::row_parts(row);
            let ink = if s.audible { t.text } else { t.text_faint };
            cx.canvas.fill_round_rect(
                Rect::new(p.name.x, p.name.y + 6, 4, p.name.h - 12),
                2,
                strip_color(&t, s.id),
            );
            cx.text(
                if selected { FontId::Bold } else { FontId::Sans },
                t.size_body,
                p.name.inset_xy(14, 0),
                Align::Left,
                ink,
                &s.name,
            );
            let open = self.picker == Some(s.id);
            let none = s.output == output::NONE as u8;
            cx.canvas.fill_round_rect(
                p.output,
                t.round(8),
                if open { t.picked() } else { t.raised },
            );
            cx.text(
                FontId::Sans,
                t.size_body,
                p.output.inset_xy(12, 0),
                Align::Left,
                if open {
                    t.on_picked()
                } else if none {
                    t.warn
                } else {
                    t.text
                },
                &if none {
                    "None (silent)".to_string()
                } else {
                    s.output_label()
                },
            );
            if let Some(inp) = s.input {
                for (r, v) in [
                    (p.source, choice(mx::SOURCE, inp.source)),
                    (p.monitor, choice(mx::MONITOR, inp.monitor)),
                ] {
                    cx.canvas.fill_round_rect(r, t.round(8), t.raised);
                    cx.text(
                        FontId::Sans,
                        t.size_body,
                        r.inset_xy(12, 0),
                        Align::Left,
                        t.text,
                        v,
                    );
                }
            }
        }
    }
}

/// The small I/O icon at the top bar's right edge (Jay's call, 2026-11-12): two
/// arrows, in and out. A tap gives `Tab(0)`; the app opens the mixer on
/// its I/O view, or closes it. Offered to the app thread, which places it.
#[derive(Clone, Copy, Debug, Default)]
pub struct IoIcon {
    /// The I/O view is showing (the icon is lit).
    pub open: bool,
}

impl Widget for IoIcon {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let side = 44.min(r.h).min(r.w);
        let b = Rect::new(r.x + (r.w - side) / 2, r.y + (r.h - side) / 2, side, side);
        cx.canvas
            .fill_round_rect(b, t.round(10), if self.open { t.raised } else { t.surface });
        if self.open {
            cx.canvas.fill_round_rect(
                Rect::new(b.x + 10, b.bottom() - 4, b.w - 20, 3),
                1,
                t.accent,
            );
        }
        let ink = if self.open { t.text } else { t.text_dim };
        let (x0, x1) = (b.x + 10, b.right() - 10);
        // In: an arrow pointing right; out: one pointing left, under it.
        for (y, right) in [(b.y + 15, true), (b.y + b.h - 17, false)] {
            cx.canvas.fill_rect(Rect::new(x0, y, x1 - x0, 2), ink);
            for k in 0..5 {
                let h = 2 + 2 * (4 - k);
                let x = if right { x1 - 5 + k } else { x0 + 4 - k };
                cx.canvas.fill_rect(Rect::new(x, y + 1 - h / 2, 1, h), ink);
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        vec![Target::tap("icon", 0, "I/O", r)]
    }

    fn touch(&mut self, _f: u64, phase: Touch, _x: i32, _y: i32, _r: Rect) -> Option<EventKind> {
        (phase == Touch::Down).then_some(EventKind::Tab(0))
    }
}

/// A state with its meters (peaks, costs, limiting) cleared, to tell a
/// meter-only update from the rest.
fn strip_meters(st: &MixState) -> MixState {
    let mut s = st.clone();
    let clear = |v: &mut StripView| {
        v.peak = [0.0; 2];
        v.cost_pct = None;
        // A take's progress is drawn like a meter; its start and end are not.
        if let Some(TakeView { done: Some(d), .. }) = v.take.as_mut() {
            *d = 0.0;
        }
        for i in &mut v.inserts {
            i.cost_pct = None;
        }
    };
    s.strips.iter_mut().for_each(clear);
    clear(&mut s.master);
    s.safety.reduction_db = 0.0;
    s.safety.overs = 0;
    s.cost_pct = None;
    s
}

/// Applies a parameter to a strip view (what the screen shows at once).
pub fn apply(s: &mut StripView, id: u16, value: f32) {
    match id {
        mx::VOLUME => s.volume_db = value,
        mx::PAN => s.pan = value,
        mx::MUTE => s.mute = value >= 0.5,
        mx::SOLO => s.solo = value >= 0.5,
        mx::OUTPUT => s.output = value as u8,
        mx::OUTPUT_TRACK => s.output_track = value as u8,
        6..=9 => {
            if let Some(x) = s.sends.as_mut() {
                x[(id - 6) as usize].db = value;
            }
        }
        10..=13 => {
            if let Some(x) = s.sends.as_mut() {
                x[(id - 10) as usize].pre = value >= 0.5;
            }
        }
        mx::SOURCE => {
            if let Some(i) = s.input.as_mut() {
                i.source = value as u8;
            }
        }
        mx::MONITOR => {
            if let Some(i) = s.input.as_mut() {
                i.monitor = value as u8;
            }
        }
        mx::ARM => {
            if let Some(i) = s.input.as_mut() {
                i.arm = value >= 0.5;
            }
        }
        _ => {}
    }
}

impl Widget for MixerView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        self.dirty.set(Dirty::Clean);
        let l = layout(r);
        self.per_page.set(self.page_size(&l));
        let clip = cx.canvas.clip();
        if !clip.intersect(Rect::new(r.x, r.y, r.w, HEAD_H)).is_empty() {
            self.paint_head(cx, &l);
        }
        let effects = self.look.rows() && self.state.view != MixView::Io;
        if effects {
            if !clip.intersect(grid(&l).panel).is_empty() {
                match self.look {
                    Look::Eq => self.paint_eq(cx, &l),
                    Look::Sends => self.paint_sends(cx, &l),
                    _ => self.paint_effects(cx, &l),
                }
            }
        } else {
            for (i, sr) in self.shown(&l) {
                if !clip.intersect(sr).is_empty() {
                    self.paint_strip(cx, &self.state.strips[i], sr);
                }
            }
        }
        if !effects && !clip.intersect(l.master).is_empty() {
            self.paint_strip(cx, &self.state.master, l.master);
        }
        if self.state.view == MixView::Io {
            if !clip.intersect(l.strips).is_empty() {
                self.paint_io(cx, &l);
            }
        } else if self.state.strips.is_empty() {
            let t = cx.theme.clone();
            let msg = match self.state.view {
                MixView::Pads => "No pad channels: every pad plays in its kit's track",
                MixView::Inputs => "No input strips",
                _ => "No strips",
            };
            let area = if effects {
                let g = grid(&l);
                Rect::new(g.left, g.top, g.width, g.master.y - 12 - g.top)
            } else {
                l.strips
            };
            cx.text(
                FontId::Sans,
                t.size_body,
                area,
                Align::Center,
                t.text_dim,
                msg,
            );
        }
        if let Some(s) = self.picker {
            self.paint_picker(cx, &l, s);
        }
        if let Some(s) = self.menu {
            self.paint_menu(cx, &l, s);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let l = layout(r);
        // A menu or the output picker takes the panel while it is open.
        if let Some(s) = self.menu {
            let v = self.state.strip(s);
            return self
                .menu_buttons(&l, s)
                .into_iter()
                .map(|(m, b)| {
                    let label = v.map_or("", |v| m.text(v).0);
                    Target::tap("menu", m as usize, label, b)
                })
                .collect();
        }
        if let Some(s) = self.picker {
            return self
                .picker_buttons(&l, s)
                .into_iter()
                .enumerate()
                .map(|(i, ((out, track), b))| {
                    Target::tap("output-choice", i, output_label(out, track), b)
                })
                .collect();
        }
        let mut out: Vec<Target> = MixView::ALL
            .iter()
            .enumerate()
            .map(|(i, v)| {
                // The view showing is drawn lit: a tap on it changes nothing.
                Target::tap("view", i, v.label(), l.chips[i]).disabled_if(*v == self.state.view)
            })
            .collect();
        let [back, on] = self.arrows(&l);
        out.push(Target::tap("prev", 0, "<", l.prev).disabled_if(!back));
        out.push(Target::tap("next", 0, ">", l.next).disabled_if(!on));
        for (i, (h, look)) in look_halves(l.look).into_iter().zip(Look::ALL).enumerate() {
            out.push(Target::tap("look", i, look.label(), h).disabled_if(look == self.look));
        }
        if self.state.view == MixView::Io {
            let il = io::io_layout(l.strips, self.io.sections.len() + 1);
            for (i, (b, label)) in il.list.iter().zip(self.io.labels()).enumerate() {
                out.push(Target::tap("io-section", i, label, *b).disabled_if(i == self.io.sel));
            }
            if let Some(sec) = self.io.section() {
                out.extend(sec.targets(il.panel));
                return out;
            }
            for (i, row) in self.io_rows(il.panel) {
                let s = &self.state.strips[i];
                let p = io::row_parts(row);
                // The selected strip's name opens its menu only with takes,
                // as on the strips.
                let picked = self.state.selected == Some(s.id);
                out.push(
                    Target::tap("io-name", i, s.name.clone(), p.name)
                        .disabled_if(picked && !self.state.takes),
                );
                if s.has_output() {
                    out.push(Target::tap(
                        "io-output",
                        i,
                        format!("{} output", s.name),
                        p.output,
                    ));
                }
                if s.input.is_some() {
                    out.push(Target::tap(
                        "io-source",
                        i,
                        format!("{} source", s.name),
                        p.source,
                    ));
                    out.push(Target::tap(
                        "io-monitor",
                        i,
                        format!("{} monitor", s.name),
                        p.monitor,
                    ));
                }
            }
            return out;
        }
        if self.look == Look::Eq {
            let mut rows: Vec<(&StripView, Rect)> = self
                .rows(&l)
                .into_iter()
                .map(|(i, row)| (&self.state.strips[i], row))
                .collect();
            rows.push((&self.state.master, grid(&l).master));
            for (k, (s, row)) in rows.into_iter().enumerate() {
                let (name, graph, _) = eq_look::row_parts(row, FX_NAME_W);
                let picked = self.state.selected == Some(s.id);
                out.push(
                    Target::tap("eq-name", k, s.name.clone(), name)
                        .disabled_if(picked && !self.state.takes),
                );
                if s.eq_slot().is_some() {
                    out.push(Target::tap("eq", k, format!("{} EQ", s.name), graph));
                } else if s.inserts.len() < s.slots {
                    out.push(Target::tap("eq-add", k, format!("{} + EQ", s.name), graph));
                }
            }
            return out;
        }
        if self.look == Look::Sends {
            for (k, (id, row)) in self.sends_rows(&l).into_iter().enumerate() {
                let Some(s) = self.state.strip(id) else {
                    continue;
                };
                let r = crate::sends_look::row_parts(row, FX_NAME_W);
                let name = &s.name;
                let obj = format!("strip {:?}", s.id);
                let edit = (Context::Make, Consequence::Disruptive, Timing::None);
                let picked = self.state.selected == Some(s.id);
                // The rows' cells are rows of one kind: Dense, as the
                // Effects look's.
                out.push(
                    Target::tap("sends-name", k, name.clone(), r.name)
                        .object(obj.clone())
                        .commit(Commit::Press)
                        .class(Class::Dense)
                        .disabled_if(picked && !self.state.takes),
                );
                out.push(
                    Target::drag("pan", k, format!("{name} pan"), r.pan)
                        .towards(40, 0)
                        .object(obj.clone())
                        .class(Class::Dense)
                        .plays(edit.0, edit.1, edit.2),
                );
                if let Some(_sends) = s.sends {
                    for j in 0..RETURNS {
                        let send = ["A", "B", "C", "D"][j];
                        out.push(
                            Target::tap(
                                "send-pre",
                                k * 4 + j,
                                format!("{name} send {send} pre/post"),
                                r.pre[j],
                            )
                            .object(obj.clone())
                            .class(Class::Dense)
                            .plays(
                                Context::Make,
                                Consequence::Minor,
                                Timing::None,
                            ),
                        );
                        out.push(
                            Target::drag(
                                "send",
                                k * 4 + j,
                                format!("{name} send {send}"),
                                r.send[j],
                            )
                            .towards(40, 0)
                            .object(obj.clone())
                            .class(Class::Dense)
                            .plays(edit.0, edit.1, edit.2),
                        );
                    }
                }
                if s.has_output() {
                    out.push(
                        Target::tap("output", k, format!("{name} output"), r.output)
                            .object(obj.clone())
                            .class(Class::Dense)
                            .plays(Context::Make, Consequence::Minor, Timing::None),
                    );
                }
            }
            return out;
        }
        if self.look == Look::Effects {
            let mut rows: Vec<(&StripView, Rect)> = self
                .rows(&l)
                .into_iter()
                .map(|(i, row)| (&self.state.strips[i], row))
                .collect();
            rows.push((&self.state.master, grid(&l).master));
            for (k, (s, row)) in rows.into_iter().enumerate() {
                let (name, cells) = row_cells(row);
                let picked = self.state.selected == Some(s.id);
                out.push(
                    Target::tap("fx-name", k, s.name.clone(), name)
                        .disabled_if(picked && !self.state.takes),
                );
                for (j, c) in cells.into_iter().enumerate().take(s.slots.min(FX_COLS)) {
                    if let Some(ins) = s.inserts.get(j) {
                        out.push(Target::tap("fx", k * FX_COLS + j, ins.name.clone(), c));
                    } else if j == s.inserts.len() {
                        out.push(Target::tap("fx-add", k, format!("{} + FX", s.name), c));
                    }
                }
            }
            return out;
        }
        let mut strips: Vec<(&StripView, Rect)> = self
            .shown(&l)
            .into_iter()
            .map(|(i, sr)| (&self.state.strips[i], sr))
            .collect();
        strips.push((&self.state.master, l.master));
        for (k, (s, sr)) in strips.into_iter().enumerate() {
            let p = parts_of(sr, s, self.perform);
            let name = &s.name;
            // What each control says about touch (layout spec 1a; the
            // touch plan's A7): Make's taps act on the lift, Perform's
            // MUTE and SOLO on the press.
            let obj = format!("strip {:?}", s.id);
            let edit = (Context::Make, Consequence::Disruptive, Timing::None);
            // The names across the page are a row of one kind: Dense. The
            // selected one's opens its menu only with takes.
            let picked = self.state.selected == Some(s.id);
            out.push(
                Target::tap("strip-name", k, name.clone(), p.name.take_top(NAME_H).0)
                    .object(obj.clone())
                    .commit(Commit::Press)
                    .class(Class::Dense)
                    .disabled_if(picked && !self.state.takes),
            );
            if !self.perform {
                // The chain's summaries across the page: Dense, as the
                // names; the Effects look is the bigger way in.
                out.push(
                    Target::tap("strip-fx", k, format!("{name} effects"), p.fx)
                        .object(obj.clone())
                        .class(Class::Dense),
                );
                if s.id != StripId::Master {
                    out.push(
                        Target::tap("strip-path", k, format!("{name} sends"), p.path)
                            .object(obj.clone()),
                    );
                }
            } else {
                out.push(
                    Target::drag("pan", k, format!("{name} pan"), p.pan)
                        .towards(40, 0)
                        .object(obj.clone())
                        .plays(Context::Perform, Consequence::Disruptive, Timing::None),
                );
            }
            // A fader is played in Perform as well.
            out.push(
                Target::drag("fader", k, format!("{name} fader"), p.fader)
                    .towards(0, -40)
                    .object(obj.clone())
                    .plays(Context::Both, Consequence::Disruptive, Timing::None),
            );
            if !self.perform {
                // The level types a number on the lift (#1007): a small
                // target, with Q-Link 1 and the wheel beside it.
                out.push(
                    Target::tap("value", k, format!("{name} level (type it)"), p.value)
                        .object(obj.clone())
                        .class(Class::Precision),
                );
            }
            if s.id == StripId::Master {
                continue;
            }
            let ids = ["mute", "solo", "arm"];
            let words = if self.perform {
                ["MUTE", "SOLO", "Arm"]
            } else {
                ["M", "S", "Arm"]
            };
            for (j, b) in p.buttons().iter().enumerate() {
                let t =
                    Target::tap(ids[j], k, format!("{name} {}", words[j]), *b).object(obj.clone());
                out.push(if self.perform {
                    // Played: on the press, at once.
                    t.commit(Commit::Press).class(Class::Performance).plays(
                        Context::Perform,
                        Consequence::Disruptive,
                        Timing::Immediate,
                    )
                } else if j == 2 {
                    // An input's Arm under M and S: Dense (the pads and
                    // REC arm too).
                    t.class(Class::Dense)
                        .plays(edit.0, edit.1, Timing::Immediate)
                } else {
                    t.plays(edit.0, edit.1, Timing::Immediate)
                });
            }
        }
        out
    }

    fn steps(&self, t: &Target) -> bool {
        matches!(t.id, "fader" | "pan" | "send")
    }

    /// The data wheel on a strip's fader, pan or send: as Q-Links 1 to 4
    /// turn them ([`crate::qlinks::level_turn`], [`crate::qlinks::pan_turn`]).
    fn step(&mut self, t: &Target, delta: i32, r: Rect) -> Option<EventKind> {
        self.turn(t, delta, false, r)
    }

    /// Held with SHIFT: the fine steps SHIFT gives the Q-Links.
    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, r: Rect) -> Option<EventKind> {
        self.turn(t, delta, fine, r)
    }

    /// SHIFT + the wheel's push: the fader to 0 dB, the pan to the middle,
    /// a send off. A tap never does it (the touch usability review, 25 Nov
    /// 2026): a tap on the level asks for a number, one on the pan does
    /// nothing, so a slip never resets either.
    fn reset(&mut self, t: &Target, r: Rect) -> Option<EventKind> {
        let (strip, j) = self.strip_of(t, r)?;
        let s = self.state.strip(strip)?;
        let (id, from, to) = match t.id {
            "fader" => (mx::VOLUME, s.volume_db, 0.0),
            "pan" => (mx::PAN, s.pan, 0.0),
            "send" => (mx::send(j), s.sends?.get(j)?.db, SILENT_DB),
            _ => return None,
        };
        if to == from {
            return None;
        }
        self.set(strip, id, to);
        self.mark(Dirty::All);
        Some(EventKind::Select(self.actions.len()))
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let before = self.actions.len();
        let changed_before = self.io_changed.len();
        let section = self.io.sel;
        match phase {
            Touch::Down => self.down(finger, x, y, r),
            Touch::Move => self.drag(finger, x, y, r),
            Touch::Up => self.lift(finger, false, x, y, r),
            Touch::Cancel => self.lift(finger, true, x, y, r),
        }
        self.sync_shown();
        self.grab_io_note();
        // Paging, the picker and the copy under the finger all show at once.
        self.mark(Dirty::All);
        // The app hears of actions, of a section's change, of another
        // section picked (its hint and Q-Links follow) and of a note to
        // say.
        (self.actions.len() > before
            || changed_before < self.io_changed.len()
            || self.io.sel != section
            || self.io_note.is_some())
        .then_some(EventKind::Select(self.actions.len()))
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        match self.dirty.get() {
            Dirty::Clean => Some(Vec::new()),
            Dirty::Meters => Some(self.meter_parts(r)),
            Dirty::Panel => {
                let mut v = self.meter_parts(r);
                v.push(self.io_panel(&layout(r)));
                Some(v)
            }
            Dirty::All => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{default_name, InsertView, SendView, TakeView};

    use mpfree_ui::{Fonts, SoftCanvas, Ui};

    fn state(n: usize) -> MixState {
        let mut st = MixState {
            counts: [n, 32, 2, BUSES + RETURNS],
            ..Default::default()
        };
        st.strips = (0..n)
            .map(|t| StripView::new(StripId::Track(t), default_name(StripId::Track(t))))
            .collect();
        st.strips[0].inserts = vec![InsertView {
            name: "Compressor".into(),
            on: true,
            key: Some("Track 2".into()),
            nothing_picked: false,
            cost_pct: None,
            eq: None,
        }];
        st.strips[0].sends.as_mut().unwrap()[1] = SendView {
            db: -12.0,
            pre: true,
        };
        st.strips[1].peak = [0.5, 0.25];
        st
    }

    fn ui(st: MixState) -> Ui {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        crate::add(&mut ui).unwrap();
        crate::update(&mut ui, st);
        ui.show(crate::MIXER_PAGE).unwrap();
        ui
    }

    fn strip_parts(ui: &Ui, col: usize) -> Parts {
        strip_parts_in(ui, col, false)
    }

    /// Strip `col`'s parts in Make or (`perform`) in Perform.
    fn strip_parts_in(ui: &Ui, col: usize, perform: bool) -> Parts {
        let r = ui.rect_of(crate::MIXER).unwrap();
        let l = layout(r);
        parts_with(
            Rect::new(
                l.strips.x + col as i32 * (STRIP_W + GAP),
                l.strips.y,
                STRIP_W,
                l.strips.h,
            ),
            0,
            perform,
            false,
        )
    }

    /// Row `k`'s cells on the Sends look.
    fn sends_row(ui: &Ui, k: usize) -> crate::sends_look::Row {
        let r = ui.rect_of(crate::MIXER).unwrap();
        crate::sends_look::row_parts(grid(&layout(r)).row(k), FX_NAME_W)
    }

    fn mid(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn the_fader_law_round_trips_and_has_a_detent() {
        for db in [-60.0, -40.0, -24.0, -12.0, -6.0, -3.5, 0.0, 2.5, 6.0] {
            assert!((pos_to_db(db_to_pos(db)) - db).abs() <= 0.05, "{db}");
        }
        assert_eq!(db_to_pos(SILENT_DB), 0.0);
        assert_eq!(pos_to_db(0.0), SILENT_DB);
        assert_eq!(db_to_pos(0.0), 0.8);
        assert_eq!(pos_to_db(0.801), 0.0, "detent");
        assert_eq!(db_to_pos(20.0), 1.0);
        let mut last = f32::NEG_INFINITY;
        for i in 0..=100 {
            let d = pos_to_db(i as f32 / 100.0);
            assert!(d >= last, "monotonic at {i}");
            last = d;
        }
        assert_eq!(
            (
                db_text(-3.04),
                db_text(0.0),
                db_text(2.0),
                db_text(SILENT_DB)
            ),
            (
                "-3.0 dB".into(),
                "0.0 dB".into(),
                "+2.0 dB".into(),
                "off".into()
            )
        );
        assert_eq!(
            (pan_text(0.2), pan_text(-40.0), pan_text(25.0)),
            ("C".into(), "L40".into(), "R25".into())
        );
    }

    #[test]
    fn dragging_a_fader_sets_the_volume_relative_to_where_the_finger_landed() {
        let mut ui = ui(state(12));
        let p = strip_parts(&ui, 2);
        let (x, y) = (p.fader.x + p.fader.w / 2, p.fader.y + 20);
        // Landing anywhere does not jump the fader.
        assert_eq!(ui.touch(1, Touch::Down, x, y), None);
        let (top, bot) = travel(p.fader);
        let ev = ui
            .touch(1, Touch::Move, x, y + ((bot - top) as f32 * 0.14) as i32)
            .unwrap();
        assert_eq!(ev.widget, crate::MIXER);
        ui.touch(1, Touch::Up, x, y);
        let a = crate::take_actions(&mut ui);
        let Some(MixAction::Set { strip, id, value }) = a.last().cloned() else {
            panic!("{a:?}")
        };
        assert_eq!((strip, id), (StripId::Track(2), mx::VOLUME));
        assert!((value - -6.0).abs() < 0.5, "{value}");
        assert_eq!(
            ui.get::<MixerView>(crate::MIXER).unwrap().state().strips[2].volume_db,
            value,
            "the screen follows at once"
        );
    }

    #[test]
    fn the_data_wheel_turns_a_fader_pan_or_send_as_the_q_links_do() {
        let mut ui = ui(state(12));
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let p = strip_parts(&ui, 2);
        let (x, y) = mid(p.fader);
        assert_eq!(ui.focus_at(x, y).unwrap().id, "fader");
        assert!(ui.focus_steps());
        let ev = ui.focus_step(1).unwrap();
        assert_eq!(ev.widget, crate::MIXER);
        let a = crate::take_actions(&mut ui);
        let want = crate::qlinks::level_turn(0.0, 1, false);
        assert_eq!(
            a,
            vec![MixAction::Set {
                strip: StripId::Track(2),
                id: mx::VOLUME,
                value: want
            }]
        );
        assert_eq!(
            ui.get::<MixerView>(crate::MIXER).unwrap().state().strips[2].volume_db,
            want,
            "the screen follows at once"
        );
        // The pan and the sends on the Sends look.
        crate::set_look(&mut ui, Look::Sends);
        ui.paint(&mut c);
        let (x, y) = mid(sends_row(&ui, 2).pan);
        ui.focus_at(x, y);
        ui.focus_step(-3);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Set {
                strip: StripId::Track(2),
                id: mx::PAN,
                value: -6.0
            }]
        );
        // Strip 1's send B sits at -12 dB.
        let (x, y) = mid(sends_row(&ui, 0).send[1]);
        assert_eq!(ui.focus_at(x, y).unwrap().id, "send");
        ui.focus_step(1);
        let a = crate::take_actions(&mut ui);
        assert_eq!(
            a,
            vec![MixAction::Set {
                strip: StripId::Track(0),
                id: mx::send(1),
                value: crate::qlinks::level_turn(-12.0, 1, false)
            }]
        );
        // A mute is a tap: the wheel does not turn it.
        crate::set_look(&mut ui, Look::Faders);
        ui.paint(&mut c);
        let (x, y) = mid(strip_parts(&ui, 0).buttons[0]);
        assert_eq!(ui.focus_at(x, y).unwrap().id, "mute");
        assert!(!ui.focus_steps());
    }

    /// SHIFT + the wheel's push (the button combos of 19 Nov): the fader
    /// back to 0 dB, the pan to the middle, a send off; at its default
    /// already, nothing.
    #[test]
    fn shift_and_the_push_put_a_fader_pan_or_send_back_to_its_default() {
        let mut ui = ui(state(12));
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let p = strip_parts(&ui, 0);
        let (x, y) = mid(p.fader);
        ui.focus_at(x, y);
        assert_eq!(ui.focus_reset(), None, "at 0 dB already");
        ui.focus_step(-4);
        crate::take_actions(&mut ui);
        assert!(ui.focus_reset().is_some());
        let set = |strip, id, value| MixAction::Set { strip, id, value };
        let t0 = StripId::Track(0);
        assert_eq!(crate::take_actions(&mut ui), vec![set(t0, mx::VOLUME, 0.0)]);
        crate::set_look(&mut ui, Look::Sends);
        ui.paint(&mut c);
        let row = sends_row(&ui, 0);
        let (x, y) = mid(row.pan);
        ui.focus_at(x, y);
        ui.focus_step(5);
        crate::take_actions(&mut ui);
        ui.focus_reset();
        assert_eq!(crate::take_actions(&mut ui), vec![set(t0, mx::PAN, 0.0)]);
        // Send B sits at -12 dB: off.
        let (x, y) = mid(row.send[1]);
        ui.focus_at(x, y);
        ui.focus_reset();
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![set(t0, mx::send(1), SILENT_DB)]
        );
    }

    /// Make's strip (the touch plan's A7): M and S act on the lift, and a
    /// finger that slides off does nothing; the level asks for a number;
    /// the chain's summary opens the Effects look and the sends' the
    /// Sends look, each on that strip; the name selects.
    #[test]
    fn taps_mute_solo_on_the_lift_type_a_level_and_open_the_looks() {
        let mut ui = ui(state(12));
        let p = strip_parts(&ui, 0);
        let b = p.buttons;
        let tap = |ui: &mut Ui, (x, y): (i32, i32)| {
            ui.touch(9, Touch::Down, x, y);
            ui.touch(9, Touch::Up, x, y);
        };
        let (mx_, my_) = mid(b[0]);
        ui.touch(9, Touch::Down, mx_, my_);
        assert_eq!(crate::take_actions(&mut ui), vec![], "nothing on the press");
        ui.touch(9, Touch::Up, mx_, my_);
        tap(&mut ui, mid(b[1]));
        tap(&mut ui, mid(p.value));
        tap(&mut ui, mid(p.name));
        // Slid off M before letting go: nothing.
        ui.touch(9, Touch::Down, mx_, my_);
        ui.touch(9, Touch::Move, mx_, b[0].y - 30);
        ui.touch(9, Touch::Up, mx_, b[0].y - 30);
        let t0 = StripId::Track(0);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![
                MixAction::Set {
                    strip: t0,
                    id: mx::MUTE,
                    value: 1.0
                },
                MixAction::Set {
                    strip: t0,
                    id: mx::SOLO,
                    value: 1.0
                },
                MixAction::TypeLevel {
                    strip: t0,
                    db: 0.0,
                    name: default_name(t0),
                },
                MixAction::Select(t0),
            ]
        );
        tap(&mut ui, mid(b[0]));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Set {
                strip: t0,
                id: mx::MUTE,
                value: 0.0
            }],
            "a second tap unmutes"
        );
        // The chain's summary: the Effects look, on the selected strip.
        tap(&mut ui, mid(p.fx));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Look(Look::Effects)]
        );
        assert_eq!(crate::look(&ui), Look::Effects);
        // Strip 2's sends: it is selected, and the Sends look shows.
        crate::set_look(&mut ui, Look::Faders);
        let p1 = strip_parts(&ui, 1);
        tap(&mut ui, mid(p1.path));
        let t1 = StripId::Track(1);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Select(t1), MixAction::Look(Look::Sends)]
        );
        assert_eq!(crate::look(&ui), Look::Sends);
        assert_eq!(crate::hint(&ui), crate::MIXER_HINT_SENDS);
    }

    /// Perform's strip: MUTE and SOLO act as the finger lands (a second
    /// press undoes); the pan row only drags; the level is only shown;
    /// nothing opens a look. The same state: switching changes no value.
    #[test]
    fn performs_strip_mutes_on_the_press_and_has_nothing_else_to_hit() {
        let mut ui = ui(state(12));
        crate::set_perform(&mut ui, true);
        assert_eq!(crate::hint(&ui), crate::MIXER_HINT_PERFORM);
        let p = strip_parts_in(&ui, 0, true);
        assert_eq!((p.buttons[0].w, p.buttons[0].h), (92, 85), "10.8 x 10 mm");
        assert_eq!(p.buttons[1].y - p.buttons[0].bottom(), 12, "a dead gap");
        let t0 = StripId::Track(0);
        let (x, y) = mid(p.buttons[0]);
        ui.touch(9, Touch::Down, x, y);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Set {
                strip: t0,
                id: mx::MUTE,
                value: 1.0
            }],
            "on the press"
        );
        ui.touch(9, Touch::Up, x, y);
        // A tap on the level, between the buttons or on the pan: nothing.
        for at in [mid(p.value), (x, p.buttons[0].bottom() + 6), mid(p.pan)] {
            ui.touch(9, Touch::Down, at.0, at.1);
            ui.touch(9, Touch::Up, at.0, at.1);
        }
        assert_eq!(crate::take_actions(&mut ui), vec![]);
        assert_eq!(crate::look(&ui), Look::Faders);
        let (x, y) = mid(p.buttons[1]);
        ui.touch(9, Touch::Down, x, y);
        ui.touch(9, Touch::Up, x, y);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Set {
                strip: t0,
                id: mx::SOLO,
                value: 1.0
            }]
        );
        // The fader is longer than Make's.
        let make = strip_parts_in(&ui, 0, false);
        assert!(
            p.fader.h > make.fader.h + 15,
            "{} {}",
            p.fader.h,
            make.fader.h
        );
        crate::set_perform(&mut ui, false);
        let v = ui.get::<MixerView>(crate::MIXER).unwrap();
        assert!(v.state().strips[0].mute && v.state().strips[0].solo, "kept");
    }

    /// Make's buttons are big and stacked; an input's Arm under them.
    #[test]
    fn makes_m_and_s_are_full_width_and_stacked() {
        let ui = ui(state(12));
        let p = strip_parts(&ui, 0);
        assert_eq!(p.n, 2);
        assert_eq!((p.buttons[0].w, p.buttons[0].h), (92, 64));
        assert_eq!(p.buttons[1].y - p.buttons[0].bottom(), 12, "M and S apart");
        assert!(p.fader.h >= 242, "longer than 238: {}", p.fader.h);
        let r = ui.rect_of(crate::MIXER).unwrap();
        let l = layout(r);
        let mut input = StripView::new(StripId::Input(0), "Line in");
        input.input = Some(crate::InputView {
            source: 1,
            monitor: 2,
            arm: false,
        });
        let sr = Rect::new(l.strips.x, l.strips.y, STRIP_W, l.strips.h);
        let pi = parts_of(sr, &input, false);
        assert_eq!(pi.n, 3);
        assert_eq!(pi.buttons[2].h, ARM_H);
        assert_eq!(parts_of(sr, &input, true).n, 2, "no Arm in Perform");
    }

    /// The strip's controls say how they act (layout spec 1a; the touch
    /// plan's A7): in Make, M and S on the lift (Normal), the summaries
    /// Dense and Normal, the level Precision; in Perform, MUTE and SOLO on
    /// the press (Performance); the fader and pan follow the finger. Each
    /// names its strip.
    #[test]
    fn the_strip_says_how_each_control_acts() {
        use mpfree_ui::{Class, Commit, Context, Movement};
        let mut ui = ui(state(12));
        let r = ui.rect_of(crate::MIXER).unwrap();
        let all = ui.get::<MixerView>(crate::MIXER).unwrap().targets(r);
        let find = |all: &[Target], id: &str| {
            all.iter()
                .find(|t| t.id == id && t.index == 0)
                .cloned()
                .unwrap()
        };
        let mute = find(&all, "mute");
        assert_eq!(mute.commit, Commit::LiftInside);
        assert_eq!(mute.context, Context::Make);
        assert_eq!(mute.needed_class(), Class::Normal);
        assert_eq!(mute.object.as_deref(), Some("strip Track(0)"));
        assert_eq!(find(&all, "fader").movement, Movement::Relative);
        assert_eq!(find(&all, "fader").object, mute.object);
        assert_eq!(find(&all, "fader").needed_class(), Class::Performance);
        assert_eq!(find(&all, "value").commit, Commit::LiftInside);
        assert_eq!(find(&all, "value").needed_class(), Class::Precision);
        assert_eq!(find(&all, "strip-fx").needed_class(), Class::Dense);
        assert_eq!(find(&all, "strip-path").needed_class(), Class::Normal);
        assert!(!all
            .iter()
            .any(|t| matches!(t.id, "pan" | "send" | "insert" | "output")));
        crate::set_perform(&mut ui, true);
        let all = ui.get::<MixerView>(crate::MIXER).unwrap().targets(r);
        let mute = find(&all, "mute");
        assert_eq!(mute.commit, Commit::Press);
        assert_eq!(mute.context, Context::Perform);
        assert_eq!(mute.needed_class(), Class::Performance);
        assert_eq!(find(&all, "pan").movement, Movement::Relative);
        assert!(!all
            .iter()
            .any(|t| matches!(t.id, "value" | "strip-fx" | "strip-path")));
    }

    /// A tap on a strip's pan leaves it where it is (the touch usability
    /// review, 25 Nov 2026: the 26 px row sat between the sends and the
    /// fader, and a tap centred it, so a finger reaching for the fader
    /// could throw away a pan set by ear). A drag moves it; SHIFT + the
    /// wheel's push centres it.
    #[test]
    fn a_tap_on_the_pan_leaves_it_and_a_drag_moves_it() {
        let mut st = state(12);
        st.strips[0].pan = -40.0;
        let mut ui = ui(st);
        crate::set_look(&mut ui, Look::Sends);
        let t0 = StripId::Track(0);
        let (x, y) = mid(sends_row(&ui, 0).pan);
        ui.touch(9, Touch::Down, x, y);
        ui.touch(9, Touch::Move, x + 3, y + 1);
        ui.touch(9, Touch::Up, x + 3, y + 1);
        assert_eq!(crate::take_actions(&mut ui), vec![], "a tap: nothing");
        ui.touch(9, Touch::Down, x, y);
        ui.touch(9, Touch::Move, x + 20, y);
        ui.touch(9, Touch::Up, x + 20, y);
        let acts = crate::take_actions(&mut ui);
        assert!(
            matches!(
                acts.last(),
                Some(MixAction::Set { strip, id, value }) if *strip == t0 && *id == mx::PAN && *value > -40.0
            ),
            "a drag right moves it right: {acts:?}"
        );
    }

    /// The level under a fader never changes it on a tap (the touch
    /// usability review, 25 Nov 2026: it sat 6 px above M and S and put
    /// the fader at 0 dB, so a finger reaching for M could jump the level
    /// by 20 dB or more). A tap let go on it asks for a number instead;
    /// slid off it first, or cancelled, nothing; a finger that goes down
    /// on it and lifts on M neither mutes nor asks.
    #[test]
    fn a_tap_on_the_level_asks_for_a_number_and_never_moves_the_fader() {
        let mut st = state(12);
        st.strips[1].volume_db = -20.0;
        let mut ui = ui(st);
        let p = strip_parts(&ui, 1);
        let t1 = StripId::Track(1);
        let (x, y) = mid(p.value);
        ui.touch(9, Touch::Down, x, y);
        assert_eq!(crate::take_actions(&mut ui), vec![], "nothing on the press");
        ui.touch(9, Touch::Up, x + 3, y + 2);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::TypeLevel {
                strip: t1,
                db: -20.0,
                name: default_name(t1),
            }]
        );
        // Slid off onto M before letting go, or cancelled: nothing.
        let m = mid(p.buttons[0]);
        ui.touch(9, Touch::Down, x, y);
        ui.touch(9, Touch::Move, m.0, m.1);
        ui.touch(9, Touch::Up, m.0, m.1);
        ui.touch(9, Touch::Down, x, y);
        ui.touch(9, Touch::Cancel, x, y);
        assert_eq!(crate::take_actions(&mut ui), vec![]);
        // The target says what a tap does.
        let label = ui
            .targets()
            .into_iter()
            .find(|t| t.target.id == "value" && t.target.index == 1)
            .map(|t| t.target.label)
            .unwrap();
        assert_eq!(label, format!("{} level (type it)", default_name(t1)));
    }

    #[test]
    fn the_output_picker_offers_only_allowed_outputs() {
        assert_eq!(output_choices(StripId::Track(0), 16).len(), 1 + BUSES + 1);
        assert_eq!(
            output_choices(StripId::Pad(0), 16).len(),
            1 + BUSES + 16 + 1
        );
        let bus = output_choices(StripId::Bus(5), 16);
        assert_eq!(
            bus,
            vec![
                (0, 0),
                (output::bus(6) as u8, 0),
                (output::bus(7) as u8, 0),
                (output::NONE as u8, 0)
            ],
            "a submix only goes up"
        );
        let mut st = state(4);
        st.view = MixView::Pads;
        st.strips = (0..4)
            .map(|p| StripView::new(StripId::Pad(p), default_name(StripId::Pad(p))))
            .collect();
        let mut ui = ui(st);
        crate::set_look(&mut ui, Look::Sends);
        let (x, y) = mid(sends_row(&ui, 1).output);
        ui.touch(1, Touch::Down, x, y);
        assert_eq!(ui.get::<MixerView>(crate::MIXER).unwrap().picker(), None);
        ui.touch(1, Touch::Up, x, y);
        assert_eq!(
            ui.get::<MixerView>(crate::MIXER).unwrap().picker(),
            Some(StripId::Pad(1))
        );
        let r = ui.rect_of(crate::MIXER).unwrap();
        let view = ui.get::<MixerView>(crate::MIXER).unwrap();
        let (_, b) = view
            .picker_buttons(&layout(r), StripId::Pad(1))
            .into_iter()
            .find(|((o, t), _)| *o == output::TRACK as u8 && *t == 2)
            .unwrap();
        let (x, y) = mid(b);
        ui.touch(2, Touch::Down, x, y);
        ui.touch(2, Touch::Up, x, y);
        let p1 = StripId::Pad(1);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![
                MixAction::Set {
                    strip: p1,
                    id: mx::OUTPUT_TRACK,
                    value: 2.0
                },
                MixAction::Set {
                    strip: p1,
                    id: mx::OUTPUT,
                    value: output::TRACK
                },
            ]
        );
        let view = ui.get::<MixerView>(crate::MIXER).unwrap();
        assert_eq!(
            (view.picker(), view.state().strips[1].output_label()),
            (None, "Track 3".to_string())
        );
    }

    #[test]
    fn pages_views_and_selection() {
        let mut ui = ui(state(16));
        let r = ui.rect_of(crate::MIXER).unwrap();
        let l = layout(r);
        assert_eq!(
            l.per_page, 10,
            "ten strips and the master on the 1280-pixel screen"
        );
        let (x, y) = mid(l.next);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
        assert_eq!(ui.get::<MixerView>(crate::MIXER).unwrap().first(), 10);
        ui.touch(1, Touch::Down, x, y);
        assert_eq!(
            ui.get::<MixerView>(crate::MIXER).unwrap().first(),
            10,
            "no page past the end"
        );
        ui.touch(1, Touch::Up, x, y);
        let (x, y) = mid(l.chips[1]);
        ui.touch(2, Touch::Down, x, y);
        ui.touch(2, Touch::Up, x, y);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::View(MixView::Pads)]
        );
        // The app selects a strip off the page (a pad pressed): the page follows.
        let mut st = state(16);
        st.selected = Some(StripId::Track(13));
        let mut ui = self::ui(state(16));
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        crate::update(&mut ui, st);
        assert_eq!(ui.get::<MixerView>(crate::MIXER).unwrap().first(), 10);
    }

    #[test]
    fn the_effects_look_shows_each_chain_and_opens_it() {
        fn tap(ui: &mut Ui, (x, y): (i32, i32)) {
            ui.touch(1, Touch::Down, x, y);
            ui.touch(1, Touch::Up, x, y);
        }
        fn view(ui: &Ui) -> &MixerView {
            ui.get::<MixerView>(crate::MIXER).unwrap()
        }
        let mut ui = ui(state(16));
        let r = ui.rect_of(crate::MIXER).unwrap();
        let l = layout(r);
        let find = |ui: &Ui, id: &str, index: usize| {
            ui.get::<MixerView>(crate::MIXER)
                .unwrap()
                .targets(r)
                .into_iter()
                .find(|t| t.id == id && t.index == index)
                .map(|t| t.rect)
        };
        assert!(find(&ui, "fx", 0).is_none(), "Faders first");
        let [faders, _, effects, _] = look_halves(l.look);
        tap(&mut ui, mid(effects));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Look(Look::Effects)]
        );
        assert_eq!(crate::look(&ui), Look::Effects);
        assert_eq!(crate::hint(&ui), crate::MIXER_HINT_EFFECTS);
        assert!(find(&ui, "fader", 0).is_none(), "no faders on Effects");
        // Track 1's compressor, then its + FX in the next slot; Track 2's
        // + FX in its first.
        let at = mid(find(&ui, "fx", 0).expect("track 1's compressor"));
        tap(&mut ui, at);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::PickInsert {
                strip: StripId::Track(0),
                slot: 0
            }],
            "a tap picks it for the Q-Links"
        );
        assert_eq!(crate::pick(&ui), Some((StripId::Track(0), 0)));
        tap(&mut ui, at);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::OpenInsert {
                strip: StripId::Track(0),
                slot: 0
            }],
            "again: its chain"
        );
        // Its "SC from Track 2" line: its chain with the sidechain list up.
        let cell = find(&ui, "fx", 0).unwrap();
        tap(&mut ui, (cell.x + 20, cell.bottom() - 8));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::OpenSidechain {
                strip: StripId::Track(0),
                slot: 0
            }]
        );
        let add = find(&ui, "fx-add", 0).expect("track 1's + FX");
        assert_eq!(add, row_cells(grid(&l).row(0)).1[1]);
        let at = mid(find(&ui, "fx-add", 1).expect("track 2's + FX"));
        tap(&mut ui, at);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::AddInsert {
                strip: StripId::Track(1)
            }]
        );
        // An empty slot past + FX does nothing.
        tap(&mut ui, mid(row_cells(grid(&l).row(0)).1[3]));
        assert!(crate::take_actions(&mut ui).is_empty());
        // A name selects, and gives the Q-Links back to the strip; the
        // master's row is on every page.
        let at = mid(find(&ui, "fx-name", 2).unwrap());
        tap(&mut ui, at);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Select(StripId::Track(2))]
        );
        assert_eq!(crate::pick(&ui), None);
        let rows = grid(&l).rows;
        assert_eq!(rows, 8, "eight rows on the 800-pixel screen");
        let master = find(&ui, "fx-add", rows).expect("the master's + FX");
        assert_eq!(master, row_cells(grid(&l).master).1[0]);
        tap(&mut ui, mid(l.next));
        assert_eq!(view(&ui).first(), 8, "a page is the rows shown");
        assert_eq!(find(&ui, "fx-name", 0).map(|_| ()), Some(()));
        // Every look paints; the app's TRACK MIX puts Faders back quietly.
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        crate::set_look(&mut ui, Look::Faders);
        assert!(crate::take_actions(&mut ui).is_empty());
        assert_eq!(view(&ui).look(), Look::Faders);
        assert_eq!(crate::pick(&ui), None, "the pick goes with the look");
        assert!(find(&ui, "look", 0).unwrap() == faders);
        assert_eq!(crate::hint(&ui), crate::MIXER_HINT);
    }

    #[test]
    fn an_effect_dragged_along_its_row_moves() {
        let mut st = state(4);
        let fx = |name: &str| InsertView {
            name: name.into(),
            on: true,
            key: None,
            nothing_picked: false,
            cost_pct: None,
            eq: None,
        };
        st.strips[1].inserts = vec![fx("Drive"), fx("EQ"), fx("Delay")];
        let mut ui = ui(st);
        let r = ui.rect_of(crate::MIXER).unwrap();
        crate::set_look(&mut ui, Look::Effects);
        let l = layout(r);
        let cells = row_cells(grid(&l).row(1)).1;
        // Picked first: the pick moves with its effect.
        let (x, y) = mid(cells[0]);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
        assert_eq!(crate::pick(&ui), Some((StripId::Track(1), 0)));
        let _ = crate::take_actions(&mut ui);
        ui.touch(2, Touch::Down, x, y);
        let (x2, _) = mid(cells[2]);
        ui.touch(2, Touch::Move, (x + x2) / 2, y);
        ui.touch(2, Touch::Move, x2, y);
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        ui.touch(2, Touch::Up, x2, y);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::MoveInsert {
                strip: StripId::Track(1),
                from: 0,
                to: 2
            }]
        );
        let names: Vec<String> = view_of(&ui).state().strips[1]
            .inserts
            .iter()
            .map(|i| i.name.clone())
            .collect();
        assert_eq!(
            names,
            ["EQ", "Delay", "Drive"],
            "moved on the screen at once"
        );
        assert_eq!(crate::pick(&ui), Some((StripId::Track(1), 2)));
        // Past the last effect it lands last; a short wobble is a tap.
        let (x, y) = mid(cells[1]);
        ui.touch(3, Touch::Down, x, y);
        ui.touch(3, Touch::Move, x + 5, y);
        ui.touch(3, Touch::Up, x + 5, y);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::PickInsert {
                strip: StripId::Track(1),
                slot: 1
            }]
        );
        assert_eq!(moved_slot(1, 0, 2), 0);
        assert_eq!(moved_slot(0, 2, 0), 1);
        assert_eq!(moved_slot(3, 0, 2), 3);
    }

    fn view_of(ui: &Ui) -> &MixerView {
        ui.get::<MixerView>(crate::MIXER).unwrap()
    }

    #[test]
    fn a_meter_only_update_redraws_only_the_meters() {
        let mut ui = ui(state(12));
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let mut st = state(12);
        st.strips[3].peak = [0.9, 0.7];
        st.master.peak = [0.5, 0.5];
        crate::update(&mut ui, st.clone());
        let damage = ui.paint(&mut c);
        let r = ui.rect_of(crate::MIXER).unwrap();
        let area: i64 = damage.iter().map(|d| i64::from(d.w) * i64::from(d.h)).sum();
        assert!(
            area > 0 && area < i64::from(r.w) * i64::from(r.h) / 3,
            "meters only: {area} pixels"
        );
        st.strips[3].mute = true;
        crate::update(&mut ui, st);
        assert_eq!(ui.paint(&mut c), vec![r], "anything else redraws the panel");
        assert!(ui.paint(&mut c).is_empty());
    }

    #[test]
    fn a_selected_strips_name_opens_its_menu_for_freeze_and_print() {
        let mut ui = ui(state(12));
        let r = ui.rect_of(crate::MIXER).unwrap();
        let (x, y) = mid(strip_parts(&ui, 0).name);
        let tap = |ui: &mut Ui, x: i32, y: i32| {
            ui.touch(1, Touch::Down, x, y);
            ui.touch(1, Touch::Up, x, y);
        };
        tap(&mut ui, x, y);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Select(StripId::Track(0))],
            "the first tap selects"
        );
        tap(&mut ui, x, y);
        assert_eq!(
            ui.get::<MixerView>(crate::MIXER).unwrap().menu(),
            None,
            "no menu until the app offers takes"
        );
        let mut on = state(12);
        on.selected = Some(StripId::Track(0));
        on.takes = true;
        crate::update(&mut ui, on);
        tap(&mut ui, x, y);
        let v = ui.get::<MixerView>(crate::MIXER).unwrap();
        assert_eq!(
            v.menu(),
            Some(StripId::Track(0)),
            "then the second tap opens the menu"
        );
        assert!(v.menu_button(r, StripMenu::Thaw).is_none());
        let b = v.menu_button(r, StripMenu::Freeze).unwrap();
        assert!(v.menu_button(r, StripMenu::Print).is_some());
        tap(&mut ui, b.x + 10, b.y + 10);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Freeze(StripId::Track(0))]
        );
        assert_eq!(ui.get::<MixerView>(crate::MIXER).unwrap().menu(), None);

        // Under way: only Cancel. Frozen: Thaw and Print.
        let mut st = state(12);
        st.selected = Some(StripId::Track(0));
        st.takes = true;
        st.strips[0].take = Some(TakeView {
            print: false,
            done: Some(0.4),
        });
        crate::update(&mut ui, st.clone());
        tap(&mut ui, x, y);
        let v = ui.get::<MixerView>(crate::MIXER).unwrap();
        assert!(v.menu_button(r, StripMenu::Freeze).is_none());
        let b = v.menu_button(r, StripMenu::Cancel).unwrap();
        tap(&mut ui, b.x + 10, b.y + 10);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::CancelTake(StripId::Track(0))]
        );
        st.strips[0].take = None;
        st.strips[0].frozen = true;
        crate::update(&mut ui, st);
        tap(&mut ui, x, y);
        let v = ui.get::<MixerView>(crate::MIXER).unwrap();
        let b = v.menu_button(r, StripMenu::Thaw).unwrap();
        tap(&mut ui, b.x + 10, b.y + 10);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Thaw(StripId::Track(0))]
        );

        // The master prints but does not freeze; a tap outside closes.
        let l = layout(r);
        let mp = parts_with(l.master, 0, false, false);
        let (mx_, my_) = mid(mp.name);
        tap(&mut ui, mx_, my_);
        tap(&mut ui, mx_, my_);
        let v = ui.get::<MixerView>(crate::MIXER).unwrap();
        assert_eq!(v.menu(), Some(StripId::Master));
        assert!(v.menu_button(r, StripMenu::Freeze).is_none());
        let b = v.menu_button(r, StripMenu::Print).unwrap();
        let _ = b;
        tap(&mut ui, r.x + 4, r.bottom() - 4);
        assert_eq!(ui.get::<MixerView>(crate::MIXER).unwrap().menu(), None);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Select(StripId::Master)]
        );
    }

    #[test]
    fn a_takes_progress_redraws_like_a_meter() {
        let mut st = state(12);
        st.strips[2].take = Some(TakeView {
            print: true,
            done: Some(0.1),
        });
        let mut ui = ui(st.clone());
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        st.strips[2].take = Some(TakeView {
            print: true,
            done: Some(0.6),
        });
        crate::update(&mut ui, st.clone());
        let damage = ui.paint(&mut c);
        let r = ui.rect_of(crate::MIXER).unwrap();
        let area: i64 = damage.iter().map(|d| i64::from(d.w) * i64::from(d.h)).sum();
        assert!(area > 0 && area < i64::from(r.w) * i64::from(r.h) / 3);
        st.strips[2].take = None;
        st.strips[2].frozen = true;
        crate::update(&mut ui, st);
        assert_eq!(
            ui.paint(&mut c),
            vec![r],
            "the end of a take redraws the panel"
        );
    }

    #[test]
    fn every_view_paints() {
        let mut st = state(16);
        st.cost_pct = Some(0.8);
        st.selected = Some(StripId::Track(1));
        st.strips[2].audible = false;
        st.strips[3].frozen = true;
        st.strips[4].take = Some(TakeView {
            print: false,
            done: None,
        });
        st.master.take = Some(TakeView {
            print: true,
            done: Some(0.5),
        });
        st.strips[0].inserts.extend((0..5).map(|i| InsertView {
            name: format!("Fx {i}"),
            on: i % 2 == 0,
            key: None,
            nothing_picked: i == 2,
            cost_pct: Some(i as f32),
            eq: None,
        }));
        let mut ui = ui(st);
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let mut inputs = MixState {
            view: MixView::Inputs,
            ..Default::default()
        };
        inputs.strips = (0..2)
            .map(|i| StripView::new(StripId::Input(i), default_name(StripId::Input(i))))
            .collect();
        crate::update(&mut ui, inputs);
        ui.paint(&mut c);
        crate::update(
            &mut ui,
            MixState {
                view: MixView::Pads,
                ..Default::default()
            },
        );
        ui.paint(&mut c);
    }
}

#[cfg(test)]
mod io_tests {
    use super::*;
    use crate::io::{IoInput, IoKnob, IoReply, IoSection};
    use crate::state::default_name;
    use mpfree_ui::{Fonts, SoftCanvas, Ui};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// What the test section saw.
    #[derive(Default)]
    struct Seen {
        touches: Vec<(u64, Touch)>,
        inputs: Vec<IoInput>,
        shown: Vec<bool>,
        ticks: usize,
        detail: bool,
        painted: Option<Rect>,
        turns: Vec<(usize, i32)>,
    }

    struct Probe(Arc<Mutex<Seen>>);

    impl IoSection for Probe {
        fn label(&self) -> &str {
            "MIDI"
        }
        fn paint(&self, _cx: &mut PaintCx, r: Rect) {
            self.0.lock().unwrap().painted = Some(r);
        }
        fn touch(&mut self, finger: u64, phase: Touch, _x: i32, _y: i32, _r: Rect) -> IoReply {
            let mut s = self.0.lock().unwrap();
            s.touches.push((finger, phase));
            if phase == Touch::Down {
                s.detail = true;
            }
            if phase == Touch::Up {
                IoReply::Redraw
            } else {
                IoReply::Handled
            }
        }
        fn input(&mut self, ev: IoInput) -> IoReply {
            self.0.lock().unwrap().inputs.push(ev);
            IoReply::Redraw
        }
        fn tick(&mut self, _now: Duration) -> IoReply {
            self.0.lock().unwrap().ticks += 1;
            IoReply::Redraw
        }
        fn shown(&mut self, on: bool) {
            self.0.lock().unwrap().shown.push(on);
        }
        fn back(&mut self) -> bool {
            std::mem::take(&mut self.0.lock().unwrap().detail)
        }
        fn hint(&self) -> Option<&str> {
            Some("Tap a port")
        }
        fn qlinks(&self) -> Option<[IoKnob; 4]> {
            Some(std::array::from_fn(|i| IoKnob {
                label: format!("K{i}"),
                text: String::new(),
                pos: 0.0,
                enabled: true,
            }))
        }
        fn qlink_turn(&mut self, knob: usize, detents: i32, _fine: bool) -> IoReply {
            self.0.lock().unwrap().turns.push((knob, detents));
            IoReply::Redraw
        }
    }

    /// The I/O view's strips: two inputs, 14 tracks, 8 submixes.
    fn io_state() -> MixState {
        let ids = (0..2)
            .map(StripId::Input)
            .chain((0..14).map(StripId::Track))
            .chain((0..BUSES).map(StripId::Bus));
        MixState {
            view: MixView::Io,
            strips: ids.map(|s| StripView::new(s, default_name(s))).collect(),
            ..Default::default()
        }
    }

    fn ui(st: MixState) -> Ui {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        crate::add(&mut ui).unwrap();
        crate::update(&mut ui, st);
        ui.show(crate::MIXER_PAGE).unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        ui
    }

    fn tap(ui: &mut Ui, (x, y): (i32, i32)) {
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
    }

    fn mid(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn view(ui: &Ui) -> &MixerView {
        ui.get::<MixerView>(crate::MIXER).unwrap()
    }

    /// The layout's parts: the section list and the Audio rows.
    fn areas(ui: &Ui, n: usize) -> (io::IoLayout, Vec<(usize, Rect)>) {
        let l = layout(ui.rect_of(crate::MIXER).unwrap());
        let il = io::io_layout(l.strips, n);
        let rows = view(ui).io_rows(il.panel);
        (il, rows)
    }

    #[test]
    fn the_io_chip_opens_the_io_view() {
        let mut ui = ui(MixState::default());
        let l = layout(ui.rect_of(crate::MIXER).unwrap());
        assert!(
            l.chips[4].right() < l.look.x,
            "the chips clear the head's right side"
        );
        // Beside the app's Q-Link column the panel is about 1088 wide.
        for w in [1280, 1088, 1040] {
            let l = layout(Rect::new(0, 60, w, 714));
            assert!(
                l.chips[4].right() < l.look.x,
                "{w}: clear of the look toggle"
            );
            assert!(l.chips[3].right() < l.chips[4].x, "{w}");
        }
        tap(&mut ui, mid(l.chips[4]));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::View(MixView::Io)]
        );
    }

    #[test]
    fn audio_rows_select_pick_outputs_and_cycle_an_inputs_source() {
        let mut ui = ui(io_state());
        let (_, rows) = areas(&ui, 1);
        assert_eq!(rows.len(), 13, "rows on the 800-pixel screen");
        // A tap on a row selects its strip.
        let p = io::row_parts(rows[3].1);
        tap(&mut ui, mid(p.name));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Select(StripId::Track(1))]
        );
        // Its output opens the picker; a choice sets it.
        tap(&mut ui, mid(p.output));
        assert_eq!(view(&ui).picker(), Some(StripId::Track(1)));
        let r = ui.rect_of(crate::MIXER).unwrap();
        let b = view(&ui)
            .picker_button(r, (output::bus(2) as u8, 0))
            .unwrap();
        tap(&mut ui, mid(b));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Set {
                strip: StripId::Track(1),
                id: mx::OUTPUT,
                value: output::bus(2)
            }]
        );
        assert_eq!(view(&ui).state().strips[3].output_label(), "Sub 3");
        // An input's Source and Monitor cycle.
        let p = io::row_parts(rows[0].1);
        tap(&mut ui, mid(p.source));
        tap(&mut ui, mid(p.monitor));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![
                MixAction::Set {
                    strip: StripId::Input(0),
                    id: mx::SOURCE,
                    value: 2.0
                },
                MixAction::Set {
                    strip: StripId::Input(0),
                    id: mx::MONITOR,
                    value: 0.0
                },
            ]
        );
        // A track's row has no Source: the tap there selects it.
        let p = io::row_parts(rows[5].1);
        tap(&mut ui, mid(p.source));
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Select(StripId::Track(3))]
        );
    }

    #[test]
    fn the_wheel_moves_the_selection_and_minus_plus_step_its_output() {
        let mut ui = ui(io_state());
        assert_eq!(crate::io_input(&mut ui, IoInput::Wheel(1)), IoReply::Redraw);
        assert_eq!(
            crate::take_actions(&mut ui),
            vec![MixAction::Select(StripId::Input(0))]
        );
        // On down the list, past the first page: the page follows.
        crate::io_input(&mut ui, IoInput::Wheel(20));
        assert_eq!(view(&ui).state().selected, Some(StripId::Bus(4)));
        assert_eq!(view(&ui).io.top, 13);
        crate::io_input(&mut ui, IoInput::Wheel(100));
        assert_eq!(view(&ui).state().selected, Some(StripId::Bus(7)));
        let _ = crate::take_actions(&mut ui);
        // Sub 8's choices: Master, then None (no later submix).
        assert_eq!(crate::io_input(&mut ui, IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(view(&ui).state().strips[23].output_label(), "None");
        assert_eq!(crate::io_input(&mut ui, IoInput::Step(1)), IoReply::Handled);
        crate::io_input(&mut ui, IoInput::Step(-1));
        assert_eq!(view(&ui).state().strips[23].output_label(), "Master");
        assert_eq!(crate::take_actions(&mut ui).len(), 2);
        // An input steps into the tracks.
        crate::io_input(&mut ui, IoInput::Wheel(-100));
        let _ = crate::take_actions(&mut ui);
        for _ in 0..9 {
            crate::io_input(&mut ui, IoInput::Step(1));
        }
        assert_eq!(view(&ui).state().strips[0].output_label(), "Track 1");
        // The page arrows.
        crate::io_input(&mut ui, IoInput::Page(1));
        assert_eq!(view(&ui).io.top, 13);
        crate::io_input(&mut ui, IoInput::Page(1));
        assert_eq!(view(&ui).io.top, 13, "no page past the end");
        // Away from the I/O view, the wheel is not the mixer's.
        crate::update(&mut ui, MixState::default());
        assert_eq!(
            crate::io_input(&mut ui, IoInput::Wheel(1)),
            IoReply::Ignored
        );
    }

    #[test]
    fn an_outside_section_gets_its_area_fingers_keys_ticks_and_back() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mut ui = ui(io_state());
        crate::add_io_section(&mut ui, Box::new(Probe(seen.clone())));
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let (il, _) = areas(&ui, 2);
        assert_eq!(il.list.len(), 2);
        // Not showing: nothing reaches it.
        assert!(!crate::io_tick(&mut ui, Duration::ZERO));
        assert!(crate::io_qlinks(&ui).is_none());
        // Picked by the app (the tempo chip opens Sync), then back to Audio.
        assert!(crate::io_select(&mut ui, 1));
        assert_eq!(view(&ui).io_section(), 1);
        assert_eq!(crate::io_hint(&ui).as_deref(), Some("Tap a port"));
        assert!(crate::io_select(&mut ui, 0));
        assert_eq!(crate::io_hint(&ui).as_deref(), Some(crate::MIXER_HINT_IO));
        assert!(!crate::io_select(&mut ui, 2), "no such section");
        assert_eq!(seen.lock().unwrap().shown, vec![true, false]);
        seen.lock().unwrap().shown.clear();
        tap(&mut ui, mid(il.list[1]));
        assert_eq!(view(&ui).io_section(), 1);
        assert_eq!(seen.lock().unwrap().shown, vec![true]);
        ui.paint(&mut c);
        assert_eq!(seen.lock().unwrap().painted, Some(il.panel));
        // A finger down in its area comes back to it until it lifts, even
        // over the master.
        let (x, y) = mid(il.panel);
        let master = layout(ui.rect_of(crate::MIXER).unwrap()).master;
        assert!(ui.touch(7, Touch::Down, x, y).is_none());
        ui.touch(7, Touch::Move, master.x + 20, master.y + 200);
        assert!(ui
            .touch(7, Touch::Up, master.x + 20, master.y + 200)
            .is_some());
        assert_eq!(
            seen.lock().unwrap().touches,
            vec![(7, Touch::Down), (7, Touch::Move), (7, Touch::Up)]
        );
        assert_eq!(crate::take_io_changes(&mut ui), vec![0]);
        assert!(crate::take_actions(&mut ui).is_empty());
        // Keys, ticks and the Q-Links.
        crate::io_input(&mut ui, IoInput::Wheel(2));
        let l = layout(ui.rect_of(crate::MIXER).unwrap());
        tap(&mut ui, mid(l.next));
        assert_eq!(
            seen.lock().unwrap().inputs,
            vec![IoInput::Wheel(2), IoInput::Page(1)]
        );
        ui.paint(&mut c);
        assert!(crate::io_tick(&mut ui, Duration::from_millis(16)));
        let r = ui.rect_of(crate::MIXER).unwrap();
        let parts = view(&ui).changed_parts(r).unwrap();
        assert!(parts.contains(&il.panel), "a tick redraws only the panel");
        assert_eq!(crate::io_qlinks(&ui).unwrap()[2].label, "K2");
        crate::io_qlink_turn(&mut ui, 2, -3, false);
        assert_eq!(seen.lock().unwrap().turns, vec![(2, -3)]);
        // The wheel, the page arrow and the Q-Link each named it too.
        assert_eq!(crate::take_io_changes(&mut ui), vec![0, 0, 0]);
        // Back closes its detail once, then lets Back go on.
        assert!(crate::io_back(&mut ui));
        assert!(!crate::io_back(&mut ui));
        // Another view hides it.
        crate::update(&mut ui, MixState::default());
        assert_eq!(crate::io_hint(&ui), None);
        assert_eq!(seen.lock().unwrap().shown, vec![true, false]);
        assert_eq!(seen.lock().unwrap().ticks, 1);
    }

    #[test]
    fn the_io_view_paints_with_and_without_sections() {
        let mut ui = ui(io_state());
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        crate::update(
            &mut ui,
            MixState {
                view: MixView::Io,
                ..Default::default()
            },
        );
        ui.paint(&mut c);
        crate::add_io_section(&mut ui, Box::new(Probe(Default::default())));
        ui.get_mut::<MixerView>(crate::MIXER)
            .unwrap()
            .show_io_section(1);
        ui.paint(&mut c);
        let mut icon = IoIcon { open: true };
        let mut cx_ui = Ui::new(Theme::dark(), Fonts::embedded(), 64, 64);
        cx_ui.add("io", icon);
        cx_ui.add_page("p", "col(io:*)").unwrap();
        cx_ui.show("p").unwrap();
        cx_ui.paint(&mut SoftCanvas::new(64, 64));
        assert_eq!(
            icon.touch(1, Touch::Down, 0, 0, Rect::new(0, 0, 64, 64)),
            Some(EventKind::Tab(0))
        );
    }
}
