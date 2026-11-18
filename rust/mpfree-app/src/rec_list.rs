//! The REC chip's target list (sequencer decision 1, mockup H): a tap on
//! the chip on Play's transport card opens it in place of the view. It
//! lists what can record, the armed one red with its pill, and a tap on a
//! row arms it; Rec song and Pin the arm are switches beside them; on the
//! right, what moves the arm and the chip's three looks.

use std::cell::RefCell;

use mpfree_ui::draw;
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use crate::play_widgets::{paint_pill, paint_rec_chip, ArmPill, RecChip, RecState};

/// The list's widget name and its page.
pub const REC_LIST: &str = "rec-list";
pub const REC_PAGE: &str = "Play · rec";

/// `Toggle` indices of the list's switches.
pub const TAP_SONG: usize = 0;
pub const TAP_PIN: usize = 1;

/// One thing REC can record.
#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub target: crate::rec_arm::Target,
    pub pill: ArmPill,
    /// "Notes · Track 1 · Clip 1", "Looper · Loop 1".
    pub title: String,
    /// What it records into, and what moves it.
    pub sub: String,
    /// Its settings in brief: "Count-in 1 bar".
    pub right: String,
}

/// The list: a tap on row `i` gives `Tab(i)`; on Rec song `Toggle(TAP_SONG)`,
/// on Pin the arm `Toggle(TAP_PIN)`; on Close `Select(0)`.
#[derive(Default)]
pub struct RecList {
    pub rows: Vec<ListRow>,
    /// Rec song is on (live to song), or in Perform comes with the take.
    pub song: bool,
    /// Perform (sequencer S4): Rec song comes with REC's take.
    pub perform: bool,
    /// Only a tap on Arm moves the arm.
    pub pinned: bool,
    /// What REC records ("Track 1 · Clip 1"), for the chip's looks.
    pub what: String,
    hits: RefCell<Hits>,
}

#[derive(Clone, Debug, Default)]
struct Hits {
    rows: Vec<Rect>,
    song: Rect,
    pin: Rect,
    close: Rect,
}

/// What moves the arm, as the right-hand panel lists it.
pub const MOVES: [&str; 8] = [
    "A tap on Arm: the loop row, the pattern, here",
    "SHIFT + Arm: both, in one pass",
    "The Sequencer tab: the notes",
    "The Looper tab: the loop",
    "Picking a track: its notes",
    "The home view, Mix, Synth and Pads leave it",
    "Nothing moves it while a take records",
    "Pinned: only a tap on Arm moves it",
];

const ROW_H: i32 = 66;
const SWITCH_H: i32 = 56;
const GAP: i32 = 10;

impl RecList {
    /// Where the parts go in `r`: the rows, the two switches, Close, and
    /// the right-hand column.
    fn layout(&self, r: Rect) -> (Hits, Rect) {
        let inner = r.inset_xy(12, 12);
        let close = Rect::new(inner.right() - 92, inner.y, 92, 36);
        let body = Rect::new(inner.x, inner.y + 48, inner.w, inner.h - 48);
        let left_w = body.w * 3 / 5;
        let right = Rect::new(body.x + left_w + 20, body.y, body.w - left_w - 20, body.h);
        let mut y = body.y;
        let mut rows = Vec::new();
        for _ in &self.rows {
            rows.push(Rect::new(body.x, y, left_w, ROW_H));
            y += ROW_H + GAP;
        }
        let song = Rect::new(body.x, y, left_w, SWITCH_H);
        let pin = Rect::new(body.x, y + SWITCH_H + GAP, left_w, SWITCH_H);
        (
            Hits {
                rows,
                song,
                pin,
                close,
            },
            right,
        )
    }

