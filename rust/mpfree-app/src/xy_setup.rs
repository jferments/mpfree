//! Setup for a layout of yours (layout spec 4o, board 3 of the XY
//! mockups; the XY controller plan's phase 2): on today's full-screen pad
//! it takes the whole view, and Done gives the pad back.
//!
//! - **The head:** SETUP, the layout's name (a tap types it, with its
//!   directions' names), then **Save to browser**, **Delete** and **Done**
//!   (the accent) at the right. Save to browser keeps a copy in the
//!   browser (Instruments › XY pad) for other songs.
//! - **Each direction:** "ACROSS →" or "UP ↑", its name, "2 of 8", and
//!   **+ across** or **+ up** (greyed at 8 of 8), which raises the waiting
//!   banner.
//! - **A row per setting** (48 px, 6 apart): a dot in its colour, its name
//!   and where it is, its from value, a range bar with two handles (each
//!   dragged; "runs backwards" under it when from is above to), its to
//!   value, its curve (a chip opening Even, Slow start, Fast start,
//!   S-shaped), **Set | Add** and ×. A tap picks a row (outlined in the
//!   accent) for the Q-Links. A setting that went away is struck out with
//!   the reason and **Take off**. The rows scroll with a drag when they
//!   don't fit.
//! - **At the foot, a card:** When you let go (Go back | Stay), Glide back
//!   (– and +), A touch (Jumps | Moves from here), Follow the selected
//!   track (Off | On), and two faint lines.
//!
//! What a finger does is kept as [`SetupAct`]s for the screen to take
//! ([`XySetup::take`]); the touch answers `Select(0)` when there are some.
//! Each drag shows at once; the sound loop's answer replaces the view.

use mpfree_mod::MapCurve;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Theme, Touch, Widget};

use mpfree_xy::GLIDE_MAX_MS;

use crate::xy_song::{curve_name, Dot, MineView, RowView, CURVES, GLIDE_STEP_MS};

/// The widget's name, and its page.
pub const XY_SETUP: &str = "xy-setup";
pub const XY_SETUP_PAGE: &str = "Play · XY setup";

/// What a finger did on Setup.
#[derive(Clone, Debug, PartialEq)]
pub enum SetupAct {
    /// Row `i` picked (for the Q-Links).
    Pick(usize),
    /// Row `row`'s ends, as `RowView::from` and `to` are.
    Range {
        row: usize,
        from: f32,
        to: f32,
    },
    Curve {
        row: usize,
        curve: MapCurve,
    },
    How {
        row: usize,
        add: bool,
    },
    /// A tap on a greyed Add: the hint line says why.
    NoAdd(usize),
    TakeOff(usize),
    /// When you let go: Stay (true) or Go back.
    Lift(bool),
    /// Glide back, in ms (– and + step it by `GLIDE_STEP_MS`).
    Glide(f32),
    /// A touch moves from here (true) or jumps.
    Start(bool),
    Follow(bool),
    /// + across (false) or + up.
    Plus(bool),
    /// A name tapped: type them.
    Name,
    /// Save to browser: a copy for other songs.
    Save,
    Delete,
    Done,
}

const HEAD_H: i32 = 48;
const FOOT_H: i32 = 128;
const DIR_H: i32 = 44;
const ROW_H: i32 = 48;
const ROW_GAP: i32 = 6;
const BUTTON_H: i32 = 36;
const CURVE_W: i32 = 128;
const HOW_W: i32 = 112;
const X_W: i32 = 40;
const VALUE_W: i32 = 72;
const NAME_W: i32 = 210;
/// A drag this far is a scroll, not a tap.
const SCROLL_SLOP: i32 = 12;

/// What a finger holds.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// A range's handle: the row, and which end (0 from, 1 to).
    Handle { finger: u64, row: usize, end: usize },
    /// The rows, from where the finger landed and the scroll then.
    Scroll {
        finger: u64,
        y: i32,
        from: i32,
        moved: bool,
    },
}

/// One item of the list of settings, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    /// A direction's head: up or across.
    Dir(bool),
    /// Row `i` of the view's rows.
    Row(usize),
}

/// Setup on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XySetup {
    /// The layout as the sound loop described it.
    pub view: MineView,
    /// The row picked.
    pub picked: Option<usize>,
    scroll: i32,
    /// The row whose curve list is open.
    curve_open: Option<usize>,
    drag: Option<Drag>,
    acts: Vec<SetupAct>,
}

/// The parts of Setup's place.
struct Geo {
    name: Rect,
    save: Rect,
    delete: Rect,
    done: Rect,
    list: Rect,
    foot: Rect,
}

impl XySetup {
    /// What the fingers did since the last call.
    pub fn take(&mut self) -> Vec<SetupAct> {
        std::mem::take(&mut self.acts)
    }

    /// A new view from the sound loop (a picked row stays picked while
    /// it is there).
    pub fn set_view(&mut self, v: MineView) {
        if self.picked.is_some_and(|p| p >= v.rows.len()) {
            self.picked = None;
        }
        if self.curve_open.is_some_and(|p| p >= v.rows.len()) {
            self.curve_open = None;
        }
        // A handle being dragged keeps what the finger shows.
        if let Some(Drag::Handle { row, .. }) = self.drag {
            let same = match (self.view.rows.get(row), v.rows.get(row)) {
                (Some(mine), Some(new)) if new.up == mine.up && new.k == mine.k => {
                    Some((mine.from, mine.to))
                }
                _ => None,
            };
            if let Some((from, to)) = same {
                self.view = v;
                self.view.rows[row].from = from;
                self.view.rows[row].to = to;
                return;
            }
        }
        self.view = v;
    }

