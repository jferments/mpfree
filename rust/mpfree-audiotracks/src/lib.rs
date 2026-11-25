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
//!   Where the same take runs straight on (a Split), nothing fades, and a
//!   gain change there ramps over [`FADE`] frames (`Join`).
//! - **Cuts keep their fades.** A region cut in two (a Split, an erase or
//!   punch across it, a song time edit) keeps the region's own fades on
//!   both parts, each counting the frames cut off it ([`Region::into`],
//!   [`Region::left`]), so a fade the cut goes through plays on as it did.
//! - **Time.** A region sits on song ticks, so it stays on its bar when the
//!   tempo before it changes. By default ([`FollowTempo::Off`]) its audio plays
//!   at the rate it was recorded at, from the region's start, as a DAW
//!   plays audio that is not warped: at another tempo it does not stretch.
//! - **Following the tempo** (the audio tracks rethink's phase 5,
//!   docs/dev/design/tempo-follow.md). A region on [`FollowTempo::Repitch`]
//!   with a made-at tempo ([`Region::bpm`]) plays the frame of its
//!   recording each song tick means at that tempo ([`Region::src_pos`],
//!   the plan's `x(t)`), so it fills its bars at any tempo and its pitch
//!   moves with the tempo, like tape. Every place that asks which frame
//!   plays (the read, a cut, a join, the clips, the stream's cues) asks
//!   that one mapping. It reads between frames with the streaming
//!   cursor's band-limited read ([`mpfree_stream::cursor::read_gathered`])
//!   from frames gathered from the pool or the take's file; at the
//!   made-at tempo it plays as Off, bit for bit. [`FollowTempo::Stretch`]
//!   plays as Repitch until its copies come (phase 5b).
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
//! - **Clips.** Given what the arranger's player says each track plays
//!   ([`AudioTracks::process_with`]), a track plays audio clips as well as
//!   its song: each in a voice with its own copy of the clip's regions,
//!   every change on its frame ([`clips`]).
//!
//! [`AudioTracks::command`] and [`AudioTracks::process`] never allocate,
//! lock or panic, and give the same output for any period size.

#![forbid(unsafe_code)]

pub mod clips;
pub mod copies;
pub mod copying;
pub mod files;
pub mod host;
pub mod loop_play;
mod pool;
pub mod retime;
pub mod save;
pub mod streaming;
pub mod stretch;

pub use clips::{
    Answer, Change, ClipPlay, ClipRegion, LoopPlay, Plays, DECLICK, MAX_CHANGES, MAX_CLIP_REGIONS,
    VOICES,
};
use clips::{Glide, Lane, Mode, Shape, Window, NO_TAKE_ID};
pub use loop_play::{LoopPlayer, LoopRun, LISTEN, LOOP_VOICES};

/// Each loop's buffer for the period, by its id
/// ([`AudioTracks::process_with_loops`]).
type LoopSource<'a, 's> = &'a dyn Fn(u128) -> Option<&'s [f32]>;
use pool::Pool;
pub use pool::CHUNK;
pub use retime::{Piece, Retime};
use std::collections::VecDeque;
use std::fmt;

use save::{Saved, TakeFile, TrackFile};

use mpfree_stream::msg::{Place, MAX_CURSORS, MAX_SOURCES};
use mpfree_stream::player::{Gaps, Lead, Player};
use mpfree_transport::time::{valid_bpm, TempoMap, Tick, PPQ};
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Mark, Span, State, TransactionId};

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
/// The fastest a region following the tempo plays its recording
/// (recording frames an output frame), as the streaming cursor's fastest:
/// past it a run plays nothing, counted ([`Stats::too_fast`]).
pub const MAX_SPEED: f64 = 4.0;
/// Recording frames a run gathers for a read following the tempo, at
/// most: what [`RUN`] frames pass at [`MAX_SPEED`] and the read's widest
/// reach either side ([`mpfree_stream::cursor::reach_of`]).
const GATHER: usize = RUN * MAX_SPEED as usize + 2 * 17 + 4;
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
    /// Whether the tracks record and play their song regions (the host
    /// turns them off when the song is not playing, say in clip launch
    /// mode). Clips play either way, as the arranger's answer says
    /// ([`AudioTracks::process_with`]), and so does monitoring.
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
    /// [`Command::Erase`] as an undo step of song edit `song` (named by
    /// its transaction, `mpfree_play::Take::txn`; not none):
    /// Rec song's block of an audio clip replacing the regions under it
    /// (phase 2, step 2d). As for [`Command::Retime`], only
    /// [`Command::SongUndo`] and [`Command::SongRedo`] take it back and
    /// bring it back; [`Command::Undo`] and [`Command::Redo`] stop at it.
    /// Refused while the track records.
    EraseSong {
        song: TransactionId,
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
    /// crossfades every region has). They are its own: a part cut from a
    /// longer region no longer counts the frames cut off it.
    SetFades {
        track: usize,
        at: Tick,
        fade_in: u32,
        fade_out: u32,
    },
    /// How the region of `track` playing at `at` follows the song's tempo
    /// (phase 5, [`FollowTempo`]), and on Stretch whether it keeps its pitch
    /// while its copy isn't ready. Its start keeps the frame it plays.
    SetFollow {
        track: usize,
        at: Tick,
        follow: FollowTempo,
        keep_pitch: bool,
    },
    /// The tempo the audio of the region of `track` playing at `at` was
    /// made at, set by hand ([`Region::bpm`]): a tempo MPFree plays, or 0
    /// for none known.
    SetTempo {
        track: usize,
        at: Tick,
        bpm: f64,
    },
    Undo,
    Redo,
    /// Give a take's memory back. Refused while it records or while a
    /// region, or a step Undo or Redo could bring back, plays it, or a
    /// clip keeps it (see [`AudioTracks::in_use`]);
    /// [`Command::ClearHistory`] first.
    Forget(u16),
    /// Keep take `take` while a clip, or an undo step holding one, names
    /// its recording (the audio tracks rethink, phase 2: the app says
    /// which, [`AudioTracks::kept`]), or let it go. A take kept can't be
    /// forgotten; one let go is forgotten at once if nothing else uses it
    /// and it is free to go (not recording, loading or being written).
    Keep {
        take: u16,
        keep: bool,
    },
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
    /// undo step that belongs to song edit `song` (named by its
    /// transaction, not none). Only
    /// [`Command::SongUndo`] and [`Command::SongRedo`] take that step back
    /// and bring it back; [`Command::Undo`] and [`Command::Redo`] stop at
    /// it. The cuts use the tempo map of the period before (the one the
    /// regions were placed on: the edit's own view brings the new one).
    /// Refused, nothing changing, while a track records or when a track's
    /// regions would not fit ([`AudioTracks::retime_refused`] says first).
    Retime {
        song: TransactionId,
        retime: Retime,
    },
    /// Undo step `step` (a recording's or a sound's put on a lane,
    /// [`AudioTracks::take_took`]) becomes one of song edit `song` (by
    /// its transaction, not none): the song edit that cut the blocks under it (phase 2, step 2d).
    /// From then on it is as a [`Command::Retime`]'s step: only
    /// [`Command::SongUndo`] and [`Command::SongRedo`] take it back and
    /// bring it back. Refused when Undo can't reach that step, or it
    /// belongs to a song edit already.
    TieStep {
        step: u32,
        song: TransactionId,
    },
    /// Take back, or bring back, song edit `song`'s steps (one a
    /// [`Command::Retime`] it was made of: + Bar pressed in a row is one
    /// song edit): refused unless they are the steps Undo, or Redo, would
    /// take next.
    SongUndo(TransactionId),
    SongRedo(TransactionId),
    /// The song can no longer take back, or bring back, song edit `song`
    /// (its history moved on past it): its steps go, with every step
    /// Undo or Redo could only reach through them, so the regions never
    /// go back to where they sat on a song that is gone. Changes no
    /// region.
    ForgetSong(TransactionId),
    /// Forget every step Redo could bring back, a song edit's too: a new
    /// step elsewhere took their place in the one UNDO across the engines
    /// (audio tracks phase 4d.3). Changes no region.
    ForgetRedo,
    /// Let go of every step Undo could take back made at or before this
    /// transaction, from the oldest, a song edit's with its take: another
    /// engine let go of a step that old (audio tracks phase 4d.3). Changes
    /// no region.
    RetireThrough(TransactionId),
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
    /// Take `take`'s file opened again, the same file, under generation
    /// `gen` (its SD card went and came back): a take playing from its
    /// file, or partly, plays from the new generation from now on, and
    /// nothing else about it changes. `rec` is the take's lasting id, as
    /// for [`Command::OnFile`]. Refused for a take not on its file at all.
    Refile {
        take: u16,
        rec: RecordingId,
        gen: u32,
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

/// How a region, or an audio clip's regions, follow the song's tempo (the
/// audio tracks rethink's phase 5). Saved as a `follow=` word when not
/// [`FollowTempo::Off`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FollowTempo {
    /// At its own speed from its start, as recorded: what every region
    /// did before phase 5.
    #[default]
    Off,
    /// Read at the speed that keeps it on its beats, its pitch moving with
    /// the tempo, like tape ([`Region::src_pos`]).
    Repitch,
    /// The same pitch, from a copy made ahead. Until the copies come
    /// (phase 5b) it plays as Repitch, as Play on time does while its copy
    /// isn't ready.
    Stretch,
}

impl FollowTempo {
    /// Its word in a save (`follow=`), none for Off.
    pub fn word(self) -> &'static str {
        match self {
            FollowTempo::Off => "off",
            FollowTempo::Repitch => "repitch",
            FollowTempo::Stretch => "stretch",
        }
    }

    /// The setting a `follow=` word names.
    pub fn from_word(w: &str) -> Option<FollowTempo> {
        match w {
            "off" => Some(FollowTempo::Off),
            "repitch" => Some(FollowTempo::Repitch),
            "stretch" => Some(FollowTempo::Stretch),
            _ => None,
        }
    }
}

/// A stretch of song ticks playing a stretch of a take: tick `start`
/// plays the take's frame `src`, and the frames after it follow at the
/// recorded rate up to tick `end` (or, following the tempo, at the rate
/// that keeps them on their beats: [`Region::follow`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub start: Tick,
    pub end: Tick,
    pub take: u16,
    /// The take's frame that plays at `start` (may be below 0 or past the
    /// take's end: those frames are silent).
    pub src: i64,
    /// Where in frame `src` its recording is at `start`, -0.5 to 0.5: the
    /// frame that plays at song tick `t` is `src` plus `phase` and the
    /// frames from `start` to `t`, rounded ([`Region::frames_to`]). Where
    /// a tick falls between frames, a part cut from a region keeps the
    /// region's rounding, so its parts play exactly as it did.
    pub phase: f32,
    pub gain: f32,
    /// Its fade in and fade out in frames, measured on the region it was
    /// cut from (`into`, `left`).
    pub fade_in: u32,
    pub fade_out: u32,
    /// Frames of the region it was cut from (a Split, an erase, a punch,
    /// a song time edit) before its start and after its end: 0 for one
    /// never cut. Its fades are that whole region's, so a cut keeps
    /// exactly its share of them.
    pub into: u32,
    pub left: u32,
    /// How it follows the song's tempo (phase 5): Off by default.
    pub follow: FollowTempo,
    /// On Stretch, what it does while its copy isn't ready: keep its
    /// pitch (true) or play on time on Repitch (false, the default).
    pub keep_pitch: bool,
    /// The tempo its audio was made at (BPM): its take's when it was made,
    /// or set by hand. 0 when none is known (a take recorded across a
    /// tempo change, a sound brought in with no tempo), which plays it
    /// Off whatever `follow` says.
    pub bpm: f64,
}

