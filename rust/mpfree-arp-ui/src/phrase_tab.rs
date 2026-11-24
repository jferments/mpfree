//! The Phrase tab: the Arp panel's fifth, where a track picks the phrase a
//! held note plays. It is built as the five boards the owner approved show
//! it (23 Nov 2026; the design is in `docs/dev/design/arpeggiator.md`,
//! "Phrases").
//!
//! Three parts under the mode tabs, left to right:
//!
//! - [`PhraseKinds`]: the shelves (Bass, Riff, Chords, Arp lines, Yours),
//!   the one the list shows lit.
//! - [`PhraseRows`]: that shelf's phrases, each with how long it is and a
//!   small picture of its notes. The track's pick is lit; while the
//!   transport plays, a new pick waits for the bar line in amber and the
//!   phrase still playing is green.
//! - [`PhrasePlays`]: the phrase that plays, larger, with a cursor where
//!   it is; how it follows the held note (In key or Exact); and the four
//!   Q-Link sliders.
//!
//! On a kit track, whose pads play as pressed, [`PhraseKit`] says so in
//! the place of the last two.
//!
//! Like the arp's other widgets these only show what the panel put in
//! them and report touches. What they show is worked out from an
//! [`ArpState`] by [`kinds`], [`rows`], [`plays`] and [`kit`], which draw
//! nothing, so the tests read those.

use crate::phrases::{Origin, Shelf};
use crate::view::ArpState;
use crate::widgets::{Slider, SliderStack};
use mpfree_arp::phrase::PLAIN_VELOCITY;
use mpfree_arp::scale::{NOTE_NAMES, SCALES};
use mpfree_arp::{Feel, Follow, Phrase, PhraseKind, Scale, Written, PPQ};
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, Color, EventKind, PaintCx, Rect, Target, Touch, Widget};
use std::cell::RefCell;

/// A shelf's row and a phrase's: a fingertip tall, the lit box [`BOX_H`]
/// of it.
const ROW_H: i32 = 50;
const BOX_H: i32 = 46;

// ---------------------------------------------------------------- sketch

/// How solid a note softer than a plain one is drawn, of 255.
const SOFT: u8 = 158;
/// The room over the large picture's notes for the bar numbers.
const BAR_NUMBERS: i32 = 14;

fn softer(ink: Color, soft: bool) -> Color {
    if soft {
        ink.with_alpha((u16::from(ink.a) * u16::from(SOFT) / 255) as u8)
    } else {
        ink
    }
}

/// One note of a [`Sketch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchNote {
    /// Where it starts and how long it is as written, in ticks.
    pub start: u32,
    pub len: u32,
    /// Semitones from the home note, as written.
    pub semis: i8,
    /// Played softer than a plain note.
    pub soft: bool,
}

/// A phrase's notes as a picture: time across, pitch up. Small in a row of
/// the list ([`Sketch::paint_small`]), large for the phrase that plays
/// ([`Sketch::paint_large`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sketch {
    /// The phrase's length and one bar's, in ticks.
    pub ticks: u32,
    pub bar_ticks: u32,
    pub notes: Vec<SketchNote>,
}

impl Sketch {
    pub fn of(p: &Phrase) -> Sketch {
        Sketch {
            ticks: p.ticks(),
            bar_ticks: p.bar_ticks(),
            notes: p
                .notes()
                .iter()
                .map(|n| SketchNote {
                    start: n.start,
                    len: n.len,
                    semis: n.semis,
                    soft: n.velocity < PLAIN_VELOCITY,
                })
                .collect(),
        }
    }

    fn x_of(&self, r: Rect, tick: u32) -> i32 {
        r.x + (tick as f32 * r.w as f32 / self.ticks.max(1) as f32).round() as i32
    }

    /// Each note's box in `r` and whether it is a soft one: the phrase's
    /// length across the whole width, its range of notes (never drawn as
    /// less than a third, so a line on one note is not a slab) in the
    /// middle of the height, and no note thinner than `min_h` pixels.
    pub fn boxes(&self, r: Rect, min_h: i32) -> Vec<(Rect, bool)> {
        let semis = || self.notes.iter().map(|n| i32::from(n.semis));
        let (Some(lo), Some(hi)) = (semis().min(), semis().max()) else {
            return Vec::new();
        };
        if r.w <= 0 || r.h <= 0 {
            return Vec::new();
        }
        let range = (hi - lo) as f32;
        let span = range.max(4.0);
        let low = lo as f32 - (span - range) / 2.0;
        let row = r.h as f32 / (span + 1.0);
        let nh = (row - 1.0).clamp(min_h.clamp(1, 9) as f32, 9.0).round() as i32;
        let sx = r.w as f32 / self.ticks.max(1) as f32;
        self.notes
            .iter()
            .map(|n| {
                let x = (r.x + (n.start as f32 * sx).round() as i32).min(r.right() - 1);
                let w = ((n.len as f32 * sx).round() as i32 - 1)
                    .max(3)
                    .min(r.right() - x);
                let up = span - (f32::from(n.semis) - low);
                let y = r.y + (up * row + (row - nh as f32) / 2.0).round() as i32;
                let y = y.clamp(r.y, (r.bottom() - nh).max(r.y));
                (Rect::new(x, y, w, nh), n.soft)
            })
            .collect()
    }

    /// The picture small, for a row of the list: a line at each bar, and
    /// the notes in `ink`.
    pub fn paint_small(&self, cx: &mut PaintCx, r: Rect, ink: Color) {
        let line = cx.theme.outline;
        for bar in 1..self.ticks / self.bar_ticks.max(1) {
            let x = self.x_of(r, bar * self.bar_ticks);
            cx.canvas.fill_rect(Rect::new(x, r.y + 2, 1, r.h - 4), line);
        }
        for (b, soft) in self.boxes(r, 3) {
            cx.canvas.fill_rect(b, softer(ink, soft));
        }
    }

    /// The picture large, in a box of its own: a line at each bar with
    /// its number, a fainter one at each beat, the notes in `ink` and,
    /// while the phrase plays, a cursor `at` of the way across (0.0 to
    /// 1.0).
    pub fn paint_large(&self, cx: &mut PaintCx, r: Rect, ink: Color, at: Option<f32>) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(r, t.round(10), t.bg);
        let inner = r.inset_xy(6, 4);
        let (numbers, notes) = inner.take_top(BAR_NUMBERS);
        let bar_ticks = self.bar_ticks.max(1);
        // A beat's line, well under a bar's.
        let beat_line = t.outline.with_alpha(t.outline.a / 3);
        let mut tick = 0;
        while self.ticks > 0 && tick <= self.ticks {
            let x = self.x_of(inner, tick).min(inner.right() - 1);
            if tick % bar_ticks == 0 {
                cx.canvas
                    .fill_rect(Rect::new(x, inner.y, 1, inner.h), t.outline);
                if tick < self.ticks {
                    cx.text(
                        FontId::Sans,
                        t.size_small - 2.0,
                        Rect::new(x + 5, numbers.y, 28, numbers.h),
                        Align::Left,
                        t.faded(t.text_dim, t.bg),
                        &format!("{}", tick / bar_ticks + 1),
                    );
                }
            } else {
                cx.canvas
                    .fill_rect(Rect::new(x, notes.y, 1, notes.h), beat_line);
            }
            tick += PPQ;
        }
        for (b, soft) in self.boxes(notes, 3) {
            cx.canvas.fill_round_rect(b, 2, softer(ink, soft));
        }
        if let Some(at) = at {
            let x = inner.x + ((inner.w - 2) as f32 * at.clamp(0.0, 1.0)).round() as i32;
            cx.canvas
                .fill_rect(Rect::new(x, inner.y, 2, inner.h), t.good);
        }
    }
}

