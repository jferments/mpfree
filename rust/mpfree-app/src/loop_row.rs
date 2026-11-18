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

use crate::looper_deck::{
    capture_icon, paint_ring, CaptureButton, Channel, RingState, CLEAR_HOLD_S,
};
use crate::looper_page::LAYER_COLOURS;
use crate::looper_tab::LoopAct;
use crate::play_widgets::{paint_arm, ArmPill, ARM_PILL_W};
use mpfree_ui::draw::{self, speaker};
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Shape, Target, Touch, Widget};
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

type Drawn = (Rect, HeadLook, Vec<CardLook>);

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
    press: Option<RingPress>,
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
        }
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
        let cy = h.y + h.h / 2;
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
        let end = LoopRow::capture_rect(r).x - 12;
        let mut chips = Vec::new();
        if !self.layers.is_empty() {
            let title = format!("Loop {}:", self.picked + 1);
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
            cx.canvas
                .fill_round_rect(chip, CHIP_H / 2, t.surface.mix(t.raised, 0.6));
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
            speaker(
                cx,
                chip.right() - 18,
                cy,
                if l.muted { t.bad } else { t.text_dim },
                l.muted,
            );
            // The whole chip answers, from the row's top to below it.
            chips.push(Rect::new(chip.x, r.y, chip.w, chip.bottom() + 6 - r.y));
            x = chip.right() + 8;
        }
        *self.chips.borrow_mut() = chips;
        // Capture: the last bars heard become a loop.
        let cap = &self.capture;
        let b = LoopRow::capture_rect(r);
        let (bg, fg) = if cap.lit {
            (t.accent, t.on_accent)
        } else if cap.works {
            (t.raised, t.text)
        } else {
            (t.raised, t.text_faint)
        };
        cx.canvas.fill_round_rect(b, 8, bg);
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
            (t.accent, t.on_accent, t.on_accent)
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

    fn paint_card(&self, cx: &mut PaintCx, r: Rect, i: usize, look: &CardLook) {
        let t = cx.theme.clone();
        let p = LoopRow::parts(r, i, self.channels.len());
        let ch = &look.ch;
        cx.canvas
            .fill_rect(p.card.inset(-GAP / 2).intersect(body(r)), t.surface);
        cx.canvas
            .fill_round_rect(p.card, 10, t.surface.mix(t.raised, 0.4));
        if look.picked {
            cx.canvas.stroke_round_rect(p.card, 10, 2, t.accent);
        } else {
            cx.canvas.stroke_round_rect(p.card, 10, 1, t.outline);
        }
        let mut s = Shape::new(Rect::new(p.name.x + 6, p.name.y + 6, 14, 14));
        s.disc(p.name.x as f32 + 14.0, p.name.y as f32 + 13.0, 4.0);
        s.draw(cx.canvas, ch.colour);
        let ink = if ch.state == RingState::Empty {
            t.text_dim
        } else {
            t.text
        };
        cx.text(
            FontId::Bold,
            14.0,
            Rect::new(p.name.x + 23, p.name.y + 3, p.name.w - 28, 20),
            Align::Left,
            ink,
            &ch.name,
        );
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
            cx.canvas.fill_round_rect(p.mute, 8, b);
            let (sx, sy) = (p.mute.x + p.mute.w / 2 + 2, p.mute.y + p.mute.h / 2);
            speaker(cx, sx, sy, if muted { t.bad } else { t.text }, muted);
            let (fill, ink) = if ch.solo {
                (t.accent, t.on_accent)
            } else {
                (t.raised, t.text_dim)
            };
            cx.canvas.fill_round_rect(p.solo, 8, fill);
            cx.text(FontId::Bold, 14.0, p.solo, Align::Center, ink, "S");
        } else {
            // What an empty loop records from, beside its ring.
            let foot = Rect::new(
                p.mute.x - 2,
                p.ring.y,
                p.card.right() - p.mute.x - 2,
                p.ring.h,
            );
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

/// The row's area under its heading (the cards' backdrop).
fn body(r: Rect) -> Rect {
    Rect::new(r.x, r.y + HEAD_H, r.w, r.h - HEAD_H)
}

impl Widget for LoopRow {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        self.paint_head(cx, r);
        let looks = self.looks();
        for (i, look) in looks.iter().enumerate() {
            self.paint_card(cx, r, i, look);
        }
        // The Arm pill, and the panel's red edge or outline while armed.
        if let Some(pill) = self.arm {
            paint_arm(cx, r, pill);
        }
        *self.drawn.borrow_mut() = Some((r, self.head_look(), looks));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, head, looks) = drawn.as_ref()?;
        if *at != r || looks.len() != self.channels.len() || head.arm != self.arm {
            return None;
        }
        let now = self.looks();
        let mut parts = Vec::new();
        if *head != self.head_look() {
            parts.push(LoopRow::head_rect(r));
        }
        for (i, (a, b)) in looks.iter().zip(&now).enumerate() {
            if a != b {
                let card = LoopRow::parts(r, i, now.len()).card;
                parts.push(card.inset(-GAP / 2).intersect(body(r)));
            }
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
            Target::tap(
                "capture",
                0,
                self.capture.label.clone(),
                LoopRow::capture_rect(r),
            )
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
            LoopRow::pads_rect(r),
        ));
        let n = self.channels.len();
        for (i, ch) in self.channels.iter().enumerate() {
            let p = LoopRow::parts(r, i, n);
            out.push(Target::tap("loop", i, ch.name.clone(), p.name));
            let does = match ch.state {
                RingState::Empty => "record",
                RingState::Waiting(_) => "cancel",
                RingState::Recording => "close and play",
                RingState::Overdubbing => "play",
                RingState::Playing | RingState::Off => "overdub",
                RingState::Muted => "unmute",
            };
            out.push(Target::tap(
                "ring",
                i,
                format!("{} ring: {does}", ch.name),
                p.ring,
            ));
            if ch.state.has_sound() {
                out.push(Target::tap(
                    "loop-mute",
                    i,
                    format!("{} speaker", ch.name),
                    p.mute,
                ));
                out.push(Target::tap(
                    "loop-solo",
                    i,
                    format!("{} solo{}", ch.name, if ch.solo { " (on)" } else { "" }),
                    p.solo,
                ));
            }
        }
        out
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
                let act = if LoopRow::pads_rect(r).contains(x, y) {
                    LoopAct::PadKeys
                } else if LoopRow::capture_rect(r).contains(x, y) {
                    LoopAct::Capture
                } else {
                    let i = (0..n).find(|&i| LoopRow::parts(r, i, n).card.contains(x, y))?;
                    let p = LoopRow::parts(r, i, n);
                    let sound = self.channels[i].state.has_sound();
                    if sound && p.mute.contains(x, y) {
                        LoopAct::Mute(i)
                    } else if sound && p.solo.contains(x, y) {
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
                    } else if p.name.contains(x, y) {
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
                None
            }
            Touch::Up | Touch::Cancel => {
                if self.press.is_some_and(|p| p.finger == finger) {
                    self.press = None;
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
}
