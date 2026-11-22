//! mpfree-audiotracks: audio tracks on the song timeline.
//!
//! Up to [`MAX_TRACKS`] audio tracks that record the line input in a line
//! along the song, the way a tape machine or a DAW does, next to the MIDI
//! tracks of the arrangement:
//!
//! - **Record.** An armed track records while the transport records
//!   (Record from stopped after the count-in, or a punch in while
//!   playing), until the punch out, a stop, or the track is disarmed.
//!   Each recording is a **take**: the audio, and the song tick its first
//!   frame belongs to (its `anchor`).
//! - **Regions.** What a track plays is its list of [`Region`]s: stretches
//!   of song ticks, each playing a stretch of one take. A new recording
//!   puts its region over whatever was there; the older audio is not lost,
//!   only covered (the take is still there to comp from).
//! - **Loop recording.** While the transport loops, each pass is a take of
//!   its own (one `group`, numbered by `pass`); the newest pass plays.
//! - **Comping.** [`Command::Comp`] plays a stretch of any take of the
//!   track in place of what was there (a swipe across a take lane). Split,
//!   move, erase, gain and fades edit regions. Undo and Redo walk one
//!   history of every edit and recording, for all tracks.
//! - **Sounds brought in.** A sound from elsewhere (the Sample page's Put
//!   on Audio track) loads as a take of its own and goes on its track as
//!   one undo step, a region a slice ([`AudioTracks::place`],
//!   [`host::AudioHost::import`]).
//! - **No clicks.** Every region fades in over [`FADE`] frames from its
//!   start and plays on [`FADE`] frames past its end fading out (equal
//!   power), so two regions that meet crossfade there: a punch in or out,
//!   or a comp boundary, is seamless when the takes are of the same part.
//! - **Time.** A region sits on song ticks, so it stays on its bar when the
//!   tempo before it changes. Its audio plays at the rate it was recorded
//!   at, from the region's start, as a DAW plays audio that is not warped:
//!   at another tempo it does not stretch (time-stretch is a later step,
//!   with `mpfree-stretch`).
//! - **Latency.** Input that arrives `latency` frames after a moment
//!   belongs to that moment ([`Command::SetLatency`]): takes are placed
//!   that much earlier, and a recording keeps writing that long after its
//!   punch out (and [`FADE`] frames more, for the crossfade past its end).
//! - **Monitoring.** [`Monitor::Auto`] (the default) hears the input on an
//!   armed track while stopped or recording, and the track's regions
//!   otherwise; [`Monitor::In`] always hears the input as well;
//!   [`Monitor::Off`] never does.
//! - **Lasting ids.** Every take is a recording with a [`RecordingId`]:
//!   given as it starts (recorded, brought in or a stretched copy) or kept
//!   from its save, saved with it, and never given to another, so a
//!   project, its undo steps and later its clips and files can name it
//!   across saves. The take's `u16` is only its place in this engine;
//!   regions and undo steps name that place, and a place an undo step
//!   names is never given to another take ([`AudioTracks::in_use`]).
//! - **One memory.** Takes share one pool of samples, asked for once in
//!   [`AudioTracks::new`], in chunks of [`CHUNK`] samples: a recording
//!   takes one chunk at a time, so several tracks record together with no
//!   planning, and nothing is ever moved. [`Command::Forget`] gives a take
//!   back.
//! - **Recording to files.** With [`AudioTracks::files`] on, every take
//!   recorded also goes to a file of its own as it records, so a crash or
//!   a power cut loses at most a few seconds of it (the recording file
//!   protocol of `mpfree-stream`, docs/dev/design/recording-files.md). The
//!   files are made when a track arms, before anything records: an armed
//!   track holds the lasting id of its next take ([`Io::Arm`]), and that
//!   take records under it ([`Io::Start`]). The host makes, writes and
//!   finishes the files off the audio thread (see [`host`]); the audio
//!   thread only says what to do, and a take stays put until the host has
//!   copied all of it out ([`AudioTracks::written`]).
//! - **Playing from files.** Given the stream layer's player
//!   ([`AudioTracks::stream`]), a take can play from its file in place of
//!   the pool: a recorded take moves to its finished file once the frames
//!   its regions are about to play are in the stream cache, so the move
//!   never changes a sample ([`Command::OnFile`]); while it records, each
//!   pool chunk its file holds already goes back, so a take can be longer
//!   than the memory; and a saved take can open as its file with nothing
//!   copied in
//!   ([`AudioTracks::add_file_take`]). Each track tells the player where
//!   it plays and where it plays next; every frame that plays silence
//!   because the stick was late is counted, a take at a time
//!   ([`AudioTracks::missed`]). Without a player every take is in the
//!   pool, as before.
//!
//! [`AudioTracks::command`] and [`AudioTracks::process`] never allocate,
//! lock or panic, and give the same output for any period size.

#![forbid(unsafe_code)]

pub mod files;
pub mod host;
mod pool;
pub mod retime;
pub mod save;
pub mod streaming;
pub mod stretch;

use pool::Pool;
pub use pool::CHUNK;
pub use retime::{Piece, Retime};
use std::collections::VecDeque;
use std::fmt;

use save::{Saved, TakeFile, TrackFile};

use mpfree_stream::msg::{Place, MAX_CURSORS, MAX_SOURCES};
use mpfree_stream::player::Player;
use mpfree_transport::time::{TempoMap, Tick};
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Mark, Span, State};

/// Audio tracks at most.
pub const MAX_TRACKS: usize = 8;
/// Takes at most, for all tracks together.
pub const MAX_TAKES: usize = 256;
/// Regions on one track at most.
pub const MAX_REGIONS: usize = 256;
/// Frames of the crossfade at each region's start and past its end (about
/// 1.5 ms at 44.1 kHz).
pub const FADE: usize = 64;
/// A loaded take's audio copies in at this many frames for each frame
/// that plays (a minute of audio in about a second).
pub const LOAD_SPEED: usize = 64;
/// Edits and recordings Undo can take back, counted a track at a time (a
/// song time edit is one step of a track each).
pub const HISTORY: usize = 64;
/// Commands waiting for the next period.
const QUEUE: usize = 64;
/// The most frames the tracks run in one go.
const RUN: usize = 128;
/// Samples in one block of the peak table: the waveform overview keeps the
/// lowest and highest sample of every block of the pool. A chunk is whole
/// blocks, so a block never holds two takes.
pub const PEAK_BLOCK: usize = 256;
/// Requests about the recording files waiting for the host, at most. The
/// host takes them every period, and a period asks for at most a few for
/// each command and each track's punch in, so the queue never fills.
pub const IO_QUEUE: usize = 1024;
/// Seconds ahead a track tells the stream player of its next region, so
/// that region's first frames are read before it starts.
pub const CUE_S: f64 = 4.0;
/// Seconds of a region's frames, from where it plays, that must be in the
/// stream cache before its take moves from the pool to its file.
pub const MOVE_S: f64 = 2.0;

// A take's stream source is its id, and each track plays through two
// cursors (see [`AudioTracks::stream`]).
const _: () = assert!(MAX_TAKES <= MAX_SOURCES && 2 * MAX_TRACKS <= MAX_CURSORS);

fn min_max(xs: &[f32]) -> (f32, f32) {
    xs.iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)))
}

/// What a track records from the line input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// The left jack, as a mono take (the MPC One's line input records
    /// on the left jack: round 7).
    Left,
    Right,
    /// Both jacks, as a stereo take.
    Stereo,
    /// Both jacks summed to a mono take.
    Sum,
}

impl Input {
    pub fn channels(self) -> usize {
        match self {
            Input::Stereo => 2,
            _ => 1,
        }
    }
}

/// When a track hears its input; see the module notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Monitor {
    Off,
    Auto,
    In,
}

/// What the screen, the buttons and the host ask for. Commands apply in
/// order at the start of the next period. A command naming a track past
/// the last, a region that is not there, or an edit that does not fit is
/// counted in [`Stats::refused`] and changes nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Arm a track to record (or disarm it: a recording ends there).
    Arm(usize, bool),
    /// What the track's next recording takes.
    SetInput(usize, Input),
    /// The track records from inside (Some: how late the host's block for
    /// it is, in frames) or the line input (None): the instrument
    /// redesign's step 9. A track recording from inside takes its block in
    /// stereo, never plays it through, and can't change while it records.
    SetInside(usize, Option<u32>),
    SetMonitor(usize, Monitor),
    /// The track's level, 0 to 2 (1 is as recorded).
    SetLevel(usize, f32),
    /// The track's balance, -1 (left) to 1 (right).
    SetPan(usize, f32),
    Mute(usize, bool),
    Solo(usize, bool),
    /// Input latency to make up for, in frames, for every track.
    SetLatency(u32),
    /// Whether the tracks play and record (the host turns them off when
    /// the song is not playing, say in clip launch mode). Monitoring goes
    /// on either way.
    SetActive(bool),
    /// Play take `take` on its track from tick `from` to `to`, over what
    /// was there (clipped to where the take has audio).
    Comp {
        take: u16,
        from: Tick,
        to: Tick,
    },
    /// Cut the region of `track` playing at tick `at` in two there.
    Split {
        track: usize,
        at: Tick,
    },
    /// Silence `track` from `from` to `to`.
    Erase {
        track: usize,
        from: Tick,
        to: Tick,
    },
    /// Move the region of `track` playing at `at` to start at `to`, over
    /// what is there (its audio moves with it).
    Move {
        track: usize,
        at: Tick,
        to: Tick,
    },
    /// The level of the region of `track` playing at `at`, 0 to 4.
    SetGain {
        track: usize,
        at: Tick,
        gain: f32,
    },
    /// Fades of the region of `track` playing at `at`, in frames from its
    /// start and to its end (each at most half the region; on top of the
    /// crossfades every region has).
    SetFades {
        track: usize,
        at: Tick,
        fade_in: u32,
        fade_out: u32,
    },
    Undo,
    Redo,
    /// Give a take's memory back. Refused while it records or while a
    /// region, or a step Undo or Redo could bring back, plays it (see
    /// [`AudioTracks::in_use`]); [`Command::ClearHistory`] first.
    Forget(u16),
    /// Forget the undo history (so takes only it used can be forgotten).
    ClearHistory,
    /// Play take `new` wherever take `old` plays, its frames `ratio` times
    /// as many (a stretched copy: see [`stretch`]); the regions keep their
    /// place on the song. One undo step. Refused unless both takes are of
    /// one track and `new` has copied in.
    Swap {
        old: u16,
        new: u16,
        ratio: f64,
    },
    /// A song time edit (the Arrange refactor's 3.4, [`retime`]): every
    /// track's regions move with the stretch of song they sit in, as one
    /// undo step that belongs to song edit `song` (not 0). Only
    /// [`Command::SongUndo`] and [`Command::SongRedo`] take that step back
    /// and bring it back; [`Command::Undo`] and [`Command::Redo`] stop at
    /// it. The cuts use the tempo map of the period before (the one the
    /// regions were placed on: the edit's own view brings the new one).
    /// Refused, nothing changing, while a track records or when a track's
    /// regions would not fit ([`AudioTracks::retime_refused`] says first).
    Retime {
        song: u32,
        retime: Retime,
    },
    /// Take back, or bring back, song edit `song`'s steps (one a
    /// [`Command::Retime`] it was made of: + Bar pressed in a row is one
    /// song edit): refused unless they are the steps Undo, or Redo, would
    /// take next.
    SongUndo(u32),
    SongRedo(u32),
    /// The song can no longer take back, or bring back, song edit `song`
    /// (its history moved on past it): its steps go, with every step
    /// Undo or Redo could only reach through them, so the regions never
    /// go back to where they sat on a song that is gone. Changes no
    /// region.
    ForgetSong(u32),
    /// Take `take`'s file is open as stream source generation `gen` (the
    /// source is the take's id): finished, or, when `growing`, still
    /// being written while the take records (opened at the frames
    /// committed so far, and grown as more are). Each pool chunk of the
    /// take that the file holds is given back, its frames played from the
    /// file from then on, and once the file holds every frame of a take no
    /// longer recording, the take plays from its file alone (see
    /// [`AudioTracks::stream`]). While a region plays the take, nothing
    /// goes until the frames it is about to play are in the stream cache,
    /// so the move never plays silence. A finished file must hold the
    /// take's frames exactly, and a growing one no more than it has; one
    /// that doesn't, or has other channels, is closed before anything
    /// goes ([`Stats::mismatched`]). `rec` is the take's lasting id, so a
    /// take forgotten meanwhile, its id given to another, is never moved.
    /// Refused unless the tracks have a player and the take is in the
    /// pool alone and not loading; a finished file is refused while the
    /// take records or is still being copied out to its file.
    OnFile {
        take: u16,
        rec: RecordingId,
        gen: u32,
        growing: bool,
    },
}

