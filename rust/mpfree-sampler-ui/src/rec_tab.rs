//! The pad editor's Rec tab: the line input, or any point in the mix,
//! onto a pad.
//!
//! - The **Source** row picks what it records: the line input, or a point
//!   in the mix the app offers (the master as heard, a track, a pad's
//!   channel, a return): resampling. The app keeps one buffer and starts
//!   it afresh on a new source, so Skip back reaches back to the change at
//!   most. A take being made keeps its source: the row waits for it.
//! - The **input** picture shows the source's level on two meters and
//!   the last half minute of it scrolling past, with the transport's bar
//!   lines, the span **Skip back** would keep and the take being recorded.
//! - **Record** starts a take with the Record list's settings: at once, when
//!   the input goes over the threshold (keeping a pre-roll from before),
//!   or on the next bar line; it ends on **Stop**, on silence, or after a
//!   number of bars (exactly on the bar line when it started on one). A
//!   take still waiting for its start is cancelled by Stop.
//! - After its bars a take is cut, **rings out** until it has been under
//!   -60 dB for the Silence time (at most 20 s more: a one-shot with its
//!   reverb), or **loops**, exactly its bars long and without a seam
//!   (Print N bars to a pad): music already going round when it started
//!   is cut on the bar line (its start holds the ringing of the time
//!   before); music that started with it is let ring, and the ringing is
//!   folded onto its start, or, if the music is still going a whole length
//!   on, that second time round is kept. Takes of whole bars land warped
//!   at the project tempo, so they follow it.
//! - **Skip back** keeps what already came in: the last 5 to 60 seconds,
//!   or the last 1 to 8 whole bars, cut on the bar lines.
//! - Either way the sound goes onto the next empty pad from the pad picked
//!   (or onto the pad picked, replacing its sound), as one undo step, and
//!   the editor picks that pad.
//! - **Flatten** resamples the pad picked as it plays: the app records the
//!   pad's own channel, plays the pad once at the Flatten velocity, lets
//!   go after the Flatten hold, and keeps the sound until it has been
//!   silent (under -60 dB) for the Silence time. Its layers, envelopes,
//!   filter, warp and pan are in the one sound; onto the next empty pad
//!   its channel's insert effects are too (onto the pad itself they are
//!   left out, as the channel will play them again).
//! - **Auto-sample** opens the mode that plays the synth on the pads note
//!   by note and makes a keygroup instrument of it ([`crate::auto_tab`]).
//! - The **Record** list holds the settings: which channels (both, left,
//!   right, or both mixed to mono), how a take starts and ends, the
//!   threshold, pre-roll, length, tail and silence, how far Skip back goes,
//!   which pad a take goes onto, and how Flatten plays the pad. A tap on a row picks it and its arrows
//!   step it; the **wheel** steps the row picked.
//!
//! The settings live in the tab ([`RecTab::settings`]); the app makes the
//! [`Recording`] from them on [`RecAction::Record`], polls it, and puts the
//! sound on the pad. The tab only reads the sampler's [`Input`].

use mpfree_sampler::record::{
    End, Input, Pick, Progress, Recording, RingOut, Start, Tail, BAR_LINES,
};
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
pub const LAYOUT: &str = "col(header:60, row(smp-rec-bar:*, smp-send:330):60, \
                          row(smp-rec-view:*, smp-rec-list:340):*, \
                          row(smp-rec-buttons:*, smp-rec-wheel:340):88, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = 6;

/// The hint line the app shows under the tab.
pub const HINT: &str =
    "Record, or keep what just came in with Skip back · Flatten resamples the pad picked · the wheel picks a \
                        row, - / + change it · UNDO takes a take off its pad again";

/// The third button: resample the pad picked.
pub const FLATTEN_LABEL: &str = "Flatten";

/// The fourth button: the Auto-sample mode.
pub const AUTO_LABEL: &str = "Auto-sample";

/// Flatten's level for silence: under it for the Silence time, the sound
/// has ended (and a sound starts when it goes over it).
pub const FLATTEN_SILENCE_DB: f32 = -60.0;

/// A take that rings out after its bars ends under this, as a Flatten's
/// sound does, after the Silence time.
pub const TAIL_SILENCE_DB: f32 = FLATTEN_SILENCE_DB;