// ---------------------------------------------------------------- kinds

/// The shelves of the list, top to bottom, the one the list shows lit. A
/// tap on shelf `i` gives `Select(i)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhraseKinds {
    /// Each shelf's name and how many phrases are on it. The last is the
    /// owner's, set a little apart from MPFree's.
    pub items: Vec<(String, usize)>,
    /// The shelf the list shows.
    pub shown: usize,
    /// A phrase can't play on this track (a kit): the names are faint.
    pub dim: bool,
}

/// The owner's shelf sits this far under MPFree's.
const APART: i32 = 14;

impl PhraseKinds {
    fn body(r: Rect) -> Rect {
        // Matches PaintCx::titled_panel with the default theme's padding.
        r.inset(14).take_top(34).1
    }

    /// Shelf `i`'s row: where it answers a finger.
    pub fn row(&self, r: Rect, i: usize) -> Rect {
        let apart = if i > 0 && i + 1 == self.items.len() {
            APART
        } else {
            0
        };
        Rect::new(
            r.x + 8,
            PhraseKinds::body(r).y + i as i32 * ROW_H + apart,
            r.w - 16,
            ROW_H,
        )
    }
}

impl Widget for PhraseKinds {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, "Phrases", "");
        for (i, (name, count)) in self.items.iter().enumerate() {
            let b = Rect {
                h: BOX_H,
                ..self.row(r, i)
            };
            let on = i == self.shown;
            if on {
                cx.canvas.fill_round_rect(b, t.round(10), t.raised);
                cx.canvas
                    .fill_round_rect(Rect::new(b.x, b.y + 10, 4, b.h - 20), 2, t.accent);
            }
            let fill = if on { t.raised } else { t.surface };
            let ink = match (self.dim, on) {
                (true, _) => t.faded(t.text, fill),
                (false, true) => t.text,
                (false, false) => t.text_dim,
            };
            let (number, words) = b.inset_xy(16, 0).take_right(34);
            let font = if on { FontId::Bold } else { FontId::Sans };
            cx.text(font, t.size_body, words, Align::Left, ink, name);
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                number,
                Align::Right,
                t.faded(t.text_dim, fill),
                &format!("{count}"),
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        (0..self.items.len())
            .map(|i| {
                Target::tap("kind", i, self.items[i].0.clone(), self.row(r, i))
                    .disabled_if(i == self.shown)
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.items.len()).find(|&i| self.row(r, i).contains(x, y))?;
        (i != self.shown).then(|| {
            self.shown = i;
            EventKind::Select(i)
        })
    }
}

// ---------------------------------------------------------------- rows

/// What a row of the list is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowState {
    #[default]
    Plain,
    /// The track's pick: what a held note plays.
    Picked,
    /// The track's pick, waiting for the bar line while another plays.
    Waiting,
    /// The phrase that plays until the pick comes in.
    Playing,
}

/// One phrase in the list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhraseRow {
    /// The phrase's place in the app's list: what a tap picks.
    pub index: usize,
    pub name: String,
    /// What it says after the name: "2 bars · swung".
    pub tags: String,
    pub state: RowState,
    /// Written in the other kind of scale than the key: drawn quieter.
    pub dim: bool,
    pub sketch: Sketch,
}

/// The shown shelf's phrases, the track's pick kept in view. A tap on a
/// row gives `Select(i)`, `i` the phrase's place in the app's list
/// ([`PhraseRow::index`]); New idea in the heading gives
/// `Toggle(NEW_IDEA)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhraseRows {
    /// The shelf's name.
    pub title: String,
    /// New idea shows in the heading (composing tools P2, board A).
    pub new_idea: bool,
    pub rows: Vec<PhraseRow>,
    /// The row the track's pick is on, when it is on this shelf.
    pub picked: Option<usize>,
    /// What the shelf says under its rows (the owner's shelf: how a
    /// phrase gets there).
    pub note: String,
}

/// A row's picture of its notes.
const SKETCH_W: i32 = 118;
const SKETCH_H: i32 = 30;

/// The heading's New idea: `Toggle(NEW_IDEA)` from the list.
pub const NEW_IDEA: usize = 0;
/// More like this, on the phrase that plays: `Toggle(MORE_LIKE)` from
/// [`PhrasePlays`].
pub const MORE_LIKE: usize = 0;

impl PhraseRows {
    /// New idea's button, at the right of the heading.
    pub fn new_idea_button(r: Rect) -> Rect {
        Rect::new(r.right() - 16 - 110, r.y + 7, 110, 34)
    }

    /// The area for rows: the panel under its heading, less the scroll
    /// bar's side.
    fn body(r: Rect) -> Rect {
        Rect::new(r.x + 10, r.y + 54, r.w - 30, r.h - 56)
    }

    fn visible(r: Rect) -> usize {
        (PhraseRows::body(r).h / ROW_H).max(0) as usize
    }

    /// The first row shown: the pick in the middle where the list is
    /// longer than the panel.
    fn first(&self, visible: usize) -> usize {
        if self.rows.len() <= visible || visible == 0 {
            return 0;
        }
        self.picked
            .unwrap_or(0)
            .saturating_sub(visible / 2)
            .min(self.rows.len() - visible)
    }

    /// Where row `k` of the shelf answers a finger, when it is in view.
    pub fn row(&self, r: Rect, k: usize) -> Option<Rect> {
        let visible = PhraseRows::visible(r);
        let first = self.first(visible);
        let body = PhraseRows::body(r);
        (k >= first && k < self.rows.len().min(first + visible))
            .then(|| Rect::new(body.x, body.y + (k - first) as i32 * ROW_H, body.w, ROW_H))
    }
}