    fn paint_switch(cx: &mut PaintCx, r: Rect, label: &str, sub: &str, on: bool) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(r, 10, t.raised);
        let lw = cx.measure(FontId::Bold, 15.0, label);
        cx.text(
            FontId::Bold,
            15.0,
            Rect::new(r.x + 16, r.y, lw + 4, r.h),
            Align::Left,
            t.text,
            label,
        );
        cx.text(
            FontId::Sans,
            13.0,
            Rect::new(r.x + 16 + lw + 16, r.y, r.w - lw - 120, r.h),
            Align::Left,
            t.text_dim,
            sub,
        );
        let track = Rect::new(r.right() - 72, r.y + r.h / 2 - 13, 52, 26);
        cx.canvas
            .fill_round_rect(track, 13, if on { t.accent } else { t.surface });
        cx.canvas.stroke_round_rect(track, 13, 1, t.outline);
        let kx = if on { track.right() - 24 } else { track.x + 2 };
        cx.canvas.fill_round_rect(
            Rect::new(kx, track.y + 2, 22, 22),
            11,
            if on { t.bg } else { t.text_dim },
        );
    }
}

impl Widget for RecList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let (hits, right) = self.layout(r);
        // The head: the title, what it is for, and Close.
        let inner = r.inset_xy(12, 12);
        let tw = cx.measure(FontId::Bold, 18.0, "Record into");
        cx.text(
            FontId::Bold,
            18.0,
            Rect::new(inner.x + 4, inner.y, tw + 4, 36),
            Align::Left,
            t.text,
            "Record into",
        );
        cx.text(
            FontId::Sans,
            13.0,
            Rect::new(
                inner.x + tw + 24,
                inner.y,
                hits.close.x - inner.x - tw - 30,
                36,
            ),
            Align::Left,
            t.text_dim,
            "REC records what is armed. A tap on a row arms it.",
        );
        cx.canvas.fill_round_rect(hits.close, 10, t.raised);
        cx.text(
            FontId::Bold,
            15.0,
            hits.close,
            Align::Center,
            t.text,
            "Close",
        );
        // The rows: the armed one red.
        for (row, rr) in self.rows.iter().zip(&hits.rows) {
            let armed = row.pill != ArmPill::Arm;
            if armed {
                cx.canvas
                    .fill_round_rect(*rr, 10, t.bad.mix(t.raised, 0.82));
                cx.canvas.stroke_round_rect(*rr, 10, 1, t.bad);
                cx.canvas
                    .fill_round_rect(Rect::new(rr.x, rr.y + 8, 4, rr.h - 16), 2, t.bad);
            } else {
                cx.canvas.fill_round_rect(*rr, 10, t.raised);
            }
            paint_pill(
                cx,
                Rect::new(rr.x + 14, rr.y + rr.h / 2 - 15, 92, 30),
                row.pill,
            );
            let x = rr.x + 124;
            let rw = if row.right.is_empty() {
                0
            } else {
                cx.measure(FontId::Sans, 12.0, &row.right) + 16
            };
            cx.text(
                FontId::Bold,
                16.0,
                Rect::new(x, rr.y + 8, rr.right() - x - rw - 8, 26),
                Align::Left,
                t.text,
                &row.title,
            );
            cx.text(
                FontId::Sans,
                13.0,
                Rect::new(x, rr.y + 34, rr.right() - x - 12, 22),
                Align::Left,
                t.text_dim,
                &row.sub,
            );
            if rw > 0 {
                cx.text(
                    FontId::Sans,
                    12.0,
                    Rect::new(rr.right() - rw, rr.y + 8, rw - 12, 22),
                    Align::Right,
                    t.text_faint,
                    &row.right,
                );
            }
        }
        RecList::paint_switch(
            cx,
            hits.song,
            if self.perform {
                "Rec song with REC"
            } else {
                "Rec song"
            },
            if self.perform {
                "each take puts launches, mutes and scenes in the song"
            } else {
                "launches, mutes and scene changes go into the song"
            },
            self.song,
        );
        RecList::paint_switch(
            cx,
            hits.pin,
            "Pin the arm",
            "pages and track picks stop moving it",
            self.pinned,
        );
        // The right: what moves the arm, then the chip's looks.
        let (top, bottom) = right.take_top(right.h / 2 - 6);
        let bottom = bottom.take_top(12).1;
        cx.canvas.fill_round_rect(top, 10, t.bg.mix(t.surface, 0.4));
        draw::card(cx, top, "What moves the arm", "");
        let (_, body) = draw::card_head(top);
        let mut y = body.y;
        for (i, line) in MOVES.iter().enumerate() {
            let ink = if i < 5 { t.text } else { t.text_dim };
            cx.text(
                FontId::Sans,
                13.0,
                Rect::new(body.x, y, body.w, 22),
                Align::Left,
                ink,
                &format!("• {line}"),
            );
            y += 24;
        }
        draw::card(cx, bottom, "The chip", "");
        let (_, body) = draw::card_head(bottom);
        let what = if self.what.is_empty() {
            "Track 1 · Clip 1".to_string()
        } else {
            self.what.clone()
        };
        let cw = (body.w / 2).min(200);
        let ch = ((body.h - 2 * 8) / 3).min(52);
        for (i, (state, said)) in [
            (RecState::Armed, "Armed: REC is dark"),
            (RecState::Ready, "Ready: REC blinks"),
            (RecState::Recording, "Recording: REC blinks"),
        ]
        .into_iter()
        .enumerate()
        {
            let cr = Rect::new(body.x, body.y + i as i32 * (ch + 8), cw, ch);
            paint_rec_chip(
                cx,
                cr,
                &RecChip {
                    state,
                    what: what.clone(),
                    add: false,
                    pinned: false,
                },
            );
            cx.text(
                FontId::Sans,
                13.0,
                Rect::new(cr.right() + 12, cr.y, body.right() - cr.right() - 12, ch),
                Align::Left,
                t.text_dim,
                said,
            );
        }
        *self.hits.borrow_mut() = hits;
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (hits, _) = self.layout(r);
        let mut out: Vec<Target> = self
            .rows
            .iter()
            .zip(&hits.rows)
            .enumerate()
            .map(|(i, (row, rr))| Target::tap("row", i, row.title.clone(), *rr))
            .collect();
        out.push(Target::tap("switch", TAP_SONG, "Rec song", hits.song));
        out.push(Target::tap("switch", TAP_PIN, "Pin the arm", hits.pin));
        out.push(Target::tap("close", 0, "Close", hits.close));
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let (hits, _) = self.layout(r);
        if let Some(i) = hits.rows.iter().position(|h| h.contains(x, y)) {
            return Some(EventKind::Tab(i));
        }
        if hits.song.contains(x, y) {
            return Some(EventKind::Toggle(TAP_SONG));
        }
        if hits.pin.contains(x, y) {
            return Some(EventKind::Toggle(TAP_PIN));
        }
        hits.close.contains(x, y).then_some(EventKind::Select(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rec_arm::Target as Rt;

    fn list() -> RecList {
        RecList {
            rows: vec![
                ListRow {
                    target: Rt::Notes,
                    pill: ArmPill::Armed,
                    title: "Notes · Track 1 · Clip 1".into(),
                    sub: "the track the pads play".into(),
                    right: "Count-in 1 bar".into(),
                },
                ListRow {
                    target: Rt::Loop,
                    pill: ArmPill::Arm,
                    title: "Looper · Loop 1".into(),
                    sub: "empty".into(),
                    right: String::new(),
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_tap_on_a_row_a_switch_or_close_says_which() {
        let mut l = list();
        let r = Rect::new(12, 180, 998, 570);
        let t = l.targets(r);
        let at = |id: &str, k: usize| {
            let h = t.iter().find(|x| x.id == id && x.index == k).unwrap().rect;
            (h.x + h.w / 2, h.y + h.h / 2)
        };
        for (id, k, want) in [
            ("row", 0, EventKind::Tab(0)),
            ("row", 1, EventKind::Tab(1)),
            ("switch", TAP_SONG, EventKind::Toggle(TAP_SONG)),
            ("switch", TAP_PIN, EventKind::Toggle(TAP_PIN)),
            ("close", 0, EventKind::Select(0)),
        ] {
            let (x, y) = at(id, k);
            assert_eq!(l.touch(1, Touch::Down, x, y, r), Some(want), "{id} {k}");
        }
        // Everything fits in the list's rectangle.
        assert!(t.iter().all(|x| r.contains(x.rect.x, x.rect.y)
            && r.contains(x.rect.right() - 1, x.rect.bottom() - 1)));
    }
}
