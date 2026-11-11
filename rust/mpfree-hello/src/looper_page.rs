//! The Looper tab: the loop's waveform with its bar lines and playhead, the
//! looper's buttons, the meter of what it records (the pads or the line
//! input) and the looper's state.
//!
//! The buttons tap as `Tab(i)` with `i` one of [`BUTTONS`]; the app turns
//! them into looper commands (`App::looper_tap`). REC, OVERDUB and UNDO on
//! the MPC do the same while this tab is showing.

use crate::app::Settings;
use crate::looping::{bars_text, LoopSource, LoopWave, LooperView};
use mpfree_arp_ui::widgets::{Button, ButtonRow};
use mpfree_looper::Phase;
use mpfree_ui::draw;
use mpfree_ui::wave::{Beats, Waveform};
use mpfree_ui::widgets::StatList;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Tone, Touch, Ui, Widget};
use std::cell::RefCell;
use std::sync::Arc;

pub const LOOPER_PAGE: &str = "Looper";
pub const LOOPER_LAYOUT: &str =
    "col(header:60, row(col(lwave:*, lbuttons:72):*, col(lmeter:150, lstatus:*):430):*, hint:26)";
const WAVE: &str = "lwave";
const BUTTONS_ID: &str = "lbuttons";
const METER: &str = "lmeter";
const STATUS: &str = "lstatus";

/// The buttons, in tap order.
pub const BUTTONS: [&str; 7] = [
    "Record", "Overdub", "Undo", "Redo", "Clear", "Mute", "Monitor",
];
pub const B_RECORD: usize = 0;
pub const B_OVERDUB: usize = 1;
pub const B_UNDO: usize = 2;
pub const B_REDO: usize = 3;
pub const B_CLEAR: usize = 4;
pub const B_MUTE: usize = 5;
pub const B_MONITOR: usize = 6;

/// The meter's scale: -60 to 0 dBFS.
const FLOOR_DB: f32 = -60.0;

/// The level of what the looper records: one bar per channel, the peak
/// since the last update and a held peak, marked red near full scale.
#[derive(Default)]
pub struct Meter {
    /// The source's name.
    pub title: String,
    pub on: bool,
    pub db: [f32; 2],
    pub hold: [f32; 2],
    pub note: String,
}

impl Meter {
    fn fraction(db: f32) -> f32 {
        if !db.is_finite() {
            return 0.0;
        }
        ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
    }
}

impl Widget for Meter {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let right = if self.on {
            let top = self.hold[0].max(self.hold[1]);
            if top.is_finite() {
                format!("peak {top:.1} dBFS")
            } else {
                "silent".to_string()
            }
        } else {
            "off".to_string()
        };
        let body = cx.titled_panel(r, &self.title, &right);
        if body.h < 20 {
            return;
        }
        let lane_h = ((body.h - 30) / 2).clamp(8, 26);
        for ch in 0..2 {
            let y = body.y + 4 + ch as i32 * (lane_h + 8);
            let label = Rect::new(body.x, y, 22, lane_h);
            cx.text(
                FontId::Bold,
                t.size_small,
                label,
                Align::Left,
                t.text_dim,
                if ch == 0 { "L" } else { "R" },
            );
            let lane = Rect::new(body.x + 24, y, (body.w - 24).max(1), lane_h);
            cx.canvas.fill_round_rect(lane, 4, t.raised);
            if !self.on {
                continue;
            }
            let f = Meter::fraction(self.db[ch]);
            let w = (lane.w as f32 * f).round() as i32;
            let hot = self.db[ch] > -1.0;
            let tone = if hot {
                t.bad
            } else if self.db[ch] > -12.0 {
                t.warn
            } else {
                t.good
            };
            if w > 0 {
                cx.canvas
                    .fill_round_rect(Rect::new(lane.x, lane.y, w, lane.h), 4, tone);
            }
            let h = (lane.w as f32 * Meter::fraction(self.hold[ch])).round() as i32;
            if h > 0 {
                cx.canvas
                    .fill_rect(Rect::new(lane.x + h - 2, lane.y, 2, lane.h), t.text);
            }
        }
        let foot = Rect::new(body.x, body.bottom() - 20, body.w, 20);
        let note = if self.on {
            self.note.as_str()
        } else {
            "Turn it on in Prefs or MENU (Line input)"
        };
        cx.text(
            FontId::Sans,
            t.size_small,
            foot,
            Align::Left,
            t.text_dim,
            note,
        );
    }
}

