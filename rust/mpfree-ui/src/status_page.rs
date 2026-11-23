//! The System status page (layout spec, section 2a; System status phase
//! 2): is anything wrong, which part, and what to do. It opens over the
//! page, under the top bar, from the status chip or Prefs.
//!
//! The widget only draws what it is given ([`StatusView`]): the app builds
//! that from its status board, so this crate needs nothing of the board.
//!
//! - **The head:** one line in the worst problem's colour, and how many
//!   problems there are now and earlier; the meters on its right.
//! - **The systems, on the left:** always all nine, each with a dot (a
//!   grey ring for one not in use), a line of plain words and a badge.
//! - **The picked system, on the right:** its problems as cards, worst and
//!   newest first. A card happening now, or opened, shows in full: what
//!   happened, what to try, its button and a line of numbers; one that
//!   ended folds to a line, and a tap opens it. Then the system's own
//!   numbers.
//! - **Now | This session**, at the top right.
//! - **Save a report**, at the foot of the cards panel on every system.

use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::theme::Tone;
use crate::ui::{EventKind, PaintCx, Target, Touch, Widget};
use std::cell::RefCell;

/// `Tab(NOW)` and `Tab(SESSION)`: the switch.
pub const NOW: usize = 100;
pub const SESSION: usize = 101;
/// `Tab(CARD + i)`: card `i` tapped (it opens or folds).
pub const CARD: usize = 200;
/// `Tab(BUTTON + i)`: card `i`'s button.
pub const BUTTON: usize = 300;
/// `Tab(SAVE_REPORT)`: Save a report, at the foot of the cards panel.
pub const SAVE_REPORT: usize = 400;

/// The head's height, and a system row's.
const HEAD_H: i32 = 76;
const ROW_H: i32 = 60;
/// The systems' column.
const SYSTEMS_W: i32 = 440;
/// A folded card's height.
const FOLDED_H: i32 = 52;

/// One system's row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemRow {
    pub name: String,
    /// Plain words: "Juno-X isn't plugged in", "Working".
    pub line: String,
    /// Its dot's colour; None: a grey ring (not in use).
    pub tone: Option<Tone>,
    /// Drawn as a ring: what made it this colour ended and hasn't been
    /// seen.
    pub ring: bool,
    /// Problems listed for it now (the badge; 0: none).
    pub badge: usize,
}

/// One problem's card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub title: String,
    pub tone: Tone,
    /// Happening now (always shown in full).
    pub now: bool,
    /// When and how often: "3 times · last 12 s ago · first at 21:14".
    pub when: String,
    pub what: String,
    pub to_try: String,
    /// The button's words, where MPFree can do it for you.
    pub button: Option<String>,
}

/// What the page shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusView {
    /// "Everything is working", or the worst problem's title.
    pub head: String,
    pub head_tone: Tone,
    /// "1 now · 2 earlier".
    pub counts: String,
    /// Each meter: its name, its value in words, how full (0 to 1), its
    /// colour.
    pub meters: Vec<(String, String, f32, Tone)>,
    pub systems: Vec<SystemRow>,
    pub picked: usize,
    /// The switch: only what is happening now.
    pub now_only: bool,
    /// The picked system's problems, worst and newest first.
    pub cards: Vec<Card>,
    /// The picked system's own numbers.
    pub numbers: Vec<(String, String)>,
}

/// The page.
#[derive(Default)]
pub struct StatusPage {
    pub view: StatusView,
    /// A card that ended, opened with a tap.
    pub opened: Option<usize>,
    /// Where each card and its button were last drawn (a card's height
    /// depends on its words), for touches and targets.
    drawn: RefCell<Vec<(Rect, Option<Rect>)>>,
}

impl StatusPage {
    /// Shows `view`; an opened card stays open while it is still there.
    pub fn set(&mut self, view: StatusView) {
        if self.view.picked != view.picked || self.opened.is_some_and(|i| i >= view.cards.len()) {
            self.opened = None;
        }
        self.view = view;
    }

