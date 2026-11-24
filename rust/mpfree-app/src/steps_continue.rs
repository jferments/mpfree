//! Continue on Steps (composing tools, step 4): a kit track's beat carried
//! on from the Sequencer's Steps, as the note editor's Continue carries a
//! tune on. The Steps head's Continue opens it and asks at once; the time
//! engine makes the new bars off the music's threads and plays them in
//! place of the clip from the next bar line, and the grid shows the clip
//! as heard with them, the new bars' steps faint ([`crate::time::StepClip`]'s
//! `new_from`). While it is open:
//!
//! - the bar strip ([`BarStrip`]) takes the timing strip's place: your
//!   bars and each new one as a small picture, its job above it, the one
//!   the grid shows outlined; a tap pages the grid to it;
//! - the Continue panel ([`ContinuePanel`]) takes the step editor's
//!   place: Style, Bars and Ending (the note editor's rows, in the same
//!   words), the last 8 tries, and Try again, ‹ back a try, Cancel and
//!   Done;
//! - the Q-Links turn Similarity, Variation and Activity; Range has
//!   nothing to do on drums.
//!
//! Every change asks again, the last result playing on until the next is
//! ready; Done keeps the result as one take, Cancel goes back to the clip
//! as it was. The design is docs/dev/design/compose.md, its boards the
//! owner's "Continue on Steps" card (24 Nov 2026).

use mpfree_ui::draw::{card, card_head};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The page while Continue is open on Steps: the grid, the bar strip in
/// the timing strip's place, the panel in the step editor's.
pub const PAGE: &str = "Play · Beats · continue";
/// The bar strip's widget, and its height (the timing strip's).
pub const BARS: &str = "steps-continue-bars";
pub const BARS_H: i32 = crate::play_page::TIMING_STRIP_H;
/// The panel's widget, and its height (the step editor's).
pub const PANEL: &str = "steps-continue";
pub const PANEL_H: i32 = 196;

/// The panel's buttons, as `Tab` indices.
pub const TRY_AGAIN: usize = 0;
pub const BACK_A_TRY: usize = 1;
pub const CANCEL: usize = 2;
pub const DONE: usize = 3;
/// The panel's rows: Style, Bars and Ending (the note editor's first
/// three). A tap on a value's ‹ or › is `Step { index: row, delta }`, on
/// its name `Select(row)`.
pub const ROWS: usize = 3;

/// One bar in the strip: what it does, and its hits as small marks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StripBar {
    /// "yours", "vary", "land home".
    pub job: String,
    pub fill: bool,
    /// A new bar (faint until Done).
    pub new: bool,
    /// Each hit: its row (top to bottom, the grid's sounds in order), its
    /// step in the bar (0 to 15), loudness and the sound's colour.
    pub hits: Vec<(usize, usize, u8, Color)>,
    /// The rows the bar draws (the grid's sounds).
    pub rows: usize,
}

/// The bar strip: the bars, the one shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarStrip {
    pub bars: Vec<StripBar>,
    pub shown: usize,
}

const STRIP_LEFT: i32 = 128;

impl BarStrip {
    /// Each bar's tile: its label above and its picture.
    fn tiles(&self, r: Rect) -> Vec<(Rect, Rect)> {
        let n = self.bars.len().max(1) as i32;
        let room = (r.w - STRIP_LEFT - 12).max(0);
        let w = (room / n - 12).clamp(24, 168);
        (0..self.bars.len() as i32)
            .map(|i| {
                let x = r.x + STRIP_LEFT + i * (w + 12);
                (
                    Rect::new(x, r.y + 4, w, 16),
                    Rect::new(x, r.y + 20, w, r.h - 26),
                )
            })
            .collect()
    }
}