/// The Play page's looper panel (the All and Loop views), as mockups F and
/// H draw it: "LOOPER" with a chip per layer, the length on the right, and
/// under them the loop with each layer's peaks stacked in its colour (full
/// behind the playhead, dimmer ahead), the bar lines and numbers, the
/// playhead and the state in a pill. When the panel is tall (the Loop
/// view) a row of the looper's buttons sits under the loop; a tap on one
/// gives `Tab(i)` for [`STRIP_BUTTONS`]`[i]`. The Looper tab is where the
/// loop is edited; this is where it is played.
pub struct LoopStrip {
    pub wave: Option<Arc<LoopWave>>,
    /// The heading's right-hand text: "4 bars · 3 layers".
    pub right: String,
    /// The pill: "PLAYING", "OVERDUB"; None while empty.
    pub state: Option<(String, Tone)>,
    /// Shown in the loop's place while it is empty.
    pub note: String,
    /// The playhead, 0 to 1 along the loop.
    pub position: Option<f32>,
    /// Bars in the loop, for the bar lines.
    pub bars: usize,
    pub layers: usize,
    /// The buttons' labels and whether each is lit (the Loop view).
    pub buttons: Vec<(String, bool)>,
    /// The playhead's x and everything else as last drawn, so a playhead
    /// move redraws only the columns it passed.
    drawn: RefCell<Option<(Rect, StripLook, i32)>>,
}

/// What the strip shows apart from the playhead.
#[derive(Clone, Debug, PartialEq)]
struct StripLook {
    right: String,
    state: Option<(String, Tone)>,
    note: String,
    bars: usize,
    layers: usize,
    buttons: Vec<(String, bool)>,
    wave: Option<u64>,
}

/// The strip's widget name.
pub const LOOP_STRIP: &str = "play-loop";
/// The buttons under the loop in the Loop view, in tap order. "Stop" stops
/// the transport; the others are the Looper tab's ([`BUTTONS`]).
pub const STRIP_BUTTONS: [&str; 7] = ["Record", "Overdub", "Stop", "Undo", "Redo", "Mute", "Clear"];
/// The Looper tab's button for each of [`STRIP_BUTTONS`] (None: Stop).
pub const STRIP_TO_BUTTON: [Option<usize>; 7] = [
    Some(B_RECORD),
    Some(B_OVERDUB),
    None,
    Some(B_UNDO),
    Some(B_REDO),
    Some(B_MUTE),
    Some(B_CLEAR),
];
/// The layers' colours, layer 1 first.
pub const LAYER_COLOURS: [Color; 8] = [
    draw::KNOB[0],
    draw::KNOB[1],
    draw::CHOP,
    draw::KNOB[2],
    draw::VOX,
    draw::PERC,
    draw::KICK,
    draw::FX,
];
/// Panels at least this tall show the buttons.
const BUTTONS_FROM_H: i32 = 300;

impl Default for LoopStrip {
    fn default() -> Self {
        LoopStrip {
            wave: None,
            right: String::new(),
            state: None,
            note: String::new(),
            position: None,
            bars: 0,
            layers: 0,
            buttons: STRIP_BUTTONS
                .iter()
                .map(|b| (b.to_string(), false))
                .collect(),
            drawn: RefCell::new(None),
        }
    }
}

impl LoopStrip {
    /// The loop's panel and the buttons' row in the strip's rectangle.
    fn parts(r: Rect) -> (Rect, Option<Rect>) {
        let (_, body) = draw::card_head(r);
        if r.h >= BUTTONS_FROM_H {
            let (btns, wave) = body.take_bottom(44);
            (wave.take_bottom(8).1, Some(btns))
        } else {
            (body, None)
        }
    }

    /// The loop's drawing area inside its panel.
    fn wave_area(r: Rect) -> Rect {
        LoopStrip::parts(r).0.inset_xy(10, 8)
    }

