//! Make a style (composing step 6, board D): the sheet that makes a style
//! from a clip, opened by the clip card's Style row in the card's place on
//! Arrange, as Make a phrase's sheet opens. It shows what was read from
//! the clip (its feel, how busy it is, where its accents fall, its
//! loudness and how far it leans), a picture of its notes, the style's
//! Name and its Plan (the built-in style lending the phrase plan and the
//! chords' habits, the nearest first), and Back and Keep. Keep puts the
//! style in every Style row (Continue, Add a part, New idea, Continue on
//! Steps), after the built-in ones, kept with the song.
//!
//! The time engine holds the sheet ([`StyleSheet`], in
//! `TimeState::style_sheet`) and acts on [`StyleAction`]s; the widget
//! ([`StyleSheetView`]) shows it and reports touches: the Plan stepper's
//! halves give `Step { index: PLAN_ROW, delta }`, the Name button
//! `Toggle(STYLE_NAME)` (the app opens its naming keyboard), × and Back
//! `Toggle(STYLE_CLOSE)` / `Toggle(STYLE_BACK)`, Keep `Toggle(STYLE_KEEP)`.

use mpfree_compose::{ClipStyle, STYLES};
use mpfree_transport::time::Tick;
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, EventKind, PaintCx, Rect, Target, Touch, Widget};

/// The sheet's widget name.
pub const STYLE_SHEET: &str = "make-style";

/// × in the corner: the same as Back.
pub const STYLE_CLOSE: usize = 0;
pub const STYLE_BACK: usize = 1;
pub const STYLE_KEEP: usize = 2;
/// The Name button: the app opens its naming keyboard on the name.
pub const STYLE_NAME: usize = 3;

/// The sheet's rows: Name, then Plan.
pub const NAME_ROW: usize = 0;
pub const PLAN_ROW: usize = 1;
/// The longest name the naming keyboard takes for a style.
pub const MAX_STYLE_NAME: usize = 24;

/// Make a style, open on a clip: what was read, and the style Keep keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleSheet {
    pub track: usize,
    pub slot: usize,
    /// The style as Keep would keep it: its name, its plan and what was
    /// read.
    pub made: ClipStyle,
    /// "From Clip 1 · Track 1 · 2 bars".
    pub from: String,
    /// Each note's start (in the clip's ticks) and how hard it is played,
    /// for the picture.
    pub hits: Vec<(Tick, u8)>,
    pub length: Tick,
    /// A bar, in the clip's ticks.
    pub bar: Tick,
    /// The row the wheel is on: Name or Plan.
    pub row: usize,
}

impl StyleSheet {
    /// Plan's place among the built-in styles.
    pub fn plan_index(&self) -> usize {
        STYLES
            .iter()
            .position(|s| s.name == self.made.plan)
            .unwrap_or(0)
    }

    /// Plan stepped `delta` among the built-in styles: true when it
    /// changed.
    pub fn step_plan(&mut self, delta: i32) -> bool {
        let at = self.plan_index();
        let to = (at as i64 + i64::from(delta)).clamp(0, STYLES.len() as i64 - 1) as usize;
        self.made.plan = STYLES[to].name;
        to != at
    }

    /// The lines under the rows.
    pub fn lines(&self) -> [&'static str; 1] {
        ["Feel, busyness, accents and loudness come from the clip; the phrase plan and chords from Plan."]
    }

    /// What Keep does: "Keep puts Clip 1 groove in every Style row:
    /// Continue, Add a part, New idea."
    pub fn keep_line(&self) -> String {
        format!(
            "Keep puts {} in every Style row: Continue, Add a part, New idea.",
            self.made.name
        )
    }

    /// The hint line while the sheet is open.
    pub fn hint(&self) -> &'static str {
        "Read from the clip: change Name and Plan before Keep   ·   Keep: in every Style row   ·   Back: nothing changes"
    }
}

/// What a touch on the sheet, the naming keyboard's Enter, or the wheel
/// and - / + while it is open ask of the time engine.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleAction {
    /// Row `row`'s ‹ (-1) or › (+1): Plan steps; on Name, why not.
    Step { row: usize, delta: i32 },
    /// The wheel's row (Name or Plan).
    Pick(usize),
    /// The style's new name, from the naming keyboard.
    Name(String),
    /// Back, or ×: the sheet closes and nothing is kept.
    Back,
    /// The style joins every Style row, kept with the song.
    Keep,
}

