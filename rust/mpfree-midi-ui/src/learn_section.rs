//! The Learn section (board 8 of the MIDI mockups, approved by the owner
//! on 16 Nov 2026; its MIDI Follow card comes later): what your
//! controllers move, on one page.
//!
//! - **The list:** each learned control (its device, what on it, and the
//!   channel when it isn't 1), what it moves, where that stands now, and
//!   where it is kept: **Project** (this song) or **Rig** (every project,
//!   [`MidiSetup::rig_learn`]). A row that does nothing says why: its
//!   device isn't plugged in, its port only plays tracks, or the song has
//!   its own mapping for the same control.
//! - **The card** under the list, for the picked row: **Learn again**
//!   (the next control moved takes its place, keeping its range, takeover
//!   and where it is kept: [`Asks::learn_again`]), **Remove**, the
//!   **Range** (each end in the parameter's units, from the app:
//!   [`LearnLive`]), **Works as** (a knob or fader, an endless encoder in
//!   one of three encodings, a button), **Takeover** (Jump, Pick up,
//!   Scale; for knobs and faders) and **Kept in** (Rig, Project).
//! - **Learning:** while MIDI Learn waits for a control ([`Live::learning`]),
//!   a banner names what it will take, with **Cancel** ([`Asks::cancel_learn`]).
//! - **Transport and tempo** (the button by the heading, and the card
//!   while nothing is learned yet): PLAY and STOP, PLAY, STOP, PLAY START,
//!   REC, OVERDUB, TAP TEMPO and Tempo. A tap arms Learn on one
//!   ([`Asks::learn_target`]); the control that takes it works in every
//!   project.
//!
//! The wheel picks a row, - and + step Works as, and Q-Links 1 to 4 turn
//! the range's two ends, Works as and Takeover.
//!
//! [`MidiSetup::rig_learn`]: mpfree_midi::setup::MidiSetup::rig_learn
//! [`Asks::learn_again`]: crate::Asks::learn_again
//! [`Asks::cancel_learn`]: crate::Asks::cancel_learn
//! [`Asks::learn_target`]: crate::Asks::learn_target
//! [`Live::learning`]: crate::Live::learning
//! [`LearnLive`]: crate::LearnLive

use std::cell::Cell;

use mpfree_midi::learn::{Encoding, Mode, Saved, Source, Takeover};
use mpfree_midi::setup::{Kept, PortInput, DIN_NAME};
use mpfree_mixer_ui::{IoInput, IoKnob, IoReply, IoSection};
use mpfree_param::{transport, Owner, ParamAddr};
use mpfree_ui::{Align, FontId, PaintCx, Rect, Target, Touch};

use crate::{note_name, IoModel, Shared};

const HEAD_H: i32 = 42;
const BANNER_H: i32 = 48;
const GAP: i32 = 10;
const TABLE_HEAD_H: i32 = 28;
const ROW_H: i32 = 40;
const CARD_H: i32 = 244;
const BUTTON_W: i32 = 130;
const BUTTON_H: i32 = 34;
const STEP_W: i32 = 40;
const LABEL_W: i32 = 110;
const LINE_H: i32 = 40;
const SEG_W: i32 = 104;
/// The heading's Transport and tempo button.
const TRANSPORT_W: i32 = 190;
/// The transport card's buttons: four a row.
const PICK_H: i32 = 44;
/// A range end's step, a tap or a Q-Link detent (SHIFT: a tenth).
const RANGE_STEP: f32 = 0.01;

pub(crate) const HINT: &str = "Tap a row to edit it  ·  wheel: pick  ·  - and +: works as  ·  Q-Links: range, works as, takeover";

/// How a learned control can move its parameter, in the order Works as
/// steps through them.
pub(crate) const MODES: [(Mode, &str); 6] = [
    (Mode::Absolute, "Knob or fader"),
    (
        Mode::Relative(Encoding::TwosComplement),
        "Endless (1 up, 127 down)",
    ),
    (
        Mode::Relative(Encoding::Offset64),
        "Endless (65 up, 63 down)",
    ),
    (Mode::Relative(Encoding::SignBit), "Endless (1 up, 65 down)"),
    (Mode::Toggle, "Button: on, then off"),
    (Mode::Momentary, "Button: while held"),
];

const TAKEOVERS: [(Takeover, &str, &str); 3] = [
    (
        Takeover::Jump,
        "Jump",
        "Jump: the value goes straight to where the knob is.",
    ),
    (
        Takeover::Pickup,
        "Pick up",
        "Pick up: the value waits until the knob passes it, so nothing jumps.",
    ),
    (
        Takeover::Scale,
        "Scale",
        "Scale: the value moves at once, in step with the knob, until they meet.",
    ),
];

const KEPT: [(Kept, &str); 2] = [(Kept::Rig, "Rig"), (Kept::Project, "Project")];

/// What a tap hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Row(usize),
    Cancel,
    Again,
    Remove,
    /// The range's low (0) or high (1) end, down or up.
    Range(usize, i32),
    Mode(i32),
    Takeover(usize),
    Kept(usize),
    /// The heading's Transport and tempo: the card shows them, or the
    /// picked row again.
    Transport,
    /// The transport card's Back to the picked row.
    Back,
    /// One of the transport card's buttons ([`transport::ALL`]).
    Pick(usize),
}

struct Layout {
    banner: Option<Rect>,
    cancel: Rect,
    head: Rect,
    /// The heading's Transport and tempo button.
    transport: Rect,
    table: Rect,
    /// The rows' area, under the table's column heads.
    body: Rect,
    card: Rect,
    again: Rect,
    remove: Rect,
    /// Each line of the card: Range, Works as, Takeover, Kept in.
    line: [Rect; 4],
}

