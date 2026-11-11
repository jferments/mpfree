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
use mpfree_ui::wave::{Beats, Waveform};
use mpfree_ui::widgets::StatList;
use mpfree_ui::{Align, FontId, PaintCx, Rect, Tone, Ui, Widget};
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

/// The Play page's loop strip (the All and Loop views): a heading line with
/// the loop's length and layers, over the loop's waveform. The Looper tab
/// is where the loop is edited; this is where it is played.
pub struct LoopStrip {
    pub wave: Waveform,
    /// The heading's right-hand text: "4 bars · 3 layers · Playing".
    pub right: String,
    /// The heading as last drawn, so a playhead move redraws only its strip.
    drawn: RefCell<Option<String>>,
}

/// The strip's widget name, and the height of its heading line.
pub const LOOP_STRIP: &str = "play-loop";
const STRIP_HEAD: i32 = 26;

impl Default for LoopStrip {
    fn default() -> Self {
        LoopStrip {
            wave: Waveform::new(),
            right: String::new(),
            drawn: RefCell::new(None),
        }
    }
}

impl LoopStrip {
    /// Where the waveform sits in the strip's rectangle.
    pub fn wave_area(r: Rect) -> Rect {
        r.take_top(STRIP_HEAD).1
    }
}

impl Widget for LoopStrip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let (head, _) = r.take_top(STRIP_HEAD);
        cx.canvas.fill_rect(head, t.bg);
        let line = head.inset_xy(4, 0);
        cx.text(
            FontId::Bold,
            t.size_small,
            line,
            Align::Left,
            t.text_dim,
            "LOOPER",
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            line,
            Align::Right,
            t.text_dim,
            &self.right,
        );
        self.wave.paint(cx, LoopStrip::wave_area(r));
        *self.drawn.borrow_mut() = Some(self.right.clone());
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        if self.drawn.borrow().as_deref() != Some(self.right.as_str()) {
            return None;
        }
        self.wave.changed_parts(LoopStrip::wave_area(r))
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
    /// Looper tab and on the Play page's strip (`strip_page`: a Play page
    /// that has it, to size it; every Play view gives it the same width).
    pub fn set_wave(&mut self, ui: &mut Ui, wave: Arc<LoopWave>, strip_page: &str) {
        let width = ui
            .rect_on(LOOPER_PAGE, WAVE)
            .map_or(600, |r| Waveform::wave_rect(r).w);
        let strip = ui
            .rect_on(strip_page, LOOP_STRIP)
            .map_or(900, |r| Waveform::wave_rect(LoopStrip::wave_area(r)).w);
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
            set(&mut s.wave, strip);
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
        // The Play page's strip: the same wave, and a line on the loop.
        let right = if s.phase == Phase::Empty {
            "Empty: REC records the pads".to_string()
        } else {
            let state = match s.phase {
                Phase::Armed => "Armed",
                Phase::Recording => "Recording",
                Phase::Overdubbing => "Overdubbing",
                _ if s.muted => "Muted",
                _ => "Playing",
            };
            let len = if s.frames == 0 {
                String::new()
            } else {
                format!("{:.0} bars · ", bars(s.ticks))
            };
            format!(
                "{len}{} layer{} · {state}",
                s.layers,
                if s.layers == 1 { "" } else { "s" }
            )
        };
        if let Some(st) = ui.get_mut::<LoopStrip>(LOOP_STRIP) {
            mark(&mut st.wave);
            if st.right != right {
                st.right = right;
            }
        }
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
                columns: Vec::new(),
            }),
            LOOPER_PAGE,
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