impl StyleAction {
    /// What a touch on the sheet's widget asks for; the Name button is the
    /// app's (its naming keyboard), so None.
    pub fn of(kind: &EventKind) -> Option<StyleAction> {
        match *kind {
            EventKind::Step { index, delta } => Some(StyleAction::Step { row: index, delta }),
            EventKind::Toggle(STYLE_CLOSE | STYLE_BACK) => Some(StyleAction::Back),
            EventKind::Toggle(STYLE_KEEP) => Some(StyleAction::Keep),
            _ => None,
        }
    }
}

/// The sheet's widget ([`STYLE_SHEET`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleSheetView {
    pub sheet: Option<StyleSheet>,
}

const PAD: i32 = 14;
const STEP_W: i32 = 140;
const STEP_H: i32 = 40;
const ROW_GAP: i32 = 10;
const LINE_H: i32 = 18;
const WORD_H: i32 = 22;
const BUTTON_H: i32 = 46;
const PICTURE_H: i32 = 100;
/// Where the words' values start, from the sheet's left edge.
const VALUE_X: i32 = 90;

/// Where the sheet's parts are.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleGeo {
    pub close: Rect,
    pub from: Rect,
    pub picture: Rect,
    /// Where the words read from the clip start.
    pub words_y: i32,
    /// Name's button and Plan's stepper.
    pub rows: [Rect; 2],
    /// Where the lines under the rows start.
    pub lines_y: i32,
    pub keep_line: Rect,
    /// Back, Keep.
    pub buttons: [Rect; 2],
}

impl StyleSheetView {
    pub fn geo(&self, r: Rect) -> StyleGeo {
        let x = r.x + PAD;
        let w = (r.w - 2 * PAD).max(1);
        let picture = Rect::new(x, r.y + 72, w, PICTURE_H);
        let words = self.sheet.as_ref().map_or(5, |s| s.made.read.words().len()) as i32;
        let words_y = picture.bottom() + 10;
        let step_w = STEP_W.min(w);
        let mut y = words_y + words * WORD_H + 12;
        let rows = [0, 1].map(|_| {
            let s = Rect::new(x + w - step_w, y, step_w, STEP_H);
            y += STEP_H + ROW_GAP;
            s
        });
        let half = (w - 10) / 2;
        let b = r.bottom() - PAD - BUTTON_H;
        StyleGeo {
            close: Rect::new(r.right() - PAD - 40, r.y + 8, 40, 34),
            from: Rect::new(x, r.y + 42, w, 22),
            picture,
            words_y,
            rows,
            lines_y: y + 2,
            keep_line: Rect::new(x, b - 10 - 3 * LINE_H, w, 3 * LINE_H),
            buttons: [
                Rect::new(x, b, half, BUTTON_H),
                Rect::new(x + half + 10, b, w - half - 10, BUTTON_H),
            ],
        }
    }
}

fn button(cx: &mut PaintCx, b: Rect, label: &str, lit: bool) {
    let t = cx.theme.clone();
    let fill = if lit { t.accent } else { t.raised };
    cx.canvas.fill_round_rect(b, t.round(10), fill);
    let ink = if lit { t.ink_on(t.accent) } else { t.text };
    cx.text(
        FontId::Bold,
        t.size_body - 1.0,
        b,
        Align::Center,
        ink,
        label,
    );
}

/// The clip's notes as bars: one a note, as tall as it is loud, those
/// well under the clip's average fainter; a faint line and a number at
/// each bar.
fn paint_hits(cx: &mut PaintCx, r: Rect, s: &StyleSheet) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, t.round(10), t.bg);
    let inner = r.inset_xy(8, 8);
    let length = s.length.max(1) as f64;
    let x_of =
        |at: Tick| inner.x + ((at.clamp(0, s.length) as f64 / length) * f64::from(inner.w)) as i32;
    let bar = s.bar.max(1);
    let mut b = 0;
    while b * bar < s.length {
        let x = x_of(b * bar);
        cx.canvas
            .fill_rect(Rect::new(x, inner.y, 1, inner.h), t.outline);
        cx.text(
            FontId::Sans,
            t.size_small - 3.0,
            Rect::new(x + 3, r.y + 2, 30, 14),
            Align::Left,
            t.text_dim,
            &format!("{}", b + 1),
        );
        b += 1;
    }
    let average = s.made.read.velocity;
    let w = (inner.w / (s.hits.len() as i32 + 1)).clamp(2, 6);
    let top = inner.y + 12;
    let tall = (inner.bottom() - top).max(1);
    for &(at, v) in &s.hits {
        let h = (i32::from(v) * tall / 127).max(3);
        let ink = if i32::from(v) + 25 < i32::from(average) {
            t.text_faint
        } else if v > average.saturating_add(5) {
            t.accent
        } else {
            t.readable(t.accent, t.bg).mix(t.bg, 0.35)
        };
        cx.canvas
            .fill_rect(Rect::new(x_of(at), inner.bottom() - h, w, h), ink);
    }
}

