//! Sample Recall (sampler R5 part 2; Jay's sampler decision 3, 16 Nov
//! 2026): over any page, the master's last two minutes with their bar
//! lines, a bracket that snaps to whole bars, and what keeps it on a pad.
//! The app keeps the master in a rolling buffer from start (R5 part 1) and
//! opens this with a pad held + SHIFT + REC, or the Rec tab's Skip back on
//! the Master source; PLAY auditions the bracket, REC keeps it, STOP
//! closes it, and the music plays on throughout.
//!
//! This module is the overlay's state ([`Recall`]: the length, where the
//! bracket ends, the fade, the pad it lands on, warp), the bracket worked
//! out from the buffer ([`Recall::bracket`]), and its page: the picture
//! ([`RecallView`]), the panel under it (Length, Lands on, Warp, Audition,
//! Keep) and the Q-Link column (Length, Ends at, Fade, Pad). The design is
//! the review's Recall mockup (mpfree-claude `sampler/rethink/mockups/
//! recall.png`) and `sampler/r5/plan.md`.

use std::cell::Cell;

use mpfree_sampler::record::Input;
use mpfree_ui::qlink::{QLinkCard, QLinkColumn};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Ui, Widget};

use crate::panel::{Control, Item, Look, Panel, Section};
use crate::rec_widgets::{Strip, REC_COLOR};
use crate::wave_tab::{duration_text, pad_label};
use crate::widgets::{Setting, SettingList};

/// The overlay's page, and the widgets it adds (the app's own are
/// `header` and `hint`).
pub const PAGE: &str = "Sample Recall";
pub const VIEW: &str = "smp-recall-view";
pub const LIST: &str = "smp-recall-list";
pub const QLINKS: &str = "smp-recall-qlinks";
/// What its widgets' names start with (inside the Sample tab's
/// [`crate`] prefix "smp-", so the app takes them first).
pub const WIDGETS: &str = "smp-recall-";

/// The picture across the page, the panel under it, the Q-Link column on
/// the right, as the Sample tab's pages have them.
pub const LAYOUT: &str = "col(header:60, row(col(smp-recall-view:*, smp-recall-list:150):*, \
                          smp-recall-qlinks:128):*, hint:26)";

/// The hint line while it is open.
pub const HINT: &str = "Hold a pad + SHIFT + REC opened this on that pad · PLAY: audition · \
                        REC: keep · STOP: close · SHIFT + REC alone still captures notes";

/// What the master keeps, in seconds: the overview spans it.
pub const SECONDS: u32 = 120;

/// The overview's columns.
const COLUMNS: u64 = 600;

/// The longest fade in and out, in ms.
pub const FADE_MAX_MS: u32 = 50;
/// The fade a recall starts with.
pub const FADE_MS: u32 = 5;

/// Free's lengths, in seconds.
pub const FREE_SECONDS: [u32; 6] = [5, 10, 20, 30, 60, 120];

/// What Sample Recall opens on, and what the Sample page's Keep card
/// keeps: the last 4 bars while the music plays, else the last 10 s.
pub const OPEN_BARS: u32 = 4;
pub const OPEN_SECONDS: u32 = 10;

/// How long the bracket is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    /// Whole bars, ending on a bar line.
    Bars(u32),
    /// A number of seconds, ending anywhere (while stopped, or a phrase
    /// that is no whole bars).
    Free(u32),
}

impl Length {
    /// Every length the Q-Link steps through: the bars, then Free's
    /// seconds.
    pub const ALL: [Length; 11] = [
        Length::Bars(1),
        Length::Bars(2),
        Length::Bars(4),
        Length::Bars(8),
        Length::Bars(16),
        Length::Free(5),
        Length::Free(10),
        Length::Free(20),
        Length::Free(30),
        Length::Free(60),
        Length::Free(120),
    ];

