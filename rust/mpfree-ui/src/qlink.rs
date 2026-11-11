//! The Q-Link column and the master bar: the right edge of the Play page
//! (layout spec, "A new Play page"), beside the MPC's four Q-Link knobs.
//!
//! [`QLinkColumn`] shows what each knob does on the page shown: four cards,
//! knob 1 at the top, each with its number, where its target lives ("DRUMS",
//! "PAD 1 · KICK", a macro's name), the target's name, a ring of dots lit as
//! far as the value goes, and the value. A card whose knob is touched is
//! outlined. The card shape is the one the mixer (`mpfree_mixer_ui::qlinks`)
//! and the mod thread's bindings (`mpfree_mod`) fill in, so every page draws
//! its knobs the same way.
//!
//! A finger dragged up or down a card turns that knob, a detent per
//! [`DRAG_DETENT`] pixels, as [`EventKind::Step`] with the card's index: the
//! app treats it exactly as a turn of the hardware knob.
//!
//! [`MasterBar`] is the thin master strip: a stereo meter (dB below full
//! scale), the level's handle, the level as text, and the limiter's gain
//! reduction when it works. A drag on it sets the level (0 to 1, as
//! [`EventKind::Value`]).

use crate::color::Color;
use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::ui::{EventKind, PaintCx, Touch, Widget};

/// The column's width on the Play page, in pixels.
pub const COLUMN_WIDTH: i32 = 168;
/// The master bar's width on the Play page, in pixels.
pub const MASTER_WIDTH: i32 = 66;
/// Pixels of drag per detent of a card's knob.
pub const DRAG_DETENT: i32 = 8;
/// Dots on a card's ring.
const RING_DOTS: usize = 28;
/// The ring's sweep, in degrees, from bottom left round to bottom right.
const RING_SWEEP: f32 = 270.0;

/// Each knob's colour when its target has none of its own: blue, green,
/// yellow and pink, as in the Play mockups.
pub const KNOB_COLOURS: [Color; 4] =
    [Color::rgb(0x6EA8FE), Color::rgb(0x3DDC97), Color::rgb(0xF5C04A), Color::rgb(0xE07BD8)];

/// One knob's card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QLinkCard {
    /// The target's name: "Swing", "Tune", "Wash". Empty when the knob has
    /// nothing to move on this page.
    pub name: String,
    /// Where the target lives, in capitals on the card: "DRUMS",
    /// "PAD 1 · KICK", "LOOPER", "MACRO 2".
    pub scope: String,
    /// The value as the target writes it: "56 %", "-2 st", "8.2 kHz".
    pub value: String,
    /// How far round the ring is lit, 0 to 1.
    pub pos: f32,
    /// Lit from the top of the ring both ways (pan, tune, a nudge).
    pub bipolar: bool,
    /// The target's own colour (a macro's); `None` takes the knob's.
    pub color: Option<Color>,
}

impl QLinkCard {
    pub fn new(name: &str, scope: &str, value: &str, pos: f32) -> QLinkCard {
        QLinkCard { name: name.to_string(), scope: scope.to_string(), value: value.to_string(), pos, ..Default::default() }
    }

    /// The knob moves nothing here.
    pub fn is_free(&self) -> bool {
        self.name.is_empty()
    }
}

/// The four Q-Link cards, knob 1 at the top.
pub struct QLinkColumn {
    pub cards: [QLinkCard; 4],
    /// The knobs a finger rests on (the MPC's Q-Links sense touch).
    pub touched: [bool; 4],
    /// The bank shown, for the column's heading: "Q-LINKS · MACROS 1-4".
    pub title: String,
    drag: Option<(usize, i32)>,
}

impl Default for QLinkColumn {
    fn default() -> Self {
        QLinkColumn::new()
    }
}

impl QLinkColumn {
    pub fn new() -> QLinkColumn {
        QLinkColumn { cards: Default::default(), touched: [false; 4], title: "Q-LINKS".to_string(), drag: None }
    }

    /// The column's heading line, then the four cards under it.
    pub fn card_rects(r: Rect) -> [Rect; 4] {
        let (_, body) = r.take_top(HEADING);
        let gap = 8;
        let h = ((body.h - 3 * gap) / 4).max(0);
        std::array::from_fn(|i| Rect::new(body.x, body.y + i as i32 * (h + gap), body.w, h))
    }

    /// The card under (x, y), if any.
    pub fn card_at(r: Rect, x: i32, y: i32) -> Option<usize> {
        QLinkColumn::card_rects(r).iter().position(|c| c.contains(x, y))
    }

    /// The colour knob `k`'s card draws in.
    pub fn colour(&self, k: usize) -> Color {
        self.cards[k].color.unwrap_or(KNOB_COLOURS[k % 4])
    }
}

const HEADING: i32 = 22;