    fn button_rects(&self, row: Rect) -> Vec<Rect> {
        let n = self.buttons.len().max(1) as i32;
        let gap = 8;
        let bw = (row.w - gap * (n - 1)) / n;
        (0..n)
            .map(|i| Rect::new(row.x + i * (bw + gap), row.y, bw, row.h))
            .collect()
    }

    fn look(&self) -> StripLook {
        StripLook {
            right: self.right.clone(),
            state: self.state.clone(),
            note: self.note.clone(),
            bars: self.bars,
            layers: self.layers,
            buttons: self.buttons.clone(),
            wave: self.wave.as_ref().map(|w| Arc::as_ptr(w) as usize as u64),
        }
    }

    fn play_x(&self, w: Rect) -> Option<i32> {
        self.position
            .filter(|_| self.wave.as_ref().is_some_and(|w| w.frames > 0))
            .map(|p| w.x + (p.clamp(0.0, 1.0) * w.w as f32) as i32)
    }
}

impl Widget for LoopStrip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        draw::card(cx, r, "Looper", &self.right);
        let (panel, btns) = LoopStrip::parts(r);
        cx.canvas
            .fill_round_rect(panel, 10, t.bg.mix(t.surface, 0.4));
        let w = LoopStrip::wave_area(r);
        let play = self.play_x(w);
        let mid = w.y + w.h / 2;
        let half = w.h as f32 / 2.0 - 2.0;
        match self.wave.as_ref().filter(|wv| wv.frames > 0) {
            Some(wave) => {
                let cols = crate::looping::WAVE_COLUMNS;
                let mut x = 0;
                while x < w.w {
                    let col = (x as usize * cols / w.w.max(1) as usize).min(cols - 1);
                    let past = play.is_some_and(|p| w.x + x < p);
                    let mut acc = 0.0f32;
                    for (li, layer) in wave.layers.iter().enumerate() {
                        let amp = layer.get(col).copied().unwrap_or(0.0);
                        if amp <= 0.001 {
                            continue;
                        }
                        let y0 = (acc * half) as i32;
                        acc += amp;
                        let y1 = (acc * half).min(half) as i32;
                        let c = LAYER_COLOURS[li % LAYER_COLOURS.len()];
                        let c = if past { c } else { c.mix(t.surface, 0.35) };
                        if y1 > y0 {
                            cx.canvas
                                .fill_rect(Rect::new(w.x + x, mid - y1, 2, y1 - y0), c);
                            cx.canvas
                                .fill_rect(Rect::new(w.x + x, mid + y0, 2, y1 - y0), c);
                        }
                    }
                    x += 3;
                }
                // Bar lines and numbers.
                let bars = self.bars.max(1);
                for b in 0..bars {
                    let bx = w.x + (b as f32 / bars as f32 * w.w as f32) as i32;
                    if b > 0 {
                        cx.canvas.fill_rect(Rect::new(bx, w.y, 1, w.h), t.outline);
                    }
                    cx.text(
                        FontId::Bold,
                        11.0,
                        Rect::new(bx + 5, w.y, 24, 14),
                        Align::Left,
                        t.text_faint,
                        &format!("{}", b + 1),
                    );
                }
                if let Some(px) = play {
                    cx.canvas
                        .fill_rect(Rect::new(px - 1, panel.y + 2, 2, panel.h - 4), t.text);
                }
            }
            None => {
                cx.text(
                    FontId::Sans,
                    14.0,
                    panel,
                    Align::Center,
                    t.text_dim,
                    &self.note,
                );
            }
        }
        // The state, top right of the loop.
        if let Some((state, tone)) = &self.state {
            let sw = draw::pill_width(cx, state, true);
            let c = match tone {
                Tone::Dim | Tone::Normal => t.text_faint,
                k => t.tone(*k),
            };
            draw::pill(cx, panel.right() - sw - 10, panel.y + 8, 26, state, c, true);
        }
        // The layers, beside the title.
        let mut lx = r.x + 12 + cx.measure(FontId::Bold, draw::HEAD_PX, "LOOPER") + 24;
        let shown = self.layers.min(4);
        for li in 0..shown {
            let name = format!("Layer {}", li + 1);
            let tw = cx.measure(FontId::Sans, draw::HEAD_PX, &name);
            draw::dot(cx, lx + 4, r.y + 19, 4, LAYER_COLOURS[li]);
            cx.text(
                FontId::Sans,
                draw::HEAD_PX,
                Rect::new(lx + 12, r.y + 10, tw + 4, 18),
                Align::Left,
                t.text_dim,
                &name,
            );
            lx += tw + 30;
        }
        if self.layers > shown {
            cx.text(
                FontId::Sans,
                draw::HEAD_PX,
                Rect::new(lx, r.y + 10, 60, 18),
                Align::Left,
                t.text_faint,
                &format!("+{}", self.layers - shown),
            );
        }
        if let Some(row) = btns {
            for (br, (label, on)) in self.button_rects(row).iter().zip(&self.buttons) {
                let lit = if label == "Overdub" || label == "Stop dub" {
                    t.warn
                } else if label == "Mute" || label == "Unmute" {
                    t.text_dim
                } else {
                    t.bad
                };
                cx.canvas
                    .fill_round_rect(*br, 10, if *on { lit } else { t.raised });
                let ink = if *on {
                    t.bg
                } else if label == "Clear" {
                    t.bad
                } else {
                    t.text
                };
                cx.text(FontId::Bold, 14.0, *br, Align::Center, ink, label);
            }
        }
        *self.drawn.borrow_mut() = Some((r, self.look(), play.unwrap_or(-1)));
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, look, old_x) = drawn.as_ref()?;
        if *at != r || *look != self.look() {
            return None;
        }
        let w = LoopStrip::wave_area(r);
        let (panel, _) = LoopStrip::parts(r);
        let new_x = self.play_x(w).unwrap_or(-1);
        if new_x == *old_x {
            return Some(Vec::new());
        }
        if *old_x < 0 || new_x < 0 || new_x < *old_x {
            // Round the loop's end: the columns behind the old place
            // turn dim again.
            return Some(vec![panel]);
        }
        Some(vec![Rect::new(
            old_x - 4,
            panel.y,
            new_x - old_x + 8,
            panel.h,
        )])
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let row = LoopStrip::parts(r).1?;
        self.button_rects(row)
            .iter()
            .position(|b| b.contains(x, y))
            .map(EventKind::Tab)
    }
}

