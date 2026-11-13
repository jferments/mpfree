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
use crate::draw;
use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::ui::{EventKind, PaintCx, Touch, Widget};

/// The column's width on the Play page, in pixels.
pub const COLUMN_WIDTH: i32 = 168;
/// The master bar's width on the Play page, in pixels.
pub const MASTER_WIDTH: i32 = 66;
/// Pixels of drag per detent of a card's knob.
pub const DRAG_DETENT: i32 = 8;

/// Each knob's colour when its target has none of its own: blue, green,
/// yellow and pink, as in the Play mockups.
pub const KNOB_COLOURS: [Color; 4] = draw::KNOB;

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
        QLinkCard {
            name: name.to_string(),
            scope: scope.to_string(),
            value: value.to_string(),
            pos,
            ..Default::default()
        }
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
    /// The bank shown ("Play", "Mixer", "Arp"): not drawn (the cards fill
    /// the column, as in the mockups), kept for the log and tests.
    pub title: String,
    drag: Option<(usize, i32)>,
}

impl Default for QLinkColumn {
    fn default() -> Self {
        QLinkColumn::new()
    }
}

/// Space between the cards.
const CARD_GAP: i32 = 10;

impl QLinkColumn {
    pub fn new() -> QLinkColumn {
        QLinkColumn {
            cards: Default::default(),
            touched: [false; 4],
            title: "Q-LINKS".to_string(),
            drag: None,
        }
    }

    /// The four cards, filling the column top to bottom.
    pub fn card_rects(r: Rect) -> [Rect; 4] {
        let h = ((r.h - 3 * CARD_GAP) / 4).max(0);
        std::array::from_fn(|i| Rect::new(r.x, r.y + i as i32 * (h + CARD_GAP), r.w, h))
    }

    /// The card under (x, y), if any.
    pub fn card_at(r: Rect, x: i32, y: i32) -> Option<usize> {
        QLinkColumn::card_rects(r)
            .iter()
            .position(|c| c.contains(x, y))
    }

    /// The colour knob `k`'s card draws in.
    pub fn colour(&self, k: usize) -> Color {
        self.cards[k].color.unwrap_or(KNOB_COLOURS[k % 4])
    }
}

impl Widget for QLinkColumn {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        for (k, cr) in QLinkColumn::card_rects(r).into_iter().enumerate() {
            let card = &self.cards[k];
            let col = self.colour(k);
            let on = self.touched[k];
            cx.canvas
                .fill_round_rect(cr, t.radius, if on { t.raised } else { t.surface });
            if on {
                cx.canvas.stroke_round_rect(cr, t.radius, 2, col);
            }
            let inner = cr.inset_xy(14, 10);
            let (top, rest) = inner.take_top(18);
            if card.is_free() {
                cx.text(
                    FontId::Bold,
                    13.0,
                    top,
                    Align::Left,
                    t.text_faint,
                    &format!("Q{}", k + 1),
                );
                cx.text(
                    FontId::Sans,
                    13.0,
                    rest,
                    Align::Center,
                    t.text_faint,
                    "free",
                );
                continue;
            }
            cx.canvas
                .fill_round_rect(Rect::new(cr.x, cr.y + 14, 4, (cr.h - 28).max(0)), 2, col);
            cx.text(
                FontId::Bold,
                13.0,
                top,
                Align::Left,
                col,
                &format!("Q{}", k + 1),
            );
            cx.text(
                FontId::Sans,
                12.0,
                top.take_left(28).1,
                Align::Right,
                t.text_faint,
                &card.scope.to_uppercase(),
            );
            let (name, rest) = rest.take_top(24);
            cx.text(FontId::Bold, 16.0, name, Align::Left, t.text, &card.name);
            // The knob: 270 degrees of dots, open at the bottom.
            let d = rest.h.min(rest.w - 10).max(20);
            let (ox, oy) = (rest.x + rest.w / 2, rest.y + rest.h / 2 + 2);
            let rad = d as f32 / 2.0 - 5.0;
            draw::ring(cx, ox, oy, rad, card.pos, card.bipolar, col);
            let vr = Rect::new(ox - rad as i32, oy - 12, 2 * rad as i32, 24);
            cx.text(FontId::Bold, 16.0, vr, Align::Center, t.text, &card.value);
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
                Some(EventKind::Step {
                    index: k,
                    delta: detents,
                })
            }
            _ => {
                self.drag = None;
                None
            }
        }
    }
}