impl Widget for BarStrip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let cap = Rect::new(r.x + 14, r.y + 10, STRIP_LEFT - 20, 16);
        cx.text(FontId::Bold, 13.0, cap, Align::Left, t.text_dim, "BARS");
        let (one, two) = (
            Rect::new(cap.x, cap.y + 20, cap.w, 14),
            Rect::new(cap.x, cap.y + 36, cap.w, 14),
        );
        cx.text(
            FontId::Sans,
            12.0,
            one,
            Align::Left,
            t.text_faint,
            "tap one: page",
        );
        cx.text(FontId::Sans, 12.0, two, Align::Left, t.text_faint, "to it");
        for (i, (label, pic)) in self.tiles(r).into_iter().enumerate() {
            let bar = &self.bars[i];
            let shown = i == self.shown;
            let ink = if shown {
                t.accent
            } else if bar.new {
                t.text_dim
            } else {
                t.text
            };
            let job = if bar.fill {
                format!("{} {} · fill", i + 1, bar.job)
            } else {
                format!("{} {}", i + 1, bar.job)
            };
            let words = if pic.w >= 80 {
                job
            } else {
                (i + 1).to_string()
            };
            cx.text(FontId::Sans, 12.0, label, Align::Left, ink, &words);
            cx.canvas.fill_round_rect(pic, 8, t.bg);
            if shown {
                cx.canvas.stroke_round_rect(pic.inset(-2), 9, 2, t.accent);
            }
            let rows = bar.rows.clamp(1, 6) as i32;
            let inner = pic.inset_xy(6, 6);
            let cw = (inner.w / 16).max(1);
            let rh = (inner.h / rows).max(2);
            // The rows centred in the picture (a mark is 3 short of its row).
            let top = inner.y + ((inner.h - rows * rh + 3) / 2).max(0);
            for &(row, step, vel, c) in &bar.hits {
                if row as i32 >= rows || step >= 16 {
                    continue;
                }
                let alpha = 0.45 + 0.55 * f32::from(vel.min(127)) / 127.0;
                let alpha = if bar.new { alpha * 0.5 } else { alpha };
                let mark = Rect::new(
                    inner.x + step as i32 * cw,
                    top + row as i32 * rh,
                    (cw - 2).max(1),
                    (rh - 3).max(1),
                );
                cx.canvas
                    .fill_round_rect(mark, 2, c.with_alpha((alpha * 255.0).round() as u8));
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.tiles(r)
            .into_iter()
            .enumerate()
            .map(|(i, (_, pic))| {
                Target::tap("bar", i, format!("Bar {}: show it", i + 1), pic)
                    .disabled_if(i == self.shown)
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        self.tiles(r)
            .into_iter()
            .position(|(l, pic)| pic.contains(x, y) || l.contains(x, y))
            .filter(|&i| i != self.shown)
            .map(EventKind::Tab)
    }
}

/// What the panel shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelLook {
    /// Style, Bars and Ending, each with its value.
    pub rows: Vec<(String, String)>,
    /// The row the wheel steps.
    pub row: usize,
    /// The last 8 tries, newest first: ("Try 3 · Groove", "+4 bars").
    pub tries: Vec<(String, String)>,
    /// Which of `tries` plays in place of the clip.
    pub shown: Option<usize>,
    /// What is happening, in words: "Try 3 of 3 plays in place of the
    /// clip: Done keeps it".
    pub status: String,
    /// Why nothing can be heard or kept, when that is so.
    pub warn: bool,
    /// ‹ back a try: its label ("‹ Try 2"), None when there is none.
    pub back: Option<String>,
    /// A result to keep.
    pub ready: bool,
}

/// The panel in the step editor's place.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContinuePanel {
    pub look: PanelLook,
}

struct PanelGeom {
    /// Each row: its name and its value with ‹ and ›.
    rows: Vec<(Rect, Rect)>,
    tries: Rect,
    status: Rect,
    buttons: [Rect; 4],
}

impl ContinuePanel {
    fn geom(&self, r: Rect) -> PanelGeom {
        let (_, body) = card_head(r);
        let left = Rect::new(r.x + 12, body.y + 2, 360, body.h);
        let rows = (0..self.look.rows.len().min(ROWS) as i32)
            .map(|i| {
                let y = left.y + i * 50;
                (
                    Rect::new(left.x, y, 160, 40),
                    Rect::new(left.right() - 190, y, 190, 40),
                )
            })
            .collect();
        let tries = Rect::new(r.x + 398, body.y + 2, 260, body.h - 6);
        let bx = r.right() - 12 - 340;
        let bw = 164;
        let y2 = r.bottom() - 12 - 46;
        let y1 = y2 - 54;
        PanelGeom {
            rows,
            tries,
            status: Rect::new(bx, body.y, 340, y1 - body.y - 6),
            buttons: [
                Rect::new(bx, y1, bw, 46),
                Rect::new(bx + bw + 12, y1, bw, 46),
                Rect::new(bx, y2, bw, 46),
                Rect::new(bx + bw + 12, y2, bw, 46),
            ],
        }
    }
}