impl Region {
    const EMPTY: Region = Region {
        start: 0,
        end: 0,
        take: 0,
        src: 0,
        phase: 0.0,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
        into: 0,
        left: 0,
        follow: FollowTempo::Off,
        keep_pitch: false,
        bpm: 0.0,
    };

    /// A region of `take` on song ticks `start..end`, its tick `start`
    /// playing frame `src` (and `phase` of the next), at full gain with
    /// no fades, Off, made at `bpm`.
    pub fn of(start: Tick, end: Tick, take: u16, (src, phase): (i64, f32), bpm: f64) -> Region {
        Region {
            start,
            end,
            take,
            src,
            phase,
            bpm,
            ..Region::EMPTY
        }
    }

    pub fn contains(&self, t: Tick) -> bool {
        (self.start..self.end).contains(&t)
    }

    /// Frames of it from song tick `at`, where it starts as it plays, to
    /// song tick `t` along `map`: the frames it plays for; to its end, its
    /// length. Off, the frame of its recording `t` plays is `src` plus
    /// this ([`Region::src_frames_to`] says which frame for any setting).
    pub fn frames_to(&self, map: &TempoMap, at: f64, t: f64, rate: u32) -> i64 {
        (f64::from(self.phase) + frames_from(map, at, t, rate)).round() as i64
    }

    /// Its recording's frames a song tick when it follows the tempo
    /// (Repitch, or Stretch until its copies come) and has a made-at
    /// tempo; None when it plays Off.
    pub fn per_tick(&self, rate: u32) -> Option<f64> {
        (self.follow != FollowTempo::Off && valid_bpm(self.bpm))
            .then(|| 60.0 * f64::from(rate) / (self.bpm * f64::from(PPQ)))
    }

    /// The plan's `x(t)`: where in its recording song tick `t` plays, as
    /// frames past `src` with the fraction, it starting at song tick `at`.
    /// Off, the frames along the song's tempo map from `at` (it plays at
    /// its own speed); following the tempo, the ticks from `at` at its
    /// made-at tempo, whatever the song's tempo, so it stays on its beats.
    /// Summed in f64 from the ticks, never rounded or added up a period at
    /// a time.
    pub fn src_pos(&self, map: &TempoMap, at: f64, t: f64, rate: u32) -> f64 {
        f64::from(self.phase)
            + match self.per_tick(rate) {
                Some(k) => (t - at) * k,
                None => frames_from(map, at, t, rate),
            }
    }

    /// [`Region::src_pos`] rounded: the frame past `src` that plays at
    /// song tick `t`. Off it is [`Region::frames_to`], to the frame.
    pub fn src_frames_to(&self, map: &TempoMap, at: f64, t: f64, rate: u32) -> i64 {
        self.src_pos(map, at, t, rate).round() as i64
    }

    /// The frames of its recording it covers, `len_f` frames long as it
    /// plays ([`Region::frames_to`] to its end): where audio that runs on
    /// from it starts. Off they are the same.
    pub fn src_len(&self, len_f: i64, rate: u32) -> i64 {
        match self.per_tick(rate) {
            Some(k) => (f64::from(self.phase) + (self.end - self.start) as f64 * k).round() as i64,
            None => len_f,
        }
    }

    /// Recording frames an output frame at song tempo `bpm`, following
    /// the tempo; None when it plays Off.
    pub fn speed(&self, bpm: f64) -> Option<f64> {
        (self.follow != FollowTempo::Off && valid_bpm(self.bpm)).then(|| bpm / self.bpm)
    }
}

/// The frame of a take, and where in it, at song tick `t` along `map`,
/// the take's frame 0 at song tick `anchor`: for a region starting at `t`
/// ([`Region::src`], [`Region::phase`]).
pub fn src_at(map: &TempoMap, anchor: f64, t: f64, rate: u32) -> (i64, f32) {
    let x = frames_from(map, anchor, t, rate);
    let src = x.round();
    (src as i64, (x - src) as f32)
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
    /// The tempo changed while it recorded (a tempo point, a ramp, the
    /// tempo turned): it has no one tempo, so a region of it can't follow
    /// the tempo (phase 5).
    pub varied: bool,
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
    /// Changes the arranger's player dropped because a track had more
    /// than [`MAX_CHANGES`] in a period ([`Answer::superseded`]).
    pub superseded: u64,
    /// Clip voices handed to the declick because a track needed a fifth
    /// ([`VOICES`]).
    pub declicked: u32,
    /// Frames of regions following the tempo that played nothing because
    /// the song asked them faster than [`MAX_SPEED`] (phase 5).
    pub too_fast: u64,
    /// Frames of regions on Stretch played from their copies, and played
    /// on Repitch for want of one (Play on time: phase 5b).
    pub copied: u64,
    pub for_now: u64,
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
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cue {
    take: u16,
    frame: u64,
    end: u64,
    /// Recording frames it moves an output frame (1 but following the
    /// tempo), for the player's read ahead.
    speed: f64,
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
    /// The tempo changed while it recorded ([`TakeInfo::varied`]).
    varied: bool,
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
    /// A clip, or an undo step holding one, names it ([`Command::Keep`]):
    /// it can't be forgotten.
    kept: bool,
    /// Let go while only a clip voice used it: forgotten once none does.
    let_go: bool,
    store: Store,
    /// Frames played from its file that played silence (see
    /// [`Stats::missed`]), and the song tick of the latest.
    missed: u64,
    miss_tick: f64,
    /// How what it last played from its file ended, and the frame after
    /// it: a read from there goes on from it ([`Take::add_from_file`]).
    after: (u64, Lead),
}

/// [`Take::after`] with nothing to go on from.
const CLEAR: (u64, Lead) = (u64::MAX, Lead::Clear);

const NO_TAKE: Take = Take {
    live: false,
    rec: RecordingId::NONE,
    track: 0,
    channels: 1,
    group: 0,
    bpm: 0.0,
    varied: false,
    pass: 0,
    anchor: 0.0,
    frames: 0,
    recording: false,
    cut_short: false,
    loading: false,
    writing: false,
    kept: false,
    let_go: false,
    store: Store::Pool,
    missed: 0,
    miss_tick: 0.0,
    after: CLEAR,
};

impl Take {
    /// The tempo a region of it was made at: its own, or 0 (none) when not
    /// known or when the tempo changed while it recorded.
    fn made_at(&self) -> f64 {
        if self.varied {
            0.0
        } else {
            self.bpm
        }
    }

    /// Adds `gain`-weighted frames `k..k + n` of take `id` into the
    /// interleaved stereo `out` (n frames); frames the take does not have
    /// add nothing. `gain(j)` is the gain of frame `j` of the run.
    /// Always inlined: left to itself the compiler stopped inlining it
    /// into [`Reader::add`] once that grew the read following the tempo,
    /// and a region played as recorded cost 3.5 % more a block (the
    /// bench's audio tracks rows).
    #[inline(always)]
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
    /// `gen` with the take's channels and those frames (or no player);
    /// and how what it played ended, for [`Take::after`].
    ///
    /// Sound never clicks as it goes missing or comes back (the SD card
    /// plan's B2a): it fades out over [`mpfree_stream::player::DECLICK`]
    /// frames before frames missing from the cache, or, when they start a
    /// read, dies away over them from the last frame played; and it fades
    /// in after them, also when they ended the last read
    /// ([`Gaps::smooth`]). Frames with no gap near play as they are, bit
    /// for bit.
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
    ) -> (u64, (u64, Lead)) {
        let n = out.len() / 2;
        let ch = self.channels as usize;
        let j0 = if k < 0 { ((-k) as usize).min(n) } else { 0 };
        let from = (k + j0 as i64) as usize;
        let have = end.min(self.frames).saturating_sub(from).min(n - j0);
        if have == 0 {
            return (0, self.after);
        }
        let holds = |&(c, f): &(u16, u64)| c == u16::from(self.channels) && f >= end as u64;
        let Some(p) = player.filter(|p| {
            p.source(id) == Some((gen, true)) && p.shape(id).as_ref().is_some_and(holds)
        }) else {
            return (have as u64, ((from + have) as u64, Lead::Quiet));
        };
        let (mut missed, mut done) = (0, 0);
        let mut lead = match self.after {
            (at, lead) if at == from as u64 => lead,
            _ => Lead::Clear,
        };
        let mut gaps = Gaps::default();
        while done < have {
            let count = (have - done).min(scratch.len() / ch);
            let buf = &mut scratch[..count * ch];
            missed += p.read_gaps(id, (from + done) as u64, buf, &mut gaps);
            gaps.smooth(buf, ch, lead);
            lead = gaps.lead_after(buf, ch);
            add_frames(buf, ch, out, j0 + done, gain);
            done += count;
        }
        (missed, ((from + have) as u64, lead))
    }

    /// [`Take::add_into`] for a take partly on its file: its frames below
    /// `freed` from the file ([`Take::add_from_file`]), the rest from the
    /// pool. Gives the frames that played silence for want of them, and
    /// where what it played went quiet.
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
    ) -> (u64, (u64, Lead)) {
        let n = out.len() / 2;
        // Frames of the run before the pool's part.
        let split = (freed as i64).saturating_sub(k).clamp(0, n as i64) as usize;
        let (file, rest) = out.split_at_mut(2 * split);
        let (missed, after) = if split > 0 {
            self.add_from_file(player, id, gen, freed as usize, k, file, scratch, gain)
        } else {
            (0, self.after)
        };
        if split < n {
            // The pool's part plays on from there, never missing.
            self.add_into(pool, id, k + split as i64, rest, &|j| gain(j + split));
            return (missed, CLEAR);
        }
        (missed, after)
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
    /// Its clips playing, its song's window and the answer's mute
    /// ([`clips`]).
    lane: Box<Lane>,
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
            lane: Box::new(Lane::new()),
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
    /// edge are cut there ([`cut`]: a cut start moves `src` on, and each
    /// part keeps its share of the fades), and one across the whole
    /// stretch becomes two. Checked by `clear_fits` first.
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
                self.regions[i] = cut(&r, from, map, rate).0;
                self.regions.copy_within(i + 1..self.n, i + 2);
                self.regions[i + 1] = cut(&r, to, map, rate).1;
                self.n += 1;
                i += 2;
                continue;
            }
            self.regions[i] = if r.start < from {
                cut(&r, from, map, rate).0
            } else {
                cut(&r, to, map, rate).1
            };
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
    /// The song edit the step belongs to, by its transaction (none: no
    /// song edit's): see
    /// [`Command::Retime`].
    song: TransactionId,
    /// The step's transaction (audio tracks phase 4d): its song edit's
    /// when made as one, else its own, given with its number
    /// ([`AudioTracks::own_steps`]).
    txn: TransactionId,
    regions: [Region; MAX_REGIONS],
}

