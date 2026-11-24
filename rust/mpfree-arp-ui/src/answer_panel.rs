//! The Phrase tab with Plays: When I stop (composing tools S5, board B):
//! Answer, call and response.
//!
//! Two panels take the places of the list and the phrase that plays; the
//! shelves stay, faint, since Answer makes its own notes:
//!
//! - [`CallPicture`], in the list's place: the call you played and the
//!   answer after it, time across and pitch up, the answer solid up to
//!   where it has played and outlined after; over them the two lanes (You,
//!   its bars; Answer, how it ends); under them the turn light (You
//!   played, Quiet, Answer, Your turn), the step it is at outlined.
//! - [`AnswerPanel`], in the detail's place: the Plays row (When I stop
//!   lit), Mode (Echo, Answer, Trade), Answers for (Like mine, 1, 2 or 4
//!   bars; in Trade, each turn's bars), the key the call was read in, when
//!   it starts, **Answer now** and **Capture both as a clip**, and the
//!   reply's Q-Links (Variation, Activity, Range) as sliders.
//!
//! What they show is worked out from an [`ArpState`] by [`call_show`],
//! [`answer_show`] and [`answer_sliders`], which draw nothing, so the
//! tests read those.

use crate::phrase_tab::WHEN;
use crate::view::{AnswerMode, AnswerNow, ArpState, HeardNote, PlaysWhen, ANSWERS_FOR};
use crate::widgets::{Slider, SliderStack};
use mpfree_arp::{Light, PPQ};
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, Color, EventKind, PaintCx, Rect, Target, Touch, Widget};

/// The Mode row's places: `Tab(MODE + i)`, `i` in [`AnswerMode::ALL`].
pub const MODE: usize = 20;
/// The Answers for row's: `Tab(FOR + i)`, `i` in [`ANSWERS_FOR`].
pub const FOR: usize = 30;
/// Answer now and Capture both as a clip: `Toggle(NOW)`, `Toggle(CAPTURE)`.
pub const NOW: usize = 0;
pub const CAPTURE: usize = 1;
/// The reply's Q-Links, on the panel's sliders.
pub const ANSWER_QLINK_LABELS: [&str; 3] = ["Variation", "Activity", "Range"];
/// Range's three places, as the slider says them.
pub const RANGES: [&str; 3] = ["narrow", "medium", "wide"];

fn bars_word(n: u32) -> String {
    match n {
        1 => "1 bar".to_string(),
        n => format!("{n} bars"),
    }
}

// ---------------------------------------------------------------- call

/// What [`CallPicture`] shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallShow {
    /// At the right of the heading: "answering", "listening", "your turn".
    pub status: String,
    /// The status in the good colour (the answer plays).
    pub status_good: bool,
    pub call: Vec<HeardNote>,
    pub answer: Vec<HeardNote>,
    pub bar_ticks: i64,
    pub call_bars: u32,
    pub answer_bars: u32,
    pub playhead: Option<i64>,
    /// The two lanes over the picture: (name, what it is).
    pub lanes: [(String, String); 2],
    /// The turn light's four steps, (name, what it says), and the one lit.
    pub steps: [(String, String); 4],
    pub lit: Option<usize>,
    /// The lines under the turn light.
    pub lines: [String; 3],
}

/// The call and its answer, and the turn light. Nothing here answers a
/// finger.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallPicture {
    pub show: CallShow,
}

struct CallParts {
    head: Rect,
    lanes: Rect,
    picture: Rect,
    light: Rect,
    lines: [Rect; 3],
}

const LIGHT_H: i32 = 54;

impl CallPicture {
    fn parts(r: Rect) -> CallParts {
        let (x, w) = (r.x + 14, (r.w - 28).max(1));
        let lines = |i: i32| Rect::new(x + 2, r.bottom() - 14 - (3 - i) * 22, w, 20);
        let light = Rect::new(x, lines(0).y - 12 - LIGHT_H, w, LIGHT_H);
        let lanes = Rect::new(x, r.y + 40, w, 26);
        let top = lanes.bottom() + 8;
        CallParts {
            head: Rect::new(r.x + 16, r.y + 10, (r.w - 32).max(1), 22),
            lanes,
            picture: Rect::new(x, top, w, (light.y - 14 - top).max(1)),
            light,
            lines: [lines(0), lines(1), lines(2)],
        }
    }

