//! The one Sample page's recorder row (one-page step 2a, Jay's mockup of
//! 18 Nov): **REC**, the **Source** with its meter, how a take **Starts**
//! and how it **Ends**, and **Keep last 4 bars** (step 2c), along the Wave
//! page's top. It sets and records with the Rec page's own settings
//! ([`RecSettings`]), so the two agree while both exist; the app holds
//! them and makes the take as the Rec page's Record does.
//!
//! - **REC** starts a take; while one records (or waits for its start)
//!   it is lit and a press stops it, keeping what came in. Its second
//!   line says where the next take goes, how long the one recording has
//!   run, what it waits for, or why REC can't start one.
//! - **Source**, **Starts** and **Ends** step with a tap on their left
//!   half (back) or right half (on): Source among the line input and the
//!   points in the mix, Starts among Now, On input (the threshold) and
//!   Next bar, Ends among On STOP, On silence and a length in bars.
//! - While REC is on, the panel under the wave is **Input** instead of
//!   the region's settings ([`INPUT_ROWS`]): the source, its channels,
//!   where the take goes when it stops (Onto: Stay here, the default,
//!   keeps it on the page; one-page step 2b), the threshold and the
//!   pre-roll.
//! - **Keep last 4 bars** (step 2c, the plan's decision 4) is Sample
//!   Recall in one tap: what just played (the master, as Recall keeps
//!   it) becomes a new take, where Onto says, as REC's would. A tap keeps;
//!   a hold opens Sample Recall over the page, to pick another length.
//!   It says what a tap keeps now ([`KeepView`]): 4 bars while the music
//!   plays (fewer early on), 10 s while it is stopped.

use std::cell::RefCell;

use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use mpfree_ui::Shape;

use crate::panel::{Control, Item, Look, Panel, Section};
use crate::rec_tab::{self, EndAt, RecSettings, Row, StartAt};
use crate::rec_widgets::{db, FLOOR_DB, REC_COLOR};
use crate::recall::{Length, OPEN_BARS};
use crate::widgets::Setting;

/// The widget's name on the Wave page.
pub const RECORDER: &str = "smp-recorder";

/// The cards that step, in their order on the row (the
/// [`EventKind::Step`] index).
pub const STEPS: [Row; 3] = [Row::Source, Row::Start, Row::End];

/// The rows of the Input section the panel shows while REC is on, in its
/// order (the [`EventKind::Step`] index on the panel).
pub const INPUT_ROWS: [Row; 5] = [
    Row::Source,
    Row::Input,
    Row::Threshold,
    Row::PreRoll,
    Row::Onto,
];

/// What the Q-Links turn while REC is on (the plan's step 2a): Q-Link 2
/// the Threshold, Q-Link 3 the Pre-roll. Q-Links 1 and 4 are faint until
/// the recorder has a Gain and a Monitor level (step 2c).
pub const QLINK_ROWS: [Option<Row>; 4] = [None, Some(Row::Threshold), Some(Row::PreRoll), None];

/// Why Q-Links 1 and 4 are faint while REC is on.
pub const QLINK_IDLE: &str = "While REC is on, Q-Links 2 and 3 turn Threshold and Pre-roll";

/// What the Keep card says: what a tap keeps now, and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct KeepView {
    /// "Keep last 4 bars", "Keep last 10 s".
    pub title: String,
    /// "what just played, as a new take", "what just played, onto A6",
    /// or why a tap keeps nothing now ("after this take").
    pub line: String,
    /// A tap keeps it now (else the card is faint and says why).
    pub ready: bool,
}

impl Default for KeepView {
    fn default() -> Self {
        KeepView {
            title: Length::Bars(OPEN_BARS).keep_text(),
            line: KEEP_NOT_YET.to_string(),
            ready: false,
        }
    }
}

/// Why the Keep card keeps nothing before the master is kept.
pub const KEEP_NOT_YET: &str = "the master isn't kept yet";