fn layout(r: Rect, learning: bool) -> Layout {
    let mut y = r.y;
    let banner = learning.then(|| {
        let b = Rect::new(r.x, y, r.w, BANNER_H);
        y += BANNER_H + GAP;
        b
    });
    let cancel = banner.map_or(Rect::new(0, 0, 0, 0), |b| {
        Rect::new(b.right() - 10 - 100, b.y + 7, 100, b.h - 14)
    });
    let head = Rect::new(r.x + 4, y, r.w - 8, HEAD_H - 8);
    let transport = Rect::new(head.right() - TRANSPORT_W, head.y, TRANSPORT_W, head.h);
    y += HEAD_H;
    let card = Rect::new(r.x, r.bottom() - CARD_H, r.w, CARD_H);
    let table = Rect::new(r.x, y, r.w, (card.y - GAP - y).max(0));
    let inner = table.inset(10);
    let body = Rect::new(
        inner.x,
        inner.y + TABLE_HEAD_H,
        inner.w,
        (inner.h - TABLE_HEAD_H).max(0),
    );
    let ci = card.inset(14);
    let remove = Rect::new(ci.right() - BUTTON_W, ci.y, BUTTON_W, BUTTON_H);
    let again = Rect::new(remove.x - 8 - BUTTON_W, ci.y, BUTTON_W, BUTTON_H);
    let line = std::array::from_fn(|i| {
        Rect::new(
            ci.x,
            ci.y + BUTTON_H + 10 + i as i32 * LINE_H,
            ci.w,
            LINE_H - 6,
        )
    });
    Layout {
        banner,
        cancel,
        head,
        transport,
        table,
        body,
        card,
        again,
        remove,
        line,
    }
}

/// The transport card's buttons, in [`transport::ALL`]'s order: two rows
/// of four under the card's title and its line of words.
fn pick_rects(l: &Layout) -> [Rect; 8] {
    let ci = l.card.inset(14);
    let w = (ci.w - 3 * 8) / 4;
    let y = ci.y + BUTTON_H + 34;
    std::array::from_fn(|i| {
        let (row, col) = ((i / 4) as i32, (i % 4) as i32);
        Rect::new(ci.x + col * (w + 8), y + row * (PICK_H + 8), w, PICK_H)
    })
}

/// The address a transport card button arms.
fn pick_target(i: usize) -> ParamAddr {
    ParamAddr::new(Owner::Transport, transport::ALL[i].0)
}

/// The table's columns: Control, Moves, Now, Kept in (x and width).
fn columns(body: Rect) -> [(i32, i32); 4] {
    let kept = 96;
    let now = 140;
    let rest = body.w - kept - now;
    let control = rest * 45 / 100;
    let moves = rest - control;
    [
        (body.x, control),
        (body.x + control, moves),
        (body.x + control + moves, now),
        (body.x + control + moves + now, kept),
    ]
}

/// The steppers and segments on a card line.
fn range_rects(line: Rect) -> [Rect; 4] {
    let x = line.x + LABEL_W;
    let w = 190;
    let lo = Rect::new(x, line.y, w, line.h);
    let hi = Rect::new(x + w + 36, line.y, w, line.h);
    [
        Rect::new(lo.x, lo.y, STEP_W, lo.h),
        Rect::new(lo.right() - STEP_W, lo.y, STEP_W, lo.h),
        Rect::new(hi.x, hi.y, STEP_W, hi.h),
        Rect::new(hi.right() - STEP_W, hi.y, STEP_W, hi.h),
    ]
}

fn mode_rects(line: Rect) -> [Rect; 2] {
    let x = line.x + LABEL_W;
    let w = 300;
    [
        Rect::new(x, line.y, STEP_W, line.h),
        Rect::new(x + w - STEP_W, line.y, STEP_W, line.h),
    ]
}

fn segments(line: Rect, n: usize) -> Vec<Rect> {
    (0..n)
        .map(|i| {
            Rect::new(
                line.x + LABEL_W + i as i32 * (SEG_W + 6),
                line.y,
                SEG_W,
                line.h,
            )
        })
        .collect()
}

/// "MIDI in", "Keystep 37" (or the name given to it), "Any port".
fn port_name(m: &IoModel, l: &Saved) -> String {
    match l.port.as_deref() {
        None => "Any port".to_string(),
        Some(DIN_NAME) => "MIDI in".to_string(),
        Some(p) => m.setup.shown(p).to_string(),
    }
}

/// "CC 74", "CC 1 Mod wheel", "note C1 ch 16", "pitch bend ch 2": a CC
/// named by its port's profile, else as most gear names it.
fn control_name(m: &IoModel, l: &Saved) -> String {
    let profile = l.port.as_deref().and_then(|p| m.profile(p));
    let what = match l.source {
        Source::Cc(n) => match mpfree_midi::profile::cc_name(profile, n) {
            Some(name) => format!("CC {n} {name}"),
            None => format!("CC {n}"),
        },
        Source::Note(n) => format!("note {}", note_name(n)),
        Source::PitchBend => "pitch bend".to_string(),
        Source::ChanPressure => "pressure".to_string(),
    };
    if l.channel == 0 {
        what
    } else {
        format!("{what} ch {}", l.channel + 1)
    }
}

/// "Cutoff (Filter, Bass)" as ("Cutoff", "Filter, Bass").
fn split_name(name: &str) -> (&str, &str) {
    match name.split_once(" (") {
        Some((a, b)) => (a, b.strip_suffix(')').unwrap_or(b)),
        None => (name, ""),
    }
}

/// Why a learned control does nothing now, if it doesn't.
fn idle_why(m: &IoModel, kept: Kept, l: &Saved) -> Option<&'static str> {
    if kept == Kept::Rig && m.setup.learn.iter().any(|p| p.same_control(l)) {
        return Some("the song's own mapping wins");
    }
    match l.port.as_deref() {
        Some(p) if m.live.port(p).is_none() => Some("unplugged, waits"),
        Some(p) if m.setup.port(p).input == PortInput::PlaysTracks => {
            Some("its port only plays tracks")
        }
        _ => None,
    }
}

fn name_of(m: &IoModel, k: usize, l: &Saved) -> String {
    m.live
        .learn_names
        .get(k)
        .cloned()
        .unwrap_or_else(|| l.target.to_string())
}

pub(crate) struct Learn {
    model: Shared,
    /// The picked row, in `MidiSetup::all_learned`'s order.
    pick: usize,
    /// The first row shown.
    top: Cell<usize>,
    down: Option<(u64, Option<Hit>)>,
    note: Option<String>,
    /// The row whose Learn again was tapped, lit while Learn waits.
    again: Option<usize>,
    /// The card shows the transport's buttons, not the picked row (it
    /// does while nothing is learned too).
    transport: bool,
    /// The transport button tapped, lit while Learn waits.
    armed: Option<usize>,
    /// What was drawn last: the model's generation, whether Learn waited
    /// and the values now (to draw again when they change).
    drawn: (u64, bool, u64),
}

