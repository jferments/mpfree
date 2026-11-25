//! Play's loop row (the looper redesign's step 5; Jay, 17 Nov 22:51Z):
//! every loop on the All view, in the 150 px the looper strip had. The
//! strip showed one loop's wave, so with eight loops you only ever saw
//! one; Play is where everything is played together, so it shows all
//! eight, and the Looper tab is where a loop is shaped. The Play, Pads and
//! Sequencer redesign's board 1 draws it so (the looper redesign's
//! Play-all-loops board).
//!
//! The heading: the Arm pill (REC records the loop), "LOOPS", the picked
//! loop's layers as chips (a dot in the layer's colour, its name and its
//! speaker: a tap mutes that layer), and at the right Capture and the
//! Pads switch (the pads as loop keys, as SHIFT + SAMPLER). Under it a
//! card a loop: its name, its ring (the Looper tab's: its state, where it
//! is in its bars and which bar) and beside the ring its speaker and S; an
//! empty loop says what it records from.
//!
//! A tap on a ring does the loop's next step on the press (record, play,
//! overdub, play), as on the Looper tab; held for
//! [`CLEAR_HOLD_S`] it clears the loop. A tap on a name picks the loop
//! without starting anything; the speaker mutes it and S solos it.
//!
//! With Dev's "Loops on audio tracks" on (audio tracks phase 4e, board 7
//! of the phase 4 screens, approved as drawn 23 Nov 2026) the row has
//! [`LoopRow::group`]: the Loop group ([`crate::loop_group`]) at its right,
//! Level, Filter and Delay and the switch (All loops, or the armed
//! track's), in place of Capture and the Pads switch, which move into the
//! heading as chips. The cards are drawn small: each named by its place
//! ("Vocal · 2", "Free" for a slot with no loop), its ring, and a word
//! under it ("4 bars", "muted", "free"). As drawn, they have no speaker or
//! S: a loop's mute and solo are its track's, on the Looper tab's card and
//! the Mix tab's strip.

use crate::loop_group::{GroupKnob, GroupSet};
use crate::looper_deck::{
    capture_icon, paint_ring, CaptureButton, Channel, RingState, CLEAR_HOLD_S,
};
use crate::looper_page::LAYER_COLOURS;
use crate::looper_tab::LoopAct;
use crate::play_widgets::{paint_arm, ArmPill, ARM_PILL_W};
use mpfree_ui::draw::{self, speaker_on};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Shape, Target, Touch, Widget};
use std::cell::RefCell;

/// The row's widget name: the looper strip's, which it replaced.
pub const LOOP_ROW: &str = "play-loop";

/// The row's events. The Arm pill gives `Select(ARM_TAP)` (as the
/// pattern's does), a layer chip `Toggle(layer)`; everything else keeps a
/// [`LoopAct`] ([`LoopRow::take`]) and gives `Tab(0)`.
pub const ARM_TAP: usize = 0;

/// The heading's height, the cards' margins and the space between them.
const HEAD_H: i32 = 44;
const SIDE: i32 = 12;
const GAP: i32 = 8;
/// A card's name line.
const NAME_H: i32 = 24;
/// The heading's buttons: Capture, and the Pads switch at the right end.
const CAPTURE_W: i32 = 176;
const PADS_W: i32 = 164;
const BUTTON_H: i32 = 30;
/// A layer chip's height.
const CHIP_H: i32 = 28;
/// A ring smaller than this shows only the big middle (its bar, "+").
const SMALL_RING: i32 = 80;

/// With the Loop group (board 7): the panel's width, the heading over the
/// small cards, the space between them and their rings.
const GROUP_W: i32 = 306;
const GROUP_HEAD_H: i32 = 34;
const SMALL_GAP: i32 = 6;
const SMALL_RING_D: i32 = 48;
/// A knob's column (its arc, name and value) and its arc.
const KNOB_W: i32 = 60;
const KNOB_D: i32 = 40;
/// The switch's halves.
const SWITCH_W: i32 = 74;
const SWITCH_H: i32 = 34;
/// Capture's and the Pads switch's chips in the heading.
const CAPTURE_CHIP_W: i32 = 136;
const PADS_CHIP_W: i32 = 112;
const HEAD_CHIP_H: i32 = 24;
/// A knob's drag: pixels a detent (up turns it up).
const DRAG_PX: i32 = 2;

/// The Loop group as the row shows it (audio tracks phase 4e, board 7).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupPanel {
    /// Where its knobs and switch are. The row moves them itself, at once,
    /// and keeps a [`LoopAct::Group`] for each change.
    pub set: GroupSet,
    /// The armed track's name, when it is an audio track: the switch's
    /// second half ("Armed: Vocal"); None: "Armed: none".
    pub armed: Option<String>,
}

/// A finger turning a group knob: where it was last, and the pixels not
/// yet a detent.
#[derive(Clone, Copy, Debug, PartialEq)]
struct KnobDrag {
    finger: u64,
    knob: GroupKnob,
    y: i32,
    rem: i32,
}

/// One of the picked loop's layers, as its chip shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerChip {
    /// "Drums", "Bass", "Line in".
    pub name: String,
    pub muted: bool,
    /// Heard (not muted, and not left out by another's solo): else grey.
    pub heard: bool,
}

/// Where a card's parts are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowParts {
    pub card: Rect,
    pub name: Rect,
    pub ring: Rect,
    pub mute: Rect,
    pub solo: Rect,
}

/// A finger on a ring: which loop, how long it has stayed, and whether
/// the hold has cleared the loop.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RingPress {
    finger: u64,
    n: usize,
    from: (i32, i32),
    held: f32,
    cleared: bool,
}

/// What the heading shows.
#[derive(Clone, Debug, PartialEq)]
struct HeadLook {
    arm: Option<ArmPill>,
    picked: usize,
    layers: Vec<LayerChip>,
    capture: CaptureButton,
    pads: bool,
    /// With the group: the loops in use ("LOOPS 6 OF 8") and the picked
    /// loop's place, over its layers; else 0 and empty.
    used: usize,
    picked_name: String,
}

/// What a card showed when last drawn, to draw again only the cards that
/// changed.
#[derive(Clone, Debug, PartialEq)]
struct CardLook {
    ch: Channel,
    picked: bool,
    /// How much of a clear hold is left, in tenths of a percent.
    clearing: Option<u32>,
}

type Drawn = (Rect, HeadLook, Vec<CardLook>, Option<GroupPanel>);

/// Play's row of loops.
#[derive(Default)]
pub struct LoopRow {
    /// Every loop, as the Looper tab's deck has them.
    pub channels: Vec<Channel>,
    pub picked: usize,
    /// The picked loop's layers, layer 1 first.
    pub layers: Vec<LayerChip>,
    pub capture: CaptureButton,
    /// The pads are the loop keys: the Pads switch lit.
    pub pads: bool,
    /// The Arm pill: the loop as a REC target.
    pub arm: Option<ArmPill>,
    /// With Dev's "Loops on audio tracks" on (phase 4e, board 7): the Loop
    /// group at the row's right, Capture and the Pads switch as chips in
    /// the heading, the cards small and named by place. None: the row as
    /// before.
    pub group: Option<GroupPanel>,
    press: Option<RingPress>,
    drag: Option<KnobDrag>,
    acts: Vec<LoopAct>,
    /// Where each layer chip was drawn, layer 1 first, for a tap.
    chips: RefCell<Vec<Rect>>,
    drawn: RefCell<Option<Drawn>>,
}

impl LoopRow {
    /// Every act the row has kept since the last call.
    pub fn take(&mut self) -> Vec<LoopAct> {
        std::mem::take(&mut self.acts)
    }