/// What the row shows: the settings, the sources and the take.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecorderView {
    pub settings: RecSettings,
    /// The sources by name; the first is the line input.
    pub sources: Vec<String>,
    /// A take records, or waits for its start: REC is lit and stops it.
    pub recording: bool,
    /// The take waits for its start (the input over the threshold, or
    /// the next bar line).
    pub waiting: bool,
    /// Why REC can't start a take now (the line input off, no empty pad
    /// left), said under REC; None when it can.
    pub cannot: Option<String>,
    /// REC's second line: where the next take goes ("next take stays
    /// here"), how long the one recording has run ("0:05.1 · onto A6"),
    /// or what it waits for ("for -30 dB · stays here").
    pub status: String,
    /// The Keep card.
    pub keep: KeepView,
}

/// What the row asks for. The app does it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderAction {
    /// Start a take with the settings.
    Record,
    /// Stop the take, keeping it (a take still waiting is cancelled).
    Stop,
    /// Step setting `row` by `delta`.
    Step(Row, i32),
    /// A finger came down on the Keep card: the app keeps the last bars
    /// when it lifts, or opens Sample Recall when it stays.
    Keep,
}

/// What the Starts card says.
pub fn starts_text(s: &RecSettings) -> String {
    match s.start {
        StartAt::Now => "Now".to_string(),
        StartAt::Threshold => format!("On input · {:.0} dB", s.threshold_db),
        StartAt::NextBar => "Next bar".to_string(),
    }
}

/// What the Ends card says.
pub fn ends_text(s: &RecSettings) -> String {
    match s.end {
        EndAt::Press => "On STOP".to_string(),
        EndAt::Silence => "On silence".to_string(),
        EndAt::Length => format!("After {}", rec_tab::Skip::Bars(s.bars).label()),
    }
}

/// The Input section's rows, controls and panel, from `v`'s settings.
pub fn input_list(v: &RecorderView) -> (Vec<Setting>, Vec<Control>, Panel) {
    let s = &v.settings;
    let rows = INPUT_ROWS
        .iter()
        .map(|r| Setting {
            name: r.label().to_string(),
            value: rec_tab::value_text(s, *r, &v.sources),
        })
        .collect();
    let busy = "Stop the take first";
    let controls = INPUT_ROWS
        .iter()
        .map(|r| match r {
            Row::Threshold | Row::PreRoll => rec_tab::knob(s, *r),
            // A take being made keeps its source.
            Row::Source => Control::new(Look::Chip).faint_because(v.recording, busy),
            _ => Control::new(Look::Chip),
        })
        .collect();
    let it = |k: usize| Item::new(k);
    let panel = Panel {
        sections: vec![Section::new(
            "Input",
            1,
            vec![
                vec![it(0).labelled(), it(1).labelled(), it(4).labelled()],
                vec![it(2), it(3)],
            ],
        )
        .note("while REC is on")],
    };
    (rows, controls, panel)
}

const GAP: i32 = 10;
/// REC, with its second line.
const REC_W: i32 = 210;
/// How wide REC's second line is (12.5 px Inter): a longer line is cut.
pub const REC_LINE_W: i32 = REC_W - 28;
const CARD_W: [i32; 3] = [250, 196, 166];
const METER_W: i32 = 44;
/// The Keep card's icon and its padding.
const KEEP_ICON: i32 = 20;
const KEEP_PAD: i32 = 14;