    /// Opened afresh: the top of the list, nothing picked.
    pub fn reset(&mut self) {
        self.scroll = 0;
        self.picked = None;
        self.curve_open = None;
        self.drag = None;
    }

    fn geo(r: Rect) -> Geo {
        let inner = r.inset(12);
        let (head, rest) = inner.take_top(HEAD_H);
        let (foot, rest) = rest.take_bottom(FOOT_H);
        let list = rest.inset_xy(0, 8);
        let by = head.y + (head.h - 44) / 2;
        let done = Rect::new(head.right() - 120, by, 120, 44);
        let delete = Rect::new(done.x - 10 - 100, by, 100, 44);
        let save = Rect::new(delete.x - 10 - 160, by, 160, 44);
        let name = Rect::new(head.x + 72, by, 260, 44);
        Geo {
            name,
            save,
            delete,
            done,
            list,
            foot,
        }
    }

    /// The list's items with their tops (from the list's top, unscrolled)
    /// and heights.
    fn items(&self) -> Vec<(Item, i32, i32)> {
        let mut out = Vec::new();
        let mut y = 0;
        for up in [false, true] {
            out.push((Item::Dir(up), y, DIR_H));
            y += DIR_H + ROW_GAP;
            for (i, _) in self
                .view
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.up == up)
            {
                out.push((Item::Row(i), y, ROW_H));
                y += ROW_H + ROW_GAP;
            }
            y += 4;
        }
        out
    }

    fn content_h(&self) -> i32 {
        self.items().last().map_or(0, |(_, y, h)| y + h)
    }

    fn max_scroll(&self, list: Rect) -> i32 {
        (self.content_h() - list.h).max(0)
    }

    /// Where an item is on the screen (it may be outside the list).
    fn item_rect(&self, list: Rect, y: i32, h: i32) -> Rect {
        Rect::new(list.x, list.y + y - self.scroll, list.w, h)
    }

    /// The parts of a direction's head: its name field and + across.
    fn dir_parts(d: Rect) -> (Rect, Rect) {
        let name = Rect::new(d.x + 96, d.y + (d.h - BUTTON_H) / 2, 170, BUTTON_H);
        let plus = Rect::new(d.right() - 124, d.y + (d.h - BUTTON_H) / 2, 120, BUTTON_H);
        (name, plus)
    }

    /// A row's parts: the name block, from, the bar, to, the curve chip,
    /// Set, Add and ×.
    fn row_parts(rr: Rect) -> RowParts {
        let cy = rr.y + (rr.h - BUTTON_H) / 2;
        let x = Rect::new(rr.right() - 6 - X_W, cy, X_W, BUTTON_H);
        let how = Rect::new(x.x - 8 - HOW_W, cy, HOW_W, BUTTON_H);
        let set = Rect::new(how.x, cy, HOW_W / 2, BUTTON_H);
        let add = Rect::new(how.x + HOW_W / 2, cy, HOW_W / 2, BUTTON_H);
        let curve = Rect::new(how.x - 10 - CURVE_W, cy, CURVE_W, BUTTON_H);
        let to = Rect::new(curve.x - 10 - VALUE_W, rr.y, VALUE_W, rr.h);
        let name = Rect::new(rr.x + 28, rr.y + 4, NAME_W, rr.h - 8);
        let from = Rect::new(name.right() + 6, rr.y, VALUE_W, rr.h);
        let bar = Rect::new(
            from.right() + 14,
            rr.y,
            (to.x - 14 - from.right() - 14).max(40),
            rr.h,
        );
        let take_off = Rect::new(rr.right() - 6 - 120, cy, 120, BUTTON_H);
        RowParts {
            name,
            from,
            bar,
            to,
            curve,
            set,
            add,
            x,
            take_off,
        }
    }

    /// The curve list under row `row`'s chip: its four choices.
    fn curve_list(chip: Rect, i: usize) -> Rect {
        Rect::new(chip.x, chip.bottom() + 4 + i as i32 * 44, chip.w + 24, 44)
    }

    /// The foot's controls: Go back, Stay, Jumps, Moves from here, –, +,
    /// Off, On.
    fn foot_parts(f: Rect) -> [Rect; 8] {
        let inner = f.inset_xy(14, 10);
        let half = inner.w / 2;
        let y1 = inner.y;
        let y2 = inner.y + BUTTON_H + 8;
        let l = inner.x + 150;
        let r = inner.x + half + 230;
        [
            Rect::new(l, y1, 126, BUTTON_H),
            Rect::new(l + 130, y1, 126, BUTTON_H),
            Rect::new(l, y2, 126, BUTTON_H),
            Rect::new(l + 130, y2, 170, BUTTON_H),
            Rect::new(r, y1, 44, BUTTON_H),
            Rect::new(r + 44 + 100, y1, 44, BUTTON_H),
            Rect::new(r, y2, 72, BUTTON_H),
            Rect::new(r + 76, y2, 72, BUTTON_H),
        ]
    }

    /// A position on the bar for an end's value: Set's travel (0 to 1), or
    /// Add's push (-1 to 1) from the middle.
    fn at(row: &RowView, v: f32) -> f32 {
        if row.add { (v + 1.0) * 0.5 } else { v }.clamp(0.0, 1.0)
    }

    fn value_at(row: &RowView, p: f32) -> f32 {
        let p = p.clamp(0.0, 1.0);
        if row.add {
            p * 2.0 - 1.0
        } else {
            p
        }
    }

    fn track(bar: Rect) -> (i32, i32) {
        (bar.x + 8, bar.right() - 8)
    }

    fn handle_x(bar: Rect, p: f32) -> i32 {
        let (a, b) = XySetup::track(bar);
        a + ((b - a) as f32 * p).round() as i32
    }

    fn colour(t: &Theme, dot: Dot) -> Color {
        match dot {
            Dot::Track(i) => t.track(i),
            Dot::Master => t.accent,
            Dot::Other => t.text_dim,
        }
    }

    fn acted(&mut self, a: SetupAct) -> Option<EventKind> {
        self.acts.push(a);
        Some(EventKind::Select(0))
    }

    /// Glide back `steps` steps longer (shorter, below 0), shown at once:
    /// steps taken before the sound loop answers add up.
    fn glide(&mut self, steps: i32) -> Option<EventKind> {
        let ms = (self.view.glide_ms + steps as f32 * GLIDE_STEP_MS).clamp(0.0, GLIDE_MAX_MS);
        if ms == self.view.glide_ms {
            return None;
        }
        self.view.glide_ms = ms;
        self.acted(SetupAct::Glide(ms))
    }

    /// Q-Link `k` turned `ticks` on Setup: Q1 and Q2 the picked row's
    /// from and to, Q3 its curve, Q4 the glide back (Go back). False when
    /// it has nothing to turn.
    pub fn qlink(&mut self, k: usize, ticks: i32) -> bool {
        let row = self
            .picked
            .filter(|&i| self.view.rows.get(i).is_some_and(|r| r.gone.is_none()));
        let act = match (k, row) {
            (0 | 1, Some(i)) => {
                let r = &mut self.view.rows[i];
                let (lo, step) = if r.add { (-1.0, 0.02) } else { (0.0, 0.01) };
                let d = ticks as f32 * step;
                if k == 0 {
                    r.from = (r.from + d).clamp(lo, 1.0);
                } else {
                    r.to = (r.to + d).clamp(lo, 1.0);
                }
                SetupAct::Range {
                    row: i,
                    from: r.from,
                    to: r.to,
                }
            }
            (2, Some(i)) => {
                let r = &mut self.view.rows[i];
                let n = CURVES.len() as i32;
                let at = CURVES.iter().position(|(c, _)| *c == r.curve).unwrap_or(0) as i32;
                r.curve = CURVES[(at + ticks).rem_euclid(n) as usize].0;
                SetupAct::Curve {
                    row: i,
                    curve: r.curve,
                }
            }
            (3, _) if !self.view.stay => return self.glide(ticks).is_some(),
            _ => return false,
        };
        self.acts.push(act);
        true
    }

    /// The Q-Link cards on Setup: the picked row's From, To and Curve, and
    /// Glide back (faint with none picked, or on Stay).
    pub fn qlink_cards(&self) -> [mpfree_ui::qlink::QLinkCard; 4] {
        use mpfree_ui::qlink::QLinkCard;
        let mut cards: [QLinkCard; 4] = Default::default();
        let row = self
            .picked
            .and_then(|i| self.view.rows.get(i))
            .filter(|r| r.gone.is_none());
        if let Some(r) = row {
            let pos = |v: f32| if r.add { (v + 1.0) / 2.0 } else { v };
            cards[0] = QLinkCard::new("From", &r.name, &r.from_text, pos(r.from));
            cards[1] = QLinkCard::new("To", &r.name, &r.to_text, pos(r.to));
            cards[0].bipolar = r.add;
            cards[1].bipolar = r.add;
            let at = CURVES.iter().position(|(c, _)| *c == r.curve).unwrap_or(0);
            let last = (CURVES.len() - 1) as f32;
            cards[2] = QLinkCard::new("Curve", &r.name, curve_name(r.curve), at as f32 / last);
        }
        if !self.view.stay {
            cards[3] = QLinkCard::new(
                "Glide back",
                &self.view.name,
                &format!("{:.0} ms", self.view.glide_ms),
                self.view.glide_ms / GLIDE_MAX_MS,
            );
        }
        cards
    }

    /// A finger on row `i` at `x` (its bar's handle, if near one).
    fn row_down(&mut self, finger: u64, i: usize, rr: Rect, x: i32, y: i32) -> Option<EventKind> {
        let p = XySetup::row_parts(rr);
        let row = self.view.rows.get(i)?.clone();
        if row.gone.is_some() {
            if p.take_off.contains(x, y) {
                return self.acted(SetupAct::TakeOff(i));
            }
            return None;
        }
        if p.x.contains(x, y) {
            return self.acted(SetupAct::TakeOff(i));
        }
        if p.set.contains(x, y) {
            return (row.add)
                .then(|| self.acted(SetupAct::How { row: i, add: false }))
                .flatten();
        }
        if p.add.contains(x, y) {
            if row.no_add.is_some() {
                return self.acted(SetupAct::NoAdd(i));
            }
            return (!row.add)
                .then(|| self.acted(SetupAct::How { row: i, add: true }))
                .flatten();
        }
        if p.curve.contains(x, y) {
            self.curve_open = if self.curve_open == Some(i) {
                None
            } else {
                Some(i)
            };
            return Some(EventKind::Select(1));
        }
        if p.bar.contains(x, y) {
            let a = XySetup::handle_x(p.bar, XySetup::at(&row, row.from));
            let b = XySetup::handle_x(p.bar, XySetup::at(&row, row.to));
            let end = usize::from((x - b).abs() < (x - a).abs());
            self.drag = Some(Drag::Handle {
                finger,
                row: i,
                end,
            });
            self.picked = Some(i);
            return self.drag_to(i, end, x, p.bar);
        }
        self.drag = Some(Drag::Scroll {
            finger,
            y,
            from: self.scroll,
            moved: false,
        });
        self.picked = Some(i);
        self.acted(SetupAct::Pick(i))
    }

    fn drag_to(&mut self, i: usize, end: usize, x: i32, bar: Rect) -> Option<EventKind> {
        let (a, b) = XySetup::track(bar);
        let p = (x - a) as f32 / (b - a).max(1) as f32;
        let row = self.view.rows.get_mut(i)?;
        let v = XySetup::value_at(row, p);
        if end == 0 {
            row.from = v;
        } else {
            row.to = v;
        }
        let (from, to) = (row.from, row.to);
        self.acted(SetupAct::Range { row: i, from, to })
    }
}