impl Snap {
    fn new() -> Snap {
        Snap {
            track: 0,
            n: 0,
            step: 0,
            song: TransactionId::NONE,
            txn: TransactionId::NONE,
            regions: [Region::EMPTY; MAX_REGIONS],
        }
    }
}

/// A step of the audio tracks' history that is their own (no song
/// edit's), by its transaction, for the one UNDO across the engines
/// (audio tracks phase 4d: [`AudioTracks::own_steps`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnStep {
    pub txn: TransactionId,
    /// The track of its first snap, and how many tracks it changed.
    pub track: usize,
    pub tracks: usize,
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
pub(crate) fn fade_table() -> [f32; FADE + 1] {
    let mut t = [0.0; FADE + 1];
    for (j, v) in t.iter_mut().enumerate() {
        *v = ((j as f64 / FADE as f64) * std::f64::consts::FRAC_PI_2).sin() as f32;
    }
    t
}

/// How a region meets its neighbour on the lane.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Join {
    /// Anything else (a comp boundary, a punch, another take): each side
    /// fades over [`FADE`] frames, equal power.
    Fade,
    /// The same take running straight on at the same gain (a Split, or
    /// rows cut from one region): no fade, so the audio is untouched.
    Straight,
    /// The same take running on at another gain: equal gain over
    /// [`FADE`] frames, which on the same audio is a straight ramp from
    /// one gain to the other. Equal power there would be 3 dB louder in
    /// the middle.
    Ramp,
}

/// How `b` meets `a`, the region before it, along `map`. The same take
/// running on (`b` starts where `a` ends, on the frame `a` would play
/// next) is [`Join::Straight`] when the gain runs on too: `a`'s gain with
/// its fades on that frame is `b`'s. So the two parts of a Split, which
/// keep their shares of one region's fades, play as that region did.
fn join(a: &Region, b: &Region, map: &TempoMap, rate: u32) -> Join {
    if b.start != a.end || b.take != a.take {
        return Join::Fade;
    }
    // The lengths only where the audio may run on (every region is
    // placed again each run).
    let frames = |r: &Region| r.frames_to(map, r.start as f64, r.end as f64, rate);
    let len_a = frames(a);
    if !runs_on(a, len_a, b, rate) {
        return Join::Fade;
    }
    ends_meet(a, len_a, b, frames(b))
}

/// How `b` meets `a` where it starts as `a` ends, `len_a` and `len_b`
/// frames long as they play: [`join`] past the ticks (clips place their
/// regions themselves). The audio runs on when `b` starts on the frame
/// of the same recording `a` would play next, on the same mapping (both
/// Off, or both following the tempo from the same made-at tempo).
fn join_run(a: &Region, len_a: i64, b: &Region, len_b: i64, rate: u32) -> Join {
    if !runs_on(a, len_a, b, rate) {
        return Join::Fade;
    }
    ends_meet(a, len_a, b, len_b)
}

/// `b` starts on the frame of the same recording `a`, `len_a` frames
/// long as it plays, would play next, on the same mapping.
fn runs_on(a: &Region, len_a: i64, b: &Region, rate: u32) -> bool {
    b.take == a.take
        && b.per_tick(rate) == a.per_tick(rate)
        && b.src == a.src + a.src_len(len_a, rate)
}

/// The audio runs on from `a` into `b`: straight on if their gains meet,
/// else a ramp.
fn ends_meet(a: &Region, len_a: i64, b: &Region, len_b: i64) -> Join {
    let end_a = a.gain * Envelope::of(a, len_a).at(len_a);
    let start_b = b.gain * Envelope::of(b, len_b).at(0);
    if end_a == start_b {
        Join::Straight
    } else {
        Join::Ramp
    }
}

/// `r` cut at tick `at` (inside it) along `map`: the part before and the
/// part after. Each keeps the region's fades and counts the frames cut
/// off it ([`Region::into`], [`Region::left`]), so it plays exactly its
/// share of them; the part after plays on from where `r` had got to.
fn cut(r: &Region, at: Tick, map: &TempoMap, rate: u32) -> (Region, Region) {
    let add = |n: u32, k: i64| (i64::from(n) + k).clamp(0, i64::from(u32::MAX)) as u32;
    let (start, at_f, end) = (r.start as f64, at as f64, r.end as f64);
    // On the region's own rounding: the part before is `k` frames, and
    // the part after starts on frame `k` of it, where its recording is.
    let k = r.frames_to(map, start, at_f, rate);
    let len = r.frames_to(map, start, end, rate);
    let (mut before, mut after) = (*r, *r);
    before.end = at;
    before.left = add(r.left, len - k);
    after.start = at;
    // Where its recording is at the cut, on its own mapping (Off: frame
    // `k`, where the part before ends).
    let x = r.src_pos(map, start, at_f, rate);
    after.src = r.src + x.round() as i64;
    after.phase = (x - x.round()) as f32;
    after.into = add(r.into, k);
    (before, after)
}

/// A region's own fades as it plays, `len_f` frames long: its fade in and
/// fade out measured on the region it was cut from, `into` frames before
/// it and `left` after it, each at most half that region.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Envelope {
    into: i64,
    /// Frames from its start to the end of the region it was cut from.
    end: i64,
    fade_in: i64,
    fade_out: i64,
}

impl Envelope {
    pub(crate) fn of(r: &Region, len_f: i64) -> Envelope {
        let (into, left) = (i64::from(r.into), i64::from(r.left));
        let half = (into + len_f + left).max(0) / 2;
        Envelope {
            into,
            end: len_f + left,
            fade_in: i64::from(r.fade_in).min(half),
            fade_out: i64::from(r.fade_out).min(half),
        }
    }

    /// The gain at frame `e` of it (1 between the fades). Past its end a
    /// fade out goes on along the same line and stays at nothing there.
    pub(crate) fn at(&self, e: i64) -> f32 {
        let mut g = 1.0;
        let from_start = e + self.into;
        if self.fade_in > 0 && from_start < self.fade_in {
            g *= from_start.max(0) as f32 / self.fade_in as f32;
        }
        let to_end = self.end - e;
        if self.fade_out > 0 && to_end < self.fade_out {
            g *= to_end.max(0) as f32 / self.fade_out as f32;
        }
        g
    }

    /// The frames of it the fades leave alone: from the first up to the
    /// second, which may be past its end (the caller stops there).
    fn plain(&self) -> (i64, i64) {
        (self.fade_in - self.into, self.end - self.fade_out)
    }
}

/// A region as a run plays it: from song tick `at` (its start, or a
/// clip pass's start and its start in the clip), `len_f` frames long (its
/// ticks' frames along the tempo map, or more for a clip's ring), meeting
/// the region before it with `before` and the one after with `after`,
/// heard through `shape` (a song's window, or a clip voice's window and
/// gain).
struct Placed<'a> {
    r: &'a Region,
    at: f64,
    len_f: i64,
    before: Join,
    after: Join,
    shape: Shape,
    /// The copy it plays (a slot of [`copies::Copies`]), on Stretch with
    /// its copy here when it was first heard; else its recording.
    copy: Option<usize>,
}

/// What a run reads regions through: the takes, the pool, the stream
/// player and its room, the crossfade table and the counts. One place
/// adds a region, for the song's regions and (phase 2) clips alike.
struct Reader<'a> {
    takes: &'a mut [Take],
    pool: &'a Pool,
    fade: &'a [f32; FADE + 1],
    player: &'a mut Option<Box<Player>>,
    reads: &'a mut [f32],
    /// A read following the tempo: the recording frames it gathers
    /// (stereo, [`GATHER`] frames) and what it reads from them (stereo,
    /// [`RUN`] frames).
    gathered: &'a mut [f32],
    pitched: &'a mut [f32],
    stats: &'a mut Stats,
    copies: &'a mut copies::Copies,
}