    /// The panel's Length buttons: the bars, then Free.
    pub const CHOICES: [&'static str; 6] = ["1", "2", "4", "8", "16", "Free"];

    /// Which of [`Length::CHOICES`] is lit.
    pub fn choice(self) -> usize {
        match self {
            Length::Bars(n) => Length::CHOICES
                .iter()
                .position(|c| *c == n.to_string())
                .unwrap_or(2),
            Length::Free(_) => 5,
        }
    }

    pub fn text(self) -> String {
        match self {
            Length::Bars(1) => "1 bar".to_string(),
            Length::Bars(n) => format!("{n} bars"),
            Length::Free(s) if s >= 60 => format!("Free · {}:{:02}", s / 60, s % 60),
            Length::Free(s) => format!("Free · {s} s"),
        }
    }

    fn index(self) -> usize {
        Length::ALL.iter().position(|l| *l == self).unwrap_or(2)
    }

    /// What the Sample page's Keep card says it keeps: "Keep last 4 bars",
    /// "Keep last bar", "Keep last 10 s", "Keep last 2:00".
    pub fn keep_text(self) -> String {
        match self {
            Length::Bars(1) => "Keep last bar".to_string(),
            Length::Bars(n) => format!("Keep last {n} bars"),
            Length::Free(s) if s >= 60 => format!("Keep last {}:{:02}", s / 60, s % 60),
            Length::Free(s) => format!("Keep last {s} s"),
        }
    }
}

/// The panel's rows, in the list's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Length,
    LandsOn,
    Warp,
    Audition,
    Keep,
}

impl Row {
    pub const ALL: [Row; 5] = [
        Row::Length,
        Row::LandsOn,
        Row::Warp,
        Row::Audition,
        Row::Keep,
    ];

    pub fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Row::Length => "Length",
            Row::LandsOn => "Lands on",
            Row::Warp => "Warp",
            Row::Audition => "Audition",
            Row::Keep => "Keep",
        }
    }
}

/// The span the bracket covers, in the buffer's frames, and the bars it
/// holds (first and last, as the transport numbers them) when it is whole
/// bars of known number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bracket {
    pub start: u64,
    pub end: u64,
    pub bars: Option<(u32, u32)>,
}

impl Bracket {
    pub fn frames(&self) -> u64 {
        self.end - self.start
    }
}

/// What a press on the panel, or a Q-Link turn, asks of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Nothing more than showing the change.
    Nothing,
    /// Hear the bracket (PLAY, or Audition).
    Audition,
    /// Keep the bracket on the pad (REC, or Keep).
    Keep,
}

/// The overlay's settings while it is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recall {
    /// The pad held when it opened.
    pub pad: usize,
    /// The pad it lands on: the held pad when it has no sound, else the
    /// next empty one from it (the app picks); the Pad Q-Link moves it.
    pub onto: usize,
    /// How many pads the kit has, for the Pad Q-Link.
    pub pads: usize,
    pub length: Length,
    /// Where the bracket ends: whole bars back from the latest bar line
    /// (bars), or tenths of a second back from `at` (Free).
    pub back: u32,
    /// The frame it opened at: Free's end, with `back` at 0.
    pub at: u64,
    /// It lands warped at the project tempo (whole bars only).
    pub warp: bool,
    /// The fade in and out, ms.
    pub fade_ms: u32,
}

impl Recall {
    /// Opened on `pad` (landing on `onto`) over `input`: while the music
    /// plays (bar lines come in), the last [`OPEN_BARS`] whole bars,
    /// warped; stopped, the last [`OPEN_SECONDS`] as played.
    pub fn open(input: &Input, pad: usize, onto: usize, pads: usize) -> Recall {
        Recall::last(input, OPEN_BARS, OPEN_SECONDS, pad, onto, pads)
    }

    /// [`Recall::open`] with its lengths: the last `bars` whole bars while
    /// the music plays (as many as have passed, halving, when fewer
    /// have), warped; stopped, the last `seconds` as played. The Sample
    /// page's Keep card keeps this in one tap (one-page step 2c).
    pub fn last(
        input: &Input,
        bars: u32,
        seconds: u32,
        pad: usize,
        onto: usize,
        pads: usize,
    ) -> Recall {
        let playing = input.bars.bar_frames().is_some() && input.bars_back(1).is_some();
        let mut r = Recall {
            pad,
            onto,
            pads: pads.max(1),
            length: if playing {
                Length::Bars(bars.max(1))
            } else {
                Length::Free(seconds.max(1))
            },
            back: 0,
            at: input.now(),
            warp: playing,
            fade_ms: FADE_MS,
        };
        // Fewer bars have passed: as many as there are.
        while r.bracket(input).is_none() {
            match r.length {
                Length::Bars(n) if n > 1 => r.length = Length::Bars(n / 2),
                _ => break,
            }
        }
        r
    }