impl Learn {
    pub(crate) fn new(model: Shared) -> Learn {
        Learn {
            model,
            pick: 0,
            top: Cell::new(0),
            down: None,
            note: None,
            again: None,
            transport: false,
            armed: None,
            drawn: (u64::MAX, false, 0),
        }
    }

    fn rows_shown(body: Rect) -> usize {
        (body.h / ROW_H).max(1) as usize
    }

    fn top_for(&self, rows: usize, shown: usize) -> usize {
        let mut top = self.top.get();
        if self.pick < top {
            top = self.pick;
        } else if self.pick >= top + shown {
            top = self.pick + 1 - shown;
        }
        top = top.min(rows.saturating_sub(shown));
        self.top.set(top);
        top
    }

    fn row_rect(l: &Layout, top: usize, k: usize) -> Option<Rect> {
        let shown = Self::rows_shown(l.body);
        (k >= top && k < top + shown).then(|| {
            Rect::new(
                l.body.x,
                l.body.y + (k - top) as i32 * ROW_H,
                l.body.w,
                ROW_H - 4,
            )
        })
    }

    fn hit(&self, x: i32, y: i32, r: Rect) -> Option<Hit> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = layout(r, m.live.learning.is_some());
        if l.banner.is_some() && l.cancel.contains(x, y) {
            return Some(Hit::Cancel);
        }
        let n = m.setup.all_learned().count();
        if n > 0 && l.transport.contains(x, y) {
            return Some(Hit::Transport);
        }
        let top = self.top_for(n, Self::rows_shown(l.body));
        for k in 0..n {
            if Self::row_rect(&l, top, k).is_some_and(|rr| rr.contains(x, y)) {
                return Some(Hit::Row(k));
            }
        }
        if !l.card.contains(x, y) {
            return None;
        }
        if self.transport || n == 0 {
            if n > 0 && l.remove.contains(x, y) {
                return Some(Hit::Back);
            }
            return pick_rects(&l)
                .iter()
                .position(|p| p.contains(x, y))
                .map(Hit::Pick);
        }
        if l.again.contains(x, y) {
            return Some(Hit::Again);
        }
        if l.remove.contains(x, y) {
            return Some(Hit::Remove);
        }
        let rr = range_rects(l.line[0]);
        for (i, r) in rr.iter().enumerate() {
            if r.contains(x, y) {
                return Some(Hit::Range(i / 2, if i % 2 == 0 { -1 } else { 1 }));
            }
        }
        let [down, up] = mode_rects(l.line[1]);
        if down.contains(x, y) {
            return Some(Hit::Mode(-1));
        }
        if up.contains(x, y) {
            return Some(Hit::Mode(1));
        }
        for (i, s) in segments(l.line[2], TAKEOVERS.len()).iter().enumerate() {
            if s.contains(x, y) {
                return Some(Hit::Takeover(i));
            }
        }
        for (i, s) in segments(l.line[3], KEPT.len()).iter().enumerate() {
            if s.contains(x, y) {
                return Some(Hit::Kept(i));
            }
        }
        None
    }

    /// Changes the picked learned control; the app applies it at its next
    /// look ([`IoModel::gen`]).
    fn edit(&mut self, f: impl FnOnce(&mut Saved) -> Result<bool, String>) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let Some(l) = m.setup.learned_at_mut(self.pick) else {
            return IoReply::Handled;
        };
        match f(l) {
            Ok(true) => {
                m.gen += 1;
                IoReply::Redraw
            }
            Ok(false) => IoReply::Handled,
            Err(e) => {
                self.note = Some(e);
                IoReply::Handled
            }
        }
    }

    fn range(&mut self, end: usize, d: i32, step: f32) -> IoReply {
        self.edit(|l| {
            let v = if end == 0 { &mut l.lo } else { &mut l.hi };
            let to = ((*v + d as f32 * step) / step).round() * step;
            let to = to.clamp(0.0, 1.0);
            if (to - *v).abs() < f32::EPSILON {
                return Err(if d < 0 {
                    "The range's end is at the bottom".to_string()
                } else {
                    "The range's end is at the top".to_string()
                });
            }
            *v = to;
            Ok(true)
        })
    }

    fn mode(&mut self, d: i32) -> IoReply {
        self.edit(|l| {
            let at = MODES.iter().position(|x| x.0 == l.mode).unwrap_or(0) as i32;
            let to = (at + d).clamp(0, MODES.len() as i32 - 1);
            if to == at {
                return Err("No more ways that way".to_string());
            }
            l.mode = MODES[to as usize].0;
            Ok(true)
        })
    }

    fn takeover(&mut self, i: usize) -> IoReply {
        self.edit(|l| {
            if l.mode != Mode::Absolute {
                return Err("Takeover is for knobs and faders".to_string());
            }
            let to = TAKEOVERS[i].0;
            let changed = l.takeover != to;
            l.takeover = to;
            Ok(changed)
        })
    }

    fn kept(&mut self, i: usize) -> IoReply {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let to = KEPT[i].0;
        if m.setup.learned_at(self.pick).is_none_or(|(k, _)| k == to) {
            return IoReply::Handled;
        }
        match m.setup.keep_at(self.pick, to) {
            Some(at) => {
                self.pick = at;
                m.gen += 1;
                IoReply::Redraw
            }
            None => {
                self.note = Some("No room: 256 learned controls are kept there".to_string());
                IoReply::Handled
            }
        }
    }

    fn tap(&mut self, h: Hit) -> IoReply {
        match h {
            Hit::Row(k) => {
                self.pick = k;
                self.transport = false;
                IoReply::Redraw
            }
            Hit::Cancel => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                m.asks.cancel_learn = true;
                m.asks.learn_again = None;
                m.asks.learn_target = None;
                self.again = None;
                self.armed = None;
                IoReply::Redraw
            }
            Hit::Transport => {
                self.transport = !self.transport;
                IoReply::Redraw
            }
            Hit::Back => {
                self.transport = false;
                IoReply::Redraw
            }
            Hit::Pick(i) => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                m.asks.learn_target = Some(pick_target(i));
                m.asks.learn_again = None;
                m.asks.cancel_learn = false;
                self.again = None;
                self.armed = Some(i);
                self.note = Some(format!(
                    "{}: press a button or move a knob on your controller",
                    transport::ALL[i].1
                ));
                IoReply::Redraw
            }
            Hit::Again => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                if m.setup.learned_at(self.pick).is_some() {
                    m.asks.learn_again = Some(self.pick);
                    m.asks.learn_target = None;
                    m.asks.cancel_learn = false;
                    self.again = Some(self.pick);
                    self.armed = None;
                    self.note =
                        Some("Move a knob, a key or a pedal on your controller".to_string());
                }
                IoReply::Redraw
            }
            Hit::Remove => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                if m.setup.forget_at(self.pick).is_none() {
                    return IoReply::Handled;
                }
                // Keep the app's words in step until it names them again.
                if self.pick < m.live.learn_names.len() {
                    m.live.learn_names.remove(self.pick);
                }
                if self.pick < m.live.learn_live.len() {
                    m.live.learn_live.remove(self.pick);
                }
                let n = m.setup.all_learned().count();
                self.pick = self.pick.min(n.saturating_sub(1));
                m.gen += 1;
                IoReply::Redraw
            }
            Hit::Range(end, d) => self.range(end, d, RANGE_STEP),
            Hit::Mode(d) => self.mode(d),
            Hit::Takeover(i) => self.takeover(i),
            Hit::Kept(i) => self.kept(i),
        }
    }

    /// The values the list shows, as a number, to draw again when one
    /// moves.
    fn now_hash(m: &IoModel) -> u64 {
        m.live.learn_live.iter().fold(0u64, |h, v| {
            let x = v.now.map_or(u64::MAX, |x| (x * 1000.0) as u64);
            h.wrapping_mul(31).wrapping_add(x)
        })
    }

    /// The transport card: a button for each of the transport's controls,
    /// the one Learn waits for lit.
    fn paint_transport(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, back: bool) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(l.card, 12, t.surface);
        let ci = l.card.inset(14);
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(ci.x, ci.y, l.again.x - ci.x - 8, BUTTON_H),
            Align::Left,
            t.text_dim,
            "TRANSPORT AND TEMPO",
        );
        if back {
            button(cx, l.remove, "Back", false);
        }
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(ci.x, ci.y + BUTTON_H + 2, ci.w, 28),
            Align::Left,
            t.text_dim,
            "Tap one, then press a button or move a knob on your controller. It works in every project.",
        );
        let waiting = m.asks.learn_target.is_some() || m.live.learning.is_some();
        for (i, r) in pick_rects(l).into_iter().enumerate() {
            button(cx, r, transport::ALL[i].1, waiting && self.armed == Some(i));
        }
        let last = pick_rects(l)[7];
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(ci.x, last.bottom() + 6, ci.w, 22),
            Align::Left,
            t.text_faint,
            "A key on PLAY and STOP plays, then stops. Tempo wants a knob or a fader. \
             Mutes: hold SHIFT and touch a strip's M.",
        );
    }

    fn paint_card(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, k: usize) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(l.card, 12, t.surface);
        let Some((kept, s)) = m.setup.learned_at(k) else {
            return;
        };
        let name = name_of(m, k, s);
        let (main, _) = split_name(&name);
        let ci = l.card.inset(14);
        let title = format!("{} → {}", control_name(m, s), main).to_uppercase();
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(ci.x, ci.y, l.again.x - ci.x - 8, BUTTON_H),
            Align::Left,
            t.text_dim,
            &title,
        );
        let waiting =
            self.again == Some(k) && (m.asks.learn_again == Some(k) || m.live.learning.is_some());
        button(cx, l.again, "Learn again", waiting);
        button(cx, l.remove, "Remove", false);
        let live = m.live.learn_live.get(k);
        let label = |cx: &mut PaintCx, r: Rect, s: &str| {
            cx.text(
                FontId::Sans,
                t.size_body,
                Rect::new(r.x, r.y, LABEL_W, r.h),
                Align::Left,
                t.text,
                s,
            );
        };
        // Range.
        label(cx, l.line[0], "Range");
        let end = |v: f32, text: Option<&String>| match text {
            Some(s) if !s.is_empty() => s.clone(),
            _ => format!("{:.0} %", v * 100.0),
        };
        let rr = range_rects(l.line[0]);
        stepper(cx, rr[0], rr[1], &end(s.lo, live.map(|v| &v.lo_text)));
        stepper(cx, rr[2], rr[3], &end(s.hi, live.map(|v| &v.hi_text)));
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(rr[1].right(), rr[1].y, rr[2].x - rr[1].right(), rr[1].h),
            Align::Center,
            t.text_dim,
            "to",
        );
        // Works as.
        label(cx, l.line[1], "Works as");
        let [down, up] = mode_rects(l.line[1]);
        let mode = MODES.iter().find(|x| x.0 == s.mode).map_or("?", |x| x.1);
        stepper(cx, down, up, mode);
        // Takeover.
        label(cx, l.line[2], "Takeover");
        let knob = s.mode == Mode::Absolute;
        for (i, sr) in segments(l.line[2], TAKEOVERS.len()).into_iter().enumerate() {
            segment(
                cx,
                sr,
                TAKEOVERS[i].1,
                knob && s.takeover == TAKEOVERS[i].0,
                knob,
            );
        }
        // Kept in.
        label(cx, l.line[3], "Kept in");
        let segs = segments(l.line[3], KEPT.len());
        for (i, sr) in segs.iter().enumerate() {
            segment(cx, *sr, KEPT[i].1, kept == KEPT[i].0, true);
        }
        let last = segs[KEPT.len() - 1];
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(
                last.right() + 12,
                last.y,
                l.line[3].right() - last.right() - 12,
                last.h,
            ),
            Align::Left,
            t.text_dim,
            match kept {
                Kept::Rig => "every project",
                Kept::Project => "this song",
            },
        );
        // What the takeover does, or why there is none.
        let why = if knob {
            TAKEOVERS
                .iter()
                .find(|x| x.0 == s.takeover)
                .map_or("", |x| x.2)
        } else {
            "Takeover is for knobs and faders: a button or an encoder never jumps."
        };
        let lr = Rect::new(ci.x, l.line[3].bottom() + 8, ci.w, 20);
        cx.text(FontId::Sans, t.size_small, lr, Align::Left, t.text_dim, why);
    }
}