impl Widget for QLinkColumn {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let (head, _) = r.take_top(HEADING);
        cx.text(FontId::Bold, t.size_small - 2.0, head, Align::Left, t.text_dim, &self.title.to_uppercase());
        for (k, cr) in QLinkColumn::card_rects(r).into_iter().enumerate() {
            let card = &self.cards[k];
            let c = self.colour(k);
            cx.canvas.fill_round_rect(cr, t.radius, t.surface);
            if self.touched[k] {
                cx.canvas.stroke_round_rect(cr, t.radius, 2, c);
            }
            if card.is_free() {
                let inner = cr.inset(12);
                cx.text(FontId::Bold, t.size_small, inner.take_top(20).0, Align::Left, t.text_faint, &format!("Q{}", k + 1));
                cx.text(FontId::Sans, t.size_small, inner, Align::Center, t.text_faint, "free");
                continue;
            }
            // The colour bar down the left edge.
            cx.canvas.fill_round_rect(Rect::new(cr.x, cr.y + 10, 3, (cr.h - 20).max(0)), 1, c);
            let inner = cr.inset_xy(14, 10);
            let (line, rest) = inner.take_top(18);
            cx.text(FontId::Bold, t.size_small - 2.0, line, Align::Left, c, &format!("Q{}", k + 1));
            cx.text(FontId::Bold, t.size_small - 4.0, line.take_left(26).1, Align::Right, t.text_faint,
                    &card.scope.to_uppercase());
            let (name, rest) = rest.take_top(26);
            cx.text(FontId::Bold, t.size_body, name, Align::Left, t.text, &card.name);
            ring(cx, rest, card.pos, card.bipolar, c, t.raised);
            let d = ring_diameter(rest);
            let vr = Rect::new(rest.x, rest.y + (rest.h - 22) / 2 + 2, rest.w, 22);
            let px = if d < 90 { t.size_small - 2.0 } else { t.size_small };
            cx.text(FontId::Bold, px, vr, Align::Center, t.text, &card.value);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                self.drag = QLinkColumn::card_at(r, x, y).map(|k| (k, y));
                None
            }
            Touch::Move => {
                let (k, from) = self.drag?;
                let detents = (from - y) / DRAG_DETENT;
                if detents == 0 {
                    return None;
                }
                self.drag = Some((k, from - detents * DRAG_DETENT));
                Some(EventKind::Step { index: k, delta: detents })
            }
            _ => {
                self.drag = None;
                None
            }
        }
    }
}

fn ring_diameter(r: Rect) -> i32 {
    (r.w.min(r.h) - 8).clamp(0, 104)
}