/// The recorder row.
pub struct Recorder {
    pub view: RecorderView,
    /// The meter's level on each side (0 to 1 from [`FLOOR_DB`] to 0
    /// dBFS), held and let fall as [`Recorder::meter`] feeds it.
    level: [f32; 2],
    /// What was last drawn, so a meter that moved redraws only the meter.
    painted: RefCell<Option<(Rect, RecorderView, [f32; 2])>>,
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder::new()
    }
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder {
            view: RecorderView::default(),
            level: [0.0; 2],
            painted: RefCell::new(None),
        }
    }

    /// The source's peak on each side over the last `dt` seconds
    /// (linear): the meter rises to it at once and falls 30 dB a second.
    /// Whether the meter moved.
    pub fn meter(&mut self, peak: [f32; 2], dt: f32) -> bool {
        let before = self.level;
        for (l, p) in self.level.iter_mut().zip(peak) {
            let fresh = ((db(p) - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
            *l = fresh.max(*l - dt * 30.0 / -FLOOR_DB).max(0.0);
        }
        // Moves under a pixel aren't worth a redraw.
        before
            .iter()
            .zip(self.level)
            .any(|(a, b)| (a - b).abs() * METER_W as f32 >= 1.0)
    }

    /// The meter's level on each side, 0 to 1.
    pub fn level(&self) -> [f32; 2] {
        self.level
    }

    /// REC's place in the row `r`.
    pub fn rec_rect(r: Rect) -> Rect {
        let r = r.inset_xy(0, 4);
        Rect::new(r.x, r.y, REC_W, r.h)
    }

    /// Card `k`'s place ([`STEPS`]'s order).
    pub fn card_rect(r: Rect, k: usize) -> Rect {
        let r = r.inset_xy(0, 4);
        let x = r.x + REC_W + GAP + CARD_W[..k].iter().map(|w| w + GAP).sum::<i32>();
        Rect::new(x, r.y, CARD_W[k.min(2)], r.h)
    }

    /// The meter's place: two bars at the Source card's right.
    fn meter_rect(r: Rect) -> Rect {
        let c = Recorder::card_rect(r, 0);
        let (_, rest) = c.take_left(26);
        let (_, mid) = rest.take_right(26);
        mid.take_right(METER_W + 14).0
    }

    /// The Keep card's place: the rest of the row.
    pub fn keep_rect(r: Rect) -> Rect {
        let last = Recorder::card_rect(r, 2);
        let x = last.right() + GAP;
        let r = r.inset_xy(0, 4);
        Rect::new(x, r.y, (r.right() - x).max(0), r.h)
    }

    fn can_press(&self) -> bool {
        self.view.recording || self.view.cannot.is_none()
    }

    fn values(&self) -> [(String, String); 3] {
        let v = &self.view;
        let s = &v.settings;
        [
            (
                "Source".to_string(),
                format!(
                    "{} · {}",
                    rec_tab::value_text(s, Row::Source, &v.sources),
                    match s.pick {
                        mpfree_sampler::record::Pick::Stereo => "stereo",
                        mpfree_sampler::record::Pick::Left => "left",
                        mpfree_sampler::record::Pick::Right => "right",
                        mpfree_sampler::record::Pick::Mono => "mono",
                    }
                ),
            ),
            (
                if v.recording && !v.waiting {
                    "Started"
                } else {
                    "Starts"
                }
                .to_string(),
                starts_text(s),
            ),
            ("Ends".to_string(), ends_text(s)),
        ]
    }
}

impl Widget for Recorder {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let red = Color::rgb(REC_COLOR);
        // REC: lit while a take records, outlined while it waits.
        let b = Recorder::rec_rect(r);
        let v = &self.view;
        let (fill, ink, dot) = if v.recording && !v.waiting {
            (red, t.bg, t.bg)
        } else if self.can_press() {
            (t.raised, t.text, red)
        } else {
            (t.raised, t.text_faint, red.mix(t.raised, 0.6))
        };
        cx.canvas.fill_round_rect(b, 10, fill);
        if v.waiting {
            cx.canvas.stroke_round_rect(b, 10, 2, red);
        }
        // "● REC" over the second line: where the take goes, how long it
        // has run, what it waits for, or why REC can't start one.
        let label = if v.waiting { "Waits" } else { "REC" };
        let inner = b.inset_xy(14, 7);
        let (top, bottom) = inner.take_top(inner.h / 2 + 2);
        let tw = cx.measure(FontId::Bold, 17.0, label);
        let d = Rect::new(top.x, top.y + top.h / 2 - 7, 14, 14);
        cx.canvas.fill_round_rect(d, 7, dot);
        cx.text(
            FontId::Bold,
            17.0,
            Rect::new(d.right() + 8, top.y, tw + 4, top.h),
            Align::Left,
            ink,
            label,
        );
        let lit = v.recording && !v.waiting;
        let (line, line_ink) = match v.cannot.as_ref().filter(|_| !v.recording) {
            Some(why) => (why.as_str(), t.warn),
            None if lit => (v.status.as_str(), t.bg),
            None if self.can_press() => (v.status.as_str(), t.text_dim),
            None => (v.status.as_str(), t.text_faint),
        };
        cx.text(FontId::Sans, 12.5, bottom, Align::Left, line_ink, line);
        // The three cards: ‹ name over value ›.
        for (k, (name, value)) in self.values().iter().enumerate() {
            let c = Recorder::card_rect(r, k);
            let busy = k == 0 && v.recording;
            cx.canvas.fill_round_rect(c, 10, t.raised);
            let ink = if busy { t.text_faint } else { t.text };
            let arrow = if busy { t.text_faint } else { t.text_dim };
            let (left, rest) = c.take_left(26);
            let (right, mid) = rest.take_right(26);
            cx.text(FontId::Sans, 18.0, left, Align::Center, arrow, "‹");
            cx.text(FontId::Sans, 18.0, right, Align::Center, arrow, "›");
            let mid = if k == 0 {
                self.paint_meter(cx, r);
                mid.take_right(METER_W + 14).1
            } else {
                mid
            };
            let (top, bottom) = mid.inset_xy(2, 8).take_top(18);
            cx.text(FontId::Sans, 12.5, top, Align::Left, t.text_dim, name);
            cx.text(FontId::Bold, 16.0, bottom, Align::Left, ink, value);
        }
        self.paint_keep(cx, r);
        *self.painted.borrow_mut() = Some((r, self.view.clone(), self.level));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let painted = self.painted.borrow();
        let (at, view, level) = painted.as_ref()?;
        if *at != r || *view != self.view {
            return None;
        }
        Some(if *level == self.level {
            Vec::new()
        } else {
            vec![Recorder::meter_rect(r)]
        })
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = vec![Target::tap(
            "rec",
            0,
            if self.view.recording { "Stop" } else { "REC" },
            Recorder::rec_rect(r),
        )
        .disabled_if(!self.can_press())];
        for (k, (name, _)) in self.values().iter().enumerate() {
            let c = Recorder::card_rect(r, k);
            let (left, right) = c.take_left(c.w / 2);
            let busy = k == 0 && self.view.recording;
            out.push(Target::tap("less", k, format!("{name} ‹"), left).disabled_if(busy));
            out.push(Target::tap("more", k, format!("{name} ›"), right).disabled_if(busy));
        }
        // Keep: a tap keeps the last bars, a hold opens Sample Recall.
        let keep = &self.view.keep;
        let at = Recorder::keep_rect(r);
        out.push(Target::tap("keep", 0, keep.title.clone(), at).disabled_if(!keep.ready));
        out.push(
            Target::hold(
                "keep-recall",
                0,
                format!("{}: hold for Sample Recall", keep.title),
                at,
            )
            .disabled_if(!keep.ready),
        );
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if Recorder::rec_rect(r).contains(x, y) {
            return Some(EventKind::Tab(0));
        }
        if Recorder::keep_rect(r).contains(x, y) {
            return Some(EventKind::Tab(1));
        }
        (0..STEPS.len()).find_map(|k| {
            let c = Recorder::card_rect(r, k);
            c.contains(x, y).then(|| EventKind::Step {
                index: k,
                delta: if x < c.x + c.w / 2 { -1 } else { 1 },
            })
        })
    }
}

