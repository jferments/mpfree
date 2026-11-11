//! The pad editor's Rec tab: the line input onto a pad.
//!
//! - The **input** picture shows the line input's level on two meters and
//!   the last half minute of it scrolling past, with the transport's bar
//!   lines, the span **Skip back** would keep and the take being recorded.
//! - **Record** starts a take with the Record list's settings: at once, when
//!   the input goes over the threshold (keeping a pre-roll from before),
//!   or on the next bar line; it ends on **Stop**, on silence, or after a
//!   number of bars (exactly on the bar line when it started on one). A
//!   take still waiting for its start is cancelled by Stop.
//! - **Skip back** keeps what already came in: the last 5 to 60 seconds,
//!   or the last 1 to 8 whole bars, cut on the bar lines.
//! - Either way the sound goes onto the next empty pad from the pad picked
//!   (or onto the pad picked, replacing its sound), as one undo step, and
//!   the editor picks that pad.
//! - The **Record** list holds the settings: which channels (both, left,
//!   right, or both mixed to mono), how a take starts and ends, the
//!   threshold, pre-roll, length and silence, how far Skip back goes, and
//!   which pad a take goes onto. A tap on a row picks it and its arrows
//!   step it; the **wheel** steps the row picked.
//!
//! The settings live in the tab ([`RecTab::settings`]); the app makes the
//! [`Recording`] from them on [`RecAction::Record`], polls it, and puts the
//! sound on the pad. The tab only reads the sampler's [`Input`].

use mpfree_sampler::record::{End, Input, Pick, Progress, Recording, Start, BAR_LINES};
use mpfree_ui::{EventKind, Ui, UiEvent};

use crate::chop_tab::pad_text;
use crate::rec_widgets::{InputView, Span, Strip};
use crate::wave_tab::{self, duration_text, pad_color, TABS};
use crate::widgets::{ButtonRow, InfoCard, PadBar, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-rec-bar";
pub const VIEW: &str = "smp-rec-view";
pub const LIST: &str = "smp-rec-list";
pub const BUTTONS: &str = "smp-rec-buttons";
pub const WHEEL: &str = "smp-rec-wheel";

/// The tab's page: the input beside the Record list, the buttons under
/// them.
pub const LAYOUT: &str = "col(header:60, smp-rec-bar:60, \
                          row(smp-rec-view:*, smp-rec-list:340):*, \
                          row(smp-rec-buttons:*, smp-rec-wheel:340):88, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = 6;

/// The hint line the app shows under the tab.
pub const HINT: &str = "Record, or keep what just came in with Skip back · tap a row, then the wheel changes it · UNDO \
                        takes a take off its pad again";

/// What the picture says while the line input is off.
pub const OFF_NOTE: &str = "The line input is off: Looper tab, MENU, Line input";

/// The input the strip shows, in seconds, and its columns a second.
pub const STRIP_SECONDS: u32 = 30;
const STRIP_COLUMNS_PER_SECOND: u32 = 30;

/// The longest take: three minutes (about 64 MB of stereo).
pub const MAX_TAKE_SECONDS: u32 = 180;

const THRESHOLDS_DB: [f32; 15] = [
    -60.0, -54.0, -48.0, -42.0, -36.0, -33.0, -30.0, -27.0, -24.0, -21.0, -18.0, -15.0, -12.0,
    -9.0, -6.0,
];
const PRE_ROLLS_MS: [u32; 8] = [0, 5, 10, 20, 50, 100, 200, 500];
const LENGTHS_BARS: [u32; 6] = [1, 2, 4, 8, 16, 32];
const SILENCES_MS: [u32; 5] = [250, 500, 1000, 2000, 4000];
const SKIPS: [Skip; 9] = [
    Skip::Seconds(5),
    Skip::Seconds(10),
    Skip::Seconds(20),
    Skip::Seconds(30),
    Skip::Seconds(60),
    Skip::Bars(1),
    Skip::Bars(2),
    Skip::Bars(4),
    Skip::Bars(8),
];

/// How a take starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartAt {
    Now,
    Threshold,
    NextBar,
}