impl Reader<'_> {
    /// The copy region `r` plays as it starts at song tick `at` on lane
    /// `owner` (a voice started on frame `since`), heard on frame `f`:
    /// decided as it is first heard (`decide`), or only looked up. Ends
    /// `ends`, its copy covering `reach` of its recording after its start.
    /// None (its recording) unless it is on Stretch and its copy was here
    /// when it was first heard.
    #[allow(clippy::too_many_arguments)]
    fn source(
        &mut self,
        owner: u16,
        since: u64,
        r: &Region,
        at: f64,
        ends: copies::Ends,
        reach: i64,
        map: &TempoMap,
        rate: u32,
        f: u64,
        decide: bool,
    ) -> Option<usize> {
        if r.follow != FollowTempo::Stretch {
            return None;
        }
        let tk = self
            .takes
            .get(r.take as usize)
            .filter(|k| k.live && !k.loading && !k.recording)?;
        let key = copies::steady(map, at, (r.end - r.start) as f64)
            .and_then(|bpm| copies::CopyKey::of(r, tk.rec, tk.frames, bpm, rate, ends, reach));
        let id = copies::instance(owner, at, since, r);
        self.copies.source(id, key, f, decide)
    }

    /// Adds the copy in `slot` into `dst` (a run's frames), from `e0`
    /// frames past its region's start: at one `plain` gain, or at
    /// `gain(j)` on frame `j` of the run. Kept apart from
    /// [`Reader::add`], so the regions without copies cost what they did.
    #[inline(never)]
    fn add_copied(
        &mut self,
        slot: usize,
        e0: i64,
        dst: &mut [f32],
        plain: Option<f32>,
        gain: &impl Fn(usize) -> f32,
    ) {
        match plain {
            Some(g) => self.copies.add(slot, e0, dst, &|_| g),
            None => self.copies.add(slot, e0, dst, gain),
        }
        self.stats.copied += (dst.len() / 2) as u64;
    }

    /// Add `p` into `dst` (interleaved stereo, a run's frames) from song
    /// tick `tick0`, frame `f0`: its gain, its own fades, the crossfades
    /// its joins ask for and the [`FADE`] frames past its end, through its
    /// shape; frames from memory or its file, those that miss counted.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        p: &Placed,
        map: &TempoMap,
        rate: u32,
        bpm: f64,
        tick0: f64,
        f0: u64,
        dst: &mut [f32],
    ) {
        let (r, len_f, before, after) = (p.r, p.len_f, p.before, p.after);
        let len = dst.len() / 2;
        let outer = p.shape.steady(f0, len);
        if outer == Some(0.0) {
            return;
        }
        let Some(tk) = self
            .takes
            .get(r.take as usize)
            .filter(|k| k.live && !k.loading)
        else {
            return;
        };
        // Frames since the region's start at the run's first frame.
        let e0 = r.frames_to(map, p.at, tick0, rate);
        let stop = len_f
            + if after == Join::Straight {
                0
            } else {
                FADE as i64
            };
        if e0 >= stop || e0 + len as i64 <= 0 {
            return;
        }
        let env = Envelope::of(r, len_f);
        let fade = self.fade;
        let shape = p.shape;
        // The shape's gain at frame `j` of the run.
        let heard = |j: usize| -> f32 {
            match outer {
                Some(g) => g,
                None => shape.at(f0 + j as u64, fade),
            }
        };
        let gain_at = |e: i64| -> f32 {
            if e < 0 || e >= stop {
                return 0.0;
            }
            let mut g = r.gain;
            if e < FADE as i64 {
                match before {
                    Join::Fade => g *= fade[e as usize],
                    Join::Ramp => g *= e as f32 / FADE as f32,
                    Join::Straight => {}
                }
            }
            if e >= len_f {
                // Never Straight: then `stop` is `len_f`.
                let j = FADE as i64 - (e - len_f);
                match after {
                    Join::Ramp => g *= j as f32 / FADE as f32,
                    _ => g *= fade[j as usize],
                }
            }
            // Its own fades. A fade out reaches nothing at the end of the
            // region it was cut from, and the crossfade past its end goes
            // on from where it got to.
            g * env.at(e)
        };
        let (plain_from, plain_to) = env.plain();
        let edge = if before == Join::Straight {
            0
        } else {
            FADE as i64
        };
        // The whole run past the fades and crossfades: one gain.
        let plain =
            outer.is_some() && e0 >= plain_from.max(edge) && e0 + len as i64 <= plain_to.min(len_f);
        let g = r.gain * outer.unwrap_or(1.0);
        // On Stretch with its copy: the copy's frames at speed 1, from
        // where the region is ([`copies`]).
        if let Some(slot) = p.copy {
            let gain = |j: usize| gain_at(e0 + j as i64) * heard(j);
            self.add_copied(slot, e0, dst, plain.then_some(g), &gain);
            return;
        }
        // Following the tempo at another speed than its own: read between
        // its frames ([`Reader::add_pitched`]). At its own speed it plays
        // its frames as Off does, from the frame its mapping says.
        let speed = r.speed(bpm);
        if let Some(sp) = speed.filter(|&s| s != 1.0) {
            if r.follow == FollowTempo::Stretch {
                self.stats.for_now += len as u64;
            }
            let x = r.src as f64 + r.src_pos(map, p.at, tick0, rate);
            let gain = |j: usize| {
                if plain {
                    g
                } else {
                    gain_at(e0 + j as i64) * heard(j)
                }
            };
            let missed = self.add_pitched(r.take, x, sp, dst, &gain);
            let m = &mut self.takes[r.take as usize];
            // Read between its frames, it ends on no frame a plain read
            // could go on from.
            m.after = CLEAR;
            if missed > 0 {
                m.missed += missed;
                m.miss_tick = tick0;
                self.stats.missed += missed;
            }
            return;
        }
        let k = match speed {
            Some(_) => r.src + r.src_frames_to(map, p.at, tick0, rate),
            None => r.src + e0,
        };
        let (pool, reads) = (self.pool, &mut *self.reads);
        let (missed, after) = match tk.store {
            Store::File { gen } => {
                let p = self.player.as_deref_mut();
                let end = tk.frames;
                if plain {
                    tk.add_from_file(p, r.take, gen, end, k, dst, reads, &|_| g)
                } else {
                    let gain = |j: usize| gain_at(e0 + j as i64) * heard(j);
                    tk.add_from_file(p, r.take, gen, end, k, dst, reads, &gain)
                }
            }
            Store::Part { gen, freed, .. } if freed > 0 => {
                let p = self.player.as_deref_mut();
                if plain {
                    tk.add_part(pool, p, r.take, gen, freed, k, dst, reads, &|_| g)
                } else {
                    let gain = |j: usize| gain_at(e0 + j as i64) * heard(j);
                    tk.add_part(pool, p, r.take, gen, freed, k, dst, reads, &gain)
                }
            }
            Store::Pool | Store::Part { .. } => {
                if plain {
                    tk.add_into(pool, r.take, k, dst, &|_| g);
                } else {
                    tk.add_into(pool, r.take, k, dst, &|j| gain_at(e0 + j as i64) * heard(j));
                }
                (0, CLEAR)
            }
        };
        let m = &mut self.takes[r.take as usize];
        m.after = after;
        if missed > 0 {
            m.missed += missed;
            m.miss_tick = tick0;
            self.stats.missed += missed;
        }
    }
}

impl Reader<'_> {
    /// Adds what take `id` plays from recording frame `x` (with its
    /// fraction), `speed` recording frames an output frame, into `dst` (a
    /// run's frames, stereo), `gain(j)` weighing frame `j`: the frames the
    /// run passes, gathered from the pool or the take's file with the
    /// read's neighbours either side, read between with the streaming
    /// cursor's band-limited read
    /// ([`mpfree_stream::cursor::read_gathered`], `Read::Sinc8`), which
    /// the pitched-reads standard measures. Faster than [`MAX_SPEED`] it
    /// plays nothing, counted ([`Stats::too_fast`]). Gives the frames
    /// passed that played silence for want of them (the neighbours aren't
    /// counted). Never inlined, so [`Reader::add`] stays small for the
    /// read of a region played as recorded.
    #[inline(never)]
    fn add_pitched(
        &mut self,
        id: u16,
        x: f64,
        speed: f64,
        dst: &mut [f32],
        gain: &impl Fn(usize) -> f32,
    ) -> u64 {
        let len = dst.len() / 2;
        if len == 0 {
            return 0;
        }
        if !(speed > 0.0 && speed <= MAX_SPEED) {
            self.stats.too_fast += len as u64;
            return 0;
        }
        let Some(tk) = self
            .takes
            .get(id as usize)
            .filter(|k| k.live && !k.loading)
            .copied()
        else {
            return 0;
        };
        let base = x.floor();
        let frac = x - base;
        let (lead, tail) = mpfree_stream::cursor::reach_of(READ, speed);
        let passed = (frac + (len - 1) as f64 * speed).floor() as usize + 1;
        let n = lead + passed + tail;
        let first = base as i64 - lead as i64;
        let g = &mut self.gathered[..2 * n];
        g.fill(0.0);
        let mut missed = 0;
        for (from, to, counted) in [
            (0, lead, false),
            (lead, lead + passed, true),
            (lead + passed, n, false),
        ] {
            let p = self.player.as_deref_mut();
            let m = fetch(
                &tk,
                id,
                self.pool,
                p,
                first + from as i64,
                &mut g[2 * from..2 * to],
                self.reads,
            );
            if counted {
                missed += m;
            }
        }
        let out = &mut self.pitched[..2 * len];
        mpfree_stream::cursor::read_gathered(READ, g, 2, lead, frac, speed, out);
        for (j, (d, o)) in dst.chunks_exact_mut(2).zip(out.chunks_exact(2)).enumerate() {
            let w = gain(j);
            d[0] += w * o[0];
            d[1] += w * o[1];
        }
        missed
    }
}

/// The read a region following the tempo reads between its frames with:
/// band-limited, 8 zero crossings a side, which meets every row of the
/// pitched-reads standard at every speed, so a region never changes read
/// (and needs no fade between two) as the tempo moves.
const READ: mpfree_stream::cursor::Read = mpfree_stream::cursor::Read::Sinc8;

/// Take `id`'s frames `k..` into `out` (stereo, interleaved, zeroed by
/// the caller), from wherever they are, as a region reads them: a frame
/// it doesn't have is silence. Gives the frames missing from the stream
/// cache.
#[allow(clippy::too_many_arguments)]
fn fetch(
    tk: &Take,
    id: u16,
    pool: &Pool,
    player: Option<&mut Player>,
    k: i64,
    out: &mut [f32],
    scratch: &mut [f32],
) -> u64 {
    let one = |_: usize| 1.0;
    match tk.store {
        Store::File { gen } => {
            tk.add_from_file(player, id, gen, tk.frames, k, out, scratch, &one)
                .0
        }
        Store::Part { gen, freed, .. } if freed > 0 => {
            tk.add_part(pool, player, id, gen, freed, k, out, scratch, &one)
                .0
        }
        Store::Pool | Store::Part { .. } => {
            tk.add_into(pool, id, k, out, &one);
            0
        }
    }
}

/// Sound put on a track's lane by an undo step of its own (a take
/// punched out, a swipe on a take lane, a region moved, a sound brought
/// in): the track, the song ticks it covers and the step, for the app to
/// cut the song's blocks there and tie the step to that cut (the audio
/// tracks rethink's phase 2, step 2d; [`AudioTracks::take_took`],
/// [`Command::TieStep`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Took {
    pub track: usize,
    pub start: Tick,
    pub end: Tick,
    pub step: u32,
}

/// [`Took`]s kept between two asks: far more than a period's commands
/// put.
const TOOK: usize = 32;

/// The audio tracks: see the module notes.
pub struct AudioTracks {
    /// Sound put on lanes since the app last asked ([`Took`]), oldest
    /// first, and how many were lost for want of room.
    took: [Took; TOOK],
    took_n: usize,
    took_lost: u32,
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
    /// The last step's number ([`Snap::step`]), and its transaction
    /// when it is no song edit's ([`Snap::txn`]).
    step: u32,
    step_txn: TransactionId,
    /// The newest step's transaction the history let go of from its
    /// oldest end ([`AudioTracks::own_steps`]).
    let_go_txn: TransactionId,
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
    /// A read following the tempo: the frames it gathers and what it
    /// reads from them ([`Reader::add_pitched`]).
    gathered: Box<[f32]>,
    pitched: Box<[f32]>,
    /// Room for one clip voice's run, before it joins its track.
    vbuf: Box<[f32]>,
    /// A take was let go while a voice played it ([`Take::let_go`]).
    let_go: bool,
    /// Where each track plays, or is about to (see [`AudioTracks::cue`]),
    /// a region in each of its two cursors.
    cues: [[Option<Cue>; 2]; MAX_TRACKS],
    /// Take ids kept for takes on their way ([`AudioTracks::reserve`]).
    reserved: [bool; MAX_TAKES],
    /// No take's pool memory goes to its file for now
    /// ([`AudioTracks::keep_memory`]).
    keep: bool,
    /// Stretch's copies and what each region heard plays (phase 5b).
    copies: copies::Copies,
}