    /// The bar line the bracket ends on, as [`Input::bars_back`] finds the
    /// latest: the newest one that has come in.
    fn latest(input: &Input) -> Option<usize> {
        let now = input.now();
        (0..mpfree_sampler::record::BAR_LINES)
            .find(|k| input.bars.back(*k).is_some_and(|f| f <= now))
    }

    /// The span the bracket covers in `input` now; None when it reaches
    /// further back than the buffer holds, or before enough bar lines.
    pub fn bracket(&self, input: &Input) -> Option<Bracket> {
        let oldest = input.audio.oldest();
        match self.length {
            Length::Bars(n) => {
                let latest = Recall::latest(input)?;
                let end_k = latest + self.back as usize;
                let (end, end_bar) = input.bars.back_numbered(end_k)?;
                let (start, start_bar) = input.bars.back_numbered(end_k + n as usize)?;
                if start < oldest || end <= start {
                    return None;
                }
                let bars = (start_bar > 0 && end_bar > start_bar).then(|| (start_bar, end_bar - 1));
                Some(Bracket { start, end, bars })
            }
            Length::Free(s) => {
                let rate = u64::from(input.rate);
                let end = self
                    .at
                    .saturating_sub(u64::from(self.back) * rate / 10)
                    .min(input.now());
                let start = end.saturating_sub(u64::from(s) * rate).max(oldest);
                (end > start).then_some(Bracket {
                    start,
                    end,
                    bars: None,
                })
            }
        }
    }

    /// Whether it lands warped: Warp on and whole bars.
    pub fn warps(&self) -> bool {
        self.warp && matches!(self.length, Length::Bars(_))
    }

    /// The line under the overview: "Bars 43 to 46 · 10.4 s · lands warped
    /// at 92 BPM", "The last 10.0 s · lands as played".
    pub fn caption(&self, b: Option<&Bracket>, rate: u32, bpm: f64) -> String {
        let Some(b) = b else {
            return match self.length {
                Length::Bars(_) => "Not enough bars kept yet: pick fewer, or Free".to_string(),
                Length::Free(_) => "Nothing kept yet".to_string(),
            };
        };
        let span = match (self.length, b.bars) {
            (Length::Bars(_), Some((a, z))) if a == z => format!("Bar {a}"),
            (Length::Bars(_), Some((a, z))) => format!("Bars {a} to {z}"),
            (Length::Bars(n), None) => Length::Bars(n).text(),
            (Length::Free(_), _) if self.back == 0 => "The last".to_string(),
            (Length::Free(_), _) => format!("Ending {:.1} s back", f64::from(self.back) / 10.0),
        };
        let lands = if self.warps() {
            format!("lands warped at {bpm:.0} BPM")
        } else {
            "lands as played".to_string()
        };
        format!("{span} · {} · {lands}", duration_text(b.frames(), rate))
    }

    /// A step of `row` by `delta` (a tap on a choice is a step from the
    /// one lit to the one tapped), or a press of a button.
    pub fn step(&mut self, row: Row, delta: i32, input: &Input) -> Ask {
        match row {
            Row::Length => {
                let at = (self.length.choice() as i32 + delta).clamp(0, 5) as usize;
                self.length = if at == 5 {
                    match self.length {
                        Length::Free(s) => Length::Free(s),
                        _ => Length::Free(10),
                    }
                } else {
                    Length::Bars(Length::CHOICES[at].parse().unwrap_or(4))
                };
                self.back = 0;
                self.at = self.at.max(input.now());
            }
            Row::LandsOn => self.onto = step_pad(self.onto, delta, self.pads),
            Row::Warp => self.warp = !self.warp,
            Row::Audition => return Ask::Audition,
            Row::Keep => return Ask::Keep,
        }
        Ask::Nothing
    }