    /// Step `i` of the turn light.
    pub fn step(r: Rect, i: usize) -> Rect {
        let l = CallPicture::parts(r).light;
        let w = (l.w - 3 * 10) / 4;
        Rect::new(l.x + i as i32 * (w + 10), l.y, w, l.h)
    }

    fn ticks(&self) -> i64 {
        let s = &self.show;
        (i64::from(s.call_bars.max(1)) + i64::from(s.answer_bars.max(1))) * s.bar_ticks.max(1)
    }

    fn x_of(&self, r: Rect, t: i64) -> i32 {
        r.x + (t as f32 * r.w as f32 / self.ticks().max(1) as f32).round() as i32
    }
}

impl Widget for CallPicture {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let s = &self.show;
        let p = CallPicture::parts(r);
        cx.panel(r);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            p.head,
            Align::Left,
            t.text_dim,
            "CALL AND ANSWER",
        );
        cx.text(
            FontId::Bold,
            t.size_small,
            p.head,
            Align::Right,
            if s.status_good {
                t.readable(t.good, t.surface)
            } else {
                t.text_dim
            },
            &s.status,
        );
        // The lanes: the call's bars, then the answer's.
        let split = self.x_of(p.lanes, i64::from(s.call_bars.max(1)) * s.bar_ticks.max(1));
        let lane = |cx: &mut PaintCx, b: Rect, ink: Color, (name, says): &(String, String)| {
            cx.canvas
                .fill_round_rect(b.inset_xy(2, 0), t.round(8), ink.with_alpha(46));
            let words = Rect::new(b.x + 12, b.y, (b.w - 16).max(1), b.h);
            cx.text(
                FontId::Bold,
                t.size_small,
                words,
                Align::Left,
                t.text,
                &format!("{name} · {says}"),
            );
        };
        let l = p.lanes;
        lane(
            cx,
            Rect::new(l.x, l.y, split - l.x, l.h),
            t.accent,
            &s.lanes[0],
        );
        lane(
            cx,
            Rect::new(split, l.y, l.right() - split, l.h),
            t.accent2,
            &s.lanes[1],
        );
        // The picture.
        let pic = p.picture;
        cx.canvas.fill_round_rect(pic, t.round(10), t.bg);
        let inner = pic.inset_xy(6, 6);
        let bar = s.bar_ticks.max(1);
        let beat_line = t.outline.with_alpha(t.outline.a / 3);
        let mut tick = 0;
        while tick <= self.ticks() {
            let x = self.x_of(inner, tick).min(inner.right() - 1);
            let ink = if tick % bar == 0 {
                t.outline
            } else {
                beat_line
            };
            cx.canvas.fill_rect(Rect::new(x, inner.y, 1, inner.h), ink);
            tick += i64::from(PPQ);
        }
        let all = s.call.iter().chain(&s.answer);
        let (lo, hi) = all.fold((127u8, 0u8), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        if lo <= hi {
            let span = i32::from(hi - lo).max(8);
            let low = i32::from(lo) - (span - i32::from(hi - lo)) / 2;
            let row = inner.h as f32 / (span + 1) as f32;
            let nh = (row - 1.0).clamp(4.0, 12.0).round() as i32;
            let note_box = |n: &HeardNote| {
                let x = self.x_of(inner, n.at).min(inner.right() - 1);
                let w = (self.x_of(inner, n.at + n.len) - x - 1)
                    .max(3)
                    .min(inner.right() - x);
                let up = (span - (i32::from(n.pitch) - low)) as f32;
                let y = inner.y + (up * row + (row - nh as f32) / 2.0).round() as i32;
                Rect::new(x, y.clamp(inner.y, inner.bottom() - nh), w, nh)
            };
            for n in &s.call {
                cx.canvas.fill_round_rect(note_box(n), 2, t.accent);
            }
            for n in &s.answer {
                let b = note_box(n);
                if s.playhead.is_some_and(|h| n.at < h) || s.playhead.is_none() && s.lit != Some(2)
                {
                    cx.canvas.fill_round_rect(b, 2, t.accent2);
                } else {
                    cx.canvas.fill_round_rect(b, 2, t.accent2.with_alpha(56));
                    cx.canvas.stroke_round_rect(b, 2, 1, t.accent2);
                }
            }
        }
        if let Some(h) = s.playhead {
            let x = self.x_of(inner, h).min(inner.right() - 2);
            cx.canvas
                .fill_rect(Rect::new(x, inner.y, 2, inner.h), t.good);
        }
        // The turn light.
        for (i, (name, says)) in s.steps.iter().enumerate() {
            let b = CallPicture::step(r, i);
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            if s.lit == Some(i) {
                cx.canvas.stroke_round_rect(b, t.round(10), 2, t.good);
            }
            let ink = match i {
                0 => t.readable(t.accent, t.raised),
                2 => t.readable(t.accent2, t.raised),
                _ => t.text,
            };
            let words = Rect::new(b.x + 10, b.y + 7, (b.w - 14).max(1), 22);
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                words,
                Align::Left,
                ink,
                name,
            );
            let under = Rect::new(b.x + 10, b.y + 29, (b.w - 14).max(1), 18);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                under,
                Align::Left,
                t.text_dim,
                says,
            );
        }
        for (i, line) in s.lines.iter().enumerate() {
            let ink = if i == 2 {
                t.faded(t.text_dim, t.surface)
            } else {
                t.text_dim
            };
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                p.lines[i],
                Align::Left,
                ink,
                line,
            );
        }
    }

    fn targets(&self, _r: Rect) -> Vec<Target> {
        Vec::new()
    }

    fn touch(&mut self, _f: u64, _p: Touch, _x: i32, _y: i32, _r: Rect) -> Option<EventKind> {
        None
    }
}