pub struct LooperPage {
    /// The frames of the wave shown, to know when the bar lines change.
    frames: u64,
    hold: [f32; 2],
    hold_age: u32,
}

impl LooperPage {
    pub fn add(ui: &mut Ui) -> Result<LooperPage, String> {
        ui.add(WAVE, Waveform::new());
        let mut row = ButtonRow::one_line(&BUTTONS.map(|b| (b, Tone::Normal)));
        row.buttons[B_RECORD].tone = Tone::Bad;
        row.buttons[B_OVERDUB].tone = Tone::Warn;
        ui.add(BUTTONS_ID, row);
        ui.add(
            METER,
            Meter {
                title: LoopSource::Pads.name().to_string(),
                ..Meter::default()
            },
        );
        ui.add(STATUS, StatList::new("Looper"));
        ui.add(LOOP_STRIP, LoopStrip::default());
        ui.add_page(LOOPER_PAGE, LOOPER_LAYOUT)?;
        let mut p = LooperPage {
            frames: 0,
            hold: [f32::NEG_INFINITY; 2],
            hold_age: 0,
        };
        p.update(ui, &LooperView::default(), &Settings::default());
        Ok(p)
    }

    /// The new loop waveform (empty when the loop was cleared), on the
    /// Looper tab and on the Play page's strip.
    pub fn set_wave(&mut self, ui: &mut Ui, wave: Arc<LoopWave>) {
        let width = ui
            .rect_on(LOOPER_PAGE, WAVE)
            .map_or(600, |r| Waveform::wave_rect(r).w);
        self.frames = wave.frames;
        let set = |w: &mut Waveform, width: i32| {
            if wave.frames == 0 {
                w.set_peaks(None, width);
                w.set_playheads(&[]);
            } else {
                w.set_peaks(Some(wave.clone()), width);
            }
        };
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            set(w, width);
        }
        if let Some(s) = ui.get_mut::<LoopStrip>(LOOP_STRIP) {
            s.wave = Some(wave.clone()).filter(|w| w.frames > 0);
        }
    }

    pub fn update(&mut self, ui: &mut Ui, v: &LooperView, settings: &Settings) {
        let s = &v.status;
        // The meter, with a peak held for about 2 s of updates.
        self.hold_age += 1;
        if self.hold_age > 8 {
            self.hold = [f32::NEG_INFINITY; 2];
            self.hold_age = 0;
        }
        for ch in 0..2 {
            self.hold[ch] = self.hold[ch].max(v.meter_db[ch]);
        }
        // Ready to record: the pads always are, the line input once it is on.
        let ready = v.source == LoopSource::Pads || v.line_on;
        let note = if v.line.overruns + v.line.short + v.line.errors > 0 {
            format!(
                "{} overruns, {} short periods, {} errors",
                v.line.overruns, v.line.short, v.line.errors
            )
        } else if v.source == LoopSource::Pads {
            "Before the click and the loop".to_string()
        } else if v.monitor {
            "Monitor on: you hear the input".to_string()
        } else {
            "Monitor off".to_string()
        };
        if let Some(m) = ui.get_mut::<Meter>(METER) {
            let next = (
                v.source.name().to_string(),
                ready,
                v.meter_db,
                self.hold,
                note,
            );
            if (m.title.clone(), m.on, m.db, m.hold, m.note.clone()) != next {
                (m.title, m.on, m.db, m.hold, m.note) = next;
            }
        }
        // The buttons: what each does now.
        let empty = s.phase == Phase::Empty;
        let record = match s.phase {
            Phase::Empty => "Record",
            Phase::Armed => "Cancel",
            Phase::Recording if s.ending => "Ending",
            Phase::Recording => "End",
            Phase::Playing => "Overdub",
            Phase::Overdubbing => "Stop dub",
        };
        let overdub = match s.phase {
            Phase::Overdubbing => "Stop dub",
            Phase::Recording if s.overdub_next => "Dub next",
            _ => "Overdub",
        };
        let want = [
            Button {
                label: record.to_string(),
                tone: Tone::Bad,
                disabled: !ready && s.phase == Phase::Empty,
            },
            Button {
                label: overdub.to_string(),
                tone: Tone::Warn,
                disabled: !matches!(
                    s.phase,
                    Phase::Playing | Phase::Overdubbing | Phase::Recording
                ),
            },
            Button {
                label: "Undo".to_string(),
                tone: Tone::Normal,
                disabled: !(matches!(
                    s.phase,
                    Phase::Armed | Phase::Recording | Phase::Overdubbing
                ) || s.layers > 1),
            },
            Button {
                label: "Redo".to_string(),
                tone: Tone::Normal,
                disabled: s.redo == 0,
            },
            Button {
                label: "Clear".to_string(),
                tone: Tone::Normal,
                disabled: empty,
            },
            Button {
                label: if s.muted { "Unmute" } else { "Mute" }.to_string(),
                tone: if s.muted { Tone::Warn } else { Tone::Normal },
                disabled: false,
            },
            Button {
                label: if v.monitor { "Monitor on" } else { "Monitor" }.to_string(),
                tone: if v.monitor { Tone::Good } else { Tone::Normal },
                disabled: !v.line_on,
            },
        ];
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS_ID) {
            if b.buttons != want {
                b.buttons = want.to_vec();
            }
        }
        // The state.
        let phase = match s.phase {
            Phase::Empty if ready => ("Empty: Record arms it".to_string(), Tone::Dim),
            Phase::Empty => ("Empty: turn on Line input first".to_string(), Tone::Dim),
            Phase::Armed => ("Armed: starts on the next bar".to_string(), Tone::Warn),
            Phase::Recording => (
                format!(
                    "Recording the first pass{}",
                    if s.ending {
                        ", ends on the next bar"
                    } else {
                        ""
                    }
                ),
                Tone::Bad,
            ),
            Phase::Playing => ("Playing".to_string(), Tone::Good),
            Phase::Overdubbing => ("Overdubbing".to_string(), Tone::Bad),
        };
        let bars = |ticks: i64| ticks as f64 / (960.0 * f64::from(v.beats_per_bar.max(1)));
        let rows = vec![
            ("State".to_string(), phase.0, phase.1),
            (
                "Records from".to_string(),
                format!("{}{}", v.source.name(), if ready { "" } else { " (off)" }),
                if ready { Tone::Normal } else { Tone::Warn },
            ),
            (
                "Layers".to_string(),
                if empty {
                    "-".to_string()
                } else {
                    format!("{} ({} to redo)", s.layers, s.redo)
                },
                Tone::Normal,
            ),
            (
                "Loop".to_string(),
                if s.frames == 0 {
                    "-".to_string()
                } else {
                    format!(
                        "{:.0} bars, {:.2} s",
                        bars(s.ticks),
                        s.frames as f64 / 44_100.0
                    )
                },
                Tone::Normal,
            ),
            (
                "First pass length".to_string(),
                bars_text(s.bars),
                Tone::Normal,
            ),
            (
                "Latency".to_string(),
                match v.source {
                    LoopSource::Pads => "none (the pads are in time)".to_string(),
                    LoopSource::Line => format!(
                        "{} frames, {:.1} ms{}",
                        s.latency,
                        s.latency as f64 / 44.1,
                        if settings.loop_latency_set {
                            ""
                        } else {
                            " (estimated)"
                        }
                    ),
                },
                Tone::Dim,
            ),
            (
                "Level".to_string(),
                format!(
                    "{:.1} dB{}",
                    20.0 * s.level.max(1e-6).log10(),
                    if s.muted { ", muted" } else { "" }
                ),
                if s.muted { Tone::Warn } else { Tone::Normal },
            ),
            (
                "Memory".to_string(),
                format!("{:.0} s for all layers", s.capacity as f64 / 44_100.0),
                Tone::Dim,
            ),
            (
                "Problems".to_string(),
                format!(
                    "{} refused, {} thrown away",
                    v.stats.refused, v.stats.aborted
                ),
                if v.stats.refused + v.stats.aborted > 0 {
                    Tone::Warn
                } else {
                    Tone::Dim
                },
            ),
        ];
        if let Some(st) = ui.get_mut::<StatList>(STATUS) {
            st.set(rows);
        }
        // The bar lines (from the loop's own length) and the playhead.
        let fpb = v.frames_per_beat;
        let frames = self.frames;
        let mark = |w: &mut Waveform| {
            if frames > 0 && fpb > 0.0 {
                let beats = Some(Beats {
                    first: 0.0,
                    frames_per_beat: fpb,
                    per_bar: v.beats_per_bar.max(1),
                });
                if w.beats() != beats {
                    w.set_beats(beats);
                }
                let at = (f64::from(s.position) * frames as f64) as u64;
                if s.phase != Phase::Empty {
                    w.set_playheads(&[at.min(frames.saturating_sub(1))]);
                }
            } else if w.beats().is_some() {
                w.set_beats(None);
            }
        };
        if let Some(w) = ui.get_mut::<Waveform>(WAVE) {
            mark(w);
        }
        // The Play page's strip: the same loop, its layers and its state.
        let empty = s.phase == Phase::Empty;
        let n_bars = if s.frames == 0 {
            0
        } else {
            bars(s.ticks).round().max(1.0) as usize
        };
        let right = if empty {
            String::new()
        } else {
            let len = if n_bars == 0 {
                String::new()
            } else if n_bars == 1 {
                "1 bar · ".to_string()
            } else {
                format!("{n_bars} bars · ")
            };
            format!(
                "{len}{} layer{}",
                s.layers,
                if s.layers == 1 { "" } else { "s" }
            )
        };
        let state = match s.phase {
            Phase::Empty => None,
            Phase::Armed => Some(("ARMED", Tone::Warn)),
            Phase::Recording => Some(("RECORDING", Tone::Bad)),
            Phase::Overdubbing => Some(("OVERDUB", Tone::Warn)),
            Phase::Playing if s.muted => Some(("MUTED", Tone::Dim)),
            Phase::Playing => Some(("PLAYING", Tone::Good)),
        }
        .map(|(a, b)| (a.to_string(), b));
        let note = if empty {
            format!(
                "Empty: REC records the {}",
                match v.source {
                    LoopSource::Pads => "pads and the synth",
                    LoopSource::Line => "line input",
                }
            )
        } else {
            String::new()
        };
        let buttons: Vec<(String, bool)> = vec![
            (
                record_label(s).to_string(),
                matches!(s.phase, Phase::Armed | Phase::Recording),
            ),
            ("Overdub".to_string(), s.phase == Phase::Overdubbing),
            ("Stop".to_string(), false),
            ("Undo".to_string(), false),
            ("Redo".to_string(), false),
            (if s.muted { "Unmute" } else { "Mute" }.to_string(), s.muted),
            ("Clear".to_string(), false),
        ];
        let position = (!empty && s.frames > 0).then_some(s.position);
        if let Some(st) = ui.get_mut::<LoopStrip>(LOOP_STRIP) {
            if st.right != right
                || st.state != state
                || st.note != note
                || st.position != position
                || st.bars != n_bars
                || st.layers != s.layers
                || st.buttons != buttons
            {
                st.right = right;
                st.state = state;
                st.note = note;
                st.position = position;
                st.bars = n_bars;
                st.layers = s.layers;
                st.buttons = buttons;
            }
        }
    }
}

