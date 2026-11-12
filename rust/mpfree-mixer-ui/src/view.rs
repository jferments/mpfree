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

use std::cell::Cell;

use mpfree_fx::ids::mixer::{self as mx, output};
use mpfree_fx::{StripId, BUSES, SILENT_DB};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Theme, Tone, Touch, Widget};

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
const ARROW_W: i32 = 52;
const PAGE_W: i32 = 120;

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
    chips: [Rect; 4],
    prev: Rect,
    next: Rect,
    page: Rect,
    cost: Rect,
    strips: Rect,
    master: Rect,
    per_page: usize,
}

fn layout(r: Rect) -> Layout {
    let (head, body) = r.take_top(HEAD_H);
    let body = body.take_top(8).1;
    let (master, rest) = body.take_right(MASTER_W);
    let strips = rest.take_left((rest.w - 12).max(0)).0;
    let per_page = ((strips.w + GAP) / (STRIP_W + GAP)).max(1) as usize;
    let ch = 36.min(head.h);
    let cy = head.y + (head.h - ch) / 2;
    let chips =
        std::array::from_fn(|i| Rect::new(head.x + i as i32 * (CHIP_W + 8), cy, CHIP_W, ch));
    let next = Rect::new(head.right() - ARROW_W, cy, ARROW_W, ch);
    let page = Rect::new(next.x - PAGE_W, cy, PAGE_W, ch);
    let prev = Rect::new(page.x - ARROW_W, cy, ARROW_W, ch);
    let cost = Rect::new(prev.x - 12 - 170, cy, 170, ch);
    Layout {
        chips,
        prev,
        next,
        page,
        cost,
        strips,
        master,
        per_page,
    }
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
    All,
}