    /// The heading's strip.
    pub fn head_rect(r: Rect) -> Rect {
        Rect::new(r.x, r.y, r.w, HEAD_H)
    }

    /// The Pads switch, at the heading's right end.
    pub fn pads_rect(r: Rect) -> Rect {
        Rect::new(
            r.right() - SIDE - PADS_W,
            r.y + (HEAD_H - BUTTON_H) / 2 - 1,
            PADS_W,
            BUTTON_H,
        )
    }

    /// Capture, left of the Pads switch.
    pub fn capture_rect(r: Rect) -> Rect {
        let p = LoopRow::pads_rect(r);
        Rect::new(p.x - 10 - CAPTURE_W, p.y, CAPTURE_W, p.h)
    }

    /// Card `i`'s parts, of `n`, in the row at `r`.
    pub fn parts(r: Rect, i: usize, n: usize) -> RowParts {
        let n = n.max(1) as i32;
        let area = Rect::new(r.x + SIDE, r.y + HEAD_H, r.w - 2 * SIDE, r.h - HEAD_H - 8);
        let w = (area.w - GAP * (n - 1)) / n;
        let card = Rect::new(area.x + i as i32 * (w + GAP), area.y, w, area.h);
        let d = (card.h - NAME_H - 8).min(card.w - 52).max(24);
        let ring = Rect::new(card.x + 8, card.y + NAME_H + 2, d, d);
        let bx = ring.right() + 6;
        let bw = (card.right() - 6 - bx).max(20);
        let bh = (d - 6) / 2;
        RowParts {
            card,
            name: Rect::new(card.x, card.y, card.w, NAME_H),
            ring,
            mute: Rect::new(bx, ring.y, bw, bh),
            solo: Rect::new(bx, ring.bottom() - bh, bw, bh),
        }
    }

    /// The Loop group's panel, at the row's right (board 7).
    pub fn group_rect(r: Rect) -> Rect {
        Rect::new(r.right() - SIDE - GROUP_W, r.y + 8, GROUP_W, r.h - 18)
    }

    /// Group knob `k`'s column ([`GroupKnob::ALL`]): its arc, name and
    /// value, what a finger drags.
    pub fn knob_rect(r: Rect, k: usize) -> Rect {
        let g = LoopRow::group_rect(r);
        Rect::new(
            g.x + 12 + k as i32 * (KNOB_W + 12),
            g.y + 26,
            KNOB_W,
            g.h - 30,
        )
    }

    /// The group's switch: All loops over the armed track's (`armed`).
    pub fn switch_rect(r: Rect, armed: bool) -> Rect {
        let g = LoopRow::group_rect(r);
        let y = g.y + 28 + if armed { SWITCH_H + 6 } else { 0 };
        Rect::new(g.right() - 10 - SWITCH_W, y, SWITCH_W, SWITCH_H)
    }

    /// With the group: the Pads switch as a chip in the heading, before
    /// the panel.
    pub fn pads_chip_rect(r: Rect) -> Rect {
        let g = LoopRow::group_rect(r);
        Rect::new(
            g.x - 16 - PADS_CHIP_W,
            r.y + (GROUP_HEAD_H - HEAD_CHIP_H) / 2 + 1,
            PADS_CHIP_W,
            HEAD_CHIP_H,
        )
    }

    /// With the group: Capture as a chip, left of the Pads chip.
    pub fn capture_chip_rect(r: Rect) -> Rect {
        let p = LoopRow::pads_chip_rect(r);
        Rect::new(p.x - 8 - CAPTURE_CHIP_W, p.y, CAPTURE_CHIP_W, p.h)
    }

    /// With the group: card `i`'s parts, of `n`, small (board 7): its
    /// place over its ring and a word under it; no speaker or S (their
    /// rects are empty).
    pub fn small_parts(r: Rect, i: usize, n: usize) -> RowParts {
        let n = n.max(1) as i32;
        let g = LoopRow::group_rect(r);
        let x0 = r.x + SIDE + 2;
        let area = Rect::new(
            x0,
            r.y + GROUP_HEAD_H,
            g.x - 8 - x0,
            r.h - GROUP_HEAD_H - 12,
        );
        let w = (area.w - SMALL_GAP * (n - 1)) / n;
        let card = Rect::new(area.x + i as i32 * (w + SMALL_GAP), area.y, w, area.h);
        let d = SMALL_RING_D.min(card.w - 12).min(card.h - 52).max(24);
        RowParts {
            card,
            name: Rect::new(card.x, card.y, card.w, 22),
            ring: Rect::new(card.x + (card.w - d) / 2, card.y + 26, d, d),
            mute: Rect::default(),
            solo: Rect::default(),
        }
    }

    /// Card `i`'s parts as the row draws them now.
    fn parts_of(&self, r: Rect, i: usize) -> RowParts {
        let n = self.channels.len();
        match self.group {
            Some(_) => LoopRow::small_parts(r, i, n),
            None => LoopRow::parts(r, i, n),
        }
    }

    /// Capture as the row draws it now: a chip with the group.
    fn capture_at(&self, r: Rect) -> Rect {
        match self.group {
            Some(_) => LoopRow::capture_chip_rect(r),
            None => LoopRow::capture_rect(r),
        }
    }

    /// The Pads switch as the row draws it now: a chip with the group.
    fn pads_at(&self, r: Rect) -> Rect {
        match self.group {
            Some(_) => LoopRow::pads_chip_rect(r),
            None => LoopRow::pads_rect(r),
        }
    }

    /// The heading as the row draws it now.
    fn head_at(&self, r: Rect) -> Rect {
        match self.group {
            Some(_) => Rect::new(r.x, r.y, r.w, GROUP_HEAD_H),
            None => LoopRow::head_rect(r),
        }
    }

    /// The area under the heading that the cards' backdrops cover.
    fn body_at(&self, r: Rect) -> Rect {
        match self.group {
            Some(_) => {
                let g = LoopRow::group_rect(r);
                Rect::new(r.x, r.y + GROUP_HEAD_H, g.x - 4 - r.x, r.h - GROUP_HEAD_H)
            }
            None => body(r),
        }
    }

    /// Knob `knob` turned `ticks` detents (`fine`: a finer step): the
    /// panel follows at once and the change is kept for the audio. False
    /// when it was at its end already, or there is no group.
    fn turn_knob(&mut self, knob: GroupKnob, ticks: i32, fine: bool) -> bool {
        let Some(g) = self.group.as_mut() else {
            return false;
        };
        let set = knob.turn(g.set, ticks, fine);
        if set == g.set {
            return false;
        }
        g.set = set;
        self.acts.push(LoopAct::Group(set));
        true
    }

    /// A finger has stayed on a ring a moment longer: held for
    /// [`CLEAR_HOLD_S`] it clears the loop (once). True when it just did.
    pub fn hold_tick(&mut self, dt: f32) -> bool {
        let Some(p) = self.press.as_mut() else {
            return false;
        };
        if p.cleared {
            return false;
        }
        p.held += dt.max(0.0);
        if p.held < CLEAR_HOLD_S {
            return false;
        }
        p.cleared = true;
        self.acts.push(LoopAct::Clear(p.n));
        true
    }

    /// A finger rests on a ring whose loop can be cleared.
    pub fn holding(&self) -> bool {
        self.press.is_some_and(|p| !p.cleared)
    }

    fn clearing(&self, n: usize) -> Option<f32> {
        self.press
            .filter(|p| p.n == n && !p.cleared && p.held > 0.0)
            .map(|p| (1.0 - p.held / CLEAR_HOLD_S).clamp(0.0, 1.0))
    }