/// What the Record button does now.
fn record_label(s: &mpfree_looper::Status) -> &'static str {
    match s.phase {
        Phase::Empty => "Record",
        Phase::Armed => "Cancel",
        Phase::Recording if s.ending => "Ending",
        Phase::Recording => "End",
        Phase::Playing => "Overdub",
        Phase::Overdubbing => "Stop dub",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme};

    #[test]
    fn the_page_draws_and_its_buttons_follow_the_state() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add(
            "header",
            mpfree_ui::widgets::Header::new(&["Play", "Looper"]),
        );
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new("", Tone::Dim, Align::Center),
        );
        let mut page = LooperPage::add(&mut ui).unwrap();
        ui.show(LOOPER_PAGE).unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let b = ui.get::<ButtonRow>(BUTTONS_ID).unwrap();
        assert!(
            !b.buttons[B_RECORD].disabled,
            "the pads can always be recorded"
        );
        let line = LooperView {
            source: LoopSource::Line,
            ..LooperView::default()
        };
        page.update(&mut ui, &line, &Settings::default());
        let b = ui.get::<ButtonRow>(BUTTONS_ID).unwrap();
        assert!(
            b.buttons[B_RECORD].disabled,
            "Record waits for the line input"
        );
        assert!(b.buttons[B_MONITOR].disabled);
        let mut v = LooperView {
            source: LoopSource::Line,
            line_on: true,
            ..LooperView::default()
        };
        v.status.phase = Phase::Playing;
        v.status.layers = 2;
        v.status.frames = 88_200;
        v.status.ticks = 3_840;
        v.frames_per_beat = 22_050.0;
        page.set_wave(
            &mut ui,
            Arc::new(LoopWave {
                frames: 88_200,
                ..LoopWave::default()
            }),
        );
        page.update(&mut ui, &v, &Settings::default());
        let b = ui.get::<ButtonRow>(BUTTONS_ID).unwrap();
        assert_eq!(b.buttons[B_RECORD].label, "Overdub");
        assert!(
            !b.buttons[B_UNDO].disabled
                && b.buttons[B_REDO].disabled
                && !b.buttons[B_CLEAR].disabled
        );
        ui.paint(&mut c);
        // A tap on Undo comes back as Tab(2).
        let r = ui.rect_of(BUTTONS_ID).unwrap();
        let undo = ui
            .get::<ButtonRow>(BUTTONS_ID)
            .unwrap()
            .button_rect(r, B_UNDO);
        let e = ui
            .touch(1, mpfree_ui::Touch::Down, undo.x + 5, undo.y + 5)
            .unwrap();
        assert_eq!(
            (e.widget.as_str(), e.kind),
            (BUTTONS_ID, mpfree_ui::EventKind::Tab(B_UNDO))
        );
    }
}