/// The longest a take rings on after its bars.
pub const TAIL_MAX_SECONDS: u32 = 20;

/// What the picture says while the line input is off (and is the source).
pub const OFF_NOTE: &str =
    "The line input is off: Looper tab, MENU, Line input · or pick another Source";

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
const VELOCITIES: [u8; 9] = [16, 32, 48, 64, 80, 96, 110, 120, 127];
const HOLDS_MS: [u32; 7] = [100, 250, 500, 1000, 2000, 4000, 8000];
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

/// What a take of a length does after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailAt {
    /// It ends on the bar line.
    Cut,
    /// It rings out until silent: a one-shot.
    Ring,
    /// It is made a loop of exactly its bars, without a seam.
    Loop,
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
    /// What it records from: an index into [`RecState::sources`] (0, the
    /// first, is the line input).
    pub source: usize,
    pub pick: Pick,
    pub start: StartAt,
    pub threshold_db: f32,
    pub pre_roll_ms: u32,
    pub end: EndAt,
    pub bars: u32,
    /// After a length: cut, ring out or loop.
    pub tail: TailAt,
    pub silence_ms: u32,
    pub skip: Skip,
    pub onto: Onto,
    /// Flatten plays the pad at this velocity and lets it go after this
    /// long (a gated or looping sound plays that long; a one-shot plays
    /// out).
    pub flatten_velocity: u8,
    pub flatten_hold_ms: u32,
}

impl Default for RecSettings {
    fn default() -> Self {
        RecSettings {
            source: 0,
            pick: Pick::Stereo,
            start: StartAt::Now,
            threshold_db: -30.0,
            pre_roll_ms: 20,
            end: EndAt::Press,
            bars: 4,
            tail: TailAt::Cut,
            silence_ms: 1000,
            skip: Skip::Seconds(10),
            onto: Onto::NextEmpty,
            flatten_velocity: 127,
            flatten_hold_ms: 1000,
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

    /// What a take does after its length: silent is under
    /// [`TAIL_SILENCE_DB`] for the Silence time, and ringing out stops
    /// [`TAIL_MAX_SECONDS`] on at most. Other ends have no tail.
    pub fn tail_at(&self, rate: u32) -> Tail {
        let level = 10f32.powf(TAIL_SILENCE_DB / 20.0);
        let quiet = ms_frames(self.silence_ms, rate);
        match (self.end, self.tail) {
            (EndAt::Length, TailAt::Ring) => Tail::Ring(RingOut {
                level,
                quiet,
                max: u64::from(TAIL_MAX_SECONDS) * u64::from(rate),
            }),
            (EndAt::Length, TailAt::Loop) => Tail::Loop { level, quiet },
            _ => Tail::Cut,
        }
    }

    /// Whether a take with these settings is whole bars at the tempo (it
    /// lands warped at it).
    pub fn in_bars(&self) -> bool {
        self.end == EndAt::Length
    }

    /// The recording Flatten makes: from the sound's first frame over
    /// [`FLATTEN_SILENCE_DB`] (with 2 ms before it) until it has been
    /// under it for the Silence time, at most the hold and 20 s more.
    pub fn flatten_recording(&self, input: &Input) -> Recording {
        let level = 10f32.powf(FLATTEN_SILENCE_DB / 20.0);
        let rate = input.rate;
        Recording::new(
            input,
            Start::Threshold {
                level,
                pre_roll: ms_frames(2, rate),
            },
            End::Silence {
                level,
                frames: ms_frames(self.silence_ms, rate),
            },
            ms_frames(self.flatten_hold_ms + 20_000, rate),
        )
    }

    /// A recording asked for now with these settings.
    pub fn recording(&self, input: &Input, bar_frames: f64) -> Recording {
        Recording::with_tail(
            input,
            self.start_at(input.rate),
            self.end_at(input.rate, bar_frames),
            self.tail_at(input.rate),
            u64::from(MAX_TAKE_SECONDS) * u64::from(input.rate),
        )
    }
}

/// The Record list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Source,
    Input,
    Start,
    Threshold,
    PreRoll,
    End,
    Length,
    Tail,
    Silence,
    SkipBack,
    Onto,
    FlattenVelocity,
    FlattenHold,
}

impl Row {
    pub const ALL: [Row; 13] = [
        Row::Source,
        Row::Input,
        Row::Start,
        Row::Threshold,
        Row::PreRoll,
        Row::End,
        Row::Length,
        Row::Tail,
        Row::Silence,
        Row::SkipBack,
        Row::Onto,
        Row::FlattenVelocity,
        Row::FlattenHold,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Source => "Source",
            Row::Input => "Channels",
            Row::Start => "Start",
            Row::Threshold => "Threshold",
            Row::PreRoll => "Pre-roll",
            Row::End => "End",
            Row::Length => "Length",
            Row::Tail => "Tail",
            Row::Silence => "Silence for",
            Row::SkipBack => "Skip back",
            Row::Onto => "Onto",
            Row::FlattenVelocity => "Flatten velocity",
            Row::FlattenHold => "Flatten hold",
        }
    }

    fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// Row `row`'s value as text; `sources` names the sources the app
/// offers.
pub fn value_text(s: &RecSettings, row: Row, sources: &[String]) -> String {
    match row {
        Row::Source => sources
            .get(s.source)
            .cloned()
            .unwrap_or_else(|| "Line in".to_string()),
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
        Row::Tail => match s.tail {
            TailAt::Cut => "Cut",
            TailAt::Ring => "Ring out",
            TailAt::Loop => "Loop",
        }
        .to_string(),
        Row::Silence => ms_text(s.silence_ms),
        Row::FlattenVelocity => s.flatten_velocity.to_string(),
        Row::FlattenHold => ms_text(s.flatten_hold_ms),
        Row::SkipBack => s.skip.label(),
        Row::Onto => match s.onto {
            Onto::NextEmpty => "Next empty",
            Onto::ThisPad => "This pad",
        }
        .to_string(),
    }
}

fn ms_text(ms: u32) -> String {
    if ms >= 1000 && ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        format!("{ms} ms")
    }
}

fn shift(i: usize, delta: i32, len: usize) -> usize {
    (i as i32 + delta).clamp(0, len as i32 - 1) as usize
}

fn step_in<T: PartialEq + Copy>(list: &[T], now: T, delta: i32) -> T {
    let i = list.iter().position(|x| *x == now).unwrap_or(0);
    list[shift(i, delta, list.len())]
}

