//! The song list (sequencer S4, the review's "song list for MPC
//! thinkers", mockup D): Arrange's third view beside the Clips grid and
//! the Song view. Each row is a step: a scene and how many times it plays
//! ("Intro ×2, Verse ×4"), its length in bars and the bar it starts on.
//! The step playing is lit green with its pass ("pass 3 of 4") and how far
//! through it is; the step waiting for its line is outlined amber.
//!
//! A step's scene steps with a tap on its left or right third (‹ ›), its
//! times with a tap on the – or + half of its box; a tap elsewhere on the
//! row picks it (on the lift: a drag up or down scrolls). "+ Add a scene"
//! after the last step adds one, and the buttons under the list lay it on
//! the timeline, play from the picked step, or remove it. The title line
//! has the list's loop switch and the way to the other two views.

use std::cell::Cell;

use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use crate::report::ReportCard;

/// One step as the list shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListRow {
    /// Its scene's name ("Verse", or "Scene 3" for a plain row).
    pub name: String,
    /// Its scene's row, for its colour.
    pub row: usize,
    pub times: u16,
    /// How long it plays, in bars.
    pub bars: i64,
    /// The bar it starts on, from 1, as the list plays from its top.
    pub starts: i64,
    /// It plays now: the pass (from 1) and how far through the step (0 to
    /// 1).
    pub playing: Option<(u16, f32)>,
    /// Its scene waits for its line.
    pub waiting: bool,
}

/// `Toggle(k)` from the title line and the buttons.
pub const LIST_LOOP: usize = 0;
pub const LIST_CLIPS: usize = 1;
pub const LIST_SONG: usize = 2;
pub const LIST_ADD: usize = 3;
pub const LIST_TO_SONG: usize = 4;
pub const LIST_PLAY: usize = 5;
pub const LIST_REMOVE: usize = 6;
/// Show, left of the loop switch while a set is open: back to the Show
/// view (live shows B1).
pub const LIST_SHOW: usize = 7;
/// `Step { index: SCENE_STEP + k, delta }` steps step `k`'s scene;
/// `Step { index: k, delta }` its times.
pub const SCENE_STEP: usize = 0x1000;

/// A row's height and the gap under it.
const ROW_H: i32 = 50;
const ROW_GAP: i32 = 8;
/// The buttons under the list.
const BUTTONS: [(&str, i32); 3] = [
    ("List → song", 150),
    ("Play from here", 150),
    ("Remove", 120),
];
const BUTTON_H: i32 = 46;
/// The title line's buttons, right to left: Song, Clips, the loop switch.
const LENS_W: i32 = 72;
const LOOP_W: i32 = 132;
/// A row's columns, from its left edge.
const SCENE_X: i32 = 84;
const SCENE_W: i32 = 210;
const TIMES_X: i32 = 310;
const TIMES_W: i32 = 120;
const BARS_X: i32 = 456;
const STARTS_X: i32 = 540;
const PASS_X: i32 = 650;
/// How far a finger moves before it scrolls instead of picking.
const DRAG_PX: i32 = 12;

/// The song list: see the module notes.
#[derive(Debug, Default)]
pub struct SongListView {
    /// "Song list · 7 steps · 76 bars · 2:27".
    pub title: String,
    pub rows: Vec<ListRow>,
    pub picked: Option<usize>,
    /// The list goes round after its last step.
    pub looped: bool,
    /// Room for another step.
    pub can_add: bool,
    /// A set is open (live shows B1): Show, left of the loop switch.
    pub show_button: bool,
    /// The last conversion's report card, open over the list (the
    /// Arrange refactor's 4.3): it takes the list's touches and targets.
    pub report: Option<ReportCard>,
    /// The first step in view, and the pick it last followed.
    top: Cell<usize>,
    followed: Cell<Option<Option<usize>>>,
    grab: Option<Grab>,
}

#[derive(Clone, Copy, Debug)]
struct Grab {
    finger: u64,
    y: i32,
    top: usize,
    step: usize,
    dragging: bool,
}

/// Where the parts are.
struct Geo {
    head: Rect,
    cols: Rect,
    rows: Rect,
    buttons: [Rect; 3],
}

impl SongListView {
    pub fn new() -> SongListView {
        SongListView::default()
    }