fn button(cx: &mut PaintCx, r: Rect, text: &str, lit: bool) {
    let t = cx.theme.clone();
    let (fill, ink) = if lit {
        (t.accent2, t.on_accent)
    } else {
        (t.raised, t.text)
    };
    cx.canvas.fill_round_rect(r, 10, fill);
    cx.text(FontId::Sans, t.size_small, r, Align::Center, ink, text);
}

fn segment(cx: &mut PaintCx, r: Rect, text: &str, on: bool, enabled: bool) {
    let t = cx.theme.clone();
    let (fill, ink) = match (on, enabled) {
        (true, _) => (t.accent, t.on_accent),
        (false, true) => (t.raised, t.text),
        (false, false) => (t.raised, t.text_faint),
    };
    cx.canvas.fill_round_rect(r, 10, fill);
    cx.text(FontId::Bold, t.size_small, r, Align::Center, ink, text);
}

fn stepper(cx: &mut PaintCx, minus: Rect, plus: Rect, value: &str) {
    let t = cx.theme.clone();
    for (r, s) in [(minus, "−"), (plus, "+")] {
        cx.canvas.fill_round_rect(r, 8, t.raised);
        cx.text(FontId::Bold, t.size_body, r, Align::Center, t.text, s);
    }
    let mid = Rect::new(
        minus.right() + 4,
        minus.y,
        plus.x - minus.right() - 8,
        minus.h,
    );
    cx.canvas.fill_round_rect(mid, 8, t.bg);
    cx.text(
        FontId::Sans,
        t.size_small,
        mid,
        Align::Center,
        t.text,
        value,
    );
}