/// What [`CallPicture`] shows for `st`.
pub fn call_show(st: &ArpState) -> CallShow {
    let a = &st.improvise.answer;
    let trade = a.mode == AnswerMode::Trade;
    let turn = a.bars.unwrap_or(crate::view::TRADE_BARS);
    let call_bars = a.call_bars.max(1);
    let answer_bars = if a.answer_bars > 0 {
        a.answer_bars
    } else if trade {
        u32::from(turn)
    } else {
        a.bars.map_or(call_bars, u32::from)
    };
    let (status, good) = match a.light {
        None => (String::new(), false),
        Some(Light::YourTurn) if a.call.is_empty() => ("your turn".to_string(), false),
        Some(Light::YourTurn) => ("your turn again".to_string(), false),
        Some(Light::You | Light::Quiet) => ("listening".to_string(), false),
        Some(Light::Coming) if a.answer.is_empty() => ("making the answer".to_string(), false),
        Some(Light::Coming) => ("answers on the bar line".to_string(), false),
        Some(Light::Answer) => ("answering".to_string(), true),
    };
    let lit = a.light.map(|l| match l {
        Light::You => 0,
        Light::Quiet | Light::Coming => 1,
        Light::Answer => 2,
        Light::YourTurn => 3,
    });
    let bar_of = a.playhead.map(|h| {
        let into = h - i64::from(call_bars) * a.bar_ticks.max(1);
        (into / a.bar_ticks.max(1)) as u32 + 1
    });
    let answer_says = match (a.light, bar_of) {
        (Some(Light::Answer), Some(b)) => format!("bar {} of {answer_bars}", b.min(answer_bars)),
        _ => bars_word(answer_bars),
    };
    let ends = a.ends.map_or_else(|| "coming".to_string(), str::to_string);
    let steps = if trade {
        [
            ("Your turn".to_string(), bars_word(u32::from(turn))),
            ("Coming".to_string(), "on the turn".to_string()),
            ("Answer".to_string(), answer_says),
            ("Your turn".to_string(), "play on the turn".to_string()),
        ]
    } else {
        [
            ("You played".to_string(), bars_word(call_bars)),
            ("Quiet".to_string(), "1 beat".to_string()),
            ("Answer".to_string(), answer_says),
            ("Your turn".to_string(), "play a note".to_string()),
        ]
    };
    let third = match &a.refused {
        Some(_) => "Nothing to answer yet: play a phrase, then stop.".to_string(),
        None => "The list is for While I hold: Answer makes its own notes.".to_string(),
    };
    CallShow {
        status,
        status_good: good,
        call: a.call.clone(),
        answer: a.answer.clone(),
        bar_ticks: a.bar_ticks.max(1),
        call_bars,
        answer_bars,
        playhead: a.playhead,
        lanes: [
            ("You".to_string(), bars_word(call_bars)),
            ("Answer".to_string(), ends),
        ],
        steps,
        lit,
        lines: [
            "The strip is the turn light: it shows whose turn it is.".to_string(),
            "Play while it answers and it stops at once: your turn again.".to_string(),
            third,
        ],
    }
}