/// Why the tracks can't take a song time edit ([`AudioTracks::retime_refused`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetimeRefused {
    /// The track records.
    Recording(usize),
    /// The track's regions, cut at the edit's edges, would be more than
    /// [`MAX_REGIONS`].
    Full(usize),
}

/// Whether a song edit's step can be taken back or brought back
/// ([`AudioTracks::song_step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SongStep {
    /// It is the step Undo (or Redo) takes next.
    Next,
    /// Other steps lie on top of it (audio edits made since).
    Under,
    /// The history no longer holds it (it ran past it, or was cleared).
    Gone,
}

/// A recording's lasting id (the audio tracks rethink, phase 1c): a
/// random 128-bit number, given as a take starts (recorded, brought in, or
/// a stretched copy) and saved with it, never given to another. 0 is none.
/// The recording files of the disk streaming plan carry the same number
/// in their names (`NAME_ID.wav`, ID as its 32 hex digits:
/// [`RecordingId::to_string`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordingId(pub u128);

impl RecordingId {
    pub const NONE: RecordingId = RecordingId(0);

    pub fn is_given(self) -> bool {
        self.0 != 0
    }

    /// From its 32 hex digits, as saved and in file names; None for
    /// anything else, or for 0.
    pub fn parse(s: &str) -> Option<RecordingId> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u128::from_str_radix(s, 16)
            .ok()
            .map(RecordingId)
            .filter(|r| r.is_given())
    }
}

impl fmt::Display for RecordingId {
    /// Its 32 hex digits, lower case.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// Makes [`RecordingId`]s without allocating (on the audio thread, as a
/// recording starts): a key drawn at random once, and a count put through
/// a mix that never maps two counts to one number, so no id repeats in a
/// run, and runs (each with its own key) don't meet but by a chance of
/// about one in 2^128.
#[derive(Clone, Copy)]
struct Ids {
    key: u128,
    next: u64,
}

impl Ids {
    fn new() -> Ids {
        use std::hash::{BuildHasher, Hasher};
        let half = |salt: u64| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(salt);
            if let Ok(d) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                h.write_u128(d.as_nanos());
            }
            h.finish()
        };
        Ids {
            key: (u128::from(half(1)) << 64) | u128::from(half(2)),
            next: 0,
        }
    }

    fn fresh(&mut self) -> RecordingId {
        loop {
            let n = self.next;
            self.next = self.next.wrapping_add(1);
            // `mix` is one to one, so the high half alone never repeats.
            let hi = mix((self.key >> 64) as u64 ^ n);
            let lo = mix(self.key as u64 ^ n.rotate_left(32) ^ 0x9e37_79b9_7f4a_7c15);
            let id = RecordingId((u128::from(hi) << 64) | u128::from(lo));
            if id.is_given() {
                return id;
            }
        }
    }
}

/// SplitMix64's finish: a one-to-one mix of the 64 bits.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A stretch of song ticks playing a stretch of a take: tick `start`
/// plays the take's frame `src`, and the frames after it follow at the
/// recorded rate up to tick `end`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub start: Tick,
    pub end: Tick,
    pub take: u16,
    /// The take's frame that plays at `start` (may be below 0 or past the
    /// take's end: those frames are silent).
    pub src: i64,
    pub gain: f32,
    pub fade_in: u32,
    pub fade_out: u32,
}

impl Region {
    const EMPTY: Region = Region {
        start: 0,
        end: 0,
        take: 0,
        src: 0,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
    };

    pub fn contains(&self, t: Tick) -> bool {
        (self.start..self.end).contains(&t)
    }
}

/// What the screen shows of a take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TakeInfo {
    /// Its place in this engine (what regions name).
    pub id: u16,
    /// Its lasting id ([`RecordingId`]), kept across saves.
    pub rec: RecordingId,
    pub track: usize,
    /// Recordings of one press of Record share a group; each pass of a
    /// loop recording is a take of its own, numbered from 0.
    pub group: u16,
    pub pass: u16,
    /// The tempo it was recorded at (0: not known).
    pub bpm: f64,
    /// The song tick (with its fraction) its frame 0 belongs to.
    pub anchor: f64,
    pub frames: usize,
    pub channels: usize,
    pub recording: bool,
    /// The recording stopped early because the memory was full.
    pub cut_short: bool,
}

/// A track's settings, as the commands last set them (or a load).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackSettings {
    pub input: Input,
    pub monitor: Monitor,
    /// 0 to 2 (1 is as recorded).
    pub level: f32,
    /// -1 (left) to 1 (right).
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

/// Counters for the screen and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Commands that changed nothing (see [`Command`]).
    pub refused: u32,
    /// Commands dropped because the queue was full.
    pub dropped: u32,
    /// Recordings cut short by a full memory or [`MAX_TAKES`].
    pub full: u32,
    /// Requests about the recording files dropped because the queue was
    /// full (it never should be): a take whose [`Io::Start`] is dropped
    /// has no file, and is saved from memory.
    pub io_dropped: u32,
    /// Frames of takes played from their files that played silence
    /// because the stream cache didn't have them yet (or their file wasn't
    /// open): every dropout, for all takes ([`AudioTracks::missed`] gives
    /// them a take at a time).
    pub missed: u64,
    /// Files opened for a take ([`Command::OnFile`]) whose channels or
    /// length didn't match it: the take stays in the pool, and the host is
    /// asked to close the file ([`Io::Close`]).
    pub mismatched: u32,
}

/// What the host does about the recording files while
/// [`AudioTracks::files`] is on, in order ([`AudioTracks::take_io`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Io {
    /// Track `track` armed (or its next take changed channels): make the
    /// files its next take records into, for recording `rec` with
    /// `channels` channels. If they can't be made the take still records,
    /// into memory only.
    Arm {
        track: u8,
        rec: RecordingId,
        channels: u8,
    },
    /// The files made for `rec` won't be recorded into (the track was
    /// disarmed, its next take changed channels, or the tracks were
    /// cleared): remove them.
    Disarm { rec: RecordingId },
    /// Take `take` started recording into the files made for its lasting
    /// id. Copy its frames out as they come ([`AudioTracks::read_take`]
    /// reads a take while it records), finish its file once it has stopped
    /// recording, then tell [`AudioTracks::written`].
    Start { take: u16 },
    /// Take `take`'s stream source, at generation `gen`, is no longer
    /// played (the take was forgotten or cleared, or its file didn't match
    /// it): close it, if it is still that generation (a newer take may
    /// have the id and its own source by then).
    Close { take: u16, gen: u32 },
}

/// The memory gauge, in samples (a stereo frame is two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub used: usize,
    pub capacity: usize,
}

/// Where a take's audio is (the audio tracks rethink, phase 3b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Store {
    /// In the pool.
    Pool,
    /// Its file is open as stream source generation `gen` (finished, or
    /// still being written while it records when `growing`): frames
    /// below `freed` play from the file, their pool chunks given back,
    /// and the rest from the pool. More chunks go as the file holds them,
    /// and once it holds every frame of a take no longer recording, the
    /// take plays from its file alone ([`Command::OnFile`]).
    Part { gen: u32, freed: u64, growing: bool },
    /// Stream source generation `gen`, played through the player; it holds
    /// no pool memory.
    File { gen: u32 },
}

/// A region a track plays, or is about to: its take, the take's frame
/// where it plays (or starts), and the frame it stops at (its crossfade's
/// end). Told to the player each period, and what a take's move to its
/// file waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cue {
    take: u16,
    frame: u64,
    end: u64,
}

#[derive(Clone, Copy)]
struct Take {
    live: bool,
    rec: RecordingId,
    track: u8,
    channels: u8,
    group: u16,
    pass: u16,
    bpm: f64,
    anchor: f64,
    /// Frames written.
    frames: usize,
    recording: bool,
    cut_short: bool,
    /// Its saved audio is still copying in: it plays nothing yet.
    loading: bool,
    /// Recorded with files on, and not yet all copied out to its file: it
    /// can't be forgotten, nor the tracks cleared, until the host says
    /// [`AudioTracks::written`].
    writing: bool,
    store: Store,
    /// Frames played from its file that played silence (see
    /// [`Stats::missed`]), and the song tick of the latest.
    missed: u64,
    miss_tick: f64,
}

const NO_TAKE: Take = Take {
    live: false,
    rec: RecordingId::NONE,
    track: 0,
    channels: 1,
    group: 0,
    bpm: 0.0,
    pass: 0,
    anchor: 0.0,
    frames: 0,
    recording: false,
    cut_short: false,
    loading: false,
    writing: false,
    store: Store::Pool,
    missed: 0,
    miss_tick: 0.0,
};

impl Take {
    /// Adds `gain`-weighted frames `k..k + n` of take `id` into the
    /// interleaved stereo `out` (n frames); frames the take does not have
    /// add nothing. `gain(j)` is the gain of frame `j` of the run.
    fn add_into(
        &self,
        pool: &Pool,
        id: u16,
        k: i64,
        out: &mut [f32],
        gain: &impl Fn(usize) -> f32,
    ) {
        let n = out.len() / 2;
        let ch = self.channels as usize;
        // Skip frames before the take.
        let mut j = if k < 0 { ((-k) as usize).min(n) } else { 0 };
        while j < n {
            let at = (k + j as i64) as usize; // take frame
            if at >= self.frames {
                break;
            }
            let s = at * ch;
            let off = s % CHUNK;
            let count = ((CHUNK - off) / ch).min(n - j).min(self.frames - at);
            let Some(slot) = pool.readable(id, (s / CHUNK) as u32) else {
                j += count;
                continue;
            };
            let b = slot * CHUNK + off;
            add_frames(&pool.data[b..b + count * ch], ch, out, j, gain);
            j += count;
        }
    }

    /// [`Take::add_into`] for a take played from its file, stream source
    /// `id` at generation `gen`, through the player's cache (`scratch`
    /// holds what is read), for its frames below `end` (its length, or
    /// where its pool part starts). Gives the frames that played silence
    /// for want of them: not in the cache yet, or the source not open at
    /// `gen` with the take's channels and those frames (or no player).
    #[allow(clippy::too_many_arguments)]
    fn add_from_file(
        &self,
        player: Option<&mut Player>,
        id: u16,
        gen: u32,
        end: usize,
        k: i64,
        out: &mut [f32],
        scratch: &mut [f32],
        gain: &impl Fn(usize) -> f32,
    ) -> u64 {
        let n = out.len() / 2;
        let ch = self.channels as usize;
        let j0 = if k < 0 { ((-k) as usize).min(n) } else { 0 };
        let from = (k + j0 as i64) as usize;
        let have = end.min(self.frames).saturating_sub(from).min(n - j0);
        if have == 0 {
            return 0;
        }
        let holds = |&(c, f): &(u16, u64)| c == u16::from(self.channels) && f >= end as u64;
        let Some(p) = player.filter(|p| {
            p.source(id) == Some((gen, true)) && p.shape(id).as_ref().is_some_and(holds)
        }) else {
            return have as u64;
        };
        let (mut missed, mut done) = (0, 0);
        while done < have {
            let count = (have - done).min(scratch.len() / ch);
            let buf = &mut scratch[..count * ch];
            missed += p.read(id, (from + done) as u64, buf);
            add_frames(buf, ch, out, j0 + done, gain);
            done += count;
        }
        missed
    }

    /// [`Take::add_into`] for a take partly on its file: its frames below
    /// `freed` from the file ([`Take::add_from_file`]), the rest from the
    /// pool. Gives the frames that played silence for want of them.
    #[allow(clippy::too_many_arguments)]
    fn add_part(
        &self,
        pool: &Pool,
        player: Option<&mut Player>,
        id: u16,
        gen: u32,
        freed: u64,
        k: i64,
        out: &mut [f32],
        scratch: &mut [f32],
        gain: &impl Fn(usize) -> f32,
    ) -> u64 {
        let n = out.len() / 2;
        // Frames of the run before the pool's part.
        let split = (freed as i64).saturating_sub(k).clamp(0, n as i64) as usize;
        let (file, rest) = out.split_at_mut(2 * split);
        let missed = if split > 0 {
            self.add_from_file(player, id, gen, freed as usize, k, file, scratch, gain)
        } else {
            0
        };
        if split < n {
            self.add_into(pool, id, k + split as i64, rest, &|j| gain(j + split));
        }
        missed
    }

    fn sample(&self, pool: &Pool, id: u16, k: usize, c: usize) -> Option<f32> {
        if k >= self.frames {
            return None;
        }
        let ch = self.channels as usize;
        let s = k * ch + c.min(ch - 1);
        let slot = pool.readable(id, (s / CHUNK) as u32)?;
        Some(pool.data[slot * CHUNK + s % CHUNK])
    }
}