    fn full(&self, i: usize) -> bool {
        self.view.cards.get(i).is_some_and(|c| c.now) || self.opened == Some(i)
    }

    fn head_rect(r: Rect) -> Rect {
        r.take_top(HEAD_H).0
    }

    /// A meter's width on the head: 150 px, or 128 when there are four.
    fn meter_w(n: usize) -> i32 {
        if n > 3 {
            128
        } else {
            150
        }
    }

    fn switch_rects(r: Rect) -> (Rect, Rect) {
        let head = Self::head_rect(r).inset(12);
        let (w, h) = (150, 44);
        let y = head.y + (head.h - h) / 2;
        let session = Rect::new(head.right() - w, y, w, h);
        let now = Rect::new(session.x - w - 6, y, w, h);
        (now, session)
    }

    fn system_rect(r: Rect, i: usize) -> Rect {
        let body = r.take_top(HEAD_H + 8).1;
        Rect::new(body.x, body.y + i as i32 * (ROW_H + 4), SYSTEMS_W, ROW_H)
    }

    fn cards_area(r: Rect) -> Rect {
        let body = r.take_top(HEAD_H + 8).1;
        body.take_left(SYSTEMS_W + 12).1
    }

    /// Save a report's button, at the foot of the cards panel, on the
    /// right.
    fn report_rect(r: Rect) -> Rect {
        let inner = Self::cards_area(r).inset(12);
        Rect::new(inner.right() - 200, inner.bottom() - 44, 200, 44)
    }

    /// The cards and numbers stop above Save a report.
    fn cards_inner(r: Rect) -> Rect {
        let inner = Self::cards_area(r).inset(12);
        let foot = Self::report_rect(r).y - 8;
        Rect::new(inner.x, inner.y, inner.w, foot - inner.y)
    }

    /// Each card's place, and its button's, as far as they fit.
    fn card_rects(&self, cx: &mut PaintCx, r: Rect) -> Vec<(Rect, Option<Rect>)> {
        let area = Self::cards_inner(r);
        let mut y = area.y;
        let mut out = Vec::new();
        for (i, c) in self.view.cards.iter().enumerate() {
            let h = if self.full(i) {
                let lines =
                    wrap(cx, &c.what, area.w - 32).len() + wrap(cx, &c.to_try, area.w - 32).len();
                let button = if c.button.is_some() { 60 } else { 0 };
                40 + 28 * lines as i32 + button + 12
            } else {
                FOLDED_H
            };
            if y + h > area.bottom() {
                break;
            }
            let rect = Rect::new(area.x, y, area.w, h);
            let button = (self.full(i) && c.button.is_some())
                .then(|| Rect::new(rect.x + 16, rect.bottom() - 16 - 48, 280, 48));
            out.push((rect, button));
            y += h + 8;
        }
        out
    }
}