// ---------------------------------------------------------------- panel

/// What [`AnswerPanel`] shows, apart from its sliders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnswerShow {
    /// Places in [`AnswerMode::ALL`] and [`ANSWERS_FOR`].
    pub mode: usize,
    pub bars: usize,
    /// Answers for's label: "Answers for", or "Turns" in Trade.
    pub bars_label: String,
    /// The lines at the right of the Mode and Answers for labels.
    pub mode_says: String,
    pub bars_says: String,
    /// The key the call was read in, and where it came from.
    pub key: String,
    pub key_says: String,
    pub starts: String,
    /// Answer now can be pressed: you are playing a call (not in Trade).
    pub can_answer_now: bool,
}

/// How Answer replies, Answer now and Capture, and the reply's sliders. A
/// tap on the Plays row gives `Tab(WHEN + i)`, on Mode `Tab(MODE + i)`, on
/// Answers for `Tab(FOR + i)`, on Answer now `Toggle(NOW)`, on Capture
/// `Toggle(CAPTURE)`; a finger on slider `i` gives `Select(i)`, then
/// `Value(v)` ([`SliderStack`]).
pub struct AnswerPanel {
    pub show: AnswerShow,
    pub sliders: SliderStack,
}

impl Default for AnswerPanel {
    fn default() -> Self {
        let mut sliders = SliderStack::new("", &ANSWER_QLINK_LABELS);
        sliders.columns = 2;
        AnswerPanel {
            show: AnswerShow::default(),
            sliders,
        }
    }
}

impl PartialEq for AnswerPanel {
    fn eq(&self, other: &Self) -> bool {
        self.show == other.show
            && self.sliders.rows == other.sliders.rows
            && self.sliders.active == other.sliders.active
    }
}

const ROW_H: i32 = 44;
const LABEL_W: i32 = 84;

struct PanelParts {
    plays: Rect,
    mode_label: Rect,
    mode: Rect,
    for_label: Rect,
    bars: Rect,
    key: Rect,
    starts: Rect,
    rule: Rect,
    now: Rect,
    capture: Rect,
    capture_says: Rect,
    sliders: Rect,
}

impl AnswerPanel {
    fn parts(r: Rect) -> PanelParts {
        let x = r.x + 14;
        let w = (r.w - 28).max(1);
        let half = (w - 10) / 2;
        let label = |y: i32| Rect::new(r.x + 16, y, (r.w - 32).max(1), 20);
        let sliders = Rect::new(r.x + 16, r.bottom() - 12 - 84, (r.w - 32).max(1), 84);
        PanelParts {
            plays: Rect::new(x, r.y + 34, w, ROW_H),
            mode_label: label(r.y + 88),
            mode: Rect::new(x, r.y + 110, w, ROW_H),
            for_label: label(r.y + 164),
            bars: Rect::new(x, r.y + 186, w, ROW_H),
            key: label(r.y + 242),
            starts: label(r.y + 270),
            rule: Rect::new(x, r.y + 302, w, 1),
            now: Rect::new(x, r.y + 316, half, 52),
            capture: Rect::new(x + half + 10, r.y + 316, w - half - 10, 52),
            capture_says: label(r.y + 376),
            sliders,
        }
    }