impl Recorder {
    /// The Keep card: the icon (a turn back round a clock), what a tap
    /// keeps, and where it goes or why it keeps nothing now.
    fn paint_keep(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let k = &self.view.keep;
        let c = Recorder::keep_rect(r);
        if c.w <= 0 {
            return;
        }
        cx.canvas.fill_round_rect(c, 10, t.raised);
        let (ink, dim, icon) = if k.ready {
            (t.text, t.text_dim, t.warn)
        } else {
            (t.text_faint, t.text_faint, t.text_faint)
        };
        let (left, rest) = c.inset_xy(KEEP_PAD, 0).take_left(KEEP_ICON + 10);
        keep_icon(cx, left.x, left.y + left.h / 2 - KEEP_ICON / 2, icon);
        let (top, bottom) = rest.inset_xy(0, 8).take_top(22);
        cx.text(FontId::Bold, 16.0, top, Align::Left, ink, &k.title);
        cx.text(FontId::Sans, 12.0, bottom, Align::Left, dim, &k.line);
    }

    /// The meter alone (a redraw of its part paints only this).
    fn paint_meter(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let m = Recorder::meter_rect(r);
        cx.canvas.fill_rect(m, t.raised);
        for (side, lv) in self.level.iter().enumerate() {
            let bar = Rect::new(m.x, m.y + m.h / 2 - 6 + side as i32 * 8, METER_W, 4);
            cx.canvas.fill_round_rect(bar, 2, t.surface);
            let w = (lv * METER_W as f32).round() as i32;
            if w > 0 {
                let c = if *lv >= 1.0 {
                    t.bad
                } else if *lv > 0.9 {
                    t.warn
                } else {
                    t.good
                };
                cx.canvas
                    .fill_round_rect(Rect::new(bar.x, bar.y, w, bar.h), 2, c);
            }
        }
    }
}