    fn head_look(&self) -> HeadLook {
        HeadLook {
            arm: self.arm,
            picked: self.picked,
            layers: self.layers.clone(),
            capture: self.capture.clone(),
            pads: self.pads,
            used: if self.group.is_some() { self.used() } else { 0 },
            picked_name: match (&self.group, self.channels.get(self.picked)) {
                (Some(_), Some(ch)) => ch.name.clone(),
                _ => String::new(),
            },
        }
    }

    /// The loops in use: in a cell, or holding sound.
    fn used(&self) -> usize {
        self.channels
            .iter()
            .filter(|c| !c.track.is_empty() || c.state != RingState::Empty)
            .count()
    }

    fn looks(&self) -> Vec<CardLook> {
        self.channels
            .iter()
            .enumerate()
            .map(|(i, ch)| CardLook {
                ch: ch.clone(),
                picked: i == self.picked,
                clearing: self.clearing(i).map(|f| (f * 1000.0) as u32),
            })
            .collect()
    }

    fn paint_head(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (h, _) = draw::card_head(r);
        let mut x = h.x;
        if self.arm.is_some() {
            x += ARM_PILL_W + 10;
        }
        x += cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            Rect::new(x, h.y, 80, h.h),
            Align::Left,
            t.text_dim,
            "LOOPS",
        ) + 16;
        // The picked loop's layers, as many as fit before Capture.
        self.paint_chips(cx, r, x, LoopRow::capture_rect(r).x - 12);
        // Capture: the last bars heard become a loop.
        let cap = &self.capture;
        let b = LoopRow::capture_rect(r);
        let (bg, fg) = if cap.lit {
            (t.picked(), t.on_picked())
        } else if cap.works {
            (t.raised, t.text)
        } else {
            (t.raised, t.text_faint)
        };
        cx.canvas.fill_round_rect(b, t.round(8), bg);
        let icon = Rect::new(b.x + 12, b.y + b.h / 2 - 8, 16, 16);
        capture_icon(cx, icon, fg);
        cx.text(
            FontId::Bold,
            13.0,
            Rect::new(icon.right() + 8, b.y, b.right() - icon.right() - 12, b.h),
            Align::Left,
            fg,
            &cap.label,
        );
        // The Pads switch: lit while the pads are the loop keys.
        let b = LoopRow::pads_rect(r);
        let (bg, dot, ink) = if self.pads {
            (t.picked(), t.on_picked(), t.on_picked())
        } else {
            (t.raised, t.text_faint, t.text_dim)
        };
        cx.canvas.fill_round_rect(b, b.h / 2, bg);
        draw::dot(cx, b.x + 18, b.y + b.h / 2, 5, dot);
        cx.text(
            FontId::Bold,
            13.0,
            Rect::new(b.x + 30, b.y, b.w - 36, b.h),
            Align::Left,
            ink,
            "Pads: loop keys",
        );
    }

    /// With the group (board 7): "LOOPS 6 OF 8", the picked loop's layers
    /// (or what a ring does, with none), and Capture and the Pads switch
    /// as chips before the panel.
    fn paint_head_small(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (h, _) = draw::card_head(r);
        let mut x = h.x;
        if self.arm.is_some() {
            x += ARM_PILL_W + 10;
        }
        let title = format!("LOOPS {} OF {}", self.used(), self.channels.len());
        x += cx.text(
            FontId::Bold,
            draw::HEAD_PX,
            Rect::new(x, h.y, 130, h.h),
            Align::Left,
            t.text_dim,
            &title,
        ) + 12;
        let end = LoopRow::capture_chip_rect(r).x - 12;
        if self.layers.is_empty() {
            *self.chips.borrow_mut() = Vec::new();
            cx.text(
                FontId::Sans,
                12.0,
                Rect::new(x, h.y, (end - x).max(0), h.h),
                Align::Left,
                t.text_faint,
                "\u{b7} tap a ring: record, play, overdub",
            );
        } else {
            self.paint_chips(cx, r, x, end);
        }
        let chip = |cx: &mut PaintCx, b: Rect, lit: bool, works: bool, label: &str| {
            let (bg, ink) = match (lit, works) {
                (true, _) => (t.picked(), t.on_picked()),
                (false, true) => (t.raised, t.text),
                (false, false) => (t.raised, t.text_faint),
            };
            cx.canvas.fill_round_rect(b, t.round(6), bg);
            cx.text(
                FontId::Sans,
                12.0,
                b.inset_xy(8, 0),
                Align::Center,
                ink,
                label,
            );
        };
        let cap = &self.capture;
        chip(
            cx,
            LoopRow::capture_chip_rect(r),
            cap.lit,
            cap.works,
            &cap.label,
        );
        chip(
            cx,
            LoopRow::pads_chip_rect(r),
            self.pads,
            true,
            "Pads: loop keys",
        );
    }

    /// The picked loop's layers as chips from `x`, as many as fit before
    /// `end`; where each was drawn is kept for a tap.
    fn paint_chips(&self, cx: &mut PaintCx, r: Rect, mut x: i32, end: i32) {
        let t = cx.theme.clone();
        let (h, _) = draw::card_head(r);
        let cy = h.y + h.h / 2;
        let mut chips = Vec::new();
        if !self.layers.is_empty() {
            // With the group, the picked loop by its place.
            let title = match (&self.group, self.channels.get(self.picked)) {
                (Some(_), Some(ch)) => format!("{}:", short_place(&ch.name)),
                _ => format!("Loop {}:", self.picked + 1),
            };
            x += cx.text(
                FontId::Bold,
                14.0,
                Rect::new(x, h.y, 90, h.h),
                Align::Left,
                t.text_dim,
                &title,
            ) + 10;
        }
        for (k, l) in self.layers.iter().enumerate() {
            let tw = cx.measure(FontId::Bold, 13.0, &l.name);
            // The dot, the name, a gap, the speaker.
            let w = 12 + 10 + tw + 12 + 22 + 8;
            let more = self.layers.len() - k - 1;
            let room = if more > 0 { 36 } else { 0 };
            if x + w + room > end {
                cx.text(
                    FontId::Bold,
                    13.0,
                    Rect::new(x, h.y, 40, h.h),
                    Align::Left,
                    t.text_faint,
                    &format!("+{}", self.layers.len() - k),
                );
                break;
            }
            let chip = Rect::new(x, cy - CHIP_H / 2, w, CHIP_H);
            let colour = LAYER_COLOURS[k % LAYER_COLOURS.len()];
            let off = !l.heard;
            let chip_fill = t.surface.mix(t.raised, 0.6);
            cx.canvas.fill_round_rect(chip, CHIP_H / 2, chip_fill);
            cx.canvas.stroke_round_rect(chip, CHIP_H / 2, 1, t.outline);
            draw::dot(
                cx,
                chip.x + 16,
                cy,
                4,
                if off { t.text_faint } else { colour },
            );
            cx.text(
                FontId::Bold,
                13.0,
                Rect::new(chip.x + 26, chip.y, tw + 4, chip.h),
                Align::Left,
                if off { t.text_faint } else { t.text },
                &l.name,
            );
            speaker_on(
                cx,
                chip.right() - 18,
                cy,
                if l.muted { t.bad } else { t.text_dim },
                l.muted,
                chip_fill,
            );
            // The whole chip answers, from the row's top to below it.
            chips.push(Rect::new(chip.x, r.y, chip.w, chip.bottom() + 6 - r.y));
            x = chip.right() + 8;
        }
        *self.chips.borrow_mut() = chips;
    }

    fn paint_card(&self, cx: &mut PaintCx, r: Rect, i: usize, look: &CardLook) {
        let t = cx.theme.clone();
        let p = LoopRow::parts(r, i, self.channels.len());
        let ch = &look.ch;
        cx.canvas
            .fill_rect(p.card.inset(-GAP / 2).intersect(body(r)), t.surface);
        cx.canvas
            .fill_round_rect(p.card, t.round(10), t.surface.mix(t.raised, 0.4));
        if look.picked {
            cx.canvas
                .stroke_round_rect(p.card, t.round(10), 2, t.accent);
        } else {
            cx.canvas
                .stroke_round_rect(p.card, t.round(10), 1, t.outline);
        }
        let mut s = Shape::new(Rect::new(p.name.x + 6, p.name.y + 6, 14, 14));
        s.disc(p.name.x as f32 + 14.0, p.name.y as f32 + 13.0, 4.0);
        s.draw(cx.canvas, ch.colour);
        let ink = if ch.state == RingState::Empty {
            t.text_dim
        } else {
            t.text
        };
        let name = Rect::new(p.name.x + 23, p.name.y + 3, p.name.w - 28, 20);
        cx.text(FontId::Bold, 14.0, name, Align::Left, ink, &ch.name);
        // A free loop: "free" after its name.
        if ch.free && ch.state.has_sound() {
            let nw = cx.measure(FontId::Bold, 14.0, &ch.name);
            cx.text(
                FontId::Sans,
                12.0,
                Rect::new(name.x + nw + 6, name.y + 1, (name.w - nw - 6).max(0), 20),
                Align::Left,
                t.text_dim,
                "free",
            );
        }
        // A small ring has room for its big middle only.
        let clearing = look.clearing.map(|c| c as f32 / 1000.0);
        if p.ring.w < SMALL_RING && !ch.centre.1.is_empty() {
            let ch = Channel {
                centre: (ch.centre.0.clone(), String::new()),
                ..ch.clone()
            };
            paint_ring(cx, p.ring, &ch, clearing);
        } else {
            paint_ring(cx, p.ring, ch, clearing);
        }
        if ch.state.has_sound() {
            let muted = ch.state == RingState::Muted;
            let b = if muted {
                t.bad.mix(t.bg, 0.72)
            } else {
                t.raised
            };
            cx.canvas.fill_round_rect(p.mute, t.round(8), b);
            let (sx, sy) = (p.mute.x + p.mute.w / 2 + 2, p.mute.y + p.mute.h / 2);
            speaker_on(cx, sx, sy, if muted { t.bad } else { t.text }, muted, b);
            let (fill, ink) = if ch.solo {
                (t.picked(), t.on_picked())
            } else {
                (t.raised, t.text_dim)
            };
            cx.canvas.fill_round_rect(p.solo, t.round(8), fill);
            cx.text(FontId::Bold, 14.0, p.solo, Align::Center, ink, "S");
        } else {
            // What an empty loop records from, beside its ring.
            let foot = Rect::new(
                p.mute.x - 2,
                p.ring.y,
                p.card.right() - p.mute.x - 2,
                p.ring.h,
            );
            // "Pads · free" on two lines: what it records from over how.
            match ch.foot.split_once(" · ") {
                Some((a, b)) => {
                    let mid = foot.y + foot.h / 2;
                    for (k, line) in [a, b].into_iter().enumerate() {
                        let r = Rect::new(foot.x, mid - 16 + k as i32 * 16, foot.w, 16);
                        cx.text(FontId::Sans, 12.0, r, Align::Center, t.text_dim, line);
                    }
                }
                None => {
                    cx.text(
                        FontId::Sans,
                        12.0,
                        foot,
                        Align::Center,
                        t.text_dim,
                        &ch.foot,
                    );
                }
            }
        }
    }
}