    fn place(row: Rect, n: usize, i: usize) -> Rect {
        let w = row.w / n.max(1) as i32;
        Rect::new(row.x + i as i32 * w, row.y, w, row.h)
    }

    /// Half `i` of the Plays row.
    pub fn when_half(r: Rect, i: usize) -> Rect {
        AnswerPanel::place(AnswerPanel::parts(r).plays, PlaysWhen::ALL.len(), i)
    }

    /// Place `i` of the Mode row.
    pub fn mode_place(r: Rect, i: usize) -> Rect {
        AnswerPanel::place(AnswerPanel::parts(r).mode, AnswerMode::ALL.len(), i)
    }

    /// Place `i` of the Answers for row.
    pub fn for_place(r: Rect, i: usize) -> Rect {
        AnswerPanel::place(AnswerPanel::parts(r).bars, ANSWERS_FOR.len(), i)
    }

    /// Answer now's button and Capture's.
    pub fn button(r: Rect, which: usize) -> Rect {
        let p = AnswerPanel::parts(r);
        if which == NOW {
            p.now
        } else {
            p.capture
        }
    }

    /// Slider `i`'s area and its track.
    pub fn slider(&self, r: Rect, i: usize) -> (Rect, Rect) {
        self.sliders.row_rects_in(AnswerPanel::parts(r).sliders, i)
    }
}

/// A row of places, the one lit.
fn segments(cx: &mut PaintCx, r: Rect, names: &[&str], on: usize) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, t.round(10), t.bg);
    for (i, name) in names.iter().enumerate() {
        let b = AnswerPanel::place(r, names.len(), i);
        let lit = i == on;
        if lit {
            cx.canvas
                .fill_round_rect(b.inset(3), t.round(8), t.picked());
        }
        let ink = if lit {
            t.on_picked()
        } else {
            t.readable(t.text_dim, t.bg)
        };
        cx.text(
            FontId::Bold,
            t.size_small + 1.0,
            b,
            Align::Center,
            ink,
            name,
        );
    }
}

