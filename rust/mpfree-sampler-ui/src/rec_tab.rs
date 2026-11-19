//! The recorder's settings and how they step: once the pad editor's Rec
//! tab, whose controls now live on the one Sample page (steps 2a to 2f):
//! the recorder row's Source, Starts and Ends cards
//! ([`crate::recorder`]), its Input section while REC is on (Channels,
//! Onto, Tail, Gain, Threshold, Pre-roll, Silence for, Monitor), Skip back
//! in Sample Recall's From ([`crate::recall`]), and Flatten and
//! Auto-sample as Sources. The Rec tab itself went in step 2f, part 4.
//!
//! - [`RecSettings`] is how a take starts (at once, over the threshold
//!   keeping a pre-roll, or on the next bar line) and ends (on STOP, on
//!   silence, or after a number of bars), what it does after its bars
//!   (cut, ring out until the Silence time of quiet, or loop), the input's
//!   gain, which channels it keeps and where it goes (Onto), and how a
//!   Flatten plays its pad. [`RecSettings::recording`] and
//!   [`RecSettings::flatten_recording`] make the [`Recording`].
//! - [`Row`] names each setting, [`value_text`] says it, [`stepped`] steps
//!   it and [`target`] finds the pad a take goes onto.

use mpfree_sampler::record::{End, Input, Pick, Recording, RingOut, Start, Tail};

use crate::panel::{Control, Look};

/// Flatten's level for silence: under it for the Silence time, the sound
/// has ended (and a sound starts when it goes over it).
pub const FLATTEN_SILENCE_DB: f32 = -60.0;

/// A take that rings out after its bars ends under this, as a Flatten's
/// sound does, after the Silence time.
pub const TAIL_SILENCE_DB: f32 = FLATTEN_SILENCE_DB;

/// The longest a take rings on after its bars.
pub const TAIL_MAX_SECONDS: u32 = 20;

/// The longest take: three minutes (about 64 MB of stereo).
pub const MAX_TAKE_SECONDS: u32 = 180;

const THRESHOLDS_DB: [f32; 15] = [
    -60.0, -54.0, -48.0, -42.0, -36.0, -33.0, -30.0, -27.0, -24.0, -21.0, -18.0, -15.0, -12.0,
    -9.0, -6.0,
];
const PRE_ROLLS_MS: [u32; 8] = [0, 5, 10, 20, 50, 100, 200, 500];
/// Why Gain is faint while the source is a point in the mix.
pub const GAIN_LINE_ONLY: &str =
    "Gain is the line input's: a point in the mix records as you hear it";

/// Gain's range, dB, a dB a step.
pub const GAIN_MIN_DB: f32 = -24.0;
pub const GAIN_MAX_DB: f32 = 24.0;
const LENGTHS_BARS: [u32; 6] = [1, 2, 4, 8, 16, 32];
const SILENCES_MS: [u32; 5] = [250, 500, 1000, 2000, 4000];
const VELOCITIES: [u8; 9] = [16, 32, 48, 64, 80, 96, 110, 120, 127];
const HOLDS_MS: [u32; 7] = [100, 250, 500, 1000, 2000, 4000, 8000];

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

/// A span of seconds or whole bars, as said: "10 s", "1 min", "2 bars"
/// (the recorder's Ends card's After N bars; once Skip back's reach).
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

/// Where a take goes when it stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Onto {
    /// It stays on the Sample page, on no pad, until it is put somewhere
    /// (the one Sample page's decision 1: recording never fills a pad the
    /// musician didn't pick).
    Stay,
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
    /// How loud a take records, dB ([`GAIN_MIN_DB`] to [`GAIN_MAX_DB`]):
    /// the input is turned up or down before the meter, the threshold
    /// and the take see it.
    pub gain_db: f32,
    pub start: StartAt,
    pub threshold_db: f32,
    pub pre_roll_ms: u32,
    pub end: EndAt,
    pub bars: u32,
    /// After a length: cut, ring out or loop.
    pub tail: TailAt,
    pub silence_ms: u32,
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
            gain_db: 0.0,
            start: StartAt::Now,
            threshold_db: -30.0,
            pre_roll_ms: 20,
            end: EndAt::Press,
            bars: 4,
            tail: TailAt::Cut,
            silence_ms: 1000,
            onto: Onto::Stay,
            flatten_velocity: 127,
            flatten_hold_ms: 1000,
        }
    }
}

fn ms_frames(ms: u32, rate: u32) -> u64 {
    u64::from(ms) * u64::from(rate) / 1000
}

impl RecSettings {
    /// The Gain as a linear factor.
    pub fn gain(&self) -> f32 {
        10f32.powf(self.gain_db / 20.0)
    }

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

    /// The Tail setting as a recording's tail, whatever the end (a take
    /// already running hears it: [`Recording::set_tail`]).
    pub fn tail_as(&self, rate: u32) -> Tail {
        RecSettings {
            end: EndAt::Length,
            ..*self
        }
        .tail_at(rate)
    }