/// How a take ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndAt {
    Press,
    Silence,
    Length,
}

/// How far Skip back goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    Seconds(u32),
    Bars(u32),
}

impl Skip {
    pub fn label(self) -> String {
        match self {
            Skip::Seconds(s) if s >= 60 => format!("{} min", s / 60),
            Skip::Seconds(s) => format!("{s} s"),
            Skip::Bars(1) => "1 bar".to_string(),
            Skip::Bars(n) => format!("{n} bars"),
        }
    }
}

/// Which pad a take goes onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Onto {
    /// The pad picked if it has no sound, else the next one without.
    NextEmpty,
    /// The pad picked, replacing its sound.
    ThisPad,
}

/// The Record list's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecSettings {
    pub pick: Pick,
    pub start: StartAt,
    pub threshold_db: f32,
    pub pre_roll_ms: u32,
    pub end: EndAt,
    pub bars: u32,
    pub silence_ms: u32,
    pub skip: Skip,
    pub onto: Onto,
}

impl Default for RecSettings {
    fn default() -> Self {
        RecSettings {
            pick: Pick::Stereo,
            start: StartAt::Now,
            threshold_db: -30.0,
            pre_roll_ms: 20,
            end: EndAt::Press,
            bars: 4,
            silence_ms: 1000,
            skip: Skip::Seconds(10),
            onto: Onto::NextEmpty,
        }
    }
}

fn ms_frames(ms: u32, rate: u32) -> u64 {
    u64::from(ms) * u64::from(rate) / 1000
}

impl RecSettings {
    /// The threshold as a linear peak.
    pub fn level(&self) -> f32 {
        10f32.powf(self.threshold_db / 20.0)
    }

    pub fn start_at(&self, rate: u32) -> Start {
        match self.start {
            StartAt::Now => Start::Now,
            StartAt::Threshold => Start::Threshold {
                level: self.level(),
                pre_roll: ms_frames(self.pre_roll_ms, rate),
            },
            StartAt::NextBar => Start::NextBar,
        }
    }

    /// How a take ends; `bar_frames` is a bar's length now (the
    /// transport's, or the project tempo's while it is stopped), for a
    /// length that does not start on a bar line.
    pub fn end_at(&self, rate: u32, bar_frames: f64) -> End {
        match self.end {
            EndAt::Press => End::Press,
            EndAt::Silence => End::Silence {
                level: self.level(),
                frames: ms_frames(self.silence_ms, rate),
            },
            EndAt::Length if self.start == StartAt::NextBar => End::Bars(self.bars),
            EndAt::Length => {
                End::Frames((f64::from(self.bars) * bar_frames.max(1.0)).round() as u64)
            }
        }
    }

    /// A recording asked for now with these settings.
    pub fn recording(&self, input: &Input, bar_frames: f64) -> Recording {
        Recording::new(
            input,
            self.start_at(input.rate),
            self.end_at(input.rate, bar_frames),
            u64::from(MAX_TAKE_SECONDS) * u64::from(input.rate),
        )
    }
}

/// The Record list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Input,
    Start,
    Threshold,
    PreRoll,
    End,
    Length,
    Silence,
    SkipBack,
    Onto,
}

impl Row {
    pub const ALL: [Row; 9] = [
        Row::Input,
        Row::Start,
        Row::Threshold,
        Row::PreRoll,
        Row::End,
        Row::Length,
        Row::Silence,
        Row::SkipBack,
        Row::Onto,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Input => "Input",
            Row::Start => "Start",
            Row::Threshold => "Threshold",
            Row::PreRoll => "Pre-roll",
            Row::End => "End",
            Row::Length => "Length",
            Row::Silence => "Silence for",
            Row::SkipBack => "Skip back",
            Row::Onto => "Onto",
        }
    }

    fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// Row `row`'s value as text.
