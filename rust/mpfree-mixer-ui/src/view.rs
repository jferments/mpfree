//! The mixer screen's widget: a row of channel strips with the master on
//! the right, the strips' views and pages along the top, and an output
//! picker that opens over the strips.
//!
//! Each strip, top to bottom: its name over a colour stripe, four insert
//! rows (a tap opens the slot's page, or the effect list on the empty
//! slot), sends A to D (drag to set; a tap on the letter takes it before or
//! after the fader), pan (drag; a tap centres it), the fader beside a
//! stereo meter (drag; it moves relative to where the finger lands, with a
//! detent at 0 dB), the level (a tap sets 0 dB), Mute and Solo (and Arm on
//! an input), and the output (a tap opens the picker). An input strip
//! trades its last two insert rows for its Source and Monitor.
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
use mpfree_fx::{StripId, BUSES, SILENT_DB};
use mpfree_ui::{
    Align, Color, EventKind, FontId, PaintCx, Rect, Target, Theme, Tone, Touch, Widget,
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
}

/// The Mix tab's three looks of the same strips (layout spec section
/// 3a): Faders, the strips side by side; Effects, each strip's chain as a
/// row of slots; and EQ, each strip's EQ as a curve. On Effects and EQ the
/// master's row is pinned under the strips'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    #[default]
    Faders,
    Effects,
    Eq,
}

impl Look {
    pub const ALL: [Look; 3] = [Look::Faders, Look::Effects, Look::Eq];