/// `s` broken into lines no wider than `w` at `px`.
fn wrap(cx: &mut PaintCx, s: &str, px: f32, w: i32) -> Vec<String> {
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

impl Widget for PhraseRows {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        // The shelf's name, and where the pick is on it: at the right, or
        // beside the name with New idea at the right.
        let head = Rect::new(r.x + 16, r.y + 6, r.w - 36, 38);
        let count = match (self.picked, self.rows.len()) {
            (_, 0) => String::new(),
            (Some(k), n) => format!("{} of {n}", k + 1),
            (None, n) => format!("{n}"),
        };
        if self.new_idea {
            let b = PhraseRows::new_idea_button(r);
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                b,
                Align::Center,
                t.text,
                "New idea",
            );
            let name = Rect {
                w: (b.x - 12 - head.x).max(1),
                ..head
            };
            let nw = cx.text(
                FontId::Bold,
                t.size_title + 1.0,
                name,
                Align::Left,
                t.text,
                &self.title,
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect {
                    x: name.x + nw + 10,
                    w: (name.w - nw - 10).max(1),
                    ..name
                },
                Align::Left,
                t.text_dim,
                &count,
            );
        } else {
            let cw = cx.text(
                FontId::Sans,
                t.size_small,
                head,
                Align::Right,
                t.text_dim,
                &count,
            );
            let name = Rect {
                w: (head.w - cw - 12).max(1),
                ..head
            };
            cx.text(
                FontId::Bold,
                t.size_title + 1.0,
                name,
                Align::Left,
                t.text,
                &self.title,
            );
        }
        cx.canvas
            .fill_rect(Rect::new(r.x + 12, r.y + 47, r.w - 24, 1), t.outline);
        let body = PhraseRows::body(r);
        let visible = PhraseRows::visible(r);
        let first = self.first(visible);
        for (k, row) in self.rows.iter().enumerate().skip(first).take(visible) {
            let b = Rect::new(body.x, body.y + (k - first) as i32 * ROW_H, body.w, BOX_H);
            // The box, the bar at its left, the name, the tags, the notes.
            let (fill, bar, ink, tag_ink, notes) = match row.state {
                RowState::Picked => (Some(t.raised), Some(t.accent), t.text, t.text_dim, t.accent),
                RowState::Waiting => {
                    let fill = t.surface.mix(t.warn, 0.10);
                    let ink = t.ink_on(fill);
                    (Some(fill), None, ink, t.readable(t.warn, fill), t.warn)
                }
                RowState::Playing => (
                    None,
                    Some(t.good),
                    t.text,
                    t.readable(t.good, t.surface),
                    t.good,
                ),
                RowState::Plain if row.dim => {
                    (None, None, t.text_dim, t.text_dim, softer(t.text_dim, true))
                }
                RowState::Plain => (None, None, t.text, t.text_dim, t.text_dim),
            };
            if let Some(fill) = fill {
                cx.canvas.fill_round_rect(b, t.round(10), fill);
            }
            if row.state == RowState::Waiting {
                cx.canvas.stroke_round_rect(b, t.round(10), 2, t.warn);
            }
            if let Some(bar) = bar {
                cx.canvas
                    .fill_round_rect(Rect::new(b.x, b.y + 10, 4, b.h - 20), 2, bar);
            }
            let sketch = Rect::new(
                b.right() - 14 - SKETCH_W,
                b.y + (b.h - SKETCH_H) / 2,
                SKETCH_W,
                SKETCH_H,
            );
            let words = Rect::new(b.x + 16, b.y, (sketch.x - 10 - b.x - 16).max(1), b.h);
            let tw = cx.text(
                FontId::Sans,
                t.size_small,
                words,
                Align::Right,
                tag_ink,
                &row.tags,
            );
            let lit = matches!(row.state, RowState::Picked | RowState::Waiting);
            cx.text(
                if lit { FontId::Bold } else { FontId::Sans },
                t.size_body,
                Rect {
                    w: (words.w - tw - 10).max(1),
                    ..words
                },
                Align::Left,
                ink,
                &row.name,
            );
            row.sketch.paint_small(cx, sketch, notes);
        }
        // The scroll bar, where the shelf is longer than the panel.
        if self.rows.len() > visible && visible > 0 {
            let track = Rect::new(r.right() - 12, body.y + 2, 4, body.h - 8);
            cx.canvas.fill_round_rect(track, 2, t.bg);
            let n = self.rows.len() as i32;
            let h = (track.h * visible as i32 / n).max(16);
            let y = track.y + (track.h - h) * first as i32 / (n - visible as i32);
            cx.canvas
                .fill_round_rect(Rect::new(track.x, y, 4, h), 2, t.outline);
        }
        // The shelf's note, in the room under its rows.
        let mut y = body.y + self.rows.len().min(visible) as i32 * ROW_H + 10;
        for line in wrap(cx, &self.note, t.size_small, body.w - 16) {
            if y + 20 > r.bottom() - 6 {
                break;
            }
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(body.x + 8, y, body.w - 16, 20),
                Align::Left,
                t.text_dim,
                &line,
            );
            y += 20;
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = (0..self.rows.len())
            .filter_map(|k| {
                let row = &self.rows[k];
                let lit = matches!(row.state, RowState::Picked | RowState::Waiting);
                Some(Target::tap("phrase", k, row.name.clone(), self.row(r, k)?).disabled_if(lit))
            })
            .collect();
        if self.new_idea {
            out.push(Target::tap(
                "new-idea",
                NEW_IDEA,
                "New idea",
                PhraseRows::new_idea_button(r),
            ));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if self.new_idea && PhraseRows::new_idea_button(r).contains(x, y) {
            return Some(EventKind::Toggle(NEW_IDEA));
        }
        let k = (0..self.rows.len()).find(|&k| self.row(r, k).is_some_and(|b| b.contains(x, y)))?;
        let row = &self.rows[k];
        // The pick is the pick already.
        (!matches!(row.state, RowState::Picked | RowState::Waiting))
            .then_some(EventKind::Select(row.index))
    }
}

// ---------------------------------------------------------------- kit

/// What the Phrase tab says on a kit track, in the place of the list and
/// the phrase that plays: a phrase moves notes, and a kit's pads are
/// sounds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhraseKit {
    pub title: String,
    pub lines: Vec<String>,
}

impl Widget for PhraseKit {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let mid = r.y + r.h / 2;
        let line = |y: i32, h: i32| Rect::new(r.x + 16, y, (r.w - 32).max(1), h);
        cx.text(
            FontId::Bold,
            t.size_title + 3.0,
            line(mid - 60, 36),
            Align::Center,
            t.text,
            &self.title,
        );
        for (i, text) in self.lines.iter().enumerate() {
            cx.text(
                FontId::Sans,
                t.size_body - 1.0,
                line(mid - 16 + i as i32 * 24, 24),
                Align::Center,
                t.text_dim,
                text,
            );
        }
    }
}

// ---------------------------------------------------------------- plays

/// What [`PhrasePlays`] shows, apart from its sliders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plays {
    /// The small heading: "Plays", or "Comes in on the next bar".
    pub head: String,
    /// The phrase shown waits for the bar line: its heading and its notes
    /// are in the waiting colour, and it has no cursor.
    pub waiting: bool,
    pub name: String,
    /// "Bass · 2 bars · straight · written in minor".
    pub about: String,
    pub sketch: Sketch,
    /// Where the phrase is as it plays, 0.0 to 1.0 of the way through.
    pub cursor: Option<f32>,
    /// How it follows the held note: a place in [`Follow::ALL`].
    pub follow: usize,
    /// The two lines beside that switch: the key, and whose it is.
    pub follows: [String; 2],
    /// More like this shows on the heading (composing tools P2, board A).
    pub more: bool,
    /// The heading in the good colour: a try of New idea or More like
    /// this ("Try 3 · Back keeps Night drive").
    pub head_good: bool,
}

impl Plays {
    /// The same but for where the cursor is.
    fn same_still(&self, other: &Plays) -> bool {
        // Every field is named, so a new one can't be left out unseen.
        let Plays {
            head,
            waiting,
            name,
            about,
            sketch,
            cursor: _,
            follow,
            follows,
            more,
            head_good,
        } = self;
        (
            head, waiting, name, about, sketch, follow, follows, more, head_good,
        ) == (
            &other.head,
            &other.waiting,
            &other.name,
            &other.about,
            &other.sketch,
            &other.follow,
            &other.follows,
            &other.more,
            &other.head_good,
        )
    }
}

/// The phrase that plays: its name and what it is, its notes with a cursor
/// where it is, how it follows the held note, and the four Q-Link sliders.
/// A tap on In key or Exact gives `Tab(i)`; a finger on slider `i` gives
/// `Select(i)`, then `Value(v)` as it moves ([`SliderStack`]).
pub struct PhrasePlays {
    pub show: Plays,
    /// The Q-Link sliders, drawn at the foot of this panel.
    pub sliders: SliderStack,
    /// What was last drawn where, the cursor apart: while only the cursor
    /// moves, only the picture is drawn again ([`Widget::changed_parts`]).
    drawn: RefCell<Option<Drawn>>,
}

struct Drawn {
    at: Rect,
    show: Plays,
    sliders: Vec<Slider>,
    active: Option<usize>,
}