/// `text` in lines no wider than `w` at `px`, broken between words.
fn wrap(cx: &mut PaintCx, text: &str, px: f32, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if cx.measure(FontId::Sans, px, &format!("{l} {word}")) <= w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

impl Widget for StyleSheetView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let Some(s) = &self.sheet else {
            return;
        };
        let g = self.geo(r);
        let x = r.x + PAD;
        let w = r.w - 2 * PAD;
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(x, r.y + 8, g.close.x - x - 8, 34),
            Align::Left,
            t.text,
            "MAKE A STYLE",
        );
        cx.canvas.fill_round_rect(g.close, t.round(10), t.raised);
        cx.text(
            FontId::Sans,
            t.size_body + 2.0,
            g.close,
            Align::Center,
            t.text,
            "×",
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            g.from,
            Align::Left,
            t.text_dim,
            &s.from,
        );
        paint_hits(cx, g.picture, s);
        let mut y = g.words_y;
        for (name, value) in s.made.read.words() {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(x, y, VALUE_X - 6, WORD_H),
                Align::Left,
                t.text_dim,
                name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(x + VALUE_X, y, w - VALUE_X, WORD_H),
                Align::Left,
                t.text,
                &value,
            );
            y += WORD_H;
        }
        let plan = STYLES[s.plan_index()].name;
        for (k, (name, value, b)) in [
            ("Name", s.made.name.as_str(), g.rows[0]),
            ("Plan", plan, g.rows[1]),
        ]
        .into_iter()
        .enumerate()
        {
            let label = Rect::new(x + 2, b.y, b.x - x - 6, b.h);
            if k == s.row {
                cx.canvas
                    .fill_round_rect(Rect::new(x - 6, b.y + 8, 3, b.h - 16), 2, t.accent);
            }
            cx.text(
                if k == s.row {
                    FontId::Bold
                } else {
                    FontId::Sans
                },
                t.size_body,
                label,
                Align::Left,
                t.text,
                name,
            );
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            if k == PLAN_ROW {
                let ends = b.inset_xy(12, 0);
                for (side, mark) in [(Align::Left, "‹"), (Align::Right, "›")] {
                    cx.text(FontId::Bold, t.size_body, ends, side, t.text_dim, mark);
                }
            }
            let ink = if k == NAME_ROW {
                t.readable(t.accent, t.raised)
            } else {
                t.text
            };
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                b.inset_xy(if k == PLAN_ROW { 24 } else { 8 }, 0),
                Align::Center,
                ink,
                value,
            );
        }
        let mut y = g.lines_y + 4;
        for line in wrap(cx, s.lines()[0], t.size_small - 2.0, w) {
            if y + LINE_H > g.keep_line.y - 4 {
                break;
            }
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(x, y, w, LINE_H),
                Align::Left,
                t.text_dim,
                &line,
            );
            y += LINE_H;
        }
        let mut y = g.keep_line.y;
        for line in wrap(cx, &s.keep_line(), t.size_small - 2.0, w) {
            if y + LINE_H > g.keep_line.bottom() {
                break;
            }
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                Rect::new(x, y, w, LINE_H),
                Align::Left,
                t.text_dim,
                &line,
            );
            y += LINE_H;
        }
        for (k, (b, label)) in g.buttons.iter().zip(["Back", "Keep"]).enumerate() {
            button(cx, *b, label, k == 1);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if self.sheet.is_none() {
            return Vec::new();
        }
        let g = self.geo(r);
        let (less, more) = g.rows[PLAN_ROW].take_left(g.rows[PLAN_ROW].w / 2);
        vec![
            Target::tap("style-close", STYLE_CLOSE, "Close", g.close),
            Target::tap("style-name", STYLE_NAME, "Name", g.rows[NAME_ROW]),
            Target::tap("style-plan-less", PLAN_ROW, "Plan ‹", less),
            Target::tap("style-plan-more", PLAN_ROW, "Plan ›", more),
            Target::tap("style-back", STYLE_BACK, "Back", g.buttons[0]),
            Target::tap("style-keep", STYLE_KEEP, "Keep", g.buttons[1]),
        ]
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down || self.sheet.is_none() {
            return None;
        }
        let g = self.geo(r);
        if g.close.contains(x, y) {
            return Some(EventKind::Toggle(STYLE_CLOSE));
        }
        if g.rows[NAME_ROW].contains(x, y) {
            return Some(EventKind::Toggle(STYLE_NAME));
        }
        let plan = g.rows[PLAN_ROW];
        if plan.contains(x, y) {
            let delta = if x < plan.x + plan.w / 2 { -1 } else { 1 };
            return Some(EventKind::Step {
                index: PLAN_ROW,
                delta,
            });
        }
        let index = [STYLE_BACK, STYLE_KEEP];
        g.buttons
            .iter()
            .position(|b| b.contains(x, y))
            .map(|k| EventKind::Toggle(index[k]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_compose::{read_clip, realize::PlainNote};
    use mpfree_transport::time::{Meter, Swing, PPQ};

    /// Two bars of 1/16s, the downbeats loud.
    pub(crate) fn sheet() -> StyleSheet {
        let bar = 4 * Tick::from(PPQ);
        let notes: Vec<PlainNote> = (0..32)
            .map(|k| PlainNote {
                at: Tick::from(k) * bar / 16,
                len: bar / 32,
                pitch: 36,
                velocity: if k % 4 == 0 { 112 } else { 70 },
            })
            .collect();
        let read = read_clip(&notes, 2 * bar, Meter::default(), 120.0, Swing::default()).unwrap();
        StyleSheet {
            track: 0,
            slot: 0,
            made: ClipStyle::new("Clip 1", read, Meter::default()),
            from: "From Clip 1 · Track 1 · 2 bars".into(),
            hits: notes.iter().map(|n| (n.at, n.velocity)).collect(),
            length: 2 * bar,
            bar,
            row: NAME_ROW,
        }
    }

    fn at(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    /// In the clip card's column: the words over the rows, the rows over
    /// the lines, Keep's line over the buttons; Name asks for the
    /// keyboard, Plan's halves step it, × and Back close, Keep keeps.
    #[test]
    fn the_rows_and_buttons_answer_where_they_are() {
        let r = Rect::new(878, 140, 250, 608);
        let mut v = StyleSheetView {
            sheet: Some(sheet()),
        };
        let g = v.geo(r);
        assert!(g.words_y + 5 * WORD_H <= g.rows[0].y);
        assert!(g.lines_y < g.keep_line.y);
        assert!(g.keep_line.bottom() <= g.buttons[0].y);
        assert!(g.buttons[1].right() <= r.right() - PAD);
        let mut tap = |(x, y): (i32, i32)| v.touch(0, Touch::Down, x, y, r);
        assert_eq!(
            tap(at(g.rows[NAME_ROW])),
            Some(EventKind::Toggle(STYLE_NAME))
        );
        let plan = g.rows[PLAN_ROW];
        assert_eq!(
            tap((plan.x + 6, plan.y + 10)),
            Some(EventKind::Step {
                index: PLAN_ROW,
                delta: -1
            })
        );
        assert_eq!(
            tap((plan.right() - 6, plan.y + 10)),
            Some(EventKind::Step {
                index: PLAN_ROW,
                delta: 1
            })
        );
        assert_eq!(tap(at(g.close)), Some(EventKind::Toggle(STYLE_CLOSE)));
        assert_eq!(tap(at(g.buttons[0])), Some(EventKind::Toggle(STYLE_BACK)));
        assert_eq!(tap(at(g.buttons[1])), Some(EventKind::Toggle(STYLE_KEEP)));
        assert_eq!(
            StyleAction::of(&EventKind::Toggle(STYLE_CLOSE)),
            Some(StyleAction::Back)
        );
        assert_eq!(StyleAction::of(&EventKind::Toggle(STYLE_NAME)), None);
        // Closed: nothing to touch.
        let mut none = StyleSheetView::default();
        assert!(none.targets(r).is_empty());
        assert_eq!(none.touch(0, Touch::Down, 900, 700, r), None);
    }

    /// Plan steps among the built-in styles and stops at their ends; the
    /// name starts as the clip's.
    #[test]
    fn plan_steps_and_stops_at_its_ends() {
        let mut s = sheet();
        assert_eq!(s.made.name, "Clip 1 groove");
        assert!(s
            .keep_line()
            .starts_with("Keep puts Clip 1 groove in every Style row"));
        s.made.plan = STYLES[0].name;
        assert!(!s.step_plan(-1));
        assert!(s.step_plan(1));
        assert_eq!(s.made.plan, STYLES[1].name);
        assert!(s.step_plan(9));
        assert!(!s.step_plan(1));
        assert_eq!(s.plan_index(), STYLES.len() - 1);
    }
}
