//! The looper in the pad player: the line input (the MPC One's capture
//! device) and the `mpfree-looper` engine on the transport.
//!
//! - [`LineIn`] keeps the capture stream: closed until the Prefs row "Line
//!   input" turns it on (never on at start), opened at the stock settings
//!   (stereo S32_LE, 44.1 kHz, 128-frame periods, a 256-frame buffer) and
//!   read without waiting, so the playback loop never blocks on it. What
//!   arrives goes through a small FIFO; a period with too little input is
//!   filled with silence and counted.
//! - [`LoopHost`] runs the looper on the transport's block each period and
//!   gives the loop to the mixer's Loop track and the line input to its
//!   input strip, which is heard when Monitor is on (see `mixing.rs`). It
//!   records from a
//!   [`LoopSource`]: the pad player's own sound (the default, which opens
//!   nothing), the line input, or a mix (the master or one strip). It logs a `looper:` line for each press and
//!   each change, and builds the loop's waveform for the Looper tab a slice
//!   at a time.
//! - [`Printer`] keeps each loop's pitch at another tempo: when a loop is
//!   playing at a tempo other than its own, the host bounces it
//!   ([`Looper::bounce_print`]), a worker thread stretches the bounce to
//!   the loop's length at the tempo (`mpfree_looper::print`, on the
//!   sampler's stretch engine), and the print goes back to the looper
//!   ([`Looper::print`]), which plays it in place of the layers. Until it
//!   is ready (a fraction of a second on the workstation, more on the MPC)
//!   the loop plays like tape. Buffers are made and dropped on the worker.
//!   Overdub over a print fits the loop to the tempo first (see
//!   `mpfree-looper`).
//!
//! Which jacks the two capture channels are is not verified (line in L/R is
//! the likely reading). Opening the capture device is a state change on the
//! MPC: it waits for the owner's go in the round 6 runbook.

use crate::alsa;
use crate::time::{Allowed, Answer, TimeHost, TimingChange};
use mpfree_looper::close::{self, Bars, Boundary, Start, Tempo, BEAT_FITS};
use mpfree_looper::find::{find, FindJob};
use mpfree_looper::print::{frames_at, PrintJob};
use mpfree_looper::save::Saved;
use mpfree_looper::{
    Bounce, Candidate, Clock, Command, Edit, FreeClock, History, LayerCommand, LayerInfo, LoopFile,
    Looper, Memory, NoteStart, Phase, Print, Reading, Shift, SmartEnd, SmartNote, SmartOutcome,
    Snap, SnapDone, SnapPlan, Starts, Stats, Status, Step, Sure, Take, HEARD_SECONDS, LOOP_MAX_BPM,
    LOOP_MIN_BPM, MAX_LAYERS, MAX_LOOPS,
};
use mpfree_seq::loops::{LoopNow, LoopPlan};
use mpfree_transport::time::{Tick, PPQ};
use mpfree_transport::{Block, Span, State, Swing};
use mpfree_ui::wave::{Peaks, WaveColumn};
use std::ops::Range;
use std::sync::mpsc::{
    channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError, TrySendError,
};
use std::sync::Arc;

pub const PERIOD: usize = 128;

/// The Length setting's "Pattern", as the app sends it in
/// `Command::SetBars(Some(PATTERN_BARS))`: a new loop takes the length of
/// the pattern playing ([`LoopHost::set_pattern_bars`]), so it always
/// fits the beat. The looper itself never sees it.
pub const PATTERN_BARS: u32 = 0;

/// The longest pattern Length: Pattern follows; a longer one records free.
pub const PATTERN_MAX_BARS: u32 = 32;
/// Frames the FIFO holds: 8 periods. More than 3 periods waiting means the
/// input runs ahead; the oldest frames are dropped down to 2 periods.
const FIFO: usize = 8 * PERIOD;
/// The capture buffer the stream must get (2 periods).
pub const CAPTURE_BUFFER: u32 = 2 * PERIOD as u32;
/// Read errors in a row (not overruns) before the input is closed.
const MAX_ERRORS: u32 = 3;
/// Columns of the loop's waveform.
pub const WAVE_COLUMNS: usize = 600;
/// Layer-frames of the loop turned into waveform columns per period,
/// whatever the loop's length or layers: a small fixed share of the
/// period (about 6 µs on the build workstation, so roughly 1 to 2 % of a
/// period on the MPC), so the wave never makes the sound late. A two-bar
/// loop at 120 BPM takes a quarter of a second. All the loops' layers
/// together hold at most `LoopHost::new`'s seconds, so the longest wave
/// takes under 4 s a minute of loop memory (Prefs' largest, 20 minutes,
/// filled by one loop: about 80 s, drawn as it goes).
const WAVE_BUDGET: usize = 2048;
/// The fewest and most frames of each layer read in a period (16 layers
/// of the fewest are the budget).
const WAVE_MIN: usize = WAVE_BUDGET / MAX_LAYERS;
const WAVE_SLICE: usize = WAVE_BUDGET;
/// Periods between the live waveform's updates while the first pass
/// records (about 35 ms).
const LIVE_EVERY: u64 = 12;
/// Waves the audio thread makes again in place (see `WavePool`).
const WAVES: usize = 4;
/// Columns of a loop's small wave on its Clips grid cells.
pub use mpfree_time_ui::widgets::CELL_WAVE;
/// Periods between looks for a loop whose small wave is out of date
/// (about 23 ms).
const CELL_WAVE_EVERY: u64 = 8;
/// Frames of what the looper heard in each of the peaks What you played
/// draws from.
const HEARD_CHUNK: u64 = 256;
/// Columns of What you played: the last [`HEARD_BARS`] bars heard, or
/// more (up to [`HEARD_MAX_BARS`]) when its window takes more.
pub const HEARD_COLS: usize = 240;
pub const HEARD_BARS: u32 = 8;
/// The most bars What you played shows: room for its window to take four
/// times an 8-bar pattern, as far as the looper's 60 s go.
pub const HEARD_MAX_BARS: u32 = 32;
/// Bars Capture takes when the loop's Length is Free.
pub const CAPTURE_FREE_BARS: u32 = 4;

/// What the looper heard over the last [`HEARD_BARS`] bars (or more), for
/// an empty loop's What you played: its level, the bar lines in it, and
/// what Capture into the picked loop would take now (its window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeardWave {
    /// Each column's peak, oldest first, dBFS (-inf silent).
    pub cols: [f32; HEARD_COLS],
    /// The bar lines in it, oldest first: each bar's number (from 1) and
    /// where it starts, 0 to 1; `n_lines` of them.
    pub lines: [(i64, f32); HEARD_MAX_BARS as usize + 1],
    pub n_lines: usize,
    /// The bars the window grows and shrinks by: the pattern's length
    /// while Length is Pattern and a pattern plays (`pattern` true), else
    /// the Length's bars, else 1.
    pub unit: u32,
    pub pattern: bool,
    /// What Capture takes now: where it starts and ends (0 to 1), its
    /// first bar's number and how many bars.
    pub take: Option<(f32, f32, i64, u32)>,
    /// Capture now waits for the bar line coming (the song is in a bar's
    /// last beat): it takes the bars up to that line.
    pub waits: bool,
}

impl Default for HeardWave {
    fn default() -> Self {
        HeardWave {
            cols: [f32::NEG_INFINITY; HEARD_COLS],
            lines: [(0, 0.0); HEARD_MAX_BARS as usize + 1],
            n_lines: 0,
            unit: 1,
            pattern: false,
            take: None,
            waits: false,
        }
    }
}

/// "1 bar", "2 bars".
/// How a first pass with Tempo from the first loop records: "free", or
/// with Length's bars set, "2 bars at the tempo you play".
pub fn free_words(bars: Option<u32>) -> String {
    match bars {
        Some(n) if n > 0 => format!("{} at the tempo you play", bars_words_n(n)),
        _ => "free".to_string(),
    }
}

/// The shortest a free first pass closes at, before [`LOOP_MAX_BPM`]: "a
/// bar", or Length's "2 bars".
pub fn shortest_words(bars: Option<u32>) -> String {
    match bars {
        Some(n) if n > 1 => format!("{n} bars"),
        _ => "a bar".to_string(),
    }
}

/// How a free first loop's length read, for the log: "1 bar at 96.00
/// BPM", and ", as Length set" when the bars were Length's, or what the
/// beat played into it said.
pub fn reading_words(r: &Reading) -> String {
    format!(
        "{} at {:.2} BPM{}",
        bars_words_n(r.bars),
        r.bpm,
        if r.said.is_some() && !r.overruled() {
            ", as Length set"
        } else {
            match r.beat {
                None => "",
                Some(_) if !r.bars.is_power_of_two() => ", from the beat you played",
                Some(f) if f >= BEAT_FITS => ", which the beat you played fits",
                Some(_) => ", which the beat you played fits only loosely",
            }
        }
    )
}

/// What the screen should hear about a reading, besides its bars and
/// tempo: a close call and the other reading, or a Length it couldn't
/// keep. None for a clear reading.
pub fn reading_note(r: &Reading) -> Option<String> {
    if let Some(n) = r.said.filter(|_| r.overruled()) {
        return Some(format!(
            "Length was {}, but that would be slower than {LOOP_MIN_BPM:.0} BPM, \
             so the bars were read from the loop",
            bars_words_n(n)
        ));
    }
    match (r.sure, r.other) {
        (Sure::Ambiguous, Some((b, bpm))) => Some(format!(
            "a close call: it could be {} at {bpm:.2} BPM",
            bars_words_n(b)
        )),
        _ => None,
    }
}

pub fn bars_words_n(bars: u32) -> String {
    if bars == 1 {
        "1 bar".to_string()
    } else {
        format!("{bars} bars")
    }
}

/// Bars `first..first + bars`, as words: "bar 3", "bars 10 and 11",
/// "bars 8 to 11".
pub fn bars_words(first: i64, bars: u32) -> String {
    match bars {
        0 | 1 => format!("bar {first}"),
        2 => format!("bars {first} and {}", first + 1),
        b => format!("bars {first} to {}", first + i64::from(b) - 1),
    }
}

/// What the pads play: an instrument, never a track (Jay, 20 Nov 2026:
/// "Instruments generate sound. Tracks organize and record performances.
/// The pads are controllers that play instruments."). One place in the
/// app says which (`main.rs`, `pads_instrument`); the mixer says where it
/// sounds ([`crate::mixing::MixHost::instrument_strip`]), separately. The
/// instrument redesign's identities (its step 2) take its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instrument {
    /// The instrument track `t` (0 based) holds of its own, which plays
    /// into that track's channel.
    Own(usize),
    /// Kit `k` of the sampler (0: kit 1, on the pads' channels and the
    /// Drums channel; another on the channel of the first track that
    /// plays it).
    Kit(usize),
    /// The shared synth, on its Synth channel: the Synth kit picked, or
    /// Play on the pads (the Synth tab).
    Synth,
    /// An instrument outside mpfree, over MIDI out: nothing of it reaches
    /// the mix.
    MidiOut,
}

impl Instrument {
    /// Its name for the log.
    pub fn name(self) -> String {
        match self {
            // A free place's (the instrument redesign's step 6): a synth
            // picked for the pads, which no track has.
            Instrument::Own(t) if t >= crate::instruments::TRACKS => {
                "the pads' own synth".to_string()
            }
            Instrument::Own(t) => format!("Track {}'s own instrument", t + 1),
            Instrument::Kit(k) => format!("Kit {}", k + 1),
            Instrument::Synth => "the synth".to_string(),
            Instrument::MidiOut => "MIDI out".to_string(),
        }
    }
}

/// Why Records from: Pads records silence while the pads play
/// MIDI out.
pub const PADS_ON_MIDI_OUT: &str = "the pads play MIDI out, which sounds outside mpfree";

/// What a take records from, in words for the log: the source and, for the
/// pads, their instrument.
pub(crate) fn source_words(s: LoopSource, pads: Option<Instrument>) -> String {
    match (s, pads) {
        (LoopSource::Pads, Some(Instrument::MidiOut)) => format!("silence ({PADS_ON_MIDI_OUT})"),
        (LoopSource::Pads, Some(i)) => format!("the pads ({})", i.name()),
        (LoopSource::Pads, None) => "the pads".to_string(),
        (LoopSource::Line, _) => "the line input".to_string(),
        (LoopSource::Mix(mpfree_fx::StripId::Master), _) => "the master".to_string(),
        (LoopSource::Mix(m), _) => crate::mixing::name(m),
        (LoopSource::Track(t), _) => match t.from {
            TrackFrom::Line(i) => {
                format!("the track's input, {}", crate::audiotracks::input_words(i))
            }
            TrackFrom::Mix(m, p) => format!(
                "the track's input, {} {} its effects",
                if m == mpfree_fx::StripId::Master {
                    "the master".to_string()
                } else {
                    crate::mixing::name(m)
                },
                if p == mpfree_fx::CapturePoint::BeforeInserts {
                    "before"
                } else {
                    "after"
                }
            ),
        },
    }
}

/// Where the looper records from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopSource {
    /// The pads: what the pads play ([`Instrument`]), on the
    /// channel it plays into, after that channel's effects and fader
    /// ([`crate::mixing::MixHost::instrument_strip`]). So a pattern on
    /// another instrument stays out; clips on the same one are heard with
    /// it (the name says so). Opens nothing, and is heard in time like a
    /// mix.
    #[default]
    Pads,
    /// The MPC's line input (Prefs, Line input).
    Line,
    /// A mix, picked on the Looper tab (Records from): the master (all
    /// you hear but the loops, before the master's effects and fader,
    /// which the loop gets when it plays), or one strip, a track or a
    /// submix, after its fader ([`crate::mixing::MixHost::loop_take`]).
    /// Heard in time, like the pads; never the Loop track itself.
    Mix(mpfree_fx::StripId),
    /// An audio track's own input, for a loop recorded into one of its
    /// cells (audio tracks phase 4b, behind Dev's switch): what a take on
    /// the track would record. Set by arming the cell, never saved.
    Track(TrackInput),
}

/// What a loop recorded into an audio track's cell records (audio tracks
/// phase 4b, [`LoopSource::Track`]): the track's own input, as its panel
/// sets it, and the track's own channel, which is never in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackInput {
    /// The track's own channel strip (`MixHost::track_channel`).
    pub channel: mpfree_fx::StripId,
    pub from: TrackFrom,
}

/// Where an audio track records from, for a loop in its cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackFrom {
    /// The line input as the track picks it; a mono pick (left, right or
    /// both summed, as the track's take sums them) fills both sides of
    /// the loop.
    Line(mpfree_audiotracks::Input),
    /// Inside the mix: a strip at a point, before its effects or after
    /// them and before its fader, as the track's take has it
    /// (`MixHost::audio_take`), where the mixer says a recording into the
    /// track takes it (`MixHost::track_loop_take`, `Mixer::capture_for`).
    Mix(mpfree_fx::StripId, mpfree_fx::CapturePoint),
}

/// The line input as `pick` takes it, into `left` and `right`: the take's
/// own sum for Sum (half of each), one jack on both sides for Left and
/// Right, both as they are for Stereo.
fn pick_line(
    pick: mpfree_audiotracks::Input,
    (l, r): (&[f32; PERIOD], &[f32; PERIOD]),
    left: &mut [f32; PERIOD],
    right: &mut [f32; PERIOD],
) {
    use mpfree_audiotracks::Input;
    for f in 0..PERIOD {
        (left[f], right[f]) = match pick {
            Input::Left => (l[f], l[f]),
            Input::Right => (r[f], r[f]),
            Input::Stereo => (l[f], r[f]),
            Input::Sum => {
                let v = 0.5 * (l[f] + r[f]);
                (v, v)
            }
        };
    }
}

impl LoopSource {
    /// Prefs' Record from steps through these (a mix is picked on the
    /// Looper tab, where the strips are named).
    pub const ALL: [LoopSource; 2] = [LoopSource::Pads, LoopSource::Line];

    pub fn name(self) -> &'static str {
        match self {
            LoopSource::Pads => "Pads",
            LoopSource::Line => "Line input",
            LoopSource::Mix(_) => "A mix (Looper tab)",
            LoopSource::Track(_) => "An audio track's input",
        }
    }

    /// Whether the sound comes from inside MPFree (the pads or a mix), in
    /// time with the transport but for the mix's own latency, which the
    /// host makes up: not the line input.
    pub fn inside(self) -> bool {
        !self.line()
    }

    /// Whether it is the line input: Line, or an audio track's line pick.
    pub fn line(self) -> bool {
        matches!(
            self,
            LoopSource::Line
                | LoopSource::Track(TrackInput {
                    from: TrackFrom::Line(_),
                    ..
                })
        )
    }
}

/// Prefs "Loop memory": the sound every loop and layer shares. It is set
/// aside, and kept in RAM, when MPFree starts (the sound can't wait for
/// memory), so a new size applies from the next start, like Sound cores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopMemory {
    /// A minute: what every build before live test 3 had.
    Min1,
    Min2,
    #[default]
    Min5,
    Min10,
    Min20,
}

impl LoopMemory {
    pub const ALL: [LoopMemory; 5] = [
        LoopMemory::Min1,
        LoopMemory::Min2,
        LoopMemory::Min5,
        LoopMemory::Min10,
        LoopMemory::Min20,
    ];

    pub fn minutes(self) -> u32 {
        match self {
            LoopMemory::Min1 => 1,
            LoopMemory::Min2 => 2,
            LoopMemory::Min5 => 5,
            LoopMemory::Min10 => 10,
            LoopMemory::Min20 => 20,
        }
    }

    pub fn seconds(self) -> u32 {
        60 * self.minutes()
    }

    /// The kept setting's value: the minutes.
    pub fn parse(v: &str) -> Option<LoopMemory> {
        let m = v.trim().parse::<u32>().ok()?;
        LoopMemory::ALL.into_iter().find(|l| l.minutes() == m)
    }

    /// What it costs, in whole megabytes, at `rate`.
    pub fn megabytes(self, rate: u32) -> usize {
        mpfree_looper::bytes_for(rate, self.seconds()).div_ceil(1_000_000)
    }

    /// Prefs' value for it: "5 min, 131 MB".
    pub fn row(self, rate: u32) -> String {
        format!("{} min, {} MB", self.minutes(), self.megabytes(rate))
    }
}

/// What the app asks of the looper and the line input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopAction {
    Command(Command),
    /// Record from the pads or the line input.
    Source(LoopSource),
    /// Hear the line input through the outputs.
    Monitor(bool),
    /// Open or close the capture device (a state change on the MPC).
    LineInput(bool),
    /// What you played's window moved or resized: the bars Capture takes,
    /// ending on the bar line that starts bar `.0` (None: the newest, and
    /// it keeps up), `.1` of them ([`LoopHost::set_capture_span`]).
    CaptureSpan(Option<i64>, u32),
    /// A layer's control (its speaker): the layer by its id
    /// ([`LayerInfo::id`]), which stays the same through undo and redo.
    Layer(u8, LayerCommand),
    /// Pick loop `n` (0 to [`MAX_LOOPS`] - 1): the buttons, the layers'
    /// controls and the screens act on it from the next period.
    Select(usize),
    /// Pick loop `n` to record into an empty Loop cell armed on the grid
    /// (`desk::arm_loop_cell`): as [`LoopAction::Select`], on the song's
    /// clock whatever Sync was last set to ([`LoopHost::select_for_cell`]).
    SelectForCell(usize),
    /// STOP with the transport stopped: an overdub waiting for PLAY is
    /// let go and an armed loop disarmed ([`LoopHost::stop_waiting`]).
    StopWaiting,
    /// SHIFT + STOP, or a set's Panic: the free loops stop too, at once
    /// ([`LoopHost::stop_free`]).
    StopFree,
    /// Listen: Some(None) plays the picked loop alone while the song is
    /// stopped, Some(Some(id)) one of its layers ([`LayerInfo::id`]);
    /// asked again, or None, it stops ([`LoopHost::listen`]).
    Listen(Option<Option<u8>>),
    /// The undo history: back `n` steps (below 0) or forward (above 0).
    Steps(i32),
    /// Loop `n`'s mute turned over without picking it, or the picked
    /// loop's (None): a muted loop stays silent when PLAY starts.
    ToggleMute(Option<usize>),
    /// Loop `n` muted (true) or let play, without picking it: a learned
    /// control's mute.
    Mute(usize, bool),
    /// Loop `n`'s solo turned over without picking it: while any loop
    /// with sound is soloed only the soloed ones play.
    ToggleSolo(usize),
    /// Loop `n`'s level (0 to 2, 1 as recorded), without picking it: the
    /// Looper tab's Q-Links and a ring held while the wheel turns.
    Level(usize, f32),
    /// Records from the other loops (Records from: Loops) or the input:
    /// Record on an empty loop resamples ([`LoopHost::resample`]).
    RecordLoops(bool),
    /// Layer `id` of loop `from` dragged onto loop `to` (an empty one: a
    /// loop of its own); `keep`: a copy, else it moves
    /// ([`Looper::copy_layer`]).
    CopyLayer {
        from: usize,
        id: u8,
        to: usize,
        keep: bool,
    },
    /// Capture: the last bars heard become a loop, or one capturing is
    /// called off ([`LoopHost::capture`]).
    Capture,
    /// Smart loop end's chip and the loop point view: a snap on the
    /// picked loop (Read as, [`LoopHost::snap`]).
    Snap(Snap),
    /// The loop point view's Snap, Use my tap, Keep my tap and Keep the
    /// snap: that side, and whether the chip is put away there
    /// ([`LoopHost::side`]).
    Side(Side, bool),
    /// The loop point view's A/B ([`LoopHost::set_ab`]).
    Ab(bool),
    /// The hint line's Snap anyway (true) or Not now
    /// ([`LoopHost::snap_anyway`], [`LoopHost::snap_not_now`]).
    Answer(bool),
    /// The loop point view opened or closed
    /// ([`LoopHost::set_point_open`]).
    PointView(bool),
    /// A footswitch's Snap or Use my tap ([`LoopHost::choose`]).
    Choose(Side),
}

/// The loop point view's windows: how many slices each draws.
pub const POINT_BARS: usize = 120;
/// Each window's length, ms: the start's from 100 ms before your press,
/// the end's from 200 ms before your tap ([`POINT_START_FROM_MS`],
/// [`POINT_END_FROM_MS`]), so what smart loop end looks at (a tenth of a
/// second each way) is in view.
pub const POINT_MS: f64 = 300.0;
pub const POINT_START_FROM_MS: f64 = -100.0;
pub const POINT_END_FROM_MS: f64 = -200.0;

/// The audio round a loop's start and end as smart loop end left it, for
/// the loop point view: each window's peaks, 0 to 255 (full scale), in
/// [`POINT_BARS`] slices. Plain data, so a view holds it without
/// allocating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopPoint {
    pub start: [u8; POINT_BARS],
    pub end: [u8; POINT_BARS],
}

/// Smart loop end on the picked loop, besides what its [`Status`] says
/// (the offer and where the loop is from your press and tap).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SmartView {
    /// The reading was a close call: Read as the other bars, while the
    /// loop is as it was read.
    pub close_call: Option<u32>,
    /// Keep my tap or Keep the snap: the chip is put away until the loop
    /// changes.
    pub kept: bool,
    /// The snap waiting on "... anyway?" ([`LoopHost::snap_asking`]),
    /// and whether it is an Undo or a Redo of Read as ([`ask_words`]).
    pub asking: Option<(Snap, bool)>,
    /// The picked loop's first pass waits on a Length it couldn't keep
    /// ([`Status::asks`]): how it read. The hint line asks
    /// ([`length_ask_words`]), Read as and Discard answer.
    pub length: Option<Reading>,
    /// A/B on the seam runs on the picked loop ([`LoopHost::set_ab`]):
    /// each time round its last beat runs into the downbeat as the other
    /// side has it, then as it is ([`mpfree_looper::Status::ab`] says
    /// which comes next).
    pub ab: bool,
    /// The loop point view's audio, while the view is open.
    pub point: Option<LoopPoint>,
}

/// A side of smart loop end's choice: the loop as you closed it (your
/// press and tap), or as it found (the snap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Tap,
    Snap,
}

/// What the line input counted since it was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineStats {
    pub periods: u64,
    pub frames: u64,
    /// Periods that had too little input (filled with silence), after the
    /// first few.
    pub short: u64,
    /// Frames dropped because the input ran ahead.
    pub dropped: u64,
    /// The input was not read in time and the stream restarted.
    pub overruns: u64,
    pub errors: u64,
}

pub struct LineIn {
    pcm: Option<alsa::Pcm>,
    raw: Vec<i32>,
    /// Interleaved stereo, a ring of FIFO frames.
    fifo: Vec<f32>,
    head: usize,
    len: usize,
    pub stats: LineStats,
    errors_in_row: u32,
}

impl Default for LineIn {
    fn default() -> Self {
        LineIn::closed()
    }
}

impl LineIn {
    pub fn closed() -> LineIn {
        LineIn {
            pcm: None,
            raw: vec![0; 2 * FIFO],
            fifo: vec![0.0; 2 * FIFO],
            head: 0,
            len: 0,
            stats: LineStats::default(),
            errors_in_row: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.pcm.is_some()
    }

    /// Open card `card`'s capture device at the stock settings and start
    /// it. On any refusal the device is closed again and nothing else
    /// changes; the error says what the card said.
    pub fn open(&mut self, card: u32) -> Result<String, String> {
        if self.is_open() {
            return Ok("line input: already on".to_string());
        }
        let path = format!("/dev/snd/pcmC{card}D0c");
        let pcm = alsa::Pcm::open_capture(&path)
            .map_err(|e| format!("line input: {path}: {e} (it stays off)"))?;
        let mut hw = alsa::HwParams::stock_capture();
        if let Err(e) = pcm.hw_params(&mut hw) {
            let mut any = alsa::HwParams::format_only();
            let allows = match pcm.refine(&mut any) {
                Ok(()) => format!(
                    "the card allows period {}, periods {}, buffer {}",
                    any.interval(alsa::PARAM_PERIOD_SIZE).describe(),
                    any.interval(alsa::PARAM_PERIODS).describe(),
                    any.interval(alsa::PARAM_BUFFER_SIZE).describe()
                ),
                Err(e2) => format!("asking what it allows failed too: {e2}"),
            };
            return Err(format!(
                "line input: {path}: the stock capture settings were refused: {e}; {allows} \
                                (closed; it stays off)"
            ));
        }
        let buffer = hw.interval(alsa::PARAM_BUFFER_SIZE).single().unwrap_or(0);
        if buffer != CAPTURE_BUFFER || !hw.mask_has(alsa::PARAM_FORMAT, alsa::FORMAT_S32_LE) {
            return Err(format!("line input: {path}: unexpected buffer {buffer} frames or format (closed; it stays off)"));
        }
        // Wake every period; never start by itself (start() below); stop
        // (an overrun) when the buffer is full.
        let mut sw = alsa::SwParams::new(PERIOD as u32, u32::MAX >> 1, buffer);
        pcm.sw_params(&mut sw)
            .map_err(|e| format!("line input: sw params: {e} (closed; it stays off)"))?;
        pcm.prepare()
            .map_err(|e| format!("line input: prepare: {e} (closed; it stays off)"))?;
        pcm.start()
            .map_err(|e| format!("line input: start: {e} (closed; it stays off)"))?;
        self.pcm = Some(pcm);
        self.head = 0;
        self.len = 0;
        self.stats = LineStats::default();
        self.errors_in_row = 0;
        Ok(format!("line input: on: {path}, rate {}, channels {}, period {} frames, buffer {buffer} frames",
                   hw.interval(alsa::PARAM_RATE).describe(), hw.interval(alsa::PARAM_CHANNELS).describe(),
                   hw.interval(alsa::PARAM_PERIOD_SIZE).describe()))
    }

    /// Stop and close the capture device.
    pub fn close(&mut self) -> String {
        let Some(pcm) = self.pcm.take() else {
            return "line input: already off".to_string();
        };
        let _ = pcm.drop_stream();
        drop(pcm);
        self.len = 0;
        format!("line input: off ({})", self.describe())
    }

    pub fn describe(&self) -> String {
        let s = &self.stats;
        format!(
            "{} periods, {} short, {} frames dropped, {} overruns, {} errors",
            s.periods, s.short, s.dropped, s.overruns, s.errors
        )
    }

    /// Interleaved S32 frames that arrived (the stream, or a test).
    pub fn push_s32(&mut self, interleaved: &[i32]) {
        let frames = interleaved.len() / 2;
        for f in 0..frames {
            if self.len == FIFO {
                self.head = (self.head + 1) % FIFO;
                self.len -= 1;
                self.stats.dropped += 1;
            }
            let at = (self.head + self.len) % FIFO;
            self.fifo[2 * at] = interleaved[2 * f] as f32 / 2_147_483_648.0;
            self.fifo[2 * at + 1] = interleaved[2 * f + 1] as f32 / 2_147_483_648.0;
            self.len += 1;
        }
        self.stats.frames += frames as u64;
    }

    /// Read what the stream has. A note comes back if the input closed
    /// itself after errors.
    fn pull(&mut self) -> Option<String> {
        for _ in 0..2 {
            let room = FIFO - self.len;
            let pcm = self.pcm.as_ref()?;
            if room == 0 {
                return None;
            }
            let read = pcm.read_frames(&mut self.raw[..2 * room]);
            match read {
                Ok(0) => return None,
                Ok(n) => {
                    self.errors_in_row = 0;
                    let n = (n as usize).min(room);
                    let raw = std::mem::take(&mut self.raw);
                    self.push_s32(&raw[..2 * n]);
                    self.raw = raw;
                }
                Err(e) if e.raw_os_error() == Some(alsa::EPIPE) => {
                    self.stats.overruns += 1;
                    if pcm.prepare().and_then(|_| pcm.start()).is_err() {
                        self.stats.errors += 1;
                        self.errors_in_row += 1;
                    }
                    return None;
                }
                Err(e) => {
                    self.stats.errors += 1;
                    self.errors_in_row += 1;
                    if self.errors_in_row > MAX_ERRORS {
                        let n = self.errors_in_row;
                        let closed = self.close();
                        return Some(format!("line input: read failed {n} times in a row ({e}); turned off: {closed}"));
                    }
                    return None;
                }
            }
        }
        None
    }

    /// One period of input into `left` and `right` (silence while off, and
    /// for whatever has not arrived). A note comes back if something went
    /// wrong that the log should say.
    pub fn period(
        &mut self,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Option<String> {
        if !self.is_open() {
            left.fill(0.0);
            right.fill(0.0);
            return None;
        }
        let note = self.pull();
        self.stats.periods += 1;
        let take = self.len.min(PERIOD);
        for f in 0..take {
            let at = (self.head + f) % FIFO;
            left[f] = self.fifo[2 * at];
            right[f] = self.fifo[2 * at + 1];
        }
        left[take..].fill(0.0);
        right[take..].fill(0.0);
        self.head = (self.head + take) % FIFO;
        self.len -= take;
        // The first periods after the start may come before any input.
        if take < PERIOD && self.stats.periods > 4 {
            self.stats.short += 1;
        }
        if self.len > 3 * PERIOD {
            let drop = self.len - 2 * PERIOD;
            self.head = (self.head + drop) % FIFO;
            self.len -= drop;
            self.stats.dropped += drop as u64;
        }
        note
    }
}

/// The loop's waveform for the Looper tab: min, max and RMS of the playing
/// layers, summed, in [`WAVE_COLUMNS`] columns per channel; and for the
/// Play page's strip, each layer's peak (either side) per column.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoopWave {
    pub frames: u64,
    /// While the first pass records: how many of its frames the columns
    /// hold so far, from the start (`frames` is then the length it is
    /// drawn against). None once the loop is whole.
    pub recorded: Option<u64>,
    pub columns: Vec<[WaveColumn; 2]>,
    /// `layers[k][col]`: layer k's peak in that column, 0 to 1.
    pub layers: Vec<Vec<f32>>,
}

impl LoopWave {
    /// While the first pass records: how far it has got along the wave,
    /// 0 to 1, for the playhead.
    pub fn live(&self) -> Option<f32> {
        let r = self.recorded?;
        (self.frames > 0).then(|| (r as f32 / self.frames as f32).min(1.0))
    }
}

impl Peaks for LoopWave {
    fn frames(&self) -> u64 {
        self.frames
    }

    fn channels(&self) -> usize {
        2
    }

    fn column(&self, ch: usize, f0: f64, f1: f64) -> WaveColumn {
        let n = self.columns.len();
        if n == 0 || ch > 1 || f1 <= 0.0 || f0 >= self.frames as f64 {
            return WaveColumn::default();
        }
        let per = self.frames as f64 / n as f64;
        let a = ((f0.max(0.0) / per).floor() as usize).min(n - 1);
        let b = ((f1 / per).ceil() as usize).clamp(a + 1, n);
        let mut c = WaveColumn {
            min: f32::MAX,
            max: f32::MIN,
            rms: 0.0,
        };
        let mut sq = 0.0;
        for col in &self.columns[a..b] {
            c.min = c.min.min(col[ch].min);
            c.max = c.max.max(col[ch].max);
            sq += col[ch].rms * col[ch].rms;
        }
        c.rms = (sq / (b - a) as f32).sqrt();
        c
    }
}

/// One column of one channel as it fills: min, max, sum of squares and
/// frames.
type Acc = (f32, f32, f32, u32);

const NO_ACC: Acc = (f32::MAX, f32::MIN, 0.0, 0);

fn acc_merge(a: Acc, b: Acc) -> Acc {
    (a.0.min(b.0), a.1.max(b.1), a.2 + b.2, a.3 + b.3)
}

fn column(a: Acc) -> WaveColumn {
    if a.3 == 0 {
        WaveColumn::default()
    } else {
        WaveColumn {
            min: a.0,
            max: a.1,
            rms: (a.2 / a.3 as f32).sqrt(),
        }
    }
}

/// The columns frames `from..from + n` of a wave of `len` frames fall in
/// (column `c` starts at frame `ceil(c * len / WAVE_COLUMNS)`), each with
/// its frames, counted from `from`.
fn runs(from: usize, n: usize, len: usize) -> impl Iterator<Item = (usize, Range<usize>)> {
    runs_in(from, n, len, WAVE_COLUMNS)
}

/// [`runs`] for a wave of `cols` columns.
fn runs_in(
    from: usize,
    n: usize,
    len: usize,
    cols: usize,
) -> impl Iterator<Item = (usize, Range<usize>)> {
    let (start, end, len) = (from as u64, (from + n) as u64, len.max(1) as u64);
    let cols = cols as u64;
    let mut f = start;
    std::iter::from_fn(move || {
        if f >= end {
            return None;
        }
        let col = (f * cols / len).min(cols - 1);
        let stop = if col + 1 < cols {
            ((col + 1) * len).div_ceil(cols).min(end)
        } else {
            end
        };
        let r = (f - start) as usize..(stop - start) as usize;
        f = stop;
        Some((col as usize, r))
    })
}

// The two below keep four lanes (two frames' left and right) apart, so
// that each step does not wait for the one before: about four times
// quicker than one running min, max and sum.

/// The peak (either side) of `x`, from `p`.
fn peak_of(p: f32, x: &[[f32; 2]]) -> f32 {
    let mut m = [0.0f32; 4];
    let pairs = x.chunks_exact(2);
    for v in pairs.remainder() {
        for (m, v) in m.iter_mut().zip(v) {
            *m = if v.abs() > *m { v.abs() } else { *m };
        }
    }
    for p in pairs {
        let v = [p[0][0], p[0][1], p[1][0], p[1][1]];
        for (m, v) in m.iter_mut().zip(v) {
            *m = if v.abs() > *m { v.abs() } else { *m };
        }
    }
    m.iter().fold(p, |a, &b| a.max(b))
}

/// `a` (left, right) with the frames `x` added.
fn acc_run(a: [Acc; 2], x: &[[f32; 2]]) -> [Acc; 2] {
    let mut lo = [f32::MAX; 4];
    let mut hi = [f32::MIN; 4];
    let mut sq = [0.0f32; 4];
    let pairs = x.chunks_exact(2);
    for v in pairs.remainder() {
        for i in 0..2 {
            lo[i] = if v[i] < lo[i] { v[i] } else { lo[i] };
            hi[i] = if v[i] > hi[i] { v[i] } else { hi[i] };
            sq[i] += v[i] * v[i];
        }
    }
    for p in pairs {
        let v = [p[0][0], p[0][1], p[1][0], p[1][1]];
        for i in 0..4 {
            lo[i] = if v[i] < lo[i] { v[i] } else { lo[i] };
            hi[i] = if v[i] > hi[i] { v[i] } else { hi[i] };
            sq[i] += v[i] * v[i];
        }
    }
    let ch = |c: usize| {
        let a = a[c];
        (
            a.0.min(lo[c]).min(lo[c + 2]),
            a.1.max(hi[c]).max(hi[c + 2]),
            a.2 + sq[c] + sq[c + 2],
            a.3 + x.len() as u32,
        )
    };
    [ch(0), ch(1)]
}

/// The waves sent to the screen, each made again in place once the screen
/// has let go of it, so that the audio thread never allocates one.
struct WavePool {
    waves: Vec<Arc<LoopWave>>,
    /// Layer rows no wave is using.
    spare: Vec<Vec<f32>>,
    /// Times the screen still held every wave, so the next one waited a
    /// period (should stay 0: the screen keeps one, and one or two may be
    /// on their way).
    waits: u64,
}

impl WavePool {
    fn new() -> WavePool {
        let wave = || {
            Arc::new(LoopWave {
                frames: 0,
                recorded: None,
                columns: vec![[WaveColumn::default(); 2]; WAVE_COLUMNS],
                layers: Vec::with_capacity(MAX_LAYERS),
            })
        };
        WavePool {
            waves: (0..WAVES).map(|_| wave()).collect(),
            spare: (0..WAVES * MAX_LAYERS)
                .map(|_| vec![0.0; WAVE_COLUMNS])
                .collect(),
            waits: 0,
        }
    }

    /// A wave with `layers` layer rows, as `fill` makes it; None (counted)
    /// while the screen holds every wave.
    fn make(&mut self, layers: usize, fill: impl FnOnce(&mut LoopWave)) -> Option<Arc<LoopWave>> {
        let Some(i) = self
            .waves
            .iter_mut()
            .position(|w| Arc::get_mut(w).is_some())
        else {
            self.waits += 1;
            return None;
        };
        let w = Arc::get_mut(&mut self.waves[i]).expect("only the pool holds it");
        let layers = layers.min(MAX_LAYERS);
        while w.layers.len() > layers {
            self.spare.extend(w.layers.pop());
        }
        while w.layers.len() < layers {
            match self.spare.pop() {
                Some(row) => w.layers.push(row),
                None => break,
            }
        }
        fill(w);
        Some(Arc::clone(&self.waves[i]))
    }
}

/// Builds the loop's [`LoopWave`] a slice at a time: [`WAVE_BUDGET`]
/// layer-frames a period, each layer read a block at a time.
struct WaveBuilder {
    /// What the wave is of: the loop's length, layers and phase, and which
    /// loop.
    of: (usize, usize, Phase, usize),
    at: usize,
    frames: usize,
    layers: usize,
    acc: Vec<[Acc; 2]>,
    /// Each layer's peak per column.
    peaks: Vec<Vec<f32>>,
    /// One slice of one layer, and of the layers summed.
    buf: Vec<[f32; 2]>,
    sum: Vec<[f32; 2]>,
    building: bool,
}

impl WaveBuilder {
    fn new() -> WaveBuilder {
        WaveBuilder {
            of: (0, 0, Phase::Empty, 0),
            at: 0,
            frames: 0,
            layers: 0,
            acc: vec![[NO_ACC; 2]; WAVE_COLUMNS],
            peaks: vec![vec![0.0; WAVE_COLUMNS]; MAX_LAYERS],
            buf: vec![[0.0; 2]; WAVE_SLICE],
            sum: vec![[0.0; 2]; WAVE_SLICE],
            building: false,
        }
    }

    fn start(&mut self, frames: usize, layers: usize, of: (usize, usize, Phase, usize)) {
        self.of = of;
        self.frames = frames;
        self.layers = layers.min(MAX_LAYERS);
        self.at = 0;
        self.building = frames > 0;
        self.acc.fill([NO_ACC; 2]);
        for l in self.peaks.iter_mut() {
            l.fill(0.0);
        }
    }

    /// One slice: true once every slice is done and the wave is ready
    /// ([`WaveBuilder::finish`]).
    fn step(&mut self, looper: &Looper) -> bool {
        if !self.building {
            return false;
        }
        let len = self.frames;
        if self.at < len {
            let from = self.at;
            let n = (WAVE_BUDGET / self.layers.max(1))
                .clamp(WAVE_MIN, WAVE_SLICE)
                .min(len - from);
            let sum = &mut self.sum[..n];
            sum.fill([0.0; 2]);
            for k in 0..self.layers {
                let Some(l) = looper.layer(k) else { continue };
                if l.len() < 2 * (from + n) {
                    continue;
                }
                let buf = &mut self.buf[..n];
                l.read(from, buf);
                let peaks = &mut self.peaks[k];
                for (c, r) in runs(from, n, len) {
                    peaks[c] = peak_of(peaks[c], &buf[r]);
                }
                for (s, x) in sum.iter_mut().zip(buf.iter()) {
                    s[0] += x[0];
                    s[1] += x[1];
                }
            }
            for (c, r) in runs(from, n, len) {
                self.acc[c] = acc_run(self.acc[c], &sum[r]);
            }
            self.at += n;
        }
        self.at >= len
    }

    /// The finished wave, in one of `pool`'s; None while the screen holds
    /// them all (it is ready again next period).
    fn finish(&mut self, pool: &mut WavePool) -> Option<Arc<LoopWave>> {
        let w = pool.make(self.layers, |w| {
            w.frames = self.frames as u64;
            w.recorded = None;
            for (to, a) in w.columns.iter_mut().zip(&self.acc) {
                *to = [column(a[0]), column(a[1])];
            }
            for (to, p) in w.layers.iter_mut().zip(&self.peaks) {
                for (t, v) in to.iter_mut().zip(p) {
                    *t = v.min(1.0);
                }
            }
        })?;
        self.building = false;
        Some(w)
    }
}

/// A loop's small wave for its cells on the Clips grid: each column's
/// peak over every layer, 0 to 255.
pub type CellWave = [u8; CELL_WAVE];

/// Columns in a loop's wave round its ring on the Looper tab's deck: three
/// to each of a cell's, so a cell's column is the loudest of its three.
pub const RING_WAVE: usize = 3 * CELL_WAVE;

/// A loop's wave round its ring on the deck: each column's peak over
/// every layer, 0 to 255.
pub type RingWave = [u8; RING_WAVE];

/// A cell's wave from a ring's: the loudest of each three columns.
pub fn cell_of_ring(r: &RingWave) -> CellWave {
    let mut c = [0u8; CELL_WAVE];
    for (i, v) in c.iter_mut().enumerate() {
        *v = r[3 * i].max(r[3 * i + 1]).max(r[3 * i + 2]);
    }
    c
}

/// Makes each loop's [`RingWave`] in turn (and from it its [`CellWave`]),
/// a slice a period ([`WAVE_BUDGET`] layer-frames, as the loop's own
/// wave), whenever a loop's frames, layers or sound (`Status::version`)
/// changed. A loop recording or overdubbing keeps the one it had until it
/// plays.
struct CellWaves {
    /// Each loop's waves, and what they are of: frames, layers and version.
    done: [CellWave; MAX_LOOPS],
    rings: [RingWave; MAX_LOOPS],
    of: [(usize, usize, u64); MAX_LOOPS],
    /// The loop being made and what of, how far it is, and its peaks.
    making: Option<(usize, (usize, usize, u64))>,
    at: usize,
    peaks: [f32; RING_WAVE],
    buf: Vec<[f32; 2]>,
}

impl CellWaves {
    fn new() -> CellWaves {
        CellWaves {
            done: [[0; CELL_WAVE]; MAX_LOOPS],
            rings: [[0; RING_WAVE]; MAX_LOOPS],
            of: [(0, 0, 0); MAX_LOOPS],
            making: None,
            at: 0,
            peaks: [0.0; RING_WAVE],
            buf: vec![[0.0; 2]; WAVE_SLICE],
        }
    }

    /// One slice of the wave being made, or every [`CELL_WAVE_EVERY`]
    /// periods a look for a loop whose wave is out of date.
    fn step(&mut self, looper: &Looper, period: u64) {
        if self.making.is_none() {
            if !period.is_multiple_of(CELL_WAVE_EVERY) {
                return;
            }
            for n in 0..MAX_LOOPS {
                let Some(s) = looper.loop_status(n) else {
                    continue;
                };
                let of = (s.frames, s.layers, s.version);
                let busy = matches!(
                    s.phase,
                    Phase::Armed | Phase::Recording | Phase::Overdubbing
                );
                if busy || of == self.of[n] {
                    continue;
                }
                if s.frames == 0 || s.phase == Phase::Empty {
                    self.done[n] = [0; CELL_WAVE];
                    self.rings[n] = [0; RING_WAVE];
                    self.of[n] = of;
                    continue;
                }
                self.making = Some((n, of));
                self.at = 0;
                self.peaks = [0.0; RING_WAVE];
                break;
            }
        }
        let Some((n, of)) = self.making else {
            return;
        };
        // The loop changed under it: it starts again on the next look.
        let now = looper
            .loop_status(n)
            .map(|s| (s.frames, s.layers, s.version));
        if now != Some(of) {
            self.making = None;
            return;
        }
        let (len, layers, _) = of;
        let from = self.at;
        let k = (WAVE_BUDGET / layers.max(1))
            .clamp(WAVE_MIN, WAVE_SLICE)
            .min(len - from);
        for layer in 0..layers.min(MAX_LAYERS) {
            let Some(l) = looper.loop_layer(n, layer) else {
                continue;
            };
            if l.len() < 2 * (from + k) {
                continue;
            }
            let buf = &mut self.buf[..k];
            l.read(from, buf);
            for (c, r) in runs_in(from, k, len, RING_WAVE) {
                self.peaks[c] = peak_of(self.peaks[c], &buf[r]);
            }
        }
        self.at += k;
        if self.at >= len {
            for (d, p) in self.rings[n].iter_mut().zip(&self.peaks) {
                *d = (p.min(1.0) * 255.0).round() as u8;
            }
            self.done[n] = cell_of_ring(&self.rings[n]);
            self.of[n] = of;
            self.making = None;
        }
    }
}

/// The first pass's waveform as it records, the new frames each period.
struct LiveWave {
    /// The loop whose first pass it is (None: not making one).
    of: Option<usize>,
    /// The frames the columns span: the pass's length when its bars are
    /// set, or two bars; twice that each time the pass gets to the end.
    scale: usize,
    /// Frames of the pass in the columns, and as many as when last sent.
    done: usize,
    sent: usize,
    acc: Vec<[Acc; 2]>,
    peaks: Vec<f32>,
    buf: Vec<[f32; 2]>,
}

impl LiveWave {
    fn new() -> LiveWave {
        LiveWave {
            of: None,
            scale: 0,
            done: 0,
            sent: 0,
            acc: vec![[NO_ACC; 2]; WAVE_COLUMNS],
            peaks: vec![0.0; WAVE_COLUMNS],
            buf: vec![[0.0; 2]; WAVE_SLICE],
        }
    }

    fn start(&mut self, n: usize, scale: usize) {
        self.of = Some(n);
        self.scale = scale.max(WAVE_COLUMNS);
        self.done = 0;
        self.sent = 0;
        self.acc.fill([NO_ACC; 2]);
        self.peaks.fill(0.0);
    }

    /// Twice the frames in the same columns: each pair becomes one.
    fn widen(&mut self) {
        let half = WAVE_COLUMNS / 2;
        for c in 0..half {
            let (a, b) = (self.acc[2 * c], self.acc[2 * c + 1]);
            self.acc[c] = [acc_merge(a[0], b[0]), acc_merge(a[1], b[1])];
            self.peaks[c] = self.peaks[2 * c].max(self.peaks[2 * c + 1]);
        }
        self.acc[half..].fill([NO_ACC; 2]);
        self.peaks[half..].fill(0.0);
        self.scale *= 2;
    }

    /// The pass's frames since the last step (a slice at most; the rest
    /// next period), `recorded` frames of it being written.
    fn step(&mut self, pass: &mpfree_looper::LayerView, recorded: usize) {
        let n = recorded.saturating_sub(self.done).min(WAVE_SLICE);
        if n == 0 || pass.len() < 2 * (self.done + n) {
            return;
        }
        while self.done + n > self.scale {
            self.widen();
        }
        let buf = &mut self.buf[..n];
        pass.read(self.done, buf);
        for (c, r) in runs(self.done, n, self.scale) {
            let x = &buf[r];
            self.peaks[c] = peak_of(self.peaks[c], x);
            self.acc[c] = acc_run(self.acc[c], x);
        }
        self.done += n;
    }

    /// The wave so far, in one of `pool`'s.
    fn make(&mut self, pool: &mut WavePool) -> Option<Arc<LoopWave>> {
        let w = pool.make(1, |w| {
            w.frames = self.scale as u64;
            w.recorded = Some(self.done as u64);
            for (to, a) in w.columns.iter_mut().zip(&self.acc) {
                *to = [column(a[0]), column(a[1])];
            }
            if let Some(row) = w.layers.first_mut() {
                for (t, v) in row.iter_mut().zip(&self.peaks) {
                    *t = v.min(1.0);
                }
            }
        })?;
        self.sent = self.done;
        Some(w)
    }
}

/// What the Looper tab shows (sent with the timing, and on changes).
#[derive(Clone, Debug, PartialEq)]
pub struct LooperView {
    /// Smart loop end on the picked loop, for its chip and the loop point
    /// view.
    pub smart: SmartView,
    pub status: Status,
    pub stats: Stats,
    /// Records from as picked: what the next take records.
    pub source: LoopSource,
    /// What the take recording now records from, while it holds another
    /// source than the one picked ([`LoopHost::settle`]).
    pub taking_from: Option<LoopSource>,
    /// Records from the other loops: Record on an empty loop resamples
    /// ([`LoopHost::resample`]).
    pub resample: bool,
    pub line_on: bool,
    pub line: LineStats,
    pub monitor: bool,
    /// The recorded source's peaks since the last view, dBFS (-inf when
    /// silent).
    pub meter_db: [f32; 2],
    /// The loop's place in the song: beats per bar, the frames of one of
    /// those beats (a quarter in 4/4, an eighth in 6/8) at the tempo it
    /// was recorded at, for the bar lines, and the bar's length in ticks
    /// ([`Meter::bar_ticks`](mpfree_transport::time::Meter::bar_ticks)),
    /// for counting a loop's bars.
    pub beats_per_bar: u32,
    pub frames_per_beat: f64,
    pub bar_ticks: Tick,
    /// The loop's layers in play order (the first pass first) with their
    /// mix, for the speakers; None past the last.
    pub layers: [Option<LayerInfo>; MAX_LAYERS],
    /// Every loop, for the Looper tab's list; `status` is `loops[selected]`.
    pub loops: [Status; MAX_LOOPS],
    pub selected: usize,
    /// The memory all the loops share (frames).
    pub memory: Memory,
    /// Frames per second, for the lengths in seconds.
    pub rate: u32,
    /// The picked loop's undo history.
    pub history: History,
    /// Each loop's small wave, for its cells on the Clips grid.
    pub cell_waves: [CellWave; MAX_LOOPS],
    /// Each loop's wave round its ring on the Looper tab's deck.
    pub ring_waves: [RingWave; MAX_LOOPS],
    /// The loop a whole loop copies into ([`LoopHost::copy_loop`]), until
    /// every layer is in.
    pub copy_into: Option<usize>,
    /// What the looper heard over the last bars (What you played).
    pub heard: HeardWave,
    /// Where Capture goes now: the picked loop if it is empty, else the
    /// first empty one (None: none is), and the bars it asks for.
    pub capture_into: Option<(usize, u32)>,
    /// Tempo from the first loop is on, and the song can take one now
    /// ([`LoopHost::set_tempo_from_loop`]).
    pub free_tempo: bool,
    /// Length: Pattern is picked ([`PATTERN_BARS`]): a new loop takes the
    /// length of the pattern playing.
    pub match_pattern: bool,
    /// Listen plays: the loop, its layer (None: the whole loop) and
    /// where it has got to, 0 to 1 ([`LoopHost::listen`]).
    pub listening: Option<(usize, Option<u8>, f32)>,
    /// The pattern's length in bars as last told
    /// ([`LoopHost::set_pattern_bars`]); None: no pattern plays.
    pub pattern_bars: Option<u32>,
    /// Sync as last set ([`LoopHost::clock`]): the clock every empty loop
    /// records on.
    pub clock: Clock,
    /// The free clock while a free loop holds audio
    /// ([`Looper::free_clock`]), where it is now ([`Looper::free_now`]),
    /// and whether a Play for the stopped free loops waits for the song's
    /// bar line ([`Looper::free_cued`]).
    pub free: Option<FreeClock>,
    pub free_now: f64,
    pub free_cued: bool,
    /// Loops on audio tracks: each slot's loop's lasting id (0: none
    /// yet), so the grid's loop cells find their loops
    /// ([`LoopHost::loop_ids`]).
    pub ids: [u128; MAX_LOOPS],
    /// Each loop holds a layer not yet on the stick (audio tracks phase
    /// 4c.3, with recordings on: [`LoopHost::unsaved_layers`]): its ring's
    /// dot.
    pub unsaved: [bool; MAX_LOOPS],
}

/// A free clock as a transport span: playing, straight (no swing), on its
/// anchor, tempo and meter (the looper copies a song span into a free
/// clock the other way, `FreeClock::of_song`), so whatever follows a span
/// keeps the free loops' time. Its offset, length and frame are each
/// period's to set.
pub fn lead_span(c: &FreeClock) -> Span {
    Span {
        offset: 0,
        len: 0,
        frame: c.anchor_frame,
        state: State::Playing,
        overdub: false,
        anchor_frame: c.anchor_frame,
        anchor_tick: c.anchor_tick,
        bpm: c.bpm,
        rate: c.rate,
        swing: Swing::default(),
        meter: c.meter,
    }
}

impl LooperView {
    /// The free loops play: the free clock is set and runs.
    pub fn free_runs(&self) -> bool {
        self.free.is_some_and(|c| c.running)
    }

    /// Loop `n` is a free loop with sound, stopped with the free clock: it
    /// waits at its place on bar 1, and PLAY, Play or its ring starts it.
    /// Not while muted: its ring unmutes it first, as any loop's.
    pub fn free_stopped(&self, n: usize) -> bool {
        self.loops.get(n).is_some_and(|l| {
            l.clock == Clock::Free
                && l.phase == Phase::Playing
                && !l.muted
                && self.free.is_some_and(|c| !c.running)
        })
    }

    /// A free loop's bar in ticks (the free clock's meter), else the
    /// song's.
    pub fn bar_ticks_of(&self, s: &Status) -> Tick {
        match (s.clock, self.free) {
            (Clock::Free, Some(c)) => c.meter.bar_ticks(),
            _ => self.bar_ticks,
        }
    }
}

impl Default for LooperView {
    fn default() -> Self {
        let l = Looper::new(44_100, 0);
        LooperView {
            smart: SmartView::default(),
            status: l.status(),
            stats: l.stats(),
            source: LoopSource::Pads,
            taking_from: None,
            resample: false,
            line_on: false,
            line: LineStats::default(),
            monitor: false,
            meter_db: [f32::NEG_INFINITY; 2],
            beats_per_bar: 4,
            frames_per_beat: 0.0,
            bar_ticks: 4 * PPQ as Tick,
            layers: [None; MAX_LAYERS],
            loops: [l.status(); MAX_LOOPS],
            selected: 0,
            memory: l.memory(),
            rate: 44_100,
            history: l.history(),
            cell_waves: [[0; CELL_WAVE]; MAX_LOOPS],
            ring_waves: [[0; RING_WAVE]; MAX_LOOPS],
            copy_into: None,
            heard: HeardWave::default(),
            capture_into: None,
            free_tempo: false,
            match_pattern: false,
            pattern_bars: None,
            ids: [0; MAX_LOOPS],
            listening: None,
            unsaved: [false; MAX_LOOPS],
            clock: Clock::Song,
            free: None,
            free_now: 0.0,
            free_cued: false,
        }
    }
}

pub struct LoopHost {
    pub looper: Looper,
    pub line: LineIn,
    pub monitor: bool,
    /// What this take records from (between takes, Records from as
    /// picked): a take holds the one it started with
    /// ([`LoopHost::settle`]).
    source: LoopSource,
    /// Records from as last picked: what the next take records.
    chosen: LoopSource,
    /// The Looper tab's Records from: Loops. Record on an empty loop
    /// resamples into it: its first pass records the other loops as heard
    /// ([`Command::Resample`]).
    pub resample: bool,
    input_l: [f32; PERIOD],
    input_r: [f32; PERIOD],
    /// The pad player's sound for this period (see [`LoopHost::take_pads`]).
    pads_l: [f32; PERIOD],
    pads_r: [f32; PERIOD],
    /// The latency the pads' sound comes with: 0 dry, a period and more
    /// from the mixer ([`LoopHost::take_pads_as`]).
    pads_latency: u32,
    /// Why the pads are recorded dry, when they are.
    pads_dry: Option<&'static str>,
    /// The instrument Records from: Pads takes, held with
    /// `source` through a take (None for another source, or before the
    /// app first said what the pads play).
    pads: Option<Instrument>,
    /// What the pads play now ([`LoopHost::follow_pads`]).
    pads_now: Option<Instrument>,
    /// Where it sounds in the mixer ([`LoopHost::set_pads_strip`]), for an
    /// audio track's input list's Pads (phase 4, board 4).
    pads_strip: Option<mpfree_fx::StripId>,
    /// The source a take held back, as the log was told (once).
    held_said: Option<(LoopSource, Option<Instrument>)>,
    /// Takes that recorded silence for a while: their source was lost
    /// while they recorded ([`LoopHost::take_pads_as`]), counted, logged
    /// and said on the screen.
    pub dropouts: u64,
    /// The latency the looper was last given.
    latency_set: u32,
    /// Whether the looper was last told to listen by ear (the line input)
    /// for a loop that starts when you play ([`Command::StartsByEar`]).
    by_ear_set: bool,
    /// The frame a pad hit was last told on ([`LoopHost::played`]): one a
    /// period is enough.
    played_at: Option<u64>,
    /// Starts on a note refused ([`LoopHost::start_refused`]): counted,
    /// logged and said on the screen.
    pub starts_refused: u64,
    /// Tempo from the first loop, as the looper was last told
    /// ([`LoopHost::set_tempo_from_loop`]).
    free_tempo: bool,
    /// The song's tempo was set by hand, as the looper was last told
    /// ([`LoopHost::set_tempo_meant`]).
    tempo_meant: bool,
    /// Smart loop end, as the looper was last told
    /// ([`LoopHost::set_smart_end`]).
    smart: SmartEnd,
    /// Periods whose notes didn't all reach smart loop end
    /// ([`LoopHost::notes_lost`]): counted, logged and said.
    pub notes_lost: u64,
    /// A snap asked for (Snap, Use my tap, or UNDO or REDO of a snap),
    /// for a loop, until the next period works it out
    /// ([`LoopHost::tend_snap`]); then the plan, waiting for the time
    /// engine's answer to its tempo. True: asked and answered with
    /// Snap anyway ([`LoopHost::snap_anyway`]).
    snap_want: Option<(usize, Snap, bool)>,
    snap_wait: Option<(SnapPlan, u32)>,
    /// A free loop's snap, worked out: it changes the free clock's tempo,
    /// not the song's, so it asks nothing of the time engine, and goes to
    /// the looper in the period its frame falls in, as the time engine
    /// commits a song loop's (until then the loop is as it was).
    free_snap: Option<SnapPlan>,
    /// A snap that would change the tempo under recorded audio (the audio
    /// tracks' recordings), waiting for Snap anyway or Not now: the hint
    /// line asks ([`LoopHost::snap_asking`]).
    snap_ask: Option<(usize, Snap)>,
    /// The loops whose wait on their Length was told
    /// ([`LoopHost::take_asks`]), a bit each.
    asks_told: u32,
    /// A side picked for a loop (the loop point view's Snap, Keep my tap,
    /// Use my tap, Keep the snap): the snap that gets
    /// there is worked out when no other is under way
    /// ([`LoopHost::side`]); true: the chip is put away once there.
    side_want: Option<(usize, Side, bool)>,
    /// A/B on the seam on loop `n` ([`LoopHost::set_ab`]), started at
    /// the loop's version `v`: the looper ends it when the loop changes,
    /// so it shows only while the loop is still at it.
    ab: Option<(usize, u64)>,
    /// Keep my tap or Keep the snap: the chip is put away while loop `n`
    /// is at this version.
    kept: Option<(usize, u64)>,
    /// The last reading that was a close call: its loop, the loop's
    /// version, and the other reading's bars, for Read as.
    close_call: Option<(usize, u64, u32)>,
    /// The loop point view is open: the picked loop's audio round its
    /// start and end is read for it ([`LoopHost::set_point_open`]).
    point_open: bool,
    /// That audio, for the loop, its version and where smart loop end
    /// left it, until one changes.
    point: Option<(usize, u64, Shift, LoopPoint)>,
    /// Where it is read into: a window at the fastest rate, allocated
    /// once.
    point_buf: Vec<[f32; 2]>,
    /// Snaps that couldn't be made or didn't land: counted, logged and
    /// said.
    pub snaps_refused: u64,
    /// Free first passes whose Length bars would have been slower than
    /// [`LOOP_MIN_BPM`], so the length was read instead
    /// ([`Reading::overruled`]): counted, logged and said.
    pub readings_overruled: u64,
    /// Tempos from a free first pass the song couldn't start at
    /// ([`LoopHost::tempo_refused`]): counted, logged and said.
    pub tempos_refused: u64,
    /// Length: Pattern ([`PATTERN_BARS`]): an empty loop's first pass
    /// takes the length of the pattern playing.
    pub match_pattern: bool,
    /// Starts, for every empty loop: the last one picked (On the bar at
    /// first). A loop picked, or recorded, while empty takes it, so the
    /// choice sticks from loop to loop.
    starts: Starts,
    /// Sync, for every empty loop, sticking as Starts does: the song's
    /// clock at first, or the free loops' ([`Clock::Free`]).
    clock: Clock,
    /// The loop picked was picked for a Loop cell
    /// ([`LoopHost::select_for_cell`]): it records on the song's clock,
    /// until another is picked or Sync is set.
    for_cell: bool,
    /// What you played's window, moved or resized: the bar line its bars
    /// end on (None: the newest, keeping up as the song plays) and how
    /// many ([`LoopHost::set_capture_span`]). None: the Length's bars.
    capture_span: Option<(Option<Tick>, u32)>,
    /// The pattern's length in bars ([`LoopHost::set_pattern_bars`]).
    pattern_bars: Option<u32>,
    /// The song was rolling (not stopped) in the last period.
    last_rolling: bool,
    input: Vec<f32>,
    out: Vec<f32>,
    last: Status,
    /// Periods where the loop or the input had to be kept within ±1
    /// (without the mixer: [`LoopHost::period`]).
    pub clipped: u64,
    wave: WaveBuilder,
    /// The first pass's wave while it records.
    live: LiveWave,
    waves: WavePool,
    /// The screen shows a wave (not the empty one).
    wave_shown: bool,
    /// Each loop's small wave for its cells on the Clips grid.
    cells: CellWaves,
    /// The recorded source's peaks since the last view.
    meter: [f32; 2],
    /// The song's meter and tempo as the last period played.
    beats_per_bar: u32,
    bpm: f64,
    rate: u32,
    printer: Printer,
    /// Smart loop end's finder, for the line input and mixes.
    pub finder: Finder,
    /// Smart loop end listens for hits ([`Command::SmartByEar`]), as the
    /// looper was last told.
    by_ear_smart: bool,
    /// A save of the loops under way, and one done.
    saving: Option<Saving>,
    saved: Option<Saved>,
    /// Saves started again because a loop changed while it was read.
    pub save_restarts: u64,
    /// An open or New project ([`LoopHost::clear_all`]): every loop is to
    /// be cleared once the command queue has room for it.
    clear_next: bool,
    /// The loops are fading out for an open; the project's loops go in
    /// once every one is empty.
    emptying: bool,
    /// An opened project's loops, waiting for the loops to empty.
    incoming: Option<Saved>,
    /// Periods to run before the loops can be seen empty: the clears
    /// queued apply with the next period.
    settle: u8,
    /// The loop the last pick asked for: commands queued after it act on
    /// it ([`LoopHost::toggle_mute`] picks it back).
    picked: usize,
    /// Undo (below 0) or Redo (above 0) presses the undo history asked for
    /// that the command queue had no room for yet ([`LoopHost::steps`]).
    steps_left: i32,
    /// A whole loop copying into an empty one ([`LoopHost::copy_loop`]),
    /// and how the last one ended, until [`LoopHost::copy_ended`] takes it.
    copy: Option<LoopCopy>,
    copy_end: Option<CopyEnd>,
    /// The peak of every [`HEARD_CHUNK`] frames the looper heard, frame
    /// `f`'s at `(f / HEARD_CHUNK) % len` (What you played).
    heard_peaks: Vec<f32>,
    /// A bar's ticks as the last period played.
    bar_ticks: Tick,
    /// Loops on audio tracks (phase 4b, behind Dev's switch): each slot's
    /// loop's lasting id, and which are on tracks.
    pub on_tracks: crate::loops_on_tracks::OnTracks,
    /// Layers as recordings (phase 4c.2, behind the same switch): saves
    /// keep each loop as the looper holds it (`mpfree-looper 2`), each
    /// layer named by its recording ([`LoopHost::set_recordings`]).
    recordings: bool,
    /// Each closed layer's recording ([`LoopHost::give_layer_recs`]).
    layer_recs: crate::loops_on_tracks::LayerRecs,
    /// Closed layers written to their recordings' files soon after they
    /// close (phase 4c.3, [`LoopHost::write_layers`]).
    layer_writer: crate::loops_on_tracks::LayerWriter,
}

/// A whole loop copying into an empty one, a layer at a time
/// ([`LoopHost::copy_loop`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LoopCopy {
    from: usize,
    to: usize,
    /// The source's layers by id, in play order, and how many.
    ids: [u8; MAX_LAYERS],
    layers: usize,
    /// Layers asked for so far.
    asked: usize,
    /// Periods since the last layer was asked for with nothing to show
    /// for it (refused, or not started yet), and tries in all.
    waited: u32,
    tries: u32,
}

/// How a whole loop's copy ended ([`LoopHost::copy_ended`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyEnd {
    pub from: usize,
    pub to: usize,
    /// Layers that went in, of how many.
    pub layers: usize,
    pub of: usize,
}

/// A save under way ([`LoopHost::save`]).
enum Saving {
    /// Until no loop is recording a first pass, overdubbing, merging,
    /// fitting or loading.
    Waiting,
    /// The worker is making room for the layers' audio.
    Sizing {
        /// Each loop's (version, layers, frames) when the save began.
        at: [(u64, usize, usize); MAX_LOOPS],
        /// A snapshot (phase 4c.4, recordings on): each layer's number,
        /// by loop and play order, read by it whatever the loop does.
        snap: Option<Borns>,
    },
    /// Reading layer `k` of `saved.loops[i]` from `frame`.
    Reading {
        saved: Saved,
        at: [(u64, usize, usize); MAX_LOOPS],
        snap: Option<Borns>,
        i: usize,
        k: usize,
        frame: usize,
    },
}

/// Each loop's layers' numbers ([`LayerInfo::born`]), by loop and play
/// order, as a snapshot save took them (phase 4c.4).
type Borns = [[u64; MAX_LAYERS]; MAX_LOOPS];

/// Frames of the loops' layers a save reads for each frame that plays.
pub const SAVE_SPEED: usize = 64;

/// Frames a save reads of layer `k` of `f`: the loop's length as it
/// plays, or as the looper holds it, its audio's size (phase 4c.2).
fn layer_frames(f: &LoopFile, k: usize) -> usize {
    match copied(f, k) {
        true => 0,
        false => f.layers[k].raw.as_ref().map_or(f.frames, |r| r.size),
    }
}

/// Layer `k` of `f` is in its recording's synced file, which a snapshot
/// save named (phase 4c.4): the project copies the file, and nothing of
/// it is read or held for the save.
pub fn copied(f: &LoopFile, k: usize) -> bool {
    let y = &f.layers[k];
    !y.file.is_empty() && y.raw.as_ref().is_some_and(|r| r.rec != 0)
}

/// What the worker is asked: a buffer for a bounce, a print, room for a
/// save's audio, or to drop what the audio loop is done with.
enum ToPrinter {
    Buffer(usize),
    Print(Bounce, usize),
    Drop(Option<Print>, Vec<f32>),
    Size(Saved),
    DropSaved(Saved),
    DropLoop(LoopFile),
}

enum FromPrinter {
    Buffer(Vec<f32>),
    /// The print for (loop, frames), or None if it could not be made.
    Done(usize, usize, Option<Print>),
    /// A save with room for every layer's audio.
    Sized(Saved),
}

/// One print under way: for loop `n`, `frames` long, from the loop at
/// `version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wanted {
    n: usize,
    frames: usize,
    version: u64,
}

/// Smart loop end's finder for the line input and mixes (the smart loop
/// end plan, step 3; [`mpfree_looper::find`]): a thread of its own, not
/// the print worker, which runs one long job at a time at idle priority,
/// so a print can never make Auto miss its moment. One job, made here
/// once, goes back and forth through two one-place channels: the looper
/// fills it when it wants a decision ([`Looper::find_job`]), the thread
/// answers it and sends it back, and the next period hands the answer to
/// the looper ([`Looper::found`]), which drops it if its take has gone.
/// The thread sleeps until asked, at the normal priority, off the sound's
/// cores. Nothing here waits or allocates on the audio thread: an ask
/// made while the job is still out waits for the next period, and the
/// looper keeps the tap when the answer comes too late (counted), as it
/// does if the thread is gone.
pub struct Finder {
    to: Option<SyncSender<Box<FindJob>>>,
    back: Option<Receiver<Box<FindJob>>>,
    /// The job, when it isn't with the thread.
    job: Option<Box<FindJob>>,
    /// Answer on the calling thread instead (tests that step periods far
    /// faster than they would play).
    pub inline: bool,
    /// Jobs answered, and periods an ask waited for the job to come back.
    pub answered: u64,
    pub waited: u64,
    /// The thread couldn't start or has stopped: the looper keeps every
    /// tap its finder would have weighed.
    pub gone: bool,
}

impl Finder {
    fn new(rate: u32) -> Finder {
        let job = Box::new(FindJob::new(mpfree_looper::ends::reach(rate, f64::NAN)));
        let (to, jobs) = sync_channel::<Box<FindJob>>(1);
        let (done, back) = sync_channel::<Box<FindJob>>(1);
        let spawned = std::thread::Builder::new()
            .name("mpfree-find".into())
            .spawn(move || {
                // Normal priority, off the sound's cores.
                let _ = crate::cores::background();
                for mut j in jobs {
                    find(&mut j);
                    if done.send(j).is_err() {
                        break;
                    }
                }
            })
            .is_ok();
        Finder {
            to: spawned.then_some(to),
            back: spawned.then_some(back),
            job: Some(job),
            inline: false,
            answered: 0,
            waited: 0,
            gone: !spawned,
        }
    }

    /// The thread is gone: nothing more goes to it.
    fn lose(&mut self) {
        self.to = None;
        self.back = None;
        self.gone = true;
    }

    /// After the looper's period: an answer back goes to the looper, and
    /// a new ask goes to the thread (or is answered here when `inline`).
    fn tend(&mut self, looper: &mut Looper) {
        if self.job.is_none() {
            match self.back.as_ref().map(Receiver::try_recv) {
                Some(Ok(j)) => {
                    looper.found(&j);
                    self.answered += 1;
                    self.job = Some(j);
                }
                Some(Err(TryRecvError::Disconnected)) => self.lose(),
                _ => {}
            }
        }
        if !self.inline && self.to.is_none() {
            return;
        }
        let Some(mut j) = self.job.take() else {
            if looper.find_wanted() {
                self.waited += 1;
            }
            return;
        };
        if !looper.find_job(&mut j) {
            self.job = Some(j);
            return;
        }
        if self.inline {
            find(&mut j);
            looper.found(&j);
            self.answered += 1;
            self.job = Some(j);
            return;
        }
        if let Some(to) = &self.to {
            match to.try_send(j) {
                Ok(()) => {}
                // (Full can't happen: there is one job, and it was here.)
                Err(TrySendError::Full(j)) => self.job = Some(j),
                Err(TrySendError::Disconnected(j)) => {
                    self.job = Some(j);
                    self.lose();
                }
            }
        }
    }
}

/// Prints made on a worker thread, one at a time (see the module notes).
pub struct Printer {
    rate: u32,
    tx: Option<Sender<ToPrinter>>,
    rx: Option<Receiver<FromPrinter>>,
    /// A buffer from the worker, for the next bounce.
    buf: Option<Vec<f32>>,
    asked: bool,
    /// The print under way, and whether its bounce is rendering.
    doing: Option<Wanted>,
    bouncing: bool,
    /// Prints handed to the looper, and ones made too late to use.
    pub made: u64,
    pub stale: u64,
    pub failed: u64,
    /// A save the worker made room for.
    sized: Option<Saved>,
}

impl Printer {
    fn new(rate: u32) -> Printer {
        let (tx, jobs) = channel::<ToPrinter>();
        let (done, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("mpfree-print".into())
            .spawn(move || {
                // Off the sound's cores and priority, and after the screen.
                mpfree_lanes::os::idle_job();
                for m in jobs {
                    let reply = match m {
                        ToPrinter::Buffer(n) => FromPrinter::Buffer(Vec::with_capacity(n)),
                        ToPrinter::Print(b, frames) => {
                            let p = PrintJob::new(&b, rate, frames).ok().and_then(|mut j| {
                                while !j.run(1 << 15) {}
                                j.finish()
                            });
                            FromPrinter::Done(b.from_loop, frames, p)
                        }
                        ToPrinter::Drop(p, b) => {
                            drop((p, b));
                            continue;
                        }
                        ToPrinter::Size(mut saved) => {
                            for (_, f) in &mut saved.loops {
                                for k in 0..f.layers.len() {
                                    // A copied layer's file holds it (4c.4).
                                    let size = layer_frames(f, k);
                                    if size > 0 {
                                        f.layers[k].audio = vec![0.0; 2 * size];
                                    }
                                }
                            }
                            FromPrinter::Sized(saved)
                        }
                        ToPrinter::DropSaved(saved) => {
                            drop(saved);
                            continue;
                        }
                        ToPrinter::DropLoop(f) => {
                            drop(f);
                            continue;
                        }
                    };
                    if done.send(reply).is_err() {
                        break;
                    }
                }
            })
            .is_ok();
        Printer {
            rate,
            tx: spawned.then_some(tx),
            rx: spawned.then_some(rx),
            buf: None,
            asked: false,
            doing: None,
            bouncing: false,
            made: 0,
            stale: 0,
            failed: 0,
            sized: None,
        }
    }

    fn send(&self, m: ToPrinter) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
    }

    /// One period's look after the looper ran at `bpm` (the song's
    /// tempo; a free loop goes by the free clock's, [`tempo_of`]): hand
    /// over what the worker made, take a finished bounce to it, drop old
    /// prints, and start the next print a loop needs (or take away one it
    /// no longer does). Cheap when nothing changes. It allocates nothing
    /// itself; a message to the worker can (a block of the channel, now
    /// and then), and only goes when a print starts, arrives or is
    /// dropped.
    fn tend(&mut self, looper: &mut Looper, bpm: f64) {
        if self.tx.is_none() || bpm.is_nan() || bpm <= 0.0 {
            return;
        }
        let free = looper.free_clock().map(|c| c.bpm);
        for n in 0..MAX_LOOPS {
            if let Some(p) = looper.take_print(n) {
                self.send(ToPrinter::Drop(Some(p), Vec::new()));
            }
            if let Some(f) = looper.take_loaded(n) {
                self.send(ToPrinter::DropLoop(f));
            }
        }
        while let Some(m) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                FromPrinter::Sized(saved) => self.sized = Some(saved),
                FromPrinter::Buffer(b) => {
                    self.asked = false;
                    self.buf = Some(b);
                }
                FromPrinter::Done(n, frames, p) => {
                    self.doing = None;
                    let Some(p) = p else {
                        self.failed += 1;
                        continue;
                    };
                    let now = looper.loop_status(n);
                    let wanted = now.is_some_and(|s| {
                        s.version == p.version
                            && Some(frames)
                                == tempo_of(&s, bpm, free).and_then(|b| want(&s, b, self.rate))
                    });
                    if !wanted {
                        self.stale += 1;
                        self.send(ToPrinter::Drop(Some(p), Vec::new()));
                        continue;
                    }
                    match looper.print(n, Some(p)) {
                        Ok(()) => self.made += 1,
                        Err(p) => self.send(ToPrinter::Drop(p, Vec::new())),
                    }
                }
            }
        }
        if self.bouncing {
            match looper.take_bounce() {
                Some(b) => {
                    self.bouncing = false;
                    let d = self.doing.expect("a print is under way");
                    if b.complete && b.version == d.version {
                        self.send(ToPrinter::Print(b, d.frames));
                    } else {
                        self.doing = None;
                        self.send(ToPrinter::Drop(None, b.audio));
                    }
                }
                None if looper.bounce_progress().is_none() => {
                    self.bouncing = false;
                    self.doing = None;
                }
                None => {}
            }
        }
        if self.doing.is_some() {
            return;
        }
        for n in 0..MAX_LOOPS {
            let Some(s) = looper.loop_status(n) else {
                break;
            };
            let have = looper.print_info(n);
            match tempo_of(&s, bpm, free).and_then(|b| want(&s, b, self.rate)) {
                None if have.is_some() && s.phase == Phase::Playing => {
                    let _ = looper.print(n, None);
                }
                None => {}
                Some(frames) if have == Some((frames, s.ticks, s.version)) => {}
                Some(frames) => {
                    let need = 2 * s.frames;
                    match self.buf.take() {
                        Some(b) if b.capacity() >= need => match looper.bounce_print(n, b) {
                            Ok(()) => {
                                self.doing = Some(Wanted {
                                    n,
                                    frames,
                                    version: s.version,
                                });
                                self.bouncing = true;
                            }
                            Err(b) => self.buf = Some(b),
                        },
                        Some(b) => {
                            self.send(ToPrinter::Drop(None, b));
                            self.ask(need);
                        }
                        None => self.ask(need),
                    }
                    return;
                }
            }
        }
    }

    fn ask(&mut self, samples: usize) {
        if !self.asked {
            self.asked = true;
            self.send(ToPrinter::Buffer(samples));
        }
    }
}

/// The tempo loop `s` plays at: a free loop ([`Clock::Free`]) the free
/// clock's, `free` (None without one), so a tempo change in the song never
/// asks for a print of it; any other the song's, `song`.
fn tempo_of(s: &Status, song: f64, free: Option<f64>) -> Option<f64> {
    match s.clock {
        Clock::Free => free,
        Clock::Song => Some(song),
    }
}

/// The print loop `s` needs at `bpm`: its length then, if that is not its
/// own (within a frame) and it is playing; within the stretch's 10x. At
/// another speed ([`Status::rate`]) it always wants one: the print makes
/// the octaves cleanly, and an overdub waits for it to fit to.
fn want(s: &Status, bpm: f64, rate: u32) -> Option<usize> {
    if s.phase != Phase::Playing || s.frames == 0 || s.ticks <= 0 {
        return None;
    }
    let frames = frames_at(s.ticks, bpm, rate);
    // The audio's length once resampled to its speed, which the stretch
    // then takes to `frames`.
    let sped = s.frames as f64 * 2f64.powi(-i32::from(s.rate));
    let r = frames as f64 / sped;
    let off = s.rate != 0 || frames.abs_diff(s.frames) > 1;
    (off && (0.1..=10.0).contains(&r)).then_some(frames)
}

impl LoopHost {
    /// Room for `seconds` of stereo loop (all layers together).
    pub fn new(rate: u32, seconds: u32) -> LoopHost {
        let looper = Looper::new(rate, seconds);
        let last = looper.status();
        LoopHost {
            looper,
            line: LineIn::closed(),
            monitor: false,
            source: LoopSource::Pads,
            chosen: LoopSource::Pads,
            resample: false,
            input_l: [0.0; PERIOD],
            input_r: [0.0; PERIOD],
            pads_l: [0.0; PERIOD],
            pads_r: [0.0; PERIOD],
            pads_latency: 0,
            pads_dry: None,
            pads: None,
            pads_now: None,
            pads_strip: None,
            held_said: None,
            dropouts: 0,
            latency_set: 0,
            by_ear_set: false,
            played_at: None,
            starts_refused: 0,
            free_tempo: false,
            tempo_meant: false,
            smart: SmartEnd::Off,
            notes_lost: 0,
            snap_want: None,
            snap_ask: None,
            asks_told: 0,
            snap_wait: None,
            free_snap: None,
            side_want: None,
            ab: None,
            kept: None,
            close_call: None,
            point_open: false,
            point: None,
            point_buf: vec![[0.0; 2]; point_frames(rate)],
            snaps_refused: 0,
            readings_overruled: 0,
            tempos_refused: 0,
            match_pattern: false,
            starts: Starts::Bar,
            clock: Clock::Song,
            for_cell: false,
            capture_span: None,
            pattern_bars: None,
            last_rolling: false,
            input: vec![0.0; 2 * PERIOD],
            out: vec![0.0; 2 * PERIOD],
            last,
            clipped: 0,
            wave: WaveBuilder::new(),
            live: LiveWave::new(),
            waves: WavePool::new(),
            wave_shown: false,
            cells: CellWaves::new(),
            meter: [0.0; 2],
            beats_per_bar: 4,
            bpm: 120.0,
            rate,
            printer: Printer::new(rate),
            finder: Finder::new(rate),
            by_ear_smart: false,
            saving: None,
            saved: None,
            save_restarts: 0,
            clear_next: false,
            emptying: false,
            incoming: None,
            settle: 0,
            steps_left: 0,
            picked: 0,
            copy: None,
            copy_end: None,
            on_tracks: Default::default(),
            recordings: false,
            layer_recs: Default::default(),
            layer_writer: Default::default(),
            heard_peaks: vec![
                0.0;
                if seconds > 0 {
                    (rate as u64 * u64::from(HEARD_SECONDS)).div_ceil(HEARD_CHUNK) as usize + 1
                } else {
                    0
                }
            ],
            bar_ticks: 4 * PPQ as Tick,
        }
    }

    /// Save every loop for a project: once no loop is recording a first
    /// pass, overdubbing, merging, fitting or loading, the worker makes
    /// room, and each loop's layers are read as they play ([`SAVE_SPEED`]
    /// frames for each frame), with no glitch. A loop that changes while
    /// it is read starts the save again. [`LoopHost::take_saved`] gives
    /// it when done, with each layer's audio: the project keeps the text
    /// (`mpfree_looper::save::to_text`, each layer naming its file) and a
    /// WAV a layer (`save::wav`). Allocates only the list of loops, here
    /// and when it starts again.
    pub fn save(&mut self) -> String {
        if self.printer.tx.is_none() {
            return "looper: cannot save, the worker did not start".into();
        }
        if let Some(s) = self.saved.take() {
            self.printer.send(ToPrinter::DropSaved(s));
        }
        if let Some(Saving::Reading { saved, .. }) = self.saving.take() {
            self.printer.send(ToPrinter::DropSaved(saved));
        }
        self.saving = Some(Saving::Waiting);
        "looper: saving the loops".into()
    }

    /// How far the save is, 0 to 1 (0 while it waits for a loop).
    pub fn saving(&self) -> Option<f32> {
        match self.saving.as_ref()? {
            Saving::Reading {
                saved, i, k, frame, ..
            } => {
                // Frames of a loop's first `upto` layers.
                let frames = |f: &LoopFile, upto: usize| -> usize {
                    (0..upto.min(f.layers.len()))
                        .map(|j| layer_frames(f, j))
                        .sum()
                };
                let all: usize = saved
                    .loops
                    .iter()
                    .map(|(_, f)| frames(f, f.layers.len()))
                    .sum();
                let done: usize = saved.loops[..*i]
                    .iter()
                    .map(|(_, f)| frames(f, f.layers.len()))
                    .sum::<usize>()
                    + saved.loops.get(*i).map_or(0, |(_, f)| frames(f, *k))
                    + frame;
                Some(done as f32 / all.max(1) as f32)
            }
            _ => Some(0.0),
        }
    }

    /// The save, once done: drop it away from the audio loop.
    pub fn take_saved(&mut self) -> Option<Saved> {
        self.saved.take()
    }

    /// A save under way stops, and one done but not taken is dropped (a
    /// project opening while the save of another goes on). Does not
    /// allocate.
    pub fn cancel_save(&mut self) {
        if let Some(s) = self.saved.take() {
            self.printer.send(ToPrinter::DropSaved(s));
        }
        if let Some(Saving::Reading { saved, .. }) = self.saving.take() {
            self.printer.send(ToPrinter::DropSaved(saved));
        }
    }

    /// Loops taken from a save that is not going ahead, dropped on the
    /// worker rather than here.
    pub fn drop_saved(&self, s: Saved) {
        self.printer.send(ToPrinter::DropSaved(s));
    }

    /// An open or New project: a save under way stops, and every loop
    /// fades out and is forgotten (as Clear does), from the next periods.
    /// The project's loops, if it has any, come with [`LoopHost::open`]
    /// and go in once every loop is empty ([`LoopHost::tend_open`]).
    pub fn clear_all(&mut self) -> String {
        self.cancel_save();
        if let Some(s) = self.incoming.take() {
            self.printer.send(ToPrinter::DropSaved(s));
        }
        self.steps_left = 0;
        self.clear_next = true;
        self.emptying = true;
        self.on_tracks.forget();
        self.tend_open();
        "looper: every loop fades out for the project".into()
    }

    /// An opened project's loops: they go in once every loop is empty.
    pub fn open(&mut self, saved: Saved) {
        if let Some(s) = self.incoming.replace(saved) {
            self.printer.send(ToPrinter::DropSaved(s));
        }
    }

    /// Whether an open's loops are still going out or coming in.
    pub fn opening(&self) -> bool {
        self.clear_next || self.emptying || self.incoming.is_some()
    }

    /// Between periods while [`LoopHost::opening`]: the clears once the
    /// queue has room, then the project's loops once every loop is empty.
    /// Returns a line for the log when they go in.
    pub fn tend_open(&mut self) -> Option<String> {
        if self.clear_next {
            if self.looper.room() < 2 * MAX_LOOPS + 1 {
                return None;
            }
            for n in 0..MAX_LOOPS {
                self.looper.select_loop(n);
                self.looper.command(Command::Forget);
            }
            self.looper.select_loop(0);
            self.picked = 0;
            self.for_cell = false;
            self.clear_next = false;
            // The clears apply with the next period: a load before then
            // would be cleared with them.
            self.settle = 2;
            return None;
        }
        if self.settle > 0 {
            return None;
        }
        let empty = (0..MAX_LOOPS).all(|n| {
            self.looper
                .loop_status(n)
                .is_some_and(|s| s.phase == Phase::Empty && s.loading.is_none())
        });
        if !empty {
            return None;
        }
        self.emptying = false;
        let saved = self.incoming.take()?;
        Some(self.load(saved))
    }

    /// Load saved loops (`save::from_text`, each layer's audio from its
    /// WAV with `save::from_wav`) into the loops they came
    /// from, which must be empty: each copies in while silent, then fades
    /// in, in time with the song. Free loops load onto the saved free
    /// clock, opened first, stopped: they open stopped at their tops, as
    /// a project opens with nothing launched. A loop that is not empty
    /// keeps what it has (its saved one is dropped on the worker), and
    /// free loops can't load while free loops still hold the free clock.
    pub fn load(&mut self, saved: Saved) -> String {
        let clock = saved
            .free_clock
            .filter(|c| self.looper.open_free_clock(c.bpm, c.meter, c.top));
        let (mut done, mut kept, mut free, mut unclocked) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (n, f) in saved.loops {
            let id = f.id;
            let on_free = f.clock == Clock::Free;
            let mut recs = [0u128; mpfree_looper::MAX_LAYERS];
            let mut from = [[0u128; mpfree_looper::MAX_LAYERS]; mpfree_looper::MAX_LAYERS];
            for ((r, m), y) in recs.iter_mut().zip(from.iter_mut()).zip(&f.layers) {
                *r = y.raw.as_ref().map_or(0, |r| r.rec);
                for (a, &b) in m
                    .iter_mut()
                    .zip(y.raw.as_ref().map_or(&[][..], |r| &r.from))
                {
                    *a = b;
                }
            }
            match self.looper.load(n, f) {
                Ok(()) => {
                    self.on_tracks.set(n, id);
                    self.layer_recs.opened(&self.looper, n, &recs, &from);
                    done.push((n + 1).to_string());
                    if on_free {
                        free.push((n + 1).to_string());
                    }
                }
                Err(f) => {
                    if on_free && clock.is_none() {
                        unclocked.push((n + 1).to_string());
                    } else {
                        kept.push((n + 1).to_string());
                    }
                    self.printer.send(ToPrinter::DropLoop(f));
                }
            }
        }
        self.looper.select_loop(saved.selected);
        self.picked = saved.selected;
        self.for_cell = false;
        let mut note = format!("looper: loading loops {}", done.join(", "));
        if done.is_empty() {
            note = "looper: no loops to load".into();
        }
        if let Some(c) = clock.filter(|_| !free.is_empty()) {
            note += &format!(
                "; loops {} are free loops, stopped at their tops: PLAY starts them with \
                 the song ({:.2} BPM in {}/{})",
                free.join(", "),
                c.bpm,
                c.meter.beats,
                c.meter.unit
            );
        }
        if !kept.is_empty() {
            note += &format!(
                "; loops {} were not empty and kept what they had",
                kept.join(", ")
            );
        }
        if !unclocked.is_empty() {
            note += &format!(
                "; free loops {} weren't loaded: other free loops still hold the free clock",
                unclocked.join(", ")
            );
        }
        if saved.rate != self.rate {
            note += &format!(
                " (saved at {} Hz: they play like tape until their prints are in)",
                saved.rate
            );
        }
        note
    }

    /// Each loop's (version, layers, frames) now.
    fn shapes(&self) -> [(u64, usize, usize); MAX_LOOPS] {
        let mut at = [(0, 0, 0); MAX_LOOPS];
        for (n, a) in at.iter_mut().enumerate() {
            if let Some(s) = self.looper.loop_status(n) {
                *a = (s.version, s.layers, s.frames);
            }
        }
        at
    }

    /// One period of a save: see [`LoopHost::save`].
    fn tend_save(&mut self) {
        let Some(state) = self.saving.take() else {
            return;
        };
        self.saving = match state {
            Saving::Waiting => {
                // Every loop empty (or armed) or still enough to read; with
                // recordings on, every layer closed too (a first pass's
                // tail in), so a snapshot can read each by its number.
                let snap = self.recordings;
                let ready = (0..MAX_LOOPS).all(|n| {
                    let s = self.looper.loop_status(n).expect("a loop");
                    matches!(s.phase, Phase::Empty | Phase::Armed) && s.loading.is_none()
                        || self.looper.read_layer(n, 0, 0, &mut []).is_some()
                            && (!snap
                                || (0..MAX_LAYERS)
                                    .map_while(|k| self.looper.loop_layer_info(n, k))
                                    .all(|y| y.closed))
                });
                if ready {
                    let mut saved = Saved {
                        rate: self.rate,
                        selected: self.looper.selected(),
                        loops: (0..MAX_LOOPS)
                            .filter_map(|n| {
                                let mut f = match self.recordings {
                                    true => self.looper.loop_file_raw(n)?,
                                    false => self.looper.loop_file(n)?,
                                };
                                // Its lasting id, if it has one (phase 4b).
                                f.id = self.on_tracks.id(n).map_or(0, |i| i.0);
                                // Each layer's recording (4c.2); one whose
                                // file is synced is copied from it (4c.4).
                                for (k, y) in f.layers.iter_mut().enumerate() {
                                    if let (Some(r), Some(info)) =
                                        (y.raw.as_mut(), self.looper.loop_layer_info(n, k))
                                    {
                                        r.rec = self.layer_recs.of(n, &info);
                                        // What a merge or a fit made it
                                        // from (4c.5).
                                        r.from = self.layer_recs.sources_of(n, &info).to_vec();
                                        if snap
                                            && r.rec != 0
                                            && self.layer_recs.file(n, &info)
                                                == crate::loops_on_tracks::FileState::Synced
                                        {
                                            y.file = format!("loop_{:032x}.wav", r.rec);
                                        }
                                    }
                                }
                                Some((n, f))
                            })
                            .collect(),
                        free_clock: self
                            .looper
                            .free_clock()
                            .map(|c| mpfree_looper::save::ClockFile::of(&c)),
                        kept: Vec::new(),
                    };
                    // Recordings held for an Undo that no layer names
                    // (4c.6): named, so the project keeps their files.
                    if snap {
                        saved.kept = self.layer_recs.held_files(&self.looper);
                        saved.kept.retain(|f| {
                            !saved
                                .loops
                                .iter()
                                .any(|(_, l)| l.layers.iter().any(|y| y.file == *f))
                        });
                    }
                    // The snapshot: each layer by its number.
                    let borns = snap.then(|| {
                        let mut b = [[0; MAX_LAYERS]; MAX_LOOPS];
                        for (n, row) in b.iter_mut().enumerate() {
                            for (k, x) in row.iter_mut().enumerate() {
                                match self.looper.loop_layer_info(n, k) {
                                    Some(y) => *x = y.born,
                                    None => break,
                                }
                            }
                        }
                        b
                    });
                    self.printer.send(ToPrinter::Size(saved));
                    Some(Saving::Sizing {
                        at: self.shapes(),
                        snap: borns,
                    })
                } else {
                    Some(Saving::Waiting)
                }
            }
            Saving::Sizing { at, snap } => match self.printer.sized.take() {
                Some(saved) => Some(Saving::Reading {
                    saved,
                    at,
                    snap,
                    i: 0,
                    k: 0,
                    frame: 0,
                }),
                None => Some(Saving::Sizing { at, snap }),
            },
            Saving::Reading {
                mut saved,
                at,
                snap,
                mut i,
                mut k,
                mut frame,
            } => {
                // A snapshot reads closed layers by their numbers, which
                // never change, so the loops may go on; without one, a
                // loop that changes starts the save again.
                if snap.is_none() && self.shapes() != at {
                    // A loop changed: start again.
                    self.save_restarts += 1;
                    self.printer.send(ToPrinter::DropSaved(saved));
                    self.saving = Some(Saving::Waiting);
                    return;
                }
                let mut budget = SAVE_SPEED * PERIOD;
                while budget > 0 && i < saved.loops.len() {
                    let (n, f) = &mut saved.loops[i];
                    let size = layer_frames(f, k);
                    let take = budget.min(size - frame);
                    let raw = f.layers[k].raw.is_some();
                    let out = &mut f.layers[k].audio[2 * frame..2 * (frame + take)];
                    // By its number (a snapshot, 4c.4), as written (4c.2)
                    // or as it plays.
                    let read = match (snap.as_ref(), raw) {
                        _ if take == 0 => Some(0),
                        (Some(b), _) => self.looper.raw_of(*n, b[*n][k], frame, out).map(|_| 0),
                        (None, true) => self.looper.raw_layer(*n, k, frame, out),
                        (None, false) => self.looper.read_layer(*n, k, frame, out),
                    };
                    if read.is_none() && snap.is_some() {
                        // The layer went from memory before it was read
                        // (cleared, undone): start again.
                        self.save_restarts += 1;
                        self.printer.send(ToPrinter::DropSaved(saved));
                        self.saving = Some(Saving::Waiting);
                        return;
                    }
                    if read.is_none() {
                        // Busy for now (a merge or a fit starting): wait.
                        break;
                    }
                    budget -= take;
                    frame += take;
                    if frame == size {
                        frame = 0;
                        k += 1;
                        if k == f.layers.len() {
                            k = 0;
                            i += 1;
                        }
                    }
                }
                if i == saved.loops.len() {
                    self.saved = Some(saved);
                    None
                } else {
                    Some(Saving::Reading {
                        saved,
                        at,
                        snap,
                        i,
                        k,
                        frame,
                    })
                }
            }
        };
    }

    /// A press or a setting, with its `looper:` line for the log.
    pub fn command(&mut self, c: Command) -> String {
        // UNDO or REDO of a snap or of Read as lands on the loop's
        // downbeat with the tempo, as the step did.
        let h = self.looper.history();
        let tempo = |s: Option<&Step>| matches!(s, Some(Step::Snap(_) | Step::Bars(_)));
        match c {
            Command::Undo if tempo(h.done().last()) => return self.snap(Snap::Undo),
            Command::Redo if tempo(h.redo().first()) => return self.snap(Snap::Redo),
            _ => {}
        }
        // The app's latency is the line input's; the pads have their own.
        // Records from: Loops makes a new loop a resample.
        let empty = self
            .looper
            .loop_status(self.picked)
            .is_some_and(|s| s.phase == Phase::Empty);
        let capturing = self
            .looper
            .loop_status(self.picked)
            .is_some_and(|s| s.capturing.is_some());
        // Length: Pattern. The first pass takes the pattern's length, kept
        // up to date by set_pattern_bars; any other Length lets go of it.
        if let Command::SetBars(b) = c {
            self.match_pattern = b == Some(PATTERN_BARS);
            // A new Length: What you played's window takes it.
            self.capture_span = None;
        }
        // Starts and Sync stick: every empty loop records as they were
        // last set.
        if let Command::SetStarts(w) = c {
            self.starts = w;
        }
        if let Command::SetClock(k) = c {
            if k != self.clock {
                // What you played counts the other clock's bars now.
                self.capture_span = None;
            }
            self.clock = k;
            // Set by hand, it is the picked loop's too (a Loop cell armed
            // for it lets go on Free: `desk::cell_lets_go`).
            self.for_cell = false;
            // A loop with sound, or sound coming in, keeps its own: the
            // looper would refuse it.
            let takes = self.looper.loop_status(self.picked).is_some_and(|s| {
                s.phase == Phase::Empty
                    && s.loading.is_none()
                    && s.copying.is_none()
                    && s.capturing.is_none()
            });
            if !takes {
                let sync = match k {
                    Clock::Song => "the song",
                    Clock::Free => "free",
                };
                return format!(
                    "looper: sync: {sync} from the next empty loop on (loop {} keeps its own)",
                    self.picked + 1
                );
            }
        }
        if c == Command::Record && empty {
            let clock = if self.for_cell {
                Clock::Song
            } else {
                self.clock
            };
            self.sticky_settings(self.picked, clock);
        }
        if self.match_pattern
            && c == Command::Record
            && empty
            && self.looper.status().bars != self.pattern_bars
        {
            self.looper.command(Command::SetBars(self.pattern_bars));
        }
        let c = match c {
            Command::SetBars(Some(PATTERN_BARS)) => Command::SetBars(self.pattern_bars),
            Command::SetLatency(_) if self.source.inside() => {
                Command::SetLatency(self.pads_latency)
            }
            // Record (a ring, REC) on a loop capturing calls it off, as
            // Capture again does.
            Command::Record if capturing => Command::Capture(1),
            Command::Record if self.resample && empty => Command::Resample,
            c => c,
        };
        if let Command::SetLatency(l) = c {
            self.latency_set = l;
        }
        self.steps_left = 0;
        let s = self.looper.status();
        let moved = self.moved_text(c, &s);
        let what = match (c, s.phase) {
            (Command::Undo | Command::Redo, Phase::Playing) if moved.is_some() => {
                moved.unwrap_or_default()
            }
            (Command::Record, Phase::Empty) if s.clock == Clock::Free && !self.resample => {
                self.free_record_words(&s)
            }
            (Command::Record, Phase::Recording) if s.clock == Clock::Free && s.free => format!(
                "Record during the free loop's first pass: it closes (once it is {} at \
                 {LOOP_MAX_BPM:.0} BPM), and its length sets the free loops' tempo",
                shortest_words(s.bars)
            ),
            (Command::Record, Phase::Empty) if s.starts == Starts::Play && !self.resample => {
                format!(
                    "Record: armed, the first pass starts when you play {} ({})",
                    self.first_note(),
                    if s.free {
                        format!(
                            "{}: REC again closes it, and its length sets the tempo",
                            free_words(s.bars)
                        )
                    } else {
                        bars_text(s.bars)
                    }
                )
            }
            (Command::Record, Phase::Empty) if s.free && !self.resample => format!(
                "Record: the first pass records {} from now: REC again closes it, \
                 and its length sets the tempo and starts the song",
                free_words(s.bars)
            ),
            (Command::Record, Phase::Empty) => format!(
                "Record: armed, the first pass starts on the next bar line ({})",
                bars_text(s.bars)
            ),
            (Command::Record, Phase::Armed) => "Record again: disarmed".to_string(),
            (Command::Record, Phase::Recording) if s.free => format!(
                "Record during the free first pass: it closes (once it is {} at \
                 {LOOP_MAX_BPM:.0} BPM), and its length sets the tempo",
                shortest_words(s.bars)
            ),
            (Command::Record, Phase::Recording) => {
                "Record during the first pass: it ends on the next bar line".to_string()
            }
            // Pressed again while it waits for its print or its fit.
            (Command::Record | Command::Overdub, Phase::Playing) if s.overdub_next => {
                "Overdub again: the overdub waiting for the fit is off".to_string()
            }
            (Command::Record | Command::Overdub, Phase::Playing) if s.rate != 0 && !s.printed => {
                format!(
                    "overdub on, once the loop's print at {} is in and the loop is fitted to it \
                     (its print becomes its layer; Undo takes the fit back)",
                    crate::looper_page::speed_text(s.speed)
                )
            }
            (Command::Record | Command::Overdub, Phase::Playing) if s.printed => {
                "overdub on, once the loop is fitted to the tempo (its print becomes its layer; \
                 Undo takes the fit back)"
                    .to_string()
            }
            (Command::Record | Command::Overdub, Phase::Playing) => {
                format!("overdub on (layer {})", s.layers + 1)
            }
            (Command::Record | Command::Overdub, Phase::Overdubbing) => "overdub off".to_string(),
            (Command::Overdub, Phase::Recording) if !s.overdub_next => {
                "Overdub during the first pass: the overdub \
                                                                         starts where it ends"
                    .to_string()
            }
            (Command::Overdub, Phase::Recording) => {
                "Overdub again: no overdub after the first pass".to_string()
            }
            (Command::Overdub, _) => format!("Overdub: nothing to overdub ({:?})", s.phase),
            (Command::Undo, Phase::Armed | Phase::Recording) => {
                "Undo: first pass thrown away".to_string()
            }
            (Command::Undo, Phase::Overdubbing) => {
                "Undo: the overdub in progress fades out".to_string()
            }
            (Command::Undo, Phase::Playing) if s.layers > 1 => {
                format!("Undo: layer {} fades out", s.layers)
            }
            (Command::Undo | Command::Capture(_), Phase::Empty) if s.capturing.is_some() => {
                format!("Capture into Loop {} called off", self.picked + 1)
            }
            (Command::Undo, Phase::Empty) if s.kept => {
                "Undo: the cleared loop comes back".to_string()
            }
            (Command::Undo, _) => {
                "Undo: nothing to undo (Clear removes the first pass)".to_string()
            }
            (Command::Redo, Phase::Playing) if s.redo > 0 => {
                format!("Redo: layer {} fades back in", s.layers + 1)
            }
            (Command::Redo, _) => "Redo: nothing to redo".to_string(),
            (Command::Clear, Phase::Empty) => "Clear: nothing to clear".to_string(),
            (Command::Clear, _) => format!(
                "Clear: the loop fades out ({} layers); UNDO brings it back",
                s.layers
            ),
            (Command::Forget, Phase::Empty) => "Forget: nothing kept".to_string(),
            (Command::Forget, _) => {
                format!("Forget: the loop fades out ({} layers), no undo", s.layers)
            }
            (Command::Solo(o), _) => {
                if o {
                    "soloed: the loops not soloed go quiet".to_string()
                } else {
                    "solo off".to_string()
                }
            }
            (Command::SetOverdubs(o), _) => format!("overdubs: {}", overdubs_text(o)),
            (Command::Mute(m), _) => {
                if m {
                    "muted".to_string()
                } else {
                    "unmuted".to_string()
                }
            }
            (Command::SetBars(b), _) => format!("first pass length {}", bars_text(b)),
            (Command::SetAfter(a), _) => format!("after recording: {a:?}"),
            (Command::SetStarts(Starts::Bar), _) => "starts: on the bar".to_string(),
            (Command::SetStarts(Starts::Play), _) => {
                format!("starts: when you play {}", self.first_note())
            }
            (Command::Played(f), _) => format!("a note played on frame {f}"),
            (Command::StartsByEar(on), _) => format!(
                "a loop that starts when you play listens to {}",
                if on { "the line input" } else { "the pads" }
            ),
            (Command::TempoFromLoop(on), _) => format!(
                "tempo from the first loop: {}",
                if on {
                    "on (a free first pass with the song stopped sets the tempo)"
                } else {
                    "off"
                }
            ),
            (Command::SetClock(Clock::Song), _) => "sync: the song".to_string(),
            (Command::SetClock(Clock::Free), _) => {
                "sync: free (the loop keeps its own time)".to_string()
            }
            (Command::FreeStop, _) => "free loops: stop, at their tops".to_string(),
            (Command::FreePlay, _) => "free loops: play from their tops".to_string(),
            (Command::UseReading, _) => match s.asks {
                Some(r) => format!(
                    "use the reading: it plays as {} from its top",
                    reading_words(&r)
                ),
                None => "use the reading: nothing waits on its Length".to_string(),
            },
            (Command::SetSmartEnd(m), _) => format!("smart loop end: {}", smart_end_text(m)),
            (Command::SmartByEar(on), _) => format!(
                "smart loop end listens to {}",
                if on {
                    "the hits it hears in the take"
                } else {
                    "the pads and MIDI notes"
                }
            ),
            (Command::TempoMeant(on), _) => format!(
                "the song's tempo counts as {} when a free first loop's length is read",
                if on {
                    "set by hand"
                } else {
                    "left from before"
                }
            ),
            (Command::SetFadeOld(k), _) => format!("fade old: {:.0} % kept a pass", 100.0 * k),
            (Command::SetLatency(l), _) => {
                format!("latency {l} frames ({:.1} ms)", l as f64 / 44.1)
            }
            (Command::SetLevel(v), _) => format!("loop level {:.1} dB", 20.0 * v.max(1e-6).log10()),
            (Command::Edit(Edit::Speed(v)), Phase::Playing) if v == s.speed => {
                format!("speed {}: already", crate::looper_page::speed_text(v))
            }
            (Command::Edit(Edit::Speed(v)), Phase::Playing) => format!(
                "speed {}: {} (Undo takes it back)",
                crate::looper_page::speed_text(v),
                match v - s.speed {
                    -2 => "two octaves down, four times the bars",
                    -1 => "an octave down, twice the bars",
                    1 => "an octave up, half the bars",
                    _ => "two octaves up, a quarter of the bars",
                }
            ),
            (Command::Edit(e), Phase::Playing) => format!("edit {e:?}"),
            (Command::Edit(e), _) => format!("edit {e:?}: waits, or is refused while recording"),
            (Command::Merge, Phase::Playing) => {
                format!("merge {} layers into one, in the background", s.layers)
            }
            (Command::Merge, _) => "merge: refused while recording".to_string(),
            (Command::Resample, Phase::Empty) => {
                "Resample: armed, the first pass records the other loops".to_string()
            }
            (Command::Resample, Phase::Armed | Phase::Recording) if s.resampling => {
                "Resample again: as Record".to_string()
            }
            (Command::Resample, _) => "Resample: only on an empty loop".to_string(),
            (Command::Fit, Phase::Playing) if s.printed => format!(
                "fit to the tempo: the print becomes the loop's one layer ({} layers before; Undo takes it back)",
                s.layers
            ),
            (Command::Fit, _) => "fit: refused, no print plays".to_string(),
            (Command::Capture(b), _) => format!("Capture: the last {} heard", bars_words_n(b)),
            (Command::CaptureTo(end, b), _) => format!(
                "Capture: {} up to the bar line on tick {end}",
                bars_words_n(b)
            ),
            (Command::NewInput, _) => "Capture forgets what it heard before".to_string(),
            (Command::ForgetRedo, _) => "every loop forgets what Redo could bring back".into(),
            (Command::RetireThrough(t), _) => {
                format!("every loop lets go of its steps up to transaction {}", t.0)
            }
            (Command::AbSeam(true), _) => "A/B on the seam".to_string(),
            (Command::AbSeam(false), _) => "A/B off".to_string(),
        };
        if !self.looper.command(c) {
            return format!("looper: {what}: NOT done, the command queue is full");
        }
        format!("looper: {what}")
    }

    /// What Record does on an empty free loop `s` ([`Clock::Free`]), for
    /// the log: with nothing playing and no free clock yet, a pass in
    /// your own time whose length sets the free clock; else the free
    /// clock's bar lines, or the song's while the song plays and the free
    /// clock doesn't (the free clock is set, or starts, there). The song
    /// never starts or stops for it.
    fn free_record_words(&self, s: &Status) -> String {
        let starts = |now: &str| match s.starts {
            Starts::Play => format!(
                "Record: armed, the first pass starts when you play {}",
                self.first_note()
            ),
            Starts::Bar => format!("Record: {now}"),
        };
        match self.looper.free_clock() {
            None if s.free => format!(
                "{}, free in your own time ({}): REC again closes it, and its length sets \
                 the free loops' tempo (the song stays stopped)",
                starts("the first pass records from now"),
                free_words(s.bars)
            ),
            None => format!(
                "{} ({}), and the free loops take the song's tempo there",
                starts("armed, the free loop's first pass starts on the song's next bar line"),
                bars_text(s.bars)
            ),
            Some(c) if c.running => format!(
                "{} ({})",
                starts("armed, the free loop's first pass starts on the free loops' next bar line"),
                bars_text(s.bars)
            ),
            Some(_) if self.last_rolling => format!(
                "{} ({}), and the stopped free loops start with it",
                starts("armed, the free loop's first pass starts on the song's next bar line"),
                bars_text(s.bars)
            ),
            Some(_) => format!(
                "{} ({}), and the stopped free loops start with it",
                starts("the free loop's first pass records from now"),
                bars_text(s.bars)
            ),
        }
    }

    /// What Undo or Redo does when the step it takes back (or brings
    /// back) is a layer dragged between loops: with the other loop's step
    /// too while each is its loop's last ([`Looper::copy_layer`]).
    fn moved_text(&self, c: Command, s: &Status) -> Option<String> {
        let me = self.picked;
        let top = |h: History| match c {
            Command::Undo => h.done().last().copied(),
            _ => h.redo().first().copied(),
        };
        let other = |o: usize| self.looper.loop_history(o).and_then(top);
        let undo = c == Command::Undo;
        match top(self.looper.history())? {
            Step::Added(_, o) => {
                let tied = matches!(other(o), Some(Step::Moved(_, t)) if t == me);
                let o = o + 1;
                Some(match (undo, tied) {
                    (true, true) if s.layers <= 1 => {
                        format!("Undo: the layer goes back to loop {o}, and this loop clears")
                    }
                    (true, true) => format!("Undo: the layer goes back to loop {o}"),
                    (true, false) if s.layers <= 1 => {
                        format!("Undo: the layer from loop {o} goes, and this loop clears")
                    }
                    (true, false) => format!("Undo: the layer from loop {o} fades out"),
                    (false, true) => format!("Redo: the layer moves here from loop {o} again"),
                    (false, false) => format!("Redo: the layer from loop {o} fades back in"),
                })
            }
            Step::Moved(_, o) => {
                let tied = matches!(other(o), Some(Step::Added(_, f)) if f == me);
                let o = o + 1;
                Some(match (undo, tied) {
                    (true, true) => format!("Undo: the layer comes back from loop {o}"),
                    (true, false) => format!("Undo: the layer moved to loop {o} comes back here"),
                    (false, true) => format!("Redo: the layer moves to loop {o} again"),
                    (false, false) => format!("Redo: the layer moved to loop {o} fades out here"),
                })
            }
            _ => None,
        }
    }

    /// The undo history: go back `n` steps (below 0) or forward (above 0),
    /// as if Undo or Redo were pressed that many times, as far as the
    /// picked loop's history goes; with one `looper:` line for the log.
    /// What the command queue can't take now goes in over the next
    /// periods, unless another press comes first.
    pub fn steps(&mut self, n: i32) -> String {
        let h = self.looper.history();
        let (way, room) = if n < 0 {
            ("back", h.done().len())
        } else {
            ("forward", h.redo().len())
        };
        let want = (n.unsigned_abs() as usize).min(room);
        if want == 0 {
            self.steps_left = 0;
            return format!("looper: undo history: nothing to go {way} to");
        }
        self.steps_left = want as i32 * n.signum();
        self.queue_steps();
        format!(
            "looper: undo history: {way} {want} step{}",
            if want == 1 { "" } else { "s" }
        )
    }

    /// UNDO, or REDO (`redo`), of loop `n`'s newest step, from the one
    /// history across the engines (audio tracks phase 4d): as the picked
    /// loop's would be, but the pick stays where it is (the loop is
    /// selected for the command and the pick again after it). A snap's or
    /// Read as's lands on the loop's downbeat with the tempo, as the step
    /// did. With its `looper:` line for the log.
    pub fn undo_loop(&mut self, n: usize, redo: bool) -> String {
        let c = if redo { Command::Redo } else { Command::Undo };
        if n == self.picked && self.looper.selected() == n {
            return self.command(c);
        }
        let Some(h) = self.looper.loop_history(n) else {
            return format!("looper: loop {}: there is no such loop", n + 1);
        };
        let top = if redo {
            h.redo().first()
        } else {
            h.done().last()
        };
        if matches!(top, Some(Step::Snap(_) | Step::Bars(_))) {
            let s = if redo { Snap::Redo } else { Snap::Undo };
            return self.snap_on(n, s);
        }
        if self.looper.room() < 3 {
            return format!(
                "looper: loop {}: {c:?} NOT done, the command queue is full",
                n + 1
            );
        }
        self.steps_left = 0;
        self.looper.select_loop(n);
        self.looper.command(c);
        self.looper.select_loop(self.picked);
        format!("looper: loop {}: {c:?}, from the one history", n + 1)
    }

    /// Loop `n`'s step as UNDO names it ("Overdub · Layer 2"), its layers
    /// by their place in the loop now; None is its Clear, kept for Undo
    /// ([`mpfree_looper::Looper::loop_steps`]).
    pub fn step_words(&self, n: usize, step: Option<Step>) -> String {
        let Some(s) = step else {
            return "Clear".to_string();
        };
        let layers: Vec<Option<LayerInfo>> = (0..MAX_LAYERS)
            .map(|k| self.looper.loop_layer_info(n, k))
            .collect();
        crate::looper_page::step_text(s, &layers, self.rate)
    }

    /// As many of the undo history's presses as the queue takes.
    fn queue_steps(&mut self) {
        while self.steps_left != 0 {
            let c = if self.steps_left < 0 {
                Command::Undo
            } else {
                Command::Redo
            };
            if !self.looper.command(c) {
                break;
            }
            self.steps_left -= self.steps_left.signum();
        }
    }

    /// Pick loop `n` for the presses and controls after this, with its
    /// `looper:` line for the log ("loop 2 picked (empty)").
    pub fn select(&mut self, n: usize) -> String {
        self.for_cell = false;
        self.select_on(n, self.clock)
    }

    /// Pick empty loop `n` to record into a Loop cell armed on the grid:
    /// it takes Sync: Song whatever Sync was last set to, as a free loop
    /// has no cell ([`LoopHost::grid_now`]), and Sync stays as set for
    /// the loops picked after it.
    pub fn select_for_cell(&mut self, n: usize) -> String {
        let line = self.select_on(n, Clock::Song);
        self.for_cell = self.picked == n.min(MAX_LOOPS - 1);
        line
    }

    /// [`LoopHost::select`], an empty loop taking `clock` for its Sync.
    fn select_on(&mut self, n: usize, clock: Clock) -> String {
        self.steps_left = 0;
        let n = n.min(MAX_LOOPS - 1);
        let what = match self.looper.loop_status(n) {
            Some(s) if s.phase == Phase::Empty => "empty".to_string(),
            Some(s) => format!(
                "{} layer{}, {:?}",
                s.layers,
                if s.layers == 1 { "" } else { "s" },
                s.phase
            ),
            None => "not there".to_string(),
        };
        if !self.looper.select_loop(n) {
            return format!(
                "looper: loop {} ({what}): NOT picked, the command queue is full",
                n + 1
            );
        }
        self.picked = n;
        // An empty loop takes Starts and Sync as they were last set (it
        // shows them).
        if self
            .looper
            .loop_status(n)
            .is_some_and(|s| s.phase == Phase::Empty)
        {
            self.sticky_settings(n, clock);
        }
        format!("looper: loop {} picked ({what})", n + 1)
    }

    /// Empty loop `n`, picked, takes Starts as last set and `clock` for
    /// its Sync (as last set, but for a Loop cell's), where it differs (a
    /// loop that is copying in or capturing keeps its Sync: the looper
    /// refuses it).
    fn sticky_settings(&mut self, n: usize, clock: Clock) {
        let Some(s) = self.looper.loop_status(n) else {
            return;
        };
        if s.starts != self.starts {
            self.looper.command(Command::SetStarts(self.starts));
        }
        let busy = s.loading.is_some() || s.copying.is_some() || s.capturing.is_some();
        if s.clock != clock && !busy {
            self.looper.command(Command::SetClock(clock));
        }
    }

    /// Loop `n`'s mute turned over (the picked loop's with None), leaving
    /// the pick as it was: the loop cards' speakers and the Mute button.
    /// STOP with the transport stopped (the round of 16 Nov: REC on the
    /// Looper tab's playing loop started an overdub that waits for PLAY,
    /// OVERDUB and PLAY blinked, and STOP twice left them blinking): an
    /// overdub waiting for the transport is let go and an armed loop
    /// disarmed, on every loop. None when nothing waited.
    pub fn stop_waiting(&mut self) -> Option<String> {
        let picked = self.picked;
        let mut said = Vec::new();
        // Free loops on the free clock while it runs wait for nothing:
        // STOP stops them first (as it stops the song first), and their
        // overdubs and armed passes then wait for PLAY as the song's do.
        let free_runs = self.looper.free_clock().is_some_and(|c| c.running);
        for n in 0..MAX_LOOPS {
            let Some(s) = self.looper.loop_status(n) else {
                continue;
            };
            if free_runs && s.clock == Clock::Free {
                continue;
            }
            let (c, what) = match s.phase {
                Phase::Overdubbing => (Command::Overdub, "the overdub waiting for PLAY is off"),
                Phase::Armed => (Command::Record, "disarmed"),
                _ => continue,
            };
            let need = if n == picked { 1 } else { 3 };
            if self.looper.room() < need {
                said.push(format!(
                    "loop {}: NOT done, the command queue is full",
                    n + 1
                ));
                continue;
            }
            if n != picked {
                self.looper.select_loop(n);
            }
            self.looper.command(c);
            if n != picked {
                self.looper.select_loop(picked);
            }
            said.push(format!("loop {}: {what}", n + 1));
        }
        (!said.is_empty()).then(|| format!("looper: STOP: {}", said.join("; ")))
    }

    /// Tempo follows: Free loops (the looper refactor's R2, tempo
    /// context): the free loops' clock as a span the time engine copies
    /// over each period ([`crate::time::TimeHost::set_lead`]), while it
    /// runs; None while it is stopped or there is none, when the song's
    /// clock leads.
    pub fn tempo_lead(&self) -> Option<Span> {
        self.looper
            .free_clock()
            .filter(|c| c.running)
            .map(|c| lead_span(&c))
    }

    /// SHIFT + STOP and a set's Panic stop everything, the free loops too
    /// (Jay's review of 23 Nov, 5.4): as STOP with the song stopped stops
    /// them ([`Command::FreeStop`]), at once and back to their bar 1. A
    /// Play waiting for the song's bar line is called off, an overdub
    /// ends, and a free first pass in your own time closes there, as a
    /// tap would. None when no free loop plays, waits to or records in
    /// your own time (STOP's [`LoopHost::stop_waiting`] lets go of the
    /// rest).
    pub fn stop_free(&mut self) -> Option<String> {
        let runs = self.looper.free_clock().is_some_and(|c| c.running);
        let own_time = (0..MAX_LOOPS).any(|n| {
            self.looper
                .loop_status(n)
                .is_some_and(|s| s.clock == Clock::Free && s.phase == Phase::Recording && s.free)
        });
        if !(runs || own_time || self.looper.free_cued()) {
            return None;
        }
        if self.looper.room() < 1 {
            return Some(
                "looper: the free loops NOT stopped, the command queue is full".to_string(),
            );
        }
        self.looper.command(Command::FreeStop);
        Some("looper: the free loops stopped, back to their bar 1".to_string())
    }

    /// Loop `n`'s level (0 to 2), leaving the picked loop picked.
    pub fn set_level(&mut self, n: usize, level: f32) -> String {
        let picked = self.picked;
        let n = n.min(MAX_LOOPS - 1);
        let level = level.clamp(0.0, 2.0);
        // Pick it, set it, pick the old one back: all three or none.
        let need = if n == picked { 1 } else { 3 };
        if self.looper.room() < need {
            return format!(
                "looper: loop {} level: NOT set, the command queue is full",
                n + 1
            );
        }
        if n != picked {
            self.looper.select_loop(n);
        }
        self.looper.command(Command::SetLevel(level));
        if n != picked {
            self.looper.select_loop(picked);
        }
        format!(
            "looper: loop {} level {}",
            n + 1,
            crate::looper_tab::level_text(level)
        )
    }

    /// Loop `n`'s solo turned over, leaving the picked loop picked. An
    /// empty loop has nothing to solo.
    pub fn toggle_solo(&mut self, n: usize) -> String {
        let picked = self.picked;
        let n = n.min(MAX_LOOPS - 1);
        let Some(s) = self.looper.loop_status(n) else {
            return format!("looper: loop {}: not there", n + 1);
        };
        if s.phase == Phase::Empty {
            return format!("looper: loop {}: empty, nothing to solo", n + 1);
        }
        let on = !s.solo;
        // Pick it, solo, pick the old one back: all three or none.
        let need = if n == picked { 1 } else { 3 };
        if self.looper.room() < need {
            return format!(
                "looper: loop {} solo: NOT done, the command queue is full",
                n + 1
            );
        }
        if n != picked {
            self.looper.select_loop(n);
        }
        self.looper.command(Command::Solo(on));
        if n != picked {
            self.looper.select_loop(picked);
        }
        format!(
            "looper: loop {} {}",
            n + 1,
            if on {
                "soloed: the loops not soloed go quiet"
            } else {
                "solo off"
            }
        )
    }

    pub fn toggle_mute(&mut self, n: Option<usize>) -> String {
        let n = n.unwrap_or(self.picked).min(MAX_LOOPS - 1);
        let on = !self.looper.loop_status(n).is_some_and(|s| s.muted);
        self.set_mute(n, on)
    }

    /// Loop `n` muted (`on`) or let play, leaving the pick as it was: a
    /// learned control's mute (MIDI Learn), which says which way, so two
    /// moves before the looper takes the first can't undo each other.
    pub fn set_mute(&mut self, n: usize, on: bool) -> String {
        let picked = self.picked;
        let n = n.min(MAX_LOOPS - 1);
        let Some(s) = self.looper.loop_status(n) else {
            return format!("looper: loop {}: not there", n + 1);
        };
        if s.phase == Phase::Empty {
            return format!("looper: loop {}: empty, nothing to mute", n + 1);
        }
        // Pick it, mute, pick the old one back: all three or none.
        let need = if n == picked { 1 } else { 3 };
        if self.looper.room() < need {
            return format!(
                "looper: loop {} {}: NOT done, the command queue is full",
                n + 1,
                if on { "mute" } else { "unmute" }
            );
        }
        if n != picked {
            self.looper.select_loop(n);
        }
        self.looper.command(Command::Mute(on));
        if n != picked {
            self.looper.select_loop(picked);
        }
        format!(
            "looper: loop {} {}",
            n + 1,
            if on {
                "muted: silent, PLAY doesn't start it"
            } else {
                "unmuted: it plays with the others"
            }
        )
    }

    /// Loop `n` on or off, as a clip launches (Loop tracks): from the bar
    /// line at song tick `at`, on the frame it falls on, or with `None` at
    /// once ([`Looper::launch`]). The line for the log.
    pub fn launch(&mut self, n: usize, on: bool, at: Option<mpfree_transport::Tick>) -> String {
        let what = if on { "on" } else { "off" };
        if n >= MAX_LOOPS {
            return format!("looper: loop {}: not there", n + 1);
        }
        if !self.looper.launch(n, on, at) {
            return format!(
                "looper: loop {} {what}: NOT done, the command queue is full",
                n + 1
            );
        }
        match at {
            Some(t) => {
                format!(
                    "looper: loop {} {what} from bar {}",
                    n + 1,
                    t.div_euclid(self.bar_ticks.max(1)) + 1
                )
            }
            None => format!("looper: loop {} {what}", n + 1),
        }
    }

    /// The loops as the Loop tracks need them
    /// ([`crate::time::TimeHost::loops_are`]): which have a first pass, how
    /// long and from where. None while an open's loops are going out or
    /// coming in, so a project's cells aren't emptied by the old loops
    /// fading out. A free loop ([`Clock::Free`]) is told as none: it stays
    /// off the grid and the song (in no cell, so the plan leaves it on and
    /// the looper alone says when it plays), as its bars aren't the
    /// song's.
    pub fn grid_now(&self) -> Option<[LoopNow; MAX_LOOPS]> {
        if self.opening() {
            return None;
        }
        Some(std::array::from_fn(|n| match self.looper.loop_status(n) {
            Some(s)
                if matches!(s.phase, Phase::Playing | Phase::Overdubbing)
                    && s.ticks > 0
                    && s.clock == Clock::Song =>
            {
                LoopNow {
                    pass: true,
                    ticks: s.ticks,
                    origin: s.origin,
                }
            }
            _ => LoopNow::default(),
        }))
    }

    /// The Loop tracks' launches ([`crate::time::TimeHost::loop_plan`]),
    /// before the period: each loop that isn't where the plan has it goes
    /// on or off, on the frame of the bar line the plan names, or at once.
    /// An empty loop waits. Lines for the log; allocates only then. A full
    /// queue tries again next period.
    pub fn follow_plan(&mut self, plan: &LoopPlan) -> Vec<String> {
        let mut lines = Vec::new();
        for n in 0..MAX_LOOPS {
            let Some(s) = self.looper.loop_status(n) else {
                continue;
            };
            let (on, at) = (plan.on[n], plan.at[n]);
            let there = match (s.cue, at) {
                (None, _) => s.on == on,
                (Some(cue), Some(t)) => cue == (on, t),
                (Some(_), None) => false,
            };
            // A loop on an audio track goes where its track says, not
            // where the Loop tracks' plan does.
            let on_track = self.looper.on_track(n);
            if there || on_track || s.phase == Phase::Empty || self.looper.room() == 0 {
                continue;
            }
            lines.push(self.launch(n, on, at));
        }
        lines
    }

    /// A layer dragged onto another loop ([`Looper::copy_layer`]): what
    /// it does, as the screen says it, and why not when it can't. A loop's
    /// only layer goes as a copy (the loop keeps it).
    pub fn copy_layer(&mut self, from: usize, id: u8, to: usize, keep: bool) -> String {
        self.steps_left = 0;
        let (Some(a), Some(b)) = (self.looper.loop_status(from), self.looper.loop_status(to))
        else {
            return "looper: no such loop".to_string();
        };
        let k = (0..MAX_LAYERS).find(|&k| {
            self.looper
                .loop_layer_info(from, k)
                .is_some_and(|l| l.id == id)
        });
        let Some(k) = k else {
            return format!("looper: loop {} has no such layer now", from + 1);
        };
        let bars = |s: &Status| {
            let b = (s.ticks as f64 / self.bar_ticks.max(1) as f64).round();
            format!("{b} bar{}", if b == 1.0 { "" } else { "s" })
        };
        let (layer, there, here) = (k + 1, to + 1, from + 1);
        if from == to {
            return format!("looper: Layer {layer} is in loop {there} already");
        }
        if b.phase != Phase::Empty && b.ticks != a.ticks {
            return format!(
                "looper: loop {there} is {} and Layer {layer} {}: a layer goes only onto a loop of its length",
                bars(&b),
                bars(&a)
            );
        }
        if !matches!(b.phase, Phase::Empty | Phase::Playing) || a.phase != Phase::Playing {
            return format!("looper: Layer {layer} can't go now: a loop is recording");
        }
        let copy = keep || a.layers <= 1;
        if !self.looper.copy_layer(from, id, to, keep) {
            return "looper: the drop is NOT done, the command queue is full".to_string();
        }
        let undo = "UNDO takes it back";
        match (b.phase == Phase::Empty, copy) {
            (true, false) => format!("looper: Layer {layer} moves into loop {there}, a loop of its own ({undo})"),
            (true, true) if a.layers <= 1 => format!(
                "looper: a copy of loop {here} goes into loop {there} (it has one layer, so loop {here} keeps it)"
            ),
            (true, true) => format!("looper: a copy of Layer {layer} goes into loop {there}, a loop of its own"),
            (false, false) => format!("looper: Layer {layer} moves to loop {there} ({undo})"),
            (false, true) if a.layers <= 1 => format!(
                "looper: a copy of Layer {layer} goes onto loop {there} (loop {here} keeps its only layer)"
            ),
            (false, true) => format!("looper: a copy of Layer {layer} goes onto loop {there} ({undo})"),
        }
    }

    /// A layer's control, by the layer's id, with its `looper:` line for
    /// the log ("layer 3 muted").
    pub fn layer_command(&mut self, id: u8, c: LayerCommand) -> String {
        if c == LayerCommand::Delete {
            self.steps_left = 0;
            // The last layer can't be deleted on its own (the looper keeps
            // it): its × clears the loop, as Clear does (13 Nov live test,
            // where the first pass's × said "deleted" and stayed).
            let s = self.looper.status();
            let last = s.layers <= 1
                && (0..MAX_LAYERS).any(|k| self.looper.layer_info(k).is_some_and(|l| l.id == id));
            if last && s.phase != Phase::Empty {
                return self.command(Command::Clear);
            }
        }
        let k = (0..MAX_LAYERS)
            .find(|&k| self.looper.layer_info(k).is_some_and(|l| l.id == id))
            .map_or_else(|| format!("layer id {id}"), |k| format!("layer {}", k + 1));
        let what = match c {
            LayerCommand::Mute(true) => "muted".to_string(),
            LayerCommand::Mute(false) => "unmuted".to_string(),
            LayerCommand::Solo(true) => "soloed".to_string(),
            LayerCommand::Solo(false) => "solo off".to_string(),
            LayerCommand::Level(v) => format!("level {:.1} dB", 20.0 * v.max(1e-6).log10()),
            LayerCommand::Pan(p) => format!("pan {p:+.2}"),
            LayerCommand::Delete => "deleted (Undo brings it back)".to_string(),
        };
        if !self.looper.layer_command(id, c) {
            return format!("looper: {k} {what}: NOT done, the command queue is full");
        }
        format!("looper: {k} {what}")
    }

    /// The pad player's sound for this period (the drums and the synth as
    /// the engines make them, before the mixer): what the looper records
    /// from [`LoopSource::Pads`]. Call it before [`LoopHost::run`] each
    /// period.
    pub fn take_pads(&mut self, left: &[f32; PERIOD], right: &[f32; PERIOD]) {
        self.pads_l = *left;
        self.pads_r = *right;
        self.pads_latency = 0;
    }

    /// What the pads play now (`main.rs`, `pads_instrument`), told each
    /// period: Records from: Pads records it, from the next
    /// take if one records now ([`LoopHost::settle`]).
    pub fn follow_pads(&mut self, now: Instrument) -> Option<(String, bool)> {
        self.pads_now = Some(now);
        self.settle()
    }

    /// Where what the pads play sounds in the mixer
    /// ([`crate::mixing::MixHost::instrument_strip`]; None on MIDI out),
    /// told each period with [`LoopHost::follow_pads`].
    pub fn set_pads_strip(&mut self, s: Option<mpfree_fx::StripId>) {
        self.pads_strip = s;
    }

    /// What the pads play now and where it sounds in the mixer: an audio
    /// track's input list's Pads (phase 4, board 4). None before the pads
    /// were first told.
    pub fn pads_play(&self) -> Option<(Instrument, Option<mpfree_fx::StripId>)> {
        Some((self.pads_now?, self.pads_strip))
    }

    /// Records from picked (the Looper tab, Prefs): this take keeps its
    /// source, so it records from the next one, or at once if none
    /// records now ([`LoopHost::settle`]).
    pub fn set_source(&mut self, s: LoopSource) -> Option<(String, bool)> {
        self.chosen = s;
        self.settle()
    }

    /// A take records one source from its start to its end: the source
    /// picked and, for the pads, the instrument they played when it
    /// started (the design review of 20 Nov, point 6). A source picked,
    /// or the pads moved to another instrument, while a take records
    /// waits for the take's end: said once, on the screen too, and the
    /// Looper tab shows both ([`LooperView::taking_from`]). Then it takes
    /// over, and What you played starts again with it
    /// ([`Command::NewInput`]), so a Capture never spans two sources. A
    /// line for the log when it changes or waits (and whether the screen
    /// says it: a wait).
    fn settle(&mut self) -> Option<(String, bool)> {
        let pads = |s: LoopSource| {
            if s == LoopSource::Pads {
                self.pads_now
            } else {
                None
            }
        };
        let next = (self.chosen, pads(self.chosen));
        let now = (self.source, self.pads);
        if next == now {
            self.held_said = None;
            return None;
        }
        if self.taking() {
            if self.held_said == Some(next) {
                return None;
            }
            self.held_said = Some(next);
            let line = format!(
                "looper: this take keeps recording {}; {} from the next take",
                source_words(now.0, now.1),
                source_words(next.0, next.1)
            );
            return Some((line, true));
        }
        self.held_said = None;
        // Before the app first said what the pads play nothing was heard.
        let heard = now != (LoopSource::Pads, None);
        (self.source, self.pads) = next;
        let restarted = match self.looper.command(Command::NewInput) {
            true if heard => ", What you played starts again",
            true => "",
            false => ", What you played NOT started again (the command queue is full)",
        };
        let line = format!(
            "looper: records {}{restarted}",
            source_words(next.0, next.1)
        );
        Some((line, false))
    }

    /// What this period records from, as a take holds it: the source and,
    /// for the pads, their instrument (`main.rs` takes its sound from the
    /// mixer, `loop_input`).
    pub fn takes_from(&self) -> (LoopSource, Option<Instrument>) {
        (self.source, self.pads)
    }

    /// Whether any loop takes sound now: a first pass, an overdub, or a
    /// Capture waiting for its bar line.
    fn taking(&self) -> bool {
        (0..MAX_LOOPS)
            .filter_map(|n| self.looper.loop_status(n))
            .any(|s| {
                matches!(s.phase, Phase::Recording | Phase::Overdubbing) || s.capturing.is_some()
            })
    }

    /// Why `c` won't start a take on the picked loop: the source it would
    /// record (the pads or a mix) can't be had now, so a take would be
    /// silence. Said on the screen; never another sound in its place.
    /// None lets it through (and every press that ends or calls off a
    /// take, and Loops, which records the loops).
    pub fn refuse(&self, c: Command) -> Option<String> {
        let why = match self.source.line() {
            true => None,
            false => self.pads_dry,
        }?;
        let s = self.looper.loop_status(self.picked)?;
        let starts = match (c, s.phase) {
            (Command::Record | Command::Overdub, Phase::Empty) => {
                s.capturing.is_none() && !self.resample
            }
            (Command::Record | Command::Overdub, Phase::Playing) => true,
            _ => false,
        };
        starts.then(|| format!("Loop {} doesn't record: {why}", self.picked + 1))
    }

    /// The sound of what it records from for this period, as the mixer
    /// had it: the pads or a mix
    /// ([`crate::mixing::MixHost::loop_take`]), `latency` frames late after
    /// the period it lags; or silence, with why it can't be had (never
    /// another sound in its place). A line for the log when that changes
    /// (rarely: a strip routed to a submix, or a plugin's latency), and
    /// whether the screen says it too: while a take records, silence is a
    /// gap in it (a dropout, counted), and its sound back is said too.
    pub fn take_pads_as(
        &mut self,
        left: &[f32; PERIOD],
        right: &[f32; PERIOD],
        how: Result<u32, &'static str>,
    ) -> Option<(String, bool)> {
        self.pads_l = *left;
        self.pads_r = *right;
        self.pads_latency = how.map_or(0, |l| PERIOD as u32 + l);
        let dry = how.err();
        if dry == self.pads_dry {
            return None;
        }
        self.pads_dry = dry;
        // The pads on MIDI out: silence, said by `settle`.
        if self.source == LoopSource::Pads && self.pads == Some(Instrument::MidiOut) {
            return None;
        }
        let taking = self.taking();
        let what = source_words(self.source, self.pads);
        let line = match (dry, taking) {
            (Some(why), true) => {
                self.dropouts += 1;
                format!(
                    "looper: the take records silence from here, {what} can't be recorded: {why}"
                )
            }
            (None, true) => format!("looper: the take hears {what} again"),
            (Some(why), false) => {
                format!("looper: records silence, {what} can't be recorded: {why}")
            }
            (None, false) => format!("looper: hears {what} again"),
        };
        Some((line, taking))
    }

    /// One period for the mixer: read the line input, run the looper on the
    /// transport's `block` with the source's sound, and write the loop's
    /// sound into `left` and `right` (the Loop track). The line input of
    /// the period is [`LoopHost::input`]: the mixer's input strip hears it,
    /// following [`LoopHost::monitor`]. A note comes back if the input had
    /// a problem.
    pub fn run(
        &mut self,
        block: &Block,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Option<String> {
        let note = self.record(block);
        for f in 0..PERIOD {
            left[f] = self.out[2 * f];
            right[f] = self.out[2 * f + 1];
        }
        note
    }

    /// The line input of the last period: this period's only after
    /// [`LoopHost::run`] (or [`LoopHost::period`]) has run; zeros while the
    /// line input is closed; and a period with too little input arrived is
    /// padded with silence at its end ([`LineStats`]' short count says
    /// when), so a gap can reach whatever records it (the sampler's input
    /// buffer, too).
    pub fn input(&self) -> (&[f32; PERIOD], &[f32; PERIOD]) {
        (&self.input_l, &self.input_r)
    }

    /// Layers as recordings (audio tracks phase 4c.2, behind Dev's "Loops
    /// on audio tracks"): with it on, a save keeps each loop as the looper
    /// holds it (`mpfree_looper::Looper::loop_file_raw`, read with
    /// `raw_layer`), each layer with its recording's id, so it opens
    /// exactly and its edits can be taken further; off, each layer as it
    /// plays, as before. A project opens either way.
    pub fn set_recordings(&mut self, on: bool) {
        self.recordings = on;
    }

    /// With recordings on, each closed layer with no recording yet gets
    /// one, `fresh()` (the audio tracks' id maker). Each period; does not
    /// allocate.
    pub fn give_layer_recs(&mut self, fresh: impl FnMut() -> u128) {
        if self.recordings {
            self.layer_recs.give(&self.looper, fresh);
        }
    }

    /// With recordings on, closed layers go to their recordings' files
    /// soon after they close (phase 4c.3, docs/dev/design/loops-on-audio-tracks.md
    /// rule 4): up to [`SAVE_SPEED`] frames of them for each of the
    /// period's `frames`, through `files` (the audio tracks' worker), one
    /// layer at a time. Each period, after
    /// [`LoopHost::give_layer_recs`]; allocates only the log's line when
    /// a file couldn't be written (the layer stays in memory, and a save
    /// writes it with the project).
    pub fn write_layers(
        &mut self,
        files: &mut dyn crate::loops_on_tracks::LayerFiles,
        frames: usize,
    ) -> Option<String> {
        if !self.recordings {
            return None;
        }
        self.layer_writer.tend(
            &mut self.layer_recs,
            &self.looper,
            files,
            frames * SAVE_SPEED,
        );
        let why = self.layer_writer.take_failure()?;
        Some(format!(
            "looper: a layer's file couldn't be written ({why}); its audio stays in memory \
             and is saved with the project"
        ))
    }

    /// Layers of loop `n` whose audio is only in memory, not yet in a
    /// synced file (phase 4c.3); 0 with recordings off, where saves are
    /// what keep the audio.
    pub fn unsaved_layers(&self, n: usize) -> usize {
        if self.recordings {
            self.layer_recs.unsaved(&self.looper, n)
        } else {
            0
        }
    }

    /// Loops on audio tracks (phase 4b, behind Dev's switch): a loop with
    /// sound and no lasting id yet gets one, `fresh()` (the audio tracks'
    /// id maker), one a period at most; a first pass starting is a new
    /// loop, so its slot's old id goes first
    /// ([`crate::loops_on_tracks::OnTracks::follow_passes`]).
    pub fn give_loop_id(&mut self, fresh: impl FnOnce() -> u128) {
        let looper = &self.looper;
        self.on_tracks
            .follow_passes(|n| crate::loops_on_tracks::OnTracks::first_pass(looper, n));
        if let Some(n) = self.on_tracks.needs_id(&self.looper) {
            self.on_tracks.set(n, fresh());
        }
    }

    /// The slot holding the loop with lasting id `id` (audio tracks phase
    /// 4b), if one does.
    pub fn slot_of(&self, id: u128) -> Option<usize> {
        self.on_tracks.slot_of(mpfree_seq::LoopId(id))
    }

    /// Slot `n`'s loop takes lasting id `id` (0: none), as a copy of an
    /// audio track's loop cell gives the copy before its audio goes in.
    pub fn set_loop_id(&mut self, n: usize, id: u128) {
        self.on_tracks.set(n, id);
    }

    /// Loops on audio tracks, before [`LoopHost::run`]: each slot a loop
    /// cell names is told its runs from the track player (`loops`, planned
    /// on this period: `TrackHost::plan_loops`). Does not allocate.
    pub fn tell_tracks(
        &mut self,
        project: &mpfree_seq::Project,
        loops: &mpfree_audiotracks::LoopPlayer,
    ) {
        self.on_tracks.tell(&mut self.looper, project, loops);
    }

    /// Loops on audio tracks: every slot's loop's lasting id (0: none
    /// yet), for the time engine (`TimeHost::loops_are_on`).
    pub fn loop_ids(&self) -> [u128; MAX_LOOPS] {
        self.on_tracks.ids()
    }

    /// Loops on audio tracks: the loops heard on the Loop strip now, by
    /// their lasting ids (0 where none): playing or overdubbing, launched,
    /// not muted, on no track. One that goes on a track carries on there
    /// unfaded (`TrackHost::loops_moving`).
    pub fn moving_ids(&self) -> [u128; MAX_LOOPS] {
        let ids = self.on_tracks.ids();
        std::array::from_fn(|n| {
            let heard = self.looper.loop_status(n).is_some_and(|s| {
                matches!(s.phase, Phase::Playing | Phase::Overdubbing) && s.on && !s.muted
            });
            if heard && !self.looper.on_track(n) {
                ids[n]
            } else {
                0
            }
        })
    }

    /// Loops on audio tracks: a loop cell was just made (a loop landed in
    /// an audio track's cell), so its loop goes on its track at the next
    /// [`LoopHost::tell_tracks`], straight on from where it plays.
    pub fn look_again(&mut self) {
        self.on_tracks.look_again();
    }

    /// After [`LoopHost::run`]: loop `id`'s sound in the period, for its
    /// audio track (`TrackHost::period_with_loops`).
    pub fn track_loop(&self, id: u128) -> Option<&[f32]> {
        self.on_tracks.out(&self.looper, id)
    }

    /// One period without the mixer (the screen's samples): as
    /// [`LoopHost::run`], then the loop (and the line input, with Monitor
    /// on) added to the output, kept within ±1. Returns the output's peak,
    /// and a note for the log if the input had a problem.
    pub fn period(
        &mut self,
        block: &Block,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> (f32, Option<String>) {
        self.last_rolling = block
            .spans()
            .iter()
            .any(|s| s.state != mpfree_transport::State::Stopped);
        let note = self.record(block);
        let mut peak = 0f32;
        let mut over = false;
        let mon = if self.monitor { 1.0 } else { 0.0 };
        for f in 0..PERIOD {
            let l = left[f] + self.out[2 * f] + mon * self.input_l[f];
            let r = right[f] + self.out[2 * f + 1] + mon * self.input_r[f];
            over |= l.abs() > 1.0 || r.abs() > 1.0;
            left[f] = l.clamp(-1.0, 1.0);
            right[f] = r.clamp(-1.0, 1.0);
            peak = peak.max(left[f].abs()).max(right[f].abs());
        }
        if over {
            self.clipped += 1;
        }
        (peak, note)
    }

    /// Reads the line input and runs the looper into `self.out`.
    fn record(&mut self, block: &Block) -> Option<String> {
        self.queue_steps();
        let note = self.line.period(&mut self.input_l, &mut self.input_r);
        // An audio track's line pick is the line input as its take has
        // it, in the pads' buffers (they are not heard from then).
        if let LoopSource::Track(TrackInput {
            from: TrackFrom::Line(pick),
            ..
        }) = self.source
        {
            let line = (&self.input_l, &self.input_r);
            pick_line(pick, line, &mut self.pads_l, &mut self.pads_r);
        }
        let (src_l, src_r) = match self.source {
            LoopSource::Line => (&self.input_l, &self.input_r),
            _ => (&self.pads_l, &self.pads_r),
        };
        for f in 0..PERIOD {
            self.input[2 * f] = src_l[f];
            self.input[2 * f + 1] = src_r[f];
        }
        for (m, x) in self.meter.iter_mut().zip([src_l, src_r]) {
            *m = x.iter().fold(*m, |a, v| a.max(v.abs()));
        }
        let chunks = self.heard_peaks.len() as u64;
        if chunks > 0 {
            for f in 0..PERIOD {
                let at = block.start_frame + f as u64;
                let p = &mut self.heard_peaks[((at / HEARD_CHUNK) % chunks) as usize];
                if at.is_multiple_of(HEARD_CHUNK) {
                    *p = 0.0;
                }
                *p = p.max(src_l[f].abs()).max(src_r[f].abs());
            }
        }
        // A period nobody handed the pads' sound to records silence.
        self.pads_l = [0.0; PERIOD];
        self.pads_r = [0.0; PERIOD];
        // The pads' latency follows the mix (a plugin's latency, or dry).
        if self.source.inside()
            && self.latency_set != self.pads_latency
            && self.looper.command(Command::SetLatency(self.pads_latency))
        {
            self.latency_set = self.pads_latency;
        }
        // A loop that starts when you play listens to the line input; the
        // pads' sound has the song in it, so a pad hit is told instead.
        let by_ear = self.source.line();
        if by_ear != self.by_ear_set && self.looper.command(Command::StartsByEar(by_ear)) {
            self.by_ear_set = by_ear;
        }
        // Smart loop end listens for the hits in the line input and a mix
        // (the notes are the pads', and a mix may hold more than them).
        let by_ear = self.source != LoopSource::Pads;
        if by_ear != self.by_ear_smart && self.looper.command(Command::SmartByEar(by_ear)) {
            self.by_ear_smart = by_ear;
        }
        if let Some(span) = block.spans().first() {
            self.beats_per_bar = u32::from(span.meter.beats);
            self.bar_ticks = span.meter.bar_ticks();
            self.bpm = span.bpm;
        }
        // Each loop's sound apart (audio tracks phase 4a), added up where
        // the Loop strip takes the mix, so nothing sounds different: the
        // sum is what `process` would have added, bit for bit. Phase 4b
        // gives each loop its own track's channel.
        self.out.fill(0.0);
        self.looper.process_each(block, &self.input);
        self.looper.outs().mix_into(&mut self.out);
        self.settle = self.settle.saturating_sub(1);
        if let Some(span) = block.spans().last() {
            // A free first pass that closed in this period sets the song's
            // tempo from the next one, so the printer sees the loops at
            // that tempo and asks for no print they won't need.
            let bpm = self.looper.tempo_coming().unwrap_or(span.bpm);
            self.printer.tend(&mut self.looper, bpm);
        }
        self.finder.tend(&mut self.looper);
        self.tend_save();
        self.tend_copy();
        note
    }

    /// Lines for the log when the looper's phase or layers changed since
    /// the last call; `at` gives the song position as text. Nothing is
    /// allocated when nothing changed.
    pub fn changes(&mut self, at: impl FnOnce() -> String) -> Vec<String> {
        let s = self.looper.status();
        let was = self.last;
        self.last = s;
        let mut lines = Vec::new();
        if s.phase == was.phase && s.layers == was.layers && s.redo == was.redo {
            return lines;
        }
        let at = at();
        if s.phase != was.phase {
            lines.push(match s.phase {
                Phase::Empty if was.phase == Phase::Recording => format!("looper: first pass thrown away at {at} (STOP \
                                                                           before a 1/16 note, or the transport jumped)"),
                Phase::Empty => format!("looper: empty at {at}"),
                Phase::Armed => format!("looper: armed at {at}"),
                Phase::Recording => format!("looper: recording the first pass from {at} ({})", bars_text(s.bars)),
                Phase::Playing if was.phase == Phase::Recording => {
                    let kept = match s.cut {
                        Some(c) => format!(", cut at STOP after {:.2} s (silent to the bar line)", c as f64 / 44_100.0),
                        None => String::new(),
                    };
                    let how = if s.asks.is_some() { "waiting on its Length" } else { "playing" };
                    format!("looper: loop of {} at {at}: {} frames ({:.2} s), {how}{kept}", ticks_text(s.ticks, self.bar_ticks),
                            s.frames, s.frames as f64 / 44_100.0)
                }
                Phase::Playing => format!("looper: playing at {at}, {} layers", s.layers),
                Phase::Overdubbing => format!("looper: overdubbing layer {} from {at}", s.layers),
            });
        } else if s.layers != was.layers || s.redo != was.redo {
            lines.push(format!(
                "looper: {} layers, {} to redo, at {at}",
                s.layers, s.redo
            ));
        }
        lines
    }

    /// Whether a loop is armed or records its first pass, and whether one
    /// overdubs: the REC and OVERDUB lights ([`crate::app::App::transport_lights`]).
    pub fn recording(&self) -> (bool, bool) {
        let (mut rec, mut dub) = (false, false);
        for s in (0..MAX_LOOPS).filter_map(|n| self.looper.loop_status(n)) {
            match s.phase {
                Phase::Armed | Phase::Recording => rec = true,
                Phase::Overdubbing => dub = true,
                Phase::Empty | Phase::Playing => {}
            }
        }
        (rec, dub)
    }

    /// For the arm ([`crate::rec_arm::Now`]): whether a loop records its
    /// first pass or overdubs, and whether the picked loop is armed (its
    /// first pass waits for PLAY or the bar line).
    pub fn rec_now(&self) -> (bool, bool) {
        let taking = (0..MAX_LOOPS)
            .filter_map(|n| self.looper.loop_status(n))
            .any(|s| matches!(s.phase, Phase::Recording | Phase::Overdubbing));
        (taking, self.looper.status().phase == Phase::Armed)
    }

    /// The loop a new one records into, for a Loop cell armed on the
    /// grid: the picked loop if it is empty, else the first empty one; or
    /// why there is none, as REC says it. Less than a second of memory
    /// left is full: a first pass with no memory for it is thrown away.
    pub fn free_loop(&self) -> Result<usize, String> {
        let copy = self.copy.map(|c| c.to);
        let empty = |n: usize| {
            copy != Some(n)
                && self.looper.loop_status(n).is_some_and(|s| {
                    s.phase == Phase::Empty
                        && s.copying.is_none()
                        && s.loading.is_none()
                        && s.capturing.is_none()
                })
        };
        let n = if empty(self.picked) {
            self.picked
        } else {
            (0..MAX_LOOPS).find(|&n| empty(n)).ok_or_else(|| {
                format!("{MAX_LOOPS} loops in use: clear one on the Looper tab to record another")
            })?
        };
        let m = self.looper.memory();
        if m.capacity.saturating_sub(m.used) < self.rate as usize {
            return Err(
                "The loops' memory is full: clear a loop on the Looper tab to record another"
                    .to_string(),
            );
        }
        Ok(n)
    }

    /// Copy loop `from` into `to`, an empty loop: every layer, with its
    /// mix, and the loop's level (loop-tracks.md part 6, a loop cell
    /// copied onto another Loop track). The copy is launched off, so it
    /// stays silent until its cell is launched. The layers copy in one at
    /// a time in the background while the loops play on
    /// ([`Looper::copy_layer`], each a step the looper's UNDO takes
    /// back); [`LoopHost::copy_ended`] says when it is all in or stopped.
    /// Refused, saying why, while another copy runs, when `from` isn't
    /// just playing, `to` isn't empty or the memory can't hold the
    /// layers.
    pub fn copy_loop(&mut self, from: usize, to: usize) -> Result<String, String> {
        if let Some(c) = self.copy {
            return Err(format!(
                "Loop {} is still copying into Loop {}: wait for it to finish",
                c.from + 1,
                c.to + 1
            ));
        }
        let (Some(a), Some(b)) = (self.looper.loop_status(from), self.looper.loop_status(to))
        else {
            return Err("No such loop".to_string());
        };
        let still = a.merging.is_none()
            && a.fitting.is_none()
            && a.loading.is_none()
            && a.copying.is_none();
        if from == to || a.phase != Phase::Playing || a.layers == 0 || !still {
            return Err(format!(
                "Loop {} can't be copied while it records or changes: try again when it just plays",
                from + 1
            ));
        }
        if b.phase != Phase::Empty || b.loading.is_some() || b.copying.is_some() {
            return Err(format!("Loop {} isn't empty", to + 1));
        }
        let m = self.looper.memory();
        let need = a.layers * a.frames + self.rate as usize;
        if m.capacity.saturating_sub(m.used) < need {
            return Err(format!(
                "The loops' memory can't hold a copy of Loop {} ({} layer{}): clear a loop on the Looper tab first",
                from + 1,
                a.layers,
                if a.layers == 1 { "" } else { "s" }
            ));
        }
        let mut ids = [0u8; MAX_LAYERS];
        let mut layers = 0;
        for (k, id) in ids.iter_mut().enumerate().take(a.layers) {
            if let Some(l) = self.looper.loop_layer_info(from, k) {
                *id = l.id;
                layers = k + 1;
            }
        }
        if layers == 0 || self.looper.room() < 4 {
            return Err(format!("Loop {} can't be copied now: try again", from + 1));
        }
        // Off until its cell is launched, at the source's level.
        self.looper.launch(to, false, None);
        self.looper.select_loop(to);
        self.looper.command(Command::SetLevel(a.level));
        self.looper.select_loop(self.picked);
        self.copy = Some(LoopCopy {
            from,
            to,
            ids,
            layers,
            asked: 0,
            waited: 0,
            tries: 0,
        });
        self.copy_end = None;
        Ok(format!(
            "looper: loop {} copies into loop {}, {} layer{}",
            from + 1,
            to + 1,
            layers,
            if layers == 1 { "" } else { "s" }
        ))
    }

    /// The end of the last whole loop's copy, once: all its layers in, or
    /// as many as went in before it stopped (none: the copy's loop is
    /// empty, so its cells should go).
    pub fn copy_ended(&mut self) -> Option<CopyEnd> {
        self.copy_end.take()
    }

    /// One period of a whole loop's copy: once the last layer asked for
    /// is in, the next is asked for. A layer that doesn't start (the
    /// loops were busy or fading) is asked for again, for up to two
    /// seconds; the copy stops then, or when its loop is cleared or
    /// undone, or its source no longer plays. Allocates nothing.
    fn tend_copy(&mut self) {
        let Some(mut c) = self.copy else {
            return;
        };
        let (Some(a), Some(b)) = (
            self.looper.loop_status(c.from),
            self.looper.loop_status(c.to),
        ) else {
            self.copy = None;
            return;
        };
        let have = if b.phase == Phase::Empty { 0 } else { b.layers };
        let ended = CopyEnd {
            from: c.from,
            to: c.to,
            layers: have,
            of: c.layers,
        };
        if b.copying.is_some() {
            c.waited = 0;
            self.copy = Some(c);
            return;
        }
        // All in; or cleared or undone under it: it stops with what it has.
        if have == c.layers || have + 1 < c.asked {
            self.copy = None;
            self.copy_end = Some(ended);
            return;
        }
        if c.asked > have {
            // Asked, but not started: it starts with the next period, or
            // it was refused.
            c.waited += 1;
            if c.waited <= 2 {
                self.copy = Some(c);
                return;
            }
            c.asked = have;
            c.tries += 1;
        }
        let limit = 2 * self.rate / PERIOD as u32;
        let gone = a.phase != Phase::Playing || (b.phase != Phase::Empty && b.ticks != a.ticks);
        if gone || c.tries > limit {
            self.copy = None;
            self.copy_end = Some(ended);
            return;
        }
        if self.looper.room() > 0 && self.looper.copy_layer(c.from, c.ids[have], c.to, true) {
            c.asked = have + 1;
            c.waited = 0;
        }
        self.copy = Some(c);
    }

    /// What the picked loop is doing (armed, recording, overdubbing...).
    pub fn phase(&self) -> Phase {
        self.looper.status().phase
    }

    /// Loop `n` is a free loop with sound, stopped with the free clock
    /// (as [`LooperView::free_stopped`]): its ring or key plays the free
    /// loops.
    pub fn free_stopped(&self, n: usize) -> bool {
        self.looper.loop_status(n).is_some_and(|l| {
            l.clock == Clock::Free
                && l.phase == Phase::Playing
                && !l.muted
                && self.looper.free_clock().is_some_and(|c| !c.running)
        })
    }

    /// Loop `n` as the looper has it (the loop keys' lights and steps).
    pub fn loop_status(&self, n: usize) -> Option<Status> {
        self.looper.loop_status(n)
    }

    /// The loop picked (0 is Loop 1): the buttons and a learned control's
    /// picked loop act on it.
    pub fn picked(&self) -> usize {
        self.picked
    }

    /// Sync as last set, for every empty loop ([`Clock::Song`] at first).
    pub fn sync(&self) -> Clock {
        self.clock
    }

    /// The loops muted, bit `n` for loop `n`, as MIDI Learn reads them.
    pub fn mute_bits(&self) -> u8 {
        (0..MAX_LOOPS.min(8)).fold(0, |bits, n| {
            let muted = self
                .looper
                .loop_status(n)
                .is_some_and(|s| s.muted && s.phase != Phase::Empty);
            bits | (u8::from(muted) << n)
        })
    }

    /// The `status: looper: ...` line.
    pub fn describe(&self) -> String {
        let s = self.looper.status();
        let st = self.looper.stats();
        format!("looper: {:?}, {} layers ({} to redo), loop {}, first pass length {}, latency {} frames, level {:.1} \
                 dB{}; records from {}; line input {}{}, monitor {}; {} overdubs refused, {} first passes thrown \
                 away, {} commands dropped, {} periods kept within ±1; {}prints {} made, {} too late, {} failed",
                s.phase, s.layers, s.redo,
                if s.frames > 0 { format!("{} frames ({:.2} s)", s.frames, s.frames as f64 / 44_100.0) } else { "none".to_string() },
                bars_text(s.bars), s.latency, 20.0 * s.level.max(1e-6).log10(), if s.muted { ", muted" } else { "" },
                self.source.name().to_lowercase(), if self.line.is_open() { "on" } else { "off" },
                if self.line.is_open() || self.line.stats.periods > 0 { format!(" ({})", self.line.describe()) } else { String::new() },
                if self.monitor { "on" } else { "off" }, st.refused, st.aborted, st.dropped_commands, self.clipped,
                if s.printed { "a print plays (pitch kept at this tempo); " } else { "" },
                self.printer.made, self.printer.stale, self.printer.failed)
            + &if self.waves.waits > 0 {
                format!("; {} waveforms waited a period for the screen", self.waves.waits)
            } else {
                String::new()
            }
            + &if self.starts_refused > 0 {
                format!("; {} starts on a note refused", self.starts_refused)
            } else {
                String::new()
            }
            + &if self.tempos_refused > 0 {
                format!("; {} tempos from a loop refused", self.tempos_refused)
            } else {
                String::new()
            }
            + &if st.taps_kept + st.ends_moved + st.ends_offered > 0 {
                format!(
                    "; smart loop end: {} ends moved, {} offered, {} taps kept, {} starts on \
                     the first hit, {} overdubs begun with what was played after the end",
                    st.ends_moved, st.ends_offered, st.taps_kept, st.starts_found, st.handoffs
                )
            } else {
                String::new()
            }
            + &if st.hits_heard + st.finds_asked > 0 || self.finder.gone {
                format!(
                    "; by ear: {} hits heard, {} decisions asked of the finder, {} answered in \
                     time, {} too late (the tap kept), {} answers dropped (late, or for a take \
                     gone), {} periods an ask waited for the finder{}",
                    st.hits_heard,
                    st.finds_asked,
                    st.finds_answered,
                    st.finds_late,
                    st.finds_stale,
                    self.finder.waited,
                    if self.finder.gone {
                        ", its thread is gone (taps are kept)"
                    } else {
                        ""
                    }
                )
            } else {
                String::new()
            }
            + &if self.snaps_refused > 0 {
                format!("; {} snaps refused or not landed", self.snaps_refused)
            } else {
                String::new()
            }
            + &{
                // Layers as recordings (phase 4c.3): layers only in memory
                // so far, and files that couldn't be written.
                let only = (0..MAX_LOOPS).map(|n| self.unsaved_layers(n)).sum::<usize>();
                let failed = self.layer_writer.failed;
                match (only, failed) {
                    (0, 0) => String::new(),
                    (only, 0) => format!("; {only} layers only in memory"),
                    (only, failed) => format!(
                        "; {only} layers only in memory, {failed} layers' files couldn't be written"
                    ),
                }
            }
            + &if self.notes_lost > 0 {
                format!(
                    "; {} periods with more notes than smart loop end could follow",
                    self.notes_lost
                )
            } else {
                String::new()
            }
            + &if self.readings_overruled > 0 {
                format!(
                    "; {} loops too long for their Length's bars (read instead)",
                    self.readings_overruled
                )
            } else {
                String::new()
            }
            + &if self.dropouts > 0 {
                format!("; {} times a take's source was lost while it recorded (silence)", self.dropouts)
            } else {
                String::new()
            }
            + &if self.chosen != self.source {
                format!("; next take from {}", self.chosen.name().to_lowercase())
            } else {
                String::new()
            }
    }

    /// What the Looper tab shows; takes the meter's peaks.
    pub fn view(&mut self, bpm: f64) -> LooperView {
        let m = std::mem::take(&mut self.meter);
        let db = |v: f32| {
            if v > 0.0 {
                20.0 * v.log10()
            } else {
                f32::NEG_INFINITY
            }
        };
        let s = self.looper.status();
        // A beat of the meter (a quarter in 4/4, an eighth in 6/8) at the
        // tempo the loop was recorded at, from its length; the tempo counts
        // quarters.
        let beat = self.bar_ticks.max(1) as f64 / f64::from(self.beats_per_bar.max(1));
        let frames_per_beat = if s.ticks > 0 {
            s.frames as f64 * beat / s.ticks as f64
        } else {
            f64::from(self.rate) * 60.0 / bpm.max(1.0) * beat / f64::from(PPQ)
        };
        let mut layers = [None; MAX_LAYERS];
        for (k, l) in layers.iter_mut().enumerate() {
            *l = self.looper.layer_info(k);
        }
        let mut loops = [s; MAX_LOOPS];
        for (n, l) in loops.iter_mut().enumerate() {
            if let Some(st) = self.looper.loop_status(n) {
                *l = st;
            }
        }
        let smart = self.smart_view(&s);
        LooperView {
            smart,
            status: s,
            stats: self.looper.stats(),
            source: self.chosen,
            taking_from: (self.source != self.chosen).then_some(self.source),
            resample: self.resample,
            line_on: self.line.is_open(),
            line: self.line.stats,
            monitor: self.monitor,
            meter_db: [db(m[0]), db(m[1])],
            beats_per_bar: self.beats_per_bar,
            frames_per_beat,
            bar_ticks: self.bar_ticks,
            layers,
            loops,
            selected: self.looper.selected(),
            memory: self.looper.memory(),
            rate: self.rate,
            history: self.looper.history(),
            cell_waves: self.cells.done,
            ring_waves: self.cells.rings,
            copy_into: self.copy.map(|c| c.to),
            heard: self.heard_wave(),
            capture_into: self.capture_into(),
            free_tempo: self.free_tempo,
            match_pattern: self.match_pattern,
            pattern_bars: self.pattern_bars,
            listening: self.looper.listening(),
            clock: self.clock,
            free: self.looper.free_clock(),
            free_now: self.looper.free_now().unwrap_or(0.0),
            free_cued: self.looper.free_cued(),
            ids: self.on_tracks.ids(),
            unsaved: std::array::from_fn(|n| self.unsaved_layers(n) > 0),
        }
    }

    /// Where Capture goes now: the picked loop if it is empty (or
    /// capturing: then Capture calls it off), else the first empty loop;
    /// and the bars that loop's Length asks for (Free:
    /// [`CAPTURE_FREE_BARS`]).
    fn capture_into(&self) -> Option<(usize, u32)> {
        let picked = self.looper.loop_status(self.picked)?;
        let n = if picked.capturing.is_some() {
            self.picked
        } else {
            self.free_loop().ok()?
        };
        let bars = self.looper.loop_status(n)?.bars;
        let bars = self
            .span()
            .map_or(bars.unwrap_or(CAPTURE_FREE_BARS), |s| s.1);
        Some((n, bars))
    }

    /// The bars What you played's window grows and shrinks by, for loop
    /// `n`, and whether they are the pattern's: the pattern's length while
    /// Length is Pattern and a pattern plays, else the Length's bars,
    /// else 1.
    fn capture_unit(&self, n: usize) -> (u32, bool) {
        match (self.match_pattern, self.pattern_bars) {
            (true, Some(b)) => (b.max(1), true),
            _ => (
                self.looper
                    .loop_status(n)
                    .and_then(|s| s.bars)
                    .unwrap_or(1)
                    .max(1),
                false,
            ),
        }
    }

    /// What you played's window as it stands: one moved back whose bar
    /// line is no longer kept (the song stopped or jumped, or it fell out
    /// of the 60 s) is let go of.
    fn span(&self) -> Option<(Option<Tick>, u32)> {
        let (end, bars) = self.capture_span?;
        match end {
            Some(t) => self
                .looper
                .capture_take_to(self.clock, t, 1)
                .map(|_| (end, bars)),
            None => Some((None, bars)),
        }
    }

    /// What Capture would take now, `bars` of it: up to the window's bar
    /// line when it was moved back, else up to the newest. On the clock
    /// the loop it goes into takes (Sync as last set): a free loop's are
    /// the free clock's bars while it is set.
    fn capture_take_now(&self, bars: u32) -> Option<Take> {
        match self.span().and_then(|s| s.0) {
            Some(end) => self.looper.capture_take_to(self.clock, end, bars),
            None => self.looper.capture_take(self.clock, bars),
        }
    }

    /// A Capture now would wait for the bar line coming: the window keeps
    /// up with the song (or the free loops), which is in a bar's last
    /// beat.
    fn capture_waits_now(&self) -> bool {
        self.span().and_then(|s| s.0).is_none() && self.looper.capture_waits(self.clock)
    }

    /// The bars a Capture now counts in, for What you played and the log:
    /// for a free loop (Sync: Free) while the free clock is set, the free
    /// clock's, numbered from its bar 1 (where the free loops start);
    /// else the song's. Their tempo, a bar's ticks and bar 1's tick.
    fn capture_grid(&self) -> (f64, Tick, Tick) {
        match (self.clock, self.looper.free_clock()) {
            (Clock::Free, Some(c)) => (c.bpm, c.meter.bar_ticks(), c.top),
            _ => (self.bpm, self.bar_ticks, 0),
        }
    }

    /// The bar a Capture's tick `t` starts, numbered in its bars
    /// ([`LoopHost::capture_grid`]).
    fn capture_bar(&self, t: Tick) -> i64 {
        let (_, bar, top) = self.capture_grid();
        (t - top).div_euclid(bar.max(1)) + 1
    }

    /// What you played's window moved or resized on the Looper tab:
    /// Capture takes `bars` ending on the bar line that starts bar `end`;
    /// None, or the newest bar line, keeps up with the song. Until a
    /// Capture, a new Length, or its bar line is no longer kept. A line
    /// for the log.
    pub fn set_capture_span(&mut self, end: Option<i64>, bars: u32) -> String {
        let bars = bars.clamp(1, HEARD_MAX_BARS);
        let mut lines = [(0 as Tick, 0u64); HEARD_MAX_BARS as usize + 1];
        let n = self.looper.heard_lines(self.clock, &mut lines);
        let number = |t: Tick| self.capture_bar(t);
        let newest = n.checked_sub(1).map(|i| number(lines[i].0));
        let pinned = match end {
            Some(e) if Some(e) != newest => match lines[..n].iter().find(|l| number(l.0) == e) {
                Some(l) => Some((e, l.0)),
                None => {
                    return format!(
                        "looper: Capture window: bar {e}'s line isn't kept: it stays as it was"
                    )
                }
            },
            _ => None,
        };
        self.capture_span = Some((pinned.map(|p| p.1), bars));
        match pinned {
            Some((e, _)) => format!(
                "looper: Capture window: {}",
                bars_words(e - i64::from(bars), bars)
            ),
            None => format!("looper: Capture window: the last {}", bars_words_n(bars)),
        }
    }

    /// What the looper heard over the last [`HEARD_BARS`] bars at the
    /// tempo now, with its bar lines and what Capture would take.
    fn heard_wave(&self) -> HeardWave {
        let mut w = HeardWave::default();
        let chunks = self.heard_peaks.len() as u64;
        if chunks == 0 {
            return w;
        }
        let now = self.looper.heard_at();
        let (bpm, bar_ticks, _) = self.capture_grid();
        let bar =
            bar_ticks.max(1) as f64 / f64::from(PPQ) * 60.0 / bpm.max(1.0) * f64::from(self.rate);
        // Room for the window to grow to four times its unit (the
        // pattern), and a bar more than it takes.
        let into = self.capture_into();
        if let Some((n, _)) = into {
            (w.unit, w.pattern) = self.capture_unit(n);
        }
        let shown = HEARD_BARS
            .max(4 * w.unit)
            .max(into.map_or(0, |i| i.1 + 1))
            .min(HEARD_MAX_BARS);
        let span = (bar * f64::from(shown)).min((chunks - 1) as f64 * HEARD_CHUNK as f64);
        let from = now as f64 - span;
        let x = |f: u64| ((f as f64 - from) / span) as f32;
        for (c, col) in w.cols.iter_mut().enumerate() {
            let a = from + span * c as f64 / HEARD_COLS as f64;
            let b = from + span * (c + 1) as f64 / HEARD_COLS as f64;
            if b <= 0.0 {
                continue;
            }
            let (a, b) = (
                a.max(0.0) as u64 / HEARD_CHUNK,
                (b as u64).div_ceil(HEARD_CHUNK),
            );
            let p = (a..b.max(a + 1))
                .map(|k| self.heard_peaks[(k % chunks) as usize])
                .fold(0.0f32, f32::max);
            *col = if p > 0.0 {
                20.0 * p.log10()
            } else {
                f32::NEG_INFINITY
            };
        }
        let mut lines = [(0 as Tick, 0u64); HEARD_MAX_BARS as usize + 1];
        let n = self.looper.heard_lines(self.clock, &mut lines);
        let number = |t: Tick| self.capture_bar(t);
        for &(t, f) in &lines[..n] {
            if f as f64 >= from && w.n_lines < w.lines.len() {
                w.lines[w.n_lines] = (number(t), x(f));
                w.n_lines += 1;
            }
        }
        w.waits = self.capture_waits_now();
        if let Some((_, bars)) = into {
            let latency = u64::from(self.looper.status().latency);
            w.take = self.capture_take_now(bars).map(|t| {
                let a = t.start.saturating_sub(latency);
                let (a, b) = (x(a), x(a + t.frames as u64));
                if w.waits {
                    // It takes the bars up to the bar line coming: one on.
                    let shift = b - a;
                    let shift = shift / t.bars as f32;
                    (a + shift, 1.0, number(t.origin) + 1, t.bars)
                } else {
                    (a, b, number(t.origin), t.bars)
                }
            });
        }
        w
    }

    /// Capture ([`Command::Capture`]): the last bars heard become a loop,
    /// the picked one if it is empty, else the first empty one; its
    /// Length gives the bars (Free: [`CAPTURE_FREE_BARS`]). Again while it
    /// waits for its bar line or copies in, it is called off. A line for
    /// the log, saying which bars, or why not.
    pub fn capture(&mut self) -> String {
        let picked = self.looper.loop_status(self.picked);
        if picked.is_some_and(|s| s.capturing.is_some()) {
            if !self.looper.command(Command::Capture(1)) {
                return "looper: Capture: NOT called off, the command queue is full".to_string();
            }
            return format!("looper: Capture into Loop {} called off", self.picked + 1);
        }
        let n = match self.free_loop() {
            Ok(n) => n,
            Err(why) => return format!("looper: Capture: {why}"),
        };
        let span = self.span();
        let asked = span.map(|s| s.1).unwrap_or_else(|| {
            self.looper
                .loop_status(n)
                .and_then(|s| s.bars)
                .unwrap_or(CAPTURE_FREE_BARS)
        });
        let pinned = span.and_then(|s| s.0);
        let waits = self.capture_waits_now();
        let take = self.capture_take_now(asked);
        // A free loop takes the free clock's bars while it is set.
        let free = self.clock == Clock::Free;
        let on_free = free && self.looper.free_clock().is_some();
        let Some(t) = take.or(waits.then_some(Take {
            bars: asked,
            start: 0,
            frames: 0,
            ticks: 0,
            origin: 0,
        })) else {
            if on_free {
                return "looper: Capture: no whole bar of the free loops heard yet: it takes the \
                        bars they have played since they last started (and since Records from \
                        changed)"
                    .to_string();
            }
            return "looper: Capture: no whole bar heard yet: it takes the bars the song has \
                    played since PLAY (and since Records from changed)"
                .to_string();
        };
        let m = self.looper.memory();
        if t.frames > m.capacity.saturating_sub(m.used) {
            return format!(
                "looper: Capture: the loops' memory can't hold {} ({:.1} s): clear a loop, or \
                 set a bigger Loop memory in Prefs",
                bars_words_n(t.bars),
                t.frames as f64 / f64::from(self.rate)
            );
        }
        if n != self.picked {
            if !self.looper.select_loop(n) {
                return "looper: Capture: NOT started, the command queue is full".to_string();
            }
            self.picked = n;
            self.for_cell = false;
        }
        // It records as a loop recorded now would: Sync as last set (the
        // bars above were counted on that clock), and Starts.
        self.sticky_settings(n, self.clock);
        let c = match pinned {
            Some(end) => Command::CaptureTo(end, asked),
            None => Command::Capture(asked),
        };
        if !self.looper.command(c) {
            return "looper: Capture: NOT started, the command queue is full".to_string();
        }
        // The window goes back to the Length's bars, keeping up.
        self.capture_span = None;
        let sync = if free { ", free" } else { "" };
        if take.is_none() || waits {
            return format!(
                "looper: Capture: on the bar line coming, the last {} become Loop {}{sync}",
                bars_words_n(asked),
                n + 1
            );
        }
        let since = if on_free {
            "since the free loops started"
        } else {
            "since PLAY"
        };
        let fewer = match (t.bars < asked, span.is_some()) {
            (false, _) => String::new(),
            (true, false) => format!(" (its Length asks {asked}: fewer were played {since})"),
            (true, true) => format!(" (the window asks {asked}: fewer were played or kept)"),
        };
        format!(
            "looper: Capture: {} become Loop {}{sync}{fewer}",
            bars_words(self.capture_bar(t.origin), t.bars),
            n + 1
        )
    }

    /// What starts a loop set to start when you play, for the log and
    /// the screen: "(the first pad you play)".
    pub fn first_note(&self) -> &'static str {
        match self.source.line() {
            false => "(the first pad you play)",
            true => "(the line input's first note over -40 dB)",
        }
    }

    /// Starts: When I play, with the pads: the loop listening for a pad
    /// hit, if one is (the pads are what it records, and no hit was told
    /// on `frame` yet).
    pub fn listens_for_pads(&self, frame: u64) -> Option<usize> {
        if !self.source.inside() || self.resample || self.played_at == Some(frame) {
            return None;
        }
        (0..MAX_LOOPS).find(|&n| self.looper.loop_status(n).is_some_and(|s| s.listening))
    }

    /// A pad hit on song frame `frame` (the coming period's first: the
    /// pads are read before the transport runs): a loop listening for one
    /// starts on it ([`LoopHost::listens_for_pads`]).
    pub fn played(&mut self, frame: u64) -> bool {
        self.played_at = Some(frame);
        self.looper.command(Command::Played(frame))
    }

    /// After the looper's period: a loop that heard you play with the song
    /// stopped asks for it to start, on the next period's first frame, so
    /// bar 1 is your note. Some(loop) once a note.
    pub fn take_start(&mut self) -> Option<usize> {
        self.looper.take_start()
    }

    /// The song can't start on loop `n`'s note (`why`): the loop forgets
    /// it and listens again. Counted; the line for the log and the screen.
    pub fn start_refused(&mut self, n: usize, why: &str) -> String {
        self.looper.start_refused();
        self.note_refused(n, why)
    }

    /// A note that would start loop `n` and the song, which can't start
    /// (`why`): counted; the line for the log and the screen. The loop
    /// still listens.
    pub fn note_refused(&mut self, n: usize, why: &str) -> String {
        self.starts_refused += 1;
        format!(
            "looper: Loop {} heard you, but the song can't start: {why}. It listens again",
            n + 1
        )
    }

    /// Tempo from the first loop on or off ([`Command::TempoFromLoop`]):
    /// the MENU setting, while the song can take a tempo from a loop.
    /// Cheap to call every period: the looper is told when it changes.
    pub fn set_tempo_from_loop(&mut self, on: bool) {
        if on != self.free_tempo && self.looper.command(Command::TempoFromLoop(on)) {
            self.free_tempo = on;
        }
    }

    /// The song's tempo was set by hand since the project opened
    /// ([`Command::TempoMeant`]): a free first loop's reading counts it
    /// strongly, a tempo left from before only lightly. Cheap to call
    /// every period: the looper is told when it changes.
    pub fn set_tempo_meant(&mut self, on: bool) {
        if on != self.tempo_meant && self.looper.command(Command::TempoMeant(on)) {
            self.tempo_meant = on;
        }
    }

    /// Loop `n` listening for a pad records free when it hears one
    /// ([`Status::free`]): the note doesn't start the song; closing the
    /// pass does (a song loop's), or sets the free clock (a free loop's).
    pub fn records_free(&self, n: usize) -> bool {
        self.looper.loop_status(n).is_some_and(|s| s.free)
    }

    /// Loop `n` keeps time with the free clock (Sync: Free,
    /// [`Clock::Free`]): a note that starts it never starts the song.
    pub fn on_free_clock(&self, n: usize) -> bool {
        self.looper
            .loop_status(n)
            .is_some_and(|s| s.clock == Clock::Free)
    }

    /// After the looper's period: a free first pass closed, so the song
    /// starts at the tempo its length sets, on the frame it comes round
    /// ([`Looper::take_tempo`]): its loop, how its length read (its bars
    /// and tempo, how sure) and that frame. A Length the reading couldn't
    /// keep is counted ([`LoopHost::readings_overruled`]).
    pub fn take_tempo(&mut self) -> Option<(usize, Reading, u64, Tick)> {
        let (n, r, at, into) = self.looper.take_tempo()?;
        if r.overruled() {
            self.readings_overruled += 1;
        }
        let version = self.looper.loop_status(n).map_or(0, |s| s.version);
        self.close_call = match (r.sure, r.other) {
            (Sure::Ambiguous, Some((bars, _))) => Some((n, version, bars)),
            _ => None,
        };
        Some((n, r, at, into))
    }

    /// Smart loop end (the MENU setting, [`Command::SetSmartEnd`]): it
    /// follows the pads and MIDI notes, and listens for the hits in the
    /// line input and a mix (step 3, [`Command::SmartByEar`], told each
    /// period with the source). Cheap to call every period: the looper is
    /// told when it changes.
    pub fn set_smart_end(&mut self, m: SmartEnd) {
        if m != self.smart && self.looper.command(Command::SetSmartEnd(m)) {
            self.smart = m;
        }
    }

    /// A note started on `track` (a pad hit or a MIDI note-on), on the
    /// song's frame clock: smart loop end weighs it. Every note of a
    /// period, before [`LoopHost::run`].
    pub fn note(&mut self, frame: u64, track: u16, key: u8, velocity: u8) {
        self.looper.note(NoteStart {
            frame,
            source: track,
            key,
            velocity,
        });
    }

    /// Notes started, the newest on `at`, that the time engine had no
    /// room to pass on: counted ([`LoopHost::notes_lost`]), and smart
    /// loop end keeps a tap whose reach they fall in.
    pub fn notes_lost(&mut self, at: u64) {
        self.notes_lost += 1;
        self.looper.notes_lost(at);
    }

    /// Smart loop end's Snap ([`Snap::Found`]), Use my tap ([`Snap::Tap`]),
    /// Use my press, Read as ([`Snap::Bars`]), or UNDO or REDO of one, on
    /// the picked loop: the loop and the song's tempo change together on
    /// its next downbeat. Worked out and asked of the time engine at the
    /// next period ([`LoopHost::tend_snap`]); what it did, or why not,
    /// comes after. The line for the log now.
    pub fn snap(&mut self, s: Snap) -> String {
        self.snap_on(self.looper.selected(), s)
    }

    /// [`LoopHost::snap`] on loop `n`, picked or not.
    fn snap_on(&mut self, n: usize, s: Snap) -> String {
        self.snap_ask = None;
        self.ab_stop();
        self.side_want = None;
        self.snap_want = Some((n, s, false));
        format!(
            "looper: {}: Loop {} changes on its next downbeat, with the tempo",
            snap_name(s, self.reads_again(s)),
            n + 1
        )
    }

    /// The loop point view's Snap and Use my tap (or press), Keep my tap
    /// and Keep the snap: the picked loop goes to that side of smart loop
    /// end's choice on its next downbeat, with the tempo, by the snap that
    /// gets there (worked out once no other is under way,
    /// [`LoopHost::tend_snap`]); already there, it stays. `keep`: the chip
    /// is put away once there. A/B stops. The line for the log.
    pub fn side(&mut self, side: Side, keep: bool) -> String {
        let n = self.looper.selected();
        self.snap_ask = None;
        self.ab_stop();
        self.side_want = Some((n, side, keep));
        let what = match (side, keep) {
            (Side::Snap, false) => "Snap",
            (Side::Snap, true) => "Keep the snap",
            (Side::Tap, false) => "Use my tap",
            (Side::Tap, true) => "Keep my tap",
        };
        format!(
            "looper: {what}: Loop {} goes there on its next downbeat, if it isn't there",
            n + 1
        )
    }

    /// A footswitch's Snap or Use my tap (the Learn section's Looper
    /// card), with your hands on your instrument: while the hint line
    /// asks, Snap anyway or Not now; else the picked loop goes to that
    /// side ([`LoopHost::side`]), the chip staying. The line for the log,
    /// also when there is nothing to choose.
    pub fn choose(&mut self, side: Side) -> String {
        let what = match side {
            Side::Snap => "Snap",
            Side::Tap => "Use my tap",
        };
        let asked = match side {
            Side::Snap => self.snap_anyway(),
            Side::Tap => self.snap_not_now(),
        };
        if let Some(line) = asked {
            return format!("{line}, from a footswitch");
        }
        let n = self.looper.selected();
        if self.side_of(n).is_none() {
            return format!(
                "looper: {what} from a footswitch: Loop {} has no snap to choose",
                n + 1
            );
        }
        format!("{}, from a footswitch", self.side(side, false))
    }

    /// A/B on the seam on the picked loop ([`Command::AbSeam`]): each
    /// time round, from half way, its last beat runs into its downbeat as
    /// the other side (the snap, or your tap) has it, then as it is. Its
    /// length and the tempo never change, so the time engine is asked
    /// nothing (the owner's looper review, 5.6). Until a side is kept
    /// ([`LoopHost::side`]) or A/B goes off (the loop stays as it is,
    /// from the next half way). The line for the log, or why not.
    pub fn set_ab(&mut self, on: bool) -> String {
        let n = self.looper.selected();
        if !on {
            self.ab_stop();
            return format!("looper: A/B off: Loop {} stays as it is", n + 1);
        }
        let (Some(now), Some(s)) = (self.side_of(n), self.looper.loop_status(n)) else {
            return format!(
                "looper: Loop {}: A/B needs your tap and a snap to swap between",
                n + 1
            );
        };
        self.snap_ask = None;
        self.side_want = None;
        if !self.looper.command(Command::AbSeam(true)) {
            return "looper: A/B: NOT done, the command queue is full".to_string();
        }
        self.ab = Some((n, s.version));
        let other = match now {
            Side::Tap => "the snap",
            Side::Snap => "your tap",
        };
        format!(
            "looper: A/B: each time round, Loop {}'s last beat runs into its downbeat as \
             {other} has it, then as it is; the tempo stays",
            n + 1
        )
    }

    /// A/B goes off, if it is on: the looper finishes the round it is in
    /// on the loop's own side.
    fn ab_stop(&mut self) {
        if self.ab.take().is_some() {
            self.looper.command(Command::AbSeam(false));
        }
    }

    /// The loop point view opened or closed: while open, the picked
    /// loop's audio round its start and end is read for it.
    pub fn set_point_open(&mut self, open: bool) {
        self.point_open = open;
        if !open {
            self.point = None;
        }
    }

    /// Which side of smart loop end's choice loop `n` is on: your tap
    /// while something is on offer, the snap while smart loop end or a
    /// snap moved it and nothing is. None when it is neither (it changed
    /// in another way, or nothing was found).
    fn side_of(&self, n: usize) -> Option<Side> {
        let s = self.looper.loop_status(n)?;
        match (s.offer, s.moved) {
            (Some(_), _) => Some(Side::Tap),
            (None, Some(m)) if !m.is_none() => Some(Side::Snap),
            _ => None,
        }
    }

    /// The snap that takes loop `n` to `side`: Redo or Snap to the snap,
    /// Undo (of a snap) or Use my tap (or press) back. None if it is there
    /// already (or neither side).
    fn snap_to(&self, n: usize, side: Side) -> Option<Snap> {
        let now = self.side_of(n)?;
        if now == side {
            return None;
        }
        let s = self.looper.loop_status(n)?;
        let h = self.looper.loop_history(n)?;
        Some(match side {
            Side::Snap if matches!(h.redo().first(), Some(Step::Snap(_))) => Snap::Redo,
            Side::Snap => Snap::Found,
            Side::Tap if matches!(h.done().last(), Some(Step::Snap(_))) => Snap::Undo,
            Side::Tap if s.moved.is_some_and(|m| m.end != 0) => Snap::Tap,
            Side::Tap => Snap::Press,
        })
    }

    /// Smart loop end on the picked loop, for the screen: the close call
    /// while the loop is as it was read, the chip put away, the question
    /// waiting, A/B, and the loop point view's audio while the view is
    /// open (read again only when the loop, or where smart loop end left
    /// it, changes; kept while a snap lands).
    fn smart_view(&mut self, s: &Status) -> SmartView {
        let n = self.looper.selected();
        if self.point_open {
            self.read_point(n, s);
        }
        SmartView {
            close_call: self
                .close_call
                .filter(|c| c.0 == n && c.1 == s.version)
                .map(|c| c.2),
            kept: self.kept.is_some_and(|k| k == (n, s.version)),
            asking: self
                .snap_ask
                .filter(|a| a.0 == n)
                .map(|a| (a.1, self.reads_again(a.1))),
            length: s.asks,
            ab: self.ab == Some((n, s.version)) && s.ab.is_some(),
            point: self
                .point
                .filter(|p| self.point_open && p.0 == n)
                .map(|p| p.3),
        }
    }

    /// The loop point view's audio for loop `n` (status `s`): the windows
    /// round your press and your tap, from the first pass as it was kept.
    fn read_point(&mut self, n: usize, s: &Status) {
        // (Not moved is None.)
        let moved = s.moved.unwrap_or(Shift::NONE);
        if self
            .point
            .is_some_and(|p| p.0 == n && p.1 == s.version && p.2 == moved)
        {
            return;
        }
        let frames = point_frames(self.rate).min(self.point_buf.len());
        let buf = &mut self.point_buf[..frames];
        let Some((start, len)) = self.looper.read_kept(n, 0, &mut buf[..0]) else {
            return;
        };
        let ms = |m: f64| (m * f64::from(self.rate) / 1000.0).round() as i64;
        let press = start as i64 - moved.start;
        let tap = (start + len) as i64 - moved.end;
        let mut point = LoopPoint {
            start: [0; POINT_BARS],
            end: [0; POINT_BARS],
        };
        self.looper
            .read_kept(n, press + ms(POINT_START_FROM_MS), buf);
        peaks_into(buf, &mut point.start);
        self.looper.read_kept(n, tap + ms(POINT_END_FROM_MS), buf);
        peaks_into(buf, &mut point.end);
        self.point = Some((n, s.version, moved, point));
    }

    /// Whether snap `s` is an Undo or a Redo of Read as, before it lands.
    fn reads_again(&self, s: Snap) -> bool {
        let h = self.looper.history();
        let step = match s {
            Snap::Undo => h.done().last(),
            Snap::Redo => h.redo().first(),
            _ => None,
        };
        matches!(step, Some(Step::Bars(_)))
    }

    /// The snap waiting on the question "... anyway?" (it would change
    /// the tempo under recorded audio), if one is: its loop and kind, for
    /// the tests (the screen reads [`SmartView::asking`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn snap_asking(&self) -> Option<(usize, Snap)> {
        self.snap_ask
    }

    /// A first pass that closed waiting on its Length ([`Status::asks`]):
    /// the line saying so ([`length_ask_words`]), once a wait, on the
    /// screen too.
    pub fn take_asks(&mut self) -> Option<(String, bool)> {
        let mut line = None;
        for n in 0..MAX_LOOPS {
            let bit = 1u32 << n;
            match self.looper.loop_status(n).and_then(|s| s.asks) {
                Some(r) if self.asks_told & bit == 0 => {
                    self.asks_told |= bit;
                    line = line.or(Some((format!("looper: {}", length_ask_words(n, &r)), true)));
                }
                None => self.asks_told &= !bit,
                Some(_) => {}
            }
        }
        line
    }

    /// The picked loop's first pass waiting on its Length, if it is:
    /// its number and how it read.
    pub fn length_asking(&self) -> Option<(usize, Reading)> {
        let n = self.looper.selected();
        self.looper.loop_status(n)?.asks.map(|r| (n, r))
    }

    /// Read as (the hint line's yes while a first pass waits on its
    /// Length): it plays as it read ([`Command::UseReading`]). The line
    /// for the log, or None if nothing waits.
    pub fn use_reading(&mut self) -> Option<String> {
        let (n, r) = self.length_asking()?;
        self.looper.command(Command::UseReading);
        Some(format!(
            "looper: Loop {} plays as {}, from its top",
            n + 1,
            reading_words(&r)
        ))
    }

    /// Discard (the hint line's no while a first pass waits on its
    /// Length): the pass is cleared, as Clear does (UNDO brings it back).
    pub fn discard_waiting(&mut self) -> Option<String> {
        let (n, _) = self.length_asking()?;
        self.looper.command(Command::Clear);
        Some(format!(
            "looper: Loop {}'s pass thrown away (UNDO brings it back)",
            n + 1
        ))
    }

    /// Snap anyway: the snap that asked goes ahead, as [`LoopHost::snap`]
    /// does, the audio tracks' recordings left at their old tempo. The
    /// line for the log, or None if none asked.
    pub fn snap_anyway(&mut self) -> Option<String> {
        let (n, s) = self.snap_ask.take()?;
        self.snap_want = Some((n, s, true));
        Some(format!(
            "looper: {} anyway: Loop {} changes on its next downbeat, with the tempo",
            snap_name(s, self.reads_again(s)),
            n + 1
        ))
    }

    /// Not now: the snap that asked is dropped and nothing changes. The
    /// line for the log, or None if none asked.
    pub fn snap_not_now(&mut self) -> Option<String> {
        let (n, _) = self.snap_ask.take()?;
        Some(format!(
            "looper: Loop {}: not now: it stays as it is",
            n + 1
        ))
    }

    /// Every period, after the time engine's and before
    /// [`LoopHost::run`]: a snap asked for is worked out
    /// ([`Looper::snap_plan`]) and its tempo asked of the time engine on
    /// the loop's next downbeat ([`TimeHost::timing_change`]): refused
    /// while another song loop has sound (its tempo would change under it;
    /// a free loop keeps the free clock's), or whatever the time engine
    /// refuses. While `audio` (recorded audio, the audio tracks' regions,
    /// would play out of step) it asks first ([`LoopHost::snap_asking`])
    /// and goes ahead on Snap anyway. Once the time engine commits it, the
    /// loop is told to change on the same frame ([`Looper::snap_at`]); if
    /// the loop changes first, the tempo is taken back. A free loop's
    /// changes the free clock's tempo instead: no other loop but a free
    /// one is in its way, nothing is asked of the time engine, and the
    /// looper gets the plan in the period its frame falls in. A refusal:
    /// the line for the log and the screen, counted.
    pub fn tend_snap(&mut self, time: &mut TimeHost, audio: bool) -> Option<(String, bool)> {
        if let Some(p) = self.free_snap {
            if self
                .looper
                .loop_status(p.n)
                .is_none_or(|s| s.version != p.version)
            {
                self.free_snap = None;
                return Some(self.snap_refused(
                    p.n,
                    p.snap,
                    "the loop changed before its downbeat",
                ));
            }
            if p.at < time.next_frame() + PERIOD as u64 {
                // Said once it has landed (take_snap).
                self.free_snap = None;
                self.looper.snap_at(p);
            }
            return None;
        }
        if let Some((p, ticket)) = self.snap_wait {
            if self
                .looper
                .loop_status(p.n)
                .is_none_or(|s| s.version != p.version)
            {
                self.snap_wait = None;
                time.cancel_timing(ticket, "the loop changed");
                return Some(self.snap_refused(
                    p.n,
                    p.snap,
                    "the loop changed before its downbeat",
                ));
            }
            match time.timing_answer(ticket) {
                Some(Answer::Waiting) => return None,
                Some(Answer::Committed(at)) => {
                    self.snap_wait = None;
                    // Said once it has landed (take_snap).
                    self.looper.snap_at(SnapPlan { at, ..p });
                    return None;
                }
                Some(Answer::Cancelled(why) | Answer::Refused(why)) => {
                    self.snap_wait = None;
                    return Some(self.snap_refused(p.n, p.snap, why));
                }
                None => {
                    self.snap_wait = None;
                    return Some(self.snap_refused(p.n, p.snap, "the time engine lost it"));
                }
            }
        }
        if self.snap_want.is_none() {
            let (n, side, keep) = self.side_want?;
            match self.snap_to(n, side) {
                Some(s) => self.snap_want = Some((n, s, false)),
                None => {
                    // There already (or neither side any more).
                    self.side_want = None;
                    if keep {
                        let v = self.looper.loop_status(n).map_or(0, |s| s.version);
                        self.kept = Some((n, v));
                    }
                    return None;
                }
            }
        }
        let (n, s, anyway) = self.snap_want.take()?;
        // A free loop's snap changes the free clock's tempo, not the
        // song's: song loops and the audio tracks aren't in its way, the
        // looper says if another free loop is, and there is nothing to ask
        // the time engine.
        if self
            .looper
            .loop_status(n)
            .is_some_and(|st| st.clock == Clock::Free)
        {
            return match self.looper.snap_plan(n, s, time.next_frame()) {
                Ok(p) => {
                    self.free_snap = Some(p);
                    None
                }
                Err(why) => Some(self.snap_refused(n, s, why)),
            };
        }
        // Another song loop with sound would change tempo under it; a
        // free loop keeps the free clock's.
        let others = (0..MAX_LOOPS)
            .filter(|&k| k != n)
            .filter_map(|k| self.looper.loop_status(k))
            .any(|st| st.phase != Phase::Empty && st.clock == Clock::Song);
        if others {
            return Some(self.snap_refused(n, s, "another song loop has sound"));
        }
        let p = match self.looper.snap_plan(n, s, time.next_frame()) {
            Ok(p) => p,
            Err(why) => return Some(self.snap_refused(n, s, why)),
        };
        let c = TimingChange {
            at: p.at,
            tempo: Some(p.bpm),
            position: None,
            play: false,
            take: None,
            made_against: time.timeline(),
        };
        match time.allowed(&c, audio) {
            Allowed::Safe => {}
            Allowed::Ask(_) if anyway => {}
            Allowed::Ask(_) => {
                self.snap_ask = Some((n, s));
                self.side_want = None;
                self.ab_stop();
                return Some((
                    format!("looper: {}", ask_words(n, s, self.reads_again(s))),
                    true,
                ));
            }
            Allowed::Refused(why) => return Some(self.snap_refused(n, s, why)),
        }
        match time.timing_change(c) {
            Ok(ticket) => {
                self.snap_wait = Some((p, ticket));
                None
            }
            Err(why) => Some(self.snap_refused(n, s, why)),
        }
    }

    /// Snap `s` on loop `n` can't be made (`why`): counted; the line for
    /// the log and the screen.
    fn snap_refused(&mut self, n: usize, s: Snap, why: &str) -> (String, bool) {
        self.snaps_refused += 1;
        // A side asked for, and A/B, stop there.
        self.side_want = None;
        self.ab_stop();
        (
            format!(
                "looper: Loop {}: can't {}: {why}",
                n + 1,
                snap_verb(s, self.reads_again(s))
            ),
            true,
        )
    }

    /// After the looper's period: what became of a snap given it
    /// ([`Looper::take_snap`]): the line for the log, said on the screen
    /// too ("snapped the end 48 ms earlier: 2 bars at 120.00 BPM"), and the tempo
    /// the song now plays at, for the app's (it changed with the loop, or
    /// already had when a loop changed first).
    pub fn take_snap(&mut self) -> (Option<(String, bool)>, Option<f64>) {
        let Some(SnapDone { plan: p, landed }) = self.looper.take_snap() else {
            return (None, None);
        };
        let n = p.n + 1;
        // A free loop's snap changes the free clock's tempo, never the
        // song's.
        let free = self
            .looper
            .loop_status(p.n)
            .is_some_and(|s| s.clock == Clock::Free);
        let song_bpm = (!free).then_some(p.bpm);
        if !landed {
            self.snaps_refused += 1;
            let why = if free {
                "the loop changed, or the free loops stopped first"
            } else {
                "the loop changed (the tempo did)"
            };
            return (
                Some((
                    format!(
                        "looper: Loop {n}: {} didn't land: {why}",
                        snap_name(p.snap, self.reads_again(p.snap))
                    ),
                    true,
                )),
                song_bpm,
            );
        }
        let Some(s) = self.looper.loop_status(p.n) else {
            return (None, song_bpm);
        };
        let bar_ticks = match self.looper.free_clock().filter(|_| free) {
            Some(c) => c.meter.bar_ticks(),
            None => self.bar_ticks,
        };
        let bars = ticks_text(s.ticks, bar_ticks);
        let moves = shift_text(p.by, self.rate);
        // Landed: an Undo's step is the next Redo's now, and the other way.
        let h = self.looper.history();
        let read = |s: Option<&Step>| matches!(s, Some(Step::Bars(_)));
        let what = match p.snap {
            Snap::Found => format!("snapped {moves}"),
            Snap::Tap => "back to your tap".to_string(),
            Snap::Press => "back to your press".to_string(),
            Snap::Bars(_) => "read again".to_string(),
            Snap::Undo if read(h.redo().first()) => "Undo: read as before".to_string(),
            Snap::Redo if read(h.done().last()) => "Redo: read again".to_string(),
            Snap::Undo => format!("Undo: {moves}"),
            Snap::Redo => format!("Redo: {moves}"),
        };
        let at = if free { ", free at" } else { " at" };
        (
            Some((
                format!(
                    "looper: Loop {n}: {what}: {bars}{at} {:.2} BPM, from its downbeat",
                    p.bpm
                ),
                true,
            )),
            song_bpm,
        )
    }

    /// After the looper's period: what smart loop end made of a closing
    /// tap ([`Looper::take_smart`]), once a pass. The line for the log
    /// (with the best two ends, scored) and, when your tap stood, the
    /// words for the screen.
    pub fn take_smart(&mut self) -> Option<(String, Option<String>)> {
        let note = self.looper.take_smart()?;
        Some(smart_words(&note, self.smart, self.rate))
    }

    /// The song can't start at the tempo loop `n` set (`why`): counted;
    /// the line for the log and the screen. The loop plays at the song's
    /// tempo when it starts.
    pub fn tempo_refused(&mut self, n: usize, bars: u32, bpm: f64, why: &str) -> String {
        self.tempos_refused += 1;
        format!(
            "looper: Loop {} is {} at {bpm:.2} BPM, but the song can't start at that tempo: \
             {why}. The loop plays at the song's tempo when it starts",
            n + 1,
            bars_words_n(bars)
        )
    }

    /// Record from `s` (the pads or the line input): Capture forgets what
    /// it heard from the other.
    /// Listen: `what` Some(None) plays the picked loop alone, round and
    /// round from its start, while the song is stopped; Some(Some(id))
    /// one of its layers, even a muted one. The same again, or None,
    /// stops it; PLAY stops it too. The line says what it did, or why
    /// not.
    pub fn listen(&mut self, what: Option<Option<u8>>) -> String {
        let n = self.picked;
        let now = self.looper.listening();
        let Some(layer) = what else {
            self.looper.stop_listening();
            return "Listen: stopped".to_string();
        };
        if now.is_some_and(|(m, l, _)| m == n && l == layer) {
            self.looper.stop_listening();
            return "Listen: stopped".to_string();
        }
        let s = self.looper.status();
        let name = match layer {
            None => format!("Loop {}", n + 1),
            Some(_) => format!("Loop {}'s layer", n + 1),
        };
        if self.looper.listen(n, layer) {
            return format!("Listen: {name} plays on its own (again or PLAY: stop)");
        }
        match s.phase {
            Phase::Empty | Phase::Armed => {
                format!("Listen: Loop {} has nothing to play yet", n + 1)
            }
            Phase::Recording | Phase::Overdubbing => {
                format!("Listen: Loop {} is recording", n + 1)
            }
            Phase::Playing if self.last_rolling => {
                "Listen plays while the music is stopped: you hear the loops with it now"
                    .to_string()
            }
            Phase::Playing => format!("Listen: {name} is busy (merging): try again in a moment"),
        }
    }

    /// The length of the pattern playing, in bars (the longest clip that
    /// plays, rounded up to whole bars; None when nothing plays, or past
    /// [`PATTERN_MAX_BARS`]). With Length: Pattern the picked loop, while
    /// empty, is kept at it, so a tap records exactly one pattern's worth.
    pub fn set_pattern_bars(&mut self, bars: Option<u32>) {
        let bars = bars.filter(|&b| (1..=PATTERN_MAX_BARS).contains(&b));
        self.pattern_bars = bars;
        if !self.match_pattern {
            return;
        }
        let s = self.looper.status();
        if s.phase == Phase::Empty && s.capturing.is_none() && s.bars != bars {
            self.looper.command(Command::SetBars(bars));
        }
    }

    /// One slice of the loop's waveform; the finished wave when it is
    /// done. The wave starts again when the loop, its layers or the phase
    /// change, and every `refresh` periods while overdubbing. While the
    /// selected loop's first pass records, its wave so far instead, every
    /// [`LIVE_EVERY`] periods. Costs at most [`WAVE_BUDGET`] layer-frames
    /// a period, and never allocates: the waves are made again in place
    /// once the screen lets go of them. While that wave is done, each
    /// loop's small wave for its cells on the Clips grid instead
    /// ([`LooperView::cell_waves`]), on the same budget.
    pub fn wave_step(&mut self, period: u64, refresh: u64) -> Option<Arc<LoopWave>> {
        let w = self.loop_wave_step(period, refresh);
        if w.is_none() && !self.wave.building && self.live.of.is_none() {
            self.cells.step(&self.looper, period);
        }
        w
    }

    fn loop_wave_step(&mut self, period: u64, refresh: u64) -> Option<Arc<LoopWave>> {
        let sel = self.looper.selected();
        let s = self.looper.status();
        if s.phase == Phase::Recording {
            return self.live_step(sel, &s, period);
        }
        self.live.of = None;
        let of = (s.frames, s.layers, s.phase, sel);
        let changed = of != self.wave.of;
        if s.frames == 0 {
            if self.wave_shown {
                // Cleared, thrown away or another loop: the empty wave
                // (next period again if the screen holds every wave).
                let w = self.waves.make(0, |w| {
                    w.frames = 0;
                    w.recorded = None;
                })?;
                self.wave_shown = false;
                self.wave.start(0, 0, of);
                return Some(w);
            }
            if changed {
                self.wave.start(0, 0, of);
            }
            return None;
        }
        let overdubbing =
            s.phase == Phase::Overdubbing && refresh > 0 && period.is_multiple_of(refresh);
        if changed || overdubbing && !self.wave.building {
            self.wave.start(s.frames, s.layers, of);
        }
        if !self.wave.step(&self.looper) {
            return None;
        }
        let w = self.wave.finish(&mut self.waves)?;
        self.wave_shown = true;
        Some(w)
    }

    /// The selected loop's first pass, recording: its new frames into the
    /// live wave, and the wave so far when due.
    fn live_step(&mut self, sel: usize, s: &Status, period: u64) -> Option<Arc<LoopWave>> {
        if self.live.of != Some(sel) {
            let bar = frames_at(self.bar_ticks.max(1), self.bpm.max(1.0), self.rate);
            self.live
                .start(sel, bar * s.bars.map_or(2, |b| b.max(1) as usize));
        }
        if let Some(pass) = self.looper.first_pass(sel) {
            self.live.step(&pass, s.recorded);
        }
        if !period.is_multiple_of(LIVE_EVERY) || self.live.done == self.live.sent {
            return None;
        }
        let w = self.live.make(&mut self.waves)?;
        self.wave_shown = true;
        Some(w)
    }
}

pub fn bars_text(b: Option<u32>) -> String {
    match b {
        Some(PATTERN_BARS) => "the pattern's length".to_string(),
        Some(1) => "1 bar".to_string(),
        Some(n) => format!("{n} bars"),
        None => "free (until STOP or Record again)".to_string(),
    }
}

/// How a loop's overdubs record, as the log says it.
pub fn overdubs_text(o: mpfree_looper::Overdubs) -> &'static str {
    match o {
        mpfree_looper::Overdubs::Add => "Add (each pass adds a layer until you stop it)",
        mpfree_looper::Overdubs::Replace => {
            "Replace (what you play takes the place of what was there)"
        }
        mpfree_looper::Overdubs::Once => "Once (one pass, then back to playing)",
    }
}

/// The frames of a loop point window at `rate`.
fn point_frames(rate: u32) -> usize {
    (f64::from(rate) * POINT_MS / 1000.0).round() as usize
}

/// `audio`'s peaks in `out.len()` slices, each its loudest sample, 0 to
/// 255 at full scale.
fn peaks_into(audio: &[[f32; 2]], out: &mut [u8]) {
    let n = out.len().max(1);
    for (i, o) in out.iter_mut().enumerate() {
        let (a, b) = (i * audio.len() / n, (i + 1) * audio.len() / n);
        let peak = audio[a..b]
            .iter()
            .fold(0.0f32, |m, x| m.max(x[0].abs()).max(x[1].abs()));
        *o = (peak.min(1.0) * 255.0).round() as u8;
    }
}

/// A snap as the log names it; `read`: an Undo or Redo is of Read as.
fn snap_name(s: Snap, read: bool) -> String {
    match s {
        Snap::Found => "Snap".to_string(),
        Snap::Tap => "Use my tap".to_string(),
        Snap::Press => "Use my press".to_string(),
        Snap::Bars(b) => format!("Read as {}", bars_words_n(b)),
        Snap::Undo if read => "Undo of Read as".to_string(),
        Snap::Redo if read => "Redo of Read as".to_string(),
        Snap::Undo => "Undo of the snap".to_string(),
        Snap::Redo => "Redo of the snap".to_string(),
    }
}

/// The question a snap over recorded audio asks, on the hint line while
/// it waits ([`LoopHost::snap_asking`]): loop `n`, snap `s`, `read` as
/// for [`snap_name`].
pub fn ask_words(n: usize, s: Snap, read: bool) -> String {
    let verb = snap_verb(s, read);
    let mut c = verb.chars();
    let verb = c
        .next()
        .map(|f| f.to_uppercase().chain(c).collect::<String>())
        .unwrap_or_default();
    format!(
        "Loop {}: that changes the tempo under the audio tracks' recordings. {verb} anyway?",
        n + 1
    )
}

/// The question a first pass waiting on its Length asks ([`Status::asks`]):
/// Length's bars would be slower than the looper goes, so read it as the
/// bars it read?
pub fn length_ask_words(n: usize, r: &Reading) -> String {
    let said = r.said.unwrap_or(r.bars);
    let slow = r.bpm * f64::from(said) / f64::from(r.bars.max(1));
    format!(
        "Loop {}: Length's {} would be {slow:.0} BPM, slower than {LOOP_MIN_BPM:.0}. \
         Read it as {} at {:.0} BPM?",
        n + 1,
        bars_words_n(said),
        bars_words_n(r.bars),
        r.bpm
    )
}

/// Read as's words on the hint line while a first pass waits on its
/// Length: "Read as 2 bars".
pub fn length_ask_button(r: &Reading) -> String {
    format!("Read as {}", bars_words_n(r.bars))
}

/// The hint line's button that answers yes to [`ask_words`]: Snap
/// anyway for a snap, Do it anyway for the rest (Read as, an Undo or a
/// Redo).
pub fn ask_button(s: Snap) -> &'static str {
    match s {
        Snap::Found | Snap::Tap | Snap::Press => "Snap anyway",
        Snap::Bars(_) | Snap::Undo | Snap::Redo => "Do it anyway",
    }
}

/// A snap as "can't ..." says it; `read` as for [`snap_name`].
fn snap_verb(s: Snap, read: bool) -> String {
    match s {
        Snap::Found => "snap".to_string(),
        Snap::Tap => "go back to your tap".to_string(),
        Snap::Press => "go back to your press".to_string(),
        Snap::Bars(b) => format!("read it as {}", bars_words_n(b)),
        Snap::Undo if read => "undo Read as".to_string(),
        Snap::Redo if read => "redo Read as".to_string(),
        Snap::Undo => "undo the snap".to_string(),
        Snap::Redo => "redo the snap".to_string(),
    }
}

/// Smart loop end's setting, as the log says it.
pub fn smart_end_text(m: SmartEnd) -> &'static str {
    match m {
        SmartEnd::Off => "Off (the loop closes on your tap)",
        SmartEnd::Suggest => "Suggest (it closes on your tap, then offers the end you played)",
        SmartEnd::Auto => {
            "Auto (it listens a moment past your tap, then closes on the end you played)"
        }
    }
}

/// `frames` at `rate` in whole milliseconds, without the sign.
fn ms(frames: i64, rate: u32) -> f64 {
    (frames.unsigned_abs() as f64 * 1000.0 / f64::from(rate.max(1))).round()
}

/// A snap's moves, for the log: "the start 80 ms later and the end 61 ms
/// earlier".
fn shift_text(by: Shift, rate: u32) -> String {
    let way = |f: i64| if f < 0 { "earlier" } else { "later" };
    let mut parts = Vec::new();
    if by.start != 0 {
        parts.push(format!(
            "the start {:.0} ms {}",
            ms(by.start, rate),
            way(by.start)
        ));
    }
    if by.end != 0 || by.start == 0 {
        parts.push(format!(
            "the end {:.0} ms {}",
            ms(by.end, rate),
            way(by.end)
        ));
    }
    parts.join(" and ")
}

/// Your tap was late or early by a move of `by` frames (negative: the
/// end found is earlier, so the tap was late).
fn tap_off(by: i64, rate: u32) -> String {
    format!(
        "your tap was {:.0} ms {}",
        ms(by, rate),
        if by < 0 { "late" } else { "early" }
    )
}

/// One scored end, for the log: where it is from the tap and its score.
fn candidate_text(c: &Candidate, tap: u64, rate: u32) -> String {
    let at = if c.end == tap {
        "your tap".to_string()
    } else {
        let by = c.end as i64 - tap as i64;
        format!(
            "{:.0} ms {} your tap",
            ms(by, rate),
            if by < 0 { "before" } else { "after" }
        )
    };
    format!(
        "{at}, score {:.2} (onset {:.2}, rhythm {:.2}, context {:.2}, prior {:.2})",
        c.score, c.onset, c.rhythm, c.context, c.prior
    )
}

/// What smart loop end made of a closing tap ([`LoopHost::take_smart`]):
/// the log line, with the best two ends scored, and when the tap stood,
/// the words for the screen (every fallback is shown).
fn smart_words(note: &SmartNote, mode: SmartEnd, rate: u32) -> (String, Option<String>) {
    let n = note.n + 1;
    // A smart start: Auto started the loop on your first hit, Suggest
    // offers to.
    let after = format!(
        "on your first hit, {:.0} ms after REC",
        ms(note.start, rate)
    );
    let (started, start_words) = match (note.start, mode) {
        (0, _) => (String::new(), String::new()),
        (_, SmartEnd::Auto) => (
            format!(", and started it {after}"),
            format!(" Started {after}."),
        ),
        _ => (
            format!(", and offers to start it {after}"),
            format!(" Snap starts it {after}."),
        ),
    };
    let scored = match (note.best, note.second) {
        (Some(b), Some(s)) => format!(
            " (best: {}; next: {})",
            candidate_text(&b, note.tap, rate),
            candidate_text(&s, note.tap, rate)
        ),
        (Some(b), None) => format!(" (best: {})", candidate_text(&b, note.tap, rate)),
        _ => String::new(),
    };
    let sure = note
        .sure
        .map_or(String::new(), |c| format!("; how sure: {}", sure_words(&c)));
    let (line, screen) = match note.outcome {
        SmartOutcome::Moved(by) => (
            format!(
                "looper: Loop {n}: {}; smart loop end closed it on the end you played{started}{scored}",
                tap_off(by, rate)
            ),
            None,
        ),
        SmartOutcome::Offered(by) => (
            format!(
                "looper: Loop {n}: {}; smart loop end offers the end you played{started}{scored}",
                tap_off(by, rate)
            ),
            None,
        ),
        SmartOutcome::Close(by) => (
            format!(
                "looper: Loop {n}: smart loop end found the end you played {:.0} ms from your \
                 tap, too near to hear{started}{scored}",
                ms(by, rate)
            ),
            (note.start != 0).then(|| start_words.trim_start().to_string()),
        ),
        SmartOutcome::Kept(why) => (
            format!("looper: Loop {n}: smart loop end kept your tap: {why}{started}{scored}"),
            Some(if why == mpfree_looper::OVERDUB_AT_ONCE {
                format!("Kept your tap: {why}. Auto can fix this before it does.")
            } else {
                format!("Kept your tap: {why}.{start_words}")
            }),
        ),
    };
    (line + &sure, screen)
}

/// How sure smart loop end was of a first pass's close, four ways
/// ([`close::Confidence`]), for the log: "the end clear, the bars a close
/// call, the tempo from the length alone, the start on your first hit".
pub fn sure_words(c: &close::Confidence) -> String {
    let end = match c.boundary {
        Boundary::Clear => "the end clear",
        Boundary::Possible => "the end only possible",
        Boundary::Unmatched => "no end to go on",
    };
    let bars = match c.bars {
        Bars::Yours => "the bars yours",
        Bars::Clear => "the bars clear",
        Bars::Ambiguous => "the bars a close call",
    };
    let tempo = match c.tempo {
        Tempo::FromYourBars => "the tempo from your bars",
        Tempo::FromTheBeat => "the tempo from the beat you played",
        Tempo::FromTheLength => "the tempo from the length alone",
    };
    let start = match c.start {
        Start::FirstHit => "the start on your first hit",
        Start::Press => "the start on your press",
    };
    format!("{end}, {bars}, {tempo}, {start}")
}

/// A loop's length in bars of `bar` ticks, for the log.
fn ticks_text(ticks: i64, bar: Tick) -> String {
    let bar = bar.max(1);
    if ticks % bar == 0 {
        bars_text(Some((ticks / bar) as u32))
    } else {
        format!("{:.2} bars", ticks as f64 / bar as f64)
    }
}

/// The latency to make up for when nothing has been measured: the output
/// buffer (`periods` of 128 frames) plus one period of input. The
/// converters add an unknown amount; measure it with a cable from out to in.
pub fn estimated_latency(periods: u32) -> u32 {
    (periods + 1) * PERIOD as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An audio track's line pick, as a loop in its cell hears it (phase
    /// 4b): one jack on both sides, half of each for Sum (as the take
    /// sums them), both as they are for Stereo.
    #[test]
    fn a_track_line_pick_fills_both_sides_as_its_take_hears_it() {
        use mpfree_audiotracks::Input;
        let (l, r) = ([0.5f32; PERIOD], [-0.25f32; PERIOD]);
        let pick = |p: Input| {
            let (mut a, mut b) = ([9f32; PERIOD], [9f32; PERIOD]);
            pick_line(p, (&l, &r), &mut a, &mut b);
            (a[PERIOD - 1], b[0])
        };
        assert_eq!(pick(Input::Left), (0.5, 0.5));
        assert_eq!(pick(Input::Right), (-0.25, -0.25));
        assert_eq!(pick(Input::Stereo), (0.5, -0.25));
        assert_eq!(pick(Input::Sum), (0.125, 0.125));
        let src = |from| {
            LoopSource::Track(TrackInput {
                channel: mpfree_fx::StripId::Track(3),
                from,
            })
        };
        assert!(src(TrackFrom::Line(Input::Left)).line());
        let mix = TrackFrom::Mix(
            mpfree_fx::StripId::Master,
            mpfree_fx::CapturePoint::BeforeInserts,
        );
        assert!(src(mix).inside() && !src(mix).line());
        assert_eq!(
            source_words(src(mix), None),
            "the track's input, the master before its effects"
        );
    }
    use mpfree_transport::{Command as T, Transport};

    fn s32(v: f32) -> i32 {
        (v * 2_147_483_647.0) as i32
    }

    #[test]
    fn a_closed_input_gives_silence_and_counts_nothing() {
        let mut l = LineIn::closed();
        let (mut a, mut b) = ([1.0; PERIOD], [1.0; PERIOD]);
        assert!(l.period(&mut a, &mut b).is_none());
        assert!(a.iter().chain(b.iter()).all(|&v| v == 0.0));
        assert_eq!(l.stats, LineStats::default());
        assert_eq!(l.close(), "line input: already off");
    }

    #[test]
    fn opening_a_device_that_is_not_there_says_so_and_stays_off() {
        let mut l = LineIn::closed();
        let e = l.open(99).unwrap_err();
        assert!(
            e.contains("/dev/snd/pcmC99D0c") && e.contains("stays off"),
            "{e}"
        );
        assert!(!l.is_open());
    }

    #[test]
    fn the_fifo_hands_out_whole_periods_and_drops_what_runs_ahead() {
        let mut l = LineIn::closed();
        // Pretend it is open: drive the FIFO directly.
        let frames: Vec<i32> = (0..3 * PERIOD)
            .flat_map(|f| [s32(f as f32 / 1000.0), s32(-0.5)])
            .collect();
        l.push_s32(&frames);
        assert_eq!(l.len, 3 * PERIOD);
        let (mut a, mut b) = ([0.0; PERIOD], [0.0; PERIOD]);
        // `period` reads only while open, so take periods by hand as it does.
        let take = |l: &mut LineIn, a: &mut [f32; PERIOD], b: &mut [f32; PERIOD]| {
            for f in 0..PERIOD {
                let at = (l.head + f) % FIFO;
                a[f] = l.fifo[2 * at];
                b[f] = l.fifo[2 * at + 1];
            }
            l.head = (l.head + PERIOD) % FIFO;
            l.len -= PERIOD;
        };
        take(&mut l, &mut a, &mut b);
        assert!((a[5] - 0.005).abs() < 1e-6 && (b[0] + 0.5).abs() < 1e-6);
        // Overfilling drops the oldest frames, counted.
        let lots: Vec<i32> = vec![s32(0.25); 2 * FIFO];
        l.push_s32(&lots);
        assert_eq!(l.len, FIFO);
        assert_eq!(l.stats.dropped, 2 * PERIOD as u64);
        take(&mut l, &mut a, &mut b);
        assert!((a[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_wave_covers_the_loop_and_answers_any_zoom() {
        let w = LoopWave {
            layers: Vec::new(),
            frames: 1000,
            recorded: None,
            columns: (0..10)
                .map(|i| {
                    let v = i as f32 / 10.0;
                    [
                        WaveColumn {
                            min: -v,
                            max: v,
                            rms: v / 2.0,
                        },
                        WaveColumn::default(),
                    ]
                })
                .collect(),
        };
        let c = w.column(0, 0.0, 1000.0);
        assert_eq!((c.min, c.max), (-0.9, 0.9));
        let c = w.column(0, 520.0, 530.0);
        assert_eq!(c.max, 0.5);
        assert_eq!(w.column(0, 2000.0, 3000.0), WaveColumn::default());
        assert_eq!(w.column(2, 0.0, 10.0), WaveColumn::default());
    }

    /// In 6/8 a bar is six eighths (2880 ticks), not six quarters: a free
    /// first loop of one bar says 1 bar (it said 0), the view's beat is an
    /// eighth, and six of them make the loop.
    #[test]
    fn a_free_loop_counts_its_bars_by_the_meter() {
        const RATE: u32 = 22_050;
        let mut h = LoopHost::new(RATE, 20);
        let mut t = Transport::new(RATE, 100.0).unwrap();
        t.set_meter(mpfree_transport::Meter::new(6, 8).unwrap())
            .unwrap();
        let mut b = Block::new();
        h.set_tempo_from_loop(true);
        h.command(Command::SetBars(None));
        let mut run = |h: &mut LoopHost, periods: usize| {
            let mut took = None;
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
                took = took.or(h.take_tempo().map(|(n, r, at, _)| (n, r, at)));
            }
            took
        };
        h.command(Command::Record);
        // 1.5 s free: one bar of 6/8 at 120 BPM (three quarters).
        assert_eq!(run(&mut h, (3 * RATE as usize / 2) / PERIOD), None);
        h.command(Command::Record);
        let (n, reading, _) = run(&mut h, 4).expect("the loop sets the tempo");
        let bpm = reading.bpm;
        assert_eq!((n, reading.bars), (0, 1), "one bar of 6/8, at {bpm:.2} BPM");
        assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
        let v = h.view(bpm);
        assert_eq!(v.bar_ticks, 3 * PPQ as Tick);
        let six = v.frames_per_beat * f64::from(v.beats_per_bar);
        assert!(
            (six - v.status.frames as f64).abs() < 1.0,
            "six eighths, {six} frames, make the loop's {}",
            v.status.frames
        );
        assert_eq!(crate::looper_page::bars_of(v.status.ticks, v.bar_ticks), 1);
    }

    /// Smart loop end's log line says how sure it was of the end, the
    /// bars, the tempo and the start, kept apart (the looper refactor's
    /// four confidences); with no decision (an overdub started on the
    /// tap) it says nothing of them.
    #[test]
    fn the_smart_loop_end_line_says_how_sure_it_was_four_ways() {
        let c = close::Confidence {
            boundary: Boundary::Clear,
            bars: Bars::Ambiguous,
            tempo: Tempo::FromTheLength,
            start: Start::FirstHit,
        };
        assert_eq!(
            sure_words(&c),
            "the end clear, the bars a close call, the tempo from the length alone, \
             the start on your first hit"
        );
        let c = close::Confidence {
            boundary: Boundary::Possible,
            bars: Bars::Yours,
            tempo: Tempo::FromYourBars,
            start: Start::Press,
        };
        assert_eq!(
            sure_words(&c),
            "the end only possible, the bars yours, the tempo from your bars, the start on your press"
        );
        let mut note = SmartNote {
            n: 0,
            outcome: SmartOutcome::Kept("no note near it"),
            start: 0,
            tap: 1_000,
            best: None,
            second: None,
            sure: Some(close::Confidence {
                boundary: Boundary::Unmatched,
                bars: Bars::Clear,
                tempo: Tempo::FromTheBeat,
                start: Start::Press,
            }),
        };
        let (line, words) = smart_words(&note, SmartEnd::Suggest, 44_100);
        assert_eq!(
            line,
            "looper: Loop 1: smart loop end kept your tap: no note near it; how sure: no end to \
             go on, the bars clear, the tempo from the beat you played, the start on your press"
        );
        assert_eq!(words.as_deref(), Some("Kept your tap: no note near it."));
        note.sure = None;
        let (line, _) = smart_words(&note, SmartEnd::Suggest, 44_100);
        assert_eq!(
            line,
            "looper: Loop 1: smart loop end kept your tap: no note near it"
        );
        // Loop memory was full: the screen says why the end after the tap
        // couldn't be offered.
        note.outcome = SmartOutcome::Kept(mpfree_looper::MEMORY_FULL);
        let (_, words) = smart_words(&note, SmartEnd::Suggest, 44_100);
        assert_eq!(
            words.as_deref(),
            Some("Kept your tap: loop memory was full, so nothing after it was kept.")
        );
    }

    /// Length's bars are final (the looper refactor's Finding C): 1 bar
    /// in 10 s would be 24 BPM, so the pass waits and the hint line asks;
    /// Read as plays it as it read, Discard clears it.
    #[test]
    fn a_length_too_slow_to_keep_asks_and_read_as_or_discard_answers() {
        const RATE: u32 = 22_050;
        for yes in [true, false] {
            let mut h = LoopHost::new(RATE, 20);
            let mut t = Transport::new(RATE, 100.0).unwrap();
            let mut b = Block::new();
            h.set_tempo_from_loop(true);
            h.command(Command::SetBars(Some(1)));
            let mut run = |h: &mut LoopHost, periods: usize| {
                let mut took = None;
                for _ in 0..periods {
                    assert!(t.process(PERIOD as u32, &mut b));
                    let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                    h.period(&b, &mut l, &mut r);
                    took = took.or(h.take_tempo());
                }
                took
            };
            h.command(Command::Record);
            assert_eq!(run(&mut h, 10 * RATE as usize / PERIOD), None);
            h.command(Command::Record);
            assert_eq!(run(&mut h, 4), None, "no tempo while it asks");
            let (line, tell) = h.take_asks().expect("it says it waits");
            assert!(tell);
            assert!(
                line.starts_with(
                    "looper: Loop 1: Length's 1 bar would be 24 BPM, slower than 30. Read it as "
                ),
                "{line}"
            );
            assert_eq!(h.take_asks(), None, "once");
            let r = h.view(100.0).smart.length.expect("the hint line asks");
            assert_eq!(r.said, Some(1));
            assert_eq!(length_ask_button(&r), format!("Read as {} bars", r.bars));
            assert_eq!(h.length_asking(), Some((0, r)));
            if yes {
                let said = h.use_reading().expect("Read as");
                assert!(said.starts_with("looper: Loop 1 plays as "), "{said}");
                let (n, reading, _, into) = run(&mut h, 2).expect("the tempo, once you say");
                assert_eq!((n, reading, into), (0, r, 0));
                assert_eq!(h.looper.loop_status(0).unwrap().phase, Phase::Playing);
            } else {
                let said = h.discard_waiting().expect("Discard");
                assert_eq!(
                    said,
                    "looper: Loop 1's pass thrown away (UNDO brings it back)"
                );
                assert_eq!(run(&mut h, 2), None);
                assert_eq!(h.looper.loop_status(0).unwrap().phase, Phase::Empty);
            }
            assert_eq!(h.view(100.0).smart.length, None);
            assert_eq!((h.use_reading(), h.discard_waiting()), (None, None));
        }
    }

    /// The period a free first loop closes on a tap, at a tempo the song
    /// doesn't have yet, asks the printer for nothing: the song takes the
    /// loop's tempo from the next period, and the printer looks at the
    /// loop at that tempo. (It asked for a print at the song's old tempo,
    /// and that first message to its worker allocated on the audio
    /// thread.) A song that doesn't take the tempo gets its print asked
    /// for in the next period, as before.
    #[test]
    fn the_period_a_free_loop_sets_the_tempo_asks_the_printer_for_nothing() {
        const RATE: u32 = 44_100;
        let mut h = LoopHost::new(RATE, 20);
        let mut t = Transport::new(RATE, 100.0).unwrap();
        let mut b = Block::new();
        h.set_tempo_from_loop(true);
        h.command(Command::SetBars(None));
        let mut period = |h: &mut LoopHost| {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            let (_, calls) = crate::sys::alloc_count::calls(|| h.period(&b, &mut l, &mut r));
            (calls, h.take_tempo())
        };
        h.command(Command::Record);
        // 2 s free: a bar at 120 BPM, with the song at 100.
        for _ in 0..(2 * RATE as usize) / PERIOD {
            assert_eq!(period(&mut h).1, None);
        }
        h.command(Command::Record);
        let (calls, taken) = (0..8)
            .map(|_| period(&mut h))
            .find(|(_, taken)| taken.is_some())
            .expect("the loop sets the tempo");
        let bpm = taken.unwrap().1.bpm;
        assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
        assert!(!h.printer.asked, "no print asked for at 100 BPM");
        assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
        // The song stayed at 100: now the loop needs its print at 100.
        period(&mut h);
        assert!(h.printer.asked, "{}", h.describe());
    }

    /// The looper on a real transport with the input closed: presses log,
    /// the phases log, and the waveform gets built.
    #[test]
    fn presses_and_changes_log_and_the_wave_is_built() {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.monitor = true;
        assert_eq!(
            h.command(Command::SetBars(Some(1))),
            "looper: first pass length 1 bar"
        );
        assert!(h
            .command(Command::Record)
            .starts_with("looper: Record: armed"));
        t.command(0, T::Play);
        let mut lines = Vec::new();
        let mut wave = None;
        for p in 0..1200u64 {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            let (peak, note) = h.period(&b, &mut l, &mut r);
            assert!(note.is_none() && peak == 0.0, "the input is off: silence");
            lines.extend(h.changes(|| format!("period {p}")));
            if let Some(w) = h.wave_step(p, 0) {
                wave = Some(w);
            }
        }
        // Armed and recording in the same period: the bar line was at once.
        assert_eq!(
            lines[0],
            "looper: recording the first pass from period 0 (1 bar)"
        );
        assert!(
            lines[1].starts_with("looper: loop of 1 bar at period 689: 88200 frames"),
            "{lines:?}"
        );
        let w = wave.expect("the wave of the (silent) loop");
        assert_eq!((w.frames, w.columns.len()), (88_200, WAVE_COLUMNS));
        let v = h.view(120.0);
        assert_eq!(
            (v.status.phase, v.status.layers, v.line_on),
            (Phase::Playing, 1, false)
        );
        assert!((v.frames_per_beat - 22_050.0).abs() < 1e-6);
        assert!(h.command(Command::Undo).contains("nothing to undo"));
        assert!(h.command(Command::Overdub).contains("overdub on (layer 2)"));
        assert!(
            h.describe().starts_with("looper: Playing, 1 layers"),
            "{}",
            h.describe()
        );
        assert!(h.describe().contains("line input off, monitor on"));
    }

    /// A layer's speaker: the mute goes to the layer by its id, the log
    /// names it, and the next view shows it muted and not heard.
    /// Listen: refused while the music plays or the loop is empty; with
    /// the music stopped it plays the picked loop, and asked again stops.
    #[test]
    fn listen_plays_the_picked_loop_while_stopped() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        assert!(h.listen(Some(None)).contains("nothing to play"));
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, t: &mut Transport, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
                h.take_pads(&l, &r);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, &mut t, 800);
        assert_eq!(h.looper.status().phase, Phase::Playing);
        assert!(h.listen(Some(None)).contains("while the music is stopped"));
        let f = t.frame();
        t.command(f, T::Stop);
        run(&mut h, &mut t, 4);
        assert!(h.listen(Some(None)).contains("Loop 1 plays on its own"));
        run(&mut h, &mut t, 4);
        assert_eq!(h.view(120.0).listening.map(|l| (l.0, l.1)), Some((0, None)));
        assert!(h.listen(Some(None)).contains("stopped"));
        assert_eq!(h.view(120.0).listening, None);
    }

    /// Length: Pattern (the default): an empty loop follows the pattern
    /// playing, a tap records exactly that many bars, and picking another
    /// Length lets go of it.
    #[test]
    fn length_pattern_follows_the_pattern_playing() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        assert!(h
            .command(Command::SetBars(Some(PATTERN_BARS)))
            .contains("first pass length"));
        assert!(h.match_pattern);
        // Nothing plays: free.
        run(&mut h, 2);
        assert_eq!(h.looper.status().bars, None);
        h.set_pattern_bars(Some(4));
        run(&mut h, 2);
        assert_eq!(h.looper.status().bars, Some(4));
        let v = h.view(120.0);
        assert!(v.match_pattern);
        assert_eq!(v.pattern_bars, Some(4));
        // Too long to follow: free.
        h.set_pattern_bars(Some(PATTERN_MAX_BARS + 1));
        run(&mut h, 2);
        assert_eq!(h.looper.status().bars, None);
        h.set_pattern_bars(Some(2));
        h.command(Command::Record);
        run(&mut h, 2);
        assert_eq!(h.looper.status().bars, Some(2));
        // Another Length lets go of the pattern.
        h.command(Command::SetBars(Some(8)));
        h.set_pattern_bars(Some(1));
        run(&mut h, 2);
        assert!(!h.match_pattern);
        assert!(!h.view(120.0).match_pattern);
    }

    /// Starts sticks from loop to loop: set on one empty loop, the next
    /// empty loop picked shows it and records with it; a loop that has
    /// sound keeps its own.
    #[test]
    fn starts_sticks_for_every_empty_loop() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, 1);
        assert_eq!(h.looper.status().starts, Starts::Bar, "on the bar at first");
        h.command(Command::SetStarts(Starts::Play));
        run(&mut h, 1);
        h.select(3);
        run(&mut h, 1);
        assert_eq!(h.looper.status().starts, Starts::Play, "loop 4 shows it");
        h.select(5);
        assert!(h.command(Command::Record).contains("starts when you play"));
        run(&mut h, 1);
        let s = h.looper.loop_status(5).unwrap();
        assert!(s.phase == Phase::Armed && s.listening, "{s:?}");
        // Back to On the bar: the next one takes that.
        h.select(6);
        h.command(Command::SetStarts(Starts::Bar));
        h.select(7);
        run(&mut h, 1);
        assert_eq!(h.looper.status().starts, Starts::Bar);
        assert_eq!(
            h.looper.loop_status(5).unwrap().starts,
            Starts::Play,
            "armed: kept"
        );
    }

    /// Records from the loops: Record on an empty loop resamples the
    /// others; on a loop that plays it still overdubs.
    #[test]
    fn records_from_the_loops_resamples_into_an_empty_loop() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, 700);
        assert_eq!(h.view(120.0).status.phase, Phase::Playing);
        h.resample = true;
        let line = h.command(Command::Record);
        assert!(line.contains("overdub"), "loop 1 has a loop: {line}");
        h.command(Command::Record);
        run(&mut h, 1);
        h.select(1);
        let line = h.command(Command::Record);
        assert!(line.contains("Resample"), "{line}");
        run(&mut h, 1);
        let v = h.view(120.0);
        assert_eq!(v.selected, 1);
        assert!(v.resample);
        assert!(v.status.resampling, "{:?}", v.status);
        assert_eq!(v.loops[0].phase, Phase::Playing);
    }

    /// A Loop cell armed on the grid records into the picked loop if it is
    /// empty, else the first empty one; with all 8 in use, or the memory
    /// full, REC says why (loop-tracks.md part 4).
    #[test]
    fn a_new_loop_goes_into_a_free_one_or_says_why_not() {
        fn record(h: &mut LoopHost, n: usize) {
            let mut t = Transport::new(44_100, 120.0).unwrap();
            let mut b = Block::new();
            t.command(0, T::Play);
            h.select(n);
            h.command(Command::Record);
            for _ in 0..2000 {
                if h.looper.loop_status(n).map(|s| s.phase) == Some(Phase::Playing) {
                    return;
                }
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
                h.take_pads(&l, &r);
                h.period(&b, &mut l, &mut r);
            }
            panic!(
                "loop {} never played: {:?}, {:?}",
                n + 1,
                h.looper.loop_status(n),
                h.looper.memory()
            );
        }
        let mut h = LoopHost::new(44_100, 60);
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        assert_eq!(h.free_loop(), Ok(0));
        record(&mut h, 0);
        assert_eq!(h.free_loop(), Ok(1), "loop 1 holds a loop");
        h.select(4);
        assert_eq!(h.free_loop(), Ok(4), "the picked one, empty");
        for n in 1..MAX_LOOPS {
            record(&mut h, n);
        }
        assert_eq!(
            h.free_loop().unwrap_err(),
            "8 loops in use: clear one on the Looper tab to record another"
        );
        // Less than a second of memory left: full.
        let mut h = LoopHost::new(44_100, 2);
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        record(&mut h, 0);
        let m = h.looper.memory();
        assert_eq!(
            h.free_loop().unwrap_err(),
            "The loops' memory is full: clear a loop on the Looper tab to record another",
            "{m:?}"
        );
    }

    /// A loop cell copied onto another Loop track (loop-tracks.md part 6):
    /// every layer of the loop copies into an empty one, a layer at a
    /// time, with the loop's level, off until launched; the copy's loop
    /// isn't free meanwhile, and a busy or full looper says why.
    #[test]
    fn a_whole_loop_copies_into_an_empty_one_a_layer_at_a_time() {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, until: &dyn Fn(&LoopHost) -> bool| {
            for _ in 0..4000 {
                if until(h) {
                    return;
                }
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
                h.take_pads(&l, &r);
                h.period(&b, &mut l, &mut r);
            }
            panic!("never: {:?}", h.looper.loop_status(3));
        };
        let st = |h: &LoopHost, n: usize| h.looper.loop_status(n).unwrap();
        // Loop 1: a first pass and an overdub, at half level.
        h.command(Command::Record);
        run(&mut h, &|h| st(h, 0).phase == Phase::Playing);
        h.command(Command::Overdub);
        run(&mut h, &|h| st(h, 0).phase == Phase::Overdubbing);
        run(&mut h, &|h| st(h, 0).position > 0.9);
        h.command(Command::Overdub);
        run(&mut h, &|h| {
            st(h, 0).phase == Phase::Playing && st(h, 0).layers == 2
        });
        h.command(Command::SetLevel(0.5));
        run(&mut h, &|h| st(h, 0).level == 0.5);

        // Refused: an empty source.
        assert!(h.copy_loop(5, 3).unwrap_err().contains("can't be copied"));

        assert_eq!(
            h.copy_loop(0, 3),
            Ok("looper: loop 1 copies into loop 4, 2 layers".to_string())
        );
        assert_eq!(
            h.copy_loop(0, 5).unwrap_err(),
            "Loop 1 is still copying into Loop 4: wait for it to finish"
        );
        assert_eq!(h.view(120.0).copy_into, Some(3));
        h.select(3);
        assert_eq!(h.free_loop(), Ok(1), "not the copy's loop, though picked");
        h.select(0);
        run(&mut h, &|h| h.copy_end.is_some());
        assert_eq!(
            h.copy_ended(),
            Some(CopyEnd {
                from: 0,
                to: 3,
                layers: 2,
                of: 2
            })
        );
        assert_eq!(h.copy_ended(), None, "once");
        let (a, c) = (st(&h, 0), st(&h, 3));
        assert_eq!(
            (c.phase, c.layers, c.ticks, c.frames),
            (Phase::Playing, 2, a.ticks, a.frames)
        );
        assert_eq!(
            (c.on, c.level),
            (false, 0.5),
            "silent until its cell is launched"
        );
        assert_eq!(h.view(120.0).copy_into, None);
        assert_eq!(h.copy_loop(0, 3).unwrap_err(), "Loop 4 isn't empty");
        for k in 0..2 {
            let (x, y) = (
                h.looper.loop_layer_info(0, k).unwrap(),
                h.looper.loop_layer_info(3, k).unwrap(),
            );
            assert_eq!((x.level, x.pan, x.muted), (y.level, y.pan, y.muted));
        }

        // The memory can't hold another copy and a second's room.
        let mut h = LoopHost::new(44_100, 3);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        t.command(0, T::Play);
        h.command(Command::Record);
        for _ in 0..4000 {
            if st(&h, 0).phase == Phase::Playing {
                break;
            }
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
        }
        assert!(
            h.copy_loop(0, 1)
                .unwrap_err()
                .starts_with("The loops' memory can't hold a copy of Loop 1 (1 layer)"),
            "{:?}",
            h.looper.memory()
        );
    }

    /// The undo history goes back and forward by as many steps as asked,
    /// as far as there are steps, and more than the command queue holds.
    #[test]
    fn the_undo_history_steps_back_and_forward() {
        use mpfree_looper::{Edit, Step};
        let mut h = LoopHost::new(44_100, 40);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                // The pads play on: an overdub of silence leaves no layer.
                h.take_pads(&[0.1; PERIOD], &[0.1; PERIOD]);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, 700);
        assert_eq!(h.view(120.0).status.phase, Phase::Playing);
        // 12 overdubs, 8 deletes and 16 edits (all the looper keeps): more
        // steps than the command queue (32) takes at once.
        for _ in 0..12 {
            h.command(Command::Overdub);
            run(&mut h, 20);
            h.command(Command::Overdub);
            run(&mut h, 20);
        }
        for _ in 0..8 {
            let id = h.view(120.0).layers[1].unwrap().id;
            h.layer_command(id, LayerCommand::Delete);
            run(&mut h, 20);
        }
        for _ in 0..16 {
            h.command(Command::Edit(Edit::Reverse));
            run(&mut h, 4);
        }
        let v = h.view(120.0);
        assert_eq!((v.history.done().len(), v.status.layers), (36, 5));
        assert_eq!(h.steps(1), "looper: undo history: nothing to go forward to");
        assert_eq!(h.steps(-34), "looper: undo history: back 34 steps");
        run(&mut h, 200);
        let v = h.view(120.0);
        assert_eq!((v.history.done().len(), v.history.redo().len()), (2, 34));
        assert!(matches!(v.history.done()[1], Step::Overdub(_)));
        assert_eq!(v.status.layers, 3);
        assert_eq!(h.steps(5), "looper: undo history: forward 5 steps");
        run(&mut h, 50);
        let v = h.view(120.0);
        assert_eq!((v.history.done().len(), v.history.redo().len()), (7, 29));
        // Back further than there are steps goes back to the first pass.
        assert_eq!(h.steps(-99), "looper: undo history: back 7 steps");
        run(&mut h, 50);
        assert_eq!(h.view(120.0).history.done().len(), 0);
        assert_eq!(h.view(120.0).status.layers, 1, "the first pass stays");
    }

    #[test]
    fn a_layer_mutes_by_its_id_and_the_view_shows_it() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                // The pads play on: an overdub of silence leaves no layer.
                h.take_pads(&[0.1; PERIOD], &[0.1; PERIOD]);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, 700);
        assert!(h.command(Command::Overdub).contains("layer 2"));
        run(&mut h, 4);
        h.command(Command::Overdub);
        run(&mut h, 2);
        let v = h.view(120.0);
        assert_eq!(v.status.layers, 2);
        let second = v.layers[1].expect("two layers");
        assert!(v.layers[2].is_none());
        assert!(!second.muted && second.audible);
        assert_eq!(
            h.layer_command(second.id, LayerCommand::Mute(true)),
            "looper: layer 2 muted"
        );
        run(&mut h, 1);
        let v = h.view(120.0);
        let (first, second) = (v.layers[0].unwrap(), v.layers[1].unwrap());
        assert!(second.muted && !second.audible);
        assert!(!first.muted && first.audible);
        assert_eq!(
            h.layer_command(second.id, LayerCommand::Mute(false)),
            "looper: layer 2 unmuted"
        );
        run(&mut h, 1);
        assert!(h.view(120.0).layers[1].unwrap().audible);
    }

    /// 13 Nov live test: the first pass's × said "deleted" and the loop
    /// stayed. The last layer's × now clears the loop; a loop card's
    /// speaker mutes that loop without picking it, and a muted loop stays
    /// silent when PLAY starts.
    #[test]
    fn the_last_layers_x_clears_and_a_loop_mutes_without_being_picked() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                // The pads play on: an overdub of silence leaves no layer.
                h.take_pads(&[0.1; PERIOD], &[0.1; PERIOD]);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, 700);
        // Loop 2 records too, and loop 1 is picked again.
        h.select(1);
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        run(&mut h, 1400);
        assert_eq!(h.view(120.0).loops[1].phase, Phase::Playing);
        h.select(0);
        run(&mut h, 1);
        // Loop 2's speaker: muted, and loop 1 stays picked.
        assert_eq!(
            h.toggle_mute(Some(1)),
            "looper: loop 2 muted: silent, PLAY doesn't start it"
        );
        run(&mut h, 2);
        let v = h.view(120.0);
        assert!(v.loops[1].muted && !v.loops[0].muted);
        assert_eq!(v.selected, 0);
        assert_eq!(
            h.toggle_mute(Some(2)),
            "looper: loop 3: empty, nothing to mute"
        );
        // The Mute button: the picked loop's own mute.
        assert!(h.toggle_mute(None).contains("loop 1 muted"));
        run(&mut h, 2);
        assert!(h.view(120.0).loops[0].muted);
        h.toggle_mute(None);
        run(&mut h, 2);
        // Loop 1 has one layer: its × clears it.
        let v = h.view(120.0);
        assert_eq!(v.status.layers, 1);
        let only = v.layers[0].unwrap();
        let line = h.layer_command(only.id, LayerCommand::Delete);
        assert_eq!(
            line,
            "looper: Clear: the loop fades out (1 layers); UNDO brings it back"
        );
        run(&mut h, 20);
        let v = h.view(120.0);
        assert_eq!(v.loops[0].phase, Phase::Empty);
        assert_eq!(v.loops[1].phase, Phase::Playing, "the other loop stays");
    }

    /// The pads' sound for song frame `n`: different every frame.
    fn pads_at(n: u64) -> (f32, f32) {
        ((n % 1000) as f32 / 2000.0, -((n % 777) as f32) / 2000.0)
    }

    /// Recording the pads: the loop plays back what the pads played in the
    /// first bar, to the frame, in the bars after it. With the source on
    /// the line input (closed), the same pads record silence.
    #[test]
    fn the_pads_are_recorded_in_time_and_the_line_source_ignores_them() {
        for source in LoopSource::ALL {
            let mut h = LoopHost::new(44_100, 20);
            let _ = h.set_source(source);
            let mut t = Transport::new(44_100, 120.0).unwrap();
            let mut b = Block::new();
            h.command(Command::SetBars(Some(1)));
            h.command(Command::SetLatency(0));
            h.command(Command::Record);
            t.command(0, T::Play);
            const BAR: u64 = 88_200;
            let mut checked = 0;
            for p in 0..(2 * BAR / PERIOD as u64 + 2) {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for f in 0..PERIOD {
                    let n = p * PERIOD as u64 + f as u64;
                    if n < BAR {
                        (l[f], r[f]) = pads_at(n);
                    }
                }
                h.take_pads(&l, &r);
                h.period(&b, &mut l, &mut r);
                // The second bar, away from the loop point's fades.
                for f in 0..PERIOD {
                    let n = p * PERIOD as u64 + f as u64;
                    if (BAR + 1_000..2 * BAR - 1_000).contains(&n) {
                        let want = if source == LoopSource::Pads {
                            pads_at(n - BAR)
                        } else {
                            (0.0, 0.0)
                        };
                        assert!(
                            (l[f] - want.0).abs() < 1e-6 && (r[f] - want.1).abs() < 1e-6,
                            "{source:?} frame {n}: {:?}, want {want:?}",
                            (l[f], r[f])
                        );
                        checked += 1;
                    }
                }
            }
            assert_eq!(checked, BAR - 2_000);
            assert_eq!(h.looper.status().phase, Phase::Playing);
            assert!(h
                .describe()
                .contains(&format!("records from {}", source.name().to_lowercase())));
        }
    }

    /// Capture: the pads played over three bars at 120 BPM; the last two
    /// whole bars become the picked empty loop (its Length), playing them
    /// in time; with it playing, the next Capture goes into a free loop.
    /// What you played shows the bar lines and what Capture takes, and
    /// the deck's button says it. A change of source forgets what was
    /// heard before.
    #[test]
    fn capture_turns_the_last_bars_played_into_a_loop() {
        const BAR: u64 = 88_200;
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetLatency(0));
        t.command(0, T::Play);
        // The song's frame is the output's length.
        let mut run = |h: &mut LoopHost, to: u64, out: &mut Vec<(f32, f32)>| {
            while (out.len() as u64) < to {
                let frame = out.len() as u64;
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for f in 0..PERIOD {
                    (l[f], r[f]) = pads_at(frame + f as u64);
                }
                h.take_pads(&l, &r);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
                out.extend(l.iter().copied().zip(r.iter().copied()));
            }
        };
        let mut out = Vec::new();
        let frame = |out: &Vec<(f32, f32)>| out.len() as u64;
        // Nothing heard on the bars yet.
        assert!(
            h.capture().contains("no whole bar heard yet"),
            "{}",
            h.capture()
        );
        // Bar 4, beat 2.
        run(&mut h, 3 * BAR + 30_000, &mut out);
        let v = h.view(120.0);
        assert_eq!(v.capture_into, Some((0, 2)), "the picked loop, its Length");
        let lines: Vec<i64> = v.heard.lines[..v.heard.n_lines]
            .iter()
            .map(|l| l.0)
            .collect();
        assert_eq!(lines, [1, 2, 3, 4], "the bar lines heard");
        let (a, b2, first, bars) = v.heard.take.expect("a take");
        assert_eq!((first, bars), (2, 2), "bars 2 and 3");
        assert!(a < b2 && b2 < 1.0);
        let button = crate::looper_page::capture_button(&v);
        assert_eq!(
            (button.label.as_str(), button.works),
            ("Capture last 2 bars", true)
        );
        assert_eq!(h.capture(), "looper: Capture: bars 2 and 3 become Loop 1");
        // It copies in at 64 times the speed it plays: about 2 800 frames.
        run(&mut h, frame(&out) + 3_200, &mut out);
        let s = h.looper.status();
        assert_eq!(
            (s.phase, s.frames, s.origin),
            (Phase::Playing, 2 * BAR as usize, 3840)
        );
        // It plays bars 2 and 3 in time: song frame n plays what the pads
        // played at the same place in those bars (past the fade in).
        let from = out.len();
        run(&mut h, frame(&out) + BAR, &mut out);
        for (n, y) in out.iter().enumerate().skip(from + 512) {
            let want = pads_at(BAR + (n as u64 - BAR) % (2 * BAR));
            assert!(
                (y.0 - want.0).abs() < 1e-6 && (y.1 - want.1).abs() < 1e-6,
                "frame {n}: {y:?}, want {want:?}"
            );
        }
        // The picked loop plays: Capture goes into Loop 2, one bar asked.
        assert!(h.select(1).contains("picked"));
        h.command(Command::SetBars(Some(1)));
        run(&mut h, frame(&out) + 256, &mut out);
        assert!(h.select(0).contains("picked"));
        run(&mut h, frame(&out) + 256, &mut out);
        assert_eq!(h.view(120.0).capture_into, Some((1, 1)));
        let said = h.capture();
        assert!(said.ends_with("become Loop 2"), "{said}");
        run(&mut h, frame(&out) + 4_096, &mut out);
        let two = h.looper.loop_status(1).unwrap();
        assert_eq!((two.phase, two.frames), (Phase::Playing, BAR as usize));
        // Into Loop 3; Record on it (its ring, REC) while it copies in
        // calls it off, as Capture again does.
        assert!(h.capture().ends_with("become Loop 3"));
        run(&mut h, frame(&out) + PERIOD as u64, &mut out);
        assert!(h.looper.loop_status(2).unwrap().capturing.is_some());
        assert_eq!(
            h.command(Command::Record),
            "looper: Capture into Loop 3 called off"
        );
        run(&mut h, frame(&out) + 4_096, &mut out);
        let three = h.looper.loop_status(2).unwrap();
        assert_eq!((three.phase, three.capturing), (Phase::Empty, None));
        // Another source: what was heard before is forgotten.
        let _ = h.set_source(LoopSource::Line);
        run(&mut h, frame(&out) + 256, &mut out);
        assert!(h.capture().contains("no whole bar heard yet"));
        assert!(h.view(120.0).heard.take.is_none());
    }

    /// What you played's window: with a 2-bar pattern it grows by two
    /// bars, and shows room for four patterns; moved back, it takes bars
    /// that are long past "the last bars", which play in time; Capture,
    /// or a new Length, lets go of it; at the newest bar line it keeps up.
    #[test]
    fn capture_window_moves_back_and_grows_by_the_pattern() {
        const BAR: u64 = 88_200;
        let mut h = LoopHost::new(44_100, 40);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetLatency(0));
        h.command(Command::SetBars(Some(PATTERN_BARS)));
        h.set_pattern_bars(Some(2));
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, to: u64, out: &mut Vec<(f32, f32)>| {
            while (out.len() as u64) < to {
                let frame = out.len() as u64;
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for f in 0..PERIOD {
                    (l[f], r[f]) = pads_at(frame + f as u64);
                }
                h.take_pads(&l, &r);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
                out.extend(l.iter().copied().zip(r.iter().copied()));
            }
        };
        let mut out = Vec::new();
        // The app follows the pattern every few periods.
        run(&mut h, 1_024, &mut out);
        h.set_pattern_bars(Some(2));
        // Bar 10, beat 2.
        run(&mut h, 9 * BAR + 30_000, &mut out);
        let v = h.view(120.0);
        assert_eq!((v.heard.unit, v.heard.pattern), (2, true));
        assert_eq!(v.capture_into, Some((0, 2)), "a pattern's length");
        let lines: Vec<i64> = v.heard.lines[..v.heard.n_lines]
            .iter()
            .map(|l| l.0)
            .collect();
        assert_eq!(
            lines,
            (3..=10).collect::<Vec<_>>(),
            "8 bars: room for 4 patterns"
        );
        // Two patterns, back to bar 8's line: bars 4 to 7.
        assert_eq!(
            h.set_capture_span(Some(8), 4),
            "looper: Capture window: bars 4 to 7"
        );
        let v = h.view(120.0);
        assert_eq!(v.capture_into, Some((0, 4)));
        let (a, b2, first, bars) = v.heard.take.expect("a take");
        assert_eq!((first, bars), (4, 4));
        assert!(0.0 < a && a < b2 && b2 < 1.0, "{a} {b2}");
        assert_eq!(h.capture(), "looper: Capture: bars 4 to 7 become Loop 1");
        assert_eq!(
            h.view(120.0).capture_into,
            Some((0, 2)),
            "let go of: the pattern again"
        );
        run(&mut h, out.len() as u64 + 8_192, &mut out);
        let s = h.looper.status();
        assert_eq!(
            (s.phase, s.frames, s.origin),
            (Phase::Playing, 4 * BAR as usize, 3 * 3840)
        );
        let from = out.len();
        run(&mut h, out.len() as u64 + BAR, &mut out);
        for (n, y) in out.iter().enumerate().skip(from + 512) {
            let want = pads_at(3 * BAR + (n as u64 - 3 * BAR) % (4 * BAR));
            assert!(
                (y.0 - want.0).abs() < 1e-6 && (y.1 - want.1).abs() < 1e-6,
                "frame {n}: {y:?}, want {want:?}"
            );
        }
        // Let go of after the Capture: Loop 2 takes the pattern again.
        h.select(1);
        run(&mut h, out.len() as u64 + 256, &mut out);
        assert_eq!(h.view(120.0).capture_into, Some((1, 2)));
        // At the newest bar line it keeps up: the last 6 bars.
        let newest = {
            let v = h.view(120.0);
            v.heard.lines[v.heard.n_lines - 1].0
        };
        assert_eq!(
            h.set_capture_span(Some(newest), 6),
            "looper: Capture window: the last 6 bars"
        );
        let first = h.view(120.0).heard.take.unwrap().2;
        run(&mut h, out.len() as u64 + BAR, &mut out);
        assert_eq!(
            h.view(120.0).heard.take.unwrap().2,
            first + 1,
            "it moves on"
        );
        // A bar line not kept leaves it; a new Length lets go of it.
        assert!(h.set_capture_span(Some(999), 2).contains("isn't kept"));
        assert_eq!(h.view(120.0).capture_into, Some((1, 6)));
        h.command(Command::SetBars(Some(1)));
        run(&mut h, out.len() as u64 + 256, &mut out);
        assert_eq!(h.view(120.0).capture_into, Some((1, 1)));
    }

    /// The pads from the mix come a period late (the looper runs before
    /// the mixer): the looper makes it up, whatever latency the app asks
    /// for (that is the line input's), so the loop is in time to the
    /// frame. The source lost is silence, never another sound, said once,
    /// and a take won't start on it (said on the screen); back again,
    /// said once.
    #[test]
    fn the_pads_from_the_mix_are_recorded_in_time_though_a_period_late() {
        let (mut h, mut t, mut b) = recorded_in_time_a_period_late(LoopSource::Pads);
        // Dry: said once, and the latency goes to 0 with it.
        let z = [0.0; PERIOD];
        let why = "the Loop track goes to a submix";
        let said = |s: &str| Some((s.to_string(), false));
        assert_eq!(
            h.take_pads_as(&z, &z, Err(why)),
            said("looper: records silence, the pads can't be recorded: the Loop track goes to a submix")
        );
        assert_eq!(h.take_pads_as(&z, &z, Err(why)), None);
        assert_eq!(h.looper.status().phase, Phase::Playing);
        let no = Some("Loop 1 doesn't record: the Loop track goes to a submix".to_string());
        assert_eq!(h.refuse(Command::Overdub), no);
        assert_eq!(h.refuse(Command::Record), no);
        assert_eq!(h.refuse(Command::Undo), None, "only what starts a take");
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        assert!(t.process(PERIOD as u32, &mut b));
        h.period(&b, &mut l, &mut r);
        assert!(t.process(PERIOD as u32, &mut b));
        h.period(&b, &mut l, &mut r);
        assert_eq!(h.looper.status().latency, 0);
        assert_eq!(
            h.take_pads_as(&z, &z, Ok(0)),
            said("looper: hears the pads again")
        );
        assert_eq!(h.refuse(Command::Overdub), None);
        assert_eq!(h.dropouts, 0, "no take recorded then");
    }

    /// Length: Pattern follows the pattern playing while the loop is
    /// empty, and holds once it is armed: a clip launched between the arm
    /// and the start doesn't change the take's length.
    #[test]
    fn length_pattern_holds_once_the_loop_is_armed() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        let mut period = |h: &mut LoopHost| {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        };
        h.command(Command::SetBars(Some(PATTERN_BARS)));
        period(&mut h);
        h.set_pattern_bars(Some(4));
        period(&mut h);
        assert_eq!(h.looper.status().bars, Some(4));
        h.set_pattern_bars(Some(2));
        period(&mut h);
        assert_eq!(h.looper.status().bars, Some(2), "empty: it follows");
        h.command(Command::Record);
        period(&mut h);
        assert_eq!(h.looper.status().phase, Phase::Armed);
        h.set_pattern_bars(Some(8));
        period(&mut h);
        assert_eq!(h.looper.status().bars, Some(2), "armed: it holds");
    }

    /// A take records one source from start to end (the design review of
    /// 20 Nov, point 6): the pads moved to another instrument, or another
    /// Records from picked, while it records wait for its end, said once
    /// on the screen too, and the Looper tab shows what this take records
    /// beside the pick; then the pick takes over and What you played
    /// starts again. A source lost while a take records is a gap of
    /// silence in it: counted, said on the screen, and its return too.
    #[test]
    fn a_take_holds_its_source_and_instrument_to_its_end() {
        use mpfree_fx::StripId;
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        assert_eq!(
            h.follow_pads(Instrument::Kit(0)),
            Some(("looper: records the pads (Kit 1)".to_string(), false))
        );
        assert_eq!(h.follow_pads(Instrument::Kit(0)), None, "said once");
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let z = [0.0; PERIOD];
        let mut period = |h: &mut LoopHost, how: Result<u32, &'static str>| {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = (z, z);
            let said = h.take_pads_as(&z, &z, how);
            h.period(&b, &mut l, &mut r);
            said
        };
        for _ in 0..4 {
            period(&mut h, Ok(0));
        }
        assert_eq!(h.looper.status().phase, Phase::Recording);
        // Play on the pads mid-take: the take keeps Kit 1.
        let held = "looper: this take keeps recording the pads (Kit 1); \
                    the pads (the synth) from the next take";
        assert_eq!(
            h.follow_pads(Instrument::Synth),
            Some((held.to_string(), true))
        );
        assert_eq!(h.follow_pads(Instrument::Synth), None, "said once");
        assert_eq!(h.takes_from(), (LoopSource::Pads, Some(Instrument::Kit(0))));
        // The master picked mid-take: the take keeps the pads too.
        let (line, tell) = h.set_source(LoopSource::Mix(StripId::Master)).unwrap();
        assert!(
            tell && line.ends_with("; the master from the next take"),
            "{line}"
        );
        assert_eq!(h.takes_from(), (LoopSource::Pads, Some(Instrument::Kit(0))));
        let v = h.view(120.0);
        assert_eq!(
            (v.source, v.taking_from),
            (LoopSource::Mix(StripId::Master), Some(LoopSource::Pads))
        );
        assert!(
            h.set_source(LoopSource::Pads).unwrap().1,
            "back: said again"
        );
        assert_eq!(h.view(120.0).taking_from, None, "the same source");
        // The channel lost mid-take: a gap of silence, said and counted.
        let why = "that strip isn't in the mix";
        assert_eq!(
            period(&mut h, Err(why)),
            Some((
                "looper: the take records silence from here, the pads (Kit 1) \
                 can't be recorded: that strip isn't in the mix"
                    .to_string(),
                true
            ))
        );
        assert_eq!(period(&mut h, Err(why)), None, "said once");
        assert_eq!(h.dropouts, 1);
        assert_eq!(
            period(&mut h, Ok(0)),
            Some((
                "looper: the take hears the pads (Kit 1) again".to_string(),
                true
            ))
        );
        assert!(h.describe().contains("1 times a take's source was lost"));
        for _ in 0..(88_200 / PERIOD + 2) {
            period(&mut h, Ok(0));
        }
        assert_eq!(h.looper.status().phase, Phase::Playing);
        // The take ended: the synth takes over.
        assert_eq!(
            h.follow_pads(Instrument::Synth),
            Some((
                "looper: records the pads (the synth), What you played starts again".to_string(),
                false
            ))
        );
        assert_eq!(h.takes_from(), (LoopSource::Pads, Some(Instrument::Synth)));
        // Records from picked between takes changes at once.
        let (line, tell) = h.set_source(LoopSource::Line).unwrap();
        assert!(
            !tell && line.starts_with("looper: records the line input"),
            "{line}"
        );
        assert_eq!(h.takes_from(), (LoopSource::Line, None));
    }

    /// The pads named in the log, silence on MIDI out (said
    /// once, not again as a mix lost), and nothing while another source
    /// records.
    #[test]
    fn pads_say_which_instrument_they_record_and_silence_on_midi_out() {
        let mut h = LoopHost::new(44_100, 8);
        assert_eq!(
            h.follow_pads(Instrument::Own(2)).map(|s| s.0).as_deref(),
            Some("looper: records the pads (Track 3's own instrument)")
        );
        assert_eq!(
            h.follow_pads(Instrument::MidiOut).map(|s| s.0).as_deref(),
            Some(
                "looper: records silence (the pads play MIDI out, which sounds outside mpfree), \
                 What you played starts again"
            )
        );
        let z = [0f32; PERIOD];
        assert_eq!(h.take_pads_as(&z, &z, Err(PADS_ON_MIDI_OUT)), None);
        let _ = h.set_source(LoopSource::Line);
        assert_eq!(
            h.follow_pads(Instrument::Synth),
            None,
            "another source: nothing to say"
        );
        assert_eq!(h.takes_from(), (LoopSource::Line, None));
    }

    #[test]
    fn a_mix_is_recorded_in_time_like_the_pads() {
        let strip = mpfree_fx::StripId::Bus(0);
        let (mut h, _, _) = recorded_in_time_a_period_late(LoopSource::Mix(strip));
        assert_eq!(h.takes_from(), (LoopSource::Mix(strip), None));
        assert_eq!(h.first_note(), "(the first pad you play)");
        let z = [0.0; PERIOD];
        let why = "that strip isn't in the mix";
        let name = crate::mixing::name(strip);
        assert_eq!(
            h.take_pads_as(&z, &z, Err(why)).map(|s| s.0),
            Some(format!(
                "looper: records silence, {name} can't be recorded: {why}"
            ))
        );
        assert_eq!(
            h.take_pads_as(&z, &z, Ok(0)).map(|s| s.0),
            Some(format!("looper: hears {name} again"))
        );
    }

    /// A loop of a bar recorded from `source` (the pads or a mix), handed
    /// over a period late as the mixer has it: the loop plays it back in
    /// time to the frame. The host, the transport and its block, for more.
    fn recorded_in_time_a_period_late(source: LoopSource) -> (LoopHost, Transport, Block) {
        let mut h = LoopHost::new(44_100, 20);
        let _ = h.set_source(source);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        // The app's latency on the pads: the pads' own stands.
        h.command(Command::SetLatency(512));
        h.command(Command::Record);
        t.command(0, T::Play);
        const BAR: u64 = 88_200;
        let late = PERIOD as u64;
        let mut checked = 0;
        for p in 0..(2 * BAR / PERIOD as u64 + 2) {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for f in 0..PERIOD {
                let n = (p * PERIOD as u64 + f as u64).checked_sub(late);
                if let Some(n) = n.filter(|&n| n < BAR) {
                    (l[f], r[f]) = pads_at(n);
                }
            }
            assert_eq!(h.take_pads_as(&l, &r, Ok(0)), None, "the mix all along");
            h.period(&b, &mut l, &mut r);
            for f in 0..PERIOD {
                let n = p * PERIOD as u64 + f as u64;
                if (BAR + 1_000..2 * BAR - 1_000).contains(&n) {
                    let want = pads_at(n - BAR);
                    assert!(
                        (l[f] - want.0).abs() < 1e-6 && (r[f] - want.1).abs() < 1e-6,
                        "frame {n}: {:?}, want {want:?}",
                        (l[f], r[f])
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, BAR - 2_000);
        assert_eq!(h.looper.status().latency, PERIOD as u32);
        (h, t, b)
    }

    /// The audio thread's part: handing over the pads, a period, the log
    /// check and the wave step allocate nothing while nothing changes.
    /// A loop recorded at 120 BPM, then the tempo at 100: it plays like
    /// tape (lower) until the worker's print is in, then at its own pitch
    /// and the new length; Overdub then fits it to the tempo.
    #[test]
    fn a_loop_at_another_tempo_gets_a_print_and_keeps_its_pitch() {
        let mut h = LoopHost::new(44_100, 30);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        // 441 Hz: 100 frames a cycle, 882 to the bar.
        let sine = |f: u64| 0.5 * (f as f64 * std::f64::consts::TAU / 100.0).sin() as f32;
        let mut f = 0u64;
        let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, rec: bool| {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            if rec {
                for i in 0..PERIOD {
                    l[i] = sine(f + i as u64);
                    r[i] = l[i];
                }
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            f += PERIOD as u64;
            l
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b, true);
        }
        assert_eq!(h.looper.status().frames, 88_200);
        t.command(t.frame(), T::SetTempo(100.0));
        // The pitch, from the first and last upward zero crossing.
        let ups = |out: &[f32]| {
            let x: Vec<usize> = (1..out.len())
                .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
                .collect();
            (x.len() - 1) as f64 * 44_100.0 / (x[x.len() - 1] - x[0]) as f64
        };
        // Like tape at first, once the tempo has changed (on the next 16th,
        // up to 5,513 frames on).
        for _ in 0..50 {
            period(&mut h, &mut t, &mut b, false);
        }
        let mut out = Vec::new();
        for _ in 0..30 {
            out.extend(period(&mut h, &mut t, &mut b, false));
        }
        assert!(!h.looper.status().printed);
        let tape = ups(&out);
        assert!((tape - 367.5).abs() < 2.0, "tape {tape}");
        let start = std::time::Instant::now();
        while !h.looper.status().printed {
            assert!(
                start.elapsed().as_secs() < 120,
                "no print: {}",
                h.describe()
            );
            period(&mut h, &mut t, &mut b, false);
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        let mut out = Vec::new();
        for _ in 0..400 {
            out.extend(period(&mut h, &mut t, &mut b, false));
        }
        let kept = ups(&out[1_000..]);
        assert!((kept - 441.0).abs() < 2.0, "print {kept}");
        assert!(h.describe().contains("a print plays"), "{}", h.describe());
        assert!(h.describe().contains("prints 1 made"), "{}", h.describe());
        // Overdub fits the loop to the tempo first.
        assert!(h
            .command(Command::Overdub)
            .contains("once the loop is fitted"));
        for _ in 0..40 {
            period(&mut h, &mut t, &mut b, false);
        }
        let s = h.looper.status();
        assert_eq!((s.phase, s.frames), (Phase::Overdubbing, 105_840));
    }

    /// The opening thread's own counts from the kernel (Linux; None
    /// elsewhere): its voluntary context switches, the times it slept,
    /// waiting for something (in /proc/thread-self/status; a thread taken
    /// off its core for another is not counted), and its page faults,
    /// minor and major (in /proc/thread-self/stat). The files stay open,
    /// so a read is one seek and one read, no path to look up.
    struct Counts {
        status: Option<std::fs::File>,
        stat: Option<std::fs::File>,
        text: String,
    }

    impl Counts {
        fn open() -> Self {
            let open = |p: &str| std::fs::File::open(p).ok();
            Counts {
                status: open("/proc/thread-self/status"),
                stat: open("/proc/thread-self/stat"),
                text: String::with_capacity(4096),
            }
        }

        fn fill(f: &mut std::fs::File, text: &mut String) -> Option<()> {
            use std::io::{Read, Seek, SeekFrom};
            text.clear();
            f.seek(SeekFrom::Start(0)).ok()?;
            f.read_to_string(text).ok().map(|_| ())
        }

        fn sleeps(&mut self) -> Option<u64> {
            Self::fill(self.status.as_mut()?, &mut self.text)?;
            self.text
                .lines()
                .find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))?
                .trim()
                .parse()
                .ok()
        }

        /// Fields 10 and 12 of stat (minflt, majflt), counted from the
        /// state after the command's name in parentheses (field 3).
        fn faults(&mut self) -> Option<u64> {
            Self::fill(self.stat.as_mut()?, &mut self.text)?;
            let mut f = self.text[self.text.rfind(')')? + 1..].split_whitespace();
            let minor: u64 = f.nth(7)?.parse().ok()?;
            let major: u64 = f.nth(1)?.parse().ok()?;
            Some(minor + major)
        }
    }

    /// Smart loop end step 3, Auto by ear while long prints run: a print
    /// of a 32 s loop runs over and over on up to four cores at the print
    /// worker's idle priority while a free first loop records two bars of
    /// a beat from the mix and closes on Auto, the periods paced as they
    /// play. The finder, on its own thread at the normal priority, answers
    /// in time and the loop closes on the downbeat it heard, or Auto keeps
    /// your tap (counted, and said). Either way no period waits for it:
    /// the looper counts no ask that waited, a period that speaks to the
    /// finder never sleeps (its thread's voluntary context switches,
    /// which a busy machine taking its core away doesn't add to; judged
    /// natively, see below), and no period allocates.
    #[test]
    fn auto_by_ear_while_long_prints_run_never_waits_for_the_finder() {
        by_ear_while_long_prints_run(true);
    }

    /// The same with a finder that never answers, as on a machine too slow
    /// for it: Auto keeps your tap at the end of its wait, says why, and
    /// starts the song at the tap's tempo; no period sleeps or allocates.
    #[test]
    fn auto_by_ear_keeps_the_tap_when_the_finder_is_too_slow_while_long_prints_run() {
        by_ear_while_long_prints_run(false);
    }

    /// The two tests above: `answers` false leaves the finder's answers
    /// unread (they never come back).
    fn by_ear_while_long_prints_run(answers: bool) {
        use mpfree_fx::StripId;
        use mpfree_looper::hits::kit::Kit;
        use mpfree_looper::Bounce;
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::time::{Duration, Instant};
        const RATE: u32 = 44_100;
        const TWO_BARS: u64 = 176_400;
        // The prints: 16 bars at 120 printed at 100, a chunk at a time as
        // the worker runs them, counted.
        let stop = Arc::new(AtomicBool::new(false));
        let chunks = Arc::new(AtomicU64::new(0));
        let frames = 32 * RATE as usize;
        let audio: Vec<f32> = (0..2 * frames)
            .map(|i| 0.3 * ((i / 2) as f64 * std::f64::consts::TAU / 100.0).sin() as f32)
            .collect();
        let bounce = Arc::new(Bounce {
            from_loop: 0,
            audio,
            frames,
            ticks: 64 * i64::from(PPQ),
            origin: 0,
            complete: true,
            version: 1,
            dry: true,
            rate: 0,
        });
        // (Well under the cores: the test suite runs beside it.)
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        let printers: Vec<_> = (0..(cores / 4).clamp(1, 4))
            .map(|_| {
                let (stop, chunks, bounce) = (stop.clone(), chunks.clone(), bounce.clone());
                std::thread::spawn(move || {
                    mpfree_lanes::os::idle_job();
                    while !stop.load(Ordering::Relaxed) {
                        let mut j = PrintJob::new(&bounce, RATE, frames * 6 / 5).unwrap();
                        while !stop.load(Ordering::Relaxed) && !j.run(1 << 15) {
                            chunks.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();
        // The prints are under way before REC. (Under qemu, or with every
        // core busy, a print's first chunk can take seconds.)
        let wait = Instant::now();
        while chunks.load(Ordering::Relaxed) == 0 {
            assert!(wait.elapsed() < Duration::from_secs(120), "no print ran");
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut h = LoopHost::new(RATE, 20);
        assert!(!h.finder.inline && !h.finder.gone, "its thread");
        // (The answers left unread: the finder's thread goes on, as a slow
        // one would, and none comes back.)
        let _unread = (!answers).then(|| h.finder.back.take());
        // (At the beat's tempo: when the finder answers, the period that
        // closes the loop wants no print.)
        let mut t = Transport::new(RATE, 120.0).unwrap();
        let mut b = Block::new();
        let _ = h.set_source(LoopSource::Mix(StripId::Master));
        h.set_tempo_from_loop(true);
        h.set_smart_end(SmartEnd::Auto);
        h.command(Command::SetBars(None));
        let period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, l: &[f32; PERIOD]| {
            assert!(t.process(PERIOD as u32, b));
            h.take_pads_as(l, l, Ok(0));
            let (mut ol, mut or) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut ol, &mut or);
        };
        period(&mut h, &mut t, &mut b, &[0.0; PERIOD]);
        // Two bars of the beat at 120, 80 ms after REC; REC again 40 ms
        // after its next downbeat.
        h.command(Command::Record);
        let rec = t.frame();
        let early = 3_528u64;
        let mut audio = vec![0.0f32; 2 * (TWO_BARS as usize + RATE as usize)];
        Kit::new(RATE).beat(&mut audio, early as usize, 120.0, 2, &|_| 0.0);
        let tap = rec + early + TWO_BARS + 1_764;
        let (mut tapped, mut printing_at_ask, mut started, mut rec_from) =
            (false, None, None, None);
        let (mut took, mut allocs) = (Vec::new(), 0);
        // Sleeps inside the periods, with their page faults: in those
        // that spoke to the finder (asked it, or looked for its answer)
        // and in the others. And outside them (the pacing: the count is
        // read for real).
        let mut counts = Counts::open();
        let (mut near, mut away, mut read, mut slept) = (Vec::new(), Vec::new(), true, 0u64);
        let (mut faulting, mut spoke) = ((0u32, 0u64), 0u32);
        let mut last = counts.sleeps();
        let beat = Duration::from_secs_f64(PERIOD as f64 / f64::from(RATE));
        let t0 = Instant::now();
        let mut p = 0u32;
        while t.frame() < tap + RATE as u64 / 2 {
            std::thread::sleep((t0 + beat * p).saturating_duration_since(Instant::now()));
            p += 1;
            if !tapped && t.frame() + PERIOD as u64 > tap {
                h.command(Command::Record);
                tapped = true;
            }
            let mut l = [0.0; PERIOD];
            for (i, x) in l.iter_mut().enumerate() {
                let at = (t.frame() - rec) as usize + i;
                *x = audio.get(2 * at).copied().unwrap_or(0.0);
            }
            let (now, from) = (Instant::now(), t.frame());
            let out = h.finder.job.is_none();
            let faults = counts.faults();
            let before = counts.sleeps();
            let ((), calls) = crate::sys::alloc_count::calls(|| period(&mut h, &mut t, &mut b, &l));
            let after = counts.sleeps();
            let faulted = counts.faults();
            allocs += calls;
            match (last, before, after, faults, faulted) {
                (Some(l), Some(b), Some(a), Some(f), Some(g)) => {
                    slept += b - l;
                    if g > f {
                        faulting = (faulting.0 + 1, faulting.1 + (g - f));
                    }
                    // (It sent the job, or looked for the answer: the
                    // job out, and its answers read.)
                    let finder =
                        (!out && h.finder.job.is_none()) || (out && h.finder.back.is_some());
                    spoke += u32::from(finder);
                    if a > b {
                        (if finder { &mut near } else { &mut away }).push((p, a - b, g - f));
                    }
                }
                _ => read = false,
            }
            last = after;
            if let Some((_, r, at, into)) = h.take_tempo() {
                let pos = t.position();
                let start = pos - pos.rem_euclid(t.meter().bar_ticks()) + into;
                t.command(at, T::Locate(start));
                t.command(at, T::SetTempo(r.bpm));
                t.command(at, T::Play);
                started = Some(r.bpm);
            }
            took.push(now.elapsed());
            if rec_from.is_none() && h.looper.status().phase == Phase::Recording {
                rec_from = Some(from);
            }
            if printing_at_ask.is_none() && h.finder.job.is_none() {
                printing_at_ask = Some(chunks.load(Ordering::Relaxed));
            }
        }
        let asked = printing_at_ask.expect("the finder was asked");
        // (A print goes on after the ask. On a starved machine its next
        // chunk may come after the pass.)
        let wait = Instant::now();
        while chunks.load(Ordering::Relaxed) <= asked && wait.elapsed() < Duration::from_secs(120) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let after = chunks.load(Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
        for p in printers {
            p.join().unwrap();
        }
        assert!(
            after > asked && asked > 0,
            "the prints ran on: {asked}, then {after}"
        );
        assert!(started.is_some(), "the song started: {}", h.describe());
        let st = h.looper.stats();
        let s = h.looper.status();
        let (line, words) = h.take_smart().expect("a word on the tap");
        if !answers {
            assert_eq!((st.finds_answered, st.finds_late), (0, 1), "{line}");
        }
        if st.finds_answered == 1 {
            assert_eq!(s.frames as u64, TWO_BARS, "{line}");
            assert_eq!(started, Some(120.0));
            // It started on the first kick: a mix comes a period late (the
            // mixer's period), so the kick handed over on a frame was
            // played a period before it.
            let kick = (rec + early - PERIOD as u64 - rec_from.unwrap()) as i64;
            assert_eq!(s.moved.map(|m| m.start), Some(kick), "{line}");
        } else {
            // Too slow a machine: the tap stands, counted and said, and
            // the song starts at its tempo (two bars and the 40 ms late,
            // less the 80 ms before the first kick).
            assert_eq!(st.finds_late, 1, "{line}");
            assert_eq!(
                words.as_deref(),
                Some("Kept your tap: it couldn't listen in time.")
            );
            assert_ne!(started, Some(120.0), "{line}");
        }
        took.sort();
        let worst = *took.last().unwrap();
        // Waiting on the finder would show in the looper's own count, and
        // would sleep in a period that spoke to it: none did. A period
        // away from the finder can sleep on a loaded machine with nothing
        // of ours to wait on: on CI, with 36 app tests at once, one did,
        // early in the recording, long before the ask. (The kernel can
        // stop a thread on its memory, for one: a page it is moving, or
        // another thread's hold on the process's memory map, both through
        // a page fault.) Those are printed with their page faults, not
        // judged. And the periods that spoke to the finder are not judged
        // as 32-bit ARM, which runs under qemu (CI's ARM job): there the
        // count is that of qemu's own thread, which sleeps on qemu's locks
        // while it translates code the period runs for the first time.
        // The loop's closing periods slept there, though they made no
        // blocking call (qemu's syscall trace), and the same pass again in
        // the same program slept in none.
        assert_eq!(
            h.finder.waited,
            0,
            "periods an ask waited for the finder: {}",
            h.describe()
        );
        if read {
            assert!(slept > 0, "the pacing sleeps, so the count moves");
            if !away.is_empty() {
                eprintln!(
                    "periods away from the finder that slept (period, times, page faults), \
                     not judged: {away:?} of {p}"
                );
            }
            if cfg!(target_arch = "arm") {
                eprintln!(
                    "periods that spoke to the finder and slept under qemu, not judged: {near:?}"
                );
            } else {
                assert!(
                    near.is_empty(),
                    "periods that spoke to the finder and slept (period, times, page faults): \
                     {near:?} of {p}: {}",
                    h.describe()
                );
            }
        }
        eprintln!(
            "periods: 99th {:?}, 99.9th {:?}, worst {worst:?}; {spoke} of {p} spoke to the finder, \
             {} took {} page faults; {line}",
            took[took.len() * 99 / 100],
            took[took.len() * 999 / 1000],
            faulting.0,
            faulting.1
        );
        assert_eq!(allocs, 0, "allocations or frees on the audio thread's path");
        assert!(h.describe().contains("; by ear: "), "{}", h.describe());
    }

    /// Half speed at the tempo it was recorded at: like tape at once (an
    /// octave down), then the worker's print (the octave made cleanly);
    /// Overdub waits for it, fits the loop to it and records.
    #[test]
    fn half_speed_gets_a_print_an_octave_down_and_an_overdub_fits_to_it() {
        let mut h = LoopHost::new(44_100, 30);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let sine = |f: u64| 0.5 * (f as f64 * std::f64::consts::TAU / 100.0).sin() as f32;
        let mut f = 0u64;
        let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, rec: bool| {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            if rec {
                for i in 0..PERIOD {
                    l[i] = sine(f + i as u64);
                    r[i] = l[i];
                }
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            f += PERIOD as u64;
            l
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b, true);
        }
        let line = h.command(Command::Edit(Edit::Speed(-1)));
        assert_eq!(
            line,
            "looper: speed ½×: an octave down, twice the bars (Undo takes it back)"
        );
        let ups = |out: &[f32]| {
            let x: Vec<usize> = (1..out.len())
                .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
                .collect();
            (x.len() - 1) as f64 * 44_100.0 / (x[x.len() - 1] - x[0]) as f64
        };
        // Overdub at once: it waits for the print.
        for _ in 0..4 {
            period(&mut h, &mut t, &mut b, false);
        }
        assert!(h
            .command(Command::Overdub)
            .contains("once the loop's print at ½× is in"));
        let start = std::time::Instant::now();
        while h.looper.status().phase != Phase::Overdubbing {
            assert!(start.elapsed().as_secs() < 120, "no fit: {}", h.describe());
            period(&mut h, &mut t, &mut b, false);
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        let s = h.looper.status();
        assert_eq!((s.frames, s.speed, s.rate), (176_400, -1, 0));
        // The fitted layer plays the octave down, sample for sample.
        h.command(Command::Overdub);
        let mut out = Vec::new();
        for _ in 0..400 {
            out.extend(period(&mut h, &mut t, &mut b, false));
        }
        let down = ups(&out[1_000..]);
        assert!((down - 220.5).abs() < 2.0, "fitted {down}");
    }

    #[test]
    fn a_steady_period_never_allocates() {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let (mut l, mut r) = ([0.25; PERIOD], [-0.25; PERIOD]);
        // Record the bar and build its wave first.
        for p in 0..1_400u64 {
            assert!(t.process(PERIOD as u32, &mut b));
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
            h.changes(|| format!("period {p}"));
            h.wave_step(p, 0);
        }
        assert_eq!(h.looper.status().phase, Phase::Playing);
        let ((), calls) = crate::sys::alloc_count::calls(|| {
            for p in 1_400..1_800u64 {
                t.process(PERIOD as u32, &mut b);
                h.take_pads(&l, &r);
                h.period(&b, &mut l, &mut r);
                assert!(h.changes(|| format!("period {p}")).is_empty());
                assert!(h.wave_step(p, 0).is_none());
                h.view(120.0);
            }
        });
        assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
    }

    #[test]
    fn the_latency_estimate_is_the_output_buffer_and_one_period() {
        assert_eq!(estimated_latency(3), 512);
        assert_eq!(estimated_latency(2), 384);
    }

    /// A host with a one-bar loop of two layers (the second at a quarter
    /// level), playing at 120 BPM.
    fn saving_host() -> (LoopHost, Transport, Block) {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut f = 0u64;
        let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, hz: f64| {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for i in 0..PERIOD {
                l[i] = 0.3
                    * ((f + i as u64) as f64 * hz * std::f64::consts::TAU / 44_100.0).sin() as f32;
                r[i] = -l[i];
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            f += PERIOD as u64;
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b, 441.0);
        }
        h.command(Command::Overdub);
        for _ in 0..200 {
            period(&mut h, &mut t, &mut b, 882.0);
        }
        h.command(Command::Overdub);
        for _ in 0..800 {
            period(&mut h, &mut t, &mut b, 0.0);
        }
        let id = h.looper.layer_info(1).unwrap().id;
        h.looper.layer_command(id, LayerCommand::Level(0.25));
        (h, t, b)
    }

    /// `periods` periods with no input: the output, interleaved.
    fn run_host(h: &mut LoopHost, t: &mut Transport, b: &mut Block, periods: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    /// Run until the save is done.
    fn saved_from(h: &mut LoopHost, t: &mut Transport, b: &mut Block) -> Saved {
        let start = std::time::Instant::now();
        loop {
            run_host(h, t, b, 1);
            if let Some(s) = h.take_saved() {
                return s;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60, "the save did not finish");
        }
    }

    /// A layer dragged onto another loop: moved into an empty loop, a
    /// loop of its own, and Undo there takes it back to where it came
    /// from, saying so; a loop of another length says why it can't take
    /// it; a loop's only layer goes as a copy.
    #[test]
    fn a_dragged_layer_moves_between_loops_and_undo_says_so() {
        let (mut h, mut t, mut b) = saving_host();
        run_host(&mut h, &mut t, &mut b, 20);
        let over = h.looper.layer_info(1).unwrap().id;
        assert!(h
            .copy_layer(0, over, 0, false)
            .contains("is in loop 1 already"));
        let said = h.copy_layer(0, over, 2, false);
        assert_eq!(
            said,
            "looper: Layer 2 moves into loop 3, a loop of its own (UNDO takes it back)"
        );
        run_host(&mut h, &mut t, &mut b, 400);
        let (a, c) = (
            h.looper.loop_status(0).unwrap(),
            h.looper.loop_status(2).unwrap(),
        );
        assert_eq!((a.layers, c.phase, c.layers), (1, Phase::Playing, 1));
        assert_eq!(c.ticks, a.ticks);
        // Undo on loop 3 puts it back in loop 1, and loop 3 clears.
        h.select(2);
        run_host(&mut h, &mut t, &mut b, 2);
        assert_eq!(
            h.command(Command::Undo),
            "looper: Undo: the layer goes back to loop 1, and this loop clears"
        );
        run_host(&mut h, &mut t, &mut b, 40);
        assert_eq!(h.looper.loop_status(2).unwrap().phase, Phase::Empty);
        assert_eq!(h.looper.loop_status(0).unwrap().layers, 2);
        // Loop 2: two bars. Loop 1's layer can't go onto it.
        h.select(1);
        h.command(Command::SetBars(Some(2)));
        h.command(Command::Record);
        while h.looper.loop_status(1).unwrap().phase != Phase::Playing {
            run_host(&mut h, &mut t, &mut b, 10);
        }
        assert_eq!(
            h.copy_layer(0, over, 1, true),
            "looper: loop 2 is 2 bars and Layer 2 1 bar: a layer goes only onto a loop of its length"
        );
        // Its only layer goes into loop 3 as a copy: loop 2 keeps it.
        let first = h.looper.loop_layer_info(1, 0).unwrap().id;
        assert!(h
            .copy_layer(1, first, 2, false)
            .contains("it has one layer, so loop 2 keeps it"));
        run_host(&mut h, &mut t, &mut b, 800);
        assert_eq!(h.looper.loop_status(1).unwrap().layers, 1);
        assert_eq!(h.looper.loop_status(2).unwrap().layers, 1);
    }

    #[test]
    fn loops_saved_with_the_project_load_back_and_play_the_same() {
        use mpfree_looper::save::{from_text, from_wav, to_text, wav};
        let (mut h, mut t, mut b) = saving_host();
        assert_eq!(h.save(), "looper: saving the loops");
        let mut saved = saved_from(&mut h, &mut t, &mut b);
        assert!(h.saving().is_none());
        assert_eq!(saved.loops.len(), 1);
        assert_eq!(saved.loops[0].1.layers.len(), 2);
        assert_eq!(saved.loops[0].1.layers[1].level, 0.25);
        // As the project keeps them: the text, and a WAV a layer.
        let mut files = Vec::new();
        for (n, f) in &mut saved.loops {
            for (k, y) in f.layers.iter_mut().enumerate() {
                y.file = format!("loop {} layer {}.wav", *n + 1, k + 1);
                files.push((y.file.clone(), wav(&y.audio[..2 * f.frames], saved.rate)));
            }
        }
        let mut back = from_text(&to_text(&saved)).unwrap();
        for (_, f) in &mut back.loops {
            for y in &mut f.layers {
                let bytes = &files.iter().find(|(n, _)| *n == y.file).unwrap().1;
                let (audio, rate) = from_wav(bytes).unwrap();
                assert_eq!(rate, 44_100);
                y.audio = audio;
            }
        }
        for ((_, a), (_, c)) in saved.loops.iter().zip(&back.loops) {
            assert_eq!(a.frames, c.frames);
            for (x, y) in a.layers.iter().zip(&c.layers) {
                assert_eq!(x.audio[..2 * a.frames], y.audio[..]);
                assert_eq!((x.level, &x.file), (y.level, &y.file));
            }
        }
        // Into a fresh host at the same song frame: it plays as the first
        // one does, a bar on.
        let (mut h2, mut t2) = (
            LoopHost::new(44_100, 20),
            Transport::new(44_100, 120.0).unwrap(),
        );
        let mut b2 = Block::new();
        t2.command(0, T::Play);
        let skip = (t.frame() / PERIOD as u64) as usize;
        run_host(&mut h2, &mut t2, &mut b2, skip);
        assert_eq!(t2.frame(), t.frame());
        assert!(h2.load(back).starts_with("looper: loading loops 1"));
        let a = run_host(&mut h, &mut t, &mut b, 1500);
        let c = run_host(&mut h2, &mut t2, &mut b2, 1500);
        assert_eq!(h2.looper.status().phase, Phase::Playing);
        let n = a.len();
        let worst = (n - 2 * 88_200..n)
            .map(|i| (a[i] - c[i]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "differs by {worst}");
    }

    /// Layers as recordings (audio tracks phase 4c.2, behind Dev's "Loops
    /// on audio tracks"): with recordings on, each closed layer gets a
    /// recording once; a save keeps each loop as the looper holds it
    /// (`mpfree-looper 2`), each layer with its recording and its file
    /// named by it; it opens in a fresh host playing the same, and keeps
    /// its recordings, so saved again it names the same files and nothing
    /// is written twice.
    #[test]
    fn loops_saved_as_recordings_open_the_same_and_keep_their_recordings() {
        use mpfree_looper::save::{from_text, wav};
        let (mut h, mut t, mut b) = saving_host();
        h.set_recordings(true);
        let mut next = 0u128;
        let mut fresh = || {
            next += 1;
            0xfeed_0000 + next
        };
        h.give_layer_recs(&mut fresh);
        h.give_layer_recs(&mut fresh);
        assert_eq!(next, 2, "a recording a layer, once");
        h.save();
        let saved = saved_from(&mut h, &mut t, &mut b);
        let (text, files, copies) = crate::project::loops_part(saved);
        assert!(copies.is_empty(), "no file is written here");
        assert!(text.starts_with("mpfree-looper 2\n"), "{text}");
        let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(
            names,
            [
                format!("loop_{:032x}.wav", 0xfeed_0001u128),
                format!("loop_{:032x}.wav", 0xfeed_0002u128)
            ]
        );
        // The first pass's audio as written: more than its loop.
        let frames = from_text(&text).unwrap().loops[0].1.frames;
        assert!(files[0].1.len() > 2 * frames);
        let wavs: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(n, a)| (n.clone(), wav(a, 44_100)))
            .collect();
        let read = |name: &str| {
            wavs.iter()
                .find(|(n, _)| n == name)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| format!("no {name}"))
        };
        let (back, notes, left) = crate::project::loops_open(&text, read).unwrap();
        assert!(notes.is_empty() && left.is_empty(), "{notes:?}");
        // Into a fresh host at the same song frame: it plays as the first
        // one does, a bar on.
        let (mut h2, mut t2) = (
            LoopHost::new(44_100, 20),
            Transport::new(44_100, 120.0).unwrap(),
        );
        h2.set_recordings(true);
        let mut b2 = Block::new();
        t2.command(0, T::Play);
        let skip = (t.frame() / PERIOD as u64) as usize;
        run_host(&mut h2, &mut t2, &mut b2, skip);
        assert!(h2.load(back).starts_with("looper: loading loops 1"));
        let a = run_host(&mut h, &mut t, &mut b, 1500);
        let c = run_host(&mut h2, &mut t2, &mut b2, 1500);
        let n = a.len();
        let worst = (n - 2 * 88_200..n)
            .map(|i| (a[i] - c[i]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "differs by {worst}");
        // Its layers keep the recordings they opened with: saved again,
        // the same files.
        h2.give_layer_recs(|| panic!("a layer opened with its recording got another"));
        h2.save();
        let again = saved_from(&mut h2, &mut t2, &mut b2);
        // Opened from the project, their files are there: a save copies
        // them (4c.4), reading none of their audio.
        let (text2, files2, copies2) = crate::project::loops_part(again);
        assert!(files2.is_empty());
        assert_eq!(copies2, names);
        assert_eq!(
            from_text(&text2).unwrap().loops,
            from_text(&text).unwrap().loops
        );
        // With recordings off, a save is as before: each layer as it plays.
        h2.set_recordings(false);
        h2.save();
        let (text3, _, _) = crate::project::loops_part(saved_from(&mut h2, &mut t2, &mut b2));
        assert!(text3.starts_with("mpfree-looper 1\n"), "{text3}");
    }

    /// Phase 4c.4: with recordings on, a save is a snapshot. The loops
    /// are kept as they were when it began, each layer read by its
    /// number, while the loop goes on: a level turned and an overdub
    /// started during the save start nothing again, and it gives the same
    /// text and audio as a save with nothing changing.
    #[test]
    fn a_snapshot_save_keeps_the_loops_as_they_were_while_they_change() {
        let (mut h, mut t, mut b) = saving_host();
        h.set_recordings(true);
        let mut next = 0u128;
        h.give_layer_recs(|| {
            next += 1;
            0xfeed_0000 + next
        });
        h.save();
        let (text, files, copies) = crate::project::loops_part(saved_from(&mut h, &mut t, &mut b));
        assert!(copies.is_empty(), "no file is written here");
        let restarts = h.save_restarts;
        h.save();
        // Into the reading, then the loop changes.
        let start = std::time::Instant::now();
        while !h.saving().is_some_and(|p| p > 0.0) {
            run_host(&mut h, &mut t, &mut b, 1);
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60, "the save never read");
        }
        let before = h.looper.loop_status(0).unwrap().version;
        let id = h.looper.layer_info(1).unwrap().id;
        h.looper.layer_command(id, LayerCommand::Level(0.75));
        h.command(Command::Overdub);
        let moved = saved_from(&mut h, &mut t, &mut b);
        assert_ne!(h.looper.loop_status(0).unwrap().version, before);
        assert_eq!(h.save_restarts, restarts, "nothing started again");
        let (text2, files2, _) = crate::project::loops_part(moved);
        assert_eq!(text2, text);
        assert!(files2 == files, "the same audio");
    }

    /// Phase 4c.5: a merged layer is a recording of its own that names
    /// the recordings it was made from (`from=`), and keeps naming them
    /// once opened from the project and saved again.
    #[test]
    fn a_merged_layer_names_the_recordings_it_was_made_from() {
        use mpfree_looper::save::{from_text, wav};
        let (mut h, mut t, mut b) = saving_host();
        h.set_recordings(true);
        let mut next = 0u128;
        let mut fresh = || {
            next += 1;
            0xfeed_0000 + next
        };
        h.give_layer_recs(&mut fresh);
        h.command(Command::Merge);
        for _ in 0..2_000 {
            if h.looper.status().layers == 1 && h.looper.layer_info(0).unwrap().closed {
                break;
            }
            run_host(&mut h, &mut t, &mut b, 1);
        }
        assert_eq!(h.looper.status().layers, 1, "merged");
        h.give_layer_recs(&mut fresh);
        assert_eq!(next, 3, "the merged layer is a recording of its own");
        h.save();
        let (text, files, _) = crate::project::loops_part(saved_from(&mut h, &mut t, &mut b));
        let line = format!(" rec={:032x} ", 0xfeed_0003u128);
        let merged = text
            .lines()
            .find(|l| l.contains(&line))
            .expect("the merged layer");
        assert!(
            merged.contains(&format!(
                " from={:032x}+{:032x} ",
                0xfeed_0001u128, 0xfeed_0002u128
            )),
            "{merged}"
        );
        // Opened and saved again: the same.
        let wavs: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(n, a)| (n.clone(), wav(a, 44_100)))
            .collect();
        let read = |name: &str| {
            wavs.iter()
                .find(|(n, _)| n == name)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| format!("no {name}"))
        };
        let (back, _, left) = crate::project::loops_open(&text, read).unwrap();
        assert!(left.is_empty());
        let (mut h2, mut t2) = (
            LoopHost::new(44_100, 20),
            Transport::new(44_100, 120.0).unwrap(),
        );
        h2.set_recordings(true);
        let mut b2 = Block::new();
        t2.command(0, T::Play);
        run_host(&mut h2, &mut t2, &mut b2, 4);
        assert!(h2.load(back).starts_with("looper: loading loops 1"));
        run_host(&mut h2, &mut t2, &mut b2, 400);
        h2.give_layer_recs(|| panic!("opened with its recording"));
        h2.save();
        let (text2, _, _) = crate::project::loops_part(saved_from(&mut h2, &mut t2, &mut b2));
        assert_eq!(
            from_text(&text2).unwrap().loops,
            from_text(&text).unwrap().loops
        );
    }

    /// Where a test's layers' files go (phase 4c.3): written at once, a
    /// piece at a time, each buffer back as soon as it is given.
    struct FakeFiles {
        on: bool,
        spare: Vec<Vec<f32>>,
        open: std::collections::HashMap<u128, (usize, Vec<f32>)>,
        done: std::collections::HashMap<u128, Vec<f32>>,
        said: Vec<(u128, Option<String>)>,
        dropped: Vec<u128>,
        fail: Option<String>,
    }

    impl FakeFiles {
        fn new() -> FakeFiles {
            FakeFiles {
                on: true,
                spare: (0..2).map(|_| vec![0.0; 8_192]).collect(),
                open: Default::default(),
                done: Default::default(),
                said: Vec::new(),
                dropped: Vec::new(),
                fail: None,
            }
        }
    }

    impl crate::loops_on_tracks::LayerFiles for FakeFiles {
        fn can_write(&self) -> bool {
            self.on
        }
        fn begin(&mut self, rec: u128, frames: usize) -> bool {
            assert!(self.open.is_empty(), "one at a time");
            self.open.insert(rec, (frames, Vec::new()));
            true
        }
        fn buffer(&mut self) -> Option<Vec<f32>> {
            self.spare.pop()
        }
        fn piece(&mut self, rec: u128, buf: Vec<f32>, len: usize) {
            if let Some((_, v)) = self.open.get_mut(&rec) {
                v.extend_from_slice(&buf[..len]);
            }
            self.spare.push(buf);
        }
        fn end(&mut self, rec: u128) {
            let (frames, v) = self.open.remove(&rec).expect("begun");
            assert_eq!(v.len(), 2 * frames, "all of it, once");
            match &self.fail {
                Some(why) => self.said.push((rec, Some(why.clone()))),
                None => {
                    self.done.insert(rec, v);
                    self.said.push((rec, None));
                }
            }
        }
        fn drop_layer(&mut self, rec: u128) {
            self.open.remove(&rec);
            self.dropped.push(rec);
        }
        fn written(&mut self) -> Option<(u128, Option<String>)> {
            (!self.said.is_empty()).then(|| self.said.remove(0))
        }
    }

    /// Phase 4c.3: with recordings on, each closed layer goes to its
    /// recording's file soon after it closes, a piece at a time while the
    /// loops play on, the same audio a save keeps; a layer still
    /// recording waits; one that goes while it is written is dropped and
    /// written again if it comes back; a file that can't be written is
    /// said once and not tried again.
    #[test]
    fn closed_layers_go_to_their_files_soon_after_they_close() {
        let (mut h, mut t, mut b) = saving_host();
        let mut files = FakeFiles::new();
        // Recordings off: nothing is written, nothing counts.
        h.give_layer_recs(|| panic!("off"));
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        assert!(files.open.is_empty() && h.unsaved_layers(0) == 0);
        h.set_recordings(true);
        let next = std::cell::Cell::new(0u128);
        let fresh = || {
            next.set(next.get() + 1);
            0xfeed_0000 + next.get()
        };
        h.give_layer_recs(fresh);
        assert_eq!(h.unsaved_layers(0), 2);
        let write = |h: &mut LoopHost, files: &mut FakeFiles, t: &mut Transport, b: &mut Block| {
            let mut periods = 0;
            while h.unsaved_layers(0) > 0 {
                assert_eq!(h.write_layers(files, PERIOD), None);
                run_host(h, t, b, 1);
                periods += 1;
                assert!(periods < 400, "still {} unsaved", h.unsaved_layers(0));
            }
            periods
        };
        // Two layers of about two seconds at 64 times the period: a few
        // dozen periods, not one.
        let periods = write(&mut h, &mut files, &mut t, &mut b);
        assert!(periods > 10, "{periods}");
        assert_eq!(files.done.len(), 2);
        // A save copies both files (4c.4): none of their audio is read.
        h.save();
        let (_, kept, copies) = crate::project::loops_part(saved_from(&mut h, &mut t, &mut b));
        assert!(kept.is_empty());
        let mut written: Vec<String> = files
            .done
            .keys()
            .map(|rec| format!("loop_{rec:032x}.wav"))
            .collect();
        written.sort();
        let mut copies = copies;
        copies.sort();
        assert_eq!(copies, written);
        // Periods with a tone on the pads, for an overdub to keep.
        let play = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, periods: usize| {
            for k in 0..periods {
                assert!(t.process(PERIOD as u32, b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for i in 0..PERIOD {
                    let f = (k * PERIOD + i) as f64;
                    l[i] = 0.2 * (f * 660.0 * std::f64::consts::TAU / 44_100.0).sin() as f32;
                    r[i] = -l[i];
                }
                h.take_pads(&l, &r);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(b, &mut l, &mut r);
            }
        };
        // Run until the loop's newest layer is closed.
        let close = |h: &mut LoopHost, t: &mut Transport, b: &mut Block| {
            for _ in 0..2_000 {
                let s = h.looper.status();
                let last = h.looper.layer_info(s.layers.saturating_sub(1)).unwrap();
                if last.closed && s.phase == Phase::Playing {
                    return;
                }
                run_host(h, t, b, 1);
            }
            panic!("the overdub never closed");
        };
        // An overdub: counted while it records, written once it closes.
        h.command(Command::Overdub);
        play(&mut h, &mut t, &mut b, 100);
        h.give_layer_recs(fresh);
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        assert_eq!((h.unsaved_layers(0), files.open.len()), (1, 0));
        h.command(Command::Overdub);
        close(&mut h, &mut t, &mut b);
        h.give_layer_recs(fresh);
        assert_eq!(next.get(), 3);
        // Undone after its first piece: dropped, and its memory is the
        // only copy again.
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        assert!(files.open.contains_key(&0xfeed_0003));
        h.command(Command::Undo);
        run_host(&mut h, &mut t, &mut b, 40);
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        assert_eq!(files.dropped, [0xfeed_0003]);
        assert!(
            files.open.is_empty() && files.spare.len() == 2,
            "the buffers back"
        );
        assert_eq!(h.unsaved_layers(0), 0);
        // Redone, it is written after all, under the same recording.
        h.command(Command::Redo);
        run_host(&mut h, &mut t, &mut b, 40);
        h.give_layer_recs(|| panic!("it has its recording"));
        write(&mut h, &mut files, &mut t, &mut b);
        assert!(files.done.contains_key(&0xfeed_0003));
        // A file that can't be written: said once, counted, not tried
        // again; the layer stays only in memory.
        files.fail = Some("the stick is full".into());
        h.command(Command::Overdub);
        play(&mut h, &mut t, &mut b, 100);
        h.command(Command::Overdub);
        close(&mut h, &mut t, &mut b);
        h.give_layer_recs(fresh);
        let mut said = Vec::new();
        for _ in 0..40 {
            said.extend(h.write_layers(&mut files, PERIOD));
            run_host(&mut h, &mut t, &mut b, 1);
        }
        assert_eq!(
            said,
            [
                "looper: a layer's file couldn't be written (the stick is full); its audio stays \
              in memory and is saved with the project"
            ]
        );
        assert_eq!(h.unsaved_layers(0), 1);
        assert!(h
            .describe()
            .contains("1 layers only in memory, 1 layers' files couldn't be written"));
        assert!(!files.done.contains_key(&0xfeed_0004));
        // No folder to write to: nothing starts.
        files.on = false;
        files.fail = None;
        h.command(Command::Overdub);
        play(&mut h, &mut t, &mut b, 100);
        h.command(Command::Overdub);
        close(&mut h, &mut t, &mut b);
        h.give_layer_recs(fresh);
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        assert!(files.open.is_empty());
        assert_eq!(h.unsaved_layers(0), 2);
    }

    /// Phase 4c.6: a layer's recording the looper still holds but no
    /// layer line names (undone, kept for Redo; in a loop a Clear kept for
    /// Undo) is named on a `kept` line, so the project keeps its file;
    /// one nothing holds any more (the loop forgotten) is not.
    #[test]
    fn recordings_held_for_an_undo_are_named_by_the_save() {
        let (mut h, mut t, mut b) = saving_host();
        let mut files = FakeFiles::new();
        h.set_recordings(true);
        let next = std::cell::Cell::new(0u128);
        h.give_layer_recs(|| {
            next.set(next.get() + 1);
            0xbead_0000 + next.get()
        });
        let mut periods = 0;
        while h.unsaved_layers(0) > 0 {
            assert_eq!(h.write_layers(&mut files, PERIOD), None);
            run_host(&mut h, &mut t, &mut b, 1);
            periods += 1;
            assert!(periods < 400, "still {} unsaved", h.unsaved_layers(0));
        }
        let name = |rec: u128| format!("loop_{rec:032x}.wav");
        let (first, over) = (name(0xbead_0001), name(0xbead_0002));
        let save = |h: &mut LoopHost, t: &mut Transport, b: &mut Block| {
            h.save();
            let s = saved_from(h, t, b);
            let layers: Vec<String> = s
                .loops
                .iter()
                .flat_map(|(_, l)| l.layers.iter().map(|y| y.file.clone()))
                .collect();
            let text = crate::project::loops_part(s.clone()).0;
            assert_eq!(text.matches("\nkept file=").count(), s.kept.len(), "{text}");
            (layers, s.kept)
        };
        // Both on their layer lines: nothing kept beside them.
        assert_eq!(
            save(&mut h, &mut t, &mut b),
            (vec![first.clone(), over.clone()], vec![])
        );
        // The overdub undone: Redo can bring it back.
        h.command(Command::Undo);
        run_host(&mut h, &mut t, &mut b, 40);
        assert_eq!(
            save(&mut h, &mut t, &mut b),
            (vec![first.clone()], vec![over.clone()])
        );
        // The loop cleared: Undo can bring both back.
        h.command(Command::Clear);
        run_host(&mut h, &mut t, &mut b, 40);
        assert_eq!(h.looper.loop_status(0).unwrap().phase, Phase::Empty);
        let (layers, mut kept) = save(&mut h, &mut t, &mut b);
        kept.sort();
        assert_eq!((layers, kept), (vec![], vec![first.clone(), over.clone()]));
        // Forgotten (no Undo): nothing holds them.
        h.command(Command::Forget);
        run_host(&mut h, &mut t, &mut b, 40);
        assert_eq!(save(&mut h, &mut t, &mut b), (vec![], vec![]));
    }

    #[test]
    fn an_open_clears_every_loop_then_its_loops_go_in() {
        let (mut h, mut t, mut b) = saving_host();
        h.save();
        let saved = saved_from(&mut h, &mut t, &mut b);
        // Loop 3 armed too: an open clears it as well.
        h.select(2);
        h.command(Command::Record);
        run_host(&mut h, &mut t, &mut b, 2);
        assert_eq!(h.looper.loop_status(2).unwrap().phase, Phase::Armed);
        // A save under way stops.
        h.save();
        assert!(h.clear_all().starts_with("looper: every loop fades out"));
        assert!(h.saving().is_none() && h.opening());
        h.open(saved.clone());
        // Nothing goes in while loop 1 fades out.
        assert_eq!(h.tend_open(), None);
        let start = std::time::Instant::now();
        let line = loop {
            run_host(&mut h, &mut t, &mut b, 1);
            if let Some(line) = h.tend_open() {
                break line;
            }
            assert!(start.elapsed().as_secs() < 60, "the loops never went in");
        };
        assert!(line.starts_with("looper: loading loops 1"), "{line}");
        assert!(!h.opening());
        assert_eq!(h.looper.loop_status(2).unwrap().phase, Phase::Empty);
        run_host(&mut h, &mut t, &mut b, 400);
        let s = h.looper.loop_status(0).unwrap();
        assert_eq!((s.phase, s.layers), (Phase::Playing, 2));
        // New project: cleared, and nothing comes in.
        h.clear_all();
        let start = std::time::Instant::now();
        while h.opening() {
            run_host(&mut h, &mut t, &mut b, 1);
            assert_eq!(h.tend_open(), None);
            assert!(start.elapsed().as_secs() < 60);
        }
        assert!((0..MAX_LOOPS).all(|n| h.looper.loop_status(n).unwrap().phase == Phase::Empty));
    }

    /// Sync sticks from loop to loop, as Starts does: set on one empty
    /// loop, the next empty loop picked takes it and records with it (with
    /// nothing playing, in your own time, as the log says); a loop that
    /// has sound keeps its own.
    #[test]
    fn sync_sticks_for_every_empty_loop() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, 1);
        assert_eq!(h.looper.status().clock, Clock::Song, "the song's at first");
        assert_eq!(
            h.command(Command::SetClock(Clock::Free)),
            "looper: sync: free (the loop keeps its own time)"
        );
        run(&mut h, 1);
        assert_eq!(h.looper.status().clock, Clock::Free);
        h.select(3);
        run(&mut h, 1);
        assert_eq!(h.looper.status().clock, Clock::Free, "loop 4 shows it");
        h.select(5);
        run(&mut h, 1);
        let said = h.command(Command::Record);
        assert!(
            said.contains("free in your own time") && said.contains("the song stays stopped"),
            "{said}"
        );
        run(&mut h, 1);
        let s = h.looper.loop_status(5).unwrap();
        assert_eq!(
            (s.phase, s.clock, s.free),
            (Phase::Recording, Clock::Free, true)
        );
        // Back to the song's, set on the loop recording: it keeps its own
        // (nothing is refused), and the next empty loop takes the song's.
        let said = h.command(Command::SetClock(Clock::Song));
        assert!(
            said.contains("from the next empty loop on (loop 6 keeps its own)"),
            "{said}"
        );
        h.select(7);
        run(&mut h, 1);
        assert_eq!(h.looper.status().clock, Clock::Song);
        assert_eq!(
            h.looper.loop_status(5).unwrap().clock,
            Clock::Free,
            "recording: kept"
        );
        assert_eq!(h.looper.stats().refused, 0);
    }

    /// Tempo follows: Free loops (the looper refactor's R2): the lead is
    /// the free loops' clock as a playing span, giving its ticks on every
    /// frame, only while it runs: none before there is a free loop, and
    /// none once a second STOP stops it.
    #[test]
    fn the_tempo_lead_is_the_free_clock_while_it_runs() {
        let mut h = LoopHost::new(44_100, 30);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        assert_eq!(h.tempo_lead(), None, "no free loop");
        h.command(Command::SetClock(Clock::Free));
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block| {
            assert!(t.process(PERIOD as u32, b));
            let l = [0.25; PERIOD];
            h.take_pads(&l, &l);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b);
        }
        t.command(t.frame(), T::Stop);
        for _ in 0..20 {
            period(&mut h, &mut t, &mut b);
        }
        assert_eq!(t.state(), mpfree_transport::State::Stopped);
        let c = h.looper.free_clock().expect("the free clock");
        assert!(c.running, "the free loop plays on");
        let s = h.tempo_lead().expect("it leads");
        assert_eq!((s.bpm, s.state, s.meter), (120.0, State::Playing, c.meter));
        for f in [t.frame(), t.frame() + 12_345, t.frame() + 1_000_000] {
            assert_eq!(s.tick_at(f), c.tick_at(f), "its ticks on frame {f}");
        }
        // A second STOP stops the free loops: the song's clock leads.
        t.command(t.frame(), T::Stop);
        for _ in 0..20 {
            period(&mut h, &mut t, &mut b);
        }
        assert!(h.looper.free_clock().is_some_and(|c| !c.running));
        assert_eq!(h.tempo_lead(), None);
    }

    /// A free loop keeps its own tempo: when the song's tempo changes, the
    /// printer asks for no print of it (one made at the song's tempo and
    /// played on the free clock would change its pitch) and it plays on
    /// at its pitch; a Speed print of it is made at the free clock's
    /// tempo.
    #[test]
    fn a_free_loop_gets_no_print_when_the_songs_tempo_changes() {
        let mut h = LoopHost::new(44_100, 30);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetClock(Clock::Free));
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        // 441 Hz: 100 frames a cycle.
        let sine = |f: u64| 0.5 * (f as f64 * std::f64::consts::TAU / 100.0).sin() as f32;
        let mut f = 0u64;
        let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, rec: bool| {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            if rec {
                for i in 0..PERIOD {
                    l[i] = sine(f + i as u64);
                    r[i] = l[i];
                }
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            f += PERIOD as u64;
            l
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b, true);
        }
        let s = h.looper.status();
        assert_eq!((s.frames, s.clock), (88_200, Clock::Free));
        t.command(t.frame(), T::SetTempo(100.0));
        for _ in 0..200 {
            period(&mut h, &mut t, &mut b, false);
        }
        assert_eq!(t.bpm(), 100.0, "the song's tempo changed");
        assert!(
            !h.printer.asked && h.printer.doing.is_none() && !h.looper.status().printed,
            "no print: {}",
            h.describe()
        );
        let ups = |out: &[f32]| {
            let x: Vec<usize> = (1..out.len())
                .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
                .collect();
            (x.len() - 1) as f64 * 44_100.0 / (x[x.len() - 1] - x[0]) as f64
        };
        let mut out = Vec::new();
        for _ in 0..30 {
            out.extend(period(&mut h, &mut t, &mut b, false));
        }
        let pitch = ups(&out);
        assert!((pitch - 441.0).abs() < 2.0, "pitch {pitch}");
        // Speed: an octave down, two bars at the free clock's 120 BPM.
        h.command(Command::Edit(Edit::Speed(-1)));
        let start = std::time::Instant::now();
        let wanted = loop {
            if let Some(w) = h.printer.doing {
                break w;
            }
            assert!(start.elapsed().as_secs() < 60, "no print: {}", h.describe());
            period(&mut h, &mut t, &mut b, false);
            std::thread::sleep(std::time::Duration::from_micros(200));
        };
        assert_eq!(wanted.frames, frames_at(2 * 3_840, 120.0, 44_100));
        assert_eq!(wanted.frames, 176_400);
    }

    /// Free loops saved with the project open on their clock, stopped at
    /// their tops, as a project opens with nothing launched, and stay off
    /// the Loop tracks' grid. The open loads them as soon as the old free
    /// loop is empty (its clock goes only with the next period), and
    /// PLAY starts them with the song.
    #[test]
    fn free_loops_saved_with_the_project_open_stopped_on_their_clock() {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetClock(Clock::Free));
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut f = 0u64;
        while h.looper.status().phase != Phase::Playing {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for i in 0..PERIOD {
                l[i] = 0.3 * ((f + i as u64) as f64 * 0.0628).sin() as f32;
                r[i] = -l[i];
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
            f += PERIOD as u64;
        }
        // The song stops; the free loop plays on, off the grid.
        t.command(t.frame(), T::Stop);
        let out = run_host(&mut h, &mut t, &mut b, 400);
        assert!(
            out[out.len() / 2..].iter().any(|&v| v != 0.0),
            "it plays on"
        );
        assert_eq!(t.state(), mpfree_transport::State::Stopped);
        assert!(!h.grid_now().unwrap()[0].pass, "in no cell");
        h.save();
        let saved = saved_from(&mut h, &mut t, &mut b);
        let clock = saved.free_clock.expect("its clock is saved");
        let (_, file) = &saved.loops[0];
        assert_eq!(file.clock, Clock::Free);
        assert_eq!(
            (clock.bpm, clock.top),
            (120.0, file.origin),
            "bar 1: its start"
        );
        // Opened again: every loop empties, then the project's go in.
        h.clear_all();
        h.open(saved);
        let start = std::time::Instant::now();
        let line = loop {
            run_host(&mut h, &mut t, &mut b, 1);
            if let Some(line) = h.tend_open() {
                break line;
            }
            assert!(start.elapsed().as_secs() < 60, "the loops never went in");
        };
        assert!(
            line.contains("loops 1 are free loops, stopped at their tops"),
            "{line}"
        );
        assert!(line.contains("120.00 BPM in 4/4"), "{line}");
        let out = run_host(&mut h, &mut t, &mut b, 400);
        let s = h.looper.loop_status(0).unwrap();
        assert_eq!(
            (s.phase, s.clock, s.loading),
            (Phase::Playing, Clock::Free, None)
        );
        assert!(out.iter().all(|&v| v == 0.0), "stopped at its top");
        assert_eq!(s.position, 0.0);
        assert!(h.looper.free_clock().is_some_and(|c| !c.running));
        assert!(!h.grid_now().unwrap()[0].pass, "in no cell");
        // PLAY: the song, and the free loop with it from its top.
        t.command(t.frame(), T::Play);
        let out = run_host(&mut h, &mut t, &mut b, 200);
        assert!(out.iter().any(|&v| v != 0.0), "it plays");
        assert!(h.looper.free_clock().is_some_and(|c| c.running));
    }

    /// Capture with Sync: Free, the picked loop playing: it goes into the
    /// first empty loop, which takes Sync as last set though it was never
    /// picked, and takes the free clock's last bars (the song has
    /// stopped; its own last bars are long gone by), which What you
    /// played numbers from the free clock's bar 1, where the free loops
    /// started.
    #[test]
    fn capture_with_sync_free_takes_the_free_loops_bars_into_a_free_loop() {
        const BAR: u64 = 88_200;
        const BAR_TICKS: Tick = 3_840;
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetLatency(0));
        h.command(Command::SetClock(Clock::Free));
        h.command(Command::SetBars(Some(1)));
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, t: &mut Transport, to: u64, out: &mut Vec<(f32, f32)>| {
            while (out.len() as u64) < to {
                let frame = out.len() as u64;
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for f in 0..PERIOD {
                    (l[f], r[f]) = pads_at(frame + f as u64);
                }
                h.take_pads(&l, &r);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
                out.extend(l.iter().copied().zip(r.iter().copied()));
            }
        };
        let mut out = Vec::new();
        // Record half a bar in: the free loop starts on bar 2's line, the
        // free clock's bar 1.
        run(&mut h, &mut t, BAR / 2, &mut out);
        h.command(Command::Record);
        while h.looper.status().phase != Phase::Playing {
            let to = out.len() as u64 + PERIOD as u64;
            run(&mut h, &mut t, to, &mut out);
        }
        assert_eq!(h.looper.free_clock().map(|c| c.top), Some(BAR_TICKS));
        // The song stops (its last bar line: bar 4's); the free loop plays
        // on, and its clock with it, on the song's old bar lines.
        run(&mut h, &mut t, 3 * BAR + 1_000, &mut out);
        t.command(t.frame(), T::Stop);
        // Bar 7, beat 2, of the song as it would have gone on.
        run(&mut h, &mut t, 6 * BAR + 30_000, &mut out);
        assert!(h.looper.free_clock().unwrap().running);
        let v = h.view(120.0);
        assert_eq!(
            v.capture_into.map(|i| i.0),
            Some(1),
            "loop 2, the first empty"
        );
        let lines: Vec<i64> = v.heard.lines[..v.heard.n_lines]
            .iter()
            .map(|l| l.0)
            .collect();
        // The song's bars 2 to 7 are the free clock's 1 to 6.
        assert_eq!(lines, [1, 2, 3, 4, 5, 6], "the free clock's bar lines");
        let (_, _, from, bars) = v.heard.take.expect("a take");
        assert_eq!(from + i64::from(bars), 6, "up to its last line");
        let said = h.capture();
        assert!(said.ends_with("become Loop 2, free"), "{said}");
        let end = out.len() as u64 + 8_192;
        run(&mut h, &mut t, end, &mut out);
        let s = h.looper.loop_status(1).unwrap();
        assert_eq!(
            (s.phase, s.clock, s.capturing),
            (Phase::Playing, Clock::Free, None)
        );
        assert_eq!(
            (s.origin, s.ticks),
            (
                (6 - i64::from(bars)) * BAR_TICKS,
                i64::from(bars) * BAR_TICKS
            ),
            "the free clock's bars, not the song's"
        );
        assert_eq!(h.looper.stats().refused, 0);
    }

    #[test]
    fn a_loop_changing_while_it_is_read_starts_the_save_again() {
        let (mut h, mut t, mut b) = saving_host();
        h.save();
        // Wait for the reading to start, then change a layer's mix.
        let start = std::time::Instant::now();
        while h.saving() == Some(0.0) {
            run_host(&mut h, &mut t, &mut b, 1);
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60);
        }
        let id = h.looper.layer_info(0).unwrap().id;
        h.looper.layer_command(id, LayerCommand::Level(0.5));
        run_host(&mut h, &mut t, &mut b, 2);
        assert_eq!(h.save_restarts, 1);
        let saved = saved_from(&mut h, &mut t, &mut b);
        assert_eq!(saved.loops[0].1.layers[0].level, 0.5);
        // A load into a loop that is not empty keeps what it has.
        assert!(h.load(saved).contains("loops 1 were not empty"));
    }

    // ------------------------------------------------------------ the wave

    /// A host to record the pads into at 120 BPM (a bar is 88 200 frames,
    /// 689 periods and a bit), its first pass `bars` long.
    fn wave_rig(bars: Option<u32>) -> (LoopHost, Transport, Block) {
        let mut h = LoopHost::new(44_100, 20);
        let t = Transport::new(44_100, 120.0).unwrap();
        h.command(Command::SetBars(bars));
        h.command(Command::SetLatency(0));
        (h, t, Block::new())
    }

    /// One period, the pads playing `input` (by song frame, from `n`).
    fn wave_period(
        h: &mut LoopHost,
        t: &mut Transport,
        b: &mut Block,
        n: &mut u64,
        input: fn(u64) -> (f32, f32),
    ) {
        assert!(t.process(PERIOD as u32, b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        for f in 0..PERIOD {
            (l[f], r[f]) = input(*n + f as u64);
        }
        *n += PERIOD as u64;
        h.take_pads(&l, &r);
        h.period(b, &mut l, &mut r);
        h.changes(String::new);
    }

    /// The wave of `layers`' first `upto` frames, drawn against `len`,
    /// worked out frame by frame from indexing: the columns, and each
    /// layer's peaks.
    fn wave_by_frame(
        layers: &[mpfree_looper::LayerView],
        len: usize,
        upto: usize,
    ) -> (Vec<[WaveColumn; 2]>, Vec<Vec<f32>>) {
        let mut acc = vec![[NO_ACC; 2]; WAVE_COLUMNS];
        let mut peaks = vec![vec![0.0f32; WAVE_COLUMNS]; layers.len()];
        for f in 0..upto {
            let c = f * WAVE_COLUMNS / len;
            for ch in 0..2 {
                let mut v = 0.0;
                for (k, l) in layers.iter().enumerate() {
                    let x = l[2 * f + ch];
                    v += x;
                    peaks[k][c] = peaks[k][c].max(x.abs());
                }
                acc[c][ch] = acc_merge(acc[c][ch], (v, v, v * v, 1));
            }
        }
        let columns = acc.iter().map(|a| [column(a[0]), column(a[1])]).collect();
        (columns, peaks)
    }

    /// `w`'s first `cols` columns are `want`'s.
    fn same_columns(w: &LoopWave, want: &(Vec<[WaveColumn; 2]>, Vec<Vec<f32>>), cols: usize) {
        assert_eq!(w.layers.len(), want.1.len());
        for c in 0..cols {
            for ch in 0..2 {
                let (a, b) = (w.columns[c][ch], want.0[c][ch]);
                assert_eq!((a.min, a.max), (b.min, b.max), "column {c}, channel {ch}");
                assert!(
                    (a.rms - b.rms).abs() <= 1e-4,
                    "column {c}: rms {} vs {}",
                    a.rms,
                    b.rms
                );
            }
            for (k, p) in want.1.iter().enumerate() {
                assert_eq!(w.layers[k][c], p[c].min(1.0), "layer {k}, column {c}");
            }
        }
    }

    /// The loop's wave is built a small slice a period (the budget's
    /// frames shared by the layers), and is what the layers hold, frame
    /// for frame.
    #[test]
    fn the_wave_takes_a_small_slice_a_period_and_is_the_layers() {
        let (mut h, mut t, mut b) = wave_rig(Some(1));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut n = 0;
        while h.looper.status().phase != Phase::Playing {
            wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
        }
        h.command(Command::Overdub);
        for _ in 0..700 {
            wave_period(&mut h, &mut t, &mut b, &mut n, |f| {
                (0.3 * ((f % 313) as f32 / 313.0), 0.2)
            });
        }
        h.command(Command::Overdub);
        while h.looper.status().phase != Phase::Playing {
            wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
        }
        let s = h.looper.status();
        assert_eq!((s.frames, s.layers), (88_200, 2));
        let mut calls = 0;
        let w = loop {
            wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
            calls += 1;
            if let Some(w) = h.wave_step(calls, 0) {
                break w;
            }
        };
        // 1 024 frames of each layer a period.
        assert_eq!(calls, 88_200usize.div_ceil(WAVE_BUDGET / 2) as u64);
        assert_eq!(
            (w.frames, w.recorded, w.columns.len()),
            (88_200, None, WAVE_COLUMNS)
        );
        let layers = [h.looper.layer(0).unwrap(), h.looper.layer(1).unwrap()];
        same_columns(&w, &wave_by_frame(&layers, 88_200, 88_200), WAVE_COLUMNS);
        // Nothing changes: no new wave.
        for p in 0..200 {
            wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
            assert!(h.wave_step(p, 0).is_none());
        }
    }

    /// While the first pass records, its wave grows with it every few
    /// periods, drawn against the pass's length, and the whole loop's
    /// wave follows once it ends.
    #[test]
    fn the_first_pass_shows_as_it_records() {
        let (mut h, mut t, mut b) = wave_rig(Some(2));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut n = 0;
        // When each live wave came, and how far it had got (the screen
        // keeps only the last wave).
        let mut live: Vec<(u64, u64)> = Vec::new();
        let mut whole = None;
        let mut checked = false;
        for p in 0..1_600u64 {
            wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
            let Some(w) = h.wave_step(p, 0) else { continue };
            let s = h.looper.status();
            let Some(r) = w.recorded else {
                whole = Some(w);
                continue;
            };
            assert_eq!(s.phase, Phase::Recording);
            assert!(r as usize <= s.recorded, "{r} of {}", s.recorded);
            assert_eq!(w.frames, 2 * 88_200, "drawn against the 2 bars");
            assert_eq!(w.live(), Some(r as f32 / w.frames as f32));
            if !checked && r > 88_200 {
                // The columns it has filled are the pass so far.
                let pass = h.looper.first_pass(0).unwrap();
                let want = wave_by_frame(&[pass], 2 * 88_200, r as usize);
                same_columns(&w, &want, r as usize * WAVE_COLUMNS / (2 * 88_200));
                checked = true;
            }
            live.push((p, r));
        }
        assert!(checked);
        assert!(live.len() > 100, "{} live waves", live.len());
        for pair in live.windows(2) {
            assert_eq!(pair[1].0 - pair[0].0, LIVE_EVERY);
            assert!(pair[1].1 > pair[0].1);
        }
        let w = whole.expect("the whole loop's wave");
        assert_eq!(
            (w.frames, w.recorded, w.layers.len()),
            (2 * 88_200, None, 1)
        );
        assert_eq!(h.waves.waits, 0);
    }

    /// A free-length first pass is drawn against two bars, then four once
    /// it gets past two, and so on: the columns it had are kept, in pairs.
    #[test]
    fn a_free_length_pass_widens_its_wave() {
        let (mut h, mut t, mut b) = wave_rig(None);
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut n = 0;
        let mut last = None;
        for p in 0..(3 * 690) {
            wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
            if let Some(w) = h.wave_step(p, 0) {
                if p < 2 * 680 {
                    assert_eq!(w.frames, 2 * 88_200, "period {p}");
                }
                last = Some(w);
            }
        }
        assert_eq!(h.looper.status().phase, Phase::Recording);
        let w = last.unwrap();
        assert_eq!(w.frames, 4 * 88_200);
        let r = w.recorded.unwrap() as usize;
        assert!(r > 2 * 88_200);
        let pass = h.looper.first_pass(0).unwrap();
        let want = wave_by_frame(&[pass], 4 * 88_200, r);
        same_columns(&w, &want, r * WAVE_COLUMNS / (4 * 88_200));
    }

    /// The wave goes when the loop does: cleared, or its first pass
    /// thrown away (STOP before a 1/16 note was played) while its wave was
    /// shown.
    #[test]
    fn the_wave_empties_when_the_loop_does() {
        for clear in [true, false] {
            let (mut h, mut t, mut b) = wave_rig(Some(1));
            h.command(Command::Record);
            t.command(0, T::Play);
            let mut n = 0;
            let until = if clear { 1_000 } else { 30 };
            let mut shown = None;
            for p in 0..until {
                wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
                if let Some(w) = h.wave_step(p, 0) {
                    shown = Some(w);
                }
            }
            assert!(shown.is_some_and(|w| w.frames > 0));
            if clear {
                h.command(Command::Clear);
            } else {
                assert!(t.command(n, T::Stop));
            }
            let mut empty = None;
            for p in until..until + 400 {
                wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
                if let Some(w) = h.wave_step(p, 0) {
                    assert!(empty.is_none(), "one empty wave");
                    empty = Some(w);
                }
            }
            assert_eq!(h.looper.status().phase, Phase::Empty);
            let w = empty.expect("the empty wave");
            assert_eq!(w.frames, 0, "clear {clear}");
        }
    }

    /// Making waves never allocates on the audio thread, while the first
    /// pass records, overdubs refresh it and a clear empties it: the
    /// waves are made again in place once the screen lets go of them.
    #[test]
    fn making_waves_never_allocates() {
        let (mut h, mut t, mut b) = wave_rig(Some(1));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut n = 0;
        let mut shown: Option<Arc<LoopWave>> = None;
        let (mut calls, mut waves, mut live) = (0, 0, 0);
        for p in 0..2_400u64 {
            match p {
                900 | 1_700 => {
                    h.command(Command::Overdub);
                }
                2_300 => {
                    h.command(Command::Clear);
                }
                _ => {}
            }
            wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
            let ((), c) = crate::sys::alloc_count::calls(|| {
                if let Some(w) = h.wave_step(p, 64) {
                    waves += 1;
                    live += usize::from(w.recorded.is_some());
                    shown = Some(w);
                }
            });
            calls += c;
        }
        assert!(
            live > 40 && waves > live + 5,
            "{live} live of {waves} waves"
        );
        assert_eq!(shown.map(|w| w.frames), Some(0), "cleared");
        assert_eq!(calls, 0, "allocations or frees making waves");
        assert_eq!(h.waves.waits, 0);
    }

    /// Each loop's small wave for its cells on the Clips grid: made after
    /// the loop's own wave, each column the peak of its frames, and
    /// emptied with the loop.
    #[test]
    fn each_loop_gets_a_small_wave_for_its_cells() {
        let (mut h, mut t, mut b) = wave_rig(Some(1));
        h.command(Command::Record);
        t.command(0, T::Play);
        let (mut n, mut p) = (0, 0u64);
        while h.looper.status().phase != Phase::Playing {
            wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
            h.wave_step(p, 0);
            p += 1;
        }
        // The loop's own wave first, then its small one.
        let mut own = false;
        while h.view(120.0).cell_waves[0] == [0; CELL_WAVE] {
            assert!(p < 2_000, "no small wave");
            wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
            if h.wave_step(p, 0).is_some() {
                own = true;
                assert_eq!(h.view(120.0).cell_waves[0], [0; CELL_WAVE]);
            }
            p += 1;
        }
        assert!(own);
        let len = h.looper.status().frames;
        let mut frames = vec![[0.0f32; 2]; len];
        h.looper.layer(0).unwrap().read(0, &mut frames);
        let mut want = [0u8; CELL_WAVE];
        for (c, w) in want.iter_mut().enumerate() {
            let from = (c * len).div_ceil(CELL_WAVE);
            let to = ((c + 1) * len).div_ceil(CELL_WAVE);
            let peak = frames[from..to]
                .iter()
                .fold(0.0f32, |m, x| m.max(x[0].abs()).max(x[1].abs()));
            *w = (peak.min(1.0) * 255.0).round() as u8;
        }
        assert_eq!(h.view(120.0).cell_waves[0], want);
        assert!(want.iter().all(|&w| w > 100), "{want:?}");
        // Cleared: empty again.
        h.command(Command::Clear);
        for _ in 0..200 {
            wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
            h.wave_step(p, 0);
            p += 1;
        }
        assert_eq!(h.view(120.0).cell_waves[0], [0; CELL_WAVE]);
    }

    #[test]
    fn the_log_says_what_the_beat_played_into_a_loop_said() {
        let r = |bars, beat| Reading {
            bars,
            bpm: 100.0,
            sure: Sure::Clear,
            other: None,
            said: None,
            beat,
        };
        assert_eq!(reading_words(&r(1, None)), "1 bar at 100.00 BPM");
        assert_eq!(
            reading_words(&r(1, Some(0.9))),
            "1 bar at 100.00 BPM, which the beat you played fits"
        );
        assert_eq!(
            reading_words(&r(2, Some(0.3))),
            "2 bars at 100.00 BPM, which the beat you played fits only loosely"
        );
        assert_eq!(
            reading_words(&r(3, Some(0.9))),
            "3 bars at 100.00 BPM, from the beat you played"
        );
    }
}