/// `s` broken into lines no wider than `w`.
fn wrap(cx: &mut PaintCx, s: &str, w: i32) -> Vec<String> {
    let px = cx.theme.size_small;
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && cx.measure(FontId::Sans, px, &next) > w {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

impl Widget for StatusPage {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let v = &self.view;
        // The head: the answer, the counts, the meters, the switch.
        let head = Self::head_rect(r);
        cx.panel(head);
        let inner = head.inset(16);
        let (now_r, session_r) = Self::switch_rects(r);
        // The meters from the switch leftwards, narrower when there are
        // four (CPU, RAM, the stick and the temperature); the words have
        // the room left of them.
        let w = Self::meter_w(v.meters.len());
        let n = v.meters.len() as i32;
        let meters_x = now_r.x - 16 - n * (w + 8) + 8;
        let room = if n > 0 {
            meters_x - 16 - inner.x
        } else {
            inner.w
        };
        let (title, _) = inner.take_top(36);
        let title = Rect::new(title.x, title.y, room, title.h);
        // A long title steps down a size before it is cut short.
        let px = if cx.measure(FontId::Bold, t.size_title, &v.head) > room {
            t.size_body
        } else {
            t.size_title
        };
        cx.text(
            FontId::Bold,
            px,
            title,
            Align::Left,
            t.tone(v.head_tone),
            &v.head,
        );
        let counts = Rect::new(inner.x, inner.y + 38, room, 22);
        cx.text(
            FontId::Sans,
            t.size_small,
            counts,
            Align::Left,
            t.text_dim,
            &v.counts,
        );
        let mut mx = now_r.x - 16;
        for (name, value, full, tone) in v.meters.iter().rev() {
            let m = Rect::new(mx - w, inner.y, w, inner.h);
            let (label, bar) = m.take_top(m.h - 12);
            cx.text(
                FontId::Sans,
                t.size_small,
                label,
                Align::Left,
                t.text_dim,
                &format!("{name}  {value}"),
            );
            let bar = Rect::new(bar.x, bar.y + 2, bar.w - 12, 8);
            cx.canvas.fill_round_rect(bar, 4, t.raised);
            let fill = (bar.w as f32 * full.clamp(0.0, 1.0)).round() as i32;
            if fill > 0 {
                cx.canvas
                    .fill_round_rect(Rect::new(bar.x, bar.y, fill, bar.h), 4, t.tone(*tone));
            }
            mx -= w + 8;
        }
        for (rect, on, words) in [
            (now_r, v.now_only, "Now"),
            (session_r, !v.now_only, "This session"),
        ] {
            cx.canvas
                .fill_round_rect(rect, t.round(10), if on { t.raised } else { t.surface });
            if on {
                let bar = Rect::new(rect.x + 12, rect.bottom() - 4, rect.w - 24, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let ink = if on { t.text } else { t.text_dim };
            cx.text(FontId::Sans, t.size_small, rect, Align::Center, ink, words);
        }
        // The systems.
        for (i, s) in v.systems.iter().enumerate() {
            let row = Self::system_rect(r, i);
            let picked = i == v.picked;
            cx.canvas
                .fill_round_rect(row, t.round(10), if picked { t.raised } else { t.surface });
            if picked {
                let bar = Rect::new(row.x, row.y + 10, 4, row.h - 20);
                cx.canvas.fill_rect(bar, t.accent);
            }
            let dot = Rect::new(row.x + 18, row.y + row.h / 2 - 6, 12, 12);
            let well = if picked { t.raised } else { t.surface };
            match s.tone {
                Some(tone) => {
                    cx.canvas.fill_round_rect(dot, 6, t.tone(tone));
                    if s.ring {
                        cx.canvas
                            .fill_round_rect(Rect::new(dot.x + 3, dot.y + 3, 6, 6), 3, well);
                    }
                }
                None => {
                    cx.canvas.fill_round_rect(dot, 6, t.text_dim);
                    cx.canvas
                        .fill_round_rect(Rect::new(dot.x + 2, dot.y + 2, 8, 8), 4, well);
                }
            }
            let text = Rect::new(row.x + 44, row.y + 6, row.w - 100, 24);
            cx.text(
                FontId::Bold,
                t.size_small,
                text,
                Align::Left,
                t.text,
                &s.name,
            );
            let line = Rect::new(row.x + 44, row.y + 32, row.w - 100, 22);
            cx.text(
                FontId::Sans,
                t.size_small,
                line,
                Align::Left,
                t.text_dim,
                &s.line,
            );
            if s.badge > 0 {
                let b = Rect::new(row.right() - 52, row.y + row.h / 2 - 14, 36, 28);
                let fill = s.tone.map_or(t.raised, |k| t.tone(k));
                cx.canvas.fill_round_rect(b, 14, fill);
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    b,
                    Align::Center,
                    t.ink_on(fill),
                    &s.badge.to_string(),
                );
            }
        }
        // The picked system's cards, then its numbers.
        let area = Self::cards_area(r);
        cx.panel(area);
        let inner = Self::cards_inner(r);
        let save = Self::report_rect(r);
        cx.canvas.fill_round_rect(save, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            save,
            Align::Center,
            t.button_ink(true),
            "Save a report",
        );
        if v.cards.is_empty() {
            let line = inner.take_top(36).0;
            let words = if v.now_only {
                "Nothing wrong here now"
            } else {
                "Nothing wrong here this session"
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                line,
                Align::Left,
                t.text_dim,
                words,
            );
        }
        let rects = self.card_rects(cx, r);
        *self.drawn.borrow_mut() = rects.clone();
        let mut bottom = inner.y + if v.cards.is_empty() { 44 } else { 0 };
        for (i, (rect, button)) in rects.iter().enumerate() {
            let c = &v.cards[i];
            cx.canvas.fill_round_rect(*rect, t.round(10), t.raised);
            cx.canvas.fill_rect(
                Rect::new(rect.x, rect.y + 8, 4, rect.h - 16),
                t.tone(c.tone),
            );
            let head = Rect::new(rect.x + 16, rect.y + 8, rect.w - 32, 28);
            cx.text(
                FontId::Bold,
                t.size_small,
                head,
                Align::Left,
                t.text,
                &c.title,
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                head,
                Align::Right,
                t.text_dim,
                &c.when,
            );
            if self.full(i) {
                let mut y = rect.y + 40;
                for (words, ink) in [(&c.what, t.text), (&c.to_try, t.text_dim)] {
                    for l in wrap(cx, words, rect.w - 32) {
                        let lr = Rect::new(rect.x + 16, y, rect.w - 32, 26);
                        cx.text(FontId::Sans, t.size_small, lr, Align::Left, ink, &l);
                        y += 28;
                    }
                }
                if let (Some(b), Some(words)) = (button, &c.button) {
                    cx.canvas.fill_round_rect(*b, t.round(10), t.surface);
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        *b,
                        Align::Center,
                        t.button_ink(true),
                        words,
                    );
                }
            }
            bottom = rect.bottom() + 8;
        }
        let mut y = bottom + 4;
        for (k, value) in &v.numbers {
            if y + 26 > inner.bottom() {
                break;
            }
            let line = Rect::new(inner.x, y, inner.w, 26);
            cx.text(FontId::Sans, t.size_small, line, Align::Left, t.text_dim, k);
            cx.text(
                FontId::Bold,
                t.size_small,
                line,
                Align::Right,
                t.text,
                value,
            );
            y += 28;
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Up {
            return None;
        }
        let (now, session) = Self::switch_rects(r);
        if now.contains(x, y) {
            return Some(EventKind::Tab(NOW));
        }
        if session.contains(x, y) {
            return Some(EventKind::Tab(SESSION));
        }
        if Self::report_rect(r).contains(x, y) {
            return Some(EventKind::Tab(SAVE_REPORT));
        }
        if let Some(i) =
            (0..self.view.systems.len()).find(|&i| Self::system_rect(r, i).contains(x, y))
        {
            return Some(EventKind::Select(i));
        }
        let drawn = self.drawn.borrow().clone();
        for (i, (rect, button)) in drawn.into_iter().enumerate() {
            if i >= self.view.cards.len() {
                break;
            }
            if button.is_some_and(|b| b.contains(x, y)) {
                return Some(EventKind::Tab(BUTTON + i));
            }
            if rect.contains(x, y) && !self.view.cards[i].now {
                self.opened = if self.opened == Some(i) {
                    None
                } else {
                    Some(i)
                };
                return Some(EventKind::Tab(CARD + i));
            }
        }
        None
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (now, session) = Self::switch_rects(r);
        // The lit side of the switch and the picked system are already
        // chosen: a press has nothing to change.
        let mut out = vec![
            Target::tap("switch", NOW, "Now", now).disabled_if(self.view.now_only),
            Target::tap("switch", SESSION, "This session", session)
                .disabled_if(!self.view.now_only),
            Target::tap(
                "save-report",
                SAVE_REPORT,
                "Save a report",
                Self::report_rect(r),
            ),
        ];
        for (i, s) in self.view.systems.iter().enumerate() {
            out.push(
                Target::tap("system", i, s.name.clone(), Self::system_rect(r, i))
                    .disabled_if(i == self.view.picked),
            );
        }
        for (i, (rect, button)) in self.drawn.borrow().iter().copied().enumerate() {
            let Some(c) = self.view.cards.get(i) else {
                break;
            };
            if let (Some(b), Some(words)) = (button, &c.button) {
                out.push(Target::tap("card-button", i, words.clone(), b));
            }
            if !c.now {
                out.push(Target::tap("card", i, c.title.clone(), rect));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> StatusPage {
        let mut p = StatusPage::default();
        p.set(StatusView {
            head: "The sound dropped out".into(),
            head_tone: Tone::Bad,
            counts: "1 now · 1 earlier".into(),
            systems: (0..9)
                .map(|i| SystemRow {
                    name: format!("System {i}"),
                    line: "Working".into(),
                    tone: Some(Tone::Good),
                    ..SystemRow::default()
                })
                .collect(),
            cards: vec![
                Card {
                    title: "A note may be stuck on".into(),
                    tone: Tone::Bad,
                    now: true,
                    what: "A note's end was lost.".into(),
                    to_try: "Press Silence all notes.".into(),
                    button: Some("Silence all notes".into()),
                    ..Card::default()
                },
                Card {
                    title: "The sound dropped out".into(),
                    tone: Tone::Bad,
                    what: "The sound stopped for a moment.".into(),
                    to_try: "Play fewer sounds at once.".into(),
                    ..Card::default()
                },
            ],
            ..StatusView::default()
        });
        p
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 60,
        w: 1280,
        h: 714,
    };

    /// Draws the page once, as the screen does before a finger reaches it.
    fn drawn(p: &StatusPage) {
        let mut canvas = crate::canvas::SoftCanvas::new(1280, 800);
        let theme = crate::theme::Theme::dark();
        let mut fonts = crate::text::Fonts::embedded();
        let mut cx = PaintCx {
            canvas: &mut canvas,
            theme: &theme,
            fonts: &mut fonts,
        };
        p.paint(&mut cx, AREA);
    }

    #[test]
    fn every_target_answers_with_its_event() {
        let mut p = page();
        drawn(&p);
        let targets = p.targets(AREA);
        let ids: Vec<(&str, usize)> = targets.iter().map(|t| (t.id, t.index)).collect();
        assert!(ids.contains(&("switch", NOW)) && ids.contains(&("switch", SESSION)));
        assert_eq!(ids.iter().filter(|(id, _)| *id == "system").count(), 9);
        assert!(
            ids.contains(&("card-button", 0)),
            "the card happening now has its button"
        );
        assert!(
            ids.contains(&("card", 1)),
            "the ended card opens with a tap"
        );
        for t in targets {
            let (x, y) = (t.rect.x + t.rect.w / 2, t.rect.y + t.rect.h / 2);
            let e = p.touch(1, Touch::Up, x, y, AREA);
            let want = match t.id {
                "switch" => EventKind::Tab(t.index),
                "system" => EventKind::Select(t.index),
                "card-button" => EventKind::Tab(BUTTON + t.index),
                _ => EventKind::Tab(CARD + t.index),
            };
            assert_eq!(e, Some(want), "{}", t.id);
        }
    }

    #[test]
    fn an_ended_card_opens_and_folds_and_a_new_pick_folds_it() {
        let mut p = page();
        drawn(&p);
        let card = p
            .targets(AREA)
            .into_iter()
            .find(|t| t.id == "card")
            .unwrap();
        let (x, y) = (card.rect.x + 20, card.rect.y + 20);
        p.touch(1, Touch::Up, x, y, AREA);
        assert_eq!(p.opened, Some(1));
        p.touch(1, Touch::Up, x, y, AREA);
        assert_eq!(p.opened, None);
        p.touch(1, Touch::Up, x, y, AREA);
        let mut v = p.view.clone();
        v.picked = 3;
        p.set(v);
        assert_eq!(p.opened, None, "another system picked");
    }
}