impl Widget for AnswerPanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let s = &self.show;
        let p = AnswerPanel::parts(r);
        cx.panel(r);
        let head = Rect::new(r.x + 16, r.y + 10, (r.w - 32).max(1), 20);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.text_dim,
            "PLAYS",
        );
        let when = PlaysWhen::ALL.map(PlaysWhen::label);
        segments(cx, p.plays, &when, 1);
        let label = |cx: &mut PaintCx, at: Rect, name: &str, says: &str| {
            cx.text(FontId::Sans, t.size_body, at, Align::Left, t.text_dim, name);
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                at,
                Align::Right,
                t.text_dim,
                says,
            );
        };
        label(cx, p.mode_label, "Mode", &s.mode_says);
        segments(cx, p.mode, &AnswerMode::ALL.map(AnswerMode::label), s.mode);
        label(cx, p.for_label, &s.bars_label, &s.bars_says);
        let fors = ["Like mine", "1 bar", "2 bars", "4 bars"];
        segments(cx, p.bars, &fors, s.bars);
        // The key and when it starts.
        let value = |at: Rect| {
            Rect::new(
                at.x + LABEL_W - 16,
                at.y,
                (at.w - LABEL_W + 16).max(1),
                at.h,
            )
        };
        cx.text(
            FontId::Sans,
            t.size_body,
            p.key,
            Align::Left,
            t.text_dim,
            "Key",
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            value(p.key),
            Align::Left,
            t.text,
            &s.key,
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            p.key,
            Align::Right,
            t.text_dim,
            &s.key_says,
        );
        cx.text(
            FontId::Sans,
            t.size_body,
            p.starts,
            Align::Left,
            t.text_dim,
            "Starts",
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            value(p.starts),
            Align::Left,
            t.text,
            &s.starts,
        );
        cx.canvas.fill_rect(p.rule, t.outline);
        // Answer now (outlined) and Capture.
        let ink = if s.can_answer_now {
            cx.canvas.stroke_round_rect(p.now, t.round(10), 2, t.accent);
            t.readable(t.accent, t.surface)
        } else {
            cx.canvas.fill_round_rect(p.now, t.round(10), t.raised);
            t.faded(t.text, t.raised)
        };
        cx.text(
            FontId::Bold,
            t.size_body,
            p.now,
            Align::Center,
            ink,
            "Answer now",
        );
        cx.canvas.fill_round_rect(p.capture, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            t.size_small + 1.0,
            p.capture,
            Align::Center,
            t.text,
            "Capture both as a clip",
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            p.capture_says,
            Align::Left,
            t.text_dim,
            "Capture: your call and its answer as one clip",
        );
        self.sliders.paint_in(cx, p.sliders);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = PlaysWhen::ALL
            .iter()
            .enumerate()
            .map(|(i, w)| {
                Target::tap("plays-when", i, w.label(), AnswerPanel::when_half(r, i))
                    .disabled_if(i == 1)
            })
            .collect();
        out.extend(AnswerMode::ALL.iter().enumerate().map(|(i, m)| {
            Target::tap("answer-mode", i, m.label(), AnswerPanel::mode_place(r, i))
                .disabled_if(i == self.show.mode)
        }));
        let fors = ["Like mine", "1 bar", "2 bars", "4 bars"];
        out.extend(fors.iter().enumerate().map(|(i, f)| {
            Target::tap("answers-for", i, *f, AnswerPanel::for_place(r, i))
                .disabled_if(i == self.show.bars)
        }));
        out.push(
            Target::tap("answer-now", NOW, "Answer now", AnswerPanel::button(r, NOW))
                .disabled_if(!self.show.can_answer_now),
        );
        out.push(Target::tap(
            "answer-capture",
            CAPTURE,
            "Capture both as a clip",
            AnswerPanel::button(r, CAPTURE),
        ));
        out.extend(self.sliders.targets_in(AnswerPanel::parts(r).sliders));
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase == Touch::Down {
            if AnswerPanel::when_half(r, 0).contains(x, y) {
                return Some(EventKind::Tab(WHEN));
            }
            if AnswerPanel::when_half(r, 1).contains(x, y) {
                return None;
            }
            if let Some(i) =
                (0..AnswerMode::ALL.len()).find(|&i| AnswerPanel::mode_place(r, i).contains(x, y))
            {
                return (i != self.show.mode).then(|| {
                    self.show.mode = i;
                    EventKind::Tab(MODE + i)
                });
            }
            if let Some(i) =
                (0..ANSWERS_FOR.len()).find(|&i| AnswerPanel::for_place(r, i).contains(x, y))
            {
                return (i != self.show.bars).then(|| {
                    self.show.bars = i;
                    EventKind::Tab(FOR + i)
                });
            }
            if AnswerPanel::button(r, NOW).contains(x, y) {
                return self.show.can_answer_now.then_some(EventKind::Toggle(NOW));
            }
            if AnswerPanel::button(r, CAPTURE).contains(x, y) {
                return Some(EventKind::Toggle(CAPTURE));
            }
        }
        self.sliders
            .touch_in(finger, phase, x, y, AnswerPanel::parts(r).sliders)
    }
}