impl LoopRow {
    /// With the group: a small card (board 7), its place over its ring and
    /// a word under it. A muted track's loop is drawn dim.
    fn paint_small_card(&self, cx: &mut PaintCx, r: Rect, i: usize, look: &CardLook) {
        let t = cx.theme.clone();
        let p = LoopRow::small_parts(r, i, self.channels.len());
        let ch = &look.ch;
        cx.canvas.fill_rect(
            p.card.inset(-SMALL_GAP / 2).intersect(self.body_at(r)),
            t.surface,
        );
        cx.canvas
            .fill_round_rect(p.card, t.round(10), t.surface.mix(t.raised, 0.4));
        if look.picked {
            cx.canvas
                .stroke_round_rect(p.card, t.round(10), 2, t.accent);
        } else {
            cx.canvas
                .stroke_round_rect(p.card, t.round(10), 1, t.outline);
        }
        let ink = if ch.state == RingState::Empty {
            t.text_dim
        } else {
            ch.colour
        };
        cx.text(
            FontId::Bold,
            11.5,
            Rect::new(p.card.x + 6, p.card.y + 5, p.card.w - 12, 16),
            Align::Left,
            ink,
            &short_place(&ch.name),
        );
        // A small ring: its big middle only; a muted track's, dim.
        let mut ring = Channel {
            centre: (ch.centre.0.clone(), String::new()),
            ..ch.clone()
        };
        if ch.track_mute && matches!(ch.state, RingState::Playing | RingState::Overdubbing) {
            ring.state = RingState::Muted;
        }
        let clearing = look.clearing.map(|c| c as f32 / 1000.0);
        paint_ring(cx, p.ring, &ring, clearing);
        cx.text(
            FontId::Sans,
            11.0,
            Rect::new(p.card.x + 3, p.ring.bottom() + 3, p.card.w - 6, 16),
            Align::Center,
            t.text_dim,
            &card_word(ch),
        );
    }

    /// The Loop group's panel: its title, Level, Filter and Delay, and
    /// the switch.
    fn paint_group(&self, cx: &mut PaintCx, r: Rect, g: &GroupPanel) {
        let t = cx.theme.clone();
        let p = LoopRow::group_rect(r);
        cx.canvas
            .fill_rect(p.inset(-4).intersect(r.inset(2)), t.surface);
        cx.canvas
            .fill_round_rect(p, t.round(10), t.surface.mix(t.raised, 0.7));
        cx.text(
            FontId::Bold,
            12.0,
            Rect::new(p.x + 12, p.y + 6, p.w - 24, 18),
            Align::Left,
            t.good,
            "LOOP GROUP \u{b7} PERFORMANCE",
        );
        let lit = [t.accent, t.warn, t.accent2];
        for (k, knob) in GroupKnob::ALL.into_iter().enumerate() {
            let col = LoopRow::knob_rect(r, k);
            let turned = knob.get(&g.set) != 0.0;
            let (v, both) = knob.arc(&g.set);
            let arc = Rect::new(col.x + (col.w - KNOB_D) / 2, col.y, KNOB_D, KNOB_D);
            paint_knob(cx, arc, v, both, if turned { lit[k] } else { t.text_dim });
            cx.text(
                FontId::Bold,
                12.0,
                Rect::new(col.x - 6, arc.bottom() + 2, col.w + 12, 16),
                Align::Center,
                t.text,
                knob.name(),
            );
            cx.text(
                FontId::Sans,
                11.0,
                Rect::new(col.x - 6, arc.bottom() + 18, col.w + 12, 15),
                Align::Center,
                if turned { t.text } else { t.text_dim },
                &knob.text(&g.set),
            );
        }
        for armed in [false, true] {
            let b = LoopRow::switch_rect(r, armed);
            let on = g.set.armed_only == armed;
            let (bg, ink) = if on {
                (t.picked(), t.on_picked())
            } else {
                (t.raised, t.text)
            };
            cx.canvas.fill_round_rect(b, t.round(6), bg);
            if armed {
                let (who, ink2) = match &g.armed {
                    Some(n) => (n.as_str(), ink),
                    None => ("none", if on { ink } else { t.text_faint }),
                };
                let top = Rect::new(b.x + 2, b.y + 2, b.w - 4, b.h / 2 - 1);
                let low = Rect::new(b.x + 2, b.y + b.h / 2 - 1, b.w - 4, b.h / 2 - 1);
                cx.text(FontId::Bold, 12.0, top, Align::Center, ink, "Armed:");
                cx.text(FontId::Bold, 12.0, low, Align::Center, ink2, who);
            } else {
                cx.text(FontId::Bold, 12.0, b, Align::Center, ink, "All loops");
            }
        }
    }
}