    /// Q-Link `k` turned `ticks`: Length, Ends at, Fade, Pad.
    pub fn qlink(&mut self, k: usize, ticks: i32, input: &Input) {
        match k {
            0 => {
                let at = (self.length.index() as i32 + ticks.signum())
                    .clamp(0, Length::ALL.len() as i32 - 1) as usize;
                self.length = Length::ALL[at];
                self.back = 0;
                self.at = self.at.max(input.now());
            }
            1 => {
                // Right is later: towards now.
                let back = (self.back as i32 - ticks).max(0) as u32;
                let was = self.back;
                self.back = back;
                if back > was && self.bracket(input).is_none() {
                    // No further back than the buffer holds.
                    self.back = was;
                }
            }
            2 => self.fade_ms = (self.fade_ms as i32 + ticks).clamp(0, FADE_MAX_MS as i32) as u32,
            3 => self.onto = step_pad(self.onto, ticks, self.pads),
            _ => {}
        }
    }

    /// Where the bracket ends, for its Q-Link card: "bar 46", "now",
    /// "1.5 s back".
    pub fn ends_text(&self, b: Option<&Bracket>) -> String {
        match (self.length, b.and_then(|b| b.bars)) {
            (Length::Bars(_), Some((_, z))) => format!("bar {z}"),
            (Length::Bars(_), None) if self.back == 0 => "last bar".to_string(),
            (Length::Bars(_), None) => format!("{} bars back", self.back),
            (Length::Free(_), _) if self.back == 0 => "now".to_string(),
            (Length::Free(_), _) => format!("{:.1} s back", f64::from(self.back) / 10.0),
        }
    }
}

fn step_pad(pad: usize, delta: i32, pads: usize) -> usize {
    (pad as i64 + i64::from(delta)).clamp(0, pads as i64 - 1) as usize
}

// ---------------------------------------------------------------- picture

/// The overlay's picture: the title and the REC chip, the two minutes with
/// their bar lines and the bracket, the bracket's own line, the bracket
/// drawn large with its bars, and what keeping does.
pub struct RecallView {
    /// The master's last two minutes, read as they come.
    pub strip: Strip,
    pub now: u64,
    /// Bar lines in the two minutes: frame and bar number (0 unknown).
    pub bars: Vec<(u64, u32)>,
    pub bracket: Option<Bracket>,
    /// "REC → Pad A6 · Master, effects on".
    pub chip: String,
    /// "Bars 43 to 46 · 10.4 s · lands warped at 92 BPM".
    pub caption: String,
    /// The bracket's peaks, a column each: (lowest, highest) of both sides.
    pub zoom: Vec<(f32, f32)>,
    /// The bar lines in the bracket: where (0 to 1) and their numbers.
    pub zoom_bars: Vec<(f32, u32)>,
    /// What keeping does: "Lands trimmed on A6, ready to chop...".
    pub note: String,
    /// The look last painted (the app redraws the picture only on a
    /// change, as the Rec tab's).
    painted: Cell<Option<u64>>,
}

impl RecallView {
    /// Two minutes at `rate`.
    pub fn new(rate: u32) -> RecallView {
        let span = u64::from(SECONDS) * u64::from(rate.max(1));
        RecallView {
            strip: Strip::new(span / COLUMNS, COLUMNS as usize),
            now: 0,
            bars: Vec::new(),
            bracket: None,
            chip: String::new(),
            caption: String::new(),
            zoom: Vec::new(),
            zoom_bars: Vec::new(),
            note: String::new(),
            painted: Cell::new(None),
        }
    }

    /// Reads what came in since the last update and the bar lines kept.
    pub fn update(&mut self, input: &Input) {
        self.strip.update(input);
        self.now = input.now();
        let first = self.first();
        self.bars = (0..mpfree_sampler::record::BAR_LINES)
            .map_while(|k| input.bars.back_numbered(k))
            .take_while(|(f, _)| *f >= first)
            .filter(|(f, _)| *f <= self.now)
            .collect();
        self.bars.reverse();
    }