/// What [`AnswerPanel`] shows for `st`.
pub fn answer_show(st: &ArpState) -> AnswerShow {
    let a: &AnswerNow = &st.improvise.answer;
    let trade = a.mode == AnswerMode::Trade;
    let turn = a.bars.unwrap_or(crate::view::TRADE_BARS);
    let bars_says = match (trade, a.bars) {
        (true, _) => format!("you, then it, {} each", bars_word(u32::from(turn))),
        (false, None) => "Like mine: as long as you played".to_string(),
        (false, Some(n)) => format!("{}, whatever you played", bars_word(u32::from(n))),
    };
    AnswerShow {
        mode: AnswerMode::ALL
            .iter()
            .position(|m| *m == a.mode)
            .unwrap_or(1),
        bars: ANSWERS_FOR.iter().position(|b| *b == a.bars).unwrap_or(0),
        bars_label: if trade { "Turns" } else { "Answers for" }.to_string(),
        mode_says: a.mode.says().to_string(),
        bars_says,
        key: a.key.clone().unwrap_or_else(|| "–".to_string()),
        key_says: if a.key.is_some() {
            "from your phrase"
        } else {
            "read from what you play"
        }
        .to_string(),
        starts: if trade {
            format!("when your {} end", bars_word(u32::from(turn)))
        } else {
            "on the next bar line after a beat of quiet".to_string()
        },
        can_answer_now: !trade && matches!(a.light, Some(Light::You | Light::Quiet)),
    }
}