struct RowParts {
    name: Rect,
    from: Rect,
    bar: Rect,
    to: Rect,
    curve: Rect,
    set: Rect,
    add: Rect,
    x: Rect,
    take_off: Rect,
}

/// Two choices side by side, the one on filled.
fn segments(
    cx: &mut PaintCx,
    t: &Theme,
    a: Rect,
    b: Rect,
    names: [&str; 2],
    on: usize,
    usable: [bool; 2],
) {
    let whole = a.union(b).inset_xy(-2, -2);
    cx.canvas.fill_round_rect(whole, t.round(10), t.bg);
    for (i, (r, n)) in [(a, names[0]), (b, names[1])].into_iter().enumerate() {
        let ink = if i == on {
            cx.canvas.fill_round_rect(r, t.round(8), t.picked());
            t.on_picked()
        } else if usable[i] {
            t.text_dim
        } else {
            t.faded(t.text_dim, t.bg)
        };
        cx.text(FontId::Bold, 13.0, r, Align::Center, ink, n);
    }
}

impl Widget for XySetup {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let g = XySetup::geo(r);
        let v = &self.view;
        // The head.
        let label = Rect::new(r.x + 16, g.name.y, 60, g.name.h);
        cx.text(FontId::Bold, 13.0, label, Align::Left, t.text_dim, "SETUP");
        cx.canvas.fill_round_rect(g.name, t.round(8), t.bg);
        cx.text(
            FontId::Bold,
            17.0,
            g.name.inset_xy(12, 0),
            Align::Left,
            t.text,
            &v.name,
        );
        cx.text(
            FontId::Sans,
            11.0,
            g.name.inset_xy(10, 0),
            Align::Right,
            t.text_dim,
            "name",
        );
        cx.canvas.fill_round_rect(g.save, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            15.0,
            g.save,
            Align::Center,
            t.text,
            "Save to browser",
        );
        cx.canvas.fill_round_rect(g.delete, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            15.0,
            g.delete,
            Align::Center,
            t.text,
            "Delete",
        );
        cx.canvas.fill_round_rect(g.done, t.round(10), t.accent);
        cx.text(
            FontId::Bold,
            16.0,
            g.done,
            Align::Center,
            t.on_accent,
            "Done",
        );
        // The settings, clipped to the list.
        let clip = cx.canvas.clip();
        cx.canvas.set_clip(clip.intersect(g.list));
        for (item, y, h) in self.items() {
            let ir = self.item_rect(g.list, y, h);
            if ir.bottom() < g.list.y || ir.y > g.list.bottom() {
                continue;
            }
            match item {
                Item::Dir(up) => {
                    let (name, plus) = XySetup::dir_parts(ir);
                    let head = if up { "UP ↑" } else { "ACROSS →" };
                    cx.text(
                        FontId::Bold,
                        15.0,
                        ir.take_left(92).0,
                        Align::Left,
                        t.text,
                        head,
                    );
                    cx.canvas.fill_round_rect(name, t.round(8), t.bg);
                    let n = if up { &v.up } else { &v.across };
                    cx.text(
                        FontId::Bold,
                        15.0,
                        name.inset_xy(12, 0),
                        Align::Left,
                        t.text,
                        n,
                    );
                    let count = format!("{} of 8", v.count(up));
                    let c = Rect::new(name.right() + 14, ir.y, 80, ir.h);
                    cx.text(FontId::Sans, 13.0, c, Align::Left, t.text_dim, &count);
                    let full = v.count(up) >= 8;
                    cx.canvas.fill_round_rect(plus, t.round(10), t.raised);
                    let what = if up { "+ up" } else { "+ across" };
                    cx.text(
                        FontId::Bold,
                        14.0,
                        plus,
                        Align::Center,
                        t.button_ink(!full),
                        what,
                    );
                }
                Item::Row(i) => self.paint_row(cx, &t, i, ir),
            }
        }
        cx.canvas.set_clip(clip);
        // The foot card.
        cx.canvas.fill_round_rect(g.foot, t.round(12), t.bg);
        let f = XySetup::foot_parts(g.foot);
        let inner = g.foot.inset_xy(14, 10);
        let lbl = |y: i32, x: i32, w: i32| Rect::new(x, y, w, BUTTON_H);
        cx.text(
            FontId::Sans,
            14.0,
            lbl(f[0].y, inner.x, 146),
            Align::Left,
            t.text_dim,
            "When you let go",
        );
        segments(
            cx,
            &t,
            f[0],
            f[1],
            ["Go back", "Stay"],
            usize::from(v.stay),
            [true; 2],
        );
        cx.text(
            FontId::Sans,
            14.0,
            lbl(f[2].y, inner.x, 146),
            Align::Left,
            t.text_dim,
            "A touch",
        );
        segments(
            cx,
            &t,
            f[2],
            f[3],
            ["Jumps", "Moves from here"],
            usize::from(v.from_here),
            [true; 2],
        );
        let rx = inner.x + inner.w / 2;
        cx.text(
            FontId::Sans,
            14.0,
            lbl(f[4].y, rx, 220),
            Align::Left,
            t.text_dim,
            "Glide back",
        );
        for (b, s) in [(f[4], "–"), (f[5], "+")] {
            cx.canvas.fill_round_rect(b, t.round(8), t.raised);
            cx.text(FontId::Bold, 18.0, b, Align::Center, t.text, s);
        }
        let ms = Rect::new(f[4].right(), f[4].y, f[5].x - f[4].right(), BUTTON_H);
        let glide = format!("{:.0} ms", v.glide_ms);
        cx.text(FontId::Bold, 15.0, ms, Align::Center, t.text, &glide);
        cx.text(
            FontId::Sans,
            14.0,
            lbl(f[6].y, rx, 226),
            Align::Left,
            t.text_dim,
            "Follow the selected track",
        );
        segments(
            cx,
            &t,
            f[6],
            f[7],
            ["Off", "On"],
            usize::from(v.follow),
            [true; 2],
        );
        let lines = Rect::new(
            inner.x,
            f[2].bottom() + 4,
            inner.w,
            inner.bottom() - f[2].bottom() - 4,
        );
        let (l1, l2) = lines.take_top(lines.h / 2);
        let faint = t.text_dim;
        cx.text(
            FontId::Sans,
            12.0,
            l1,
            Align::Left,
            faint,
            "Stay: Set settings stay where you lift (one UNDO step; never while recording). Add settings always glide back. Hold keeps everything.",
        );
        cx.text(
            FontId::Sans,
            12.0,
            l2,
            Align::Left,
            faint,
            "Add is greyed on a synth's or a pad's own settings until the mod engine reaches them.",
        );
        // The curve list, over everything.
        if let Some(i) = self.curve_open {
            if let Some(ir) = self.row_rect(g.list, i) {
                let chip = XySetup::row_parts(ir).curve;
                let row_curve = self.view.rows[i].curve;
                for (k, (c, n)) in CURVES.iter().enumerate() {
                    let o = XySetup::curve_list(chip, k);
                    let on = *c == row_curve;
                    let fill = if on { t.picked() } else { t.raised };
                    cx.canvas.fill_round_rect(o, t.round(6), fill);
                    let ink = if on { t.on_picked() } else { t.text };
                    cx.text(FontId::Bold, 14.0, o.inset_xy(12, 0), Align::Left, ink, n);
                }
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = XySetup::geo(r);
        let v = &self.view;
        let mut out = vec![
            Target::tap("name", 0, "Name", g.name),
            Target::tap("save", 0, "Save to browser", g.save),
            Target::tap("delete", 0, "Delete", g.delete),
            Target::tap("done", 0, "Done", g.done),
        ];
        let seen = |rr: Rect| rr.y >= g.list.y && rr.bottom() <= g.list.bottom();
        for (item, y, h) in self.items() {
            let ir = self.item_rect(g.list, y, h);
            if !seen(ir) {
                continue;
            }
            match item {
                Item::Dir(up) => {
                    let (name, plus) = XySetup::dir_parts(ir);
                    let w = if up { "Up" } else { "Across" };
                    out.push(Target::tap(
                        "dir-name",
                        usize::from(up),
                        format!("{w} name"),
                        name,
                    ));
                    out.push(
                        Target::tap(
                            "plus",
                            usize::from(up),
                            format!("+ {}", w.to_lowercase()),
                            plus,
                        )
                        .disabled_if(v.count(up) >= 8),
                    );
                }
                Item::Row(i) => {
                    let p = XySetup::row_parts(ir);
                    let row = &v.rows[i];
                    if row.gone.is_some() {
                        out.push(Target::tap("take-off", i, "Take off", p.take_off));
                        continue;
                    }
                    out.push(Target::tap("row", i, row.name.clone(), p.name));
                    out.push(Target::drag("from", i, "From", p.bar));
                    out.push(Target::tap("curve", i, curve_name(row.curve), p.curve));
                    out.push(Target::tap("set", i, "Set", p.set).disabled_if(!row.add));
                    out.push(Target::tap("add", i, "Add", p.add).disabled_if(row.add));
                    out.push(Target::tap("take-off", i, "×", p.x));
                }
            }
        }
        let f = XySetup::foot_parts(g.foot);
        let names = [
            "Go back",
            "Stay",
            "Jumps",
            "Moves from here",
            "–",
            "+",
            "Off",
            "On",
        ];
        let on = [
            !v.stay,
            v.stay,
            !v.from_here,
            v.from_here,
            false,
            false,
            !v.follow,
            v.follow,
        ];
        for (k, rect) in f.into_iter().enumerate() {
            out.push(Target::tap("foot", k, names[k], rect).disabled_if(on[k]));
        }
        if let Some(i) = self.curve_open {
            if let Some(ir) = self.row_rect(g.list, i) {
                let chip = XySetup::row_parts(ir).curve;
                for (k, (_, n)) in CURVES.iter().enumerate() {
                    out.insert(
                        0,
                        Target::tap("curve-pick", k, *n, XySetup::curve_list(chip, k)),
                    );
                }
            }
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = XySetup::geo(r);
        match phase {
            Touch::Down => {}
            Touch::Move => {
                return match self.drag {
                    Some(Drag::Handle {
                        finger: f,
                        row,
                        end,
                    }) if f == finger => {
                        let ir = self.row_rect_any(g.list, row)?;
                        self.drag_to(row, end, x, XySetup::row_parts(ir).bar)
                    }
                    Some(Drag::Scroll {
                        finger: f,
                        y: y0,
                        from,
                        moved,
                    }) if f == finger => {
                        if moved || (y - y0).abs() > SCROLL_SLOP {
                            let max = self.max_scroll(g.list);
                            self.scroll = (from - (y - y0)).clamp(0, max);
                            self.drag = Some(Drag::Scroll {
                                finger,
                                y: y0,
                                from,
                                moved: true,
                            });
                            self.curve_open = None;
                            return Some(EventKind::Select(1));
                        }
                        None
                    }
                    _ => None,
                };
            }
            Touch::Up | Touch::Cancel => {
                if self.drag.is_some_and(|d| match d {
                    Drag::Handle { finger: f, .. } | Drag::Scroll { finger: f, .. } => f == finger,
                }) {
                    self.drag = None;
                }
                return None;
            }
        }
        // The curve list, open, takes the first touch.
        if let Some(i) = self.curve_open.take() {
            if let Some(ir) = self.row_rect(g.list, i) {
                let chip = XySetup::row_parts(ir).curve;
                if let Some(k) =
                    (0..CURVES.len()).find(|&k| XySetup::curve_list(chip, k).contains(x, y))
                {
                    if let Some(row) = self.view.rows.get_mut(i) {
                        row.curve = CURVES[k].0;
                    }
                    return self.acted(SetupAct::Curve {
                        row: i,
                        curve: CURVES[k].0,
                    });
                }
                if chip.contains(x, y) {
                    return Some(EventKind::Select(1));
                }
            }
        }
        if g.name.contains(x, y) {
            return self.acted(SetupAct::Name);
        }
        if g.save.contains(x, y) {
            return self.acted(SetupAct::Save);
        }
        if g.delete.contains(x, y) {
            return self.acted(SetupAct::Delete);
        }
        if g.done.contains(x, y) {
            return self.acted(SetupAct::Done);
        }
        let f = XySetup::foot_parts(g.foot);
        if let Some(k) = (0..f.len()).find(|&k| f[k].contains(x, y)) {
            let v = &self.view;
            let a = match k {
                0 | 1 if v.stay != (k == 1) => SetupAct::Lift(k == 1),
                2 | 3 if v.from_here != (k == 3) => SetupAct::Start(k == 3),
                4 => return self.glide(-1),
                5 => return self.glide(1),
                6 | 7 if v.follow != (k == 7) => SetupAct::Follow(k == 7),
                _ => return None,
            };
            return self.acted(a);
        }
        if !g.list.contains(x, y) {
            return None;
        }
        for (item, iy, h) in self.items() {
            let ir = self.item_rect(g.list, iy, h);
            if !ir.contains(x, y) {
                continue;
            }
            return match item {
                Item::Dir(up) => {
                    let (name, plus) = XySetup::dir_parts(ir);
                    if name.contains(x, y) {
                        self.acted(SetupAct::Name)
                    } else if plus.contains(x, y) && self.view.count(up) < 8 {
                        self.acted(SetupAct::Plus(up))
                    } else {
                        None
                    }
                }
                Item::Row(i) => self.row_down(finger, i, ir, x, y),
            };
        }
        // Between the rows: a drag scrolls.
        self.drag = Some(Drag::Scroll {
            finger,
            y,
            from: self.scroll,
            moved: false,
        });
        None
    }

    fn steps(&self, t: &Target) -> bool {
        matches!(t.id, "from") || (t.id == "foot" && matches!(t.index, 4 | 5))
    }

    fn step(&mut self, t: &Target, delta: i32, _r: Rect) -> Option<EventKind> {
        match t.id {
            "foot" => self.glide(delta),
            "from" => {
                let row = self.view.rows.get_mut(t.index)?;
                let step = if row.add { 0.02 } else { 0.01 };
                row.to =
                    (row.to + step * delta as f32).clamp(if row.add { -1.0 } else { 0.0 }, 1.0);
                let (from, to) = (row.from, row.to);
                self.acted(SetupAct::Range {
                    row: t.index,
                    from,
                    to,
                })
            }
            _ => None,
        }
    }
}

impl XySetup {
    /// Row `i`'s place on the screen, when it shows whole in the list.
    fn row_rect(&self, list: Rect, i: usize) -> Option<Rect> {
        self.row_rect_any(list, i)
            .filter(|ir| ir.y >= list.y && ir.bottom() <= list.bottom())
    }

    /// Row `i`'s place on the screen, shown or not.
    fn row_rect_any(&self, list: Rect, i: usize) -> Option<Rect> {
        let (_, y, h) = self
            .items()
            .into_iter()
            .find(|(item, ..)| *item == Item::Row(i))?;
        Some(self.item_rect(list, y, h))
    }

    fn paint_row(&self, cx: &mut PaintCx, t: &Theme, i: usize, ir: Rect) {
        let row = &self.view.rows[i];
        let p = XySetup::row_parts(ir);
        let picked = self.picked == Some(i);
        cx.canvas.fill_round_rect(ir, t.round(10), t.raised);
        if picked {
            cx.canvas.stroke_round_rect(ir, t.round(10), 2, t.accent);
        }
        let colour = XySetup::colour(t, row.dot);
        let (top, bottom) = p.name.take_top(p.name.h / 2 + 2);
        if let Some(why) = &row.gone {
            mpfree_ui::draw::dot(cx, ir.x + 14, ir.y + ir.h / 2, 5, t.text_faint);
            let w = cx.text(FontId::Bold, 15.0, top, Align::Left, t.text_dim, &row.name);
            let mid = top.y + top.h / 2 + 1;
            cx.canvas
                .fill_rect(Rect::new(top.x, mid, w.max(8), 2), t.text_dim);
            cx.text(
                FontId::Sans,
                12.0,
                bottom,
                Align::Left,
                t.text_dim,
                &row.place,
            );
            let words = Rect::new(
                p.from.x + 20,
                ir.y + 4,
                p.take_off.x - p.from.x - 30,
                ir.h - 8,
            );
            let (w1, w2) = words.take_top(words.h / 2);
            cx.text(FontId::Sans, 13.0, w1, Align::Left, t.warn, why);
            cx.text(
                FontId::Sans,
                12.0,
                w2,
                Align::Left,
                t.text_dim,
                "It does nothing until you take it off or pick another.",
            );
            cx.canvas.fill_round_rect(p.take_off, t.round(8), t.bg);
            cx.text(
                FontId::Bold,
                14.0,
                p.take_off,
                Align::Center,
                t.text,
                "Take off",
            );
            return;
        }
        mpfree_ui::draw::dot(cx, ir.x + 14, ir.y + ir.h / 2, 5, colour);
        cx.text(FontId::Bold, 15.0, top, Align::Left, t.text, &row.name);
        cx.text(
            FontId::Sans,
            12.0,
            bottom,
            Align::Left,
            t.text_dim,
            &row.place,
        );
        cx.text(
            FontId::Sans,
            13.0,
            p.from,
            Align::Right,
            t.text,
            &row.from_text,
        );
        cx.text(FontId::Sans, 13.0, p.to, Align::Left, t.text, &row.to_text);
        // The range bar.
        let (a, b) = XySetup::track(p.bar);
        let cy = p.bar.y + p.bar.h / 2 - if row.backwards() { 4 } else { 0 };
        cx.canvas
            .fill_round_rect(Rect::new(a, cy - 2, b - a, 4), 2, t.bg);
        let hf = XySetup::handle_x(p.bar, XySetup::at(row, row.from));
        let ht = XySetup::handle_x(p.bar, XySetup::at(row, row.to));
        let (lo, hi) = (hf.min(ht), hf.max(ht));
        let line = t.swatch(colour).line;
        cx.canvas
            .fill_round_rect(Rect::new(lo, cy - 2, (hi - lo).max(1), 4), 2, line);
        if row.add {
            let mid = (a + b) / 2;
            cx.canvas
                .fill_rect(Rect::new(mid, cy - 7, 1, 14), t.text_dim);
        }
        for h in [hf, ht] {
            mpfree_ui::draw::dot(cx, h, cy, 7, t.text);
            mpfree_ui::draw::dot(cx, h, cy, 5, line);
        }
        if row.backwards() {
            let under = Rect::new(p.bar.x, cy + 6, p.bar.w, 14);
            cx.text(
                FontId::Sans,
                11.0,
                under,
                Align::Center,
                t.text_dim,
                "runs backwards",
            );
        }
        // The curve chip.
        cx.canvas.fill_round_rect(p.curve, t.round(8), t.bg);
        cx.text(
            FontId::Bold,
            13.0,
            p.curve.inset_xy(12, 0),
            Align::Left,
            t.text,
            curve_name(row.curve),
        );
        cx.text(
            FontId::Sans,
            12.0,
            p.curve.inset_xy(10, 0),
            Align::Right,
            t.text_dim,
            "▾",
        );
        segments(
            cx,
            t,
            p.set,
            p.add,
            ["Set", "Add"],
            usize::from(row.add),
            [true, row.no_add.is_none()],
        );
        cx.text(FontId::Bold, 18.0, p.x, Align::Center, t.text_dim, "×");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Ui};

    fn row(up: bool, k: usize, name: &str) -> RowView {
        RowView {
            up,
            k,
            name: name.into(),
            place: "Bass · Poly".into(),
            dot: Dot::Track(1),
            from: 0.2,
            to: 0.85,
            from_text: "20 %".into(),
            to_text: "85 %".into(),
            ..RowView::default()
        }
    }

    fn view() -> MineView {
        let mut rows = vec![
            row(false, 0, "Cutoff"),
            row(false, 1, "Drive"),
            row(true, 0, "Send A"),
        ];
        rows[1].add = true;
        rows[1].from = 0.0;
        rows[1].to = 0.5;
        rows[2].no_add = Some("why");
        rows.push(RowView {
            gone: Some("Track 7 was removed.".into()),
            ..row(true, 1, "Mix")
        });
        MineView {
            name: "Squelch".into(),
            across: "Bite".into(),
            up: "Wet".into(),
            rows,
            glide_ms: 120.0,
            ..MineView::default()
        }
    }

    fn ui() -> Ui {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut s = XySetup::default();
        s.set_view(view());
        ui.add(XY_SETUP, s);
        ui.add_full_page("p", &format!("col({XY_SETUP}:*)"))
            .unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        ui
    }

    fn tap(ui: &mut Ui, at: (&str, usize)) -> Vec<SetupAct> {
        let r = target(ui, at.0, at.1);
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
        ui.get_mut::<XySetup>(XY_SETUP).unwrap().take()
    }

    fn target(ui: &Ui, id: &str, index: usize) -> Rect {
        let r = ui.rect_of(XY_SETUP).unwrap();
        let s = ui.get::<XySetup>(XY_SETUP).unwrap();
        s.targets(r)
            .into_iter()
            .find(|t| t.id == id && t.index == index)
            .unwrap_or_else(|| panic!("no {id} {index}"))
            .rect
    }

    /// Every control is a fingertip; Set | Add, ×, Take off, the foot and
    /// the head say what they do.
    #[test]
    fn setup_offers_each_setting_its_range_curve_and_how() {
        let mut ui = ui();
        let r = ui.rect_of(XY_SETUP).unwrap();
        for t in ui.get::<XySetup>(XY_SETUP).unwrap().targets(r) {
            assert!(
                t.rect.h >= 36 && t.rect.w >= 36,
                "{} {}: {:?}",
                t.id,
                t.index,
                t.rect
            );
        }
        assert_eq!(
            tap(&mut ui, ("add", 0)),
            [SetupAct::How { row: 0, add: true }]
        );
        assert_eq!(
            tap(&mut ui, ("set", 1)),
            [SetupAct::How { row: 1, add: false }]
        );
        assert_eq!(tap(&mut ui, ("add", 2)), [SetupAct::NoAdd(2)]);
        assert_eq!(tap(&mut ui, ("take-off", 3)), [SetupAct::TakeOff(3)]);
        assert_eq!(tap(&mut ui, ("take-off", 0)), [SetupAct::TakeOff(0)]);
        assert_eq!(tap(&mut ui, ("foot", 1)), [SetupAct::Lift(true)]);
        let glide = ui.get::<XySetup>(XY_SETUP).unwrap().view.glide_ms;
        assert_eq!(
            tap(&mut ui, ("foot", 5)),
            [SetupAct::Glide(glide + GLIDE_STEP_MS)]
        );
        assert_eq!(tap(&mut ui, ("foot", 7)), [SetupAct::Follow(true)]);
        assert_eq!(tap(&mut ui, ("plus", 1)), [SetupAct::Plus(true)]);
        assert_eq!(tap(&mut ui, ("done", 0)), [SetupAct::Done]);
        assert_eq!(tap(&mut ui, ("name", 0)), [SetupAct::Name]);
        assert_eq!(tap(&mut ui, ("save", 0)), [SetupAct::Save]);
        let g = XySetup::geo(r);
        assert!(g.name.right() + 10 <= g.save.x, "{:?} {:?}", g.name, g.save);
        // The curve list opens on the chip and picks.
        assert!(tap(&mut ui, ("curve", 0)).is_empty());
        assert_eq!(
            tap(&mut ui, ("curve-pick", 2)),
            [SetupAct::Curve {
                row: 0,
                curve: MapCurve::Log
            }]
        );
    }

    /// A handle dragged sends the row's new ends as it goes, and shows
    /// them at once.
    #[test]
    fn a_handle_dragged_sets_the_range() {
        let mut ui = ui();
        let bar = target(&ui, "from", 0);
        let (a, b) = XySetup::track(bar);
        let y = bar.y + bar.h / 2;
        let to_x = XySetup::handle_x(bar, 0.85);
        ui.touch(1, Touch::Down, to_x, y);
        ui.touch(1, Touch::Move, b, y);
        ui.touch(1, Touch::Move, a, y);
        ui.touch(1, Touch::Up, a, y);
        let acts = ui.get_mut::<XySetup>(XY_SETUP).unwrap().take();
        assert_eq!(
            acts.last(),
            Some(&SetupAct::Range {
                row: 0,
                from: 0.2,
                to: 0.0
            })
        );
        let s = ui.get::<XySetup>(XY_SETUP).unwrap();
        assert!(s.view.rows[0].backwards());
        assert_eq!(s.picked, Some(0));
        // An Add row's bar runs from -1 to 1, 0 in the middle.
        let add = target(&ui, "from", 1);
        let (a, b) = XySetup::track(add);
        ui.touch(2, Touch::Down, b, add.y + add.h / 2);
        ui.touch(2, Touch::Move, (a + b) / 2, add.y + add.h / 2);
        ui.touch(2, Touch::Up, (a + b) / 2, add.y + add.h / 2);
        let acts = ui.get_mut::<XySetup>(XY_SETUP).unwrap().take();
        let Some(SetupAct::Range { from, to, .. }) = acts.last() else {
            panic!("{acts:?}");
        };
        assert_eq!(*from, 0.0);
        assert!(to.abs() < 0.02, "{to}");
    }
}