    /// The bracket drawn large: `cols` columns of `b` read from `input`
    /// (only when the bracket changes), with its bar lines.
    pub fn zoom_in(&mut self, input: &Input, b: Option<Bracket>, cols: usize) {
        self.bracket = b;
        self.zoom.clear();
        self.zoom_bars.clear();
        let Some(b) = b else {
            return;
        };
        let frames = b.frames().max(1);
        let mut scratch = Vec::new();
        for c in 0..cols as u64 {
            let s = b.start + frames * c / cols as u64;
            let e = (b.start + frames * (c + 1) / cols as u64).max(s + 1);
            scratch.clear();
            if input.audio.copy(s, e.min(b.end), &mut scratch).is_err() {
                self.zoom.push((0.0, 0.0));
                continue;
            }
            let lo = scratch.iter().copied().fold(0.0f32, f32::min);
            let hi = scratch.iter().copied().fold(0.0f32, f32::max);
            self.zoom.push((lo, hi));
        }
        self.zoom_bars = self
            .bars
            .iter()
            .filter(|(f, _)| *f >= b.start && *f < b.end)
            .map(|&(f, n)| ((f - b.start) as f32 / frames as f32, n))
            .collect();
    }

    fn edge(&self) -> u64 {
        let per = self.strip.per();
        self.now / per * per
    }

    fn first(&self) -> u64 {
        self.edge().saturating_sub(self.strip.span())
    }

    fn x_of(&self, r: Rect, f: u64) -> i32 {
        let span = self.strip.span().max(1);
        let back = self
            .edge()
            .saturating_sub(f.clamp(self.first(), self.edge()));
        r.right() - (back as f64 / span as f64 * f64::from(r.w)).round() as i32
    }

    /// The areas: the title line, the overview, its numbers, the caption,
    /// the zoom, the note.
    pub fn areas(r: Rect) -> [Rect; 6] {
        let inner = r.inset(16);
        let (title, rest) = inner.take_top(30);
        let (overview, rest) = rest.take_top(10).1.take_top(72);
        let (numbers, rest) = rest.take_top(20);
        let (caption, rest) = rest.take_top(28);
        let (note, zoom) = rest.take_bottom(30);
        [
            title,
            overview,
            numbers,
            caption,
            zoom.take_bottom(6).1,
            note,
        ]
    }

    /// What the picture depends on, as one number: the app redraws it only
    /// when it changes.
    pub fn look(&self, r: Rect) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        (&self.chip, &self.caption, &self.note).hash(&mut h);
        let [_, overview, ..] = RecallView::areas(r);
        self.edge().hash(&mut h);
        for &(f, n) in &self.bars {
            (self.x_of(overview, f), n).hash(&mut h);
        }
        self.bracket.hash(&mut h);
        self.zoom.len().hash(&mut h);
        h.finish()
    }

    pub fn painted(&self) -> Option<u64> {
        self.painted.get()
    }
}