pub fn value_text(s: &RecSettings, row: Row) -> String {
    match row {
        Row::Input => s.pick.label().to_string(),
        Row::Start => match s.start {
            StartAt::Now => "Now",
            StartAt::Threshold => "Threshold",
            StartAt::NextBar => "Next bar",
        }
        .to_string(),
        Row::Threshold => format!("{:.0} dB", s.threshold_db),
        Row::PreRoll => format!("{} ms", s.pre_roll_ms),
        Row::End => match s.end {
            EndAt::Press => "Stop",
            EndAt::Silence => "Silence",
            EndAt::Length => "Length",
        }
        .to_string(),
        Row::Length => Skip::Bars(s.bars).label(),
        Row::Silence => {
            if s.silence_ms >= 1000 {
                format!("{} s", s.silence_ms / 1000)
            } else {
                format!("{} ms", s.silence_ms)
            }
        }
        Row::SkipBack => s.skip.label(),
        Row::Onto => match s.onto {
            Onto::NextEmpty => "Next empty",
            Onto::ThisPad => "This pad",
        }
        .to_string(),
    }
}

fn shift(i: usize, delta: i32, len: usize) -> usize {
    (i as i32 + delta).clamp(0, len as i32 - 1) as usize
}

fn step_in<T: PartialEq + Copy>(list: &[T], now: T, delta: i32) -> T {
    let i = list.iter().position(|x| *x == now).unwrap_or(0);
    list[shift(i, delta, list.len())]
}

/// `s` with row `row` stepped by `delta` steps.
pub fn stepped(s: &RecSettings, row: Row, delta: i32) -> RecSettings {
    let mut s = *s;
    match row {
        Row::Input => s.pick = step_in(&Pick::ALL, s.pick, delta),
        Row::Start => {
            s.start = step_in(
                &[StartAt::Now, StartAt::Threshold, StartAt::NextBar],
                s.start,
                delta,
            )
        }
        Row::Threshold => {
            let i = THRESHOLDS_DB
                .iter()
                .position(|d| *d >= s.threshold_db)
                .unwrap_or(0);
            s.threshold_db = THRESHOLDS_DB[shift(i, delta, THRESHOLDS_DB.len())];
        }
        Row::PreRoll => s.pre_roll_ms = step_in(&PRE_ROLLS_MS, s.pre_roll_ms, delta),
        Row::End => s.end = step_in(&[EndAt::Press, EndAt::Silence, EndAt::Length], s.end, delta),
        Row::Length => s.bars = step_in(&LENGTHS_BARS, s.bars, delta),
        Row::Silence => s.silence_ms = step_in(&SILENCES_MS, s.silence_ms, delta),
        Row::SkipBack => s.skip = step_in(&SKIPS, s.skip, delta),
        Row::Onto => s.onto = step_in(&[Onto::NextEmpty, Onto::ThisPad], s.onto, delta),
    }
    s
}

/// What the tab shows besides the input: the pad picked and where a take
/// would go.
#[derive(Clone, Debug, PartialEq)]
pub struct RecState {
    /// The pad picked in the editor, and its sound's name if it has one.
    pub pad: usize,
    pub pad_name: Option<String>,
    /// The pad a take goes onto now (None: no empty pad left).
    pub target: Option<usize>,
    /// The line input is on (Looper tab, MENU).
    pub line_on: bool,
    /// The last take, said in a line.
    pub last: String,
}

/// What a touch or control on the tab asks for. The app does it.
#[derive(Clone, Debug, PartialEq)]
pub enum RecAction {
    /// Start a take with [`RecTab::settings`].
    Record,
    /// Stop the take (cancel one still waiting).
    Stop,
    /// Keep what the settings' Skip back covers, now.
    SkipBack,
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
}

