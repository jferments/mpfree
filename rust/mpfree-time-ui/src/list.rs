//! The song list (sequencer S4, the review's "song list for MPC
//! thinkers", mockup D): Arrange's third view beside the Clips grid and
//! the Song view. Each row is a step: a scene and how many times it plays
//! ("Intro ×2, Verse ×4"), its length in bars and the bar it starts on.
//! The step playing is lit green with its pass ("pass 3 of 4") and how far
//! through it is; the step waiting for its line is outlined amber.
//!
//! A step's scene steps with a tap on its left or right third (‹ ›), its
//! times with a tap on the – or + half of its box; a tap elsewhere on the
//! row picks it (on the lift: a drag up or down scrolls). A drag that
//! starts on its grip (the two short bars at its left, with its number)
//! moves the step: a line shows where it lands, as dragging a section's
//! name on the Song view's ruler does, and letting go there moves it.
//! "+ Add a scene" after the last step adds one, and the buttons under the
//! list lay it on the timeline, play from the picked step, or remove it.
//! The title line has the list's loop switch and the way to the other two
//! views.
//!
//! Linked to the song (the Arrange refactor's phase 4b) the rows are the
//! song's parts; the song's bars before its first marker are the first
//! row, "Song", in faint words and with no grip: it stays first. A part
//! edited in the song says "● changed in the song" where the pass shows
//! while it plays, and Back to the scene takes List → song's place.
//!
//! A list that isn't linked (a project saved before) has **Link** on its
//! title line (4b.5): a tap opens a chooser under it, Song from the list
//! or List from the song, and a touch anywhere else closes it.

use std::cell::Cell;

use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use crate::report::ReportCard;

/// One step as the list shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListRow {
    /// Its scene's name ("Verse", or "Scene 3" for a plain row).
    pub name: String,
    /// Its scene's row, for its colour (None: a linked list's section
    /// that is the song's own, phase 4b).
    pub scene: Option<usize>,
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
    /// A linked list's Song row: the song's bars before its first marker,
    /// in faint words and with no grip (it stays first; phase 4b).
    pub lead: bool,
    /// A linked list's part edited in the song (phase 4b.3): "● changed
    /// in the song" at its right, and Back to the scene puts its scene's
    /// clips back.
    pub changed: bool,
}