    fn geo(r: Rect) -> Geo {
        let inner = r.inset(14);
        let (head, rest) = inner.take_top(26);
        let (_, rest) = rest.take_top(10);
        let (cols, rest) = rest.take_top(20);
        let (foot, rest) = rest.take_bottom(BUTTON_H + 12);
        let mut buttons = [Rect::new(0, 0, 0, 0); 3];
        let mut x = foot.x;
        for (k, (_, w)) in BUTTONS.iter().enumerate() {
            buttons[k] = Rect::new(x, foot.bottom() - BUTTON_H, *w, BUTTON_H);
            x += w + 8;
        }
        Geo {
            head,
            cols,
            rows: rest.take_top(rest.h - 4).0.inset_xy(0, 4),
            buttons,
        }
    }

    /// Rows that fit, the add row taking one at the end.
    fn visible(r: Rect) -> usize {
        ((SongListView::geo(r).rows.h + ROW_GAP) / (ROW_H + ROW_GAP)).max(1) as usize
    }

    /// The first step in view: it follows the pick when the pick changes,
    /// and stays where a drag left it otherwise.
    fn first(&self, visible: usize) -> usize {
        // The add row is one more line.
        let lines = self.rows.len() + 1;
        if lines <= visible {
            return 0;
        }
        let last = lines - visible;
        let mut top = self.top.get().min(last);
        if self.followed.get() != Some(self.picked) {
            if let Some(p) = self.picked {
                // The last step brings the add row with it.
                let low = if p + 1 == self.rows.len() { p + 1 } else { p };
                if p < top {
                    top = p;
                } else if low >= top + visible {
                    top = (low + 1).saturating_sub(visible).min(last);
                }
            }
            self.followed.set(Some(self.picked));
        }
        self.top.set(top);
        top
    }

    /// Line `k`'s rectangle, if it is in view: step `k`, or the add row
    /// at `rows.len()`.
    pub fn line_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let g = SongListView::geo(r);
        let visible = SongListView::visible(r);
        let first = self.first(visible);
        if k < first || k >= first + visible || k > self.rows.len() {
            return None;
        }
        let y = g.rows.y + (k - first) as i32 * (ROW_H + ROW_GAP);
        let h = if k == self.rows.len() {
            ROW_H - 6
        } else {
            ROW_H
        };
        Some(Rect::new(g.rows.x, y, g.rows.w, h))
    }

    /// Step `k`'s scene box and times box.
    pub fn boxes(&self, r: Rect, k: usize) -> Option<(Rect, Rect)> {
        let line = self.line_rect(r, k).filter(|_| k < self.rows.len())?;
        let scene = Rect::new(line.x + SCENE_X, line.y + 8, SCENE_W, line.h - 16);
        let times = Rect::new(line.x + TIMES_X, line.y + 8, TIMES_W, line.h - 16);
        Some((scene, times))
    }

    /// The title line's buttons: the loop switch, Clips and Song.
    pub fn head_rects(r: Rect) -> [Rect; 3] {
        let h = SongListView::geo(r).head;
        let song = Rect::new(h.right() - LENS_W, h.y - 4, LENS_W, h.h + 8);
        let clips = Rect::new(song.x - 8 - LENS_W, song.y, LENS_W, song.h);
        let looped = Rect::new(clips.x - 8 - LOOP_W, song.y, LOOP_W, song.h);
        [looped, clips, song]
    }

    /// Where Show is, while a set is open: left of the loop switch.
    pub fn show_rect(&self, r: Rect) -> Option<Rect> {
        let [looped, ..] = SongListView::head_rects(r);
        let w = crate::widgets::SHOW_W;
        self.show_button
            .then(|| Rect::new(looped.x - 8 - w, looped.y, w, looped.h))
    }

    /// The buttons under the list: List → song, Play from here, Remove.
    pub fn button_rects(r: Rect) -> [Rect; 3] {
        SongListView::geo(r).buttons
    }
}