impl Widget for RecallView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        self.painted.set(Some(self.look(r)));
        cx.panel(r);
        let [title, overview, numbers, caption, zoom, note] = RecallView::areas(r);
        let rec = Color::rgb(REC_COLOR);
        let small = t.size_small;
        // The title, and the REC chip on the right.
        let w = cx.text(
            FontId::Bold,
            small,
            title,
            Align::Left,
            t.text,
            "SAMPLE RECALL",
        );
        let sub = Rect::new(title.x + w + 16, title.y, title.w - w - 16, title.h);
        cx.text(
            FontId::Sans,
            small,
            sub,
            Align::Left,
            t.text_dim,
            "the master's last 2:00, always kept",
        );
        if !self.chip.is_empty() {
            let cw = cx.measure(FontId::Bold, small, &self.chip) + 28;
            let chip = Rect::new(title.right() - cw, title.y - 2, cw, title.h + 4);
            cx.canvas.fill_round_rect(chip, 8, rec.with_alpha(40));
            cx.canvas.stroke_round_rect(chip, 8, 2, rec);
            cx.text(FontId::Bold, small, chip, Align::Center, t.text, &self.chip);
        }
        // The two minutes: each column's reach, the bar lines, the bracket.
        cx.canvas.fill_round_rect(overview, 6, t.bg);
        let mid = overview.y + overview.h / 2;
        let half = (overview.h / 2 - 3).max(1) as f32;
        let per = self.strip.per();
        let first = self.first();
        for k in 0..COLUMNS {
            let index = first / per + k;
            let Some(c) = self.strip.column(index) else {
                continue;
            };
            let hi = c[1].max(c[3]).clamp(0.0, 1.0);
            let lo = c[0].min(c[2]).clamp(-1.0, 0.0);
            let x0 = self.x_of(overview, index * per);
            let x1 = self.x_of(overview, (index + 1) * per).max(x0 + 1);
            let inside = self
                .bracket
                .is_some_and(|b| index * per + per > b.start && index * per < b.end);
            let col = if inside { t.accent } else { t.text_faint };
            let up = (hi * half) as i32;
            let down = (-lo * half) as i32;
            cx.canvas
                .fill_rect(Rect::new(x0, mid - up, x1 - x0, up + down + 1), col);
        }
        for (i, &(f, n)) in self.bars.iter().enumerate() {
            let x = self.x_of(overview, f);
            let tall = n % 8 == 1 || (n == 0 && i % 8 == 0);
            let h = if tall { overview.h } else { 8 };
            cx.canvas
                .fill_rect(Rect::new(x, overview.y, 1, h), t.outline);
            if tall && n > 0 && x + 30 < numbers.right() - 40 {
                cx.text(
                    FontId::Sans,
                    small - 2.0,
                    Rect::new(x, numbers.y, 40, numbers.h),
                    Align::Left,
                    t.text_dim,
                    &n.to_string(),
                );
            }
        }
        if let Some(b) = self.bracket {
            let (x0, x1) = (self.x_of(overview, b.start), self.x_of(overview, b.end));
            let br = Rect::new(x0, overview.y, (x1 - x0).max(2), overview.h);
            cx.canvas.fill_rect(br, t.accent.with_alpha(40));
            cx.canvas.stroke_round_rect(br, 2, 2, t.accent);
        }
        cx.text(FontId::Bold, small - 2.0, numbers, Align::Right, rec, "now");
        cx.text(
            FontId::Sans,
            t.size_body,
            caption,
            Align::Left,
            t.text,
            &self.caption,
        );
        // The bracket drawn large.
        cx.canvas.fill_round_rect(zoom, 6, t.bg);
        if !self.zoom.is_empty() {
            let mid = zoom.y + zoom.h / 2;
            let half = (zoom.h / 2 - 8).max(1) as f32;
            let n = self.zoom.len() as i32;
            for (i, &(lo, hi)) in self.zoom.iter().enumerate() {
                let x0 = zoom.x + zoom.w * i as i32 / n;
                let x1 = (zoom.x + zoom.w * (i as i32 + 1) / n).max(x0 + 1);
                let up = (hi.clamp(0.0, 1.0) * half) as i32;
                let down = (-lo.clamp(-1.0, 0.0) * half) as i32;
                cx.canvas
                    .fill_rect(Rect::new(x0, mid - up, x1 - x0, up + down + 1), t.accent);
            }
            cx.canvas
                .fill_rect(Rect::new(zoom.x, mid, zoom.w, 1), t.outline);
            for &(at, num) in &self.zoom_bars {
                let x = zoom.x + (at * zoom.w as f32) as i32;
                cx.canvas
                    .fill_rect(Rect::new(x, zoom.y, 1, zoom.h), t.outline);
                if num > 0 {
                    cx.text(
                        FontId::Sans,
                        small - 2.0,
                        Rect::new(x + 6, zoom.bottom() - 22, 60, 18),
                        Align::Left,
                        t.text_dim,
                        &num.to_string(),
                    );
                }
            }
        }
        cx.text(
            FontId::Sans,
            small,
            note,
            Align::Left,
            t.text_dim,
            &self.note,
        );
    }
}

// ---------------------------------------------------------------- page

/// The overlay's widgets and state, while the app has it.
pub struct RecallPage {
    pub recall: Recall,
    /// The project tempo, for the caption.
    pub bpm: f64,
    /// Which of the kit's pads have a sound (when it opened).
    pub filled: Vec<bool>,
    /// The pad it first picked to land on: "next free".
    next_free: usize,
    /// The bracket last shown.
    zoomed: Option<Option<Bracket>>,
    /// Seconds since the picture was last redrawn, or the bracket's move
    /// last shown ([`RecallPage::tick`]).
    since_drawn: f32,
}