/// One track's room in [`AudioTracks::inside_input`].
const INSIDE: usize = 2 * MAX_PERIOD as usize;

impl AudioTracks {
    /// Audio tracks at `rate` with memory for `seconds` of stereo audio
    /// (twice that of mono takes), all allocated here.
    pub fn new(rate: u32, seconds: u32) -> AudioTracks {
        // The reads' tables, made here, never on the audio thread.
        mpfree_stream::cursor::prepare_reads();
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
            step_txn: TransactionId::NONE,
            let_go_txn: TransactionId::NONE,
            took: [Took::default(); TOOK],
            took_n: 0,
            took_lost: 0,
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
            keep: false,
            reads: vec![0.0; 2 * RUN].into_boxed_slice(),
            gathered: vec![0.0; 2 * GATHER].into_boxed_slice(),
            pitched: vec![0.0; 2 * RUN].into_boxed_slice(),
            vbuf: vec![0.0; 2 * RUN].into_boxed_slice(),
            let_go: false,
            cues: [[None; 2]; MAX_TRACKS],
            reserved: [false; MAX_TAKES],
            copies: copies::Copies::new(),
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

    /// The stream player's own counters, while takes play from their
    /// files (for System status: frames played as silence).
    pub fn player_stats(&self) -> Option<mpfree_stream::player::Stats> {
        self.player.as_ref().map(|p| p.stats())
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

    /// Track `t`'s once clip still sounding past its stop (audio tracks
    /// phase 2, a ring): its clip number, and how far through its sound it
    /// is at frame `now`, 0 to 1, for the cell's dimmed playhead.
    pub fn ringing(&self, t: usize, now: u64) -> Option<(u32, f32)> {
        let v = self
            .tracks
            .get(t)?
            .lane
            .voices
            .iter()
            .find(|v| v.on && v.ring)?;
        let s = &v.cur;
        let first = s.regions[..s.n].first()?.src;
        let total = (s.last_frame - first).max(1) as f64;
        let done = now.saturating_sub(v.since) as f64;
        Some((v.clip, (done / total).clamp(0.0, 1.0) as f32))
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
            varied: t.varied,
            anchor: t.anchor,
            frames: t.frames,
            channels: t.channels as usize,
            recording: t.recording,
            cut_short: t.cut_short,
        })
    }