/// The sliders' rows, the Follows switch and the room between the parts.
const SLIDERS_H: i32 = 168;
const SWITCH_W: i32 = 196;
const SWITCH_H: i32 = 44;

/// Where each part of the panel is.
struct Parts {
    head: Rect,
    name: Rect,
    about: Rect,
    picture: Rect,
    follows: Rect,
    switch: Rect,
    says: [Rect; 2],
    sliders: Rect,
}

impl PhrasePlays {
    pub fn new(labels: &[&str]) -> PhrasePlays {
        PhrasePlays {
            show: Plays::default(),
            sliders: SliderStack::new("", labels),
            drawn: RefCell::new(None),
        }
    }

    /// From the foot up the parts keep their sizes, so the picture takes
    /// whatever height the panel has over them.
    fn parts(r: Rect) -> Parts {
        let (x, w) = (r.x + 16, (r.w - 32).max(1));
        let sliders = Rect::new(x, r.bottom() - 10 - SLIDERS_H, w, SLIDERS_H);
        let switch = Rect::new(r.x + 14, sliders.y - 26 - SWITCH_H, SWITCH_W, SWITCH_H);
        let follows = Rect::new(x, switch.y - 26, w, 22);
        let top = r.y + 90;
        let beside = switch.right() + 12;
        let says = |i: i32| {
            Rect::new(
                beside,
                switch.y + 2 + i * 20,
                (r.right() - 16 - beside).max(1),
                20,
            )
        };
        Parts {
            head: Rect::new(x, r.y + 10, w, 22),
            name: Rect::new(x, r.y + 30, w, 30),
            about: Rect::new(x, r.y + 60, w, 22),
            picture: Rect::new(
                r.x + 14,
                top,
                (r.w - 28).max(1),
                (follows.y - 8 - top).max(1),
            ),
            follows,
            switch,
            says: [says(0), says(1)],
            sliders,
        }
    }

    /// More like this's button, at the right of the heading.
    pub fn more_button(r: Rect) -> Rect {
        Rect::new(r.right() - 14 - 140, r.y + 10, 140, 34)
    }

    /// Half `i` of the Follows switch (In key, Exact).
    pub fn half(r: Rect, i: usize) -> Rect {
        let s = PhrasePlays::parts(r).switch;
        Rect::new(s.x + i as i32 * (s.w / 2), s.y, s.w / 2, s.h)
    }

    /// Slider `i`'s area and its track ([`SliderStack::row_rects_in`]).
    pub fn slider(&self, r: Rect, i: usize) -> (Rect, Rect) {
        self.sliders.row_rects_in(PhrasePlays::parts(r).sliders, i)
    }

    /// What was last drawn at `at` is what shows now, but for where the
    /// cursor is.
    fn drawn_still(&self, at: Rect) -> bool {
        self.drawn.borrow().as_ref().is_some_and(|d| {
            d.at == at
                && d.active == self.sliders.active
                && d.sliders == self.sliders.rows
                && d.show.same_still(&self.show)
        })
    }
}

impl Widget for PhrasePlays {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        self.drawn_still(r)
            .then(|| vec![PhrasePlays::parts(r).picture])
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        if !self.drawn_still(r) {
            *self.drawn.borrow_mut() = Some(Drawn {
                at: r,
                show: self.show.clone(),
                sliders: self.sliders.rows.clone(),
                active: self.sliders.active,
            });
        }
        let p = PhrasePlays::parts(r);
        let s = &self.show;
        cx.panel(r);
        // The picture's own ink: the pick's, or the waiting colour.
        let ink = if s.waiting { t.warn } else { t.accent };
        s.sketch.paint_large(cx, p.picture, ink, s.cursor);
        // While only the cursor moves, only the picture is asked for.
        let clip = cx.canvas.clip();
        if clip.intersect(p.picture) == clip.intersect(r) {
            return;
        }
        let (head, name) = if s.more {
            let b = PhrasePlays::more_button(r);
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                b,
                Align::Center,
                t.text,
                "More like this",
            );
            let narrow = |q: Rect| Rect {
                w: (b.x - 10 - q.x).max(1),
                ..q
            };
            (narrow(p.head), narrow(p.name))
        } else {
            (p.head, p.name)
        };
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            if s.waiting {
                t.readable(t.warn, t.surface)
            } else if s.head_good {
                t.readable(t.good, t.surface)
            } else {
                t.text_dim
            },
            &s.head.to_uppercase(),
        );
        cx.text(
            FontId::Bold,
            t.size_title + 2.0,
            name,
            Align::Left,
            t.text,
            &s.name,
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            p.about,
            Align::Left,
            t.text_dim,
            &s.about,
        );
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            p.follows,
            Align::Left,
            t.text_dim,
            "FOLLOWS",
        );
        cx.canvas.fill_round_rect(p.switch, t.round(10), t.bg);
        for (i, f) in Follow::ALL.iter().enumerate() {
            let half = PhrasePlays::half(r, i);
            let on = i == s.follow;
            if on {
                cx.canvas
                    .fill_round_rect(half.inset(3), t.round(8), t.picked());
            }
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                half,
                Align::Center,
                if on {
                    t.on_picked()
                } else {
                    t.readable(t.text_dim, t.bg)
                },
                f.label(),
            );
        }
        cx.text(
            FontId::Bold,
            t.size_small,
            p.says[0],
            Align::Left,
            t.text,
            &s.follows[0],
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            p.says[1],
            Align::Left,
            t.text_dim,
            &s.follows[1],
        );
        self.sliders.paint_in(cx, p.sliders);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = Follow::ALL
            .iter()
            .enumerate()
            .map(|(i, f)| {
                Target::tap("follow", i, f.label(), PhrasePlays::half(r, i))
                    .disabled_if(i == self.show.follow)
            })
            .collect();
        if self.show.more {
            out.push(Target::tap(
                "more-like-this",
                MORE_LIKE,
                "More like this",
                PhrasePlays::more_button(r),
            ));
        }
        out.extend(self.sliders.targets_in(PhrasePlays::parts(r).sliders));
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase == Touch::Down && self.show.more && PhrasePlays::more_button(r).contains(x, y) {
            return Some(EventKind::Toggle(MORE_LIKE));
        }
        if phase == Touch::Down {
            if let Some(i) =
                (0..Follow::ALL.len()).find(|&i| PhrasePlays::half(r, i).contains(x, y))
            {
                return (i != self.show.follow).then(|| {
                    self.show.follow = i;
                    EventKind::Tab(i)
                });
            }
        }
        self.sliders
            .touch_in(finger, phase, x, y, PhrasePlays::parts(r).sliders)
    }
}

// ---------------------------------------------------------------- words

fn bars(p: &Phrase) -> String {
    match p.bars {
        1 => "1 bar".to_string(),
        n => format!("{n} bars"),
    }
}

/// A meter that is not 4/4, as a row says it.
fn meter(p: &Phrase) -> Option<String> {
    (p.beats != 4).then(|| format!("{}/4", p.beats))
}

/// What a row says after the name: how long the phrase is, then its meter
/// and its feel when they are not the plain ones, the scale it was written
/// in when that is the other kind than the key's, and whose it is when it
/// is only the open project's.
pub fn tags(p: &Phrase, other_scale: bool, origin: Origin) -> String {
    let mut parts = vec![bars(p)];
    parts.extend(meter(p));
    if p.feel() != Feel::Straight {
        parts.push(p.feel().label().to_lowercase());
    }
    if other_scale {
        parts.push(p.written.word().to_string());
    }
    if origin == Origin::Project {
        parts.push("this project".to_string());
    }
    parts.join(" · ")
}