/// The least time between two redraws of the picture while it only
/// follows the music (as the Rec tab's [`crate::rec_tab::LIVE_GAP_S`]):
/// the overview moves a column every 0.2 s.
pub const DRAW_GAP_S: f32 = 0.1;

/// The columns the bracket is drawn large in.
const ZOOM_COLUMNS: usize = 560;

impl RecallPage {
    /// Adds the widgets and the page to `ui` once (the app adds `header`
    /// and `hint` first). Nothing shows until [`RecallPage::open`].
    pub fn add(ui: &mut Ui, rate: u32) -> Result<(), String> {
        if ui.get::<RecallView>(VIEW).is_some() {
            return Ok(());
        }
        ui.add(VIEW, RecallView::new(rate));
        let mut list = SettingList::new("Sample Recall");
        list.buttons = vec![Row::Audition.index(), Row::Keep.index()];
        list.selected = Row::Length.index();
        ui.add(LIST, list);
        ui.add(QLINKS, QLinkColumn::new());
        ui.add_page(PAGE, LAYOUT)
    }

    /// Opened over `input`: `recall` as [`Recall::open`] set it, at `bpm`,
    /// with `filled` saying which pads have a sound.
    pub fn open(
        ui: &mut Ui,
        input: &Input,
        recall: Recall,
        bpm: f64,
        filled: Vec<bool>,
    ) -> RecallPage {
        if let Some(v) = ui.get_mut::<RecallView>(VIEW) {
            v.strip.reset();
            v.update(input);
        }
        let mut p = RecallPage {
            recall,
            bpm,
            filled,
            next_free: recall.onto,
            zoomed: None,
            since_drawn: 0.0,
        };
        p.show(ui, input);
        p
    }

    /// The pad named, as "A6".
    pub fn pad_text(pad: usize) -> String {
        pad_label(pad)
    }

    /// Reads what came in (every frame while open, `dt` seconds apart).
    /// The picture is redrawn only when it differs from the one on the
    /// screen ([`RecallView::look`]) and at most every [`DRAW_GAP_S`]; the
    /// panel and the cards only when the bracket moves (a new bar line).
    pub fn tick(&mut self, ui: &mut Ui, input: &Input, dt: f32) {
        self.since_drawn += dt;
        if self.since_drawn < DRAW_GAP_S {
            return;
        }
        if let Some(v) = ui.get_mut_quiet::<RecallView>(VIEW) {
            v.update(input);
        }
        if self.zoomed != Some(self.recall.bracket(input)) {
            self.show(ui, input);
            self.since_drawn = 0.0;
            return;
        }
        let area = ui.rect_of(VIEW);
        let stale = ui
            .get::<RecallView>(VIEW)
            .zip(area)
            .is_some_and(|(v, r)| v.painted() != Some(v.look(r)));
        if stale {
            ui.get_mut::<RecallView>(VIEW);
            self.since_drawn = 0.0;
        }
    }