/// A ring of dots in `r`, lit as far as `pos` (from the top both ways when
/// `bipolar`).
fn ring(cx: &mut PaintCx, r: Rect, pos: f32, bipolar: bool, lit: Color, unlit: Color) {
    let d = ring_diameter(r) as f32;
    if d < 24.0 {
        return;
    }
    let (ox, oy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    let rad = d / 2.0 - 4.0;
    let dot = if d < 80.0 { 4 } else { 6 };
    let pos = pos.clamp(0.0, 1.0);
    for i in 0..RING_DOTS {
        let f = i as f32 / (RING_DOTS - 1) as f32;
        let on = if bipolar { (f - 0.5) * (pos - 0.5) >= 0.0 && (f - 0.5).abs() <= (pos - 0.5).abs() } else { f <= pos };
        // 0 is the bottom left, sweeping clockwise through the top.
        let a = (135.0 + f * RING_SWEEP).to_radians();
        let (x, y) = (ox + rad * a.cos(), oy + rad * a.sin());
        let c = if on && pos > 0.0 || on && bipolar { lit } else { unlit };
        cx.canvas.fill_round_rect(Rect::new(x as i32 - dot / 2, y as i32 - dot / 2, dot, dot), dot / 2, c);
    }
}

/// The master strip: a stereo meter, the level's handle and the level.
pub struct MasterBar {
    /// Each side's peak, dB below full scale (0 is full scale, -60 or
    /// lower is silence).
    pub peak_db: [f32; 2],
    /// The level, 0 to 1 along the bar.
    pub level: f32,
    /// The level as text: "-1.5".
    pub text: String,
    /// The safety limiter's gain reduction, dB (0 when it rests).
    pub limit_db: f32,
}

impl Default for MasterBar {
    fn default() -> Self {
        MasterBar::new()
    }
}

/// The meter's range: 0 dB at the top to this many dB below at the bottom.
pub const METER_FLOOR_DB: f32 = 48.0;

impl MasterBar {
    pub fn new() -> MasterBar {
        MasterBar { peak_db: [-120.0; 2], level: 0.75, text: String::new(), limit_db: 0.0 }
    }

    /// The meter's track inside the bar.
    pub fn track(r: Rect) -> Rect {
        let inner = r.inset_xy(10, 0);
        let (_, rest) = inner.take_top(36);
        let (_, body) = rest.take_bottom(48);
        body
    }

    /// Where `db` sits on the track (0 dB at the top).
    fn y_of(track: Rect, db: f32) -> i32 {
        let f = (-db / METER_FLOOR_DB).clamp(0.0, 1.0);
        track.y + (f * track.h as f32) as i32
    }
}

impl Widget for MasterBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(r, t.radius, t.surface);
        let (head, _) = r.take_top(36);
        cx.text(FontId::Bold, t.size_small - 4.0, head, Align::Center, t.text_dim, "MASTER");
        let tr = MasterBar::track(r);
        let w = (tr.w - 10) / 2;
        for side in 0..2 {
            let bar = Rect::new(tr.x + side * (w + 4), tr.y, w.max(1), tr.h);
            cx.canvas.fill_rect(bar, t.raised);
            let top = MasterBar::y_of(tr, self.peak_db[side as usize]);
            if self.peak_db[side as usize] > -METER_FLOOR_DB {
                let lit = Rect::new(bar.x, top, bar.w, bar.bottom() - top);
                let c = if self.peak_db[side as usize] > -0.5 { t.bad } else if self.peak_db[side as usize] > -6.0 { t.warn } else { t.good };
                cx.canvas.fill_rect(lit, c);
            }
        }
        // The level's handle across both sides.
        let hy = tr.y + ((1.0 - self.level.clamp(0.0, 1.0)) * tr.h as f32) as i32;
        cx.canvas.fill_round_rect(Rect::new(tr.x - 4, hy - 3, tr.w + 8, 6), 3, t.text);
        let (foot, _) = r.take_bottom(48);
        let (val, lim) = foot.take_top(26);
        cx.text(FontId::Bold, t.size_small, val, Align::Center, t.text, &self.text);
        if self.limit_db > 0.05 {
            cx.text(FontId::Bold, t.size_small - 4.0, lim, Align::Center, t.warn, &format!("LIM {:.1}", self.limit_db));
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, _x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if !matches!(phase, Touch::Down | Touch::Move) {
            return None;
        }
        let tr = MasterBar::track(r);
        if tr.h <= 0 {
            return None;
        }
        self.level = (1.0 - (y - tr.y) as f32 / tr.h as f32).clamp(0.0, 1.0);
        Some(EventKind::Value(self.level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column() -> Rect {
        Rect::new(1100, 84, COLUMN_WIDTH, 666)
    }

    #[test]
    fn four_cards_fill_the_column_in_order() {
        let cards = QLinkColumn::card_rects(column());
        assert!(cards.windows(2).all(|w| w[0].bottom() < w[1].y), "{cards:?}");
        assert!(cards[3].bottom() <= column().bottom());
        assert!(cards.iter().all(|c| c.h > 140 && c.w == COLUMN_WIDTH), "{cards:?}");
        let mid = cards[2];
        assert_eq!(QLinkColumn::card_at(column(), mid.x + 10, mid.y + 10), Some(2));
        assert_eq!(QLinkColumn::card_at(column(), mid.x + 10, column().y + 2), None, "the heading is not a card");
    }

    #[test]
    fn a_drag_up_a_card_turns_its_knob_by_detents() {
        let mut q = QLinkColumn::new();
        let r = column();
        let c = QLinkColumn::card_rects(r)[1];
        let y = c.y + c.h / 2;
        assert_eq!(q.touch(1, Touch::Down, c.x + 20, y, r), None);
        assert_eq!(q.touch(1, Touch::Move, c.x + 20, y - DRAG_DETENT + 1, r), None, "less than a detent");
        assert_eq!(q.touch(1, Touch::Move, c.x + 20, y - 3 * DRAG_DETENT, r), Some(EventKind::Step { index: 1, delta: 3 }));
        assert_eq!(q.touch(1, Touch::Move, c.x + 20, y - DRAG_DETENT, r), Some(EventKind::Step { index: 1, delta: -2 }));
        assert_eq!(q.touch(1, Touch::Up, c.x + 20, y, r), None);
        assert_eq!(q.touch(2, Touch::Move, c.x + 20, y - 40, r), None, "no drag after the lift");
    }

    #[test]
    fn a_card_takes_its_target_colour_or_the_knobs() {
        let mut q = QLinkColumn::new();
        q.cards[0] = QLinkCard::new("Wash", "Macro 1", "80 %", 0.8);
        q.cards[0].color = Some(Color::rgb(0xC77DBA));
        assert_eq!(q.colour(0), Color::rgb(0xC77DBA));
        assert_eq!(q.colour(1), KNOB_COLOURS[1]);
        assert!(q.cards[2].is_free());
    }

    #[test]
    fn the_master_bar_maps_a_drag_to_the_level() {
        let mut m = MasterBar::new();
        let r = Rect::new(1032, 84, MASTER_WIDTH, 666);
        let tr = MasterBar::track(r);
        assert_eq!(m.touch(1, Touch::Down, tr.x, tr.y, r), Some(EventKind::Value(1.0)));
        assert_eq!(m.touch(1, Touch::Move, tr.x, tr.bottom(), r), Some(EventKind::Value(0.0)));
        assert_eq!(m.touch(1, Touch::Up, tr.x, tr.y, r), None);
        assert_eq!(MasterBar::y_of(tr, 0.0), tr.y);
        assert_eq!(MasterBar::y_of(tr, -METER_FLOOR_DB), tr.bottom());
    }
}