/// The master strip, as the mockups' wide meter: "MASTER", a dB scale, the
/// two sides' meters in segments with their peaks, the safety limiter's
/// gain reduction beside them, the fader's handle across them, then the
/// level and "LIM" under. Levels sit on the meter's scale
/// ([`draw::meter_frac`]: -48 dB at the bottom to +6 dB at the top), the
/// handle too, so a drag on the bar gives the fader's place on that scale
/// as [`EventKind::Value`] (turn it into dB with [`draw::meter_db`]).
pub struct MasterBar {
    /// Each side's level now, dBFS (-120 for silence).
    pub peak_db: [f32; 2],
    /// Each side's held peak, dBFS.
    pub hold_db: [f32; 2],
    /// The master fader, dB.
    pub fader_db: f32,
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

/// How far the held peaks fall at each [`MasterBar::feed`], dB.
const HOLD_FALL_DB: f32 = 0.6;
/// The limiter's scale: this many dB of gain reduction fill its bar.
const LIMIT_SCALE_DB: f32 = 12.0;

impl MasterBar {
    pub fn new() -> MasterBar {
        MasterBar {
            peak_db: [-120.0; 2],
            hold_db: [-120.0; 2],
            fader_db: 0.0,
            text: String::new(),
            limit_db: 0.0,
        }
    }

    /// New levels: the held peaks follow them up at once and fall slowly.
    /// True when anything drawn changed.
    pub fn feed(&mut self, db: [f32; 2]) -> bool {
        let before = (self.peak_db, self.hold_db);
        for ch in 0..2 {
            self.peak_db[ch] = db[ch];
            self.hold_db[ch] = db[ch].max(self.hold_db[ch] - HOLD_FALL_DB);
        }
        let seen = |x: f32| (x.max(-48.0) * 2.0).round();
        (0..2).any(|ch| {
            seen(before.0[ch]) != seen(self.peak_db[ch])
                || seen(before.1[ch]) != seen(self.hold_db[ch])
        })
    }

    /// The meters' track inside the bar `r`.
    pub fn track(r: Rect) -> Rect {
        let inner = r.inset_xy(8, 12);
        let (_, rest) = inner.take_top(18);
        let (_, rest) = rest.take_bottom(46);
        rest.inset_xy(0, 8)
    }