    /// Shows the settings, the bracket and the cards.
    pub fn show(&mut self, ui: &mut Ui, input: &Input) {
        let r = self.recall;
        let b = r.bracket(input);
        let onto = pad_label(r.onto);
        let caption = r.caption(b.as_ref(), input.rate, self.bpm);
        let note = if self.replaces() {
            format!("Replaces {onto}'s sound, trimmed: UNDO takes it back. SAMPLE EDIT opens it.")
        } else {
            format!("Lands trimmed on {onto}, ready to chop: SAMPLE EDIT opens it.")
        };
        if let Some(v) = ui.get_mut::<RecallView>(VIEW) {
            v.chip = format!("REC → Pad {onto} · Master, effects on");
            v.caption = caption;
            v.note = note;
            if self.zoomed != Some(b) {
                v.zoom_in(input, b, ZOOM_COLUMNS);
            }
        }
        self.zoomed = Some(b);
        let value = |row: Row| match row {
            Row::Length => r.length.text(),
            Row::LandsOn => {
                if self.replaces() {
                    format!("Pad {onto} · replaces its sound")
                } else if r.onto == r.pad {
                    format!("Pad {onto}")
                } else if r.onto == self.next_free {
                    format!("Pad {onto} · next free")
                } else {
                    format!("Pad {onto} · empty")
                }
            }
            Row::Warp => if r.warp { "On" } else { "Off" }.to_string(),
            Row::Audition => "Audition".to_string(),
            Row::Keep => "Keep".to_string(),
        };
        let bars = matches!(r.length, Length::Bars(_));
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = Row::ALL
                .iter()
                .map(|row| Setting {
                    name: row.label().into(),
                    value: value(*row),
                })
                .collect();
            list.controls = vec![
                Control::new(Look::Choice {
                    options: Length::CHOICES.iter().map(|c| c.to_string()).collect(),
                    on: r.length.choice(),
                    off: Vec::new(),
                }),
                Control::new(Look::Chip),
                Control::new(Look::Choice {
                    options: vec!["On".into(), "Off".into()],
                    on: usize::from(!r.warp),
                    off: Vec::new(),
                })
                .faint_because(!bars, "Free lands as played: warp is for whole bars"),
                Control::new(Look::Button("Audition".into())).faint_if(b.is_none()),
                Control::new(Look::Primary("Keep".into())).faint_if(b.is_none()),
            ];
            let length_note = if bars {
                "bars, ending on the last bar line"
            } else {
                "seconds, ending where Ends at says"
            };
            list.panel = Some(Panel {
                sections: vec![
                    Section::new("Length", 330, vec![vec![Item::new(Row::Length.index())]])
                        .note(length_note),
                    Section::new("Lands on", 200, vec![vec![Item::new(Row::LandsOn.index())]]),
                    Section::new("Warp", 130, vec![vec![Item::new(Row::Warp.index())]]),
                    Section::new(
                        "",
                        260,
                        vec![vec![
                            Item::new(Row::Audition.index()),
                            Item::new(Row::Keep.index()),
                        ]],
                    ),
                ],
            });
        }
        if let Some(q) = ui.get_mut::<QLinkColumn>(QLINKS) {
            let place = |i: usize, n: usize| i as f32 / (n.max(2) - 1) as f32;
            q.title = "RECALL".into();
            q.cards = [
                QLinkCard::new(
                    "Length",
                    "RECALL",
                    &r.length.text(),
                    place(r.length.index(), Length::ALL.len()),
                ),
                QLinkCard::new("Ends at", "RECALL", &r.ends_text(b.as_ref()), 1.0),
                QLinkCard::new(
                    "Fade",
                    "RECALL",
                    &format!("{} ms", r.fade_ms),
                    r.fade_ms as f32 / FADE_MAX_MS as f32,
                ),
                QLinkCard::new("Pad", "RECALL", &onto, place(r.onto, r.pads)),
            ];
        }
    }

    /// Keeping replaces the sound on the pad it lands on.
    pub fn replaces(&self) -> bool {
        self.filled.get(self.recall.onto).copied().unwrap_or(false)
    }

    /// A step or a press on the panel's row `row`.
    pub fn step(&mut self, ui: &mut Ui, input: &Input, row: usize, delta: i32) -> Ask {
        let Some(&row) = Row::ALL.get(row) else {
            return Ask::Nothing;
        };
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.selected = row.index();
        }
        let ask = self.recall.step(row, delta, input);
        self.show(ui, input);
        ask
    }

    /// Q-Link `k` turned `ticks`.
    pub fn qlink(&mut self, ui: &mut Ui, input: &Input, k: usize, ticks: i32) {
        self.recall.qlink(k, ticks, input);
        self.show(ui, input);
    }

    /// The row the wheel picks, and steps.
    pub fn wheel(&mut self, ui: &mut Ui, input: &Input, delta: i32, editing: bool) -> Ask {
        let selected = ui.get::<SettingList>(LIST).map_or(0, |l| l.selected);
        if editing {
            return self.step(ui, input, selected, delta);
        }
        let at = (selected as i32 + delta).clamp(0, Row::ALL.len() as i32 - 1) as usize;
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.selected = at;
        }
        Ask::Nothing
    }
}