impl IoSection for Learn {
    fn label(&self) -> &str {
        crate::LEARN
    }

    fn hint(&self) -> Option<&str> {
        Some(HINT)
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = layout(r, m.live.learning.is_some());
        // Learning: what the next control moved takes.
        if let (Some(b), Some(name)) = (l.banner, &m.live.learning) {
            cx.canvas.fill_round_rect(b, 12, t.surface);
            cx.canvas
                .fill_round_rect(Rect::new(b.x, b.y + 6, 4, b.h - 12), 2, t.accent2);
            let chip = Rect::new(b.x + 16, b.y + 12, 96, b.h - 24);
            cx.canvas.fill_round_rect(chip, 6, t.accent2);
            cx.text(
                FontId::Bold,
                t.size_small * 0.85,
                chip,
                Align::Center,
                t.on_accent,
                "LEARNING",
            );
            let text = Rect::new(chip.right() + 12, b.y, l.cancel.x - chip.right() - 20, b.h);
            cx.text(
                FontId::Sans,
                t.size_small,
                text,
                Align::Left,
                t.text,
                &format!("{name}: now move a knob, a key or a pedal on any controller"),
            );
            button(cx, l.cancel, "Cancel", false);
        }
        // The heading and the key.
        let all: Vec<(Kept, &Saved)> = m.setup.all_learned().collect();
        cx.text(
            FontId::Bold,
            t.size_small,
            l.head,
            Align::Left,
            t.text_dim,
            &format!("WHAT YOUR CONTROLLERS MOVE · {}", all.len()),
        );
        if !all.is_empty() {
            button(cx, l.transport, "Transport and tempo", self.transport);
        }
        let key = Rect::new(l.transport.x - 12 - 300, l.head.y, 300, l.head.h);
        cx.text(
            FontId::Sans,
            t.size_small * 0.85,
            key,
            Align::Right,
            t.text_faint,
            "RIG: every project  ·  PROJECT: this song",
        );
        cx.canvas.fill_round_rect(l.table, 12, t.surface);
        let cols = columns(l.body);
        let inner = l.table.inset(10);
        for (i, h) in ["Control", "Moves", "Now", "Kept in"].iter().enumerate() {
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                Rect::new(cols[i].0 + 10, inner.y, cols[i].1 - 10, TABLE_HEAD_H),
                Align::Left,
                t.text_faint,
                h,
            );
        }
        if all.is_empty() {
            let lr = Rect::new(l.body.x + 10, l.body.y + 8, l.body.w - 20, 24);
            cx.text(
                FontId::Sans,
                t.size_small,
                lr,
                Align::Left,
                t.text_dim,
                "No controls learned yet.",
            );
            let lr = Rect::new(lr.x, lr.bottom(), lr.w, 24);
            cx.text(
                FontId::Sans,
                t.size_small,
                lr,
                Align::Left,
                t.text_faint,
                "Hold SHIFT and touch a value on any page, then move a knob on your controller.",
            );
            self.paint_transport(cx, &m, &l, false);
            return;
        }
        let shown = Self::rows_shown(l.body);
        let top = self.top_for(all.len(), shown);
        for (k, (kept, s)) in all.iter().enumerate() {
            let Some(rr) = Self::row_rect(&l, top, k) else {
                continue;
            };
            let idle = idle_why(&m, *kept, s);
            if k == self.pick {
                cx.canvas.stroke_round_rect(rr, 8, 2, t.accent2);
            } else if k % 2 == 1 {
                cx.canvas.fill_round_rect(rr, 8, t.raised);
            }
            let ink = if idle.is_some() { t.text_faint } else { t.text };
            let dim = if idle.is_some() {
                t.text_faint
            } else {
                t.text_dim
            };
            // Control: the device, then what on it.
            let (x, w) = cols[0];
            let c = Rect::new(x + 10, rr.y, w - 14, rr.h);
            let port = port_name(&m, s);
            let pw = (port.chars().count() as i32 * 9 + 10).min(c.w / 2);
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(c.x, c.y, pw, c.h),
                Align::Left,
                ink,
                &port,
            );
            cx.text(
                FontId::Sans,
                t.size_small * 0.9,
                Rect::new(c.x + pw, c.y, c.w - pw, c.h),
                Align::Left,
                dim,
                &control_name(&m, s),
            );
            // Moves: the parameter, then where it is (or why it waits).
            let (x, w) = cols[1];
            let c = Rect::new(x + 10, rr.y, w - 14, rr.h);
            let name = name_of(&m, k, s);
            let (main, rest) = split_name(&name);
            let mw = (main.chars().count() as i32 * 9 + 12).min(c.w * 3 / 5);
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(c.x, c.y, mw, c.h),
                Align::Left,
                ink,
                main,
            );
            let sub = idle.unwrap_or(rest);
            let sub_ink = match idle {
                Some("its port only plays tracks") => t.warn,
                Some(_) => t.text_faint,
                None => t.text_dim,
            };
            cx.text(
                FontId::Sans,
                t.size_small * 0.9,
                Rect::new(c.x + mw, c.y, c.w - mw, c.h),
                Align::Left,
                sub_ink,
                sub,
            );
            // Now: where its parameter stands.
            let (x, w) = cols[2];
            let bar = Rect::new(x + 10, rr.y + rr.h / 2 - 3, w - 30, 6);
            match m.live.learn_live.get(k).and_then(|v| v.now) {
                Some(v) => {
                    cx.canvas.fill_round_rect(bar, 3, t.bg);
                    let fill = (bar.w as f32 * v.clamp(0.0, 1.0)) as i32;
                    if fill > 0 {
                        cx.canvas.fill_round_rect(
                            Rect::new(bar.x, bar.y, fill, bar.h),
                            3,
                            t.accent2,
                        );
                    }
                }
                None => {
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        Rect::new(x + 10, rr.y, w - 14, rr.h),
                        Align::Left,
                        t.text_faint,
                        "—",
                    );
                }
            }
            // Kept in.
            let (x, _) = cols[3];
            let chip = Rect::new(x + 6, rr.y + 7, 80, rr.h - 14);
            let (fill, word) = match kept {
                Kept::Rig => (t.accent, "RIG"),
                Kept::Project => (t.accent2, "PROJECT"),
            };
            cx.canvas.stroke_round_rect(chip, 6, 1, fill);
            cx.text(
                FontId::Bold,
                t.size_small * 0.8,
                chip,
                Align::Center,
                fill,
                word,
            );
        }
        if all.len() > shown {
            let lr = Rect::new(l.body.x + 10, l.table.bottom() - 22, l.body.w - 20, 18);
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                lr,
                Align::Left,
                t.text_faint,
                &format!(
                    "rows {}–{} of {}",
                    top + 1,
                    (top + shown).min(all.len()),
                    all.len()
                ),
            );
        }
        if self.transport {
            self.paint_transport(cx, &m, &l, true);
            return;
        }
        let k = self.pick.min(all.len() - 1);
        self.paint_card(cx, &m, &l, k);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = layout(r, m.live.learning.is_some());
        let mut out = Vec::new();
        if l.banner.is_some() {
            out.push(Target::tap("learn-cancel", 0, "Cancel learning", l.cancel));
        }
        let all: Vec<(Kept, &Saved)> = m.setup.all_learned().collect();
        if !all.is_empty() {
            out.push(Target::tap(
                "learn-transport",
                0,
                "Transport and tempo",
                l.transport,
            ));
        }
        let top = self.top_for(all.len(), Self::rows_shown(l.body));
        for (k, (_, s)) in all.iter().enumerate() {
            if let Some(rr) = Self::row_rect(&l, top, k) {
                out.push(Target::tap(
                    "learn-row",
                    k,
                    format!(
                        "{} {}: {}",
                        port_name(&m, s),
                        control_name(&m, s),
                        name_of(&m, k, s)
                    ),
                    rr,
                ));
            }
        }
        if self.transport || all.is_empty() {
            if !all.is_empty() {
                out.push(Target::tap("learn-back", 0, "Back", l.remove));
            }
            for (i, r) in pick_rects(&l).into_iter().enumerate() {
                out.push(Target::tap("learn-pick", i, transport::ALL[i].1, r));
            }
            return out;
        }
        let Some((_, s)) = all.get(self.pick.min(all.len().saturating_sub(1))) else {
            return out;
        };
        out.push(Target::tap("learn-again", 0, "Learn again", l.again));
        out.push(Target::tap("learn-remove", 0, "Remove", l.remove));
        let rr = range_rects(l.line[0]);
        for (i, label) in ["Low −", "Low +", "High −", "High +"].iter().enumerate() {
            out.push(Target::tap("learn-range", i, *label, rr[i]));
        }
        let [down, up] = mode_rects(l.line[1]);
        out.push(Target::tap("learn-mode", 0, "Works as −", down));
        out.push(Target::tap("learn-mode", 1, "Works as +", up));
        for (i, sr) in segments(l.line[2], TAKEOVERS.len()).into_iter().enumerate() {
            out.push(
                Target::tap("learn-takeover", i, TAKEOVERS[i].1, sr)
                    .disabled_if(s.mode != Mode::Absolute),
            );
        }
        for (i, sr) in segments(l.line[3], KEPT.len()).into_iter().enumerate() {
            out.push(Target::tap("learn-kept", i, KEPT[i].1, sr));
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> IoReply {
        match phase {
            Touch::Down => {
                let h = self.hit(x, y, r);
                self.down = Some((finger, h));
                IoReply::Handled
            }
            Touch::Move => IoReply::Handled,
            Touch::Cancel => {
                self.down = None;
                IoReply::Handled
            }
            Touch::Up => {
                let Some((f, Some(h))) = self.down.take() else {
                    return IoReply::Handled;
                };
                if f != finger || self.hit(x, y, r) != Some(h) {
                    return IoReply::Handled;
                }
                self.tap(h)
            }
        }
    }

    fn input(&mut self, ev: IoInput) -> IoReply {
        let n = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            m.setup.all_learned().count()
        };
        match ev {
            IoInput::Wheel(d) | IoInput::Page(d) => {
                let d = if matches!(ev, IoInput::Page(_)) {
                    d * 4
                } else {
                    d
                };
                let to = (self.pick as i32 + d).clamp(0, n.saturating_sub(1) as i32) as usize;
                if to == self.pick {
                    self.note = Some(if n == 0 {
                        "No controls learned yet".to_string()
                    } else if d > 0 {
                        "The last one is picked".to_string()
                    } else {
                        "The first one is picked".to_string()
                    });
                    return IoReply::Handled;
                }
                self.pick = to;
                IoReply::Redraw
            }
            IoInput::Step(_) if self.transport || n == 0 => IoReply::Handled,
            IoInput::Step(d) if d != 0 => self.mode(d.signum()),
            IoInput::Step(_) => IoReply::Handled,
        }
    }

    fn tick(&mut self, _now: std::time::Duration) -> IoReply {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let now = (m.gen, m.live.learning.is_some(), Self::now_hash(&m));
        if now != self.drawn {
            self.drawn = now;
            return IoReply::Redraw;
        }
        IoReply::Ignored
    }

    fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    fn qlinks(&self) -> Option<[IoKnob; 4]> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let off = || IoKnob {
            label: String::new(),
            text: String::new(),
            pos: 0.0,
            enabled: false,
        };
        let Some((_, s)) = m.setup.learned_at(self.pick).filter(|_| !self.transport) else {
            return Some([off(), off(), off(), off()]);
        };
        let live = m.live.learn_live.get(self.pick);
        let end = |v: f32, text: Option<&String>| match text {
            Some(x) if !x.is_empty() => x.clone(),
            _ => format!("{:.0} %", v * 100.0),
        };
        let mode = MODES.iter().position(|x| x.0 == s.mode).unwrap_or(0);
        let take = TAKEOVERS
            .iter()
            .position(|x| x.0 == s.takeover)
            .unwrap_or(0);
        Some([
            IoKnob {
                label: "Low".to_string(),
                text: end(s.lo, live.map(|v| &v.lo_text)),
                pos: s.lo,
                enabled: true,
            },
            IoKnob {
                label: "High".to_string(),
                text: end(s.hi, live.map(|v| &v.hi_text)),
                pos: s.hi,
                enabled: true,
            },
            IoKnob {
                label: "Works as".to_string(),
                text: MODES[mode].1.to_string(),
                pos: mode as f32 / (MODES.len() - 1) as f32,
                enabled: true,
            },
            IoKnob {
                label: "Takeover".to_string(),
                text: TAKEOVERS[take].1.to_string(),
                pos: take as f32 / (TAKEOVERS.len() - 1) as f32,
                enabled: s.mode == Mode::Absolute,
            },
        ])
    }

    fn qlink_turn(&mut self, knob: usize, detents: i32, fine: bool) -> IoReply {
        if detents == 0 || self.transport {
            return IoReply::Ignored;
        }
        let step = if fine { RANGE_STEP / 10.0 } else { RANGE_STEP };
        match knob {
            0 | 1 => self.range(knob, detents, step),
            2 => self.mode(detents.signum()),
            3 => {
                let at = {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    m.setup
                        .learned_at(self.pick)
                        .and_then(|(_, s)| TAKEOVERS.iter().position(|x| x.0 == s.takeover))
                        .unwrap_or(0)
                };
                let to = (at as i32 + detents.signum()).clamp(0, TAKEOVERS.len() as i32 - 1);
                self.takeover(to as usize)
            }
            _ => IoReply::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, AREA};
    use crate::LearnLive;
    use std::sync::{Arc, Mutex};

    fn learned(port: Option<&str>, cc: u8, param: u16) -> Saved {
        use mpfree_param::{OnTrack, ParamAddr};
        Saved {
            port: port.map(String::from),
            channel: 0,
            source: Source::Cc(cc),
            target: ParamAddr::new(OnTrack::Inst.on(1), param),
            lo: 0.0,
            hi: 1.0,
            mode: Mode::Absolute,
            takeover: Takeover::Jump,
        }
    }

    /// The sample rig with two controls learned for the song and one, the
    /// tempo fader on the 5-pin jacks, for every project.
    fn model() -> Shared {
        let mut m = crate::sample::rig();
        m.setup.learned(learned(Some("Keystep 37"), 74, 1));
        m.setup.learned(learned(Some("Digitakt"), 1, 2));
        m.setup
            .learned_in(Kept::Rig, learned(Some(DIN_NAME), 20, 3));
        m.live.learn_names = vec![
            "Cutoff (Filter, Bass)".into(),
            "Send A (Lead)".into(),
            "Volume (Drums)".into(),
        ];
        m.live.learn_live = vec![
            LearnLive {
                now: Some(0.5),
                lo_text: "20 Hz".into(),
                hi_text: "20.0 kHz".into(),
            },
            LearnLive::default(),
            LearnLive {
                now: Some(0.8),
                ..LearnLive::default()
            },
        ];
        Arc::new(Mutex::new(m))
    }

    fn tap(s: &mut Learn, id: &str, index: usize) -> IoReply {
        let r = s
            .targets(AREA)
            .into_iter()
            .find(|t| t.id == id && t.index == index)
            .unwrap_or_else(|| panic!("no {id} {index}"))
            .rect;
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        s.touch(1, Touch::Down, x, y, AREA);
        s.touch(1, Touch::Up, x, y, AREA)
    }

    #[test]
    fn every_control_fits_the_section() {
        for learning in [false, true] {
            let m = model();
            m.lock().unwrap().live.learning = learning.then(|| "Drive (Filter, Bass)".to_string());
            let s = Learn::new(m);
            let _ = paint(&s);
            let ts = s.targets(AREA);
            assert!(ts.len() > 10, "{} targets", ts.len());
            for t in ts {
                let r = t.rect;
                assert!(
                    r.x >= AREA.x
                        && r.y >= AREA.y
                        && r.right() <= AREA.right()
                        && r.bottom() <= AREA.bottom(),
                    "{} \"{}\" at {r:?} is outside {AREA:?}",
                    t.id,
                    t.label
                );
            }
        }
        // With nothing learned it says how to learn one, and the card
        // offers the transport.
        let s = Learn::new(Arc::new(Mutex::new(crate::sample::rig())));
        let _ = paint(&s);
        let ids: Vec<&str> = s.targets(AREA).iter().map(|t| t.id).collect();
        assert_eq!(ids, ["learn-pick"; 8]);
        // The transport card fits too.
        let mut s = Learn::new(model());
        assert_eq!(tap(&mut s, "learn-transport", 0), IoReply::Redraw);
        let _ = paint(&s);
        for t in s.targets(AREA) {
            assert!(
                AREA.contains(t.rect.x, t.rect.y)
                    && t.rect.right() <= AREA.right()
                    && t.rect.bottom() <= AREA.bottom(),
                "{} \"{}\" at {:?} is outside {AREA:?}",
                t.id,
                t.label,
                t.rect
            );
        }
    }

    #[test]
    fn the_transport_card_arms_learn_on_play_and_stop_the_tempo_and_the_rest() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        // The row's card until the heading's button swaps it.
        assert!(!s.targets(AREA).iter().any(|t| t.id == "learn-pick"));
        tap(&mut s, "learn-transport", 0);
        let labels: Vec<String> = s
            .targets(AREA)
            .into_iter()
            .filter(|t| t.id == "learn-pick")
            .map(|t| t.label)
            .collect();
        assert_eq!(
            labels,
            [
                "PLAY and STOP",
                "PLAY",
                "STOP",
                "PLAY START",
                "REC",
                "OVERDUB",
                "TAP TEMPO",
                "Tempo"
            ]
        );
        // The Q-Links have nothing to turn while it shows.
        assert!(s.qlinks().unwrap().iter().all(|k| !k.enabled));
        assert_eq!(s.qlink_turn(0, 1, false), IoReply::Ignored);
        // PLAY and STOP: Learn waits for a control to take it.
        assert_eq!(tap(&mut s, "learn-pick", 0), IoReply::Redraw);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Transport, transport::PLAYING))
        );
        assert_eq!(
            s.take_note().unwrap(),
            "PLAY and STOP: press a button or move a knob on your controller"
        );
        // Tempo in its place; Cancel takes it back.
        tap(&mut s, "learn-pick", 7);
        assert_eq!(
            m.lock().unwrap().asks.learn_target,
            Some(ParamAddr::new(Owner::Transport, transport::TEMPO))
        );
        m.lock().unwrap().live.learning = Some("Tempo".into());
        let _ = paint(&s);
        tap(&mut s, "learn-cancel", 0);
        {
            let v = m.lock().unwrap();
            assert_eq!(v.asks.learn_target, None);
            assert!(v.asks.cancel_learn);
        }
        m.lock().unwrap().live.learning = None;
        // Back, or a tap on a row, shows the row's card again.
        tap(&mut s, "learn-back", 0);
        assert!(s.targets(AREA).iter().any(|t| t.id == "learn-again"));
        tap(&mut s, "learn-transport", 0);
        tap(&mut s, "learn-row", 1);
        assert!(s.targets(AREA).iter().any(|t| t.id == "learn-again"));
        assert_eq!(s.pick, 1);
    }

    #[test]
    fn the_list_says_where_each_is_kept_and_why_one_does_nothing() {
        let m = model();
        let s = Learn::new(m.clone());
        let labels: Vec<String> = s
            .targets(AREA)
            .into_iter()
            .filter(|t| t.id == "learn-row")
            .map(|t| t.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Keystep 37 CC 74 Cutoff: Cutoff (Filter, Bass)",
                "Digitakt CC 1 Mod wheel: Send A (Lead)",
                "MIDI in CC 20: Volume (Drums)",
            ]
        );
        let v = m.lock().unwrap();
        let all: Vec<_> = v.setup.all_learned().collect();
        assert_eq!(all[2].0, Kept::Rig);
        // The keyboard is Learn only in the sample rig: it learns.
        assert_eq!(idle_why(&v, all[0].0, all[0].1), None);
        assert_eq!(idle_why(&v, all[1].0, all[1].1), Some("unplugged, waits"));
        assert_eq!(idle_why(&v, all[2].0, all[2].1), None);
    }

    #[test]
    fn the_card_sets_the_range_how_it_works_and_takes_over_and_where_it_is_kept() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        let gen = m.lock().unwrap().gen;
        // Low end up two steps, high end down one.
        assert_eq!(tap(&mut s, "learn-range", 1), IoReply::Redraw);
        tap(&mut s, "learn-range", 1);
        tap(&mut s, "learn-range", 2);
        {
            let v = m.lock().unwrap();
            assert!(v.gen > gen);
            assert!((v.setup.learn[0].lo - 0.02).abs() < 1e-6);
            assert!((v.setup.learn[0].hi - 0.99).abs() < 1e-6);
        }
        // At the bottom, the low end says so.
        tap(&mut s, "learn-range", 0);
        tap(&mut s, "learn-range", 0);
        assert_eq!(tap(&mut s, "learn-range", 0), IoReply::Handled);
        assert_eq!(s.take_note().unwrap(), "The range's end is at the bottom");
        // Scale, then an endless encoder: takeover is for knobs.
        tap(&mut s, "learn-takeover", 2);
        assert_eq!(m.lock().unwrap().setup.learn[0].takeover, Takeover::Scale);
        assert_eq!(s.input(IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(
            m.lock().unwrap().setup.learn[0].mode,
            Mode::Relative(Encoding::TwosComplement)
        );
        assert!(s
            .targets(AREA)
            .iter()
            .any(|t| t.id == "learn-takeover" && !t.enabled));
        assert_eq!(tap(&mut s, "learn-takeover", 0), IoReply::Handled);
        assert_eq!(s.take_note().unwrap(), "Takeover is for knobs and faders");
        // Kept for every project: it moves to the rig's list.
        assert_eq!(tap(&mut s, "learn-kept", 0), IoReply::Redraw);
        {
            let v = m.lock().unwrap();
            assert_eq!(v.setup.learn.len(), 1);
            assert_eq!(v.setup.rig_learn.len(), 2);
            assert_eq!(v.setup.learned_at(s.pick).unwrap().0, Kept::Rig);
            assert_eq!(v.setup.learned_at(s.pick).unwrap().1.source, Source::Cc(74));
        }
        // The Q-Links turn the range and the way it works.
        let q = s.qlinks().unwrap();
        assert_eq!(q[0].label, "Low");
        assert!(!q[3].enabled);
        assert_eq!(s.qlink_turn(1, -1, true), IoReply::Redraw);
    }

    #[test]
    fn learn_again_asks_the_app_and_cancel_stops_it() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        s.input(IoInput::Wheel(1));
        tap(&mut s, "learn-again", 0);
        assert_eq!(m.lock().unwrap().asks.learn_again, Some(1));
        assert!(s.take_note().unwrap().starts_with("Move a knob"));
        // The app arms Learn and names what it waits for: the banner
        // shows, with Cancel.
        m.lock().unwrap().live.learning = Some("Send A (Lead)".to_string());
        assert_eq!(s.tick(std::time::Duration::ZERO), IoReply::Redraw);
        let _ = paint(&s);
        tap(&mut s, "learn-cancel", 0);
        let v = m.lock().unwrap();
        assert!(v.asks.cancel_learn);
        assert_eq!(v.asks.learn_again, None);
    }

    #[test]
    fn remove_forgets_the_picked_one() {
        let m = model();
        let mut s = Learn::new(m.clone());
        let _ = paint(&s);
        s.input(IoInput::Wheel(2));
        assert_eq!(tap(&mut s, "learn-remove", 0), IoReply::Redraw);
        let v = m.lock().unwrap();
        assert!(v.setup.rig_learn.is_empty());
        assert_eq!(v.setup.learn.len(), 2);
        assert_eq!(v.live.learn_names.len(), 2);
        assert_eq!(s.pick, 1);
    }
}