    /// The Silence for time, in frames at `rate`.
    pub fn silence_frames(&self, rate: u32) -> u64 {
        ms_frames(self.silence_ms, rate)
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

/// The settings and the buttons, in the panel's order (which the wheel
/// walks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Source,
    Input,
    /// How loud a take records: a gain on what comes in, before the
    /// meter, the threshold and the take (one-page step 2d).
    Gain,
    Start,
    Threshold,
    PreRoll,
    End,
    Length,
    Silence,
    Tail,
    Onto,
    FlattenVelocity,
    FlattenHold,
}

impl Row {
    pub const ALL: [Row; 13] = [
        Row::Source,
        Row::Input,
        Row::Gain,
        Row::Start,
        Row::Threshold,
        Row::PreRoll,
        Row::End,
        Row::Length,
        Row::Silence,
        Row::Tail,
        Row::Onto,
        Row::FlattenVelocity,
        Row::FlattenHold,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Source => "Source",
            Row::Input => "Channels",
            Row::Gain => "Gain",
            Row::Start => "Start",
            Row::Threshold => "Threshold",
            Row::PreRoll => "Pre-roll",
            Row::End => "End",
            Row::Length => "Length",
            Row::Tail => "Tail",
            Row::Silence => "Silence for",
            Row::Onto => "Onto",
            Row::FlattenVelocity => "Flatten velocity",
            Row::FlattenHold => "Flatten hold",
        }
    }

    /// Its place in [`Row::ALL`]: the list's row.
    pub fn index(self) -> usize {
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
        Row::Gain => gain_text(s.gain_db),
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
        Row::Onto => match s.onto {
            Onto::Stay => "Stay here",
            Onto::NextEmpty => "Next empty",
            Onto::ThisPad => "This pad",
        }
        .to_string(),
    }
}

/// Gain as the knob says it: "+0 dB", "-6 dB", "+12 dB".
pub fn gain_text(db: f32) -> String {
    format!("{:+.0} dB", db.round() + 0.0)
}

/// Where Gain's knob is lit to, 0 to 1 (0 dB in the middle).
fn gain_place(db: f32) -> f32 {
    (db - GAIN_MIN_DB) / (GAIN_MAX_DB - GAIN_MIN_DB)
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

/// The Onto row's choices, in order.
pub const ONTOS: [Onto; 3] = [Onto::Stay, Onto::NextEmpty, Onto::ThisPad];

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
        Row::Gain => s.gain_db = (s.gain_db.round() + delta as f32).clamp(GAIN_MIN_DB, GAIN_MAX_DB),
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
        Row::End => {
            // One list, so the Ends card reaches every length (one-page
            // step 2f, part 4: the Rec page's Length row goes): On STOP,
            // On silence, then After 1 bar to After 32 bars.
            let now = match s.end {
                EndAt::Press => 0,
                EndAt::Silence => 1,
                EndAt::Length => 2 + LENGTHS_BARS.iter().position(|b| *b == s.bars).unwrap_or(2),
            };
            match shift(now, delta, 2 + LENGTHS_BARS.len()) {
                0 => s.end = EndAt::Press,
                1 => s.end = EndAt::Silence,
                i => {
                    s.end = EndAt::Length;
                    s.bars = LENGTHS_BARS[i - 2];
                }
            }
        }
        Row::Length => s.bars = step_in(&LENGTHS_BARS, s.bars, delta),
        Row::Tail => s.tail = step_in(&[TailAt::Cut, TailAt::Ring, TailAt::Loop], s.tail, delta),
        Row::Silence => s.silence_ms = step_in(&SILENCES_MS, s.silence_ms, delta),
        Row::Onto => s.onto = step_in(&ONTOS, s.onto, delta),
        Row::FlattenVelocity => {
            s.flatten_velocity = step_in(&VELOCITIES, s.flatten_velocity, delta)
        }
        Row::FlattenHold => s.flatten_hold_ms = step_in(&HOLDS_MS, s.flatten_hold_ms, delta),
    }
    s
}

/// Where `now` is in `list`, 0 to 1 along it (a knob's place).
fn place_in<T: PartialEq>(list: &[T], now: &T) -> f32 {
    let i = list.iter().position(|x| x == now).unwrap_or(0);
    i as f32 / (list.len().max(2) - 1) as f32
}

/// Gain's, Threshold's or Pre-roll's knob, lit to where `s` has it among
/// its steps (another row: a chip). Gain's is lit from the middle, 0 dB.
pub(crate) fn knob(s: &RecSettings, row: Row) -> Control {
    if row == Row::Gain {
        return gain_knob(s.gain_db);
    }
    let (pos, len) = match row {
        Row::Threshold => (
            place_in(&THRESHOLDS_DB, &s.threshold_db),
            THRESHOLDS_DB.len(),
        ),
        Row::PreRoll => (place_in(&PRE_ROLLS_MS, &s.pre_roll_ms), PRE_ROLLS_MS.len()),
        Row::Silence => (place_in(&SILENCES_MS, &s.silence_ms), SILENCES_MS.len()),
        _ => return Control::new(Look::Chip),
    };
    Control::new(Look::Knob {
        pos,
        steps: len.saturating_sub(1) as u32,
        bipolar: false,
    })
}

/// Gain's knob: lit from the middle (0 dB), a dB a step.
fn gain_knob(db: f32) -> Control {
    Control::new(Look::Knob {
        pos: gain_place(db),
        steps: (GAIN_MAX_DB - GAIN_MIN_DB) as u32,
        bipolar: true,
    })
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
        assert_eq!(Skip::Seconds(60).label(), "1 min");
        assert_eq!(Skip::Bars(2).label(), "2 bars");
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
