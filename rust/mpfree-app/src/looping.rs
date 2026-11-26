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
    Snap, SnapDone, SnapPlan, SongPlan, Starts, Stats, Status, Step, Sure, Take, HEARD_SECONDS,
    LOOP_MAX_BPM, LOOP_MIN_BPM, MAX_LAYERS, MAX_LOOPS,
};
use mpfree_seq::loops::LoopNow;
use mpfree_transport::time::{Tick, PPQ};
use mpfree_transport::{Block, Span, State, Swing};
use mpfree_ui::wave::{Peaks, WaveColumn};
use std::ops::Range;
use std::sync::mpsc::{
    channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError, TrySendError,
};
use std::sync::Arc;

mod capture;
mod commands;
mod controls;
mod copy;
mod line_in;
mod period;
mod point;
mod save;
mod smart_end;
mod song;
mod sources;
mod starts;
mod state;
mod tracks;
mod view;
mod waves;
mod words;
mod workers;

// The looper host's parts, which were all in this file once: each sees
// the rest through `use super::*`, and what they hold is here as before
// (the parts holding only `LoopHost`'s methods have nothing to bring).
pub use copy::*;
pub use line_in::*;
pub use point::*;
pub use save::*;
use song::*;
pub use view::*;
pub use waves::*;
pub use words::*;
pub use workers::*;

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
    /// cells (audio tracks phase 4b): what a take on
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
    /// A free loop's header's Make song ([`LoopHost::make_song`]).
    MakeSong,
    /// The hint line's Snap anyway (true) or Not now
    /// ([`LoopHost::snap_anyway`], [`LoopHost::snap_not_now`]).
    Answer(bool),
    /// The loop point view opened or closed
    /// ([`LoopHost::set_point_open`]).
    PointView(bool),
    /// A footswitch's Snap or Use my tap ([`LoopHost::choose`]).
    Choose(Side),
    /// The loop point view's Move start / Move end ([`LoopHost::move_act`]).
    Move(MoveAct),
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
    /// That audio, for the loop, its version, where smart loop end left
    /// it and the move set up, until one changes.
    point: Option<(usize, u64, Shift, Option<Shift>, LoopPoint)>,
    /// A move set up in the loop point view: the loop, its version, and
    /// where the start and end would go ([`LoopHost::move_act`]).
    moving: Option<(usize, u64, Shift)>,
    /// The step Move start / Move end goes by.
    move_step: MoveStep,
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
    /// Make song ([`LoopHost::make_song`]), as it goes, and the times it
    /// was refused, didn't land or couldn't be taken back: counted,
    /// logged and said.
    song: SongStep,
    pub songs_refused: u64,
    /// Make song's transaction on the one history (phase 4d.2): given
    /// when it is made, kept while it is taken back and made again, none
    /// for a new press.
    song_txn: mpfree_transport::TransactionId,
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
    /// Loops on audio tracks (phase 4b): each slot's
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
            moving: None,
            move_step: MoveStep::default(),
            point_buf: vec![[0.0; 2]; point_frames(rate)],
            snaps_refused: 0,
            readings_overruled: 0,
            tempos_refused: 0,
            song: SongStep::Idle,
            songs_refused: 0,
            song_txn: mpfree_transport::TransactionId::default(),
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
}

#[cfg(test)]
mod tests;