/// The Keep card's icon, `KEEP_ICON` square at (`x`, `y`): an arrow
/// turning back round a clock's hands, as the mockup draws it.
fn keep_icon(cx: &mut PaintCx, x: i32, y: i32, c: Color) {
    use std::f32::consts::PI;
    let s = KEEP_ICON as f32 / 24.0;
    let (ox, oy) = (x as f32, y as f32);
    let p = |px: f32, py: f32| (ox + px * s, oy + py * s);
    let w = 2.0 * s;
    let mut m = Shape::new(Rect::new(x - 2, y - 2, KEEP_ICON + 4, KEEP_ICON + 4));
    // From the top left round clockwise to the left (the gap between
    // is where the arrow turns back).
    let (cxp, cyp) = p(12.0, 12.0);
    m.arc(cxp, cyp, 9.0 * s, PI * 1.27, PI * 3.0, w);
    // The arrowhead at the arc's start, and the hands.
    m.polyline(&[p(3.0, 4.0), p(3.0, 9.0), p(8.0, 9.0)], w);
    m.polyline(&[p(12.0, 8.0), p(12.0, 12.0), p(15.0, 14.0)], w);
    m.draw(cx.canvas, c);
}

/// What a touch on the row asks for: REC records or stops, a card steps
/// its setting, Keep keeps the last bars (or opens Sample Recall).
pub fn action(v: &RecorderView, kind: &EventKind) -> Option<RecorderAction> {
    match kind {
        EventKind::Tab(0) if v.recording => Some(RecorderAction::Stop),
        EventKind::Tab(0) => Some(RecorderAction::Record),
        EventKind::Tab(1) => Some(RecorderAction::Keep),
        EventKind::Step { index, delta } => Some(RecorderAction::Step(*STEPS.get(*index)?, *delta)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Rect {
        Rect::new(0, 120, 1280, 64)
    }

    #[test]
    fn the_cards_say_how_a_take_starts_and_ends() {
        let mut s = RecSettings::default();
        assert_eq!(
            (starts_text(&s), ends_text(&s)),
            ("Now".into(), "On STOP".into())
        );
        s.start = StartAt::Threshold;
        s.end = EndAt::Length;
        assert_eq!(starts_text(&s), "On input · -30 dB");
        assert_eq!(ends_text(&s), "After 4 bars");
        s.start = StartAt::NextBar;
        s.end = EndAt::Silence;
        assert_eq!(
            (starts_text(&s), ends_text(&s)),
            ("Next bar".into(), "On silence".into())
        );
    }

    #[test]
    fn rec_records_or_stops_and_a_cards_halves_step_it() {
        let mut w = Recorder::new();
        let r = row();
        let rec = Recorder::rec_rect(r);
        let tap = |w: &mut Recorder, x, y| w.touch(1, Touch::Down, x, y, r);
        let kind = tap(&mut w, rec.x + 5, rec.y + 5).unwrap();
        assert_eq!(action(&w.view, &kind), Some(RecorderAction::Record));
        w.view.recording = true;
        assert_eq!(action(&w.view, &kind), Some(RecorderAction::Stop));
        for (k, row) in STEPS.iter().enumerate() {
            let c = Recorder::card_rect(r, k);
            let back = tap(&mut w, c.x + 4, c.y + 10).unwrap();
            let on = tap(&mut w, c.right() - 4, c.y + 10).unwrap();
            assert_eq!(action(&w.view, &back), Some(RecorderAction::Step(*row, -1)));
            assert_eq!(action(&w.view, &on), Some(RecorderAction::Step(*row, 1)));
        }
        // Past the cards, the Keep card: a finger on it asks the app,
        // which keeps on the lift or opens Sample Recall on a hold.
        let keep = tap(&mut w, r.right() - 4, r.y + 30).unwrap();
        assert_eq!(action(&w.view, &keep), Some(RecorderAction::Keep));
        // The cards don't overlap REC or each other, and Keep fits the
        // rest of a 1116 px row with its icon and its longest line.
        assert!(Recorder::card_rect(r, 0).x > rec.right());
        let k = Recorder::keep_rect(r);
        assert!(k.x > Recorder::card_rect(r, 2).right());
        assert_eq!(k.right(), r.right());
        let narrow = Rect::new(12, 120, 1116, 64);
        assert!(Recorder::keep_rect(narrow).w >= 2 * KEEP_PAD + KEEP_ICON + 10 + 180);
    }

    #[test]
    fn keep_is_faint_and_says_why_until_a_tap_keeps_something() {
        let mut w = Recorder::new();
        let keep = |w: &Recorder| {
            w.targets(row())
                .into_iter()
                .filter(|t| t.id.starts_with("keep"))
                .map(|t| (t.id, t.gesture, t.enabled))
                .collect::<Vec<_>>()
        };
        use mpfree_ui::Gesture::{Hold, Tap};
        assert_eq!(w.view.keep.title, "Keep last 4 bars");
        assert_eq!(w.view.keep.line, KEEP_NOT_YET);
        assert_eq!(
            keep(&w),
            [("keep", Tap, false), ("keep-recall", Hold, false)]
        );
        w.view.keep = KeepView {
            title: "Keep last 10 s".into(),
            line: "what just played, as a new take".into(),
            ready: true,
        };
        assert_eq!(keep(&w), [("keep", Tap, true), ("keep-recall", Hold, true)]);
    }

    #[test]
    fn rec_is_faint_when_it_cant_start_a_take_and_the_source_while_one_records() {
        let mut w = Recorder::new();
        w.view.cannot = Some("No empty pad left for a take".into());
        let t = w.targets(row());
        assert!(!t[0].enabled);
        w.view.recording = true;
        let t = w.targets(row());
        assert!(t[0].enabled, "REC stops the take");
        assert!(
            !t[1].enabled && !t[2].enabled,
            "the source waits for the take"
        );
        assert!(t[3].enabled);
    }

    #[test]
    fn the_meter_rises_at_once_and_falls_thirty_db_a_second() {
        let mut w = Recorder::new();
        assert!(w.meter([1.0, 0.1], 0.0));
        assert_eq!(w.level(), [1.0, 2.0 / 3.0]);
        // Half a second of silence: 15 dB down, a quarter of the scale.
        w.meter([0.0, 0.0], 0.5);
        assert!((w.level()[0] - 0.75).abs() < 1e-4);
        assert!(!w.meter([0.0, 0.0], 0.001), "under a pixel");
    }

    #[test]
    fn the_input_section_has_the_source_channels_threshold_pre_roll_and_onto() {
        let v = RecorderView {
            sources: vec!["Line in".into(), "Master".into()],
            recording: true,
            ..Default::default()
        };
        let (rows, controls, panel) = input_list(&v);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["Source", "Channels", "Threshold", "Pre-roll", "Onto"]
        );
        assert_eq!(rows[4].value, "Stay here", "a take stays on the page");
        assert_eq!(rows[0].value, "Line in");
        assert_eq!(rows[2].value, "-30 dB");
        assert!(controls[0].faint, "a take keeps its source");
        assert!(matches!(controls[2].look, Look::Knob { .. }));
        assert_eq!(panel.sections[0].title, "Input");
    }
}