impl Widget for SongListView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, "");
        let [looped, clips, song] = SongListView::head_rects(r);
        for (b, word, lit) in [
            (
                looped,
                if self.looped {
                    "Loop the list: on"
                } else {
                    "Loop the list: off"
                },
                self.looped,
            ),
            (clips, "Clips", false),
            (song, "Song", false),
        ]
        .into_iter()
        .chain(self.show_rect(r).map(|b| (b, "Show", false)))
        {
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            if lit {
                cx.canvas.stroke_round_rect(b, t.round(10), 2, t.accent);
            }
            cx.text(FontId::Bold, t.size_small, b, Align::Center, t.text, word);
        }
        let g = SongListView::geo(r);
        let c = g.cols;
        for (x, w, word) in [
            (14, 30, "#"),
            (SCENE_X, SCENE_W, "Scene"),
            (TIMES_X, TIMES_W, "Times"),
            (BARS_X, 70, "Bars"),
            (STARTS_X, 100, "Starts"),
        ] {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(c.x + x, c.y, w, c.h),
                Align::Left,
                t.text_faint,
                word,
            );
        }
        for (k, row) in self.rows.iter().enumerate() {
            let Some(line) = self.line_rect(r, k) else {
                continue;
            };
            let picked = self.picked == Some(k);
            let fill = if row.playing.is_some() {
                t.raised.mix(t.good, 0.22)
            } else {
                t.raised
            };
            cx.canvas.fill_round_rect(line, t.round(10), fill);
            if row.playing.is_some() {
                cx.canvas.stroke_round_rect(line, t.round(10), 2, t.good);
            } else if row.waiting {
                cx.canvas.stroke_round_rect(line, t.round(10), 2, t.warn);
            } else if picked {
                cx.canvas.stroke_round_rect(line, t.round(10), 2, t.accent);
            }
            let mid = |x: i32, w: i32| Rect::new(line.x + x, line.y, w, line.h);
            cx.text(
                FontId::Mono,
                t.size_body,
                mid(14, 30),
                Align::Left,
                t.text_dim,
                &(k + 1).to_string(),
            );
            let sw = Rect::new(line.x + 44, line.y + 10, 30, line.h - 20);
            cx.canvas.fill_round_rect(sw, t.round(6), t.track(row.row));
            let (scene, times) = self.boxes(r, k).expect("in view");
            for (b, word) in [
                (scene, row.name.as_str()),
                (times, &format!("× {}", row.times)),
            ] {
                cx.canvas.fill_round_rect(b, t.round(8), t.surface);
                let (less, more) = if b == scene {
                    ("‹", "›")
                } else {
                    ("–", "+")
                };
                let (l, rest) = b.take_left(28);
                let (rr, m) = rest.take_right(28);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    l,
                    Align::Center,
                    t.text_faint,
                    less,
                );
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    rr,
                    Align::Center,
                    t.text_faint,
                    more,
                );
                cx.text(FontId::Bold, t.size_body, m, Align::Center, t.text, word);
            }
            cx.text(
                FontId::Sans,
                t.size_body,
                mid(BARS_X, 70),
                Align::Left,
                t.text_dim,
                &row.bars.to_string(),
            );
            cx.text(
                FontId::Mono,
                t.size_small,
                mid(STARTS_X, 100),
                Align::Left,
                t.text_dim,
                &format!("bar {}", row.starts),
            );
            let tail = Rect::new(line.x + PASS_X, line.y, line.w - PASS_X - 14, line.h);
            if let Some((pass, done)) = row.playing {
                let (words, bar) = tail.take_top(line.h / 2 + 4);
                cx.text(
                    FontId::Bold,
                    t.size_small - 1.0,
                    words,
                    Align::Left,
                    t.good,
                    &format!("pass {pass} of {}", row.times),
                );
                let track = Rect::new(bar.x, bar.y + 2, bar.w, 5);
                cx.canvas.fill_round_rect(track, 2, t.surface);
                let w = (done.clamp(0.0, 1.0) * track.w as f32).round() as i32;
                cx.canvas
                    .fill_round_rect(Rect::new(track.x, track.y, w, track.h), 2, t.good);
            } else if row.waiting {
                cx.text(
                    FontId::Bold,
                    t.size_small - 1.0,
                    tail,
                    Align::Left,
                    t.warn,
                    "next",
                );
            }
        }
        if let Some(add) = self.line_rect(r, self.rows.len()) {
            cx.canvas.stroke_round_rect(add, t.round(10), 1, t.outline);
            let then = if self.looped {
                "then: from the top"
            } else {
                "then: Stop"
            };
            let word = if self.can_add {
                format!("+ Add a scene  ·  {then}")
            } else {
                then.to_string()
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                add.inset_xy(16, 0),
                Align::Left,
                if self.can_add {
                    t.text_dim
                } else {
                    t.text_faint
                },
                &word,
            );
        }
        if self.rows.is_empty() {
            let msg = Rect::new(g.rows.x, g.rows.y + ROW_H + 16, g.rows.w, 40);
            cx.text(
                FontId::Sans,
                t.size_small,
                msg,
                Align::Left,
                t.text_faint,
                "Add the scenes in the order they play, and how many times each",
            );
        }
        for (k, b) in SongListView::button_rects(r).iter().enumerate() {
            let usable = k == 0 && !self.rows.is_empty() || k > 0 && self.picked.is_some();
            cx.canvas
                .fill_round_rect(*b, t.round(10), if usable { t.raised } else { t.surface });
            cx.text(
                FontId::Bold,
                t.size_small,
                *b,
                Align::Center,
                t.button_ink(usable),
                BUTTONS[k].0,
            );
        }
        if let Some(card) = &self.report {
            card.paint(cx, r);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if let Some(card) = &self.report {
            return card.targets(r);
        }
        let [looped, clips, song] = SongListView::head_rects(r);
        let mut out = vec![
            Target::tap("loop", 0, "Loop the list", looped),
            Target::tap("lens", 0, "Clips", clips),
            Target::tap("lens", 1, "Song", song),
        ];
        if let Some(b) = self.show_rect(r) {
            out.push(Target::tap("show-view", 0, "Show", b));
        }
        for (k, row) in self.rows.iter().enumerate() {
            let (Some(line), Some((scene, times))) = (self.line_rect(r, k), self.boxes(r, k))
            else {
                continue;
            };
            let pick = Rect::new(line.x, line.y, scene.x - line.x, line.h);
            out.push(
                Target::tap("step", k, format!("Step {}", k + 1), pick)
                    .disabled_if(self.picked == Some(k)),
            );
            let (less, rest) = scene.take_left(scene.w / 3);
            let more = rest.take_right(scene.w / 3).0;
            out.push(Target::tap(
                "scene-less",
                k,
                format!("{} ‹", row.name),
                less,
            ));
            out.push(Target::tap(
                "scene-more",
                k,
                format!("{} ›", row.name),
                more,
            ));
            let (fewer, more) = times.take_left(times.w / 2);
            out.push(Target::tap(
                "times-less",
                k,
                format!("Step {} –", k + 1),
                fewer,
            ));
            out.push(Target::tap(
                "times-more",
                k,
                format!("Step {} +", k + 1),
                more,
            ));
        }
        if let Some(add) = self.line_rect(r, self.rows.len()) {
            out.push(Target::tap("add", 0, "+ Add a scene", add).disabled_if(!self.can_add));
        }
        for (k, b) in SongListView::button_rects(r).iter().enumerate() {
            let usable = k == 0 && !self.rows.is_empty() || k > 0 && self.picked.is_some();
            out.push(Target::tap("button", k, BUTTONS[k].0, *b).disabled_if(!usable));
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if let Some(card) = self.report.as_mut() {
            return card.touch(finger, phase, x, y, r);
        }
        match phase {
            Touch::Down => {}
            Touch::Move => {
                let visible = SongListView::visible(r);
                let g = self.grab.as_mut().filter(|g| g.finger == finger)?;
                if !g.dragging && (y - g.y).abs() < DRAG_PX {
                    return None;
                }
                g.dragging = true;
                let lines = self.rows.len() + 1;
                if lines > visible {
                    let moved = (g.y - y) / (ROW_H + ROW_GAP);
                    let last = (lines - visible) as i32;
                    let top = (g.top as i32 + moved).clamp(0, last) as usize;
                    self.top.set(top);
                    self.followed.set(Some(self.picked));
                }
                return None;
            }
            Touch::Up | Touch::Cancel => {
                let g = self.grab.take().filter(|g| g.finger == finger)?;
                if g.dragging || phase == Touch::Cancel {
                    return None;
                }
                return Some(EventKind::Select(g.step));
            }
        }
        let [looped, clips, song] = SongListView::head_rects(r);
        for (b, k) in [(looped, LIST_LOOP), (clips, LIST_CLIPS), (song, LIST_SONG)] {
            if b.contains(x, y) {
                return Some(EventKind::Toggle(k));
            }
        }
        if self.show_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Toggle(LIST_SHOW));
        }
        for (k, b) in SongListView::button_rects(r).iter().enumerate() {
            if b.contains(x, y) {
                return Some(EventKind::Toggle(LIST_TO_SONG + k));
            }
        }
        if self
            .line_rect(r, self.rows.len())
            .is_some_and(|a| a.contains(x, y))
        {
            return self.can_add.then_some(EventKind::Toggle(LIST_ADD));
        }
        let k = (0..self.rows.len())
            .find(|&k| self.line_rect(r, k).is_some_and(|l| l.contains(x, y)))?;
        let (scene, times) = self.boxes(r, k)?;
        if scene.contains(x, y) && x < scene.x + scene.w / 3 {
            return Some(EventKind::Step {
                index: SCENE_STEP + k,
                delta: -1,
            });
        }
        if scene.contains(x, y) && x >= scene.right() - scene.w / 3 {
            return Some(EventKind::Step {
                index: SCENE_STEP + k,
                delta: 1,
            });
        }
        if times.contains(x, y) {
            let delta = if x < times.x + times.w / 2 { -1 } else { 1 };
            return Some(EventKind::Step { index: k, delta });
        }
        // Elsewhere on the row: picked on the lift, unless it drags.
        let top = self.first(SongListView::visible(r));
        self.grab = Some(Grab {
            finger,
            y,
            top,
            step: k,
            dragging: false,
        });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> SongListView {
        let mut l = SongListView::new();
        l.title = "Song list".into();
        l.can_add = true;
        l.rows = (0..n)
            .map(|k| ListRow {
                name: format!("Scene {}", k + 1),
                row: k,
                times: 2,
                bars: 8,
                starts: 1 + 8 * k as i64,
                ..ListRow::default()
            })
            .collect();
        l
    }

    /// The rows, the add row and the buttons, and what a tap on each
    /// gives; a drag scrolls a long list and picks nothing.
    #[test]
    fn steps_step_their_scene_and_times_and_a_tap_picks() {
        let r = Rect::new(12, 128, 854, 622);
        let mut l = list(3);
        let visible = SongListView::visible(r);
        assert!(visible >= 7, "{visible}");
        let (scene, times) = l.boxes(r, 1).unwrap();
        let tap = |l: &mut SongListView, x: i32, y: i32| l.touch(1, Touch::Down, x, y, r);
        assert_eq!(
            tap(&mut l, scene.x + 3, scene.y + 3),
            Some(EventKind::Step {
                index: SCENE_STEP + 1,
                delta: -1
            })
        );
        assert_eq!(
            tap(&mut l, scene.right() - 3, scene.y + 3),
            Some(EventKind::Step {
                index: SCENE_STEP + 1,
                delta: 1
            })
        );
        assert_eq!(
            tap(&mut l, times.right() - 3, times.y + 3),
            Some(EventKind::Step { index: 1, delta: 1 })
        );
        // A row picks on the lift.
        let line = l.line_rect(r, 2).unwrap();
        assert_eq!(tap(&mut l, line.x + 6, line.y + 6), None);
        assert_eq!(
            l.touch(1, Touch::Up, line.x + 6, line.y + 6, r),
            Some(EventKind::Select(2))
        );
        let add = l.line_rect(r, 3).unwrap();
        assert_eq!(
            tap(&mut l, add.x + 6, add.y + 6),
            Some(EventKind::Toggle(LIST_ADD))
        );
        let b = SongListView::button_rects(r);
        assert!(b[0].bottom() <= r.bottom() - 14 && b[0].right() < b[1].x);
        assert!(add.bottom() < b[0].y, "{add:?} {:?}", b[0]);
        assert_eq!(
            tap(&mut l, b[2].x + 3, b[2].y + 3),
            Some(EventKind::Toggle(LIST_REMOVE))
        );
        let [lp, clips, song] = SongListView::head_rects(r);
        assert!(lp.right() < clips.x && clips.right() < song.x);
        assert_eq!(
            tap(&mut l, song.x + 3, song.y + 3),
            Some(EventKind::Toggle(LIST_SONG))
        );
        // While a set is open, Show sits left of the loop switch.
        assert_eq!(l.show_rect(r), None);
        l.show_button = true;
        let show = l.show_rect(r).unwrap();
        assert!(show.right() < lp.x && show.w == 72, "{show:?}");
        assert!(show.x > r.x + 200, "clear of the title: {show:?}");
        assert!(l.targets(r).iter().any(|t| t.id == "show-view"));
        assert_eq!(
            tap(&mut l, show.x + 3, show.y + 3),
            Some(EventKind::Toggle(LIST_SHOW))
        );
        l.show_button = false;
        // Every step and button is a target.
        let t = l.targets(r);
        assert_eq!(t.iter().filter(|t| t.id == "step").count(), 3);
        assert_eq!(t.iter().filter(|t| t.id == "button").count(), 3);
        // A long list: a drag scrolls, and the pick brings its step in view.
        let mut l = list(20);
        let first = l.line_rect(r, 0).unwrap();
        l.touch(1, Touch::Down, first.x + 6, first.y + 140, r);
        l.touch(1, Touch::Move, first.x + 6, first.y + 20, r);
        assert_eq!(l.touch(1, Touch::Up, first.x + 6, first.y + 20, r), None);
        assert!(l.line_rect(r, 0).is_none(), "scrolled");
        l.picked = Some(0);
        assert!(l.line_rect(r, 0).is_some(), "the pick in view");
        l.picked = Some(19);
        assert!(l.line_rect(r, 19).is_some());
        assert!(l.line_rect(r, 20).is_some(), "the add row after it");
    }
}