/// Adds `gain`-weighted frames `src` (interleaved, `ch` channels: a mono
/// frame goes to both sides) into the interleaved stereo `out` from its
/// frame `j`: `gain(j + i)` weighs frame `i`.
fn add_frames(src: &[f32], ch: usize, out: &mut [f32], j: usize, gain: &impl Fn(usize) -> f32) {
    let count = src.len() / ch;
    let dst = &mut out[2 * j..2 * (j + count)];
    if ch == 1 {
        for (i, (d, s)) in dst.chunks_exact_mut(2).zip(src).enumerate() {
            let g = gain(j + i) * *s;
            d[0] += g;
            d[1] += g;
        }
    } else {
        for (i, (d, s)) in dst.chunks_exact_mut(2).zip(src.chunks_exact(2)).enumerate() {
            let g = gain(j + i);
            d[0] += g * s[0];
            d[1] += g * s[1];
        }
    }
}

/// A recording in progress: input frame `g` is the take's frame `g - g0`.
#[derive(Clone, Copy, Debug)]
struct Rec {
    take: u16,
    g0: u64,
    /// Song tick at `g0`.
    t_in: f64,
}

/// A recording past its punch out, still writing the input that belongs
/// before it (the latency), until frame `stop_at`.
#[derive(Clone, Copy, Debug)]
struct Tail {
    take: u16,
    stop_at: u64,
}

struct Track {
    regions: [Region; MAX_REGIONS],
    n: usize,
    armed: bool,
    input: Input,
    monitor: Monitor,
    level: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    rec: Option<Rec>,
    tail: Option<Tail>,
    /// The memory ran out: no new take until the transport stops
    /// recording.
    blocked: bool,
    /// Gains (left, right) at the end of the last period, for the ramp.
    gains: [f32; 2],
    buf: Vec<f32>,
    /// Records from inside, not the line input (the instrument redesign's
    /// step 9): the host's block for it ([`AudioTracks::inside_input`]),
    /// as late as this many frames, which a take makes up in place of the
    /// line input's latency. It is never heard through the track.
    inside: Option<u32>,
    /// Files on and armed: the lasting id its next take records under,
    /// whose files the host has been asked to make ([`Io::Arm`]), and the
    /// channels they were made for. None otherwise.
    next: Option<(RecordingId, u8)>,
}

impl Track {
    fn new() -> Track {
        Track {
            regions: [Region::EMPTY; MAX_REGIONS],
            n: 0,
            armed: false,
            input: Input::Left,
            monitor: Monitor::Auto,
            level: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            rec: None,
            tail: None,
            blocked: false,
            gains: [1.0, 1.0],
            buf: vec![0.0; 2 * MAX_PERIOD as usize],
            inside: None,
            next: None,
        }
    }

    /// The channels its next take records: from inside, the mix in
    /// stereo; else as its input says.
    fn channels(&self) -> u8 {
        match self.inside {
            Some(_) => 2,
            None => self.input.channels() as u8,
        }
    }

    fn regions(&self) -> &[Region] {
        &self.regions[..self.n]
    }

    fn at(&self, t: Tick) -> Option<usize> {
        self.regions().iter().position(|r| r.contains(t))
    }

    /// Regions the clear of `from..to` would leave, or None if they would
    /// not fit (with `extra` more to come).
    fn clear_fits(&self, from: Tick, to: Tick, extra: usize) -> bool {
        let splits = self
            .regions()
            .iter()
            .filter(|r| r.start < from && r.end > to)
            .count();
        self.n + splits + extra <= MAX_REGIONS
    }

    /// Take `from..to` out of the regions: ones inside go, ones across an
    /// edge are cut there (a cut start moves `src` on), and one across the
    /// whole stretch becomes two. Checked by `clear_fits` first.
    fn clear(&mut self, map: &TempoMap, rate: u32, from: Tick, to: Tick) {
        if to <= from {
            return;
        }
        let mut i = 0;
        while i < self.n {
            let r = self.regions[i];
            if r.end <= from || r.start >= to {
                i += 1;
                continue;
            }
            if r.start >= from && r.end <= to {
                self.regions.copy_within(i + 1..self.n, i);
                self.n -= 1;
                continue;
            }
            if r.start < from && r.end > to {
                let mut right = r;
                right.start = to;
                right.src =
                    r.src + frames_from(map, r.start as f64, to as f64, rate).round() as i64;
                self.regions[i].end = from;
                self.regions.copy_within(i + 1..self.n, i + 2);
                self.regions[i + 1] = right;
                self.n += 1;
                i += 2;
                continue;
            }
            if r.start < from {
                self.regions[i].end = from;
            } else {
                self.regions[i].start = to;
                self.regions[i].src =
                    r.src + frames_from(map, r.start as f64, to as f64, rate).round() as i64;
            }
            i += 1;
        }
    }

    /// Put `r` over whatever is in its stretch. False (nothing changed) if
    /// it would not fit.
    fn put(&mut self, map: &TempoMap, rate: u32, r: Region) -> bool {
        if r.end <= r.start || !self.clear_fits(r.start, r.end, 1) {
            return false;
        }
        self.clear(map, rate, r.start, r.end);
        let i = self.regions().partition_point(|x| x.start < r.start);
        self.regions.copy_within(i..self.n, i + 1);
        self.regions[i] = r;
        self.n += 1;
        true
    }
}

/// One track of a step of the history: its regions before (or, once
/// undone, after) an edit.
struct Snap {
    track: usize,
    n: usize,
    /// The step it belongs to: the snaps of one step share it (a song time
    /// edit changes several tracks at once), and it goes up by one a step.
    step: u32,
    /// The song edit the step belongs to (0: none): see
    /// [`Command::Retime`].
    song: u32,
    regions: [Region; MAX_REGIONS],
}

impl Snap {
    fn new() -> Snap {
        Snap {
            track: 0,
            n: 0,
            step: 0,
            song: 0,
            regions: [Region::EMPTY; MAX_REGIONS],
        }
    }
}

/// Frames from tick `a` to tick `b` (fractional) along the tempo map, as
/// the transport plays it (tempo ramps included).
pub fn frames_from(map: &TempoMap, a: f64, b: f64, rate: u32) -> f64 {
    map.frames_between(a, b, rate)
}

/// The tick (fractional) `frames` frames after tick `a` along the tempo map.
pub fn tick_after(map: &TempoMap, a: f64, frames: f64, rate: u32) -> f64 {
    map.tick_after(a, frames, rate)
}

/// Equal-power fade-in over [`FADE`] frames: `xfade_in(j)` for frame `j`
/// of the fade; the matching fade-out is `xfade_in(FADE - j)`.
fn fade_table() -> [f32; FADE + 1] {
    let mut t = [0.0; FADE + 1];
    for (j, v) in t.iter_mut().enumerate() {
        *v = ((j as f64 / FADE as f64) * std::f64::consts::FRAC_PI_2).sin() as f32;
    }
    t
}

/// The audio tracks: see the module notes.
pub struct AudioTracks {
    rate: u32,
    /// Where new takes' lasting ids come from.
    ids: Ids,
    /// [`AudioTracks::set_only`]: the one track heard (an export's replay).
    only: Option<usize>,
    pool: Pool,
    takes: Vec<Take>,
    tracks: Vec<Track>,
    queue: [Option<Command>; QUEUE],
    q_len: usize,
    latency: u32,
    active: bool,
    groups: u16,
    hist: Vec<Snap>,
    /// The track's regions before the edit in progress.
    scratch: Box<Snap>,
    /// A loop jump ended these tracks' takes: the next pass (group, pass)
    /// starts where the song lands.
    pending: [Option<(u16, u16)>; MAX_TRACKS],
    h_base: usize,
    h_done: usize,
    h_redo: usize,
    /// The last step's number ([`Snap::step`]).
    step: u32,
    /// The tempo map of the last period run: the one the regions were
    /// placed on, for a song time edit whose view brings a new one.
    last_map: TempoMap,
    stats: Stats,
    /// Takes whose saved audio is copying in (room for all, made once).
    loads: Vec<(u16, Vec<f32>)>,
    /// Loaded takes' audio, done with, for the host to drop off the audio
    /// thread.
    returned: Vec<Vec<f32>>,
    fade: [f32; FADE + 1],
    /// Recording to files (see the module notes), and what the host is
    /// asked to do about them.
    files: bool,
    io: VecDeque<Io>,
    /// Each track's block from inside (step 9), stereo interleaved,
    /// [`MAX_PERIOD`] frames a track, which the host fills before
    /// [`AudioTracks::process`].
    inside: Box<[f32]>,
    /// The stream layer's sound-thread side, for takes played from their
    /// files ([`AudioTracks::stream`]), and room for what a run reads
    /// through it.
    player: Option<Box<Player>>,
    reads: Box<[f32]>,
    /// Where each track plays, or is about to (see [`AudioTracks::cue`]),
    /// a region in each of its two cursors.
    cues: [[Option<Cue>; 2]; MAX_TRACKS],
    /// Take ids kept for takes on their way ([`AudioTracks::reserve`]).
    reserved: [bool; MAX_TAKES],
}

/// One track's room in [`AudioTracks::inside_input`].
const INSIDE: usize = 2 * MAX_PERIOD as usize;

impl AudioTracks {
    /// Audio tracks at `rate` with memory for `seconds` of stereo audio
    /// (twice that of mono takes), all allocated here.
    pub fn new(rate: u32, seconds: u32) -> AudioTracks {
        let samples = rate as usize * seconds as usize * 2;
        let hist = (0..HISTORY).map(|_| Snap::new()).collect();
        AudioTracks {
            rate,
            ids: Ids::new(),
            only: None,
            pool: Pool::new(samples),
            takes: vec![NO_TAKE; MAX_TAKES],
            tracks: (0..MAX_TRACKS).map(|_| Track::new()).collect(),
            queue: [None; QUEUE],
            q_len: 0,
            latency: 0,
            active: true,
            groups: 0,
            hist,
            scratch: Box::new(Snap::new()),
            step: 0,
            last_map: TempoMap::constant(120.0).expect("a valid tempo"),
            pending: [None; MAX_TRACKS],
            h_base: 0,
            h_done: 0,
            h_redo: 0,
            stats: Stats::default(),
            loads: Vec::with_capacity(MAX_TAKES),
            returned: Vec::with_capacity(MAX_TAKES),
            fade: fade_table(),
            files: false,
            io: VecDeque::with_capacity(IO_QUEUE),
            inside: vec![0.0; MAX_TRACKS * INSIDE].into_boxed_slice(),
            player: None,
            reads: vec![0.0; 2 * RUN].into_boxed_slice(),
            cues: [[None; 2]; MAX_TRACKS],
            reserved: [false; MAX_TAKES],
        }
    }

    /// Where the host puts track `t`'s block from inside before each
    /// [`AudioTracks::process`] (stereo interleaved, as the line input),
    /// while the track records from inside (step 9). None past the last
    /// track. Real-time safe.
    pub fn inside_input(&mut self, t: usize) -> Option<&mut [f32]> {
        self.inside.get_mut(t * INSIDE..(t + 1) * INSIDE)
    }

    /// Whether track `t` records from inside (step 9).
    pub fn records_inside(&self, t: usize) -> bool {
        self.tracks.get(t).is_some_and(|tr| tr.inside.is_some())
    }