/// A place as a small card says it: "Vocal · 2" for "Vocal · row 2".
fn short_place(name: &str) -> String {
    name.replace("\u{b7} row ", "\u{b7} ")
}

/// What a small card says under its ring (board 7): its length, "muted"
/// for a muted track's, where an armed loop waits, "free" for a slot with
/// no loop or a free loop.
fn card_word(ch: &Channel) -> String {
    let bars = |n: usize| match n {
        0 => String::new(),
        1 => "1 bar".to_string(),
        n => format!("{n} bars"),
    };
    match ch.state {
        // A slot in no cell.
        RingState::Empty if ch.track.is_empty() => "free".to_string(),
        RingState::Empty => ch.foot.split(" \u{b7} ").next().unwrap_or("").to_string(),
        RingState::Waiting(_) => match ch.centre.1.as_str() {
            "REC on bar" => format!("REC on bar {}", ch.centre.0),
            "REC on PLAY" => "REC on PLAY".to_string(),
            small => format!("{} {small}", ch.centre.0),
        },
        RingState::Recording => "REC".to_string(),
        RingState::Overdubbing => "overdub".to_string(),
        _ if ch.track_mute => "muted".to_string(),
        RingState::Muted => "muted".to_string(),
        RingState::Off => "off".to_string(),
        RingState::Stopped(_) => "stopped".to_string(),
        RingState::Playing if ch.free => "free".to_string(),
        RingState::Playing => bars(ch.bars),
    }
}

/// A group knob: its 270-degree track, the value's arc (from the middle
/// both ways when `both`, as Filter's) and its pointer.
fn paint_knob(cx: &mut PaintCx, b: Rect, v: f32, both: bool, lit: Color) {
    use std::f32::consts::PI;
    let t = cx.theme.clone();
    let (x, y) = (b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0);
    let rad = b.w.min(b.h) as f32 / 2.0 - 3.0;
    let bbox = b.inset(-4);
    let (a0, a1) = (0.75 * PI, 2.25 * PI);
    let mut s = Shape::new(bbox);
    s.arc(x, y, rad, a0, a1, 4.0);
    s.draw(cx.canvas, t.outline);
    let at = a0 + (a1 - a0) * v.clamp(0.0, 1.0);
    let from = if both { (a0 + a1) / 2.0 } else { a0 };
    let (lo, hi) = if at < from { (at, from) } else { (from, at) };
    if hi - lo > 0.01 {
        let mut s = Shape::new(bbox);
        s.arc(x, y, rad, lo, hi, 4.0);
        s.draw(cx.canvas, lit);
    }
    let mut s = Shape::new(bbox);
    s.disc(x + rad * at.cos(), y + rad * at.sin(), 3.0);
    s.draw(cx.canvas, t.text);
}

/// The row's area under its heading (the cards' backdrop).
fn body(r: Rect) -> Rect {
    Rect::new(r.x, r.y + HEAD_H, r.w, r.h - HEAD_H)
}