/// The line under the name of the phrase that plays: what kind it is, how
/// long, its feel and the scale it was written in.
pub fn about(p: &Phrase) -> String {
    let kind = match p.kind {
        PhraseKind::Line => "Arp line",
        k => k.label(),
    };
    let mut parts = vec![kind.to_string(), bars(p)];
    parts.extend(meter(p));
    parts.push(p.feel().label().to_lowercase());
    parts.push(match p.written {
        Written::Either => "either scale".to_string(),
        w => format!("written in {}", w.word()),
    });
    parts.join(" · ")
}

/// A key as the tab names it: "A minor". A scale of the owner's own has
/// no name here.
pub fn key_name(key: Scale) -> String {
    let root = NOTE_NAMES[usize::from(key.root % 12)];
    match SCALES
        .iter()
        .find(|d| d.mask == key.mask && d.down == key.down)
    {
        Some(d) => format!("{root} {}", d.name.to_lowercase()),
        None => format!("{root}, your scale"),
    }
}

/// The two lines beside the Follows switch: what the phrase follows, and
/// whose key that is.
pub fn follow_lines(st: &ArpState) -> [String; 2] {
    let (what, whose) = match (st.settings.follow, st.phrase_key()) {
        (Follow::Exact, _) => ("By semitones".to_string(), "every interval as written"),
        (Follow::InKey, Some(key)) if st.settings.scale.is_some() => {
            (key_name(key), "the arp's own scale")
        }
        (Follow::InKey, Some(key)) => (key_name(key), "the pads' key"),
        (Follow::InKey, None) => ("No key set".to_string(), "plays by semitones"),
    };
    [what, whose.to_string()]
}

/// What the owner's shelf says under its rows. (The boards' line names
/// the three ways to make a phrase: a clip's card, a MIDI file in the
/// browser, a pattern that plays one note. None of them is built yet, so
/// it says only what is true today, and takes the boards' words as each
/// way lands.)
pub const OWN_NOTE: &str = "Phrases of your own are kept here. They play from any note, \
                            like MPFree's own. Making one from a clip, a MIDI file or a \
                            pattern comes next.";

// ---------------------------------------------------------------- models

/// What [`PhraseKinds`] shows for `st`.
pub fn kinds(st: &ArpState) -> PhraseKinds {
    let shown = st.shelf();
    PhraseKinds {
        items: Shelf::ALL
            .iter()
            .map(|s| (s.label().to_string(), st.phrases.count(*s)))
            .collect(),
        shown: Shelf::ALL.iter().position(|s| *s == shown).unwrap_or(0),
        dim: st.phrase.kit_track.is_some(),
    }
}

/// What [`PhraseRows`] shows for `st`: the shown shelf in the order that
/// suits the key, the pick lit, and, while the pick waits for the bar
/// line, the pick amber and the phrase that plays until then green.
pub fn rows(st: &ArpState) -> PhraseRows {
    let shelf = st.shelf();
    let list = st.phrases.rows(shelf, st.phrase_key());
    let waits = st.phrase.waits;
    let rows = list
        .iter()
        .map(|row| {
            let (name, p) = &st.phrases.phrases[row.index];
            let state = if row.index == st.phrase_index {
                if waits {
                    RowState::Waiting
                } else {
                    RowState::Picked
                }
            } else if waits && st.phrase.playing == Some(row.index) {
                RowState::Playing
            } else {
                RowState::Plain
            };
            PhraseRow {
                index: row.index,
                name: name.clone(),
                tags: match state {
                    RowState::Waiting => "next bar".to_string(),
                    RowState::Playing => "playing".to_string(),
                    _ => tags(p, row.other_scale, row.origin),
                },
                state,
                dim: row.other_scale,
                sketch: Sketch::of(p),
            }
        })
        .collect();
    PhraseRows {
        title: shelf.label().to_string(),
        new_idea: true,
        picked: list.iter().position(|r| r.index == st.phrase_index),
        rows,
        note: if shelf == Shelf::Own {
            OWN_NOTE.to_string()
        } else {
            String::new()
        },
    }
}

/// What [`PhrasePlays`] shows for `st`: the track's pick, which is the
/// phrase a held note plays, or the one that comes in on the next bar.
pub fn plays(st: &ArpState) -> Plays {
    let follow = Follow::ALL
        .iter()
        .position(|f| *f == st.settings.follow)
        .unwrap_or(0);
    let follows = follow_lines(st);
    // New idea or More like this: its try, which a held pad plays.
    if let Some((name, p)) = st.maker.as_ref().and_then(|m| m.shown.as_ref()) {
        return Plays {
            head: st
                .maker
                .as_ref()
                .map_or_else(String::new, |m| m.head.clone()),
            head_good: true,
            name: name.clone(),
            about: about(p),
            sketch: Sketch::of(p),
            cursor: st.phrase.at,
            follow,
            follows,
            ..Plays::default()
        };
    }
    let Some((name, p)) = st.phrases.phrases.get(st.phrase_index) else {
        return Plays {
            head: "Plays".to_string(),
            name: "No phrase".to_string(),
            follow,
            follows,
            ..Plays::default()
        };
    };
    let waiting = st.phrase.waits;
    Plays {
        head: if waiting {
            "Comes in on the next bar"
        } else {
            "Plays"
        }
        .to_string(),
        waiting,
        name: name.clone(),
        about: about(p),
        sketch: Sketch::of(p),
        cursor: if waiting { None } else { st.phrase.at },
        follow,
        follows,
        more: st.maker.is_none(),
        head_good: false,
    }
}