/// `Toggle(k)` from the title line and the buttons.
pub const LIST_LOOP: usize = 0;
pub const LIST_CLIPS: usize = 1;
pub const LIST_SONG: usize = 2;
pub const LIST_ADD: usize = 3;
pub const LIST_TO_SONG: usize = 4;
pub const LIST_PLAY: usize = 5;
pub const LIST_REMOVE: usize = 6;
/// Link (4b.5): the button opens or closes its chooser; the chooser's
/// choices, Song from the list and List from the song; a touch off it.
pub const LIST_LINK: usize = 7;
pub const LIST_LINK_FROM_LIST: usize = 8;
pub const LIST_LINK_FROM_SONG: usize = 9;
pub const LIST_LINK_CLOSE: usize = 10;
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
/// Link, left of the loop switch, and its chooser: a card a choice.
const LINK_W: i32 = 84;
const CHOOSER_W: i32 = 470;
const CHOICE_H: i32 = 62;
/// The chooser's choices' names, in the order of their `Toggle`s.
pub const LINK_CHOICES: [&str; 2] = ["Song from the list", "List from the song"];
/// A row's columns, from its left edge.
const SCENE_X: i32 = 84;
const SCENE_W: i32 = 210;
const TIMES_X: i32 = 310;
const TIMES_W: i32 = 120;
const BARS_X: i32 = 456;
const STARTS_X: i32 = 540;
const PASS_X: i32 = 650;
/// How far a finger moves before it scrolls (or moves its step, from the
/// grip) instead of picking.
const DRAG_PX: i32 = 12;
/// A row's grip: where a drag moves its step, from the row's left edge
/// over its number (the bars are drawn at its left).
const GRIP_W: i32 = 44;

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
    /// Linked to the song (phase 4b): Back to the scene takes List →
    /// song's place, and Link goes.
    pub linked: bool,
    /// Link's chooser is open (4b.5), with what each choice does ("The
    /// list's 7 steps become the song; its old bars go after them").
    pub link_choices: Option<[String; 2]>,
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
    /// It started on the step's grip: a drag moves the step.
    grip: bool,
    /// Where the finger is now.
    at: i32,
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

    /// Step `k`'s grip, if it has one in view.
    pub fn grip_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let line = self.line_rect(r, k)?;
        self.rows.get(k).filter(|row| !row.lead)?;
        Some(Rect::new(line.x, line.y, GRIP_W, line.h))
    }

    /// The gap a step dragged by its grip lands in, with the finger at
    /// `y`: the nearest gap between the lines in view (0 before the first
    /// step, the list's length after the last), never before a Song row.
    fn gap_at(&self, r: Rect, y: i32) -> usize {
        let g = SongListView::geo(r);
        let first = self.first(SongListView::visible(r)) as i32;
        let pitch = ROW_H + ROW_GAP;
        let gap = (y - g.rows.y + pitch / 2).div_euclid(pitch) + first;
        let lead = usize::from(self.rows.first().is_some_and(|row| row.lead));
        (gap.max(0) as usize).clamp(lead, self.rows.len())
    }

    /// Where gap `gap`'s line is drawn: half way between the lines either
    /// side of it, if one of them is in view.
    fn gap_y(&self, r: Rect, gap: usize) -> Option<i32> {
        match self.line_rect(r, gap) {
            Some(l) => Some(l.y - ROW_GAP / 2),
            None => gap
                .checked_sub(1)
                .and_then(|k| self.line_rect(r, k))
                .map(|l| l.bottom() + ROW_GAP / 2),
        }
    }

    /// The move a grip drag makes, let go with the finger at `y`: step
    /// `step` to gap `gap`, unless that leaves it where it is.
    fn grip_drop(&self, r: Rect, step: usize, y: i32) -> Option<EventKind> {
        let gap = self.gap_at(r, y);
        (gap != step && gap != step + 1).then_some(EventKind::Drop {
            from: step,
            to: gap,
        })
    }

    /// The title line's buttons: the loop switch, Clips and Song.
    pub fn head_rects(r: Rect) -> [Rect; 3] {
        let h = SongListView::geo(r).head;
        let song = Rect::new(h.right() - LENS_W, h.y - 4, LENS_W, h.h + 8);
        let clips = Rect::new(song.x - 8 - LENS_W, song.y, LENS_W, song.h);
        let looped = Rect::new(clips.x - 8 - LOOP_W, song.y, LOOP_W, song.h);
        [looped, clips, song]
    }

    /// Link, on a list that isn't linked (4b.5): left of the loop switch.
    pub fn link_rect(&self, r: Rect) -> Option<Rect> {
        let [looped, ..] = SongListView::head_rects(r);
        (!self.linked).then(|| Rect::new(looped.x - 8 - LINK_W, looped.y, LINK_W, looped.h))
    }

    /// Link's chooser and its two choices, while it is open: under Link,
    /// its right edge under Link's.
    pub fn chooser_rects(&self, r: Rect) -> Option<(Rect, [Rect; 2])> {
        self.link_choices.as_ref()?;
        let link = self.link_rect(r)?;
        let w = CHOOSER_W.min(link.right() - r.x - 14);
        let pad = 8;
        let card = Rect::new(
            link.right() - w,
            link.bottom() + 8,
            w,
            2 * CHOICE_H + 4 + 2 * pad,
        );
        let first = Rect::new(card.x + pad, card.y + pad, card.w - 2 * pad, CHOICE_H);
        let second = Rect::new(first.x, first.bottom() + 4, first.w, CHOICE_H);
        Some((card, [first, second]))
    }

    /// The buttons under the list: List → song, Play from here, Remove.
    pub fn button_rects(r: Rect) -> [Rect; 3] {
        SongListView::geo(r).buttons
    }

    /// Button `k`'s word: linked to the song, Back to the scene takes List
    /// → song's place.
    fn button_word(&self, k: usize) -> &'static str {
        if k == 0 && self.linked {
            "Back to the scene"
        } else {
            BUTTONS[k].0
        }
    }

    /// Whether button `k` does something now: List → song with a step,
    /// Back to the scene with a part changed in the song picked, the
    /// others with a pick.
    fn usable(&self, k: usize) -> bool {
        match k {
            0 if self.linked => self
                .picked
                .and_then(|p| self.rows.get(p))
                .is_some_and(|row| row.changed),
            0 => !self.rows.is_empty(),
            _ => self.picked.is_some(),
        }
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
        .chain(
            self.link_rect(r)
                .map(|b| (b, "Link", self.link_choices.is_some())),
        ) {
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            if lit {
                cx.canvas.stroke_round_rect(b, t.round(10), 2, t.accent);
            }
            cx.text(FontId::Bold, t.size_small, b, Align::Center, t.text, word);
        }
        let g = SongListView::geo(r);
        let c = g.cols;
        for (x, w, word) in [
            (24, 30, "#"),
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
        // A step dragged by its grip.
        let moving = self.grab.filter(|g| g.grip && g.dragging);
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
            if moving.is_some_and(|g| g.step == k) {
                cx.canvas.stroke_round_rect(line, t.round(10), 2, t.outline);
            } else if row.playing.is_some() {
                cx.canvas.stroke_round_rect(line, t.round(10), 2, t.good);
            } else if row.waiting {
                cx.canvas.stroke_round_rect(line, t.round(10), 2, t.warn);
            } else if picked {
                cx.canvas.stroke_round_rect(line, t.round(10), 2, t.accent);
            }
            let mid = |x: i32, w: i32| Rect::new(line.x + x, line.y, w, line.h);
            if !row.lead {
                for gx in [10, 15] {
                    cx.canvas.fill_round_rect(
                        Rect::new(line.x + gx, line.y + 17, 2, line.h - 34),
                        1,
                        t.text_faint,
                    );
                }
            }
            cx.text(
                FontId::Mono,
                t.size_body,
                mid(24, 20),
                Align::Left,
                t.text_dim,
                &(k + 1).to_string(),
            );
            if let Some(scene) = row.scene {
                let sw = Rect::new(line.x + 44, line.y + 10, 30, line.h - 20);
                cx.canvas.fill_round_rect(sw, t.round(6), t.track(scene));
            }
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
                // The Song row's are faint: it is the song's own bars.
                let ink = if row.lead { t.text_faint } else { t.text };
                cx.text(FontId::Bold, t.size_body, m, Align::Center, ink, word);
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
            } else if row.changed {
                let dot = Rect::new(tail.x, tail.y + tail.h / 2 - 4, 8, 8);
                cx.canvas.fill_round_rect(dot, 4, t.warn);
                cx.text(
                    FontId::Bold,
                    t.size_small - 1.0,
                    Rect::new(tail.x + 14, tail.y, tail.w - 14, tail.h),
                    Align::Left,
                    t.warn,
                    "changed in the song",
                );
            }
        }
        // Where the dragged step lands: a line across the list, with a dot
        // at each end.
        if let Some(y) = moving.and_then(|g| self.gap_y(r, self.gap_at(r, g.at))) {
            let rows = g.rows;
            cx.canvas
                .fill_round_rect(Rect::new(rows.x, y - 2, rows.w, 4), 2, t.accent);
            for x in [rows.x, rows.right() - 12] {
                cx.canvas
                    .fill_round_rect(Rect::new(x, y - 6, 12, 12), 6, t.accent);
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
            let usable = self.usable(k);
            cx.canvas
                .fill_round_rect(*b, t.round(10), if usable { t.raised } else { t.surface });
            cx.text(
                FontId::Bold,
                t.size_small,
                *b,
                Align::Center,
                t.button_ink(usable),
                self.button_word(k),
            );
        }
        // Link's chooser, over the rows under it.
        if let (Some((card, choices)), Some(lines)) =
            (self.chooser_rects(r), self.link_choices.as_ref())
        {
            cx.canvas.fill_round_rect(card, t.round(10), t.surface);
            cx.canvas.stroke_round_rect(card, t.round(10), 1, t.outline);
            for ((b, name), line) in choices.iter().zip(LINK_CHOICES).zip(lines) {
                cx.canvas.fill_round_rect(*b, t.round(8), t.raised);
                // The name a little above the middle of its half, as drawn.
                let inner = b.inset_xy(14, 8);
                let top = Rect::new(inner.x, inner.y, inner.w, 16);
                let under = Rect::new(inner.x, inner.y + 24, inner.w, inner.h - 24);
                cx.text(FontId::Bold, t.size_body, top, Align::Left, t.text, name);
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    under,
                    Align::Left,
                    t.text_dim,
                    line,
                );
            }
        }
        if let Some(card) = &self.report {
            card.paint(cx, r);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if let Some(card) = &self.report {
            return card.targets(r);
        }
        let link = self.link_rect(r);
        // Link's chooser open: its choices, and Link to close it.
        if let Some((_, choices)) = self.chooser_rects(r) {
            let mut out: Vec<Target> = link
                .map(|b| Target::tap("link", 0, "Link", b))
                .into_iter()
                .collect();
            for (k, (b, name)) in choices.iter().zip(LINK_CHOICES).enumerate() {
                out.push(Target::tap("link-choice", k, name, *b));
            }
            return out;
        }
        let [looped, clips, song] = SongListView::head_rects(r);
        let mut out = vec![
            Target::tap("loop", 0, "Loop the list", looped),
            Target::tap("lens", 0, "Clips", clips),
            Target::tap("lens", 1, "Song", song),
        ];
        if let Some(b) = link {
            out.push(Target::tap("link", 0, "Link", b));
        }
        for (k, row) in self.rows.iter().enumerate() {
            let (Some(line), Some((scene, times))) = (self.line_rect(r, k), self.boxes(r, k))
            else {
                continue;
            };
            let grip = self.grip_rect(r, k);
            let left = grip.map_or(line.x, |g| g.right());
            let pick = Rect::new(left, line.y, scene.x - left, line.h);
            out.push(
                Target::tap("step", k, format!("Step {}", k + 1), pick)
                    .disabled_if(self.picked == Some(k)),
            );
            if let Some(grip) = grip {
                out.push(
                    // Two rows down: past the next step, so the drag
                    // moves even the first step (a drag of one row
                    // lands in the gap the step is already beside).
                    Target::drag("grip", k, format!("Step {} grip", k + 1), grip)
                        .towards(0, 2 * (ROW_H + ROW_GAP)),
                );
            }
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
            out.push(
                Target::tap("button", k, self.button_word(k), *b).disabled_if(!self.usable(k)),
            );
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if let Some(card) = self.report.as_mut() {
            return card.touch(finger, phase, x, y, r);
        }
        // Link's chooser open: a choice, Link again, or a touch anywhere
        // else closes it (and does nothing else).
        if let Some((_, choices)) = self.chooser_rects(r) {
            if phase != Touch::Down {
                return None;
            }
            if let Some(k) = choices.iter().position(|b| b.contains(x, y)) {
                return Some(EventKind::Toggle(LIST_LINK_FROM_LIST + k));
            }
            if self.link_rect(r).is_some_and(|b| b.contains(x, y)) {
                return Some(EventKind::Toggle(LIST_LINK));
            }
            return Some(EventKind::Toggle(LIST_LINK_CLOSE));
        }
        match phase {
            Touch::Down => {}
            Touch::Move => {
                let visible = SongListView::visible(r);
                let g = self.grab.as_mut().filter(|g| g.finger == finger)?;
                g.at = y;
                if !g.dragging && (y - g.y).abs() < DRAG_PX {
                    return None;
                }
                g.dragging = true;
                if g.grip {
                    // The step moves on the lift; the list stays put.
                    return None;
                }
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
                if phase == Touch::Cancel {
                    return None;
                }
                if g.dragging {
                    return if g.grip {
                        self.grip_drop(r, g.step, y)
                    } else {
                        None
                    };
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
        if self.link_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Toggle(LIST_LINK));
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
        // Elsewhere on the row: picked on the lift, unless it drags (from
        // the grip, to move the step; else to scroll).
        let top = self.first(SongListView::visible(r));
        let grip = self.grip_rect(r, k).is_some_and(|g| g.contains(x, y));
        self.grab = Some(Grab {
            finger,
            y,
            top,
            step: k,
            dragging: false,
            grip,
            at: y,
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
                scene: Some(k),
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
        // Every step and button is a target.
        let t = l.targets(r);
        assert_eq!(t.iter().filter(|t| t.id == "step").count(), 3);
        assert_eq!(t.iter().filter(|t| t.id == "button").count(), 3);
        // A long list: a drag (not from a grip) scrolls, and the pick
        // brings its step in view.
        let mut l = list(20);
        let first = l.line_rect(r, 0).unwrap();
        let x = first.x + GRIP_W + 6;
        l.touch(1, Touch::Down, x, first.y + 140, r);
        l.touch(1, Touch::Move, x, first.y + 20, r);
        assert_eq!(l.touch(1, Touch::Up, x, first.y + 20, r), None);
        assert!(l.line_rect(r, 0).is_none(), "scrolled");
        l.picked = Some(0);
        assert!(l.line_rect(r, 0).is_some(), "the pick in view");
        l.picked = Some(19);
        assert!(l.line_rect(r, 19).is_some());
        assert!(l.line_rect(r, 20).is_some(), "the add row after it");
    }

    /// A drag from a step's grip moves it to the gap nearest the finger,
    /// and a tap there still picks it; let go where it was, or from
    /// elsewhere on the row, nothing moves. A linked list's Song row has
    /// no grip, and nothing lands before it.
    #[test]
    fn a_step_dragged_by_its_grip_moves_and_the_song_row_stays_first() {
        let r = Rect::new(12, 128, 854, 622);
        let mut l = list(4);
        let pitch = ROW_H + ROW_GAP;
        let grip = l.grip_rect(r, 1).unwrap();
        let (x, y) = (grip.x + 12, grip.y + grip.h / 2);
        let drag = |l: &mut SongListView, x: i32, y: i32, to: i32| {
            assert_eq!(l.touch(1, Touch::Down, x, y, r), None);
            assert_eq!(l.touch(1, Touch::Move, x, to, r), None);
            l.touch(1, Touch::Up, x, to, r)
        };
        // Two rows down: before step 4 (gap 3).
        assert_eq!(
            drag(&mut l, x, y, y + 2 * pitch),
            Some(EventKind::Drop { from: 1, to: 3 })
        );
        // Past the last: after it.
        assert_eq!(
            drag(&mut l, x, y, y + 9 * pitch),
            Some(EventKind::Drop { from: 1, to: 4 })
        );
        // Up to the top.
        assert_eq!(
            drag(&mut l, x, y, y - pitch),
            Some(EventKind::Drop { from: 1, to: 0 })
        );
        // A small drag lands where it was.
        assert_eq!(drag(&mut l, x, y, y + pitch / 3), None);
        // A tap on the grip picks; a drag from elsewhere scrolls.
        assert_eq!(l.touch(1, Touch::Down, x, y, r), None);
        assert_eq!(l.touch(1, Touch::Up, x, y, r), Some(EventKind::Select(1)));
        let line = l.line_rect(r, 1).unwrap();
        assert_eq!(drag(&mut l, line.x + 60, y, y + 2 * pitch), None);
        // Every grip is a target.
        let t = l.targets(r);
        assert_eq!(t.iter().filter(|t| t.id == "grip").count(), 4);
        // Linked: the Song row has none, and stays first.
        l.rows[0].lead = true;
        assert!(l.grip_rect(r, 0).is_none());
        // Step 2 dragged above it lands after it, where it was.
        assert_eq!(drag(&mut l, x, y, y - 2 * pitch), None);
        let grip = l.grip_rect(r, 3).unwrap();
        let y3 = grip.y + grip.h / 2;
        assert_eq!(
            drag(&mut l, grip.x + 12, y3, y3 - 3 * pitch),
            Some(EventKind::Drop { from: 3, to: 1 })
        );
        let t = l.targets(r);
        assert_eq!(t.iter().filter(|t| t.id == "grip").count(), 3);
    }

    /// Linked to the song (Arrange 4b.3), Back to the scene takes List →
    /// song's place, lit only for a part changed in the song, picked.
    #[test]
    fn linked_back_to_the_scene_is_lit_for_a_changed_part_picked() {
        let r = Rect::new(12, 128, 854, 622);
        let mut l = list(3);
        let first = |l: &SongListView| -> (String, bool) {
            let t = l.targets(r);
            let b = t.iter().find(|t| t.id == "button" && t.index == 0).unwrap();
            (b.label.clone(), b.enabled)
        };
        assert_eq!(first(&l), ("List → song".into(), true));
        l.linked = true;
        assert_eq!(first(&l), ("Back to the scene".into(), false));
        l.picked = Some(1);
        assert_eq!(first(&l), ("Back to the scene".into(), false));
        l.rows[1].changed = true;
        assert_eq!(first(&l), ("Back to the scene".into(), true));
        let b = SongListView::button_rects(r)[0];
        let (x, y) = (b.x + b.w / 2, b.y + b.h / 2);
        assert_eq!(
            l.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(LIST_TO_SONG))
        );
    }

    /// Link (Arrange 4b.5): on a list of its own, left of the loop switch;
    /// its chooser under it, its right edge under Link's: a choice, Link
    /// again, or a touch anywhere else closes it, and while it is open the
    /// rows under it take no touch. A linked list has no Link.
    #[test]
    fn link_opens_its_chooser_and_a_touch_off_it_closes_it() {
        let r = Rect::new(12, 128, 854, 622);
        let mut l = list(3);
        let link = l.link_rect(r).unwrap();
        let [looped, ..] = SongListView::head_rects(r);
        assert_eq!(link.right() + 8, looped.x);
        assert_eq!(link.w, LINK_W);
        let mid = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = mid(link);
        assert_eq!(
            l.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(LIST_LINK))
        );
        assert!(l.targets(r).iter().any(|t| t.id == "link"));
        assert_eq!(l.chooser_rects(r), None);
        l.link_choices = Some(["a".into(), "b".into()]);
        let (card, choices) = l.chooser_rects(r).unwrap();
        assert_eq!((card.right(), card.w), (link.right(), CHOOSER_W));
        assert!(card.y > link.bottom() && choices[0].bottom() < choices[1].y);
        for (k, b) in choices.iter().enumerate() {
            let (x, y) = mid(*b);
            assert_eq!(
                l.touch(1, Touch::Down, x, y, r),
                Some(EventKind::Toggle(LIST_LINK_FROM_LIST + k))
            );
        }
        assert_eq!(
            l.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(LIST_LINK))
        );
        // A row under the chooser, and one past it: only closes it.
        let line = l.line_rect(r, 2).unwrap();
        for (x, y) in [mid(choices[1]), (line.x + 6, line.y + 6)] {
            let (x, y) = (x, y + CHOICE_H);
            assert_eq!(
                l.touch(1, Touch::Down, x, y, r),
                Some(EventKind::Toggle(LIST_LINK_CLOSE))
            );
        }
        let t = l.targets(r);
        let ids: Vec<(&str, usize)> = t.iter().map(|t| (t.id, t.index)).collect();
        assert_eq!(ids, [("link", 0), ("link-choice", 0), ("link-choice", 1)]);
        l.linked = true;
        assert_eq!((l.link_rect(r), l.chooser_rects(r)), (None, None));
        assert!(!l.targets(r).iter().any(|t| t.id.starts_with("link")));
    }
}