    /// Whether a clip, or an undo step holding one, keeps the take
    /// ([`Command::Keep`]).
    pub fn kept(&self, id: u16) -> bool {
        self.takes
            .get(id as usize)
            .is_some_and(|t| t.live && t.kept)
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

    /// The history's own steps (audio tracks phase 4d), each by its
    /// transaction, for the one UNDO across the engines: those Undo takes
    /// back into `done`, oldest first, and those Redo brings back into
    /// `redo`, the next first. A song edit's steps are left out: they are
    /// the song's, and its UNDO takes them back ([`Command::Retime`]).
    /// Returns the newest step's transaction the history let go of from
    /// its oldest end (full, or a song edit's steps forgotten with those
    /// under them: [`Command::ForgetSong`]); none if it never did. Both
    /// lists are cleared first; they allocate only to grow.
    pub fn own_steps(&self, done: &mut Vec<OwnStep>, redo: &mut Vec<OwnStep>) -> TransactionId {
        done.clear();
        redo.clear();
        let put = |to: &mut Vec<OwnStep>, from: usize, n: usize| {
            let mut last = None;
            for i in from..from + n {
                let s = &self.hist[(self.h_base + i) % HISTORY];
                if last == Some(s.step) {
                    if let Some(o) = to.last_mut().filter(|o| o.txn == s.txn) {
                        o.tracks += 1;
                    }
                    continue;
                }
                last = Some(s.step);
                if !s.song.is_some() {
                    to.push(OwnStep {
                        txn: s.txn,
                        track: s.track,
                        tracks: 1,
                    });
                }
            }
        };
        put(done, 0, self.h_done);
        put(redo, self.h_done, self.h_redo);
        self.let_go_txn
    }

    /// Whether a region, or a step of the history, plays this take, or a
    /// clip keeps it ([`Command::Keep`]).
    pub fn in_use(&self, id: u16) -> bool {
        self.in_use_apart_from_voices(id) || self.tracks.iter().any(|t| t.lane.uses(id))
    }

    /// [`AudioTracks::in_use`] but for the clip voices.
    fn in_use_apart_from_voices(&self, id: u16) -> bool {
        let uses = |rs: &[Region]| rs.iter().any(|r| r.take == id);
        self.kept(id)
            || self.tracks.iter().any(|t| uses(t.regions()))
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
        self.step_txn = TransactionId::next();
        self.step
    }

    /// Save the scratch copy (a track's regions before an edit) in undo
    /// step `step`, of song edit `song` (by its transaction; none: no song
    /// edit's), dropping anything Redo
    /// could bring back and, when full, the oldest step (all its tracks).
    fn push_history(&mut self, step: u32, song: TransactionId) {
        if self.h_done == HISTORY {
            let oldest = self.hist[self.h_base].step;
            self.let_go_txn = self.hist[self.h_base].txn;
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
        dst.txn = if song.is_some() { song } else { self.step_txn };
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
    pub fn undo_is_song(&self) -> Option<TransactionId> {
        self.undo_snap().map(|s| s.song).filter(|s| s.is_some())
    }

    /// The same for Redo.
    pub fn redo_is_song(&self) -> Option<TransactionId> {
        self.redo_snap().map(|s| s.song).filter(|s| s.is_some())
    }

    /// Where song edit `song`'s step is: next for Undo (`undo`) or Redo,
    /// under other steps, or gone.
    pub fn song_step(&self, song: TransactionId, undo: bool) -> SongStep {
        let next = if undo {
            self.undo_snap()
        } else {
            self.redo_snap()
        };
        if song.is_some() && next.is_some_and(|s| s.song == song) {
            return SongStep::Next;
        }
        let (from, n) = if undo {
            (0, self.h_done)
        } else {
            (self.h_done, self.h_redo)
        };
        let held = (from..from + n).any(|i| self.hist[(self.h_base + i) % HISTORY].song == song);
        if song.is_some() && held {
            SongStep::Under
        } else {
            SongStep::Gone
        }
    }

    /// Drop song edit `song`'s steps and every step only reachable
    /// through them: on the Undo side the newest of them and all older
    /// steps, on the Redo side the first of them and all steps redone
    /// after it ([`Command::ForgetSong`]).
    fn forget_song(&mut self, song: TransactionId) {
        if !song.is_some() {
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
            self.let_go_txn = self.hist[(self.h_base + i) % HISTORY].txn;
            self.h_base = (self.h_base + i + 1) % HISTORY;
            self.h_done -= i + 1;
        }
    }

    /// Let go of every step made at or before transaction `txn` from the
    /// oldest end of the Undo side, whole steps ([`Command::RetireThrough`]).
    fn retire_through(&mut self, txn: TransactionId) {
        while self.h_done > 0 {
            let s = &self.hist[self.h_base];
            if s.txn > txn {
                break;
            }
            let (step, gone) = (s.step, s.txn);
            self.let_go_txn = self.let_go_txn.max(gone);
            while self.h_done > 0 && self.hist[self.h_base].step == step {
                self.h_base = (self.h_base + 1) % HISTORY;
                self.h_done -= 1;
            }
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
        (0..self.h_done + self.h_redo)
            .any(|i| self.hist[(self.h_base + i) % HISTORY].song.is_some())
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
    fn retime(&mut self, song: TransactionId, r: &Retime) -> bool {
        if !song.is_some() || self.retime_refused(r).is_some() {
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

    /// [`AudioTracks::edit`] putting sound on `track`'s lane over
    /// `start..end`: once it goes in, a [`Took`] says so.
    fn put_edit(
        &mut self,
        track: usize,
        (start, end): (Tick, Tick),
        f: impl FnOnce(&mut Track, &TempoMap, u32) -> bool,
        map: &TempoMap,
    ) {
        let step = self.step;
        self.edit(track, f, map);
        if self.step == step {
            return;
        }
        let took = Took {
            track,
            start,
            end,
            step: self.step,
        };
        if self.took_n == TOOK {
            // Never, as the app asks every period: the oldest goes,
            // counted ([`AudioTracks::took_lost`]).
            self.took.copy_within(1.., 0);
            self.took_n -= 1;
            self.took_lost = self.took_lost.saturating_add(1);
        }
        self.took[self.took_n] = took;
        self.took_n += 1;
    }

    /// The oldest sound put on a lane since the last ask ([`Took`]), for
    /// the app to cut the song's blocks there.
    pub fn take_took(&mut self) -> Option<Took> {
        if self.took_n == 0 {
            return None;
        }
        let t = self.took[0];
        self.took.copy_within(1..self.took_n, 0);
        self.took_n -= 1;
        Some(t)
    }

    /// [`Took`]s lost for want of room (never, as the app asks every
    /// period), for the status line.
    pub fn took_lost(&self) -> u32 {
        self.took_lost
    }

    /// Undo step `step` becomes one of song edit `song`
    /// ([`Command::TieStep`]): false, nothing changed, when Undo can't
    /// reach it or it belongs to a song edit.
    fn tie_step(&mut self, step: u32, song: TransactionId) -> bool {
        let (base, done) = (self.h_base, self.h_done);
        let mine = move |k: usize| (base + k) % HISTORY;
        let snaps = || (0..done).map(mine);
        let mut found = false;
        for i in snaps() {
            if self.hist[i].step == step {
                if self.hist[i].song.is_some() {
                    return false;
                }
                found = true;
            }
        }
        if !song.is_some() || !found {
            return false;
        }
        for i in snaps() {
            if self.hist[i].step == step {
                self.hist[i].song = song;
            }
        }
        true
    }

    /// Silence `track` from `from` to `to`, as an undo step of song edit
    /// `song` (by its transaction; none: no song edit's); refused where no
    /// region plays.
    fn erase(&mut self, track: usize, from: Tick, to: Tick, song: TransactionId, map: &TempoMap) {
        if !self.tracks[track]
            .regions()
            .iter()
            .any(|r| r.start < to && r.end > from)
        {
            return self.refuse();
        }
        self.edit_of(
            track,
            song,
            |t, m, rate| {
                t.clear_fits(from, to, 0) && {
                    t.clear(m, rate, from, to);
                    true
                }
            },
            map,
        );
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
        self.edit_of(track, TransactionId::NONE, f, map)
    }

    /// [`AudioTracks::edit`], its step one of song edit `song` (by its transaction; none: no song
    /// edit's).
    fn edit_of(
        &mut self,
        track: usize,
        song: TransactionId,
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
            self.push_history(step, song);
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
                let r = Region::of(
                    from,
                    to,
                    take,
                    src_at(map, a, from as f64, rate),
                    tk.made_at(),
                );
                self.put_edit(
                    tk.track as usize,
                    (from, to),
                    |t, m, rate| t.put(m, rate, r),
                    map,
                );
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
                        let (left, right) = cut(&t.regions[i], at, m, rate);
                        t.regions[i] = left;
                        t.regions.copy_within(i + 1..t.n, i + 2);
                        t.regions[i + 1] = right;
                        t.n += 1;
                        true
                    },
                    map,
                );
            }
            Command::Erase { track, from, to } if ok_track(track) && from < to => {
                self.erase(track, from, to, TransactionId::NONE, map)
            }
            Command::EraseSong {
                song,
                track,
                from,
                to,
            } if ok_track(track)
                && from < to
                && song.is_some()
                && self.tracks[track].rec.is_none() =>
            {
                self.erase(track, from, to, song, map)
            }
            Command::Move { track, at, to } if ok_track(track) && to >= 0 => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                let r = self.tracks[track].regions[i];
                self.put_edit(
                    track,
                    (to, to + (r.end - r.start)),
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
            Command::SetFollow {
                track,
                at,
                follow,
                keep_pitch,
            } if ok_track(track) => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, _, _| {
                        let r = &mut t.regions[i];
                        (r.follow, r.keep_pitch) = (follow, keep_pitch);
                        true
                    },
                    map,
                );
            }
            Command::SetTempo { track, at, bpm }
                if ok_track(track) && (bpm == 0.0 || valid_bpm(bpm)) =>
            {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, _, _| {
                        t.regions[i].bpm = bpm;
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
                        let r = &mut t.regions[i];
                        (r.fade_in, r.fade_out) = (fade_in, fade_out);
                        // Fades set on it are its own, cut or not.
                        (r.into, r.left) = (0, 0);
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
            Command::TieStep { step, song } => {
                if !self.tie_step(step, song) {
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
            Command::ForgetRedo => self.h_redo = 0,
            Command::RetireThrough(txn) => self.retire_through(txn),
            Command::Forget(id) => {
                if !self.free_to_forget(id) || self.in_use(id) {
                    return self.refuse();
                }
                self.forget(id);
            }
            Command::Keep { take, keep } => {
                let Some(t) = self.takes.get_mut(take as usize).filter(|t| t.live) else {
                    return self.refuse();
                };
                t.kept = keep;
                t.let_go = false;
                if !keep && self.free_to_forget(take) && !self.in_use_apart_from_voices(take) {
                    if self.tracks.iter().any(|t| t.lane.uses(take)) {
                        // A voice still plays it: it goes once none does.
                        self.takes[take as usize].let_go = true;
                        self.let_go = true;
                    } else {
                        self.forget(take);
                    }
                }
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
            Command::Refile { take, rec, gen } => {
                let Some(t) = self
                    .takes
                    .get_mut(take as usize)
                    .filter(|t| t.live && t.rec == rec)
                else {
                    return self.refuse();
                };
                match &mut t.store {
                    Store::Part { gen: g, .. } | Store::File { gen: g } => *g = gen,
                    Store::Pool => self.refuse(),
                }
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
                                let x = (r.src as f64 + f64::from(r.phase)) * ratio;
                                (r.src, r.phase) = (x.round() as i64, (x - x.round()) as f32);
                                let scale = |n: u32| (f64::from(n) * ratio).round() as u32;
                                (r.into, r.left) = (scale(r.into), scale(r.left));
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
            f.varied = k.varied;
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
            varied: f.varied,
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
            t.lane.stop();
        }
        self.let_go = false;
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
        // Their copies go with them (nothing plays now).
        self.copies.unlatch();
        self.copies.clear();
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
        let span = (a.ceil() as Tick, tick(frames).floor() as Tick);
        self.put_edit(
            tk.track as usize,
            span,
            |t, m, rate| {
                let mut start = a.ceil() as Tick;
                let mut placed = false;
                for end in ends {
                    if end <= start {
                        continue;
                    }
                    let r = Region::of(
                        start,
                        end,
                        take,
                        src_at(m, a, start as f64, rate),
                        tk.made_at(),
                    );
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

    // ---- Stretch's copies (phase 5b) -------------------------------------

    /// Takes a finished copy in ([`copies`]), in place of one with its key.
    /// Gives it back if [`copies::MAX_COPIES`] are held (drop one first)
    /// or one with its key plays now. Real-time safe.
    #[allow(clippy::result_large_err)]
    pub fn put_copy(&mut self, c: copies::Copy) -> Result<(), copies::Copy> {
        self.copies.put(c)
    }

    /// Lets the copy with `key` go, unless a region plays it now: true if
    /// it went. Its audio waits in [`AudioTracks::take_dropped_copy`].
    pub fn drop_copy(&mut self, key: &copies::CopyKey) -> bool {
        self.copies.drop_key(key)
    }

    /// Lets every copy go that no region plays now (Clear copies): how
    /// many went. No recording is touched; each is made again when next
    /// needed.
    pub fn clear_copies(&mut self) -> usize {
        self.copies.clear()
    }

    /// A copy's audio let go, for the host to drop off the audio thread.
    pub fn take_dropped_copy(&mut self) -> Option<std::sync::Arc<[f32]>> {
        self.copies.take_dropped()
    }

    /// The copies held.
    pub fn copies(&self) -> impl Iterator<Item = copies::CopyView> + '_ {
        self.copies.views()
    }

    /// Whether the copy with `key` is held.
    pub fn has_copy(&self, key: &copies::CopyKey) -> bool {
        self.copies.has(key)
    }

    /// Memory the copies hold, and how many there are.
    pub fn copy_bytes(&self) -> (usize, usize) {
        (self.copies.bytes(), self.copies.len())
    }

    /// The copy key of song region `r` along `map`: None unless it is on
    /// Stretch over one steady tempo, its take finished and here.
    pub fn song_copy_key(&self, r: &Region, map: &TempoMap) -> Option<copies::CopyKey> {
        let tk = self
            .takes
            .get(r.take as usize)
            .filter(|k| k.live && !k.loading && !k.recording)?;
        let bpm = copies::steady(map, r.start as f64, (r.end - r.start) as f64)?;
        let reach = r.src_len(0, self.rate);
        copies::CopyKey::of(
            r,
            tk.rec,
            tk.frames,
            bpm,
            self.rate,
            copies::Ends::Edges,
            reach,
        )
    }

    /// The copy keys of clip `c`'s regions on Stretch at a steady `bpm`,
    /// as its voice works them out, each given to `each` with the region's
    /// place in the clip.
    pub fn clip_copy_keys(
        &self,
        c: &ClipPlay,
        bpm: f64,
        each: &mut dyn FnMut(usize, copies::CopyKey),
    ) {
        let mut set = clips::Set::EMPTY;
        set.fill(c, &self.takes_for(c));
        let regions = &set.regions[..set.n];
        for (i, r) in regions.iter().enumerate() {
            if r.follow != FollowTempo::Stretch {
                continue;
            }
            let Some(tk) = self
                .takes
                .get(r.take as usize)
                .filter(|k| k.live && !k.loading && !k.recording)
            else {
                continue;
            };
            let (ends, reach) = copies::clip_shape(
                regions,
                i,
                set.len,
                set.last_frame,
                Some(tk.frames),
                self.rate,
            );
            if let Some(k) = copies::CopyKey::of(r, tk.rec, tk.frames, bpm, self.rate, ends, reach)
            {
                each(i, k);
            }
        }
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
            let tk = &self.takes[id];
            let at = src_at(map, tk.anchor, start as f64, rate);
            let r = Region::of(start, end_t, rec.take, at, tk.made_at());
            self.put_edit(t, (start, end_t), |tr, m, rate| tr.put(m, rate, r), map);
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
        self.process_with(block, map, input, out, &[]);
    }

    /// [`AudioTracks::process`], with what each track plays this period
    /// from the arranger's player: `answers[t]` for track `t`, and its
    /// song for a track past the end ([`clips`]). Every change lands on
    /// its frame. Real-time safe.
    pub fn process_with(
        &mut self,
        block: &Block,
        map: &TempoMap,
        input: &[f32],
        out: &mut [f32],
        answers: &[Answer],
    ) {
        self.process_inner(block, map, input, out, answers, None);
    }

    /// [`AudioTracks::process_with`], with the tracks' loops (phase 4,
    /// [`loop_play`]): `loops` planned on the same block and answers
    /// ([`LoopPlayer::plan`]), and `src` each loop's buffer for the period
    /// by its id (the looper's slot buffer, rendered from `loops`' runs).
    /// Each track's loops go into its sound before its level, pan, mute and
    /// solo, so they act on a loop as on its regions. Real-time safe.
    #[allow(clippy::too_many_arguments)]
    pub fn process_with_loops<'s>(
        &mut self,
        block: &Block,
        map: &TempoMap,
        input: &[f32],
        out: &mut [f32],
        answers: &[Answer],
        loops: &LoopPlayer,
        src: &dyn Fn(u128) -> Option<&'s [f32]>,
    ) {
        self.process_inner(block, map, input, out, answers, Some((loops, src)));
    }

    fn process_inner<'s>(
        &mut self,
        block: &Block,
        map: &TempoMap,
        input: &[f32],
        out: &mut [f32],
        answers: &[Answer],
        loops: Option<(&LoopPlayer, LoopSource<'_, 's>)>,
    ) {
        let n = (block.frames as usize)
            .min(input.len() / 2)
            .min(out.len() / 2)
            .min(MAX_PERIOD as usize);
        let now = block.start_frame;
        if let Some(p) = self.player.as_mut() {
            p.begin();
        }
        self.copies.set_now(now);
        // Stopped: what each region plays is decided again as it is next
        // heard.
        if block
            .spans()
            .iter()
            .any(|s| !matches!(s.state, State::Playing | State::Recording))
        {
            self.copies.unlatch();
        }
        for i in 0..self.q_len {
            if let Some(c) = self.queue[i].take() {
                self.apply(c, map, now);
            }
        }
        self.q_len = 0;
        self.forget_let_go();
        self.copy_loads(n);
        for t in &mut self.tracks {
            t.buf[..2 * n].fill(0.0);
        }
        for a in answers.iter().take(MAX_TRACKS) {
            self.stats.superseded += u64::from(a.superseded);
        }
        // Each track's next change, and whether its answer's start has
        // been taken.
        let mut next = [0usize; MAX_TRACKS];
        let mut begun = [false; MAX_TRACKS];
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
                // The changes due now, then the next on any track ends
                // the run before it.
                self.answer(map, span, f, off, n, answers, &mut next, &mut begun);
                for (t, a) in answers.iter().enumerate().take(MAX_TRACKS) {
                    if let Some(c) = a.changes.get(next[t]) {
                        let to = c.frame as usize;
                        if to > off {
                            end = end.min(i + (to - off));
                        }
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
        // Each track's loops, before its level (phase 4).
        if let Some((loops, src)) = loops {
            for (t, tr) in self.tracks.iter_mut().enumerate() {
                loops.mix_into(t, src, &mut tr.buf[..2 * n]);
            }
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

    /// The generation of take `id`'s file source, if any of the take
    /// plays from its file (all of it, or what its file holds so far).
    pub fn file_gen(&self, id: u16) -> Option<u32> {
        match self.takes.get(id as usize).filter(|t| t.live)?.store {
            Store::Part { gen, .. } | Store::File { gen } => Some(gen),
            Store::Pool => None,
        }
    }

    /// Whether take `id` plays from its file (it holds no pool memory).
    pub fn on_file(&self, id: u16) -> bool {
        self.takes
            .get(id as usize)
            .is_some_and(|t| t.live && matches!(t.store, Store::File { .. }))
    }

    /// While `keep`, no take gives its pool memory back to its file (see
    /// [`Command::OnFile`]): what each has in the pool stays there, so a
    /// save reading takes from the pool and their files meanwhile finds
    /// each frame where it looked. Does not allocate.
    pub fn keep_memory(&mut self, keep: bool) {
        self.keep = keep;
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
            varied: f.varied,
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
    /// Whether take `id` is here and free to go: not recording, loading or
    /// being written to its file.
    fn free_to_forget(&self, id: u16) -> bool {
        self.takes
            .get(id as usize)
            .is_some_and(|t| t.live && !t.recording && !t.loading && !t.writing)
    }

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
        let rate = self.rate;
        let tail = mpfree_transport::time::ticks_in(FADE as f64, bpm, rate) + 1.0;
        let ahead = CUE_S * f64::from(rate);
        for t in 0..MAX_TRACKS {
            let tr = &self.tracks[t];
            if tr.rec.is_some() {
                continue;
            }
            // The region playing and the next: the song's, or the clip
            // playing's (each with the song tick it starts at, the frames
            // it plays to past its own, and its cursor).
            let mut next: [Option<(Region, f64, i64, usize)>; 2] = [None; 2];
            match tr.lane.mode {
                Mode::Clip { voice, .. } if tr.lane.voices[voice].on => {
                    next = tr.lane.voices[voice].upcoming(tick, tail);
                }
                Mode::Nothing => {}
                _ if !self.active => {}
                _ => {
                    let first = tr
                        .regions()
                        .partition_point(|r| r.end as f64 + tail <= tick);
                    for (k, i) in (first..(first + 2).min(tr.n)).enumerate() {
                        next[k] = Some((tr.regions[i], tr.regions[i].start as f64, 0, i % 2));
                    }
                }
            }
            for (r, at, more, slot) in next.into_iter().flatten() {
                let Some(tk) = self
                    .takes
                    .get(r.take as usize)
                    .filter(|k| k.live && k.store != Store::Pool)
                else {
                    continue;
                };
                // Its recording's frames from its start to the tick: 0
                // for one still to come, if it comes soon enough.
                let e0 = if at > tick {
                    if frames_from(map, tick, at, rate) > ahead {
                        break;
                    }
                    0
                } else {
                    r.src_frames_to(map, at, tick, rate)
                };
                let to = at + (r.end - r.start) as f64;
                // Following the tempo, a recording frame passes in less
                // or more than an output frame: the read ahead and the
                // fade past its end count its own frames.
                let speed = r.speed(bpm).unwrap_or(1.0);
                let len_f = r.src_frames_to(map, at, to, rate).max(more);
                let fade = (FADE as f64 * speed).ceil() as i64;
                let end = (r.src + len_f + fade).clamp(0, tk.frames as i64) as u64;
                let frame = (r.src + e0).max(0) as u64;
                if frame < end {
                    self.cues[t][slot] = Some(Cue {
                        take: r.take,
                        frame,
                        end,
                        speed,
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
                        speed: c.speed,
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
        if self.player.is_none() || self.keep {
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
        // Every region heard from here is heard anew.
        self.copies.unlatch();
        for t in &mut self.tracks {
            t.lane.jump();
        }
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
                if span.bpm != self.takes[id].bpm {
                    // A tempo point, a ramp's step or the tempo turned:
                    // the take has no one tempo now.
                    self.takes[id].varied = true;
                }
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
            let recording = tr.rec.is_some();
            if rolling {
                if recording {
                    // Replace: its clips go on silently under the take.
                    self.tracks[t].lane.hold();
                } else {
                    // The song only while the tracks are on (song mode);
                    // clips as the answer says.
                    if self.active {
                        self.play(t, map, span, tick0, f, off, len);
                    }
                    self.play_clips(t, map, span, tick0, f, off, len);
                }
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
    /// (frame `f0`, `off` of the period), through its song's window.
    #[allow(clippy::too_many_arguments)]
    fn play(
        &mut self,
        t: usize,
        map: &TempoMap,
        span: &Span,
        tick0: f64,
        f0: u64,
        off: usize,
        len: usize,
    ) {
        let song = self.tracks[t].lane.song;
        if song.silent(f0, len) {
            return;
        }
        let (from, to) = (self.tracks[t].lane.song_from, self.tracks[t].lane.song_to);
        let shape = Shape {
            win: song,
            ..Shape::OPEN
        };
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
            gathered,
            pitched,
            stats,
            copies,
            ..
        } = self;
        let tr = &mut tracks[t];
        let first = tr
            .regions()
            .partition_point(|r| r.end as f64 + tail <= tick0);
        let (regions, buf) = (&tr.regions[..tr.n], &mut tr.buf);
        let mut reader = Reader {
            takes,
            pool,
            fade,
            player,
            reads,
            gathered,
            pitched,
            stats,
            copies,
        };
        // The song's lane, for its regions' choices of copy ([`copies`]):
        // a song region's copy covers its own frames, with its edges.
        let owner = (t as u16) << 3;
        let source = |reader: &mut Reader, r: &Region, decide: bool| {
            // Only Stretch plays copies: nothing more is worked out for
            // the rest, so Off and Repitch cost what they did.
            if r.follow != FollowTempo::Stretch {
                return None;
            }
            let reach = r.src_len(0, rate);
            let at = r.start as f64;
            reader.source(
                owner,
                0,
                r,
                at,
                copies::Ends::Edges,
                reach,
                map,
                rate,
                f0,
                decide,
            )
        };
        let dst = &mut buf[2 * off..2 * (off + len)];
        for (i, r) in regions.iter().enumerate().skip(first) {
            let (start, end) = (r.start as f64, r.end as f64);
            if start >= tick1 || start + clips::LINE > to {
                break;
            }
            if end < from + clips::LINE {
                // Over before the song came in.
                continue;
            }
            let copy = source(&mut reader, r, true);
            // How it meets the regions either side ([`Join`]), or the
            // song's own start or end ([`clips::Lane::song_from`]); two
            // playing from different audio (a copy) fade.
            let before = match i.checked_sub(1) {
                _ if (start - from).abs() < clips::LINE => Join::Fade,
                Some(h) => {
                    let a = &regions[h];
                    let j = join(a, r, map, rate);
                    copies::join_sources(j, source(&mut reader, a, false), copy)
                }
                None => Join::Fade,
            };
            let after = if (end - to).abs() < clips::LINE {
                Join::Fade
            } else {
                regions.get(i + 1).map_or(Join::Fade, |n| {
                    let j = join(r, n, map, rate);
                    // The next is decided if it is heard this run.
                    let heard = (n.start as f64) < tick1;
                    copies::join_sources(j, copy, source(&mut reader, n, heard))
                })
            };
            let len_f = r.frames_to(map, start, end, rate);
            let placed = Placed {
                r,
                at: r.start as f64,
                len_f,
                before,
                after,
                shape,
                copy,
            };
            reader.add(&placed, map, rate, span.bpm, tick0, f0, dst);
        }
    }

    /// Add track `t`'s clip voices for `len` frames from song tick `tick0`
    /// (frame `f0`, `off` of the period), then its declick, and the
    /// answer's mute over what it played.
    #[allow(clippy::too_many_arguments)]
    fn play_clips(
        &mut self,
        t: usize,
        map: &TempoMap,
        span: &Span,
        tick0: f64,
        f0: u64,
        off: usize,
        len: usize,
    ) {
        let rate = self.rate;
        let AudioTracks {
            tracks,
            takes,
            pool,
            fade,
            player,
            reads,
            gathered,
            pitched,
            stats,
            vbuf,
            copies,
            ..
        } = self;
        let tr = &mut tracks[t];
        let lane = &mut *tr.lane;
        let dst = &mut tr.buf[2 * off..2 * (off + len)];
        if len == 0 {
            return;
        }
        lane.resume(f0, tick0);
        if lane.any_voice() {
            let mut reader = Reader {
                takes,
                pool,
                fade,
                player,
                reads,
                gathered,
                pitched,
                stats,
                copies,
            };
            let vb = &mut vbuf[..2 * len];
            for (k, v) in lane.voices.iter_mut().enumerate().filter(|(_, v)| v.on) {
                vb.fill(0.0);
                // Each voice its own lane for its regions' choices of copy.
                let owner = ((t as u16) << 3) | (k as u16 + 1);
                let on = reader.add_voice(v, owner, map, rate, span.bpm, tick0, f0, vb);
                for (d, s) in dst.iter_mut().zip(vb.iter()) {
                    *d += *s;
                }
                v.last = [vb[2 * len - 2], vb[2 * len - 1]];
                if !on {
                    v.on = false;
                    v.last = [0.0; 2];
                }
            }
        }
        lane.declick.add(dst);
        match lane.mute.settled(f0) {
            Some(1.0) => {}
            Some(g) => dst.iter_mut().for_each(|d| *d *= g),
            None => {
                for (j, d) in dst.chunks_exact_mut(2).enumerate() {
                    let g = lane.mute.at(f0 + j as u64);
                    d[0] *= g;
                    d[1] *= g;
                }
            }
        }
    }

    /// The answers' starts and changes due at frame `f` (`off` of the
    /// period's `n`), in order, a track at a time: `next` and `begun` keep
    /// each track's place in its answer. Nothing plays while the
    /// transport is stopped.
    #[allow(clippy::too_many_arguments)]
    fn answer(
        &mut self,
        map: &TempoMap,
        span: &Span,
        f: u64,
        off: usize,
        n: usize,
        answers: &[Answer],
        next: &mut [usize; MAX_TRACKS],
        begun: &mut [bool; MAX_TRACKS],
    ) {
        let rolling = matches!(span.state, State::Playing | State::Recording);
        let tick = span.tick_at(f);
        for t in 0..MAX_TRACKS {
            let a = answers.get(t).copied().unwrap_or_default();
            if !rolling {
                let lane = &mut self.tracks[t].lane;
                if lane.mode != Mode::Unknown || lane.any_voice() {
                    lane.stop();
                }
                begun[t] = false;
                while a
                    .changes
                    .get(next[t])
                    .is_some_and(|c| c.frame as usize <= off)
                {
                    next[t] += 1;
                }
                continue;
            }
            if !begun[t] {
                begun[t] = true;
                self.begin(t, &a.start, a.muted, f, tick, map);
            }
            while let Some(c) = a.changes.get(next[t]) {
                if c.frame as usize > off {
                    break;
                }
                next[t] += 1;
                if (c.frame as usize) < n {
                    self.change(t, c, f, map);
                }
            }
        }
    }

    /// Track `t` plays `plays` at frame `f` (song tick `tick`), the
    /// period's start: as it is if nothing played before; else carried
    /// on, a new stamp taken in, or switched to as a change.
    fn begin(&mut self, t: usize, plays: &Plays, muted: bool, f: u64, tick: f64, map: &TempoMap) {
        // A loop is the looper's (`loop_play`): nothing of the regions'.
        let plays = &plays.for_regions();
        let heard = if muted { 0.0 } else { 1.0 };
        let mode = self.tracks[t].lane.mode;
        match (mode, plays) {
            (Mode::Unknown, _) => {
                let lane = &mut self.tracks[t].lane;
                lane.mute = Glide::steady(heard);
                lane.song = if *plays == Plays::Song {
                    Window::OPEN
                } else {
                    Window::SHUT
                };
                lane.song_from = f64::NEG_INFINITY;
                lane.song_to = f64::INFINITY;
                let mode = match plays {
                    Plays::Nothing => Mode::Nothing,
                    Plays::Song => Mode::Song,
                    Plays::Clip(c) => Mode::Clip {
                        clip: c.clip,
                        voice: self.start_voice(t, c, f, tick),
                    },
                    // Made nothing above (`for_regions`).
                    Plays::Loop(_) => Mode::Nothing,
                };
                self.tracks[t].lane.mode = mode;
            }
            (Mode::Song, Plays::Song) | (Mode::Nothing, Plays::Nothing) => {
                self.tracks[t].lane.mute.set(heard, f);
            }
            (_, Plays::Clip(c)) if self.carries_on(t, c).is_some() => {
                self.carry_on(t, c, f, tick);
                self.tracks[t].lane.mute.set(heard, f);
            }
            _ => {
                let c = Change {
                    frame: 0,
                    tick,
                    plays: *plays,
                    muted,
                    ring: false,
                };
                self.change(t, &c, f, map);
            }
        }
    }

    /// Track `t` changes to what `c` says at frame `f`: what it played
    /// fades out (or rings, or runs straight on into the same audio) and
    /// what it plays now fades in. On the change's own song tick, the line
    /// it lands on: the frame's tick is within half a frame of it, either
    /// side, where lines fall between frames, and which pass or region a
    /// voice is in at a line can't hang on that.
    fn change(&mut self, t: usize, c: &Change, f: u64, map: &TempoMap) {
        // A loop is the looper's (`loop_play`): nothing of the regions'.
        let c = &Change {
            plays: c.plays.for_regions(),
            ..*c
        };
        let tick = c.tick;
        let rate = self.rate;
        // A ring from before ends with the usual fade.
        for v in self.tracks[t].lane.voices.iter_mut() {
            if v.on && v.ring {
                v.shape.win.close_at(f, Join::Fade);
            }
        }
        let heard = if c.muted { 0.0 } else { 1.0 };
        if let Plays::Clip(nc) = &c.plays {
            if self.carries_on(t, nc).is_some() {
                // The clip playing, muted or edited: no launch.
                self.carry_on(t, nc, f, tick);
                self.tracks[t].lane.mute.set(heard, f);
                return;
            }
        }
        let prev = self.tracks[t].lane.mode;
        // A song region running on through the change: cut there, crossed
        // over. Else its regions end or start there, or nothing plays.
        let song_through = {
            let rs = self.tracks[t].regions();
            let k = rs.partition_point(|r| r.end as f64 <= tick + clips::LINE);
            rs.get(k)
                .is_some_and(|r| (r.start as f64) < tick - clips::LINE)
        };
        let mode = match &c.plays {
            Plays::Nothing => Mode::Nothing,
            Plays::Song => Mode::Song,
            Plays::Clip(nc) => Mode::Clip {
                clip: nc.clip,
                voice: self.start_voice(t, nc, f, tick),
            },
            // Made nothing above (`for_regions`).
            Plays::Loop(_) => Mode::Nothing,
        };
        let takes = &self.takes;
        let lane = &mut self.tracks[t].lane;
        match prev {
            Mode::Song if mode != Mode::Song => {
                if song_through {
                    lane.song.close_at(f, Join::Fade);
                } else {
                    // Nothing of the song from here on; a region ending
                    // here fades out past its end by itself, as at a gap.
                    lane.song_to = tick;
                    lane.song.close_at(f + 2 * FADE as u64, Join::Straight);
                }
            }
            Mode::Clip { voice, .. } if lane.voices[voice].on => {
                let new = match mode {
                    Mode::Clip { voice: nv, .. } => Some(nv),
                    _ => None,
                };
                if new == Some(voice) {
                    // Taken for the new clip (a fifth voice): declicked.
                } else if c.ring {
                    lane.voices[voice].ring_from(tick);
                } else if lane.voices[voice].restart {
                    // Declicked at the jump already.
                    lane.voices[voice].on = false;
                } else {
                    // The song block it played ends here: nothing of its
                    // from here on plays. The same audio running on isn't
                    // faded: a legato switch to the same audio, or a song
                    // block's end into a block whose clip plays on from
                    // the frame its region ended on (Song → scenes cuts a
                    // region at a section's edge in two: step 2d).
                    let mut join = Join::Fade;
                    if let Some(nv) = new {
                        let a = lane.voices[voice].heard_into(map, rate, tick);
                        let b = lane.voices[nv].heard(map, rate, tick);
                        if let Some((a, b)) = a.zip(b) {
                            if a.0 == b.0 && a.1 == b.1 && a.0 != NO_TAKE_ID {
                                join = if a.2 == b.2 {
                                    Join::Straight
                                } else {
                                    Join::Ramp
                                };
                            }
                        }
                        lane.voices[nv].shape.win.open = join;
                    }
                    let old = &mut lane.voices[voice];
                    let frames =
                        |id: u16| takes.get(id as usize).filter(|k| k.live).map(|k| k.frames);
                    if old.runs_on(map, rate, tick, &frames) {
                        // Its audio runs on (through a region, or into
                        // the same audio next): crossed over to what
                        // follows.
                        old.shape.win.close_at(f, join);
                    } else {
                        // Its regions end by here, each meeting what
                        // follows by its own join as the song's regions
                        // do; nothing of its from here on plays, and the
                        // window only closes once their fades are over.
                        old.end_at(tick, join);
                        old.shape.win.close_at(f + 2 * FADE as u64, Join::Straight);
                    }
                }
            }
            _ => {}
        }
        if mode == Mode::Song && prev != Mode::Song {
            if song_through {
                lane.song = Window::opening(f, Join::Fade);
                lane.song_from = f64::NEG_INFINITY;
            } else {
                // A region starting here fades in by itself, as after a gap.
                lane.song = Window::opening(f, Join::Straight);
                lane.song_from = tick;
            }
            lane.song_to = f64::INFINITY;
        }
        lane.mode = mode;
        lane.mute.set(heard, f);
    }

    /// The voice playing clip `c` on track `t` as it is playing now (the
    /// same clip from the same origin), if any.
    fn carries_on(&self, t: usize, c: &ClipPlay) -> Option<usize> {
        let lane = &self.tracks[t].lane;
        match lane.mode {
            Mode::Clip { clip, voice }
                if clip == c.clip
                    && lane.voices[voice].on
                    && lane.voices[voice].origin == c.origin =>
            {
                Some(voice)
            }
            _ => None,
        }
    }

    /// Clip `c` carries on on track `t`: a new stamp from its next pass,
    /// its gain at once (from frame `f`, song tick `tick`).
    fn carry_on(&mut self, t: usize, c: &ClipPlay, f: u64, tick: f64) {
        let Some(voice) = self.carries_on(t, c) else {
            return;
        };
        if self.tracks[t].lane.voices[voice].stamp() != c.stamp {
            let takes = self.takes_for(c);
            self.tracks[t].lane.voices[voice].restamp(c, &takes, f, tick);
        } else {
            self.tracks[t].lane.voices[voice].shape.glide.set(c.gain, f);
        }
    }

    /// A voice on track `t` for clip `c` from frame `f` (song tick
    /// `tick`), fading in; a fifth taken from the quietest is counted.
    fn start_voice(&mut self, t: usize, c: &ClipPlay, f: u64, tick: f64) -> usize {
        let takes = self.takes_for(c);
        let lane = &mut self.tracks[t].lane;
        let (i, took) = lane.free_voice();
        if took {
            self.stats.declicked += 1;
        }
        lane.voices[i].start(c, &takes, f, tick, Join::Fade);
        i
    }

    /// The take each of `c`'s regions plays ([`NO_TAKE_ID`] for one not
    /// here), with the tempo it was made at (0: none).
    fn takes_for(&self, c: &ClipPlay) -> [(u16, f64); MAX_CLIP_REGIONS] {
        let mut out = [(NO_TAKE_ID, 0.0); MAX_CLIP_REGIONS];
        for (o, r) in out.iter_mut().zip(c.regions) {
            *o = self.take_of(r.rec).map_or((NO_TAKE_ID, 0.0), |id| {
                (id, self.takes[id as usize].made_at())
            });
        }
        out
    }

    /// Takes let go while a voice played them, forgotten once none does
    /// (and nothing else holds them).
    fn forget_let_go(&mut self) {
        if !self.let_go {
            return;
        }
        self.let_go = false;
        for id in 0..MAX_TAKES as u16 {
            if !self.takes[id as usize].let_go {
                continue;
            }
            if self.tracks.iter().any(|t| t.lane.uses(id)) {
                self.let_go = true;
                continue;
            }
            self.takes[id as usize].let_go = false;
            let tk = &self.takes[id as usize];
            if tk.live && !tk.kept && self.free_to_forget(id) && !self.in_use_apart_from_voices(id)
            {
                self.forget(id);
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
            let frames = tr.buf[..2 * n]
                .chunks_exact_mut(2)
                .zip(out.chunks_exact_mut(2));
            if from == target {
                // Steady (most periods): the same sound as the ramp, which
                // adds a step of nothing.
                for (d, o) in frames {
                    d[0] *= from[0];
                    d[1] *= from[1];
                    o[0] += d[0];
                    o[1] += d[1];
                }
            } else {
                let step = [
                    (target[0] - from[0]) / n.max(1) as f32,
                    (target[1] - from[1]) / n.max(1) as f32,
                ];
                for (j, (d, o)) in frames.enumerate() {
                    let k = (j + 1) as f32;
                    d[0] *= from[0] + step[0] * k;
                    d[1] *= from[1] + step[1] * k;
                    o[0] += d[0];
                    o[1] += d[1];
                }
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