/// The reply's sliders for `st`: Variation, Activity and Range.
pub fn answer_sliders(st: &ArpState) -> [Slider; 3] {
    let a = &st.improvise.answer;
    let variation = match a.variation {
        v if v < 0.34 => "steady",
        v if v < 0.67 => "some",
        _ => "restless",
    };
    let activity = match a.activity {
        v if v < 0.4 => "sparser",
        v if v <= 0.6 => "like mine",
        _ => "busier",
    };
    let range = usize::from(a.range.min(2));
    [
        Slider {
            label: ANSWER_QLINK_LABELS[0].to_string(),
            value: a.variation,
            text: variation.to_string(),
            ..Slider::default()
        },
        Slider {
            label: ANSWER_QLINK_LABELS[1].to_string(),
            value: a.activity,
            text: activity.to_string(),
            ..Slider::default()
        },
        Slider {
            label: ANSWER_QLINK_LABELS[2].to_string(),
            value: range as f32 / 2.0,
            text: RANGES[range].to_string(),
            ..Slider::default()
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{samples, Sample};
    use crate::view::{
        hint_for, layout_of, AnswerAction, ArpAction, ImproviseAction, ANSWER, ANSWER_HINT, CALL,
        KINDS, PHRASE_ANSWER_LAYOUT,
    };
    use mpfree_ui::{Fonts, Ui};

    fn sample() -> Sample {
        samples(Fonts::embedded)
            .unwrap()
            .into_iter()
            .find(|s| s.name == "phrase-answer")
            .expect("the When I stop sample")
    }

    fn tap(ui: &mut Ui, r: Rect) -> Option<mpfree_ui::UiEvent> {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        let ev = ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
        ev
    }

    /// Board B: the call and its answer in the list's place, how it
    /// answers in the detail's, the shelves faint, its own hint.
    #[test]
    fn when_i_stop_puts_call_and_answer_where_the_list_was() {
        let s = sample();
        assert!(s.state.answering());
        assert_eq!(layout_of(&s.state), PHRASE_ANSWER_LAYOUT);
        let (call, panel) = (s.ui.rect_of(CALL).unwrap(), s.ui.rect_of(ANSWER).unwrap());
        assert_eq!(panel.w, 397);
        assert!(call.right() <= panel.x && call.y == panel.y && call.h == panel.h);
        assert!(
            s.ui.get::<crate::phrase_tab::PhraseKinds>(KINDS)
                .unwrap()
                .dim
        );
        assert_eq!(hint_for(&s.state), ANSWER_HINT);
        let c = call_show(&s.state);
        assert_eq!(c.status, "answering");
        assert!(c.status_good);
        assert_eq!(c.lit, Some(2));
        assert_eq!(c.steps[2].1, "bar 1 of 1");
        assert_eq!(c.lanes[0], ("You".to_string(), "1 bar".to_string()));
        assert_eq!(c.lanes[1], ("Answer".to_string(), "home".to_string()));
        let a = answer_show(&s.state);
        assert_eq!((a.mode, a.bars), (1, 0));
        assert_eq!(a.key, "A minor");
        assert_eq!(a.bars_says, "Like mine: as long as you played");
        assert!(!a.can_answer_now, "it is answering already");
        let sliders = answer_sliders(&s.state);
        let texts: Vec<&str> = sliders.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["steady", "like mine", "narrow"]);
    }

    /// Every control a fingertip big and inside its panel; each gives its
    /// action.
    #[test]
    fn the_panel_s_rows_and_buttons_give_answer_s_actions() {
        let mut s = sample();
        let r = s.ui.rect_of(ANSWER).unwrap();
        let w = s.ui.get::<AnswerPanel>(ANSWER).unwrap();
        for t in w.targets(r) {
            assert!(
                t.rect.w >= 44 && t.rect.h >= 40 && r.contains(t.rect.x, t.rect.y),
                "{t:?}"
            );
            assert!(
                t.rect.right() <= r.right() && t.rect.bottom() <= r.bottom(),
                "{t:?}"
            );
        }
        let act = |s: &mut Sample, at: Rect| {
            let ev = tap(&mut s.ui, at)?;
            s.page.action(&ev, &s.state, &mut None)
        };
        assert_eq!(
            act(&mut s, AnswerPanel::mode_place(r, 2)),
            Some(ArpAction::Answer(AnswerAction::Mode(AnswerMode::Trade)))
        );
        assert_eq!(
            act(&mut s, AnswerPanel::for_place(r, 3)),
            Some(ArpAction::Answer(AnswerAction::For(Some(4))))
        );
        assert_eq!(
            act(&mut s, AnswerPanel::button(r, CAPTURE)),
            Some(ArpAction::Answer(AnswerAction::Capture))
        );
        assert_eq!(
            act(&mut s, AnswerPanel::button(r, NOW)),
            None,
            "faint while it answers"
        );
        assert_eq!(
            act(&mut s, AnswerPanel::when_half(r, 0)),
            Some(ArpAction::Improvise(ImproviseAction::Plays(
                PlaysWhen::WhileIHold
            )))
        );
        // A finger on Activity, moved to its right end.
        let w = s.ui.get::<AnswerPanel>(ANSWER).unwrap();
        let (_, track) = w.slider(r, 1);
        let mut qlink = None;
        let ev = s.ui.touch(2, Touch::Down, track.x + 5, track.y).unwrap();
        assert_eq!(s.page.action(&ev, &s.state, &mut qlink), None);
        let ev =
            s.ui.touch(2, Touch::Move, track.right() + 40, track.y)
                .unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut qlink),
            Some(ArpAction::Answer(AnswerAction::Set {
                knob: 1,
                value: 1000
            }))
        );
        s.ui.touch(2, Touch::Up, 0, 0);
    }

    /// In Trade the row is each turn's bars, and the turn light names
    /// turns; while you play, Answer now can be pressed.
    #[test]
    fn trade_names_turns_and_answer_now_waits_for_a_call() {
        let mut s = sample();
        let a = &mut s.state.improvise.answer;
        a.mode = AnswerMode::Trade;
        a.bars = Some(4);
        let show = answer_show(&s.state);
        assert_eq!(show.bars_label, "Turns");
        assert_eq!(show.starts, "when your 4 bars end");
        assert!(!show.can_answer_now, "Trade's turns end on their own");
        assert_eq!(call_show(&s.state).steps[0].1, "4 bars");
        let a = &mut s.state.improvise.answer;
        a.mode = AnswerMode::Answer;
        a.light = Some(Light::You);
        a.answer.clear();
        assert!(answer_show(&s.state).can_answer_now);
        let c = call_show(&s.state);
        assert_eq!((c.status.as_str(), c.lit), ("listening", Some(0)));
    }
}