impl Widget for ContinuePanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let l = &self.look;
        cx.canvas.fill_rect(r, t.bg);
        card(cx, r, "Continue · drums", "");
        let g = self.geom(r);
        let (h, _) = card_head(r);
        let hint = Rect::new(r.x + 12, h.y, 360, h.h);
        cx.text(
            FontId::Sans,
            13.0,
            hint,
            Align::Right,
            t.text_faint,
            "wheel or arrows",
        );
        for (i, (name_r, value_r)) in g.rows.iter().enumerate() {
            let (name, value) = &l.rows[i];
            if i == l.row {
                let mark = Rect::new(name_r.x - 8, name_r.y + 4, 3, name_r.h - 8);
                cx.canvas.fill_round_rect(mark, 2, t.accent);
            }
            cx.text(FontId::Sans, 17.0, *name_r, Align::Left, t.text, name);
            cx.canvas.fill_round_rect(*value_r, 10, t.raised);
            let (lt, rest) = value_r.take_left(28);
            let (rt, mid) = rest.take_right(28);
            cx.text(FontId::Sans, 20.0, lt, Align::Center, t.text_dim, "‹");
            cx.text(FontId::Sans, 20.0, rt, Align::Center, t.text_dim, "›");
            cx.text(FontId::Bold, 16.0, mid, Align::Center, t.text, value);
        }
        // The tries, newest first; the one heard lit.
        let tries_head = Rect::new(g.tries.x, h.y, g.tries.w, h.h);
        cx.text(
            FontId::Bold,
            13.0,
            tries_head,
            Align::Left,
            t.text_dim,
            "TRIES",
        );
        cx.text(
            FontId::Sans,
            13.0,
            tries_head,
            Align::Right,
            t.text_faint,
            "the last 8",
        );
        for (i, (name, bars)) in l.tries.iter().enumerate().take(4) {
            let row = Rect::new(g.tries.x, g.tries.y + i as i32 * 42, g.tries.w, 36);
            let lit = l.shown == Some(i);
            if lit {
                cx.canvas.fill_round_rect(row, 8, t.raised);
            }
            let dot_c = if lit { t.good } else { t.text_faint };
            cx.canvas
                .fill_round_rect(Rect::new(row.x + 10, row.y + 14, 8, 8), 4, dot_c);
            let ink = if lit { t.text } else { t.text_dim };
            let text_r = Rect::new(row.x + 26, row.y, row.w - 36, row.h);
            cx.text(FontId::Sans, 15.0, text_r, Align::Left, ink, name);
            cx.text(FontId::Mono, 14.0, text_r, Align::Right, t.text_dim, bars);
        }
        // What is happening, then the buttons.
        let (title_r, line_r) = g.status.take_top(22);
        cx.text(FontId::Bold, 15.0, title_r, Align::Left, t.text, "Continue");
        let ink = if l.warn { t.warn } else { t.text_dim };
        cx.text(
            FontId::Sans,
            13.0,
            line_r.take_top(20).0,
            Align::Left,
            ink,
            &l.status,
        );
        let back = l.back.clone().unwrap_or_else(|| "‹".to_string());
        let labels = ["Try again", back.as_str(), "Cancel", "Done"];
        let usable = [true, l.back.is_some(), true, l.ready];
        for (k, b) in g.buttons.iter().enumerate() {
            let lit = k == DONE && l.ready;
            let fill = if lit { t.picked() } else { t.raised };
            cx.canvas.fill_round_rect(*b, t.round(10), fill);
            let ink = if lit {
                t.ink_on(fill)
            } else {
                t.button_ink(usable[k])
            };
            cx.text(FontId::Bold, 16.0, *b, Align::Center, ink, labels[k]);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geom(r);
        let l = &self.look;
        let mut out = Vec::new();
        for (i, (name_r, value_r)) in g.rows.iter().enumerate() {
            let name = &l.rows[i].0;
            out.push(Target::tap("row", i, format!("{name}: the wheel"), *name_r));
            let (lt, rest) = value_r.take_left(value_r.w / 2);
            out.push(Target::tap("less", i, format!("{name} ‹"), lt));
            out.push(Target::tap("more", i, format!("{name} ›"), rest));
        }
        let names = ["Try again", "Back a try", "Cancel", "Done"];
        let usable = [true, l.back.is_some(), true, l.ready];
        for (k, b) in g.buttons.iter().enumerate() {
            out.push(Target::tap("button", k, names[k], *b).disabled_if(!usable[k]));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let g = self.geom(r);
        for (i, (name_r, value_r)) in g.rows.iter().enumerate() {
            if name_r.contains(x, y) {
                return Some(EventKind::Select(i));
            }
            if value_r.contains(x, y) {
                let delta = if x < value_r.x + value_r.w / 2 { -1 } else { 1 };
                return Some(EventKind::Step { index: i, delta });
            }
        }
        g.buttons
            .iter()
            .position(|b| b.contains(x, y))
            .map(EventKind::Tab)
    }
}