/// The Rec tab's widgets and settings.
pub struct RecTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`wave_tab::WaveTab::bank`].
    pub bank: Option<char>,
    pub settings: RecSettings,
    /// A take is being recorded (or waits for its start): the first button
    /// stops it.
    recording: bool,
}

impl RecTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`. The strip shows input at `rate`.
    pub fn add(ui: &mut Ui, name: &str, st: &RecState, rate: u32) -> Result<RecTab, String> {
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        let per = u64::from(rate / STRIP_COLUMNS_PER_SECOND).max(1);
        let strip = Strip::new(per, (STRIP_SECONDS * STRIP_COLUMNS_PER_SECOND) as usize);
        ui.add(VIEW, InputView::new("Line input", strip, rate));
        let mut list = SettingList::new("Record");
        list.selected = Row::Start.index();
        ui.add(LIST, list);
        let mut buttons = ButtonRow::new(&["Record", "Skip back"]);
        buttons.primary = Some(0);
        ui.add(BUTTONS, buttons);
        ui.add(WHEEL, InfoCard::new("Wheel · the row picked"));
        ui.add_page(name, LAYOUT)?;
        let mut tab = RecTab {
            name: name.to_string(),
            bank: None,
            settings: RecSettings::default(),
            recording: false,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The row picked in the Record list.
    pub fn row(&self, ui: &Ui) -> Row {
        ui.get::<SettingList>(LIST)
            .and_then(|l| Row::ALL.get(l.selected).copied())
            .unwrap_or(Row::Start)
    }

    /// Shows `st` and the settings.
    pub fn update(&mut self, ui: &mut Ui, st: &RecState) {
        let target = st.target.map(|p| pad_text(self.bank, p));
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.target.unwrap_or(st.pad));
            bar.active = TAB;
            // A pad without a sound has nothing for the other tabs to edit.
            bar.enabled = TABS
                .iter()
                .map(|t| {
                    wave_tab::BUILT.contains(t)
                        && (st.pad_name.is_some() || *t == TABS[TAB] || *t == TABS[0])
                })
                .collect();
            bar.title = match &target {
                Some(p) => format!("Record onto pad {p}"),
                None => "Record: no empty pad in this bank".to_string(),
            };
            bar.subtitle = if st.last.is_empty() {
                match (&st.pad_name, self.settings.onto) {
                    (Some(n), Onto::ThisPad) => format!("replacing {n}"),
                    _ => format!(
                        "{} · from the line input",
                        value_text(&self.settings, Row::Input)
                    ),
                }
            } else {
                st.last.clone()
            };
        }
        let rows = Row::ALL
            .iter()
            .map(|r| Setting {
                name: r.label().into(),
                value: value_text(&self.settings, *r),
            })
            .collect();
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
            list.right = target.map(|p| format!("onto {p}")).unwrap_or_default();
        }
        let can = st.line_on && st.target.is_some();
        let recording = self.recording;
        let skip = self.settings.skip.label();
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels = vec![
                if recording { "Stop" } else { "Record" }.to_string(),
                format!("Skip back {skip}"),
            ];
            b.enabled = vec![recording || can, can && !recording];
        }
        self.show_wheel(ui);
    }

    fn show_wheel(&self, ui: &mut Ui) {
        let row = self.row(ui);
        let text = format!("{}: {}", row.label(), value_text(&self.settings, row));
        if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
            card.text = text;
        }
    }

    /// Reads what came into `input` since the last call (`dt` seconds ago)
    /// and marks the bar lines, what Skip back would keep, and `rec` (the
    /// take, if one is being made) on the strip. `line_on`: the line
    /// input is on.
    pub fn feed(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rec: Option<&Recording>,
        line_on: bool,
        dt: f32,
    ) {
        let was = self.recording;
        self.recording = rec.is_some_and(|r| {
            matches!(r.progress(), Progress::Waiting | Progress::Recording { .. })
        });
        let settings = self.settings;
        let Some(v) = ui.get_mut::<InputView>(VIEW) else {
            return;
        };
        v.update(input, dt);
        let now = v.now;
        let rate = input.rate;
        v.bars = (0..BAR_LINES).map_while(|n| input.bars.back(n)).collect();
        v.threshold = (settings.start == StartAt::Threshold || settings.end == EndAt::Silence)
            .then(|| settings.level());
        v.skip = match settings.skip {
            _ if !line_on => None,
            Skip::Seconds(s) => {
                let start = now
                    .saturating_sub(u64::from(s) * u64::from(rate))
                    .max(input.audio.oldest());
                (now > start).then(|| Span {
                    start,
                    end: now,
                    label: format!("Skip back {}", settings.skip.label()),
                })
            }
            Skip::Bars(n) => match (input.bars.back(n as usize), input.bars.back(0)) {
                (Some(start), Some(end)) if start >= input.audio.oldest() => Some(Span {
                    start,
                    end,
                    label: format!("Skip back {}", settings.skip.label()),
                }),
                _ => None,
            },
        };
        v.take = rec.and_then(|r| {
            let start = r.started_at()?;
            let frames = match r.progress() {
                Progress::Recording { frames } | Progress::Done { frames } => frames,
                _ => 0,
            };
            Some(Span {
                start,
                end: (start + frames).max(start + 1),
                label: format!("● {}", duration_text(frames, rate)),
            })
        });
        v.note = if !line_on {
            OFF_NOTE.to_string()
        } else {
            match rec.map(Recording::progress) {
                Some(Progress::Waiting) => match settings.start {
                    StartAt::NextBar if input.bars.bar_frames().is_none() => {
                        "Waiting for the next bar line: press PLAY".to_string()
                    }
                    StartAt::NextBar => "Waiting for the next bar line".to_string(),
                    _ => format!(
                        "Waiting for the input to go over {:.0} dB",
                        settings.threshold_db
                    ),
                },
                _ => String::new(),
            }
        };
        let held = input.held();
        v.caption = if matches!(settings.skip, Skip::Bars(_)) && v.skip.is_none() {
            "Skip back by bars: the transport has to play that many bars first".to_string()
        } else {
            format!("{} held", duration_text(held, rate))
        };
        v.right = match input.bars.bar_frames() {
            Some(bar) => format!(
                "bar lines every {}",
                duration_text(bar.round() as u64, rate)
            ),
            None => "transport stopped".to_string(),
        };
        if was != self.recording {
            if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
                b.labels[0] = if self.recording { "Stop" } else { "Record" }.to_string();
                if self.recording {
                    b.enabled[0] = true;
                    b.enabled[1] = false;
                }
            }
        }
    }

    /// No input to show (the line input is off, or just turned on): the
    /// picture says so and the strip starts again.
    pub fn idle(&mut self, ui: &mut Ui, line_on: bool) {
        self.recording = false;
        if let Some(v) = ui.get_mut::<InputView>(VIEW) {
            v.strip.reset();
            v.skip = None;
            v.take = None;
            v.bars.clear();
            v.threshold = None;
            v.meter_db = [f32::NEG_INFINITY; 2];
            v.note = if line_on {
                "Starting the line input"
            } else {
                OFF_NOTE
            }
            .to_string();
            v.caption.clear();
            v.right.clear();
        }
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels[0] = "Record".to_string();
            b.enabled = vec![false, false];
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change the settings or what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &RecState) -> Option<RecAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(RecAction::Tab(*i)),
            (BUTTONS, EventKind::Tab(0)) => Some(if self.recording {
                RecAction::Stop
            } else {
                RecAction::Record
            }),
            (BUTTONS, EventKind::Tab(1)) => Some(RecAction::SkipBack),
            (LIST, EventKind::Step { index, delta }) => {
                let row = *Row::ALL.get(*index)?;
                self.settings = stepped(&self.settings, row, *delta);
                self.update(ui, st);
                None
            }
            (LIST, EventKind::Select(_)) => {
                self.show_wheel(ui);
                None
            }
            _ => None,
        }
    }

    /// The data wheel: steps the row picked by `delta` detents.
    pub fn wheel(&mut self, ui: &mut Ui, st: &RecState, delta: i32) {
        let row = self.row(ui);
        self.settings = stepped(&self.settings, row, delta);
        self.update(ui, st);
    }

    /// A take is being made (the first button says Stop).
    pub fn recording(&self) -> bool {
        self.recording
    }
}

/// The pad a take goes onto, by `onto`, from pad `pad` within its bank of
/// 16: the pad itself if it has no sound (or `onto` is
/// [`Onto::ThisPad`]), else the next one without a sound after it,
/// wrapping round the bank. `has_sound` says whether a pad has one.
pub fn target(pad: usize, onto: Onto, has_sound: impl Fn(usize) -> bool) -> Option<usize> {
    if onto == Onto::ThisPad || !has_sound(pad) {
        return Some(pad);
    }
    let first = pad - pad % 16;
    (1..16)
        .map(|k| first + (pad % 16 + k) % 16)
        .find(|&p| !has_sound(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_step_within_their_lists() {
        let s = RecSettings::default();
        assert_eq!(value_text(&s, Row::Start), "Now");
        let s = stepped(&s, Row::Start, 1);
        assert_eq!(s.start, StartAt::Threshold);
        let s = stepped(&s, Row::Start, 5);
        assert_eq!(s.start, StartAt::NextBar, "stops at the end");
        let s = stepped(&s, Row::Threshold, 2);
        assert_eq!(s.threshold_db, -24.0);
        let s = stepped(&s, Row::SkipBack, 5);
        assert_eq!(s.skip, Skip::Bars(2));
        assert_eq!(value_text(&s, Row::SkipBack), "2 bars");
        assert_eq!(Skip::Seconds(60).label(), "1 min");
        assert_eq!(
            value_text(&stepped(&s, Row::Silence, -1), Row::Silence),
            "500 ms"
        );
    }

    #[test]
    fn the_settings_make_the_recording_asked_for() {
        let mut s = RecSettings {
            start: StartAt::Threshold,
            threshold_db: -20.0,
            pre_roll_ms: 100,
            ..RecSettings::default()
        };
        assert_eq!(
            s.start_at(44_100),
            Start::Threshold {
                level: 0.1,
                pre_roll: 4_410
            }
        );
        s.end = EndAt::Length;
        s.bars = 2;
        // Not starting on a bar line: two bars at the tempo now.
        assert_eq!(s.end_at(44_100, 88_200.0), End::Frames(176_400));
        s.start = StartAt::NextBar;
        assert_eq!(s.end_at(44_100, 88_200.0), End::Bars(2));
        s.end = EndAt::Silence;
        s.silence_ms = 500;
        assert!(matches!(
            s.end_at(44_100, 88_200.0),
            End::Silence { frames: 22_050, .. }
        ));
    }

    #[test]
    fn a_take_goes_onto_the_next_empty_pad_in_the_bank() {
        let full = [0usize, 1, 2, 5, 15];
        let has = |p: usize| full.contains(&(p % 16));
        assert_eq!(target(3, Onto::NextEmpty, has), Some(3));
        assert_eq!(target(1, Onto::NextEmpty, has), Some(3));
        assert_eq!(target(15, Onto::NextEmpty, has), Some(3), "wraps round");
        assert_eq!(target(17, Onto::NextEmpty, has), Some(19), "in bank B");
        assert_eq!(target(1, Onto::ThisPad, has), Some(1));
        assert_eq!(target(4, Onto::NextEmpty, |_| true), None);
    }
}