    /// Where `db` sits on the track (+6 dB at the top, -48 dB at the bottom).
    pub fn y_of(track: Rect, db: f32) -> i32 {
        track.bottom() - (draw::meter_frac(db) * track.h as f32) as i32
    }
}

impl Widget for MasterBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let inner = r.inset_xy(8, 12);
        let (top, _) = inner.take_top(18);
        cx.text(FontId::Bold, 10.5, top, Align::Center, t.text_dim, "MASTER");
        let (bottom, _) = inner.take_bottom(46);
        let bars = MasterBar::track(r);
        // The scale on the left, two bars, then the limiter's reduction.
        let scale = Rect::new(bars.x, bars.y, 20, bars.h);
        for db in [0.0f32, -6.0, -12.0, -24.0, -36.0] {
            let y = MasterBar::y_of(bars, db);
            let txt = format!("{}", db.abs() as i32);
            cx.text(
                FontId::Sans,
                10.0,
                Rect::new(scale.x, y - 6, scale.w, 12),
                Align::Right,
                t.text_faint,
                &txt,
            );
        }
        let bx = scale.right() + 4;
        let bw = 9;
        for side in 0..2 {
            let br = Rect::new(bx + side as i32 * (bw + 2), bars.y, bw, bars.h);
            draw::meter_bar(cx, br, self.peak_db[side], self.hold_db[side], true);
        }
        let gx = bx + 2 * bw + 6;
        cx.canvas
            .fill_rect(Rect::new(gx, bars.y, 5, bars.h), t.raised);
        if self.limit_db > 0.05 {
            let gh = (self.limit_db / LIMIT_SCALE_DB * bars.h as f32) as i32;
            cx.canvas
                .fill_rect(Rect::new(gx, bars.y, 5, gh.clamp(3, bars.h.max(3))), t.warn);
        }
        let fy = MasterBar::y_of(bars, self.fader_db);
        cx.canvas
            .fill_round_rect(Rect::new(bx - 3, fy - 3, 2 * bw + 8, 6), 3, t.text);
        let (b1, b2) = bottom.take_top(22);
        cx.text(FontId::Bold, 14.0, b1, Align::Center, t.text, &self.text);
        if self.limit_db > 0.05 {
            cx.text(
                FontId::Sans,
                10.5,
                b2,
                Align::Center,
                t.warn,
                &format!("LIM {:.1}", self.limit_db),
            );
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
        let f = (tr.bottom() - y) as f32 / tr.h as f32;
        Some(EventKind::Value(f.clamp(0.0, 1.0)))
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
        assert!(
            cards.windows(2).all(|w| w[0].bottom() < w[1].y),
            "{cards:?}"
        );
        assert!(cards[3].bottom() <= column().bottom());
        assert!(
            cards.iter().all(|c| c.h > 140 && c.w == COLUMN_WIDTH),
            "{cards:?}"
        );
        let mid = cards[2];
        assert_eq!(
            QLinkColumn::card_at(column(), mid.x + 10, mid.y + 10),
            Some(2)
        );
        assert_eq!(
            QLinkColumn::card_at(column(), mid.x + 10, cards[1].bottom() + 4),
            None,
            "the gap is not a card"
        );
    }

    #[test]
    fn a_drag_up_a_card_turns_its_knob_by_detents() {
        let mut q = QLinkColumn::new();
        let r = column();
        let c = QLinkColumn::card_rects(r)[1];
        let y = c.y + c.h / 2;
        assert_eq!(q.touch(1, Touch::Down, c.x + 20, y, r), None);
        assert_eq!(
            q.touch(1, Touch::Move, c.x + 20, y - DRAG_DETENT + 1, r),
            None,
            "less than a detent"
        );
        assert_eq!(
            q.touch(1, Touch::Move, c.x + 20, y - 3 * DRAG_DETENT, r),
            Some(EventKind::Step { index: 1, delta: 3 })
        );
        assert_eq!(
            q.touch(1, Touch::Move, c.x + 20, y - DRAG_DETENT, r),
            Some(EventKind::Step {
                index: 1,
                delta: -2
            })
        );
        assert_eq!(q.touch(1, Touch::Up, c.x + 20, y, r), None);
        assert_eq!(
            q.touch(2, Touch::Move, c.x + 20, y - 40, r),
            None,
            "no drag after the lift"
        );
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
        assert_eq!(
            m.touch(1, Touch::Down, tr.x, tr.y, r),
            Some(EventKind::Value(1.0))
        );
        assert_eq!(
            m.touch(1, Touch::Move, tr.x, tr.bottom(), r),
            Some(EventKind::Value(0.0))
        );
        assert_eq!(m.touch(1, Touch::Up, tr.x, tr.y, r), None);
        assert_eq!(MasterBar::y_of(tr, 6.0), tr.y);
        assert_eq!(MasterBar::y_of(tr, -48.0), tr.bottom());
        // The handle sits where a drag to it would put the fader.
        let y = MasterBar::y_of(tr, -1.5);
        let Some(EventKind::Value(f)) = m.touch(1, Touch::Down, tr.x, y, r) else {
            panic!("a value");
        };
        assert!(
            (draw::meter_db(f) + 1.5).abs() < 0.2,
            "{}",
            draw::meter_db(f)
        );
        // Held peaks follow up at once and fall slowly.
        assert!(m.feed([-6.0, -12.0]));
        assert!(m.feed([-30.0, -30.0]));
        assert_eq!(m.hold_db, [-6.0 - HOLD_FALL_DB, -12.0 - HOLD_FALL_DB]);
    }
}