    /// Only track `only` is heard (None: every track, as their mutes and
    /// solos say): an export's replay of one audio track. The others go
    /// quiet at once, with no fade, so none of them is in the replay's
    /// first frames; they fade back in when it ends. Real-time safe.
    pub fn set_only(&mut self, only: Option<usize>) {
        if only == self.only {
            return;
        }
        self.only = only;
        if let Some(o) = only {
            for (k, tr) in self.tracks.iter_mut().enumerate() {
                if k != o {
                    tr.gains = [0.0; 2];
                }
            }
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Queue a command for the next period. False if the queue is full.
    pub fn command(&mut self, c: Command) -> bool {
        if self.q_len == QUEUE {
            self.stats.dropped += 1;
            return false;
        }
        self.queue[self.q_len] = Some(c);
        self.q_len += 1;
        true
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn memory(&self) -> Memory {
        Memory {
            used: self.pool.used(),
            capacity: self.pool.capacity(),
        }
    }

    pub fn latency(&self) -> u32 {
        self.latency
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// The regions of a track, in order (empty past the last track).
    pub fn regions(&self, track: usize) -> &[Region] {
        self.tracks.get(track).map_or(&[], Track::regions)
    }

    pub fn armed(&self, track: usize) -> bool {
        self.tracks.get(track).is_some_and(|t| t.armed)
    }

    /// The take a track is recording, if any.
    pub fn recording(&self, track: usize) -> Option<u16> {
        self.tracks.get(track)?.rec.map(|r| r.take)
    }

    /// A track's settings (None past the last track).
    pub fn settings(&self, track: usize) -> Option<TrackSettings> {
        let t = self.tracks.get(track)?;
        Some(TrackSettings {
            input: t.input,
            monitor: t.monitor,
            level: t.level,
            pan: t.pan,
            mute: t.mute,
            solo: t.solo,
        })
    }

    pub fn take(&self, id: u16) -> Option<TakeInfo> {
        let t = self.takes.get(id as usize).filter(|t| t.live)?;
        Some(TakeInfo {
            id,
            rec: t.rec,
            track: t.track as usize,
            group: t.group,
            pass: t.pass,
            bpm: t.bpm,
            anchor: t.anchor,
            frames: t.frames,
            channels: t.channels as usize,
            recording: t.recording,
            cut_short: t.cut_short,
        })
    }

    /// The take whose lasting id is `rec`, if it is here.
    pub fn take_of(&self, rec: RecordingId) -> Option<u16> {
        if !rec.is_given() {
            return None;
        }
        self.takes
            .iter()
            .position(|t| t.live && t.rec == rec)
            .map(|i| i as u16)
    }

    /// The lasting id a take loaded or brought in keeps: its own when it
    /// has one no other take has, else a new one (a part saved before
    /// takes had them; two takes never share one).
    fn keep_id(&mut self, rec: RecordingId) -> RecordingId {
        if rec.is_given() && self.take_of(rec).is_none() {
            rec
        } else {
            self.ids.fresh()
        }
    }

    /// A new lasting id, never given before in this run, for a take made
    /// off the audio thread whose file is named for it before it comes in
    /// (a sound brought in, or a stretched copy, while takes play from
    /// their files). Does not allocate.
    pub fn fresh_rec(&mut self) -> RecordingId {
        self.ids.fresh()
    }

    /// An id no take has, for a take made off the audio thread (a stretch)
    /// to load under.
    pub fn free_take_id(&self) -> Option<u16> {
        self.free_id().map(|i| i as u16)
    }

    /// The first take id neither in use nor kept for a take on its way
    /// ([`AudioTracks::reserve`]).
    fn free_id(&self) -> Option<usize> {
        (0..MAX_TAKES).find(|&i| !self.takes[i].live && !self.reserved[i])
    }

    /// Keep take id `id` for a take on its way (a project's take whose
    /// file is opening: [`AudioTracks::add_file_take`] takes it), so no
    /// recording or sound brought in gets it meanwhile; or let it go.
    /// [`AudioTracks::clear`] lets every id go. Real-time safe.
    pub fn reserve(&mut self, id: u16, keep: bool) {
        if let Some(r) = self.reserved.get_mut(id as usize) {
            *r = keep;
        }
    }

    /// Every take, oldest id first.
    pub fn takes(&self) -> impl Iterator<Item = TakeInfo> + '_ {
        (0..MAX_TAKES as u16).filter_map(|id| self.take(id))
    }

    /// Frame `k`, channel `c` of a take (a mono take answers for both).
    pub fn sample(&self, id: u16, k: usize, c: usize) -> Option<f32> {
        self.pooled(id)?.sample(&self.pool, id, k, c)
    }

    /// A take whose audio is in the pool (not played from its file).
    fn pooled(&self, id: u16) -> Option<&Take> {
        self.takes
            .get(id as usize)
            .filter(|t| t.live && matches!(t.store, Store::Pool | Store::Part { freed: 0, .. }))
    }

    /// The lowest and highest sample of frames `from..to` of a take, over
    /// its channels: for a waveform on the screen. It reads the peak table
    /// for whole blocks of [`PEAK_BLOCK`] samples, so a long stretch costs
    /// little; it works while the take records (up to what is written) and
    /// does not allocate. A host shares the result with the screen thread
    /// as it shares the rest of the state.
    pub fn peaks(&self, id: u16, from: usize, to: usize) -> Option<(f32, f32)> {
        let t = self.pooled(id)?;
        let to = to.min(t.frames);
        if from >= to {
            return None;
        }
        let ch = t.channels as usize;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut any = false;
        let (mut s, end) = (from * ch, to * ch);
        while s < end {
            let off = s % CHUNK;
            let count = (CHUNK - off).min(end - s);
            if let Some(slot) = self.pool.readable(id, (s / CHUNK) as u32) {
                let (l, h) = self.pool.range_peaks(slot * CHUNK + off, count);
                lo = lo.min(l);
                hi = hi.max(h);
                any = true;
            }
            s += count;
        }
        any.then_some((lo, hi))
    }

    /// Steps Undo can take back and Redo can bring back.
    pub fn history(&self) -> (usize, usize) {
        (self.h_done, self.h_redo)
    }

    /// Whether a region, or a step of the history, plays this take.
    pub fn in_use(&self, id: u16) -> bool {
        let uses = |rs: &[Region]| rs.iter().any(|r| r.take == id);
        self.tracks.iter().any(|t| uses(t.regions()))
            || (0..self.h_done + self.h_redo).any(|i| {
                let s = &self.hist[(self.h_base + i) % HISTORY];
                uses(&s.regions[..s.n])
            })
    }

    /// A track's sound in the last period: interleaved stereo, after its
    /// level, pan, mute and solo (for the host's mixer strip).
    pub fn track_out(&self, track: usize, frames: usize) -> &[f32] {
        self.tracks
            .get(track)
            .map_or(&[], |t| &t.buf[..2 * frames.min(MAX_PERIOD as usize)])
    }

    /// The take as a 32-bit float WAV (for the store). Allocates: not on
    /// the audio thread.
    pub fn take_wav(&self, id: u16) -> Option<Vec<u8>> {
        let t = self.pooled(id)?;
        let ch = t.channels as usize;
        let bytes = t.frames * ch * 4;
        let mut w = Vec::with_capacity(44 + bytes);
        let u32le = |w: &mut Vec<u8>, v: u32| w.extend_from_slice(&v.to_le_bytes());
        let u16le = |w: &mut Vec<u8>, v: u16| w.extend_from_slice(&v.to_le_bytes());
        w.extend_from_slice(b"RIFF");
        u32le(&mut w, 36 + bytes as u32);
        w.extend_from_slice(b"WAVEfmt ");
        u32le(&mut w, 16);
        u16le(&mut w, 3);
        u16le(&mut w, ch as u16);
        u32le(&mut w, self.rate);
        u32le(&mut w, self.rate * ch as u32 * 4);
        u16le(&mut w, ch as u16 * 4);
        u16le(&mut w, 32);
        w.extend_from_slice(b"data");
        u32le(&mut w, bytes as u32);
        for k in 0..t.frames {
            for c in 0..ch {
                let s = t.sample(&self.pool, id, k, c).unwrap_or(0.0);
                w.extend_from_slice(&s.to_le_bytes());
            }
        }
        Some(w)
    }

    // ---- commands ------------------------------------------------------

    fn refuse(&mut self) {
        self.stats.refused += 1;
    }

    /// A new step's number, for the snaps [`AudioTracks::push_history`]
    /// puts in it.
    fn next_step(&mut self) -> u32 {
        self.step = self.step.wrapping_add(1);
        self.step
    }

    /// Save the scratch copy (a track's regions before an edit) in undo
    /// step `step`, of song edit `song` (0: none), dropping anything Redo
    /// could bring back and, when full, the oldest step (all its tracks).
    fn push_history(&mut self, step: u32, song: u32) {
        if self.h_done == HISTORY {
            let oldest = self.hist[self.h_base].step;
            while self.h_done > 0 && self.hist[self.h_base].step == oldest {
                self.h_base = (self.h_base + 1) % HISTORY;
                self.h_done -= 1;
            }
        }
        let idx = (self.h_base + self.h_done) % HISTORY;
        let (src, dst) = (&*self.scratch, &mut self.hist[idx]);
        dst.track = src.track;
        dst.n = src.n;
        dst.step = step;
        dst.song = song;
        dst.regions[..src.n].copy_from_slice(&src.regions[..src.n]);
        self.h_done += 1;
        self.h_redo = 0;
    }

    fn swap_history(&mut self, idx: usize) {
        let s = &mut self.hist[idx];
        let t = &mut self.tracks[s.track];
        let n = s.n.max(t.n);
        for i in 0..n {
            std::mem::swap(&mut s.regions[i], &mut t.regions[i]);
        }
        std::mem::swap(&mut s.n, &mut t.n);
    }

    /// The snap Undo takes next and the one Redo brings next.
    fn undo_snap(&self) -> Option<&Snap> {
        let i = self.h_done.checked_sub(1)?;
        Some(&self.hist[(self.h_base + i) % HISTORY])
    }

    fn redo_snap(&self) -> Option<&Snap> {
        (self.h_redo > 0).then(|| &self.hist[(self.h_base + self.h_done) % HISTORY])
    }

    /// Take back the step on top, every track of it.
    fn undo_step(&mut self) {
        let Some(step) = self.undo_snap().map(|s| s.step) else {
            return;
        };
        while self.undo_snap().is_some_and(|s| s.step == step) {
            self.h_done -= 1;
            self.h_redo += 1;
            self.swap_history((self.h_base + self.h_done) % HISTORY);
        }
    }

    /// Bring back the step Redo brings next, every track of it.
    fn redo_step(&mut self) {
        let Some(step) = self.redo_snap().map(|s| s.step) else {
            return;
        };
        while self.redo_snap().is_some_and(|s| s.step == step) {
            self.swap_history((self.h_base + self.h_done) % HISTORY);
            self.h_done += 1;
            self.h_redo -= 1;
        }
    }

    /// The song edit whose step Undo would take next (Undo stops there:
    /// it is the app's UNDO's), if it is one.
    pub fn undo_is_song(&self) -> Option<u32> {
        self.undo_snap().map(|s| s.song).filter(|&s| s != 0)
    }

    /// The same for Redo.
    pub fn redo_is_song(&self) -> Option<u32> {
        self.redo_snap().map(|s| s.song).filter(|&s| s != 0)
    }

    /// Where song edit `song`'s step is: next for Undo (`undo`) or Redo,
    /// under other steps, or gone.
    pub fn song_step(&self, song: u32, undo: bool) -> SongStep {
        let next = if undo {
            self.undo_snap()
        } else {
            self.redo_snap()
        };
        if song != 0 && next.is_some_and(|s| s.song == song) {
            return SongStep::Next;
        }
        let (from, n) = if undo {
            (0, self.h_done)
        } else {
            (self.h_done, self.h_redo)
        };
        let held = (from..from + n).any(|i| self.hist[(self.h_base + i) % HISTORY].song == song);
        if song != 0 && held {
            SongStep::Under
        } else {
            SongStep::Gone
        }
    }

    /// Drop song edit `song`'s steps and every step only reachable
    /// through them: on the Undo side the newest of them and all older
    /// steps, on the Redo side the first of them and all steps redone
    /// after it ([`Command::ForgetSong`]).
    fn forget_song(&mut self, song: u32) {
        if song == 0 {
            return self.refuse();
        }
        let at = |i: usize| self.hist[(self.h_base + i) % HISTORY].song == song;
        let undo = (0..self.h_done).rev().find(|&i| at(i));
        let redo = (self.h_done..self.h_done + self.h_redo).find(|&i| at(i));
        if undo.is_none() && redo.is_none() {
            return self.refuse();
        }
        if let Some(j) = redo {
            self.h_redo = j - self.h_done;
        }
        if let Some(i) = undo {
            self.h_base = (self.h_base + i + 1) % HISTORY;
            self.h_done -= i + 1;
        }
    }

    /// Whether a command sent now might not do what the tracks say now
    /// ([`AudioTracks::retime_refused`], [`AudioTracks::song_step`]): one
    /// waiting for the next period changes the regions or the history
    /// (or may: an arm can end a recording), or the queue is full.
    /// Settings waiting (input, monitor, level, pan, mute, solo, latency)
    /// don't count.
    pub fn busy(&self) -> bool {
        self.q_len == QUEUE
            || self.queue[..self.q_len].iter().flatten().any(|c| {
                !matches!(
                    c,
                    Command::SetInput(..)
                        | Command::SetMonitor(..)
                        | Command::SetLevel(..)
                        | Command::SetPan(..)
                        | Command::Mute(..)
                        | Command::Solo(..)
                        | Command::SetLatency(_)
                )
            })
    }

    /// Whether any step of the history, done or undone, belongs to a song
    /// edit.
    pub fn has_song_steps(&self) -> bool {
        (0..self.h_done + self.h_redo).any(|i| self.hist[(self.h_base + i) % HISTORY].song != 0)
    }

    /// Whether any track holds a region.
    pub fn has_regions(&self) -> bool {
        self.tracks.iter().any(|t| t.n > 0)
    }

    /// Why the tracks can't take song time edit `r` now, if they can't: a
    /// track records, or a track's regions would not fit. For the app to
    /// ask before it edits the song, so a refusal changes nothing
    /// anywhere.
    pub fn retime_refused(&self, r: &Retime) -> Option<RetimeRefused> {
        for (k, t) in self.tracks.iter().enumerate() {
            if t.rec.is_some() {
                return Some(RetimeRefused::Recording(k));
            }
            if r.count(t.regions()) > MAX_REGIONS {
                return Some(RetimeRefused::Full(k));
            }
        }
        None
    }

    /// Song time edit `r` of song edit `song` on every track it changes,
    /// as one step (see [`Command::Retime`]). False, nothing changed, when
    /// a track records or one's regions would not fit.
    fn retime(&mut self, song: u32, r: &Retime) -> bool {
        if song == 0 || self.retime_refused(r).is_some() {
            return false;
        }
        let step = self.next_step();
        let (map, rate) = (self.last_map, self.rate);
        for k in 0..MAX_TRACKS {
            if !r.changes(self.tracks[k].regions()) {
                continue;
            }
            {
                let (t, s) = (&self.tracks[k], &mut *self.scratch);
                s.track = k;
                s.n = t.n;
                s.regions[..t.n].copy_from_slice(t.regions());
            }
            let s = &*self.scratch;
            let t = &mut self.tracks[k];
            match r.apply(&s.regions[..s.n], &map, rate, &mut t.regions) {
                Some(n) => t.n = n,
                // Checked above: it fits.
                None => {
                    t.regions[..s.n].copy_from_slice(&s.regions[..s.n]);
                    t.n = s.n;
                    continue;
                }
            }
            self.push_history(step, song);
        }
        true
    }

    /// An edit of one track's regions: `f` changes them and says whether it
    /// did; if so, the regions before it become an undo step, and if not,
    /// they are put back as they were.
    fn edit(
        &mut self,
        track: usize,
        f: impl FnOnce(&mut Track, &TempoMap, u32) -> bool,
        map: &TempoMap,
    ) {
        if track >= MAX_TRACKS {
            return self.refuse();
        }
        {
            let (t, s) = (&self.tracks[track], &mut *self.scratch);
            s.track = track;
            s.n = t.n;
            s.regions[..t.n].copy_from_slice(t.regions());
        }
        let rate = self.rate;
        if f(&mut self.tracks[track], map, rate) {
            let step = self.next_step();
            self.push_history(step, 0);
        } else {
            let (t, s) = (&mut self.tracks[track], &*self.scratch);
            t.n = s.n;
            t.regions[..s.n].copy_from_slice(&s.regions[..s.n]);
            self.refuse();
        }
    }

    fn apply(&mut self, c: Command, map: &TempoMap, now: u64) {
        let ok_track = |t: usize| t < MAX_TRACKS;
        match c {
            Command::Arm(t, on) if ok_track(t) => {
                self.tracks[t].armed = on;
                if !on {
                    self.punch_out(t, map, now, None);
                }
                self.ready_next(t);
            }
            Command::SetInput(t, i) if ok_track(t) => {
                self.tracks[t].input = i;
                self.ready_next(t);
            }
            Command::SetInside(t, lat) if ok_track(t) && self.tracks[t].rec.is_none() => {
                self.tracks[t].inside = lat.map(|l| l.min(MAX_PERIOD * 8));
                self.ready_next(t);
            }
            Command::SetMonitor(t, m) if ok_track(t) => self.tracks[t].monitor = m,
            Command::SetLevel(t, l) if ok_track(t) && l.is_finite() => {
                self.tracks[t].level = l.clamp(0.0, 2.0)
            }
            Command::SetPan(t, p) if ok_track(t) && p.is_finite() => {
                self.tracks[t].pan = p.clamp(-1.0, 1.0)
            }
            Command::Mute(t, m) if ok_track(t) => self.tracks[t].mute = m,
            Command::Solo(t, s) if ok_track(t) => self.tracks[t].solo = s,
            Command::SetLatency(l) => self.latency = l.min(MAX_PERIOD * 4),
            Command::SetActive(a) => {
                self.active = a;
                if !a {
                    for t in 0..MAX_TRACKS {
                        self.punch_out(t, map, now, None);
                    }
                }
            }
            Command::Comp { take, from, to } => {
                let Some(tk) = self.takes.get(take as usize).filter(|t| t.live).copied() else {
                    return self.refuse();
                };
                let rate = self.rate;
                let a = tk.anchor;
                let b = tick_after(map, a, tk.frames as f64, rate);
                let from = from.max(a.ceil() as Tick);
                let to = to.min(b.floor() as Tick);
                if to <= from {
                    return self.refuse();
                }
                let r = Region {
                    start: from,
                    end: to,
                    take,
                    src: frames_from(map, a, from as f64, rate).round() as i64,
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
                };
                self.edit(tk.track as usize, |t, m, rate| t.put(m, rate, r), map);
            }
            Command::Split { track, at } if ok_track(track) => {
                let Some(i) = self.tracks[track]
                    .at(at)
                    .filter(|&i| self.tracks[track].regions[i].start < at)
                else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, m, rate| {
                        if t.n == MAX_REGIONS {
                            return false;
                        }
                        let r = t.regions[i];
                        let mut right = r;
                        right.start = at;
                        right.src =
                            r.src + frames_from(m, r.start as f64, at as f64, rate).round() as i64;
                        right.fade_in = 0;
                        t.regions[i].end = at;
                        t.regions[i].fade_out = 0;
                        t.regions.copy_within(i + 1..t.n, i + 2);
                        t.regions[i + 1] = right;
                        t.n += 1;
                        true
                    },
                    map,
                );
            }
            Command::Erase { track, from, to } if ok_track(track) && from < to => {
                if !self.tracks[track]
                    .regions()
                    .iter()
                    .any(|r| r.start < to && r.end > from)
                {
                    return self.refuse();
                }
                self.edit(
                    track,
                    |t, m, rate| {
                        t.clear_fits(from, to, 0) && {
                            t.clear(m, rate, from, to);
                            true
                        }
                    },
                    map,
                );
            }
            Command::Move { track, at, to } if ok_track(track) && to >= 0 => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, m, rate| {
                        let mut r = t.regions[i];
                        t.regions.copy_within(i + 1..t.n, i);
                        t.n -= 1;
                        r.end = to + (r.end - r.start);
                        r.start = to;
                        t.put(m, rate, r)
                    },
                    map,
                );
            }
            Command::SetGain { track, at, gain } if ok_track(track) && gain.is_finite() => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, _, _| {
                        t.regions[i].gain = gain.clamp(0.0, 4.0);
                        true
                    },
                    map,
                );
            }
            Command::SetFades {
                track,
                at,
                fade_in,
                fade_out,
            } if ok_track(track) => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, _, _| {
                        t.regions[i].fade_in = fade_in;
                        t.regions[i].fade_out = fade_out;
                        true
                    },
                    map,
                );
            }
            Command::Undo => {
                if self.h_done == 0 || self.any_recording() || self.undo_is_song().is_some() {
                    return self.refuse();
                }
                self.undo_step();
            }
            Command::Redo => {
                if self.h_redo == 0 || self.any_recording() || self.redo_is_song().is_some() {
                    return self.refuse();
                }
                self.redo_step();
            }
            Command::Retime { song, retime } => {
                if !self.retime(song, &retime) {
                    self.refuse();
                }
            }
            Command::SongUndo(song) => {
                if self.any_recording() || self.song_step(song, true) != SongStep::Next {
                    return self.refuse();
                }
                while self.undo_is_song() == Some(song) {
                    self.undo_step();
                }
            }
            Command::SongRedo(song) => {
                if self.any_recording() || self.song_step(song, false) != SongStep::Next {
                    return self.refuse();
                }
                while self.redo_is_song() == Some(song) {
                    self.redo_step();
                }
            }
            Command::ForgetSong(song) => self.forget_song(song),
            Command::Forget(id) => {
                let ok = self
                    .takes
                    .get(id as usize)
                    .is_some_and(|t| t.live && !t.recording && !t.loading && !t.writing);
                if !ok || self.in_use(id) {
                    return self.refuse();
                }
                self.forget(id);
            }
            Command::OnFile {
                take,
                rec,
                gen,
                growing,
            } => {
                let ok = self.player.is_some()
                    && self.takes.get(take as usize).is_some_and(|t| {
                        t.live
                            && t.rec == rec
                            && t.store == Store::Pool
                            && !t.loading
                            && (growing || (!t.recording && !t.writing))
                    });
                if !ok {
                    return self.refuse();
                }
                self.takes[take as usize].store = Store::Part {
                    gen,
                    freed: 0,
                    growing,
                };
            }
            Command::Swap { old, new, ratio } => {
                let ok = |id: u16| {
                    self.takes
                        .get(id as usize)
                        .filter(|t| t.live && !t.recording && !t.loading)
                        .map(|t| t.track as usize)
                };
                let (Some(a), Some(b)) = (ok(old), ok(new)) else {
                    return self.refuse();
                };
                if a != b || old == new || !ratio.is_finite() || ratio <= 0.0 {
                    return self.refuse();
                }
                self.edit(
                    a,
                    |t, _, _| {
                        let mut any = false;
                        for r in &mut t.regions[..t.n] {
                            if r.take == old {
                                r.take = new;
                                r.src = (r.src as f64 * ratio).round() as i64;
                                any = true;
                            }
                        }
                        any
                    },
                    map,
                );
            }
            Command::ClearHistory => {
                self.h_done = 0;
                self.h_redo = 0;
            }
            _ => self.refuse(),
        }
    }

    fn any_recording(&self) -> bool {
        self.tracks.iter().any(|t| t.rec.is_some())
    }

    // ---- saving and loading --------------------------------------------

    /// Whether every take has settled for a save: no track records or has
    /// a recording's tail still coming, and no take is loading. Does not
    /// allocate.
    pub fn settled(&self) -> bool {
        !self
            .tracks
            .iter()
            .any(|t| t.rec.is_some() || t.tail.is_some())
            && !self.takes.iter().any(|t| t.loading)
    }

    /// The tracks' settings and regions and every take's place, for a
    /// save (see [`save`]): None until every take has settled
    /// ([`AudioTracks::settled`]). Tracks with nothing to keep (no
    /// regions, no takes, settings as new) are left out. The takes' audio
    /// is not in it: read each with [`AudioTracks::read_take`]. Allocates:
    /// not on the audio thread (that fills a room with
    /// [`AudioTracks::save_into`]).
    pub fn save_file(&self) -> Option<Saved> {
        let mut s = Saved::room(self.rate);
        let (tracks, takes) = self.save_into(&mut s)?;
        s.tracks.truncate(tracks);
        s.takes.truncate(takes);
        Some(s)
    }

    /// [`AudioTracks::save_file`] into `room` ([`Saved::room`]), without
    /// allocating: the first `tracks` of `room.tracks` and `takes` of
    /// `room.takes` are the save, the rest is room to be trimmed off the
    /// audio thread. None until every take has settled, or if `room` is
    /// smaller than a [`Saved::room`] (it is left as it was). Each take's
    /// `file` is emptied, keeping its room.
    pub fn save_into(&self, room: &mut Saved) -> Option<(usize, usize)> {
        if !self.settled()
            || room.tracks.len() < MAX_TRACKS
            || room.takes.len() < MAX_TAKES
            || room
                .tracks
                .iter()
                .any(|t| t.regions.capacity() < MAX_REGIONS)
        {
            return None;
        }
        room.rate = self.rate;
        let mut takes = 0;
        for k in self.takes() {
            let f = &mut room.takes[takes];
            f.id = k.id;
            f.rec = k.rec;
            f.track = k.track;
            f.group = k.group;
            f.pass = k.pass;
            f.bpm = k.bpm;
            f.anchor = k.anchor;
            f.frames = k.frames;
            f.channels = k.channels;
            f.file.clear();
            takes += 1;
        }
        let mut tracks = 0;
        for (i, t) in self.tracks.iter().enumerate() {
            let keep = t.n > 0
                || room.takes[..takes].iter().any(|k| k.track == i)
                || t.input != Input::Left
                || t.monitor != Monitor::Auto;
            if !keep {
                continue;
            }
            let f = &mut room.tracks[tracks];
            f.index = i;
            f.input = t.input;
            f.monitor = t.monitor;
            f.from.clear();
            f.regions.clear();
            f.regions.extend_from_slice(t.regions());
            tracks += 1;
        }
        Some((tracks, takes))
    }

    /// Copy frames `from..` of a take into `out` (interleaved, the take's
    /// channels), as many as fit and it has: the number of frames copied,
    /// or None if the take is not there, is loading, or has `from` on its
    /// file rather than in the pool ([`AudioTracks::file_frames`]). A take
    /// recording gives the frames it has so far (they never change once
    /// written). Does not allocate.
    pub fn read_take(&self, id: u16, from: usize, out: &mut [f32]) -> Option<usize> {
        let t = self
            .takes
            .get(id as usize)
            .filter(|t| t.live && !t.loading && from >= Self::on_file_of(t))?;
        let ch = t.channels as usize;
        let want = (out.len() / ch).min(t.frames.saturating_sub(from));
        let mut done = 0;
        while done < want {
            let s = (from + done) * ch;
            let off = s % CHUNK;
            let count = ((CHUNK - off) / ch).min(want - done);
            let Some(slot) = self.pool.readable(id, (s / CHUNK) as u32) else {
                break;
            };
            let b = slot * CHUNK + off;
            out[done * ch..(done + count) * ch].copy_from_slice(&self.pool.data[b..b + count * ch]);
            done += count;
        }
        Some(done)
    }

    /// Put a saved take back, under its own id and its lasting id (a new
    /// one if it has none or another take has it): its memory is taken now,
    /// and its audio (moved in, never grown or freed here) copies in at
    /// [`LOAD_SPEED`] over the next periods; it plays once copied. Then
    /// the audio waits in [`AudioTracks::take_returned`] for the host to
    /// drop. Does not allocate. Gives `f` back if the id is past the last
    /// or in use, the track past the last, the audio not `frames *
    /// channels` long, the memory short, or [`MAX_TAKES`] loads waiting
    /// to copy or to be collected.
    // The take comes back whole on a refusal, so its audio is never freed
    // here; boxing it would allocate on the audio thread.
    #[allow(clippy::result_large_err)]
    pub fn load_take(&mut self, f: TakeFile) -> Result<(), TakeFile> {
        let id = f.id as usize;
        let need = f.frames * f.channels;
        if id >= MAX_TAKES
            || self.takes[id].live
            || f.track >= MAX_TRACKS
            || !(1..=2).contains(&f.channels)
            || f.frames == 0
            || f.audio.len() != need
            || !f.anchor.is_finite()
            || self.loads.len() + self.returned.len() >= MAX_TAKES
        {
            return Err(f);
        }
        let rec = self.keep_id(f.rec);
        self.takes[id] = Take {
            live: true,
            rec,
            track: f.track as u8,
            channels: f.channels as u8,
            group: f.group,
            pass: f.pass,
            bpm: if f.bpm.is_finite() && f.bpm > 0.0 {
                f.bpm
            } else {
                0.0
            },
            anchor: f.anchor,
            loading: true,
            ..NO_TAKE
        };
        for c in 0..need.div_ceil(CHUNK) as u32 {
            if self.pool.alloc(f.id, c).is_none() {
                self.pool.release_take(f.id);
                self.takes[id] = NO_TAKE;
                return Err(f);
            }
        }
        self.groups = self.groups.max(f.group);
        self.loads.push((f.id, f.audio));
        Ok(())
    }

    /// Whether a saved track is one [`AudioTracks::load_track`] can put
    /// back (a track that exists, regions that fit, each with a length,
    /// in order and not overlapping), whatever the tracks are doing.
    pub fn track_fits(f: &TrackFile) -> bool {
        f.index < MAX_TRACKS
            && f.regions.len() <= MAX_REGIONS
            && f.regions.iter().all(|r| r.end > r.start)
            && f.regions.windows(2).all(|w| w[0].end <= w[1].start)
    }

    /// Put a saved track's input, monitor and regions back (its takes
    /// loaded with [`AudioTracks::load_take`], before or after); its level,
    /// pan, mute and solo stay as they are, since a save doesn't hold
    /// them (the app keeps them in its mixer and track list). The undo
    /// history is forgotten. Does not allocate. False (nothing changed) if the
    /// track is past the last or recording, or the regions are too many or
    /// out of order.
    pub fn load_track(&mut self, f: &TrackFile) -> bool {
        if !Self::track_fits(f) || self.tracks[f.index].rec.is_some() {
            return false;
        }
        let t = &mut self.tracks[f.index];
        t.input = f.input;
        t.monitor = f.monitor;
        t.n = f.regions.len();
        t.regions[..t.n].copy_from_slice(&f.regions);
        self.h_done = 0;
        self.h_redo = 0;
        true
    }

    /// Every track as new (no regions, unarmed, settings as at start)
    /// and every take forgotten, its memory given back, with the undo
    /// history: New project, or a project opening. The latency, whether
    /// the tracks play and whether they record to files are kept; the
    /// files made for armed tracks go ([`Io::Disarm`]). Refused (false,
    /// nothing changed) while a track records, a take copies in, or a
    /// take is still being copied out to its file. Does not allocate.
    pub fn clear(&mut self) -> bool {
        let busy = self
            .tracks
            .iter()
            .any(|t| t.rec.is_some() || t.tail.is_some())
            || !self.loads.is_empty()
            || self.takes.iter().any(|t| t.live && t.writing);
        if busy {
            return false;
        }
        for t in 0..MAX_TRACKS {
            self.tracks[t].armed = false;
            self.ready_next(t);
        }
        for t in &mut self.tracks {
            t.n = 0;
            t.armed = false;
            t.input = Input::Left;
            t.inside = None;
            t.monitor = Monitor::Auto;
            t.level = 1.0;
            t.pan = 0.0;
            t.mute = false;
            t.solo = false;
            t.blocked = false;
        }
        for id in 0..MAX_TAKES {
            if self.takes[id].live {
                self.forget(id as u16);
            }
        }
        self.pending = [None; MAX_TRACKS];
        self.reserved = [false; MAX_TAKES];
        self.groups = 0;
        self.h_done = 0;
        self.h_redo = 0;
        true
    }

    /// Play take `take` on its track from its anchor to its end, over what
    /// was there, cut at its frames `cuts` (rising; ones outside it are
    /// left out): a region a piece, one undo step. A sound brought in with
    /// its slices ([`host::AudioHost::import`]): each slice is a region of
    /// its own to move, erase or turn up. Applies at once (not queued), so
    /// call it between periods, as the host does. False, counted as
    /// refused and changing nothing, if the take is not there, records or
    /// copies in still, or the pieces don't fit on the track.
    pub fn place(&mut self, take: u16, cuts: &[u64], map: &TempoMap) -> bool {
        let Some(tk) = self
            .takes
            .get(take as usize)
            .filter(|t| t.live && !t.recording && !t.loading)
            .copied()
        else {
            self.refuse();
            return false;
        };
        let (rate, a, frames) = (self.rate, tk.anchor, tk.frames as u64);
        let tick = move |f: u64| tick_after(map, a, f as f64, rate);
        let ends = cuts
            .iter()
            .filter(|&&c| 0 < c && c < frames)
            .map(|&c| tick(c).round() as Tick)
            .chain(std::iter::once(tick(frames).floor() as Tick));
        let before = self.stats.refused;
        self.edit(
            tk.track as usize,
            |t, m, rate| {
                let mut start = a.ceil() as Tick;
                let mut placed = false;
                for end in ends {
                    if end <= start {
                        continue;
                    }
                    let r = Region {
                        start,
                        end,
                        take,
                        src: frames_from(m, a, start as f64, rate).round() as i64,
                        gain: 1.0,
                        fade_in: 0,
                        fade_out: 0,
                    };
                    if !t.put(m, rate, r) {
                        return false;
                    }
                    (start, placed) = (end, true);
                }
                placed
            },
            map,
        );
        self.stats.refused == before
    }

    /// Takes still copying in.
    pub fn loading(&self) -> usize {
        self.loads.len()
    }

    /// A loaded take's audio, done with: drop it off the audio thread.
    pub fn take_returned(&mut self) -> Option<Vec<f32>> {
        self.returned.pop()
    }

    /// Copy the loads in, `LOAD_SPEED * n` frames' worth of samples a
    /// period, oldest first.
    fn copy_loads(&mut self, n: usize) {
        let mut budget = LOAD_SPEED * n.max(1) * 2;
        while budget > 0 && !self.loads.is_empty() {
            let (id, ref audio) = self.loads[0];
            let tk = &mut self.takes[id as usize];
            let ch = tk.channels as usize;
            let done = tk.frames * ch;
            let count = (audio.len() - done).min(budget);
            // Write samples done..done + count across the chunks.
            let mut w = 0;
            while w < count {
                let at = done + w;
                let off = at % CHUNK;
                let c = (CHUNK - off).min(count - w);
                let c_at = (at / CHUNK) as u32;
                // Every chunk was taken by load_take.
                let Some(slot) = self.pool.readable(id, c_at) else {
                    break;
                };
                let b = slot * CHUNK + off;
                self.pool.data[b..b + c].copy_from_slice(&audio[at..at + c]);
                self.pool.note(b, c);
                w += c;
            }
            let count = w;
            tk.frames = (done + count) / ch;
            let total = audio.len();
            if count == 0 && done < total {
                break;
            }
            budget -= count.max(1);
            let finished = done + count == total;
            if finished {
                tk.loading = false;
            }
            if finished {
                let (_, audio) = self.loads.remove(0);
                if self.returned.len() < self.returned.capacity() {
                    self.returned.push(audio);
                }
            }
        }
    }

    // ---- recording -----------------------------------------------------

    /// Start recording track `t` at absolute frame `g`, song tick `tick`
    /// (`bpm` there, for the latency).
    fn punch_in(&mut self, t: usize, g: u64, tick: f64, bpm: f64, group: u16, pass: u16) {
        let Some(id) = self.free_id() else {
            self.stats.full += 1;
            self.tracks[t].blocked = true;
            return;
        };
        let lat = self.tracks[t].inside.unwrap_or(self.latency);
        let lat_ticks = mpfree_transport::time::ticks_in(f64::from(lat), bpm, self.rate);
        // With files on, the take records under the id its files were made
        // for (they match its channels: ready_next keeps them so).
        self.ready_next(t);
        let ch = self.tracks[t].channels();
        let (rec, writing) = match self.tracks[t].next {
            Some((rec, _)) => (rec, true),
            None => (self.ids.fresh(), false),
        };
        self.takes[id] = Take {
            live: true,
            rec,
            track: t as u8,
            channels: ch,
            group,
            pass,
            bpm,
            anchor: tick - lat_ticks,
            recording: true,
            writing,
            ..NO_TAKE
        };
        if self.pool.alloc(id as u16, 0).is_none() {
            self.takes[id] = NO_TAKE;
            self.stats.full += 1;
            self.tracks[t].blocked = true;
            return;
        }
        self.tracks[t].rec = Some(Rec {
            take: id as u16,
            g0: g,
            t_in: tick,
        });
        if writing {
            // Its files are the take's now; the track's next take gets
            // files of its own.
            self.tracks[t].next = None;
            self.ask(Io::Start { take: id as u16 });
            self.ready_next(t);
        }
    }

    /// Files on: an armed track has files ready for its next take, made
    /// for the channels it records now, and a disarmed one has none. Asks
    /// the host to make or remove them as that changes.
    fn ready_next(&mut self, t: usize) {
        let tr = &self.tracks[t];
        let want = (self.files && tr.armed).then(|| tr.channels());
        if tr.next.map(|(_, ch)| ch) == want {
            return;
        }
        if let Some((rec, _)) = self.tracks[t].next.take() {
            self.ask(Io::Disarm { rec });
        }
        if let Some(channels) = want {
            let rec = self.ids.fresh();
            self.tracks[t].next = Some((rec, channels));
            self.ask(Io::Arm {
                track: t as u8,
                rec,
                channels,
            });
        }
    }

    /// End track `t`'s recording at absolute frame `g`. `end` is the song
    /// tick it ends on when known exactly (a loop jump); otherwise the tick
    /// at `g` comes from `span`, or (no span: a command at the period's
    /// start) from the take's own length.
    fn punch_out(&mut self, t: usize, map: &TempoMap, g: u64, end: Option<f64>) {
        let Some(rec) = self.tracks[t].rec.take() else {
            return;
        };
        let rate = self.rate;
        let id = rec.take as usize;
        let t_out =
            end.unwrap_or_else(|| tick_after(map, rec.t_in, g.saturating_sub(rec.g0) as f64, rate));
        // Close an older tail at once.
        if let Some(old) = self.tracks[t].tail.take() {
            self.close(old.take as usize);
        }
        // Go on writing the input that belongs before the punch out (the
        // latency), and a crossfade's worth more, so the end can fade into
        // what follows.
        let lat = self.tracks[t].inside.unwrap_or(self.latency);
        let stop_at = g + u64::from(lat) + FADE as u64;
        if self.takes[id].recording && !self.takes[id].cut_short && stop_at > g {
            self.tracks[t].tail = Some(Tail {
                take: rec.take,
                stop_at,
            });
        } else {
            self.close(id);
        }
        let start = rec.t_in.round() as Tick;
        let end_t = t_out.round() as Tick;
        if end_t > start {
            let anchor = self.takes[id].anchor;
            let r = Region {
                start,
                end: end_t,
                take: rec.take,
                src: frames_from(map, anchor, start as f64, rate).round() as i64,
                gain: 1.0,
                fade_in: 0,
                fade_out: 0,
            };
            self.edit(t, |tr, m, rate| tr.put(m, rate, r), map);
        }
    }

    /// The take stops recording.
    fn close(&mut self, id: usize) {
        let tk = &mut self.takes[id];
        if !tk.recording {
            return;
        }
        tk.recording = false;
        if tk.frames == 0 {
            // Nothing was written: its first chunk goes back.
            self.pool.release_take(id as u16);
        }
    }

    /// A request for the host. One that does not fit in the queue (never,
    /// as the host takes them every period) is counted, and a take whose
    /// start it was is left without a file rather than waiting for a copy
    /// that won't come.
    fn ask(&mut self, r: Io) {
        if self.io.len() < self.io.capacity() {
            self.io.push_back(r);
            return;
        }
        self.stats.io_dropped += 1;
        if let Io::Start { take } = r {
            if let Some(t) = self.takes.get_mut(take as usize) {
                t.writing = false;
            }
        }
    }

    /// Append input frames to a recording take. False if the memory ran
    /// out (the take is cut short; what was written stays).
    fn write(&mut self, id: usize, input: &[f32], sel: Input) -> bool {
        let mut j = 0;
        let n = input.len() / 2;
        let ch = self.takes[id].channels as usize;
        while j < n {
            let s = self.takes[id].frames * ch;
            let (c, off) = ((s / CHUNK) as u32, s % CHUNK);
            let slot = match self.pool.find(id as u16, c) {
                Some(slot) => slot as usize,
                None => match self.pool.alloc(id as u16, c) {
                    Some(slot) => slot,
                    None => {
                        self.takes[id].cut_short = true;
                        return false;
                    }
                },
            };
            let count = ((CHUNK - off) / ch).min(n - j);
            let b = slot * CHUNK + off;
            let dst = &mut self.pool.data[b..b + count * ch];
            let src = &input[2 * j..2 * (j + count)];
            match sel {
                Input::Stereo => dst.copy_from_slice(src),
                Input::Left => {
                    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
                        *d = s[0];
                    }
                }
                Input::Right => {
                    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
                        *d = s[1];
                    }
                }
                Input::Sum => {
                    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
                        *d = 0.5 * (s[0] + s[1]);
                    }
                }
            }
            self.pool.note(b, count * ch);
            self.takes[id].frames += count;
            j += count;
        }
        true
    }

    // ---- the period ----------------------------------------------------

    /// Run one period. `input` is the line input and `out` the mix, both
    /// interleaved stereo with `block.frames` frames; the tracks are added
    /// to `out`. `map` is the transport's tempo map
    /// ([`mpfree_transport::Transport::tempo_map`]).
    pub fn process(&mut self, block: &Block, map: &TempoMap, input: &[f32], out: &mut [f32]) {
        let n = (block.frames as usize)
            .min(input.len() / 2)
            .min(out.len() / 2)
            .min(MAX_PERIOD as usize);
        let now = block.start_frame;
        if let Some(p) = self.player.as_mut() {
            p.begin();
        }
        for i in 0..self.q_len {
            if let Some(c) = self.queue[i].take() {
                self.apply(c, map, now);
            }
        }
        self.q_len = 0;
        self.copy_loads(n);
        for t in &mut self.tracks {
            t.buf[..2 * n].fill(0.0);
        }
        let markers = block.markers();
        let mut m = 0;
        // Each track's block from inside, lent to the runs (no allocation:
        // an empty box stands in meanwhile).
        let inside = std::mem::take(&mut self.inside);
        for span in block.spans() {
            let span_len = span.len as usize;
            let mut i = 0;
            while i < span_len {
                let off = span.offset as usize + i;
                if off >= n {
                    break;
                }
                let f = span.frame + i as u64;
                while m < markers.len() && markers[m].frame <= f {
                    self.marker(map, markers[m].mark, markers[m].frame);
                    m += 1;
                }
                let mut end = span_len.min(i + RUN).min(n - span.offset as usize);
                if m < markers.len() {
                    let to = (markers[m].frame - span.frame) as usize;
                    if to > i {
                        end = end.min(to);
                    }
                }
                self.run(
                    map,
                    span,
                    f,
                    off,
                    end - i,
                    &input[2 * off..2 * (off + end - i)],
                    &inside,
                );
                i = end;
            }
        }
        self.inside = inside;
        while m < markers.len() {
            self.marker(map, markers[m].mark, markers[m].frame);
            m += 1;
        }
        self.mix(n, out);
        // Where the tracks play next, for the player, and the takes that
        // can move to their files now.
        if self.player.is_some() {
            match block.spans().last() {
                Some(sp) => self.cue(map, sp.tick_at(sp.frame + u64::from(sp.len)), sp.bpm),
                None => self.cues = [[None; 2]; MAX_TRACKS],
            }
            self.free_to_files();
        }
        if let Some(p) = self.player.as_mut() {
            p.end(n as u32);
        }
        self.last_map = *map;
    }

    // ---- recording to files -------------------------------------------

    /// Record to files, or stop (see the module notes): the host turns it
    /// on once it has a folder for them. Armed tracks get their files
    /// ([`Io::Arm`]) or let them go ([`Io::Disarm`]). Refused (false,
    /// nothing changed) while a track records or a take is still being
    /// copied out to its file.
    pub fn files(&mut self, on: bool) -> bool {
        let busy = self
            .tracks
            .iter()
            .any(|t| t.rec.is_some() || t.tail.is_some())
            || self.takes.iter().any(|t| t.live && t.writing);
        if busy {
            return false;
        }
        self.files = on;
        for t in 0..MAX_TRACKS {
            self.ready_next(t);
        }
        true
    }

    pub fn records_to_files(&self) -> bool {
        self.files
    }

    /// The lasting id track `t`'s next take records under, and the
    /// channels its files were made for (files on and armed).
    pub fn next_recording(&self, t: usize) -> Option<(RecordingId, usize)> {
        let (rec, ch) = self.tracks.get(t)?.next?;
        Some((rec, usize::from(ch)))
    }

    /// The next request about the recording files, oldest first.
    pub fn take_io(&mut self) -> Option<Io> {
        self.io.pop_front()
    }

    /// Every frame of take `take` is copied out to its file and it has
    /// stopped recording ([`Io::Start`]): it can be forgotten again.
    pub fn written(&mut self, take: u16) {
        if let Some(t) = self
            .takes
            .get_mut(take as usize)
            .filter(|t| t.live && !t.recording)
        {
            t.writing = false;
        }
    }

    /// Whether take `take` is still being copied out to its file.
    pub fn writing(&self, take: u16) -> bool {
        self.takes
            .get(take as usize)
            .is_some_and(|t| t.live && t.writing)
    }

    // ---- playing from files -------------------------------------------

    /// Play takes from their files through `player`, the stream layer's
    /// sound-thread side (made and boxed off the audio thread, with every
    /// buffer it will use, its coordinator on a thread of its own), or stop
    /// (None). The player given before comes back, to be dropped off the
    /// audio thread. Real-time safe.
    ///
    /// A take's stream source is its id. A recorded take moves from the
    /// pool to its file with [`Command::OnFile`], a chunk at a time while
    /// it records (its file growing) or at once when finished; a saved take
    /// opens as its file ([`AudioTracks::add_file_take`]). Each period the
    /// tracks begin and end the player's period, read what file takes play
    /// through it, and tell it where each track plays, or is about to:
    /// the region there and the next one if it starts within [`CUE_S`],
    /// through cursors `2 * track` (even regions) and `2 * track + 1` (odd
    /// ones), so a region keeps its cursor as it goes from next to
    /// playing. Every frame that plays silence because it wasn't in the
    /// cache is counted ([`AudioTracks::missed`]). Without a player, file
    /// takes play silence, counted the same.
    pub fn stream(&mut self, player: Option<Box<Player>>) -> Option<Box<Player>> {
        std::mem::replace(&mut self.player, player)
    }

    /// The player, for its counters.
    pub fn player(&self) -> Option<&Player> {
        self.player.as_deref()
    }

    /// Whether take `id` plays from its file (it holds no pool memory).
    pub fn on_file(&self, id: u16) -> bool {
        self.takes
            .get(id as usize)
            .is_some_and(|t| t.live && matches!(t.store, Store::File { .. }))
    }

    /// How many of take `id`'s frames, from its start, play from its file
    /// rather than the pool: all of a take on its file, those below the
    /// pool's part of a take whose file holds only some of it so far (a
    /// long recording, its memory given back as its file holds it), none
    /// of a take in the pool. None if the take is not there.
    pub fn file_frames(&self, id: u16) -> Option<usize> {
        let t = self.takes.get(id as usize).filter(|t| t.live)?;
        Some(Self::on_file_of(t))
    }

    fn on_file_of(t: &Take) -> usize {
        match t.store {
            Store::Pool => 0,
            Store::Part { freed, .. } => freed as usize,
            Store::File { .. } => t.frames,
        }
    }

    /// Frames of take `id` that played silence because its file's frames
    /// weren't in the cache yet (or its file wasn't open), and the song
    /// tick of the latest. None if the take is not there.
    pub fn missed(&self, id: u16) -> Option<(u64, f64)> {
        let t = self.takes.get(id as usize).filter(|t| t.live)?;
        Some((t.missed, t.miss_tick))
    }

    /// Put a saved take back as its file, stream source `f.id` at
    /// generation `gen`, which the host has opened and checked against
    /// `f`'s channels and length: nothing is copied in and no pool memory
    /// is taken. It plays once the player has the source open at `gen`
    /// (until then, what it should play is counted as missed). Under its
    /// own id and lasting id, as [`AudioTracks::load_take`]; refused
    /// (false, nothing changed) as that is but for the audio, which isn't
    /// needed, and without a player. Does not allocate.
    pub fn add_file_take(&mut self, f: &TakeFile, gen: u32) -> bool {
        let id = f.id as usize;
        if self.player.is_none()
            || id >= MAX_TAKES
            || self.takes[id].live
            || f.track >= MAX_TRACKS
            || !(1..=2).contains(&f.channels)
            || f.frames == 0
            || !f.anchor.is_finite()
        {
            return false;
        }
        let rec = self.keep_id(f.rec);
        self.takes[id] = Take {
            live: true,
            rec,
            track: f.track as u8,
            channels: f.channels as u8,
            group: f.group,
            pass: f.pass,
            bpm: if f.bpm.is_finite() && f.bpm > 0.0 {
                f.bpm
            } else {
                0.0
            },
            anchor: f.anchor,
            frames: f.frames,
            store: Store::File { gen },
            ..NO_TAKE
        };
        self.reserved[id] = false;
        self.groups = self.groups.max(f.group);
        true
    }

    /// Take `id` goes: its pool memory back, and its stream source closed
    /// if it has one.
    fn forget(&mut self, id: u16) {
        self.pool.release_take(id);
        if let Store::Part { gen, .. } | Store::File { gen } = self.takes[id as usize].store {
            self.ask(Io::Close { take: id, gen });
        }
        self.takes[id as usize] = NO_TAKE;
    }

    /// Where each track plays, or is about to, from song tick `tick` (see
    /// [`AudioTracks::stream`]), for the regions of takes played from, or
    /// moving to, their files; told to the player. A track that records,
    /// or the tracks turned off, play nothing.
    fn cue(&mut self, map: &TempoMap, tick: f64, bpm: f64) {
        if self.player.is_none() {
            return;
        }
        self.cues = [[None; 2]; MAX_TRACKS];
        if !self.active {
            return;
        }
        let rate = self.rate;
        let tail = mpfree_transport::time::ticks_in(FADE as f64, bpm, rate) + 1.0;
        let ahead = CUE_S * f64::from(rate);
        for t in 0..MAX_TRACKS {
            let tr = &self.tracks[t];
            if tr.rec.is_some() {
                continue;
            }
            let first = tr
                .regions()
                .partition_point(|r| r.end as f64 + tail <= tick);
            for i in first..(first + 2).min(tr.n) {
                let r = tr.regions[i];
                let Some(tk) = self
                    .takes
                    .get(r.take as usize)
                    .filter(|k| k.live && k.store != Store::Pool)
                else {
                    continue;
                };
                // Frames from the region's start to the tick: 0 for one
                // still to come, if it comes soon enough.
                let e0 = if r.start as f64 > tick {
                    if frames_from(map, tick, r.start as f64, rate) > ahead {
                        break;
                    }
                    0
                } else {
                    frames_from(map, r.start as f64, tick, rate).round() as i64
                };
                let len_f = frames_from(map, r.start as f64, r.end as f64, rate).round() as i64;
                let end = (r.src + len_f + FADE as i64).clamp(0, tk.frames as i64) as u64;
                let frame = (r.src + e0).max(0) as u64;
                if frame < end {
                    self.cues[t][i % 2] = Some(Cue {
                        take: r.take,
                        frame,
                        end,
                    });
                }
            }
        }
        let AudioTracks {
            player,
            cues,
            takes,
            ..
        } = self;
        let Some(p) = player.as_mut() else {
            return;
        };
        for (t, two) in cues.iter().enumerate() {
            for (k, c) in two.iter().enumerate() {
                let Some(c) = c else {
                    continue;
                };
                let (Store::Part { gen, .. } | Store::File { gen }) = takes[c.take as usize].store
                else {
                    continue;
                };
                if p.source(c.take) == Some((gen, true)) {
                    let place = Place {
                        frame: c.frame,
                        speed: 1.0,
                        looping: None,
                        end: Some(c.end),
                    };
                    p.playing(c.take, (2 * t + k) as u16, place);
                }
            }
        }
    }

    /// Each take whose file is open ([`Command::OnFile`]) gives back the
    /// pool chunks its file holds, and once the file holds every frame of
    /// a take no longer recording, the take plays from its file alone:
    /// while a region plays it, or is about to, only once [`MOVE_S`] of
    /// every such region, from where it plays, is in the cache (at once if
    /// none does), so the move never plays silence. A take recording is
    /// never played, so it gives its chunks back as soon as they are on
    /// the file. One whose file doesn't match it (other channels; a
    /// finished file of another length, or a growing one longer than the
    /// take) stays in the pool (counted in [`Stats::mismatched`]), and its
    /// source is closed.
    fn free_to_files(&mut self) {
        if self.player.is_none() {
            return;
        }
        let need = (MOVE_S * f64::from(self.rate)) as u64;
        for id in 0..MAX_TAKES as u16 {
            let Store::Part {
                gen,
                freed,
                growing,
            } = self.takes[id as usize].store
            else {
                continue;
            };
            let Some(p) = self.player.as_ref() else {
                return;
            };
            if p.source(id) != Some((gen, true)) {
                continue;
            }
            let tk = &self.takes[id as usize];
            let (ch, frames) = (tk.channels as usize, tk.frames as u64);
            let have = match p.shape(id) {
                Some((c, have))
                    if usize::from(c) == ch && (have == frames || (growing && have < frames)) =>
                {
                    have
                }
                // Nothing has gone yet (a file only grows, and a take's
                // channels never change, so it can't be later).
                _ if freed == 0 => {
                    self.takes[id as usize].store = Store::Pool;
                    self.stats.mismatched += 1;
                    self.ask(Io::Close { take: id, gen });
                    continue;
                }
                _ => continue,
            };
            let whole = have == frames && !tk.recording;
            // Whole chunks the file holds.
            let per = (CHUNK / ch) as u64;
            let upto = have / per * per;
            if !whole && upto <= freed {
                continue;
            }
            let covered = self
                .cues
                .iter()
                .flatten()
                .flatten()
                .filter(|c| c.take == id)
                .all(|c| p.covers(id, c.frame, need.min(c.end - c.frame)));
            if !covered {
                continue;
            }
            if whole {
                self.takes[id as usize].store = Store::File { gen };
                self.pool.release_take(id);
            } else {
                for c in freed / per..upto / per {
                    if let Some(slot) = self.pool.find(id, c as u32) {
                        self.pool.release(slot as usize);
                    }
                }
                self.takes[id as usize].store = Store::Part {
                    gen,
                    freed: upto,
                    growing,
                };
            }
        }
    }

    /// A loop jump or a locate while recording: each recording track ends
    /// its take where the song left off and starts a new one (the next
    /// pass) where it lands, at the same frame.
    fn marker(&mut self, map: &TempoMap, mark: Mark, frame: u64) {
        let end = match mark {
            Mark::Looped { end, .. } => end as f64,
            Mark::Located { from, .. } => from as f64,
            _ => return,
        };
        for t in 0..MAX_TRACKS {
            if let Some(rec) = self.tracks[t].rec {
                let tk = self.takes[rec.take as usize];
                self.punch_out(t, map, frame, Some(end));
                // The next pass starts in run(), on the new span's tick.
                self.pending[t] = Some((tk.group, tk.pass + 1));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        map: &TempoMap,
        span: &Span,
        f: u64,
        off: usize,
        len: usize,
        line: &[f32],
        inside: &[f32],
    ) {
        let recording_state = span.state == State::Recording;
        let rolling = matches!(span.state, State::Playing | State::Recording);
        let tick0 = span.tick_at(f);
        for t in 0..MAX_TRACKS {
            // What this track records: its block from inside, or the line.
            let input = match self.tracks[t].inside {
                Some(_) => &inside[t * INSIDE + 2 * off..t * INSIDE + 2 * (off + len)],
                None => line,
            };
            let want = self.active && recording_state && self.tracks[t].armed;
            if !want {
                // A recording cut short waits for the next punch in.
                self.tracks[t].blocked = false;
            }
            match (self.tracks[t].rec.is_some(), want) {
                (false, true) if !self.tracks[t].blocked => {
                    let (group, pass) = self.pending[t].take().unwrap_or_else(|| {
                        self.groups = self.groups.wrapping_add(1);
                        (self.groups, 0)
                    });
                    self.punch_in(t, f, tick0, span.bpm, group, pass);
                }
                (true, false) => {
                    self.punch_out(t, map, f, Some(tick0));
                    self.pending[t] = None;
                }
                _ => {
                    if !want {
                        self.pending[t] = None;
                    }
                }
            }
            // Tails: input that belongs before a punch out.
            if let Some(tail) = self.tracks[t].tail {
                let upto = tail.stop_at.saturating_sub(f).min(len as u64) as usize;
                let id = tail.take as usize;
                let sel = self.input_of(id);
                if upto > 0 {
                    self.write(id, &input[..2 * upto], sel);
                }
                if f + len as u64 >= tail.stop_at
                    || !self.takes[id].recording
                    || self.takes[id].cut_short
                {
                    self.close(id);
                    self.tracks[t].tail = None;
                }
            }
            if let Some(rec) = self.tracks[t].rec {
                let id = rec.take as usize;
                let sel = self.input_of(id);
                if !self.write(id, input, sel) {
                    // The memory is full: the take ends where its audio
                    // does.
                    self.stats.full += 1;
                    self.tracks[t].blocked = true;
                    let g = rec.g0 + self.takes[id].frames as u64;
                    self.punch_out(t, map, g, None);
                }
            }
            let tr = &self.tracks[t];
            // A track recording from inside never plays it through: it is
            // heard where it comes from already.
            let hear_input = tr.inside.is_none()
                && match tr.monitor {
                    Monitor::Off => false,
                    Monitor::In => true,
                    Monitor::Auto => tr.armed && (span.state != State::Playing || !self.active),
                };
            if rolling && self.active && tr.rec.is_none() {
                self.play(t, map, span, tick0, off, len);
            }
            if hear_input {
                let tr = &mut self.tracks[t];
                let sel = tr.input;
                let dst = &mut tr.buf[2 * off..2 * (off + len)];
                for (d, s) in dst.chunks_exact_mut(2).zip(input.chunks_exact(2)) {
                    match sel {
                        Input::Stereo => {
                            d[0] += s[0];
                            d[1] += s[1];
                        }
                        Input::Left => {
                            d[0] += s[0];
                            d[1] += s[0];
                        }
                        Input::Right => {
                            d[0] += s[1];
                            d[1] += s[1];
                        }
                        Input::Sum => {
                            let v = 0.5 * (s[0] + s[1]);
                            d[0] += v;
                            d[1] += v;
                        }
                    }
                }
            }
        }
    }

    fn input_of(&self, id: usize) -> Input {
        let tk = &self.takes[id];
        if tk.channels == 2 {
            Input::Stereo
        } else {
            match self.tracks[tk.track as usize].input {
                Input::Stereo => Input::Left,
                i => i,
            }
        }
    }

    /// Add track `t`'s regions for `len` frames from song tick `tick0`
    /// (frame `off` of the period).
    fn play(&mut self, t: usize, map: &TempoMap, span: &Span, tick0: f64, off: usize, len: usize) {
        let rate = self.rate;
        let tail = mpfree_transport::time::ticks_in(FADE as f64, span.bpm, rate) + 1.0;
        let tick1 = tick0 + mpfree_transport::time::ticks_in(len as f64, span.bpm, rate) + 1.0;
        let AudioTracks {
            tracks,
            takes,
            pool,
            fade,
            player,
            reads,
            stats,
            ..
        } = self;
        let tr = &mut tracks[t];
        let first = tr
            .regions()
            .partition_point(|r| r.end as f64 + tail <= tick0);
        let (regions, buf) = (&tr.regions[..tr.n], &mut tr.buf);
        for r in &regions[first..] {
            if r.start as f64 >= tick1 {
                break;
            }
            let Some(tk) = takes.get(r.take as usize).filter(|k| k.live && !k.loading) else {
                continue;
            };
            // Frames since the region's start at the run's first frame,
            // and the region's length in frames.
            let e0 = frames_from(map, r.start as f64, tick0, rate).round() as i64;
            let len_f = frames_from(map, r.start as f64, r.end as f64, rate).round() as i64;
            let stop = len_f + FADE as i64;
            if e0 >= stop || e0 + len as i64 <= 0 {
                continue;
            }
            let fi = i64::from(r.fade_in).min(len_f / 2).max(0);
            let fo = i64::from(r.fade_out).min(len_f / 2).max(0);
            let fade = &*fade;
            let gain_at = |e: i64| -> f32 {
                if e < 0 || e >= stop {
                    return 0.0;
                }
                let mut g = r.gain;
                if e < FADE as i64 {
                    g *= fade[e as usize];
                }
                if e >= len_f {
                    g *= fade[(FADE as i64 - (e - len_f)) as usize];
                }
                if fi > 0 && e < fi {
                    g *= e as f32 / fi as f32;
                }
                if fo > 0 && e >= len_f - fo && e < len_f {
                    g *= (len_f - e) as f32 / fo as f32;
                }
                g
            };
            let dst = &mut buf[2 * off..2 * (off + len)];
            let plain_from = (FADE as i64).max(fi);
            // The whole run past the fades: one gain.
            let plain = e0 >= plain_from && e0 + len as i64 <= len_f - fo;
            let (g, k) = (r.gain, r.src + e0);
            let missed = match tk.store {
                Store::File { gen } => {
                    let p = player.as_deref_mut();
                    let end = tk.frames;
                    if plain {
                        tk.add_from_file(p, r.take, gen, end, k, dst, reads, &|_| g)
                    } else {
                        let gain = |j: usize| gain_at(e0 + j as i64);
                        tk.add_from_file(p, r.take, gen, end, k, dst, reads, &gain)
                    }
                }
                Store::Part { gen, freed, .. } if freed > 0 => {
                    let p = player.as_deref_mut();
                    if plain {
                        tk.add_part(pool, p, r.take, gen, freed, k, dst, reads, &|_| g)
                    } else {
                        let gain = |j: usize| gain_at(e0 + j as i64);
                        tk.add_part(pool, p, r.take, gen, freed, k, dst, reads, &gain)
                    }
                }
                Store::Pool | Store::Part { .. } => {
                    if plain {
                        tk.add_into(pool, r.take, k, dst, &|_| g);
                    } else {
                        tk.add_into(pool, r.take, k, dst, &|j| gain_at(e0 + j as i64));
                    }
                    0
                }
            };
            if missed > 0 {
                let m = &mut takes[r.take as usize];
                m.missed += missed;
                m.miss_tick = tick0;
                stats.missed += missed;
            }
        }
    }

    /// Each track's level, pan, mute and solo, ramped over the period, and
    /// the sum into `out`.
    fn mix(&mut self, n: usize, out: &mut [f32]) {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        let only = self.only;
        for (k, tr) in self.tracks.iter_mut().enumerate() {
            let audible = !tr.mute && (!any_solo || tr.solo) && only.is_none_or(|o| o == k);
            let lv = if audible { tr.level } else { 0.0 };
            let target = [lv * (1.0 - tr.pan).min(1.0), lv * (1.0 + tr.pan).min(1.0)];
            let from = tr.gains;
            let step = [
                (target[0] - from[0]) / n.max(1) as f32,
                (target[1] - from[1]) / n.max(1) as f32,
            ];
            for (j, (d, o)) in tr.buf[..2 * n]
                .chunks_exact_mut(2)
                .zip(out.chunks_exact_mut(2))
                .enumerate()
            {
                let k = (j + 1) as f32;
                d[0] *= from[0] + step[0] * k;
                d[1] *= from[1] + step[1] * k;
                o[0] += d[0];
                o[1] += d[1];
            }
            if n > 0 {
                tr.gains = target;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids never repeat in a run, are never 0, and two runs draw two keys.
    #[test]
    fn recording_ids_never_repeat() {
        let mut ids = Ids::new();
        let mut seen: Vec<u128> = (0..20_000).map(|_| ids.fresh().0).collect();
        assert!(seen.iter().all(|&r| r != 0));
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 20_000);
        assert_ne!(Ids::new().key, Ids::new().key);
        // Each half looks random: about half its bits set.
        let r = ids.fresh().0;
        assert!((32..=96).contains(&r.count_ones()), "{r:032x}");
    }
}
