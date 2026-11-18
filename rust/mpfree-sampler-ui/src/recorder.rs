//! The one Sample page's recorder row (one-page step 2a, Jay's mockup of
//! 18 Nov): **REC**, the **Source** with its meter, how a take **Starts**
//! and how it **Ends**, along the Wave page's top, with a line saying where
//! a take goes. It sets and records with the Rec page's own settings
//! ([`RecSettings`]), so the two agree while both exist; the app holds
//! them and makes the take as the Rec page's Record does.
//!
//! - **REC** starts a take; while one records (or waits for its start)
//!   it is lit and a press stops it, keeping what came in.
//! - **Source**, **Starts** and **Ends** step with a tap on their left
//!   half (back) or right half (on): Source among the line input and the
//!   points in the mix, Starts among Now, On input (the threshold) and
//!   Next bar, Ends among On STOP, On silence and a length in bars.
//! - While REC is on, the panel under the wave is **Input** instead of
//!   the region's settings ([`INPUT_ROWS`]): the source, its channels,
//!   the threshold and the pre-roll.

use std::cell::RefCell;

use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use crate::panel::{Control, Item, Look, Panel, Section};
use crate::rec_tab::{self, EndAt, RecSettings, Row, StartAt};
use crate::rec_widgets::{db, FLOOR_DB, REC_COLOR};
use crate::widgets::Setting;

/// The widget's name on the Wave page.
pub const RECORDER: &str = "smp-recorder";

/// The cards that step, in their order on the row (the
/// [`EventKind::Step`] index).
pub const STEPS: [Row; 3] = [Row::Source, Row::Start, Row::End];

/// The rows of the Input section the panel shows while REC is on, in its
/// order (the [`EventKind::Step`] index on the panel).
pub const INPUT_ROWS: [Row; 4] = [Row::Source, Row::Input, Row::Threshold, Row::PreRoll];

/// What the Q-Links turn while REC is on (the plan's step 2a): Q-Link 2
/// the Threshold, Q-Link 3 the Pre-roll. Q-Links 1 and 4 are faint until
/// the recorder has a Gain and a Monitor level (step 2c).
pub const QLINK_ROWS: [Option<Row>; 4] = [None, Some(Row::Threshold), Some(Row::PreRoll), None];

/// Why Q-Links 1 and 4 are faint while REC is on.
pub const QLINK_IDLE: &str = "While REC is on, Q-Links 2 and 3 turn Threshold and Pre-roll";

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
    /// left), said on the row; None when it can.
    pub cannot: Option<String>,
    /// Where a take goes, or how the one recording is going, in a line.
    pub status: String,
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
            vec![vec![it(0).labelled(), it(1).labelled()], vec![it(2), it(3)]],
        )
        .note("while REC is on")],
    };
    (rows, controls, panel)
}

const GAP: i32 = 10;
const REC_W: i32 = 120;
const CARD_W: [i32; 3] = [260, 200, 180];
const METER_W: i32 = 44;

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

    /// Where the status line goes: the rest of the row.
    fn status_rect(r: Rect) -> Rect {
        let last = Recorder::card_rect(r, 2);
        let x = last.right() + 2 * GAP;
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
        let label = if v.waiting { "Waits" } else { "REC" };
        let tw = cx.measure(FontId::Bold, 17.0, label);
        let x = b.x + (b.w - tw - 22) / 2;
        let d = Rect::new(x, b.y + b.h / 2 - 7, 14, 14);
        cx.canvas.fill_round_rect(d, 7, dot);
        cx.text(
            FontId::Bold,
            17.0,
            Rect::new(d.right() + 8, b.y, tw + 4, b.h),
            Align::Left,
            ink,
            label,
        );
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
        *self.painted.borrow_mut() = Some((r, self.view.clone(), self.level));
        // Where a take goes, or why REC can't start one.
        let s = Recorder::status_rect(r).inset_xy(0, 8);
        let (top, bottom) = s.take_top(s.h / 2);
        cx.text(FontId::Sans, 13.5, top, Align::Left, t.text, &v.status);
        if let Some(why) = v.cannot.as_ref().filter(|_| !v.recording) {
            cx.text(FontId::Sans, 12.5, bottom, Align::Left, t.warn, why);
        } else {
            let how = if v.recording {
                "REC again or STOP keeps the take"
            } else {
                "Tap a card's ‹ or › half to step it"
            };
            cx.text(FontId::Sans, 12.5, bottom, Align::Left, t.text_dim, how);
        }
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
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if Recorder::rec_rect(r).contains(x, y) {
            return Some(EventKind::Tab(0));
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

/// What a touch on the row asks for: REC records or stops, a card steps
/// its setting.
pub fn action(v: &RecorderView, kind: &EventKind) -> Option<RecorderAction> {
    match kind {
        EventKind::Tab(0) if v.recording => Some(RecorderAction::Stop),
        EventKind::Tab(0) => Some(RecorderAction::Record),
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
        // Nothing past the cards answers but the status line, which is
        // only words.
        assert_eq!(tap(&mut w, r.right() - 4, r.y + 30), None);
        // The cards don't overlap REC or each other, and fit the row.
        assert!(Recorder::card_rect(r, 0).x > rec.right());
        assert!(Recorder::card_rect(r, 2).right() < r.right() - 300);
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
    fn the_input_section_has_the_source_channels_threshold_and_pre_roll() {
        let v = RecorderView {
            sources: vec!["Line in".into(), "Master".into()],
            recording: true,
            ..Default::default()
        };
        let (rows, controls, panel) = input_list(&v);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Source", "Channels", "Threshold", "Pre-roll"]);
        assert_eq!(rows[0].value, "Line in");
        assert_eq!(rows[2].value, "-30 dB");
        assert!(controls[0].faint, "a take keeps its source");
        assert!(matches!(controls[2].look, Look::Knob { .. }));
        assert_eq!(panel.sections[0].title, "Input");
    }
}