/// What [`PhraseKit`] says for `st`'s kit track.
pub fn kit(st: &ArpState) -> PhraseKit {
    let track = st.phrase.kit_track.as_deref().unwrap_or("This track");
    PhraseKit {
        title: "Phrases play on a synth: set the track to a synth".to_string(),
        lines: vec![
            format!("{track}'s instrument is a kit, so its pads play as pressed."),
            "On a synth, a held pad plays the phrase from its note.".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{samples, Sample};
    use crate::view::{
        chip, hint_for, qlink_rows, ArpAction, KINDS, KIT, MODES, PHRASES, PLAYS, STRAIGHT,
        SWING_TRIPLETS,
    };
    use mpfree_arp::{Mode, Settings, Speed};
    use mpfree_ui::{Fonts, SoftCanvas, Touch, Ui};

    fn sample(name: &str) -> Sample {
        samples(Fonts::embedded)
            .unwrap()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no sample {name}"))
    }

    fn tap(ui: &mut Ui, r: Rect) -> Option<mpfree_ui::UiEvent> {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        let ev = ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
        ev
    }

    /// The five boards' panels: the kinds 166 wide at the left, the phrase
    /// that plays 397 wide at the right, the list between them, all the
    /// same height under the tabs.
    #[test]
    fn the_three_panels_sit_side_by_side_under_the_tabs() {
        let s = sample("phrase-browse");
        let [tabs, kinds, list, plays] =
            [MODES, KINDS, PHRASES, PLAYS].map(|n| s.ui.rect_of(n).unwrap());
        assert_eq!((kinds.w, plays.w), (166, 397));
        assert_eq!(kinds.x, tabs.x);
        assert_eq!(plays.right(), tabs.right());
        assert_eq!((list.x, list.right()), (kinds.right() + 12, plays.x - 12));
        for p in [kinds, list, plays] {
            assert_eq!((p.y, p.h), (tabs.bottom() + 12, kinds.h), "{p:?}");
        }
        assert!(s.ui.rect_of(KIT).is_none());
    }

    #[test]
    fn the_kinds_show_each_shelfs_count_and_the_one_the_list_shows() {
        let mut s = sample("phrase-browse");
        let k = kinds(&s.state);
        let items: Vec<(&str, usize)> = k.items.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        assert_eq!(
            items,
            [
                ("Bass", 16),
                ("Riff", 16),
                ("Chords", 16),
                ("Arp lines", 16),
                ("Yours", 0)
            ]
        );
        assert_eq!((k.shown, k.dim), (0, false));
        // An empty kind stepped to shows in place of the pick's.
        s.state.phrase.shelf = Some(Shelf::Own);
        assert_eq!(kinds(&s.state).shown, 4);
        // Every row is a fingertip tall and inside the panel, the owner's
        // a little apart from MPFree's.
        let r = s.ui.rect_of(KINDS).unwrap();
        let rows: Vec<Rect> = (0..5).map(|i| k.row(r, i)).collect();
        for (i, row) in rows.iter().enumerate() {
            assert!(
                row.h >= 44 && row.x >= r.x && row.right() <= r.right(),
                "{i}"
            );
            assert!(row.bottom() <= r.bottom(), "{i}: {row:?} in {r:?}");
        }
        assert_eq!(rows[1].y - rows[0].y, ROW_H);
        assert_eq!(rows[4].y - rows[3].y, ROW_H + APART);
        // A tap on Chords asks for its phrases; the kind shown answers
        // nothing.
        let ev = tap(&mut s.ui, rows[2]).unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut None),
            Some(ArpAction::Shelf(Shelf::Kind(PhraseKind::Chords)))
        );
        let mut again = sample("phrase-browse");
        assert_eq!(tap(&mut again.ui, rows[0]), None);
    }

    /// The list is the shown kind's phrases in the order that suits the
    /// key, each with how long it is and what is not plain about it, the
    /// pick lit and kept in view.
    #[test]
    fn the_list_is_the_shown_kind_with_the_pick_lit() {
        let s = sample("phrase-browse");
        let list = rows(&s.state);
        assert_eq!((list.title.as_str(), list.rows.len()), ("Bass", 16));
        let at = list.picked.expect("the pick is a bass");
        assert_eq!(list.rows[at].name, "Night drive");
        assert_eq!(list.rows[at].index, s.state.phrase_index);
        for (k, row) in list.rows.iter().enumerate() {
            let want = if k == at {
                RowState::Picked
            } else {
                RowState::Plain
            };
            assert_eq!(row.state, want, "{}", row.name);
            assert!(!row.sketch.notes.is_empty(), "{}", row.name);
        }
        let tags = |name: &str| {
            let row = list.rows.iter().find(|r| r.name == name).unwrap();
            row.tags.clone()
        };
        assert_eq!(tags("Night drive"), "2 bars");
        assert_eq!(tags("Gallop bass"), "1 bar");
        assert_eq!(tags("Garage skip"), "2 bars · swung");
        assert_eq!(tags("Waltz bass"), "4 bars · 3/4");
        assert_eq!(list.note, "");
        // On the screen: the pick's row is in view, every row in view is a
        // fingertip tall and inside the panel, and the rest are not there
        // to touch.
        let r = s.ui.rect_of(PHRASES).unwrap();
        let seen: Vec<usize> = (0..16).filter(|&k| list.row(r, k).is_some()).collect();
        assert!(seen.contains(&at), "{seen:?}");
        assert!(seen.len() >= 9 && seen.windows(2).all(|w| w[1] == w[0] + 1));
        for &k in &seen {
            let row = list.row(r, k).unwrap();
            assert!(row.h >= 44 && row.y > r.y && row.bottom() <= r.bottom());
        }
        // The rows in view, and New idea in the heading.
        let w = s.ui.get::<PhraseRows>(PHRASES).unwrap();
        assert_eq!(w.targets(r).len(), seen.len() + 1);
        assert_eq!(*w, list, "the panel shows the model");
    }

    #[test]
    fn a_tap_on_a_phrase_picks_it() {
        let mut s = sample("phrase-browse");
        let list = rows(&s.state);
        let r = s.ui.rect_of(PHRASES).unwrap();
        let at = list.picked.unwrap();
        let next = list.row(r, at + 1).unwrap();
        let ev = tap(&mut s.ui, next).unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut None),
            Some(ArpAction::Phrase(list.rows[at + 1].index))
        );
        assert_eq!(list.rows[at + 1].name, "Acid line");
        // The pick is the pick already.
        let picked = list.row(r, at).unwrap();
        assert_eq!(tap(&mut s.ui, picked), None);
    }

    /// In a minor key the phrases written in major come last, quieter,
    /// and say so; the pick among them is lit all the same.
    #[test]
    fn phrases_written_in_the_other_scale_come_last_and_say_so() {
        let s = sample("phrase-exact");
        let list = rows(&s.state);
        let split = list.rows.iter().position(|r| r.dim).unwrap();
        assert!(split > 0 && list.rows[split..].iter().all(|r| r.dim));
        assert!(list.rows[..split].iter().all(|r| !r.dim));
        let row = |name: &str| list.rows.iter().find(|r| r.name == name).unwrap();
        assert_eq!(row("Anthem").tags, "4 bars · major");
        assert_eq!(row("Waltz tune").tags, "4 bars · 3/4 · major");
        assert_eq!(row("Lazy swing").tags, "2 bars · swung · major");
        assert_eq!(row("Shuffle riff").tags, "2 bars · triplet");
        let pick = row("Synth pop hook");
        assert_eq!((pick.state, pick.dim), (RowState::Picked, true));
        assert_eq!(pick.tags, "2 bars · major");
    }

    /// While the song plays a pick waits for the bar line: it is amber and
    /// says "next bar", the phrase still playing is green and says
    /// "playing", the large picture is the pick's with no cursor, and the
    /// chip still names what is heard.
    #[test]
    fn a_pick_waiting_for_the_bar_line_is_amber_and_the_one_playing_green() {
        let s = sample("phrase-next-bar");
        let list = rows(&s.state);
        let state = |name: &str| {
            let row = list.rows.iter().find(|r| r.name == name).unwrap();
            (row.state, row.tags.as_str())
        };
        assert_eq!(state("Acid line"), (RowState::Waiting, "next bar"));
        assert_eq!(state("Night drive"), (RowState::Playing, "playing"));
        assert_eq!(state("Sub drop"), (RowState::Plain, "2 bars"));
        let p = plays(&s.state);
        assert_eq!(
            (p.head.as_str(), p.name.as_str(), p.waiting, p.cursor),
            ("Comes in on the next bar", "Acid line", true, None)
        );
        assert_eq!(chip(&s.state).text, "Phrase: Night drive");
        // Neither answers a tap for a pick: one is the pick, and the
        // other can be picked back.
        let r = s.ui.rect_of(PHRASES).unwrap();
        let w = s.ui.get::<PhraseRows>(PHRASES).unwrap();
        let off: Vec<String> = w
            .targets(r)
            .into_iter()
            .filter(|t| !t.enabled)
            .map(|t| t.label)
            .collect();
        assert_eq!(off, ["Acid line"]);
        // Once it is in (or with the transport stopped, where it comes in
        // on the next step) nothing waits: the pick is lit as ever.
        let mut st = s.state.clone();
        st.phrase.waits = false;
        let list = rows(&st);
        assert!(list
            .rows
            .iter()
            .all(|r| matches!(r.state, RowState::Plain | RowState::Picked)));
        assert_eq!(plays(&st).head, "Plays");
        assert_eq!(chip(&st).text, "Phrase: Acid line");
    }

    #[test]
    fn the_phrase_that_plays_says_what_it_is_and_where_it_is() {
        let s = sample("phrase-browse");
        let p = plays(&s.state);
        assert_eq!((p.head.as_str(), p.name.as_str()), ("Plays", "Night drive"));
        assert_eq!(p.about, "Bass · 2 bars · straight · written in minor");
        assert_eq!(p.follow, 0);
        assert_eq!(
            p.follows,
            ["A minor".to_string(), "the pads' key".to_string()]
        );
        // Three beats into two bars.
        let at = p.cursor.expect("it plays");
        assert!((at - 0.375).abs() < 0.01, "{at}");
        assert_eq!(
            p.sketch,
            Sketch::of(&s.state.phrases.phrases[s.state.phrase_index].1)
        );
        // A line's kind in the singular, and a phrase for either scale.
        let mut st = s.state.clone();
        st.phrase_index = st
            .phrases
            .phrases
            .iter()
            .position(|(n, _)| n == "Up the chord")
            .unwrap();
        assert_eq!(
            plays(&st).about,
            "Arp line · 1 bar · straight · either scale"
        );
        // Nothing held: no cursor.
        st.phrase.at = None;
        assert_eq!(plays(&st).cursor, None);
    }

    #[test]
    fn follows_says_what_the_phrase_follows_and_whose_key_it_is() {
        let mut st = sample("phrase-browse").state;
        let says = |st: &ArpState| follow_lines(st).join(" / ");
        assert_eq!(says(&st), "A minor / the pads' key");
        // The arp's own scale wins over the pads'.
        st.settings.scale = Some(Scale::new(2, mpfree_arp::scale::MAJOR));
        assert_eq!(says(&st), "D major / the arp's own scale");
        st.settings.scale = None;
        st.pads_key = None;
        assert_eq!(says(&st), "No key set / plays by semitones");
        st.settings.follow = Follow::Exact;
        assert_eq!(says(&st), "By semitones / every interval as written");
        // A scale that is nobody's but the owner's has no name here.
        assert_eq!(key_name(Scale::new(4, 0b0000_1001_0001)), "E, your scale");
    }

    /// In key and Exact are a switch; the four sliders under it are the
    /// Q-Links', the first the phrase's speed.
    #[test]
    fn the_switch_and_the_sliders_set_how_the_phrase_plays() {
        let mut s = sample("phrase-browse");
        let r = s.ui.rect_of(PLAYS).unwrap();
        let halves = [PhrasePlays::half(r, 0), PhrasePlays::half(r, 1)];
        for h in halves {
            assert!(h.w >= 44 && h.h >= 44 && h.x >= r.x && h.right() <= r.right());
        }
        assert_eq!(tap(&mut s.ui, halves[0]), None, "In key is on already");
        let ev = tap(&mut s.ui, halves[1]).unwrap();
        let Some(ArpAction::Settings(set)) = s.page.action(&ev, &s.state, &mut None) else {
            panic!("{ev:?}")
        };
        assert_eq!(set.follow, Follow::Exact);
        assert_eq!(
            Settings {
                follow: Follow::InKey,
                ..set
            },
            s.state.settings,
            "and nothing else"
        );
        // The sliders: none over another or over the switch, all inside.
        let w = s.ui.get::<PhrasePlays>(PLAYS).unwrap();
        let labels: Vec<&str> = w.sliders.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Speed · Q1", "Gate · Q2", "Swing · Q3", "Octaves · Q4"]
        );
        assert_eq!(w.sliders.rows[0].text, "as written");
        let areas: Vec<Rect> = (0..4).map(|i| w.slider(r, i).0).collect();
        for (i, a) in areas.iter().enumerate() {
            assert!(a.h >= 40 && a.y >= halves[0].bottom() && a.bottom() <= r.bottom());
            assert!(areas[i + 1..].iter().all(|b| a.intersect(*b).is_empty()));
        }
        // A finger on Speed, moved to the right end: double.
        let (_, track) = w.slider(r, 0);
        let mut qlink = None;
        let ev = s.ui.touch(2, Touch::Down, track.x + 5, track.y).unwrap();
        assert_eq!(s.page.action(&ev, &s.state, &mut qlink), None);
        assert_eq!(qlink, Some(0));
        let ev =
            s.ui.touch(2, Touch::Move, track.right() + 40, track.y)
                .unwrap();
        let Some(ArpAction::Settings(set)) = s.page.action(&ev, &s.state, &mut qlink) else {
            panic!("{ev:?}")
        };
        assert_eq!(
            (set.speed, set.rate),
            (Speed::Double, s.state.settings.rate)
        );
        s.ui.touch(2, Touch::Up, 0, 0);
    }

    /// Swing leaves a phrase in triplets as it is, so its slider is quiet
    /// and says so; and a Modifier's push on the rate shows on the speed.
    #[test]
    fn swing_is_quiet_on_triplets_and_a_push_shows_on_the_speed() {
        let s = sample("phrase-triplets-pushed");
        let rows = qlink_rows(&s.state);
        assert_eq!(
            (rows[2].label.as_str(), rows[2].text.as_str(), rows[2].quiet),
            (SWING_TRIPLETS, STRAIGHT, true)
        );
        assert_eq!(rows[2].pushed, None);
        assert!(rows[2].value > 0.0, "the setting keeps its place");
        // 1/16 pushed to 1/16T plays the phrase one and a half times as
        // fast.
        assert_eq!(rows[0].text, "as written");
        let (at, to) = rows[0].pushed.clone().expect("pushed");
        assert!(to == "1.5×" && at > rows[0].value, "{at} {to}");
        assert!(rows.iter().enumerate().all(|(i, r)| r.quiet == (i == 2)));
        // A straight phrase swings as set.
        let plain = qlink_rows(&sample("phrase-exact").state);
        assert_eq!(
            (
                plain[2].label.as_str(),
                plain[2].text.as_str(),
                plain[2].quiet
            ),
            ("Swing · Q3", "58 %", false)
        );
        assert!(plain[0].pushed.is_none());
        // The panel's own sliders show the same rows.
        let w = s.ui.get::<PhrasePlays>(PLAYS).unwrap();
        assert_eq!(w.sliders.rows, rows);
    }

    /// On a kit track the kinds are faint and one panel says why no phrase
    /// plays, naming the track.
    #[test]
    fn a_kit_track_says_its_pads_play_as_pressed() {
        let s = sample("phrase-kit");
        assert_eq!(s.state.tab(), Mode::Phrase);
        assert_eq!(
            s.state.settings.mode,
            Mode::Direct,
            "the pads play as pressed"
        );
        assert!(kinds(&s.state).dim);
        let (k, panel) = (s.ui.rect_of(KINDS).unwrap(), s.ui.rect_of(KIT).unwrap());
        assert!(s.ui.rect_of(PHRASES).is_none() && s.ui.rect_of(PLAYS).is_none());
        assert_eq!((k.w, panel.x), (166, k.right() + 12));
        let says = kit(&s.state);
        assert_eq!(
            says.title,
            "Phrases play on a synth: set the track to a synth"
        );
        assert_eq!(
            says.lines,
            [
                "Track 1's instrument is a kit, so its pads play as pressed.",
                "On a synth, a held pad plays the phrase from its note."
            ]
        );
        assert_eq!(
            hint_for(&s.state),
            "Track 1's instrument is a kit · phrases play on a synth · − / +: kind"
        );
        // The Phrase tab is the one lit.
        let tabs = s.ui.get::<crate::widgets::Segments>(MODES).unwrap();
        assert_eq!(tabs.items[tabs.active], "Phrase");
    }

    /// The owner's shelf holds their phrases whatever their kind, then the
    /// open project's, which say so; under them it says what the shelf is.
    #[test]
    fn the_owners_shelf_holds_their_phrases_and_the_projects() {
        let s = sample("phrase-yours");
        let list = rows(&s.state);
        assert_eq!(list.title, "Yours");
        let got: Vec<(&str, &str)> = list
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.tags.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Verse bass", "1 bar"),
                ("riff-idea", "1 bar"),
                ("Climbing line", "1 bar · this project")
            ]
        );
        assert_eq!((list.picked, list.note.as_str()), (Some(0), OWN_NOTE));
        assert_eq!(kinds(&s.state).items[4], ("Yours".to_string(), 3));
        // Before the owner has any: the note alone, the pick still shown
        // as the phrase that plays.
        let s = sample("phrase-yours-empty");
        let list = rows(&s.state);
        assert_eq!((list.rows.len(), list.picked), (0, None));
        assert_eq!(list.note, OWN_NOTE);
        assert_eq!(kinds(&s.state).shown, 4);
        assert_eq!(plays(&s.state).name, "Night drive");
    }

    /// Every one of MPFree's phrases draws inside its picture, small and
    /// large, one mark a note.
    #[test]
    fn every_phrases_picture_stays_inside_its_box() {
        let list = crate::phrases::PhraseList::built_in();
        for (name, p) in &list.phrases {
            let sketch = Sketch::of(p);
            assert_eq!(sketch.notes.len(), p.len(), "{name}");
            for (r, min_h) in [
                (Rect::new(40, 30, SKETCH_W, SKETCH_H), 3),
                (Rect::new(700, 300, 357, 110), 3),
            ] {
                let boxes = sketch.boxes(r, min_h);
                assert_eq!(boxes.len(), p.len(), "{name}");
                for (b, _) in boxes {
                    assert!(
                        b.x >= r.x && b.right() <= r.right() && b.w >= 1,
                        "{name}: {b:?} across {r:?}"
                    );
                    assert!(
                        b.y >= r.y && b.bottom() <= r.bottom() && b.h >= 3,
                        "{name}: {b:?} down {r:?}"
                    );
                }
            }
        }
        // Nothing to draw: nothing drawn.
        assert!(Sketch::default()
            .boxes(Rect::new(0, 0, 100, 30), 3)
            .is_empty());
    }

    /// While only the cursor moves, only the picture is drawn again, and
    /// what is on the screen is what a whole new drawing would show.
    #[test]
    fn a_moving_cursor_redraws_only_the_picture() {
        let mut s = sample("phrase-browse");
        let mut canvas = SoftCanvas::new(1280, 800);
        s.ui.paint(&mut canvas);
        let r = s.ui.rect_of(PLAYS).unwrap();
        let mut st = s.state.clone();
        st.phrase.at = Some(0.6);
        s.page.update(&mut s.ui, &st).unwrap();
        let w = s.ui.get::<PhrasePlays>(PLAYS).unwrap();
        let parts = w.changed_parts(r).expect("only a part");
        assert_eq!(parts.len(), 1);
        let picture = parts[0];
        assert!(picture.h < r.h / 2 && picture.x >= r.x && picture.right() <= r.right());
        let damage = s.ui.paint(&mut canvas);
        assert_eq!(damage, vec![picture], "nothing else was drawn again");
        // The same state drawn whole on a new screen.
        let mut fresh = sample("phrase-browse");
        fresh.page.update(&mut fresh.ui, &st).unwrap();
        let mut whole = SoftCanvas::new(1280, 800);
        fresh.ui.paint(&mut whole);
        assert!(canvas.px == whole.px, "the part and the whole differ");
        // The same state again: nothing is drawn at all.
        s.page.update(&mut s.ui, &st).unwrap();
        assert!(s.ui.paint(&mut canvas).is_empty());
        // Another phrase: the whole panel.
        st.phrase_index += 1;
        s.page.update(&mut s.ui, &st).unwrap();
        let w = s.ui.get::<PhrasePlays>(PLAYS).unwrap();
        assert_eq!(w.changed_parts(r), None);
    }

    /// Board A: New idea in the list's heading, More like this on the
    /// phrase that plays, each a fingertip tall and each asking the app.
    #[test]
    fn new_idea_and_more_like_this_open_from_the_tab() {
        let mut s = sample("phrase-browse");
        let list = s.ui.rect_of(PHRASES).unwrap();
        let plays_r = s.ui.rect_of(PLAYS).unwrap();
        let b = PhraseRows::new_idea_button(list);
        assert!(list.contains(b.x, b.y) && b.h >= 34 && b.right() < list.right());
        let ev = tap(&mut s.ui, b).unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut None),
            Some(ArpAction::NewIdea)
        );
        let m = PhrasePlays::more_button(plays_r);
        assert!(plays_r.contains(m.x, m.y) && m.right() < plays_r.right());
        let ev = tap(&mut s.ui, m).unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut None),
            Some(ArpAction::MoreLike)
        );
        assert!(plays(&s.state).more);
    }

    /// Boards B and C: the panel takes the list's place, the detail panel
    /// shows the try in the good colour, and the panel's touches ask the
    /// app.
    #[test]
    fn the_makers_panel_takes_the_lists_place_and_shows_its_try() {
        use crate::maker::{PhraseMaker, MAKER};
        use crate::view::MakerAction;
        for name in ["phrase-new-idea", "phrase-more-like"] {
            let mut s = sample(name);
            assert!(s.ui.rect_of(PHRASES).is_none(), "{name}");
            let panel = s.ui.rect_of(MAKER).expect("the maker's panel");
            let kinds = s.ui.rect_of(KINDS).unwrap();
            let plays_r = s.ui.rect_of(PLAYS).unwrap();
            assert_eq!(
                (panel.x, panel.right()),
                (kinds.right() + 12, plays_r.x - 12)
            );
            let p = plays(&s.state);
            let m = s.state.maker.as_ref().unwrap();
            assert!(p.head_good && !p.more);
            assert_eq!(p.name, m.shown.as_ref().unwrap().0);
            assert_eq!(p.head, m.head);
            assert_eq!(hint_for(&s.state), crate::view::MAKER_HINT);
            let w = s.ui.get::<PhraseMaker>(MAKER).unwrap();
            let g = w.geo(panel);
            let last = g.steppers.len() - 1;
            let st = s.state.clone();
            let s1 = g.steppers[last];
            let ev = tap(&mut s.ui, Rect::new(s1.right() - 30, s1.y, 20, s1.h)).unwrap();
            assert_eq!(
                s.page.action(&ev, &st, &mut None),
                Some(ArpAction::Maker(MakerAction::Step {
                    row: last,
                    delta: 1
                }))
            );
            let want = [
                MakerAction::TryAgain,
                MakerAction::Previous,
                MakerAction::Back,
                MakerAction::Keep,
            ];
            for (k, b) in g.buttons.iter().enumerate() {
                let ev = tap(&mut s.ui, *b).unwrap();
                assert_eq!(
                    s.page.action(&ev, &st, &mut None),
                    Some(ArpAction::Maker(want[k]))
                );
            }
            let ev = tap(&mut s.ui, g.close).unwrap();
            assert_eq!(
                s.page.action(&ev, &st, &mut None),
                Some(ArpAction::Maker(MakerAction::Back))
            );
            // Closed, the list is back.
            s.state.maker = None;
            s.page.update(&mut s.ui, &s.state).unwrap();
            assert!(s.ui.rect_of(MAKER).is_none() && s.ui.rect_of(PHRASES).is_some());
        }
    }
}