    pub fn label(self) -> &'static str {
        match self {
            Look::Faders => "Faders",
            Look::Effects => "Effects",
            Look::Eq => "EQ",
        }
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
/// The Faders, Effects and EQ toggle.
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

/// The toggle's parts: Faders, Effects, EQ.
fn look_halves(r: Rect) -> [Rect; 3] {
    let inner = r.inset(4);
    let w = (inner.w - 8) / 3;
    [
        Rect::new(inner.x, inner.y, w, inner.h),
        Rect::new(inner.x + w + 4, inner.y, w, inner.h),
        Rect::new(
            inner.x + 2 * (w + 4),
            inner.y,
            inner.w - 2 * (w + 4),
            inner.h,
        ),
    ]
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

/// The parts of one strip.
struct Parts {
    name: Rect,
    inserts: [Rect; 4],
    sends: [Rect; 4],
    /// The sends' area, which the master uses for Master safety.
    send_area: Rect,
    pan: Rect,
    fader: Rect,
    value: Rect,
    buttons: Rect,
    output: Rect,
}

fn parts(s: Rect) -> Parts {
    let inner = s.inset_xy(6, 6);
    let (name, rest) = inner.take_top(30);
    let rest = rest.take_top(6).1;
    let (ins, rest) = rest.take_top(4 * 24);
    let rest = rest.take_top(6).1;
    let (send_area, rest) = rest.take_top(4 * 20);
    let rest = rest.take_top(6).1;
    let (pan, rest) = rest.take_top(26);
    let rest = rest.take_top(8).1;
    let (output, rest) = rest.take_bottom(30);
    let rest = rest.take_bottom(6).1;
    let (buttons, rest) = rest.take_bottom(36);
    let rest = rest.take_bottom(6).1;
    let (value, rest) = rest.take_bottom(22);
    let fader = rest.take_bottom(6).1;
    let row = |area: Rect, i: i32, h: i32| Rect::new(area.x, area.y + i * h, area.w, h - 3);
    Parts {
        name,
        inserts: std::array::from_fn(|i| row(ins, i as i32, 24)),
        sends: std::array::from_fn(|i| row(send_area, i as i32, 20)),
        send_area,
        pan,
        fader,
        value,
        buttons,
        output,
    }
}

/// A strip's insert rows as one area (an input's first two: the last two
/// are its Source and Monitor).
fn insert_area(p: &Parts, s: &StripView) -> Rect {
    let n = if s.input.is_some() { 2 } else { 4 };
    let (a, b) = (p.inserts[0], p.inserts[n - 1]);
    Rect::new(a.x, a.y, a.w, b.bottom() - a.y)
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

/// M, S and (on an input) Arm buttons.
fn buttons(b: Rect, n: i32) -> Vec<Rect> {
    let w = (b.w - (n - 1) * 4) / n;
    (0..n)
        .map(|i| Rect::new(b.x + i * (w + 4), b.y, w, b.h))
        .collect()
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
    },
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
    /// Faders or Effects.
    look: Look,
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
    grabs: Vec<(u64, Grab)>,
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
    pub fn new() -> MixerView {
        MixerView {
            state: MixState::default(),
            look: Look::Faders,
            pick: None,
            eq_dot: None,
            eq_curves: RefCell::new(Vec::new()),
            first: 0,
            picker: None,
            menu: None,
            grabs: Vec::new(),
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
        for (_, g) in &self.grabs {
            let (id, ids): (StripId, Vec<u16>) = match *g {
                Grab::Fader { id, .. } => (id, vec![mx::VOLUME]),
                Grab::Pan { id, .. } => (id, vec![mx::PAN]),
                Grab::Send { id, r, .. } => (id, vec![mx::send(r)]),
                Grab::Fx { .. } => continue,
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

    /// The look showing: Faders or Effects.
    pub fn look(&self) -> Look {
        self.look
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
            Look::Effects | Look::Eq => grid(l).rows.min(l.per_page.max(1)),
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
        let mut hits: Vec<(StripId, Rect)> = self
            .shown(&l)
            .into_iter()
            .map(|(i, sr)| (self.state.strips[i].id, sr))
            .collect();
        hits.push((StripId::Master, l.master));
        let Some((id, sr)) = hits.into_iter().find(|(_, sr)| sr.contains(x, y)) else {
            return;
        };
        let p = parts(sr);
        let Some(s) = self.state.strip(id).cloned() else {
            return;
        };
        if p.name.contains(x, y) {
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
        for (i, row) in p.inserts.iter().enumerate() {
            if !row.contains(x, y) {
                continue;
            }
            if let Some(inp) = s.input.filter(|_| i >= 2) {
                if i == 2 {
                    self.set(id, mx::SOURCE, f32::from((inp.source + 1) % 4));
                } else {
                    self.set(id, mx::MONITOR, f32::from((inp.monitor + 1) % 3));
                }
            } else if i == 3 && s.inserts.len() > 4 {
                // "+N more": the strip's first hidden slot.
                self.push(MixAction::OpenInsert { strip: id, slot: 3 });
            } else if i < s.inserts.len() {
                self.push(MixAction::OpenInsert { strip: id, slot: i });
            } else if s.inserts.len() < s.slots {
                // Every empty row is one "+ FX" button.
                self.push(MixAction::AddInsert { strip: id });
            }
            return;
        }
        if let Some(sends) = s.sends {
            for (k, row) in p.sends.iter().enumerate() {
                if !row.contains(x, y) {
                    continue;
                }
                let (letter, bar) = row.take_left(18);
                if letter.contains(x, y) {
                    self.set(id, mx::send_pre(k), if sends[k].pre { 0.0 } else { 1.0 });
                } else {
                    let w = (bar.w - 34).max(1) as f32;
                    self.grabs.push((
                        finger,
                        Grab::Send {
                            id,
                            r: k,
                            x0: x,
                            pos0: db_to_pos(sends[k].db),
                            w,
                        },
                    ));
                }
                return;
            }
        }
        if p.pan.contains(x, y) {
            self.grabs.push((
                finger,
                Grab::Pan {
                    id,
                    x0: x,
                    v0: s.pan,
                    w: p.pan.w.max(1) as f32,
                    moved: false,
                },
            ));
            return;
        }
        if p.fader.contains(x, y) {
            let (top, bot) = travel(p.fader);
            self.grabs.push((
                finger,
                Grab::Fader {
                    id,
                    y0: y,
                    pos0: db_to_pos(s.volume_db),
                    travel: (bot - top).max(1) as f32,
                },
            ));
            return;
        }
        if p.value.contains(x, y) {
            self.set(id, mx::VOLUME, 0.0);
            return;
        }
        if p.buttons.contains(x, y) && id != StripId::Master {
            let n = if s.input.is_some() { 3 } else { 2 };
            let b = buttons(p.buttons, n);
            if b[0].contains(x, y) {
                self.set(id, mx::MUTE, if s.mute { 0.0 } else { 1.0 });
            } else if b[1].contains(x, y) {
                self.set(id, mx::SOLO, if s.solo { 0.0 } else { 1.0 });
            } else if n == 3 && b[2].contains(x, y) {
                let arm = s.input.is_some_and(|i| i.arm);
                self.set(id, mx::ARM, if arm { 0.0 } else { 1.0 });
            }
            return;
        }
        if p.output.contains(x, y) && s.has_output() {
            self.picker = Some(id);
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
                self.grabs.push((
                    finger,
                    Grab::Fx {
                        id,
                        from: k,
                        x0: x,
                        to: None,
                    },
                ));
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
        self.grabs.push((
            finger,
            Grab::Eq {
                id,
                slot,
                dot,
                x0: x,
                y0: y,
                moved: false,
            },
        ));
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
        let Some(gi) = self.grabs.iter().position(|(f, _)| *f == finger) else {
            return;
        };
        match self.grabs[gi].1 {
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
                    self.grabs[gi].1 = Grab::Eq {
                        id,
                        slot,
                        dot: Some(dot),
                        x0,
                        y0,
                        moved: true,
                    };
                    self.eq_pick(id, slot, Some(dot));
                }
                self.eq_drag(id, slot, dot, x, y, r);
            }
            Grab::Eq { .. } => {}
            Grab::Fx { id, from, x0, to } => {
                if to.is_none() && (x - x0).abs() < 12 {
                    return;
                }
                let at = self.fx_slot_at(id, x, r).or(Some(from));
                self.grabs[gi].1 = Grab::Fx {
                    id,
                    from,
                    x0,
                    to: at,
                };
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
                self.grabs[gi].1 = Grab::Pan {
                    id,
                    x0,
                    v0,
                    w,
                    moved: true,
                };
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
        let Some(gi) = self.grabs.iter().position(|(f, _)| *f == finger) else {
            return;
        };
        let (_, g) = self.grabs.remove(gi);
        if let Grab::Pan {
            id, moved: false, ..
        } = g
        {
            if !cancel {
                self.set(id, mx::PAN, 0.0);
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
        if let Grab::Fx { id, from, to, .. } = g {
            match to {
                _ if cancel => {}
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
                .fill_round_rect(c, 10, if on { t.raised } else { t.surface });
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
        let (first, shown, n, outside) = self.span(l);
        for (b, s, live) in [
            (l.prev, "<", outside || first > 0),
            (l.next, ">", outside || first + shown < n),
        ] {
            cx.canvas
                .fill_round_rect(b, 10, if live { t.raised } else { t.surface });
            cx.text(
                FontId::Bold,
                t.size_title,
                b,
                Align::Center,
                if live { t.text } else { t.text_faint },
                s,
            );
        }
        let page = if outside {
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
        cx.canvas.fill_round_rect(l.look, 12, t.surface);
        for (half, look) in look_halves(l.look).into_iter().zip(Look::ALL) {
            let on = look == self.look;
            if on {
                cx.canvas.fill_round_rect(half, 8, t.accent);
            }
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_small,
                half,
                Align::Center,
                if on { t.on_accent } else { t.text_dim },
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
        let p = parts(sr);
        let master = s.id == StripId::Master;
        let selected = self.state.selected == Some(s.id);
        cx.canvas.fill_round_rect(sr, 12, t.surface);
        if selected {
            cx.canvas.stroke_round_rect(sr, 12, 2, t.accent);
        }
        let ink = if s.audible { t.text } else { t.text_faint };
        // Name over a colour stripe.
        let color = strip_color(&t, s.id);
        cx.canvas.fill_round_rect(
            Rect::new(p.name.x, p.name.y, p.name.w, 4),
            2,
            if s.audible {
                color
            } else {
                color.mix(t.surface, 0.6)
            },
        );
        let label = p.name.take_top(6).1;
        cx.text(FontId::Bold, t.size_small, label, Align::Left, ink, &s.name);
        if master {
            self.paint_cost(cx, label, false);
        }
        // Inserts (an input's last two rows are its Source and Monitor).
        for (i, row) in p.inserts.iter().enumerate() {
            if let Some(inp) = s.input.filter(|_| i >= 2) {
                let (k, v) = if i == 2 {
                    ("Src", choice(mx::SOURCE, inp.source))
                } else {
                    ("Mon", choice(mx::MONITOR, inp.monitor))
                };
                cx.canvas.fill_round_rect(*row, 6, t.raised);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    row.inset_xy(7, 0),
                    Align::Left,
                    t.text_dim,
                    k,
                );
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    row.inset_xy(7, 0),
                    Align::Right,
                    t.text,
                    v,
                );
                continue;
            }
            let more = s.inserts.len() > 4 && i == 3;
            if more {
                cx.canvas.stroke_round_rect(*row, 6, 1, t.outline);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    row.inset_xy(7, 0),
                    Align::Left,
                    t.text_dim,
                    &format!("+{} more", s.inserts.len() - 3),
                );
            } else if let Some(ins) = s.inserts.get(i) {
                cx.canvas.fill_round_rect(
                    *row,
                    6,
                    if ins.on {
                        t.raised
                    } else {
                        t.surface.mix(t.raised, 0.4)
                    },
                );
                if let Some(c) = ins.cost_pct {
                    let w = ((row.w as f32) * (c / 10.0).clamp(0.0, 1.0)) as i32;
                    cx.canvas.fill_round_rect(
                        Rect::new(row.x, row.bottom() - 3, w.max(2), 3),
                        1,
                        if c <= 2.0 {
                            t.good
                        } else if c <= 5.0 {
                            t.warn
                        } else {
                            t.bad
                        },
                    );
                }
                let inner = row.inset_xy(7, 0);
                let (tag, name_r) = if ins.key.is_some() {
                    inner.take_right(22)
                } else {
                    (Rect::default(), inner)
                };
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    name_r,
                    Align::Left,
                    t.button_ink(ins.on),
                    &ins.name,
                );
                if ins.key.is_some() {
                    // Keyed by another strip: a sidechain.
                    cx.text(
                        FontId::Bold,
                        t.size_small - 3.0,
                        tag,
                        Align::Right,
                        t.accent,
                        "SC",
                    );
                }
            } else if i == s.inserts.len() && s.inserts.len() < s.slots {
                // The empty rows as one "+ FX" button (Jay after live
                // test 3: "+ Insert" was too small and unclear).
                let last = if s.input.is_some() { 1 } else { 3 };
                let b = Rect::new(row.x, row.y, row.w, p.inserts[last].bottom() - row.y);
                cx.canvas.fill_round_rect(b, 6, t.raised);
                cx.canvas.stroke_round_rect(b, 6, 1, t.outline);
                cx.text(FontId::Bold, t.size_small, b, Align::Center, t.text, "+ FX");
            }
        }
        self.paint_freeze(cx, s, insert_area(&p, s));
        // Sends, or Master safety on the master.
        if master {
            self.paint_safety(cx, p.send_area);
        } else if let Some(sends) = s.sends {
            for (k, row) in p.sends.iter().enumerate() {
                let (letter, rest) = row.take_left(18);
                let l = ["A", "B", "C", "D"][k];
                if sends[k].pre {
                    cx.canvas.fill_round_rect(letter.inset_xy(1, 1), 4, t.warn);
                    cx.text(
                        FontId::Bold,
                        t.size_small - 2.0,
                        letter,
                        Align::Center,
                        t.bg,
                        l,
                    );
                } else {
                    cx.text(
                        FontId::Bold,
                        t.size_small - 2.0,
                        letter,
                        Align::Center,
                        t.text_dim,
                        l,
                    );
                }
                let (val, bar) = rest.take_right(34);
                let track = Rect::new(bar.x + 3, bar.y + bar.h / 2 - 3, bar.w - 6, 6);
                cx.canvas.fill_round_rect(track, 3, t.raised);
                let pos = db_to_pos(sends[k].db);
                if pos > 0.0 {
                    cx.canvas.fill_round_rect(
                        Rect::new(track.x, track.y, ((track.w as f32) * pos) as i32, track.h),
                        3,
                        if s.audible { t.accent } else { t.text_faint },
                    );
                }
                let v = if sends[k].db <= SILENT_DB {
                    "off".to_string()
                } else {
                    format!("{:.0}", sends[k].db)
                };
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    val,
                    Align::Right,
                    if sends[k].db <= SILENT_DB {
                        t.text_faint
                    } else {
                        t.text_dim
                    },
                    &v,
                );
            }
        } else {
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                p.send_area.take_top(20).0,
                Align::Left,
                t.text_faint,
                "No sends",
            );
        }
        // Pan.
        cx.canvas.fill_round_rect(p.pan, 7, t.raised);
        let cxm = p.pan.x + p.pan.w / 2;
        let px = cxm + ((p.pan.w / 2 - 4) as f32 * (s.pan / 100.0)) as i32;
        let (a, b) = if px < cxm { (px, cxm) } else { (cxm, px) };
        if b > a {
            cx.canvas.fill_rect(
                Rect::new(a, p.pan.y + 3, b - a, p.pan.h - 6),
                t.accent.mix(t.raised, 0.45),
            );
        }
        cx.canvas
            .fill_rect(Rect::new(cxm, p.pan.y + 3, 1, p.pan.h - 6), t.outline);
        cx.canvas
            .fill_rect(Rect::new(px - 1, p.pan.y + 2, 3, p.pan.h - 4), ink);
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            p.pan.inset_xy(6, 0),
            Align::Left,
            t.text_faint,
            "Pan",
        );
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            p.pan.inset_xy(6, 0),
            Align::Right,
            ink,
            &pan_text(s.pan),
        );
        // Fader and meter.
        self.paint_fader(cx, s, p.fader, master);
        cx.text(
            FontId::Bold,
            t.size_small,
            p.value,
            Align::Center,
            ink,
            &db_text(s.volume_db),
        );
        // Mute, solo, arm; the master shows its limiting.
        if master {
            let red = self.state.safety.reduction_db;
            let tone = if red < -6.0 { t.warn } else { t.text_dim };
            cx.canvas.fill_round_rect(p.buttons, 8, t.raised);
            cx.text(
                FontId::Sans,
                t.size_small,
                p.buttons.inset_xy(8, 0),
                Align::Left,
                t.text_dim,
                "Limit",
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                p.buttons.inset_xy(8, 0),
                Align::Right,
                tone,
                &format!("{red:.1} dB"),
            );
        } else {
            let arm = s.input.map(|i| i.arm);
            let n = if arm.is_some() { 3 } else { 2 };
            let bs = buttons(p.buttons, n);
            let spec = [
                ("M", s.mute, t.warn),
                ("S", s.solo, t.accent),
                ("R", arm.unwrap_or(false), t.bad),
            ];
            for (i, b) in bs.iter().enumerate() {
                let (txt, on, c) = spec[i];
                cx.canvas
                    .fill_round_rect(*b, 8, if on { c } else { t.raised });
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    *b,
                    Align::Center,
                    if on { t.bg } else { t.text_dim },
                    txt,
                );
            }
        }
        // Output.
        if s.has_output() {
            let open = self.picker == Some(s.id);
            cx.canvas
                .fill_round_rect(p.output, 8, if open { t.accent } else { t.raised });
            let none = s.output == output::NONE as u8;
            let c = if open {
                t.bg
            } else if none {
                t.warn
            } else {
                t.text
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                p.output,
                Align::Center,
                c,
                &s.output_label(),
            );
        } else {
            let to = if master { "Out 1+2" } else { "Master" };
            cx.text(
                FontId::Sans,
                t.size_small,
                p.output,
                Align::Center,
                t.text_faint,
                to,
            );
        }
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
                    10,
                    if ins.on {
                        t.raised
                    } else {
                        t.surface.mix(t.raised, 0.4)
                    },
                );
                let dot = Rect::new(c.right() - 16, c.y + 9, 8, 8);
                let tone = if !ins.on {
                    t.text_faint
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
                    (None, true) => None,
                };
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
                        FontId::Sans,
                        t.size_small - 4.0,
                        Rect::new(b.x, b.y, c.w - 16, b.h),
                        Align::Left,
                        t.text_dim,
                        &line,
                    );
                }
                if self.pick == Some((s.id, k)) && drop.is_none() {
                    cx.canvas.stroke_round_rect(c, 10, 2, t.accent);
                }
                match drop {
                    Some((from, _)) if from == k => {
                        cx.canvas.stroke_round_rect(c, 10, 1, t.text_dim);
                    }
                    Some((_, to)) if to == k => {
                        cx.canvas.stroke_round_rect(c, 10, 2, t.accent);
                    }
                    _ => {}
                }
            } else if k == s.inserts.len() {
                cx.canvas.fill_round_rect(c, 10, t.raised);
                cx.text(FontId::Bold, t.size_small, c, Align::Center, t.text, "+ FX");
            } else {
                cx.canvas
                    .stroke_round_rect(c, 10, 1, t.outline.mix(t.surface, 0.5));
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
                .stroke_round_rect(graph, 8, 1, t.outline.mix(t.surface, 0.3));
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
                cx.canvas.fill_round_rect(b, 8, t.raised);
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
            .fill_round_rect(graph, 8, t.bg.mix(t.surface, 0.4));
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
                d.color()
            } else {
                t.text_faint
            };
            m.draw(cx.canvas, ink);
        }
        cx.canvas.set_clip(clip);
        if picked {
            cx.canvas.stroke_round_rect(graph, 8, 2, t.accent);
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
            cx.canvas.fill_round_rect(name, 10, t.raised);
            cx.canvas.stroke_round_rect(name, 10, 2, t.accent);
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
            .fill_round_rect(area, 8, t.raised.mix(t.surface, 0.5));
        let rows = area.inset_xy(8, 4);
        let h = rows.h / 4;
        let row = |i: i32| Rect::new(rows.x, rows.y + i * h, rows.w, h);
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
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            row(2),
            Align::Left,
            t.text_dim,
            &format!("Ceiling {:.1} dB", sf.ceiling_db),
        );
        let tone = if sf.overs > 0 { t.warn } else { t.text_dim };
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            row(3),
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
                .fill_round_rect(b, 10, if on { t.accent } else { t.raised });
            cx.text(
                FontId::Sans,
                t.size_body,
                b,
                Align::Center,
                if on { t.bg } else { t.text },
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
            cx.canvas.fill_round_rect(b, 10, t.raised);
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
            .fill_round_rect(area, 8, t.surface.mix(t.accent, 0.12));
        cx.canvas.stroke_round_rect(area, 8, 1, t.accent);
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
        let mut add = |sr: Rect, master: bool, s: &StripView| {
            let p = parts(sr);
            v.push(fader_columns(p.fader, master).2);
            if s.take.is_some() {
                // A take's progress.
                v.push(insert_area(&p, s));
            }
            if master {
                v.push(p.send_area);
                v.push(p.buttons);
            }
            if profiling {
                v.extend(p.inserts);
            }
        };
        if self.look == Look::Effects && self.state.view != MixView::Io {
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
                v.push(parts(l.master).name);
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
    fn span(&self, l: &Layout) -> (usize, usize, usize, bool) {
        let n = self.state.strips.len();
        if self.state.view != MixView::Io {
            let shown = match self.look {
                Look::Faders => self.shown(l).len(),
                Look::Effects | Look::Eq => self.rows(l).len(),
            };
            return (self.first, shown, n, false);
        }
        if self.io.section().is_some() {
            return (0, 0, 0, true);
        }
        let rows = io::rows_per_page(self.io_panel(l));
        let top = self.io.top.min(n);
        (top, rows.min(n - top), n, false)
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
                .fill_round_rect(*b, 10, if on { t.raised } else { t.surface });
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
            cx.canvas.fill_round_rect(row, 10, t.surface);
            if selected {
                cx.canvas.stroke_round_rect(row, 10, 2, t.accent);
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
            cx.canvas
                .fill_round_rect(p.output, 8, if open { t.accent } else { t.raised });
            cx.text(
                FontId::Sans,
                t.size_body,
                p.output.inset_xy(12, 0),
                Align::Left,
                if open {
                    t.bg
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
                    cx.canvas.fill_round_rect(r, 8, t.raised);
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
            .fill_round_rect(b, 10, if self.open { t.raised } else { t.surface });
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
        let effects = self.look != Look::Faders && self.state.view != MixView::Io;
        if effects {
            if !clip.intersect(grid(&l).panel).is_empty() {
                if self.look == Look::Eq {
                    self.paint_eq(cx, &l);
                } else {
                    self.paint_effects(cx, &l);
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
        let (first, shown, n, outside) = self.span(&l);
        out.push(Target::tap("prev", 0, "<", l.prev).disabled_if(!(outside || first > 0)));
        out.push(Target::tap("next", 0, ">", l.next).disabled_if(!(outside || first + shown < n)));
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
            let p = parts(sr);
            let name = &s.name;
            // The selected strip's name opens its menu only with takes.
            let picked = self.state.selected == Some(s.id);
            out.push(
                Target::tap("strip-name", k, name.clone(), p.name)
                    .disabled_if(picked && !self.state.takes),
            );
            let rows = if s.input.is_some() { 2 } else { 4 };
            for (i, row) in p.inserts.iter().enumerate() {
                if let Some(_inp) = s.input.filter(|_| i >= 2) {
                    let id = if i == 2 { "source" } else { "monitor" };
                    out.push(Target::tap(id, k, format!("{name} {id}"), *row));
                } else if i < rows && i < s.inserts.len().min(4) {
                    let label = s.inserts.get(i).map_or(String::new(), |v| v.name.clone());
                    out.push(Target::tap("insert", k * 4 + i, label, *row));
                } else if i < rows && i == s.inserts.len() && s.inserts.len() < s.slots {
                    out.push(Target::tap(
                        "add-insert",
                        k,
                        format!("{name} + Insert"),
                        *row,
                    ));
                }
            }
            if s.sends.is_some() {
                for (j, row) in p.sends.iter().enumerate() {
                    let (letter, bar) = row.take_left(18);
                    let send = ["A", "B", "C", "D"][j.min(3)];
                    out.push(Target::tap(
                        "send-pre",
                        k * 4 + j,
                        format!("{name} send {send} pre/post"),
                        letter,
                    ));
                    out.push(
                        Target::drag("send", k * 4 + j, format!("{name} send {send}"), bar)
                            .towards(40, 0),
                    );
                }
            }
            out.push(Target::drag("pan", k, format!("{name} pan"), p.pan).towards(40, 0));
            out.push(Target::drag("fader", k, format!("{name} fader"), p.fader).towards(0, -40));
            out.push(Target::tap(
                "value",
                k,
                format!("{name} level (to 0 dB)"),
                p.value,
            ));
            if s.id != StripId::Master {
                let n = if s.input.is_some() { 3 } else { 2 };
                for (j, b) in buttons(p.buttons, n).into_iter().enumerate() {
                    let id = ["mute", "solo", "arm"][j];
                    out.push(Target::tap(
                        id,
                        k,
                        format!("{name} {}", ["M", "S", "Arm"][j]),
                        b,
                    ));
                }
            }
            if s.has_output() {
                out.push(Target::tap("output", k, format!("{name} output"), p.output));
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
        let l = layout(r);
        let mut strips: Vec<StripId> = self
            .shown(&l)
            .into_iter()
            .map(|(i, _)| self.state.strips[i].id)
            .collect();
        strips.push(self.state.master.id);
        let (k, j) = match t.id {
            "send" => (t.index / 4, t.index % 4),
            _ => (t.index, 0),
        };
        let strip = *strips.get(k)?;
        let s = self.state.strip(strip)?;
        let (id, from, to) = match t.id {
            "fader" => (
                mx::VOLUME,
                s.volume_db,
                crate::qlinks::level_turn(s.volume_db, delta, false),
            ),
            "pan" => (mx::PAN, s.pan, crate::qlinks::pan_turn(s.pan, delta, false)),
            "send" => {
                let db = s.sends?.get(j)?.db;
                (mx::send(j), db, crate::qlinks::level_turn(db, delta, false))
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
    use mpfree_fx::RETURNS;
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
        let r = ui.rect_of(crate::MIXER).unwrap();
        let l = layout(r);
        parts(Rect::new(
            l.strips.x + col as i32 * (STRIP_W + GAP),
            l.strips.y,
            STRIP_W,
            l.strips.h,
        ))
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
        let (x, y) = mid(p.pan);
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
        let p = strip_parts(&ui, 0);
        let (x, y) = mid(p.sends[1].take_left(18).1);
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
        let (x, y) = mid(buttons(p.buttons, 2)[0]);
        assert_eq!(ui.focus_at(x, y).unwrap().id, "mute");
        assert!(!ui.focus_steps());
    }

    #[test]
    fn taps_mute_solo_centre_reset_and_open_inserts() {
        let mut ui = ui(state(12));
        let p = strip_parts(&ui, 0);
        let b = buttons(p.buttons, 2);
        let tap = |ui: &mut Ui, (x, y): (i32, i32)| {
            ui.touch(9, Touch::Down, x, y);
            ui.touch(9, Touch::Up, x, y);
        };
        tap(&mut ui, mid(b[0]));
        tap(&mut ui, mid(b[1]));
        tap(&mut ui, mid(p.value));
        tap(&mut ui, mid(p.pan));
        tap(&mut ui, mid(p.inserts[0]));
        tap(&mut ui, mid(p.inserts[1]));
        // Every empty row is part of the one "+ FX" button.
        tap(&mut ui, mid(p.inserts[3]));
        tap(&mut ui, mid(p.sends[1].take_left(18).0));
        tap(&mut ui, mid(p.name));
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
                MixAction::Set {
                    strip: t0,
                    id: mx::VOLUME,
                    value: 0.0
                },
                MixAction::Set {
                    strip: t0,
                    id: mx::PAN,
                    value: 0.0
                },
                MixAction::OpenInsert { strip: t0, slot: 0 },
                MixAction::AddInsert { strip: t0 },
                MixAction::AddInsert { strip: t0 },
                MixAction::Set {
                    strip: t0,
                    id: mx::send_pre(1),
                    value: 0.0
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
        let p = strip_parts(&ui, 1);
        let (x, y) = mid(p.output);
        ui.touch(1, Touch::Down, x, y);
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
        let [faders, effects, _] = look_halves(l.look);
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
        let mp = parts(l.master);
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