impl Widget for LoopRow {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let looks = self.looks();
        match &self.group {
            Some(g) => {
                self.paint_head_small(cx, r);
                for (i, look) in looks.iter().enumerate() {
                    self.paint_small_card(cx, r, i, look);
                }
                self.paint_group(cx, r, g);
            }
            None => {
                self.paint_head(cx, r);
                for (i, look) in looks.iter().enumerate() {
                    self.paint_card(cx, r, i, look);
                }
            }
        }
        // The Arm pill, and the panel's red edge or outline while armed.
        if let Some(pill) = self.arm {
            paint_arm(cx, r, pill);
        }
        *self.drawn.borrow_mut() = Some((r, self.head_look(), looks, self.group.clone()));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, head, looks, group) = drawn.as_ref()?;
        if *at != r
            || looks.len() != self.channels.len()
            || head.arm != self.arm
            || group.is_some() != self.group.is_some()
        {
            return None;
        }
        let now = self.looks();
        let mut parts = Vec::new();
        if *head != self.head_look() {
            parts.push(self.head_at(r));
        }
        for (i, (a, b)) in looks.iter().zip(&now).enumerate() {
            if a != b {
                let gap = if self.group.is_some() { SMALL_GAP } else { GAP };
                let card = self.parts_of(r, i).card;
                parts.push(card.inset(-gap / 2).intersect(self.body_at(r)));
            }
        }
        if *group != self.group {
            parts.push(LoopRow::group_rect(r).inset(-4).intersect(r));
        }
        Some(parts)
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = Vec::new();
        if let Some(pill) = self.arm {
            out.push(Target::tap(
                "arm",
                0,
                pill.label(),
                crate::play_widgets::arm_pill_hit(r),
            ));
        }
        for (k, c) in self.chips.borrow().iter().enumerate() {
            let name = self.layers.get(k).map_or("", |l| l.name.as_str());
            out.push(Target::tap("layer-mute", k, format!("{name} speaker"), *c));
        }
        out.push(
            Target::tap("capture", 0, self.capture.label.clone(), self.capture_at(r))
                .disabled_if(!self.capture.works && !self.capture.lit),
        );
        out.push(Target::tap(
            "pads-keys",
            0,
            if self.pads {
                "Pads: loop keys (on)"
            } else {
                "Pads: loop keys"
            },
            self.pads_at(r),
        ));
        let small = self.group.is_some();
        for (i, ch) in self.channels.iter().enumerate() {
            let p = self.parts_of(r, i);
            // A small card picks anywhere but its ring.
            let pick = if small { p.card } else { p.name };
            out.push(Target::tap("loop", i, ch.who().to_string(), pick));
            let does = match ch.state {
                RingState::Empty => "record",
                RingState::Waiting(_) => "cancel",
                RingState::Recording => "close and play",
                RingState::Overdubbing => "play",
                RingState::Playing | RingState::Off => "overdub",
                RingState::Muted => "unmute",
                RingState::Stopped(false) => "play the free loops",
                RingState::Stopped(true) => "call off the free loops' Play",
            };
            out.push(Target::tap(
                "ring",
                i,
                format!("{} ring: {does}", ch.who()),
                p.ring,
            ));
            if ch.state.has_sound() && !small {
                out.push(Target::tap(
                    "loop-mute",
                    i,
                    format!("{} speaker", ch.who()),
                    p.mute,
                ));
                out.push(Target::tap(
                    "loop-solo",
                    i,
                    format!("{} solo{}", ch.who(), if ch.solo { " (on)" } else { "" }),
                    p.solo,
                ));
            }
        }
        if let Some(g) = &self.group {
            for (k, knob) in GroupKnob::ALL.into_iter().enumerate() {
                out.push(Target::drag(
                    "group-knob",
                    k,
                    format!("Loop group {}: {}", knob.name(), knob.text(&g.set)),
                    LoopRow::knob_rect(r, k),
                ));
            }
            let on = |b: bool| if b { " (on)" } else { "" };
            out.push(Target::tap(
                "group-switch",
                0,
                format!("All loops{}", on(!g.set.armed_only)),
                LoopRow::switch_rect(r, false),
            ));
            out.push(Target::tap(
                "group-switch",
                1,
                format!(
                    "Armed: {}{}",
                    g.armed.as_deref().unwrap_or("none"),
                    on(g.set.armed_only)
                ),
                LoopRow::switch_rect(r, true),
            ));
        }
        out
    }

    fn steps(&self, t: &Target) -> bool {
        t.id == "group-knob" && self.group.is_some()
    }

    fn step(&mut self, t: &Target, delta: i32, r: Rect) -> Option<EventKind> {
        self.held_step(t, delta, false, r)
    }

    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, _r: Rect) -> Option<EventKind> {
        let knob = *GroupKnob::ALL
            .get(t.index)
            .filter(|_| t.id == "group-knob")?;
        self.turn_knob(knob, delta, fine)
            .then_some(EventKind::Tab(0))
    }

    /// SHIFT + the wheel's push on a group knob: back at rest.
    fn reset(&mut self, t: &Target, _r: Rect) -> Option<EventKind> {
        let knob = *GroupKnob::ALL
            .get(t.index)
            .filter(|_| t.id == "group-knob")?;
        let g = self.group.as_mut()?;
        let mut set = g.set;
        match knob {
            GroupKnob::Level => set.level_db = 0.0,
            GroupKnob::Filter => set.filter = 0.0,
            GroupKnob::Delay => set.delay = 0.0,
        }
        if set == g.set {
            return None;
        }
        g.set = set;
        self.acts.push(LoopAct::Group(set));
        Some(EventKind::Tab(0))
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let n = self.channels.len();
        match phase {
            Touch::Down => {
                if self.arm.is_some() && crate::play_widgets::arm_pill_hit(r).contains(x, y) {
                    return Some(EventKind::Select(ARM_TAP));
                }
                if let Some(k) = self.chips.borrow().iter().position(|c| c.contains(x, y)) {
                    return Some(EventKind::Toggle(k));
                }
                if let Some(g) = self.group.as_mut() {
                    // A knob turns as the finger moves; the switch on the
                    // press.
                    if let Some(k) = (0..3).find(|&k| LoopRow::knob_rect(r, k).contains(x, y)) {
                        self.drag = Some(KnobDrag {
                            finger,
                            knob: GroupKnob::ALL[k],
                            y,
                            rem: 0,
                        });
                        return None;
                    }
                    if let Some(armed) = [false, true]
                        .into_iter()
                        .find(|&a| LoopRow::switch_rect(r, a).contains(x, y))
                    {
                        if g.set.armed_only == armed {
                            return None;
                        }
                        g.set.armed_only = armed;
                        let set = g.set;
                        self.acts.push(LoopAct::Group(set));
                        return Some(EventKind::Tab(0));
                    }
                    if LoopRow::group_rect(r).contains(x, y) {
                        return None;
                    }
                }
                let small = self.group.is_some();
                let act = if self.pads_at(r).contains(x, y) {
                    LoopAct::PadKeys
                } else if self.capture_at(r).contains(x, y) {
                    LoopAct::Capture
                } else {
                    let i = (0..n).find(|&i| self.parts_of(r, i).card.contains(x, y))?;
                    let p = self.parts_of(r, i);
                    let sound = self.channels[i].state.has_sound();
                    if sound && !small && p.mute.contains(x, y) {
                        LoopAct::Mute(i)
                    } else if sound && !small && p.solo.contains(x, y) {
                        LoopAct::Solo(i)
                    } else if p.ring.inset(-4).contains(x, y) {
                        // On the press: a punch is where it was played.
                        if sound {
                            self.press = Some(RingPress {
                                finger,
                                n: i,
                                from: (x, y),
                                held: 0.0,
                                cleared: false,
                            });
                        }
                        LoopAct::Tap(i)
                    } else if small || p.name.contains(x, y) {
                        LoopAct::Pick(i)
                    } else {
                        return None;
                    }
                };
                self.acts.push(act);
                Some(EventKind::Tab(0))
            }
            Touch::Move => {
                // A finger that slides off is no hold.
                if let Some(p) = self.press.filter(|p| p.finger == finger) {
                    let (dx, dy) = (x - p.from.0, y - p.from.1);
                    if dx.abs().max(dy.abs()) > 24 {
                        self.press = None;
                    }
                }
                // A knob: up turns it up, a detent each DRAG_PX.
                let d = self.drag.filter(|d| d.finger == finger)?;
                let moved = d.rem + (d.y - y);
                let ticks = moved / DRAG_PX;
                self.drag = Some(KnobDrag {
                    y,
                    rem: moved - ticks * DRAG_PX,
                    ..d
                });
                (ticks != 0 && self.turn_knob(d.knob, ticks, false)).then_some(EventKind::Tab(0))
            }
            Touch::Up | Touch::Cancel => {
                if self.press.is_some_and(|p| p.finger == finger) {
                    self.press = None;
                }
                if self.drag.is_some_and(|d| d.finger == finger) {
                    self.drag = None;
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> (LoopRow, Rect) {
        let r = LoopRow {
            channels: (0..8)
                .map(|i| Channel {
                    name: format!("Loop {}", i + 1),
                    state: if i < 3 {
                        RingState::Playing
                    } else {
                        RingState::Empty
                    },
                    foot: "Pads".to_string(),
                    ..Channel::default()
                })
                .collect(),
            layers: vec![
                LayerChip {
                    name: "Drums".into(),
                    muted: false,
                    heard: true,
                },
                LayerChip {
                    name: "Bass".into(),
                    muted: true,
                    heard: false,
                },
            ],
            arm: Some(ArmPill::Arm),
            capture: CaptureButton {
                label: "Capture last 2 bars".into(),
                works: true,
                lit: false,
            },
            ..LoopRow::default()
        };
        // Play's All view at 1280 x 800: the row under the transport card.
        (r, Rect::new(12, 180, 1038, 150))
    }

    fn down(w: &mut LoopRow, r: Rect, b: Rect) -> Option<EventKind> {
        let (x, y) = (b.x + b.w / 2, b.y + b.h / 2);
        let e = w.touch(1, Touch::Down, x, y, r);
        w.touch(1, Touch::Up, x, y, r);
        e
    }

    /// Eight cards side by side in the strip's 150 px, each ring big
    /// enough to hit, and the speaker and S beside it.
    #[test]
    fn eight_loops_fit_in_the_strips_room() {
        let (_, r) = row();
        let a = LoopRow::parts(r, 0, 8);
        let h = LoopRow::parts(r, 7, 8);
        assert!(a.card.w >= 110, "{:?}", a.card);
        assert!(a.ring.w >= 60 && a.ring.h == a.ring.w, "{:?}", a.ring);
        assert!(a.ring.bottom() <= a.card.bottom(), "{a:?}");
        assert!(a.mute.x > a.ring.right() && a.mute.right() <= a.card.right());
        assert!(a.mute.w >= 28 && a.mute.h >= 28, "{:?}", a.mute);
        assert!(a.solo.y > a.mute.bottom(), "S under the speaker");
        assert!(h.card.right() <= r.right() - SIDE + 1);
        assert!(
            a.card.y >= LoopRow::capture_rect(r).bottom(),
            "under the heading"
        );
        let c = LoopRow::capture_rect(r);
        let p = LoopRow::pads_rect(r);
        assert!(c.right() < p.x && p.right() <= r.right() - SIDE);
    }

    /// A ring acts on the press (its loop's next step), its name picks,
    /// its speaker mutes and its S solos; an empty loop has no speaker.
    #[test]
    fn a_ring_acts_on_the_press_and_the_buttons_beside_it_mute_and_solo() {
        let (mut w, r) = row();
        let p = LoopRow::parts(r, 1, 8);
        let (x, y) = (p.ring.x + p.ring.w / 2, p.ring.y + p.ring.h / 2);
        assert_eq!(w.touch(1, Touch::Down, x, y, r), Some(EventKind::Tab(0)));
        assert_eq!(w.take(), [LoopAct::Tap(1)], "on the press");
        w.touch(1, Touch::Up, x, y, r);
        assert_eq!(w.take(), []);
        down(&mut w, r, p.name);
        down(&mut w, r, p.mute);
        down(&mut w, r, p.solo);
        assert_eq!(
            w.take(),
            [LoopAct::Pick(1), LoopAct::Mute(1), LoopAct::Solo(1)]
        );
        let e = LoopRow::parts(r, 5, 8);
        assert_eq!(down(&mut w, r, e.mute), None, "an empty loop: no speaker");
        down(&mut w, r, e.ring);
        assert_eq!(w.take(), [LoopAct::Tap(5)], "an empty ring records");
        down(&mut w, r, LoopRow::capture_rect(r));
        down(&mut w, r, LoopRow::pads_rect(r));
        assert_eq!(w.take(), [LoopAct::Capture, LoopAct::PadKeys]);
    }

    /// Held a second, a ring clears its loop, once; a finger that slides
    /// off is no hold, and an empty ring never starts one.
    #[test]
    fn a_ring_held_a_second_clears_its_loop() {
        let (mut w, r) = row();
        let p = LoopRow::parts(r, 0, 8);
        let (x, y) = (p.ring.x + p.ring.w / 2, p.ring.y + p.ring.h / 2);
        w.touch(1, Touch::Down, x, y, r);
        w.take();
        assert!(w.holding());
        assert!(!w.hold_tick(CLEAR_HOLD_S / 2.0));
        assert!(w.hold_tick(CLEAR_HOLD_S / 2.0 + 0.01));
        assert_eq!(w.take(), [LoopAct::Clear(0)]);
        assert!(!w.hold_tick(1.0) && !w.holding(), "once");
        w.touch(1, Touch::Up, x, y, r);
        w.touch(2, Touch::Down, x, y, r);
        w.touch(2, Touch::Move, x + 30, y, r);
        assert!(!w.holding(), "slid off");
        w.touch(2, Touch::Up, x + 30, y, r);
        let e = LoopRow::parts(r, 6, 8);
        w.touch(3, Touch::Down, e.ring.x + 30, e.ring.y + 30, r);
        assert!(!w.holding(), "nothing to clear");
    }

    /// The Arm pill and the layer chips keep their own events (the
    /// pattern's Arm, a layer's mute), found where they were drawn.
    #[test]
    fn the_arm_pill_and_the_layer_chips_answer_where_drawn() {
        let (mut w, r) = row();
        let mut canvas = mpfree_ui::SoftCanvas::new(1280, 800);
        let (theme, mut fonts) = (mpfree_ui::Theme::dark(), mpfree_ui::Fonts::embedded());
        let mut cx = PaintCx {
            canvas: &mut canvas,
            theme: &theme,
            fonts: &mut fonts,
        };
        w.paint(&mut cx, r);
        let chips = w.chips.borrow().clone();
        assert_eq!(chips.len(), 2);
        assert!(chips[0].right() <= chips[1].x, "in order");
        assert!(
            chips[1].right() < LoopRow::capture_rect(r).x,
            "before Capture"
        );
        assert_eq!(down(&mut w, r, chips[1]), Some(EventKind::Toggle(1)));
        let arm = crate::play_widgets::arm_pill_rect(r);
        assert_eq!(down(&mut w, r, arm), Some(EventKind::Select(ARM_TAP)));
        assert_eq!(w.take(), [], "neither is a loop act");
        let ids: Vec<&str> = w.targets(r).iter().map(|t| t.id).collect();
        for id in ["arm", "layer-mute", "capture", "pads-keys", "loop", "ring"] {
            assert!(ids.contains(&id), "{id} in {ids:?}");
        }
    }

    /// Only the cards that changed are drawn again: a ring moving on is
    /// its card, the heading only when it changed.
    #[test]
    fn a_moving_ring_draws_only_its_card_again() {
        let (mut w, r) = row();
        let mut canvas = mpfree_ui::SoftCanvas::new(1280, 800);
        let (theme, mut fonts) = (mpfree_ui::Theme::dark(), mpfree_ui::Fonts::embedded());
        let mut cx = PaintCx {
            canvas: &mut canvas,
            theme: &theme,
            fonts: &mut fonts,
        };
        w.paint(&mut cx, r);
        assert_eq!(w.changed_parts(r), Some(Vec::new()));
        w.channels[2].position = 0.5;
        let parts = w.changed_parts(r).unwrap();
        assert_eq!(parts.len(), 1);
        assert!(parts[0].contains(
            LoopRow::parts(r, 2, 8).ring.x + 1,
            LoopRow::parts(r, 2, 8).ring.y + 1
        ));
        w.paint(&mut cx, r);
        w.layers[0].muted = true;
        assert_eq!(w.changed_parts(r), Some(vec![LoopRow::head_rect(r)]));
    }

    /// The row with Dev's switch on: three of Vocal's loops (in rows 1 to
    /// 3) and five free slots, the Loop group with Vocal armed.
    fn grouped() -> (LoopRow, Rect) {
        let (mut w, r) = row();
        for (i, ch) in w.channels.iter_mut().enumerate() {
            if i < 3 {
                ch.name = format!("Vocal \u{b7} row {}", i + 1);
                ch.track = "VOCAL'S TRACK".into();
                ch.bars = 4;
            } else {
                ch.name = "Free".into();
            }
        }
        w.group = Some(GroupPanel {
            armed: Some("Vocal".into()),
            ..GroupPanel::default()
        });
        (w, r)
    }

    fn painted(w: &LoopRow, r: Rect) {
        let mut canvas = mpfree_ui::SoftCanvas::new(1280, 800);
        let (theme, mut fonts) = (mpfree_ui::Theme::dark(), mpfree_ui::Fonts::embedded());
        let mut cx = PaintCx {
            canvas: &mut canvas,
            theme: &theme,
            fonts: &mut fonts,
        };
        w.paint(&mut cx, r);
    }

    /// With the group (board 7): eight small cards before the panel, each
    /// a 48 px ring with room for its place over it and its word under it;
    /// Capture and the Pads switch as chips in the heading; the knobs and
    /// the switch inside the panel.
    #[test]
    fn with_the_group_eight_small_cards_and_the_panel_share_the_row() {
        let (w, r) = grouped();
        let g = LoopRow::group_rect(r);
        assert_eq!(g.w, GROUP_W);
        assert!(g.right() <= r.right() - SIDE && g.bottom() <= r.bottom());
        let a = LoopRow::small_parts(r, 0, 8);
        let h = LoopRow::small_parts(r, 7, 8);
        assert_eq!((a.card.w, a.ring.w, a.ring.h), (82, 48, 48), "{a:?}");
        assert!(a.ring.y >= a.card.y + 22 && a.ring.bottom() + 18 <= a.card.bottom());
        assert!(h.card.right() < g.x, "{h:?} {g:?}");
        let (c, p) = (LoopRow::capture_chip_rect(r), LoopRow::pads_chip_rect(r));
        assert!(c.right() < p.x && p.right() < g.x);
        assert!(a.card.y >= p.bottom(), "the chips over the cards");
        let (s0, s1) = (
            LoopRow::switch_rect(r, false),
            LoopRow::switch_rect(r, true),
        );
        for k in 0..3 {
            let kr = LoopRow::knob_rect(r, k);
            assert!(
                g.contains(kr.x, kr.y) && kr.bottom() <= g.bottom(),
                "{kr:?}"
            );
            assert!(kr.right() < s0.x, "{kr:?}");
        }
        assert!(s0.bottom() < s1.y && s1.bottom() <= g.bottom());
        painted(&w, r);
    }

    /// A small card: its ring acts on the press, anywhere else on it picks
    /// its loop; it has no speaker or S (a loop's are its track's). The
    /// heading's chips are Capture and the Pads switch.
    #[test]
    fn with_the_group_a_small_card_taps_and_the_heading_chips_answer() {
        let (mut w, r) = grouped();
        let p = LoopRow::small_parts(r, 1, 8);
        down(&mut w, r, p.ring);
        down(
            &mut w,
            r,
            Rect::new(p.card.x + 2, p.ring.bottom() + 6, 8, 8),
        );
        down(&mut w, r, LoopRow::capture_chip_rect(r));
        down(&mut w, r, LoopRow::pads_chip_rect(r));
        assert_eq!(
            w.take(),
            [
                LoopAct::Tap(1),
                LoopAct::Pick(1),
                LoopAct::Capture,
                LoopAct::PadKeys
            ]
        );
        // The panel's own face answers nothing.
        let g = LoopRow::group_rect(r);
        assert_eq!(down(&mut w, r, Rect::new(g.x + 4, g.y + 4, 4, 4)), None);
        let ids: Vec<&str> = w.targets(r).iter().map(|t| t.id).collect();
        assert!(!ids.contains(&"loop-mute") && !ids.contains(&"loop-solo"));
        for id in [
            "group-knob",
            "group-switch",
            "capture",
            "pads-keys",
            "ring",
            "loop",
        ] {
            assert!(ids.contains(&id), "{id} in {ids:?}");
        }
    }

    /// A knob turns as the finger moves (up turns it up, a detent each
    /// 2 px), the panel following at once and each change kept; the switch
    /// moves on the press, once.
    #[test]
    fn a_group_knob_turns_with_the_finger_and_the_switch_on_the_press() {
        let (mut w, r) = grouped();
        let level = |w: &LoopRow| w.group.as_ref().unwrap().set.level_db;
        let k = LoopRow::knob_rect(r, 0);
        let (x, y) = (k.x + k.w / 2, k.y + k.h / 2);
        assert_eq!(w.touch(1, Touch::Down, x, y, r), None);
        assert_eq!(
            w.touch(1, Touch::Move, x, y - 1, r),
            None,
            "not a detent yet"
        );
        assert_eq!(
            w.touch(1, Touch::Move, x, y - 13, r),
            Some(EventKind::Tab(0))
        );
        assert_eq!(level(&w), 3.0);
        w.touch(1, Touch::Move, x, y + 12, r);
        assert_eq!(level(&w), -3.0);
        w.touch(1, Touch::Up, x, y + 12, r);
        let acts = w.take();
        assert_eq!(acts.len(), 2);
        assert!(matches!(acts[1], LoopAct::Group(s) if s.level_db == -3.0 && !s.armed_only));
        assert_eq!(w.touch(1, Touch::Move, x, y - 40, r), None, "let go");
        down(&mut w, r, LoopRow::switch_rect(r, true));
        down(&mut w, r, LoopRow::switch_rect(r, true));
        let acts = w.take();
        assert_eq!(acts.len(), 1, "once: {acts:?}");
        assert!(matches!(acts[0], LoopAct::Group(s) if s.armed_only && s.level_db == -3.0));
        painted(&w, r);
    }

    /// The data wheel turns a knob held or outlined (SHIFT: finer), and
    /// SHIFT + its push puts it back at rest; without the group it turns
    /// nothing.
    #[test]
    fn the_wheel_turns_a_group_knob_and_shift_push_puts_it_at_rest() {
        let (mut w, r) = grouped();
        let filter = |w: &LoopRow| w.group.as_ref().unwrap().set.filter;
        let t = w
            .targets(r)
            .into_iter()
            .find(|t| t.id == "group-knob" && t.index == 1)
            .unwrap();
        assert!(w.steps(&t));
        let (x, y) = t.centre();
        assert_eq!(
            w.held(x, y, r).map(|h| (h.id, h.index)),
            Some(("group-knob", 1))
        );
        assert_eq!(w.held_step(&t, -5, false, r), Some(EventKind::Tab(0)));
        assert_eq!(filter(&w), -10.0);
        w.held_step(&t, 4, true, r);
        assert_eq!(filter(&w), -8.0);
        assert_eq!(w.reset(&t, r), Some(EventKind::Tab(0)));
        assert!(w.group.as_ref().unwrap().set.at_rest());
        assert_eq!(w.reset(&t, r), None, "at rest already");
        assert_eq!(w.take().len(), 3);
        w.group = None;
        assert!(!w.steps(&t));
        assert_eq!(w.step(&t, 3, r), None);
    }

    /// A small card's place and word (board 7).
    #[test]
    fn a_small_card_says_its_place_and_a_word() {
        assert_eq!(short_place("Vocal \u{b7} row 2"), "Vocal \u{b7} 2");
        let word = |state: RingState, f: fn(&mut Channel)| {
            let mut c = Channel {
                state,
                track: "VOCAL'S TRACK".into(),
                bars: 4,
                ..Channel::default()
            };
            f(&mut c);
            card_word(&c)
        };
        assert_eq!(word(RingState::Playing, |_| {}), "4 bars");
        assert_eq!(word(RingState::Playing, |c| c.bars = 1), "1 bar");
        assert_eq!(word(RingState::Playing, |c| c.track_mute = true), "muted");
        assert_eq!(word(RingState::Playing, |c| c.free = true), "free");
        assert_eq!(word(RingState::Empty, |c| c.track.clear()), "free");
        assert_eq!(
            word(RingState::Empty, |c| c.foot = "Line in \u{b7} free".into()),
            "Line in"
        );
        assert_eq!(
            word(RingState::Waiting(Some(16)), |c| c.centre =
                ("17".into(), "REC on bar".into())),
            "REC on bar 17"
        );
        assert_eq!(word(RingState::Recording, |_| {}), "REC");
    }

    /// A knob turned draws only the panel again; the switch going off
    /// (the group gone) draws the whole row.
    #[test]
    fn a_turned_knob_draws_only_the_panel_again() {
        let (mut w, r) = grouped();
        painted(&w, r);
        assert_eq!(w.changed_parts(r), Some(Vec::new()));
        w.group.as_mut().unwrap().set.delay = 0.5;
        assert_eq!(
            w.changed_parts(r),
            Some(vec![LoopRow::group_rect(r).inset(-4).intersect(r)])
        );
        painted(&w, r);
        w.channels[0].position = 0.5;
        let parts = w.changed_parts(r).unwrap();
        assert_eq!(parts.len(), 1);
        let ring = LoopRow::small_parts(r, 0, 8).ring;
        assert!(parts[0].contains(ring.x + 1, ring.y + 1));
        painted(&w, r);
        w.group = None;
        assert_eq!(w.changed_parts(r), None);
    }
}