/// `s` with row `row` stepped by `delta` steps, among `sources` sources
/// for the Source row.
pub fn stepped(s: &RecSettings, row: Row, delta: i32, sources: usize) -> RecSettings {
    let mut s = *s;
    match row {
        Row::Source => s.source = shift(s.source, delta, sources.max(1)),
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
        Row::Tail => s.tail = step_in(&[TailAt::Cut, TailAt::Ring, TailAt::Loop], s.tail, delta),
        Row::Silence => s.silence_ms = step_in(&SILENCES_MS, s.silence_ms, delta),
        Row::SkipBack => s.skip = step_in(&SKIPS, s.skip, delta),
        Row::Onto => s.onto = step_in(&[Onto::NextEmpty, Onto::ThisPad], s.onto, delta),
        Row::FlattenVelocity => {
            s.flatten_velocity = step_in(&VELOCITIES, s.flatten_velocity, delta)
        }
        Row::FlattenHold => s.flatten_hold_ms = step_in(&HOLDS_MS, s.flatten_hold_ms, delta),
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
    /// The sources it can record from, by name; the first is the line
    /// input.
    pub sources: Vec<String>,
    /// The source picked comes in (for the line input: it is on, on the
    /// Looper tab's MENU; a point in the mix always does).
    pub live: bool,
    /// The last take, said in a line.
    pub last: String,
    /// Flatten can resample the pad picked now (it has a sound that plays
    /// on its own channel, and there is a pad to put it on).
    pub can_flatten: bool,
    /// A Flatten running, said in a line (the bar's title while it runs).
    pub flattening: Option<String>,
}

/// What a touch or control on the tab asks for. The app does it.
#[derive(Clone, Debug, PartialEq)]
pub enum RecAction {
    /// Start a take with [`RecTab::settings`].
    Record,
    /// Stop the take (cancel one still waiting), or a Flatten.
    Stop,
    /// Keep what the settings' Skip back covers, now.
    SkipBack,
    /// The Source row picked source `i` of [`RecState::sources`].
    Source(usize),
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
    /// Resample the pad picked as it plays ([`RecState::can_flatten`]).
    Flatten,
    /// Open the Auto-sample mode ([`crate::auto_tab`]).
    Auto,
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
    /// A Flatten is running: the first button stops it too.
    flattening: bool,
    /// The sources' names, as last shown.
    sources: Vec<String>,
}

impl RecTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`. The strip shows input at `rate`.
    pub fn add(ui: &mut Ui, name: &str, st: &RecState, rate: u32) -> Result<RecTab, String> {
        crate::send::add(ui);
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
        let mut buttons = ButtonRow::new(&["Record", "Skip back", FLATTEN_LABEL, AUTO_LABEL]);
        buttons.primary = Some(0);
        ui.add(BUTTONS, buttons);
        ui.add(WHEEL, InfoCard::new("The row picked"));
        ui.add_page(name, LAYOUT)?;
        let mut tab = RecTab {
            name: name.to_string(),
            bank: None,
            settings: RecSettings::default(),
            recording: false,
            flattening: false,
            sources: st.sources.clone(),
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
                .map(|t| wave_tab::BUILT.contains(t) && (st.pad_name.is_some() || *t == TABS[TAB]))
                .collect();
            bar.title = match (&st.flattening, &target) {
                (Some(f), _) => f.clone(),
                (None, Some(p)) => format!("Record onto pad {p}"),
                (None, None) => "Record: no empty pad in this bank".to_string(),
            };
            bar.subtitle = if st.flattening.is_some() {
                format!(
                    "velocity {} · held {} · until {} of silence",
                    self.settings.flatten_velocity,
                    ms_text(self.settings.flatten_hold_ms),
                    ms_text(self.settings.silence_ms)
                )
            } else if st.last.is_empty() {
                match (&st.pad_name, self.settings.onto) {
                    (Some(n), Onto::ThisPad) => format!("replacing {n}"),
                    _ if self.settings.source == 0 => format!(
                        "{} · from the line input",
                        value_text(&self.settings, Row::Input, &st.sources)
                    ),
                    _ => format!(
                        "{} · resampling {}",
                        value_text(&self.settings, Row::Input, &st.sources),
                        value_text(&self.settings, Row::Source, &st.sources)
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
                value: value_text(&self.settings, *r, &st.sources),
            })
            .collect();
        let title = if st.flattening.is_some() {
            format!("Pad {} channel", pad_text(self.bank, st.pad))
        } else if self.settings.source == 0 {
            "Line input".to_string()
        } else {
            value_text(&self.settings, Row::Source, &st.sources)
        };
        if let Some(v) = ui.get_mut::<InputView>(VIEW) {
            v.title = title;
        }
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
            list.right = target.map(|p| format!("onto {p}")).unwrap_or_default();
        }
        self.flattening = st.flattening.is_some();
        let can = st.live && st.target.is_some() && !self.flattening;
        let recording = self.recording || self.flattening;
        let skip = self.settings.skip.label();
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels = vec![
                if recording { "Stop" } else { "Record" }.to_string(),
                format!("Skip back {skip}"),
                FLATTEN_LABEL.to_string(),
                AUTO_LABEL.to_string(),
            ];
            b.enabled = vec![
                recording || can,
                can && !recording,
                st.can_flatten && !recording,
                !recording,
            ];
            let cannot = if !st.live {
                OFF_NOTE
            } else if st.target.is_none() {
                "No empty pad left for a take"
            } else {
                "Wait for Flatten to finish"
            };
            b.why = vec![
                cannot.to_string(),
                if recording {
                    "Stop the take first"
                } else {
                    cannot
                }
                .to_string(),
                if recording {
                    "Stop the take first"
                } else {
                    "Flatten needs a pad with a sound, and an empty pad to put it on"
                }
                .to_string(),
                "Stop the take first".to_string(),
            ];
        }
        self.sources = st.sources.clone();
        self.show_wheel(ui);
    }

    fn show_wheel(&self, ui: &mut Ui) {
        let row = self.row(ui);
        let text = format!(
            "{}: {}",
            row.label(),
            value_text(&self.settings, row, &self.sources)
        );
        if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
            card.text = text;
        }
    }

    /// Reads what came into `input` since the last call (`dt` seconds ago)
    /// and marks the bar lines, what Skip back would keep, and `rec` (the
    /// take, if one is being made) on the strip. `live`: the source
    /// comes in.
    pub fn feed(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rec: Option<&Recording>,
        live: bool,
        dt: f32,
    ) {
        let was = self.recording;
        self.recording = rec.is_some_and(|r| {
            matches!(r.progress(), Progress::Waiting | Progress::Recording { .. })
        });
        let settings = self.settings;
        // Redrawn only when its picture changes (InputView::look): the view
        // spans most of the panel's rows.
        let area = ui.rect_of(VIEW);
        let Some(v) = ui.get_mut_quiet::<InputView>(VIEW) else {
            return;
        };
        let before = area.map(|r| v.look(r));
        v.update(input, dt);
        let now = v.now;
        let rate = input.rate;
        v.bars = (0..BAR_LINES).map_while(|n| input.bars.back(n)).collect();
        v.threshold = (settings.start == StartAt::Threshold || settings.end == EndAt::Silence)
            .then(|| settings.level());
        v.skip = match settings.skip {
            _ if !live => None,
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
            Skip::Bars(n) => match input.bars_back(n as usize) {
                Some((start, end)) if start >= input.audio.oldest() => Some(Span {
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
        v.note = if !live {
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
            // Whole seconds: in hundredths the caption, and so the view,
            // changed at every frame until the buffer was full.
            let rate64 = u64::from(rate.max(1));
            match (held + rate64 / 2) / rate64 {
                0 => format!("{} held", duration_text(held, rate)),
                secs => format!("{secs} s held"),
            }
        };
        v.right = match input.bars.bar_frames() {
            Some(bar) => format!(
                "bar lines every {}",
                duration_text(bar.round() as u64, rate)
            ),
            None => "transport stopped".to_string(),
        };
        if area.zip(before).is_none_or(|(r, b)| v.look(r) != b) {
            ui.get_mut::<InputView>(VIEW);
        }
        if was != self.recording {
            if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
                b.labels[0] = if self.recording { "Stop" } else { "Record" }.to_string();
                if self.recording {
                    b.enabled[0] = true;
                    b.enabled[1] = false;
                    b.enabled[2] = false;
                    b.enabled[3] = false;
                }
            }
        }
    }

    /// No input to show (the line input is off, or just turned on): the
    /// picture says so and the strip starts again.
    ///
    /// Called every frame while that lasts, so it changes the widgets (and
    /// has them redrawn) only the first time.
    pub fn idle(&mut self, ui: &mut Ui, live: bool) {
        self.recording = false;
        let note = if live { "Starting the input" } else { OFF_NOTE };
        let shown = ui.get::<InputView>(VIEW).is_none_or(|v| {
            v.strip.read_to() == 0
                && v.skip.is_none()
                && v.take.is_none()
                && v.bars.is_empty()
                && v.threshold.is_none()
                && v.meter_db == [f32::NEG_INFINITY; 2]
                && v.note == note
                && v.caption.is_empty()
                && v.right.is_empty()
        });
        if !shown {
            if let Some(v) = ui.get_mut::<InputView>(VIEW) {
                v.strip.reset();
                v.skip = None;
                v.take = None;
                v.bars.clear();
                v.threshold = None;
                v.meter_db = [f32::NEG_INFINITY; 2];
                v.note = note.to_string();
                v.caption.clear();
                v.right.clear();
            }
        }
        let third = ui
            .get::<ButtonRow>(BUTTONS)
            .is_some_and(|b| b.enabled.get(2) == Some(&true));
        let enabled = vec![false, false, third, true];
        let buttons = ui.get::<ButtonRow>(BUTTONS).is_none_or(|b| {
            b.labels.first().is_some_and(|l| l == "Record") && b.enabled == enabled
        });
        if !buttons {
            if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
                b.labels[0] = "Record".to_string();
                b.enabled = enabled;
            }
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change the settings or what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &RecState) -> Option<RecAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(RecAction::Tab(*i)),
            (BUTTONS, EventKind::Tab(0)) => Some(if self.recording || self.flattening {
                RecAction::Stop
            } else {
                RecAction::Record
            }),
            (BUTTONS, EventKind::Tab(1)) => Some(RecAction::SkipBack),
            (BUTTONS, EventKind::Tab(2))
                if !self.recording && !self.flattening && st.can_flatten =>
            {
                Some(RecAction::Flatten)
            }
            (BUTTONS, EventKind::Tab(3)) if !self.recording && !self.flattening => {
                Some(RecAction::Auto)
            }
            (LIST, EventKind::Step { index, delta }) => {
                let row = *Row::ALL.get(*index)?;
                self.step(ui, st, row, *delta)
            }
            (LIST, EventKind::Select(_)) => {
                self.show_wheel(ui);
                None
            }
            _ => None,
        }
    }

    /// The data wheel: steps the row picked by `delta` detents. A new
    /// source is for the app to start.
    pub fn wheel(&mut self, ui: &mut Ui, st: &RecState, delta: i32) -> Option<RecAction> {
        let row = self.row(ui);
        self.step(ui, st, row, delta)
    }

    fn step(&mut self, ui: &mut Ui, st: &RecState, row: Row, delta: i32) -> Option<RecAction> {
        if row == Row::Source && (self.recording || self.flattening) {
            // A take keeps its source until it ends.
            if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
                card.text = "Source: stop the take first".to_string();
            }
            return None;
        }
        let was = self.settings.source;
        self.settings = stepped(&self.settings, row, delta, st.sources.len());
        self.update(ui, st);
        (self.settings.source != was).then_some(RecAction::Source(self.settings.source))
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
        let names = ["Line in".to_string(), "Master".to_string()];
        let s = RecSettings::default();
        assert_eq!(value_text(&s, Row::Start, &names), "Now");
        assert_eq!(value_text(&s, Row::Source, &names), "Line in");
        let s = stepped(&s, Row::Start, 1, 2);
        assert_eq!(s.start, StartAt::Threshold);
        let s = stepped(&s, Row::Start, 5, 2);
        assert_eq!(s.start, StartAt::NextBar, "stops at the end");
        let s = stepped(&s, Row::Threshold, 2, 2);
        assert_eq!(s.threshold_db, -24.0);
        let s = stepped(&s, Row::SkipBack, 5, 2);
        assert_eq!(s.skip, Skip::Bars(2));
        assert_eq!(value_text(&s, Row::SkipBack, &names), "2 bars");
        assert_eq!(Skip::Seconds(60).label(), "1 min");
        assert_eq!(
            value_text(&stepped(&s, Row::Silence, -1, 2), Row::Silence, &names),
            "500 ms"
        );
        let s = stepped(&s, Row::Source, 3, 2);
        assert_eq!(
            value_text(&s, Row::Source, &names),
            "Master",
            "stops at the last"
        );
        assert_eq!(stepped(&s, Row::Source, -1, 2).source, 0);
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
    fn a_take_of_bars_is_cut_rings_out_or_loops() {
        let names = ["Line in".to_string()];
        let mut s = RecSettings {
            end: EndAt::Length,
            silence_ms: 500,
            ..RecSettings::default()
        };
        assert_eq!(s.tail_at(44_100), Tail::Cut);
        assert_eq!(value_text(&s, Row::Tail, &names), "Cut");
        s = stepped(&s, Row::Tail, 1, 1);
        assert_eq!(value_text(&s, Row::Tail, &names), "Ring out");
        let Tail::Ring(ring) = s.tail_at(44_100) else {
            panic!("rings out");
        };
        assert_eq!((ring.quiet, ring.max), (22_050, 882_000));
        assert!((ring.level - 0.001).abs() < 1e-6, "-60 dB");
        s = stepped(&s, Row::Tail, 5, 1);
        assert_eq!(s.tail, TailAt::Loop, "stops at the end");
        assert_eq!(
            s.tail_at(44_100),
            Tail::Loop {
                level: ring.level,
                quiet: 22_050
            }
        );
        assert!(s.in_bars());
        // A take that ends on a press or on silence has no tail.
        s.end = EndAt::Silence;
        assert_eq!(s.tail_at(44_100), Tail::Cut);
        assert!(!s.in_bars());
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