/// The mixer panel. See the module docs.
pub struct MixerView {
    state: MixState,
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
            first: 0,
            picker: None,
            menu: None,
            grabs: Vec::new(),
            actions: Vec::new(),
            dirty: Cell::new(Dirty::All),
            per_page: Cell::new(10),
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
            self.picker = None;
        }
        if st.selected != self.state.selected {
            if let Some(i) = st
                .selected
                .and_then(|s| st.strips.iter().position(|v| v.id == s))
            {
                let n = self.per_page.get().max(1);
                if i < self.first || i >= self.first + n {
                    self.first = i / n * n;
                }
            }
        }
        self.state = st;
        self.first = self.first.min(self.state.strips.len().saturating_sub(1));
        self.mark(kind);
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
        self.actions.push(a);
    }

    fn set(&mut self, strip: StripId, id: u16, value: f32) {
        self.push(MixAction::Set { strip, id, value });
    }

    /// The strips shown and their rectangles.
    fn shown(&self, l: &Layout) -> Vec<(usize, Rect)> {
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
                self.push(MixAction::View(v));
            }
            return;
        }
        let n = self.state.strips.len();
        if l.prev.contains(x, y) {
            self.first = self.first.saturating_sub(l.per_page);
            return;
        }
        if l.next.contains(x, y) {
            if self.first + l.per_page < n {
                self.first += l.per_page;
            }
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
            } else if i == s.inserts.len() && s.inserts.len() < s.slots {
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

    fn drag(&mut self, finger: u64, x: i32, y: i32) {
        let Some(gi) = self.grabs.iter().position(|(f, _)| *f == finger) else {
            return;
        };
        match self.grabs[gi].1 {
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

    fn lift(&mut self, finger: u64, cancel: bool) {
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
            let count = c.inset_xy(14, 0).take_right(34).0;
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
                count,
                Align::Right,
                t.text_faint,
                &self.state.counts[i].to_string(),
            );
        }
        let n = self.state.strips.len();
        let shown = self.shown(l).len();
        for (b, s, live) in [
            (l.prev, "<", self.first > 0),
            (l.next, ">", self.first + shown < n),
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
        let page = if n == 0 {
            "none".to_string()
        } else {
            format!("{}-{} of {n}", self.first + 1, self.first + shown)
        };
        cx.text(
            FontId::Sans,
            t.size_small,
            l.page,
            Align::Center,
            t.text_dim,
            &page,
        );
        if let Some(c) = self.state.cost_pct {
            // The whole mixer, inserts included: against the mix and fx
            // budgets together (1 % and 10 % of a period on the MPC).
            let tone = if c <= 5.0 {
                Tone::Good
            } else if c <= 11.0 {
                Tone::Warn
            } else {
                Tone::Bad
            };
            cx.canvas.fill_round_rect(l.cost, l.cost.h / 2, t.surface);
            let dot = Rect::new(l.cost.x + 14, l.cost.y + l.cost.h / 2 - 4, 8, 8);
            cx.canvas.fill_round_rect(dot, 4, t.tone(tone));
            cx.text(
                FontId::Sans,
                t.size_small,
                l.cost.inset_xy(28, 0),
                Align::Left,
                t.text,
                &format!("Mix + FX {c:.1} %"),
            );
        }
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
                    if ins.on { t.text } else { t.text_faint },
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
                cx.canvas.stroke_round_rect(*row, 6, 1, t.outline);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    row.inset_xy(7, 0),
                    Align::Left,
                    t.text_faint,
                    "+ Insert",
                );
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
        for (i, sr) in self.shown(&l) {
            add(sr, false, &self.state.strips[i]);
        }
        add(l.master, true, &self.state.master);
        if profiling {
            v.push(l.cost);
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

/// The small mixer icon for the top bar's right edge (layout spec, section
/// 2): three sliders. A tap gives `Tab(0)`; the app opens or closes the
/// mixer. Offered to the Hello thread, which owns the bar.
#[derive(Clone, Copy, Debug, Default)]
pub struct MixerIcon {
    /// The mixer is open (the icon is lit).
    pub open: bool,
}

impl Widget for MixerIcon {
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
        let (top, h) = (b.y + 11, b.h - 22);
        for (i, knob) in [0.35f32, 0.75, 0.5].iter().enumerate() {
            let x = b.x + 12 + i as i32 * 9;
            cx.canvas
                .fill_rect(Rect::new(x, top, 2, h), ink.mix(t.surface, 0.4));
            let ky = top + ((h - 6) as f32 * (1.0 - knob)) as i32;
            cx.canvas
                .fill_round_rect(Rect::new(x - 3, ky, 8, 6), 2, ink);
        }
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
        self.per_page.set(l.per_page);
        let clip = cx.canvas.clip();
        if !clip.intersect(Rect::new(r.x, r.y, r.w, HEAD_H)).is_empty() {
            self.paint_head(cx, &l);
        }
        for (i, sr) in self.shown(&l) {
            if !clip.intersect(sr).is_empty() {
                self.paint_strip(cx, &self.state.strips[i], sr);
            }
        }
        if !clip.intersect(l.master).is_empty() {
            self.paint_strip(cx, &self.state.master, l.master);
        }
        if self.state.strips.is_empty() {
            let t = cx.theme.clone();
            let msg = match self.state.view {
                MixView::Pads => "No pad channels: every pad plays in its kit's track",
                MixView::Inputs => "No input strips",
                _ => "No strips",
            };
            cx.text(
                FontId::Sans,
                t.size_body,
                l.strips,
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

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let before = self.actions.len();
        match phase {
            Touch::Down => self.down(finger, x, y, r),
            Touch::Move => self.drag(finger, x, y),
            Touch::Up => self.lift(finger, false),
            Touch::Cancel => self.lift(finger, true),
        }
        // Paging, the picker and the copy under the finger all show at once.
        self.mark(Dirty::All);
        (self.actions.len() > before).then_some(EventKind::Select(self.actions.len()))
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        match self.dirty.get() {
            Dirty::Clean => Some(Vec::new()),
            Dirty::Meters => Some(self.meter_parts(r)),
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
