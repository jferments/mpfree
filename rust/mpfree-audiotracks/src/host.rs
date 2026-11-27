//! The audio tracks in a host: what the app needs around the engine, so
//! wiring it in is a few lines.
//!
//! [`AudioHost`] owns the [`AudioTracks`] (on the audio thread) and a
//! worker thread (`mpfree-audiotracks`) that makes and drops every big
//! buffer, so the audio loop never allocates audio memory:
//!
//! - **Each period:** [`AudioHost::period`] runs the tracks on the
//!   transport's block, then tends the save, the fit and the buffers.
//! - **Save:** [`AudioHost::save`] waits until nothing records or loads,
//!   has the worker make room for every take, reads the takes as they sit
//!   at [`READ_SPEED`] frames for each frame that plays, and gives the
//!   result in [`AudioHost::take_saved`]: the text
//!   ([`crate::save::to_text`]) and a WAV a take ([`crate::save::wav`])
//!   are what the project keeps. A take never changes once recorded, so a
//!   save read across many periods cannot tear.
//! - **Open:** [`AudioHost::load`] puts a saved part back (each take's
//!   audio copies in over the next periods).
//! - **Fit to tempo:** [`AudioHost::fit`] reads a take, stretches it on the
//!   worker to the tempo now (see [`crate::stretch`]), loads the result
//!   and swaps it in, as one undo step.
//! - **Waveforms:** [`AudioHost::wave`] reads a take's peaks into columns
//!   for the screen ([`TakeWave`]), at [`WAVE_SPEED`]. The pool lives with
//!   the tracks on the audio thread, so this is how the screen sees it.
//! - **Recording to files:** with [`AudioHost::record_to`], each take
//!   recorded goes to a file of its own as it records (see
//!   [`crate::files`]). The worker owns the writer and knows every take's
//!   file; each period the audio thread hands it the tracks' requests
//!   ([`crate::Io`]) and copies the takes recording out of the pool into
//!   buffers made once, at most [`COPY_SAMPLES`] a period, a buffer at a
//!   time. A save names each take's file and copies the finished ones
//!   whole ([`crate::save::TakeFile::path`]); only takes without one are
//!   read from memory. How each file stands comes back for the lanes
//!   ([`AudioHost::file`]) and the log ([`AudioHost::take_event`]).
//! - **Playing from files:** with [`AudioHost::stream`] (before
//!   [`AudioHost::record_to`]), the worker starts the stream layer (see
//!   [`crate::streaming`]): the player goes to the tracks, and the
//!   coordinator runs on a thread of its own, the writer waiting for its
//!   reads. Each recorded take's file is opened once the writer has
//!   finished it, and the take moves to it ([`crate::Command::OnFile`]); a
//!   project's takes open as their files with nothing copied in
//!   ([`AudioHost::load`]; its tracks go in once they have, without the
//!   regions of one left out, and a save waits for them). A waveform or
//!   a fit of a take on its file reads the file on the worker, behind
//!   playback's reads, and one reading a take from memory when it moves
//!   goes on from its file; a save copies it whole, as before.
//!
//! - **Renders:** [`AudioHost::render`] makes a recording from a
//!   [`crate::render::RenderDesc`] the one way the plan gives (phase 6.0.3:
//!   reserve, snapshot, render on the scheduler into a hidden `.part`,
//!   check, make durable, commit as one undo step, release); see
//!   `host/rendering.rs` and docs/dev/design/renders.md.
//!
//! The channels to the worker are `std::sync::mpsc`: a message now and
//! then can allocate a block of the channel, as in the looper's host. A
//! period with nothing under way sends nothing.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use mpfree_stream::player::Player;
use mpfree_stream::status::Watch;
use mpfree_stream::storage::{CardHandle, CardMark};
use mpfree_stream::writer::Gate;

use mpfree_transport::time::TempoMap;
use mpfree_transport::{Block, Mark, Tick};

use crate::copies::Copy;
use crate::copying::{Ask, ClipWant, CopyCounts, CopySide, CopyState, MAX_MAKING};
use crate::files::{
    self, DriveProbe, DriveSeen, FileEvent, FileView, Files, Growing, LayerWrite, Name, Problem,
    Said, WriterConfig,
};
use crate::render::{Provenance, RenderDesc};
use crate::save::{Saved, TakeFile};
use crate::streaming::{
    self, Board, FromStream, Head, Read, Stream, StreamConfig, Syncs, Timings, ToStream,
};
use crate::stretch::{fit_ratio, StretchJob};
use crate::{
    AudioTracks, Command, Io, RecordingId, TakeInfo, CHUNK, MAX_REGIONS, MAX_TAKES, MAX_TRACKS,
    PEAK_BLOCK,
};
use mpfree_stretch::sched::{self, Sched};

mod rendering;
use rendering::{render_key, Leftovers, RenderSide, RenderWork};
pub use rendering::{RenderFailure, RenderOutcome, RenderRefused};

/// The most starts held in the cache: every region of every track, and
/// each track's frame at the loop's start.
pub const MAX_HEADS: usize = MAX_TRACKS * (MAX_REGIONS + 1);
/// The most audio clip region starts held with them
/// ([`AudioHost::clip_starts`]).
pub const MAX_CLIP_STARTS: usize = 512;
/// Periods between looks at whether the regions' starts to hold have
/// changed (about 93 ms at 128 frames).
const HEADS_EVERY: u64 = 32;
/// The playhead moving into another stretch of this many ticks (four bars
/// of 4/4) sorts the starts again.
const HEADS_SPAN: Tick = 16 * mpfree_transport::time::PPQ as Tick;
/// The shortest start held, in seconds.
const MIN_HEAD_S: f64 = 0.25;
/// A held start lasts the plan's stall and this much more, in seconds: the
/// read for what follows it lands within the stall.
const HEAD_MARGIN_S: f64 = 0.5;
/// Takes being written to files at once: a sound brought in, a fit, or a
/// project's takes converted from another rate (every take at most).
const TO_WRITE: usize = MAX_TAKES;
/// Periods between asks again while the cache holds only some of the
/// starts (about 2.7 s at 128 frames): room may have come back.
const HEADS_RETRY: u64 = 1024;

/// Recording to files: samples copied out of the tracks' memory in one
/// period at most: 64 times what 8 stereo tracks record in a 128-frame
/// period would need a quarter of, a small, steady cost on the audio
/// thread.
pub const COPY_SAMPLES: usize = 8192;
/// Recording to files: buffers of [`CHUNK`] samples kept for copying takes
/// out to the worker (made once; each comes back once the writer has its
/// frames).
const SPARE: usize = 32;
/// Loop layers' files (phase 4c.3): buffers of [`CHUNK`] samples kept
/// for the looper's closed layers on their way to the worker (made once;
/// each comes back once written).
const LAYER_SPARE: usize = 4;
/// Loop layers' files' outcomes waiting to be taken, at most.
const LAYERS_DONE: usize = 64;
/// Events for the log waiting to be taken, at most.
const EVENTS: usize = 64;
/// Room for System status's reports between looks (made at start).
const PROBLEMS: usize = 64;
/// The player's counters go to System status every this many periods.
const PLAYER_LOOK_EVERY: u64 = 64;
/// Answers about cues' audio waiting to be taken, at most (made at start).
const CUE_ANSWERS: usize = 64;

/// What the host says about the audio a cue waits for
/// ([`AudioHost::hold_cue`], [`AudioHost::cue_audio`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CueAudio {
    /// It is all in memory or in the cache, and held there.
    Held,
    /// It can't be held now: a recording isn't here (yet), a file isn't
    /// open, or the cache has no room. Ask again later.
    Short,
    /// It was held, and the cache gave it back for room.
    GivenBack,
}

/// A take recording to its file: how far it has been copied out, and the
/// buffer filling for the worker.
struct Out {
    take: u16,
    rec: RecordingId,
    channels: usize,
    /// Frames copied out.
    sent: usize,
    buf: Option<Vec<f32>>,
    /// Samples in `buf`.
    fill: usize,
}

/// Recording to files, on the audio thread's side.
struct FileSide {
    spare: Vec<Vec<f32>>,
    outs: Vec<Out>,
    /// Each track's name for its take files.
    names: [Name; MAX_TRACKS],
    /// Each take's file as last said (by take id), with the recording it
    /// belongs to.
    views: Vec<Option<(RecordingId, FileView)>>,
    /// Each track's next take's files: the recording, and whether they are
    /// made.
    armed: [Option<(RecordingId, bool)>; MAX_TRACKS],
    events: Vec<FileEvent>,
    /// Reports for System status not taken yet ([`AudioHost::take_problem`]).
    problems: Vec<Problem>,
    /// Loop layers' files (phase 4c.3): the buffers for their pieces, and
    /// what came of each file, not taken yet ([`AudioHost::layer_written`]).
    layer_spare: Vec<Vec<f32>>,
    layers_done: Vec<(RecordingId, Option<String>)>,
    /// Loop layers' files read back, not taken yet
    /// ([`AudioHost::layer_read_back`]).
    layers_read: Vec<(RecordingId, Result<Vec<f32>, String>)>,
    /// Loop layers' files begun and not yet written or dropped: the takes'
    /// folder doesn't change under them ([`AudioHost::record_to`]).
    layers_open: Vec<RecordingId>,
}

/// Frames of a take a save or a fit reads for each frame that plays.
pub const READ_SPEED: usize = 64;

/// Frames of a take a waveform covers for each frame that plays (it reads
/// the peak table, one entry for each [`PEAK_BLOCK`] samples, so this is
/// about 4 entries a frame for a mono take).
pub const WAVE_SPEED: usize = 1024;

/// The most columns a waveform has.
pub const MAX_COLUMNS: usize = 4096;

/// A take's waveform for the screen: the lowest and highest sample of each
/// column of `frames_per_column` frames, over its channels.
#[derive(Clone, Debug, PartialEq)]
pub struct TakeWave {
    pub id: u16,
    /// Frames of the take it covers (a recording take grows: ask again).
    pub frames: usize,
    pub channels: usize,
    pub frames_per_column: usize,
    pub columns: Vec<(f32, f32)>,
}

// The audio thread sends these, so a large one ([`Ask::Make`], whose copy
// over a tempo change carries the song's map, 5d.1) isn't boxed: a box
// would be made on the audio thread.
#[allow(clippy::large_enum_variant)]
enum ToWorker {
    /// Room for a save (`Saved::room`), for the audio thread to fill.
    Room,
    /// Look at the drive takes record to with this, once they do.
    WatchDrive(DriveProbe, Arc<DriveSeen>),
    /// A filled room: trimmed to its first `tracks` and `takes`, each
    /// take named for its file, and room made for the audio of those
    /// whose file can't be copied whole ([`Files::name_save`]).
    Size {
        saved: Saved,
        tracks: usize,
        takes: usize,
    },
    /// A zeroed buffer of this many samples (a take to fit), for the
    /// tracks of generation `gen`.
    Buffer {
        n: usize,
        gen: u32,
    },
    Stretch {
        /// Boxed: the message stays small.
        take: Box<TakeInfo>,
        audio: Vec<f32>,
        ratio: f64,
        bpm: f64,
        id: u16,
        gen: u32,
    },
    /// Room for a waveform of this many columns.
    Wave(usize),
    Drop(Vec<f32>),
    DropSaved(Saved),
    DropWave(TakeWave),
    /// The card opened, its files under its root: a drive of its own
    /// (reads there get an I/O thread of their own, and a writer there
    /// waits only for that drive's reads), and an insertion that takes
    /// there open through.
    Device(Arc<dyn CardHandle>),
    /// Record into `dir` from now on.
    Files {
        dir: PathBuf,
        cfg: WriterConfig,
    },
    /// The drive takes record on went: every take in flight there carries
    /// on in `dir`, and takes record there from now on
    /// ([`crate::files::Files::carry_on`]).
    CarryOn(PathBuf),
    /// The card opening `mark` names is in: takes that carried on when it
    /// went get their first part back from it, looked for in `places`
    /// too ([`crate::files::Files::join_card`]).
    Join {
        mark: CardMark,
        places: Vec<PathBuf>,
    },
    /// The tracks' requests about the files ([`crate::Io`]), with what the
    /// worker needs to act on them.
    Arm {
        track: u8,
        rec: RecordingId,
        channels: u8,
        name: Name,
    },
    Disarm(RecordingId),
    /// Take `take` started recording into `rec`'s files.
    Start {
        rec: RecordingId,
        take: u16,
    },
    /// Frames of a recording, interleaved: the first `len` samples of
    /// `buf`, which comes back.
    Push {
        rec: RecordingId,
        buf: Vec<f32>,
        len: usize,
    },
    /// The recording stopped and every frame was pushed.
    Stop(RecordingId),
    /// Take `take` opened with a project, and its file there.
    Known {
        rec: RecordingId,
        take: u16,
        name: String,
        path: Option<PathBuf>,
        frames: usize,
        channels: usize,
    },
    /// Where a render made recording `rec` from (phase 6.0.1), for a save
    /// to write again; with no recording, only to be dropped.
    Prov {
        rec: RecordingId,
        prov: Provenance,
    },
    /// The tracks were emptied: what is asked from now on is for the
    /// tracks of generation `gen`.
    Clear {
        gen: u32,
    },
    /// A project save finished into this `samples/`.
    SavedInto(PathBuf),
    /// Start the stream layer: takes play from their files (boxed: sent
    /// once, at start, and far bigger than the rest).
    Stream(Box<StreamConfig>),
    /// Hold these regions' starts in the cache (the room comes back).
    Heads(Vec<Head>),
    /// Hold what cue `cue` needs from takes on their files.
    Cue {
        cue: u32,
        parts: Vec<(u16, u64, u64)>,
    },
    /// Cue `cue`'s hold goes.
    LetCueGo {
        cue: u32,
    },
    /// A loop's closed layer to write as recording `rec`'s file in the
    /// work folder (phase 4c.3, [`crate::files::LayerWrite`]): `frames`
    /// frames of `channels`, its pieces to follow.
    LayerBegin {
        rec: RecordingId,
        frames: usize,
        channels: usize,
    },
    /// The next piece of it: the first `len` samples of `buf`, which
    /// comes back.
    LayerPiece {
        rec: RecordingId,
        buf: Vec<f32>,
        len: usize,
    },
    /// Every piece of it has been sent: finish its file.
    LayerEnd(RecordingId),
    /// Stop writing it: its part file goes.
    LayerDrop(RecordingId),
    /// Read a loop layer's file this run wrote back, for UNDO (phase
    /// 4d.4): its audio comes back whole.
    LayerRead(RecordingId),
    /// Write a sound brought in, or a stretched copy, as recording
    /// `rec`'s take file in the work folder, for the tracks of generation
    /// `gen` (the audio is dropped once written, or comes back).
    WriteTake {
        rec: RecordingId,
        channels: usize,
        audio: Vec<f32>,
        gen: u32,
    },
    /// Close take `take`'s stream source, if still at generation `gen`.
    Close {
        take: u16,
        gen: u32,
    },
    /// The tracks were emptied: every stream source goes.
    Project,
    /// Fit a take on its file: read the file, then stretch it.
    FitFile {
        take: TakeInfo,
        ratio: f64,
        bpm: f64,
        gen: u32,
    },
    /// The waveform of a take on its file, in `columns` columns.
    WaveFile {
        take: TakeInfo,
        columns: usize,
    },
    /// A whole file read for one of the two above (from the stream
    /// thread).
    Read(Read),
    DropPlayer(Box<Player>),
    /// About Stretch's copies ([`crate::copying`]).
    Copy(Ask),
    /// A copy's audio, or the clips' old list, to drop.
    DropCopy(Arc<[f32]>),
    DropClips(Vec<ClipWant>),
    /// Make copies on this scheduler from now on (a test's).
    Sched(Sched),
    /// A zeroed buffer of `n` samples for a render's source (phase
    /// 6.0.3), for the tracks of generation `gen`.
    RenderBuffer {
        n: usize,
        gen: u32,
    },
    /// Render `desc`: `audio` (`frames` frames of `channels`) written as
    /// recording `rec`'s render file in the work folder, on the
    /// scheduler, its provenance made at `at`.
    Render {
        desc: RenderDesc,
        audio: Vec<f32>,
        channels: usize,
        frames: usize,
        rec: RecordingId,
        at: u64,
        gen: u32,
        progress: Arc<AtomicU32>,
    },
    /// Stop recording `rec`'s render on the scheduler.
    CancelRender(RecordingId),
    /// What a render left, to drop (and its file, if it asks).
    RenderLeftovers(Leftovers),
}

/// Why a copy won't come ([`FromWorker::CopyGone`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gone {
    /// No longer wanted: stopped on the scheduler.
    Cancelled,
    /// The scheduler's memory check said no.
    Refused,
    /// Its stretch couldn't be made.
    Failed,
}

#[allow(clippy::large_enum_variant)]
enum FromWorker {
    /// A copy's source buffer, made.
    CopyBuffer {
        key: crate::copies::CopyKey,
        audio: Vec<f32>,
        gen: u32,
    },
    /// A copy made.
    Copied {
        copy: Copy,
        gen: u32,
    },
    /// A copy that won't come, and why.
    CopyGone {
        key: crate::copies::CopyKey,
        why: Gone,
        gen: u32,
    },
    Room(Saved),
    /// For System status (the takes' files, the drive).
    Problem(Problem),
    Sized(Saved, Vec<FileEvent>),
    Buffer {
        audio: Vec<f32>,
        gen: u32,
    },
    Stretched {
        out: Option<(crate::save::TakeFile, Command)>,
        gen: u32,
    },
    Wave(Vec<(f32, f32)>),
    /// A buffer the writer has the frames of.
    Spare(Vec<f32>),
    /// A buffer a loop layer's file has the samples of (phase 4c.3).
    LayerSpare(Vec<f32>),
    /// A loop layer's file is whole in the work folder (None), or
    /// couldn't be written: why.
    LayerWritten {
        rec: RecordingId,
        why: Option<String>,
    },
    /// A loop layer's file read back (phase 4d.4): its audio,
    /// interleaved stereo, or why not.
    LayerRead {
        rec: RecordingId,
        audio: Result<Vec<f32>, String>,
    },
    Files(Said),
    /// A take started with no file: why.
    NoFile {
        rec: RecordingId,
        why: String,
    },
    /// Recording to files couldn't start: why.
    FilesFailed(String),
    /// Recording couldn't carry on in the folder asked, its card gone
    /// ([`ToWorker::CarryOn`]): why. The tracks record into memory only.
    CarryFailed(String),
    /// The stream layer's player, for the tracks.
    Player(Box<Player>),
    /// The stream layer couldn't start: why.
    StreamFailed(String),
    /// What came of opening a take's file (from the stream thread).
    Stream(FromStream),
    /// A waveform read from a take's file.
    WaveFile(TakeWave),
    /// What came of a [`ToWorker::WriteTake`]: the file's name and path,
    /// or why not with the audio back.
    TakeWritten {
        rec: RecordingId,
        gen: u32,
        out: Result<(String, PathBuf), (String, Vec<f32>)>,
    },
    /// A render's source buffer, made.
    RenderBuffer {
        audio: Vec<f32>,
        gen: u32,
    },
    /// Recording `rec`'s render is whole and durable as `name` at `path`.
    Rendered {
        rec: RecordingId,
        name: String,
        path: PathBuf,
        audio: Vec<f32>,
        desc: RenderDesc,
        prov: Provenance,
        gen: u32,
    },
    /// Recording `rec`'s render stopped without one, and why.
    RenderGone {
        rec: RecordingId,
        why: RenderOutcome,
        leftovers: Leftovers,
        gen: u32,
    },
}

enum Waving {
    /// Waiting for room for `columns` columns.
    Sizing { id: u16, columns: usize },
    /// Read from take `id`'s file on the worker.
    File { id: u16 },
    /// Column `at` is next.
    Reading { wave: TakeWave, at: usize },
}

enum Saving {
    /// Waiting for the worker's room.
    Asking,
    /// The room, until every take has settled (nothing records, loads or
    /// is still being written): then it is filled without allocating.
    Waiting(Saved),
    /// The filled room with the worker.
    Sizing,
    /// Reading take `i` of `saved.takes` from `frame`.
    Reading {
        saved: Saved,
        i: usize,
        frame: usize,
    },
}

enum Fitting {
    Sizing {
        take: TakeInfo,
        ratio: f64,
        bpm: f64,
    },
    Reading {
        take: TakeInfo,
        ratio: f64,
        bpm: f64,
        audio: Vec<f32>,
        frame: usize,
    },
    Stretching {
        take: TakeInfo,
    },
    /// The stretched take is copying in; then `swap` plays it.
    Loading {
        new: u16,
        swap: Command,
    },
}

/// A fitted copy made for a clip ([`AudioHost::fit_copy`]): recording
/// `to`, recording `from` stretched by `ratio` (its frames per one of
/// `from`'s) to the tempo asked for, keeping its pitch. The song still
/// plays `from`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitCopy {
    pub from: RecordingId,
    pub to: RecordingId,
    pub ratio: f64,
}

/// The audio tracks with their worker; see the module notes.
pub struct AudioHost {
    pub tracks: AudioTracks,
    /// Each drive's read and sync times ([`AudioHost::timings`]).
    timings: Board,
    /// The drive's last look (System status's stick meter).
    drive: Arc<DriveSeen>,
    tx: Option<Sender<ToWorker>>,
    rx: Option<Receiver<FromWorker>>,
    saving: Option<Saving>,
    saved: Option<Saved>,
    /// Saves cancelled while the worker sized them: their room is dropped
    /// when it comes.
    cancelled: u8,
    fitting: Option<Fitting>,
    waving: Option<Waving>,
    wave: Option<TakeWave>,
    files: FileSide,
    /// Fits swapped in, and ones that could not be made or loaded.
    pub fits: u64,
    pub fits_failed: u64,
    /// The fit under way makes a copy for a clip ([`AudioHost::fit_copy`]):
    /// loaded, it is said in `fitted`, not played in the song.
    fit_copy: bool,
    fitted: Option<FitCopy>,
    /// A sound brought in ([`AudioHost::import`]) copying in: its take,
    /// placed once it is in, and where it is cut (kept, not freed, here).
    importing: Option<u16>,
    import_cuts: Vec<u64>,
    /// The sound brought in goes in a clip, not on the song
    /// ([`AudioHost::import_for_clip`]): once in, it is said in
    /// `imported`, not placed.
    import_place: bool,
    imported: Option<u16>,
    /// Sounds brought in and placed.
    pub imports: u64,
    /// Which tracks a background job is for: one more each time they are
    /// emptied ([`AudioHost::clear`], a project opening or New). A fit's
    /// buffer or stretched take carries the generation it was asked for
    /// in, and one for tracks since emptied is dropped.
    generation: u32,
    /// Background results dropped because the tracks were emptied after
    /// they were asked for.
    pub stale: u64,
    /// Takes play from their files ([`AudioHost::stream`]).
    streaming: bool,
    /// A project's takes waiting for their files to open (room for every
    /// take, made once; a take here holds no audio, name or path), and
    /// sounds brought in or stretched copies whose files were written.
    opening: Vec<TakeFile>,
    /// Sounds brought in and stretched copies being written to files
    /// while takes play from theirs (no audio here: it went to the
    /// worker); once written they open as a project's takes do.
    to_write: Vec<TakeFile>,
    /// Room for the starts of the regions of takes on their files, to
    /// hold in the cache (made when streaming starts; away while the
    /// stream thread holds them), what they were last made from, and how
    /// many frames each is.
    heads: Option<Vec<Head>>,
    heads_from: u64,
    head_frames: u64,
    /// Region starts held in the cache, of those wanted, as the stream
    /// thread last said; the shortfall last said in the log; periods run,
    /// and the period the starts last went.
    pub heads_held: usize,
    pub heads_wanted: usize,
    heads_said: (usize, usize),
    periods: u64,
    heads_sent: u64,
    /// Where each audio clip's regions start in their recordings, to
    /// hold with the regions' starts ([`AudioHost::clip_starts`]).
    clip_starts: Vec<(RecordingId, u64)>,
    /// Answers about cues' audio not taken yet, oldest first; ones that
    /// found no room are counted.
    cue_answers: Vec<(u32, CueAudio)>,
    pub cue_answers_lost: u64,
    /// The transport's loop, as its blocks last said.
    loop_range: Option<(Tick, Tick)>,
    /// That project's tracks (its takes taken out), waiting until every
    /// take has come in or been left out: then they go in, without the
    /// regions of a take left out, so no region ever plays a take that
    /// won't come.
    waiting: Option<Saved>,
    /// The stream player's counters as System status last heard them.
    watch: Watch,
    /// Stretch's copies being made ([`crate::copying`]).
    copies: CopySide,
    /// Copies come back from the scheduler, taken in after this period's
    /// look if they are still wanted.
    arrived: Vec<Copy>,
    /// Reports for System status that found no room (taken too slowly).
    pub problems_lost: u64,
    /// The card last opened at each folder ([`AudioHost::own_device`]),
    /// and the one the takes' folder is on, if it is on one: once that one
    /// goes, takes in flight carry on where they are told to record next
    /// ([`AudioHost::record_to`]).
    cards: Vec<Arc<dyn CardHandle>>,
    work_card: Option<Arc<dyn CardHandle>>,
    /// The render under way and the last one's outcome (phase 6.0.3).
    render: RenderSide,
    /// Takes whose first part is on a card that went (they carried on,
    /// the SD card plan's B2c), by take id, with their lasting id, until
    /// the part is joined back (made at start: none allocates later).
    card_parts: Vec<Option<RecordingId>>,
    /// Recording couldn't carry on off the card that went
    /// ([`FromWorker::CarryFailed`]), until it moves again.
    carry_failed: bool,
}

/// Why [`AudioHost::import`] could not take a sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRefused {
    /// Another sound is still copying in.
    Busy,
    /// No take id is free.
    NoTake,
    /// Not enough audio track memory left, or the sound or track is not
    /// one the tracks take (no frames, 3 channels, a track past the last).
    NoRoom,
}

impl AudioHost {
    /// The tracks at `rate` with `seconds` of stereo memory (see
    /// [`AudioTracks::new`]), and the worker thread.
    pub fn new(rate: u32, seconds: u32) -> AudioHost {
        let (tx, jobs) = channel::<ToWorker>();
        let (done, rx) = channel();
        let back = tx.clone();
        let timings = Board::default();
        let board = timings.clone();
        let spawned = std::thread::Builder::new()
            .name("mpfree-audiotracks".into())
            .spawn(move || work(rate, &jobs, &done, &back, board))
            .is_ok();
        AudioHost {
            tracks: AudioTracks::new(rate, seconds),
            timings,
            drive: Arc::new(DriveSeen::default()),
            tx: spawned.then_some(tx),
            rx: spawned.then_some(rx),
            saving: None,
            saved: None,
            cancelled: 0,
            fitting: None,
            waving: None,
            wave: None,
            files: FileSide {
                // The buffers for copying takes out, made now so turning
                // files on later never allocates.
                spare: (0..SPARE).map(|_| vec![0.0; CHUNK]).collect(),
                outs: Vec::with_capacity(MAX_TAKES),
                names: [Name::default(); MAX_TRACKS],
                views: vec![None; MAX_TAKES],
                armed: [None; MAX_TRACKS],
                events: Vec::with_capacity(EVENTS),
                problems: Vec::with_capacity(PROBLEMS),
                layer_spare: (0..LAYER_SPARE).map(|_| vec![0.0; CHUNK]).collect(),
                layers_done: Vec::with_capacity(LAYERS_DONE),
                layers_read: Vec::with_capacity(LAYERS_DONE),
                layers_open: Vec::with_capacity(LAYERS_DONE),
            },
            watch: Watch::new(),
            copies: CopySide::new(),
            arrived: Vec::with_capacity(MAX_MAKING),
            problems_lost: 0,
            cards: Vec::new(),
            work_card: None,
            card_parts: vec![None; MAX_TAKES],
            carry_failed: false,
            fits: 0,
            fits_failed: 0,
            fit_copy: false,
            fitted: None,
            importing: None,
            import_cuts: Vec::new(),
            import_place: true,
            imported: None,
            imports: 0,
            generation: 0,
            stale: 0,
            streaming: false,
            opening: Vec::with_capacity(MAX_TAKES),
            to_write: Vec::with_capacity(TO_WRITE),
            waiting: None,
            heads: None,
            heads_from: 0,
            head_frames: 0,
            heads_held: 0,
            heads_wanted: 0,
            heads_said: (0, 0),
            periods: 0,
            heads_sent: 0,
            clip_starts: Vec::new(),
            cue_answers: Vec::with_capacity(CUE_ANSWERS),
            cue_answers_lost: 0,
            loop_range: None,
            render: RenderSide::new(),
        }
    }

    /// Play takes from their files from now on (see the module notes and
    /// [`crate::streaming`]): the worker starts the stream layer with
    /// `cfg`, and the tracks get its player a moment later. Call it once,
    /// at start, before [`AudioHost::record_to`], so the writer waits for
    /// playback's reads. False (nothing changed) once files are on, if it
    /// is on already, or with no worker. Makes the room for the regions'
    /// starts it holds in the cache, each the longest stall the plan
    /// expects of the drive (`cfg.plan.stall_s`) and half a second more:
    /// not on the audio thread.
    pub fn stream(&mut self, cfg: StreamConfig) -> bool {
        if self.tx.is_none() || self.streaming || self.tracks.records_to_files() {
            return false;
        }
        self.streaming = true;
        self.heads = Some(Vec::with_capacity(MAX_HEADS + MAX_CLIP_STARTS));
        let head_s = (cfg.plan.stall_s + HEAD_MARGIN_S).max(MIN_HEAD_S);
        self.head_frames = (head_s * f64::from(self.tracks.rate())) as u64;
        self.send(ToWorker::Stream(Box::new(cfg)));
        true
    }

    /// How long each drive's reads and the recording's syncs take (the
    /// median, the slowest 1 in 100 and 1 in 1,000, the worst), for Dev
    /// and the device rounds' logs (the SD card plan's B2a): the reads
    /// once takes play from their files, the syncs while the writer runs.
    /// Allocates: never in a period.
    pub fn timings(&self) -> Timings {
        self.timings.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// Whether takes play from their files (asked for: the player may be
    /// on its way).
    pub fn streams(&self) -> bool {
        self.streaming
    }

    /// Hold what a cue that launches audio needs (the time engine's
    /// `AudioNeed`s: frames `from..to` of each recording), under the
    /// host's number `cue`, in place of anything held for it before; the
    /// answer comes from [`AudioHost::cue_audio`]. What is in memory needs
    /// nothing; frames of takes on their files go to the stream thread,
    /// which answers once they are all in the cache. A recording not here
    /// (still loading) is [`CueAudio::Short`].
    pub fn hold_cue(&mut self, cue: u32, needs: &[(RecordingId, i64, i64)]) {
        let mut parts = Vec::new();
        for &(rec, from, to) in needs {
            let Some(take) = self.tracks.take_of(rec) else {
                self.cue_said(cue, CueAudio::Short);
                return;
            };
            let frames = self.tracks.take(take).map_or(0, |k| k.frames) as i64;
            // Frames on the file only: all of a take there, the start of a
            // take partly there.
            let on_file = if self.streaming {
                self.tracks.file_frames(take).unwrap_or(0) as i64
            } else {
                0
            };
            let (from, to) = (from.clamp(0, frames), to.clamp(0, frames).min(on_file));
            if from < to {
                parts.push((take, from as u64, to as u64));
            }
        }
        if parts.is_empty() {
            self.cue_said(cue, CueAudio::Held);
        } else {
            self.send(ToWorker::Cue { cue, parts });
        }
    }

    /// Cue `cue` landed or was cancelled: what was held for it goes.
    pub fn let_cue_go(&mut self, cue: u32) {
        if self.streaming {
            self.send(ToWorker::LetCueGo { cue });
        }
    }

    /// The next answer about a cue's audio, oldest first.
    pub fn cue_audio(&mut self) -> Option<(u32, CueAudio)> {
        (!self.cue_answers.is_empty()).then(|| self.cue_answers.remove(0))
    }

    fn cue_said(&mut self, cue: u32, a: CueAudio) {
        if self.cue_answers.len() < CUE_ANSWERS {
            self.cue_answers.push((cue, a));
        } else {
            self.cue_answers_lost += 1;
        }
    }

    /// Where each audio clip's regions start in their recordings: held in
    /// the cache with the song regions' starts while takes stream, so a
    /// clip launched with the transport stopped starts from memory at
    /// PLAY, and a cue's audio is mostly held already.
    /// The first [`MAX_CLIP_STARTS`] count.
    pub fn clip_starts(&mut self, mut starts: Vec<(RecordingId, u64)>) {
        starts.truncate(MAX_CLIP_STARTS);
        self.clip_starts = starts;
    }

    /// Takes still on their way: a project's waiting for their files to
    /// open, and sounds brought in or stretched being written to theirs.
    pub fn opening(&self) -> usize {
        self.opening.len() + self.to_write.len()
    }

    /// Whether a take made off the audio thread is written to a file and
    /// played from it: takes play from their files, the player is here,
    /// takes record to files (so there is a folder), and there is room.
    fn writes_files(&self) -> bool {
        self.streaming
            && self.tracks.player().is_some()
            && self.tracks.records_to_files()
            && self.to_write.len() < self.to_write.capacity()
            && self.opening.len() < self.opening.capacity()
    }

    /// Whether take `id` is on its way (being written, or opening).
    fn coming(&self, id: u16) -> bool {
        self.to_write
            .iter()
            .chain(&self.opening)
            .any(|k| k.id == id)
    }

    /// Send `file` (whose audio is in `audio`) to be written to a file of
    /// its own under its lasting id (a new one if it has none or another
    /// take has it), its take id kept for it meanwhile.
    fn write_take(&mut self, mut file: TakeFile, audio: Vec<f32>) {
        if !file.rec.is_given() || self.tracks.take_of(file.rec).is_some() {
            file.rec = self.tracks.fresh_rec();
        }
        self.tracks.reserve(file.id, true);
        self.send(ToWorker::WriteTake {
            rec: file.rec,
            channels: file.channels,
            audio,
            gen: self.generation,
        });
        self.to_write.push(file);
    }

    /// The SD card opening `mark` names is in again (the same card as its
    /// marker and volume serial say): each take that carried on on the
    /// stick when it went gets its first part back from it, into its file
    /// on the stick, nothing written to the card (the SD card plan's B2c).
    /// The part is looked for where the take's hole record says and in
    /// each of `places`; each take that comes back whole says so
    /// ([`FileEvent::Joined`]) and plays from its whole file. Sends a
    /// message (it carries `places`).
    pub fn join_card(&self, mark: CardMark, places: Vec<PathBuf>) {
        self.send(ToWorker::Join { mark, places });
    }

    /// Record every take to a file of its own in `dir` (made if missing;
    /// the library's work folder for the audio tracks), from the next
    /// armed track on (see [`crate::files`]), after anything a cut-off run
    /// left there is recovered ([`crate::files::recover_into`]). At start,
    /// and again to move to another folder (the SD card's, when it opens):
    /// an armed track's files made for its next take go, and are made
    /// again in `dir`. Takes already saved stay where they are. Does not
    /// allocate (the message carries `dir`). Refused, with `dir` given back
    /// to try again later, while a track records, a take is still being
    /// written (to the folder before: copied out, or Recording or Saving
    /// as the writer last said) or a loop layer's file is, while a save
    /// runs, or with no worker.
    ///
    /// But once the SD card the takes record on has gone (taken out, or
    /// closed: its opening revoked, the SD card plan's B2c), nothing waits:
    /// every take in flight there, recording or not, carries on in `dir`
    /// with the same writer (`cfg` is not used), what it committed on the
    /// card left there ([`crate::files::Files::carry_on`]), and plays from
    /// its file in `dir` once it is there ([`FileEvent::CarriedOn`]).
    pub fn record_to(&mut self, dir: PathBuf, cfg: WriterConfig) -> Result<(), PathBuf> {
        if self.tx.is_none() {
            return Err(dir);
        }
        let gone = self.work_card.as_ref().is_some_and(|c| !c.valid());
        if self.tracks.records_to_files() && gone {
            self.work_card = self.card_for(&dir);
            self.send(ToWorker::CarryOn(dir));
            return Ok(());
        }
        if self.tracks.records_to_files() {
            // Another folder: nothing may be in flight in the one before,
            // as its writer closes. The armed tracks' files go first
            // (their Disarm reaches the worker before the new folder) and
            // are made again there below.
            if !self.files.layers_open.is_empty()
                || self.saving.is_some()
                || !self.files_finished()
                || !self.tracks.files(false)
            {
                return Err(dir);
            }
            self.send_asks();
        }
        if !self.tracks.files(true) {
            return Err(dir);
        }
        self.work_card = self.card_for(&dir);
        self.carry_failed = false;
        self.send(ToWorker::Files { dir, cfg });
        Ok(())
    }

    /// The card opening that holds `dir`, if one does and still holds.
    /// Does not allocate.
    fn card_for(&self, dir: &Path) -> Option<Arc<dyn CardHandle>> {
        self.cards
            .iter()
            .find(|c| c.valid() && dir.starts_with(c.root()))
            .cloned()
    }

    /// Track `track`'s name for its next takes' files (see
    /// [`crate::files::Name`]). Does not allocate.
    pub fn set_name(&mut self, track: usize, name: &str) {
        if let Some(n) = self.files.names.get_mut(track) {
            if n.as_str() != name {
                *n = Name::new(name);
            }
        }
    }

    /// Take `id`'s file as the writer last said (None: no file: recorded
    /// with files off, brought in, or its files couldn't be made).
    pub fn file(&self, id: u16) -> Option<FileView> {
        let (rec, v) = (*self.files.views.get(id as usize)?)?;
        (self.tracks.take(id)?.rec == rec).then_some(v)
    }

    /// The tracks whose takes are being written on the SD card opening at
    /// `root` (recording, being finished, or at risk there): what an Eject
    /// carries on on the stick (the SD card plan's B2c). Allocates.
    pub fn recording_on(&self, root: &Path) -> Vec<usize> {
        let on = self
            .work_card
            .as_ref()
            .is_some_and(|c| c.valid() && c.root() == root);
        let mut out = Vec::new();
        if !on {
            return out;
        }
        for k in self.tracks.takes() {
            let writing = self
                .file(k.id)
                .is_some_and(|f| f.state != files::TakeState::Saved);
            if writing && !out.contains(&k.track) {
                out.push(k.track);
            }
        }
        out
    }

    /// How many tracks are armed to record to files (their next take's
    /// files made, or being made): for System status's recording room.
    /// Does not allocate.
    pub fn armed_files(&self) -> usize {
        self.files.armed.iter().flatten().count()
    }

    /// Whether track `track`'s next take's files are made: Some(false)
    /// while they are being made, None when it isn't armed to record to a
    /// file.
    pub fn armed_file(&self, track: usize) -> Option<bool> {
        let (rec, ready) = (*self.files.armed.get(track)?)?;
        (self.tracks.next_recording(track).map(|(r, _)| r) == Some(rec)).then_some(ready)
    }

    /// The next thing about the files for the log, oldest first.
    pub fn take_event(&mut self) -> Option<FileEvent> {
        if self.files.events.is_empty() {
            None
        } else {
            Some(self.files.events.remove(0))
        }
    }

    /// Loop layers' files (phase 4c.3, docs/dev/design/loops-on-audio-tracks.md
    /// rule 4): whether a closed layer can be written now. Takes record to
    /// files, so there is a work folder.
    pub fn layer_files(&self) -> bool {
        self.tx.is_some() && self.tracks.records_to_files()
    }

    /// Start writing a loop's closed layer as recording `rec`'s file in
    /// the work folder: `frames` frames of `channels`, given a piece at a
    /// time ([`AudioHost::layer_piece`]), then ended
    /// ([`AudioHost::layer_end`]). False if it can't be written now
    /// ([`AudioHost::layer_files`]). Does not allocate.
    pub fn layer_begin(&mut self, rec: RecordingId, frames: usize, channels: usize) -> bool {
        if !self.layer_files() || self.files.layers_open.len() >= LAYERS_DONE {
            return false;
        }
        // Begun again, the worker replaces the write: one answer comes.
        if !self.files.layers_open.contains(&rec) {
            self.files.layers_open.push(rec);
        }
        self.send(ToWorker::LayerBegin {
            rec,
            frames,
            channels,
        });
        true
    }

    /// A buffer of [`CHUNK`] samples for a layer's next piece, if one is
    /// back from the worker (None: wait, the worker is behind).
    pub fn layer_buffer(&mut self) -> Option<Vec<f32>> {
        self.files.layer_spare.pop()
    }

    /// The first `len` samples of `buf` (from [`AudioHost::layer_buffer`])
    /// are recording `rec`'s next piece.
    pub fn layer_piece(&mut self, rec: RecordingId, buf: Vec<f32>, len: usize) {
        if self.tx.is_some() {
            self.send(ToWorker::LayerPiece { rec, buf, len });
        } else {
            self.files.layer_spare.push(buf);
        }
    }

    /// Every piece of recording `rec`'s file has been given: it is synced
    /// and named, and [`AudioHost::layer_written`] says so.
    pub fn layer_end(&mut self, rec: RecordingId) {
        self.send(ToWorker::LayerEnd(rec));
    }

    /// Stop writing recording `rec`'s file (its layer went): nothing of it
    /// is kept.
    pub fn layer_drop(&mut self, rec: RecordingId) {
        self.layer_closed(rec);
        self.send(ToWorker::LayerDrop(rec));
    }

    fn layer_closed(&mut self, rec: RecordingId) {
        if let Some(i) = self.files.layers_open.iter().position(|r| *r == rec) {
            self.files.layers_open.swap_remove(i);
        }
    }

    /// What came of a loop layer's file, oldest first: whole in the work
    /// folder (None), or why not.
    pub fn layer_written(&mut self) -> Option<(RecordingId, Option<String>)> {
        if self.files.layers_done.is_empty() {
            None
        } else {
            Some(self.files.layers_done.remove(0))
        }
    }

    /// Read loop layer recording `rec`'s file back, as this run wrote it
    /// (phase 4d.4: UNDO brings the layer back from it): its audio comes
    /// back whole ([`AudioHost::layer_read_back`]). False with no worker.
    /// Allocates the message: between periods.
    pub fn layer_read(&mut self, rec: RecordingId) -> bool {
        if self.tx.is_none() {
            return false;
        }
        self.send(ToWorker::LayerRead(rec));
        true
    }

    /// A layer's file read back ([`AudioHost::layer_read`]), oldest first:
    /// its audio, interleaved stereo, or why not. Give the audio back
    /// with [`AudioHost::drop_audio`] once done with it.
    pub fn layer_read_back(&mut self) -> Option<(RecordingId, Result<Vec<f32>, String>)> {
        if self.files.layers_read.is_empty() {
            None
        } else {
            Some(self.files.layers_read.remove(0))
        }
    }

    /// A project save finished into `samples` (its `samples/` folder): the
    /// takes it has a file of are kept from there from now on, and their
    /// copies in the work folder go. Allocates the message: between
    /// periods.
    pub fn saved_into(&self, samples: PathBuf) {
        self.send(ToWorker::SavedInto(samples));
    }

    fn event(&mut self, e: FileEvent) {
        if self.files.events.len() < EVENTS {
            self.files.events.push(e);
        }
    }

    /// The next report for System status about the takes' files, the
    /// drive and streaming (phase 3b step 5d), oldest first: the engine
    /// track it is about, if any, for the app to name. Does not allocate.
    pub fn take_problem(&mut self) -> Option<Problem> {
        if self.files.problems.is_empty() {
            None
        } else {
            Some(self.files.problems.remove(0))
        }
    }

    fn problem(&mut self, p: Problem) {
        keep_problem(&mut self.files.problems, &mut self.problems_lost, p);
    }

    /// The tracks whose takes wait for their first part from a card that
    /// went (they carried on on the stick, the SD card plan's B2c), each
    /// once, in track order: what System status lists until the same card
    /// is back. Allocates.
    pub fn card_parts(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .card_parts
            .iter()
            .enumerate()
            .filter_map(|(id, p)| {
                let rec = (*p)?;
                let t = self.tracks.take(id as u16)?;
                (t.rec == rec).then_some(t.track)
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Card out for System status, from the takes still waiting for
    /// their first part from a card that went (`Watch::card_out`): red
    /// when recording couldn't carry on, or a take that carried on
    /// stopped by itself. A take gone since (erased, or the project
    /// closed) waits no more. Does not allocate.
    fn look_card_out(&mut self) {
        let mut parts = 0;
        let mut stopped = self.carry_failed;
        for id in 0..self.card_parts.len() {
            let Some(rec) = self.card_parts[id] else {
                continue;
            };
            if self.tracks.take(id as u16).is_none_or(|t| t.rec != rec) {
                self.card_parts[id] = None;
                continue;
            }
            parts += 1;
            stopped |= self
                .file(id as u16)
                .is_some_and(|f| matches!(f.state, files::TakeState::Stopped(_)));
        }
        let (room, lost) = (&mut self.files.problems, &mut self.problems_lost);
        self.watch.card_out(parts, stopped, &mut |r| {
            keep_problem(room, lost, Problem::of(r))
        });
    }

    /// The SD card opened, its files under `card.root()` (its mount
    /// point): a drive of its own, so takes there play from an I/O thread
    /// of their own and the recording writer there waits only for that
    /// drive's reads, never for the stick's (the SD card plan's S2b, phase
    /// B2); and an insertion (phase B2a), so takes there are read only
    /// while `card` holds, and those that went with an earlier opening
    /// open again. The same opening given again changes nothing.
    /// Allocates the message: between periods.
    pub fn own_device(&mut self, card: Arc<dyn CardHandle>) {
        self.cards.retain(|c| c.root() != card.root());
        self.cards.push(Arc::clone(&card));
        self.send(ToWorker::Device(card));
    }

    /// Look at the drive takes record to with `probe` every few seconds,
    /// on a thread of its own, for System status (missing, nearly full),
    /// once takes record to files ([`AudioHost::record_to`]). Allocates
    /// the message: between periods.
    pub fn watch_drive(&self, probe: DriveProbe) {
        self.send(ToWorker::WatchDrive(probe, Arc::clone(&self.drive)));
    }

    /// The drive's last look, as [`AudioHost::watch_drive`]'s thread keeps
    /// it (System status's stick meter): nothing until it starts.
    pub fn drive_seen(&self) -> Arc<DriveSeen> {
        Arc::clone(&self.drive)
    }

    fn send(&self, m: ToWorker) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
    }

    pub fn command(&mut self, c: Command) -> bool {
        // It may edit a region on Stretch: what is wanted is looked at
        // again once it has run, and a copy under way for the old region
        // stops.
        self.copies.look_soon();
        self.tracks.command(c)
    }

    // ---- Stretch's copies (phase 5b) -------------------------------------

    /// The audio clips with regions on Stretch, for their copies to be
    /// made ([`crate::copying`]): the app lists them as it lists their
    /// starts. The old list goes to the worker to drop.
    pub fn clip_copies(&mut self, clips: Vec<ClipWant>) {
        let old = self.copies.set_clips(clips);
        if old.capacity() > 0 {
            self.send(ToWorker::DropClips(old));
        }
    }

    /// The audio clips launched and waiting for their copies (5b.4), at
    /// their priority (Urgent) and the tempo at their line: asked for
    /// before anything else, at the next steady period. The old list goes
    /// to the worker to drop.
    pub fn launch_copies(&mut self, clips: Vec<ClipWant>) {
        let old = self.copies.set_launches(clips);
        if old.capacity() > 0 {
            self.send(ToWorker::DropClips(old));
        }
    }

    /// Clear copies (System status's Memory, 5c.4): lets every copy go
    /// that isn't playing or about to play ([`AudioHost::copy_soon`]). No
    /// recording is touched, it is no undo step, and each is made again
    /// when it is next wanted. How many went. Real-time safe.
    pub fn clear_copies(&mut self) -> usize {
        let copies = &self.copies;
        let n = self.tracks.clear_copies_but(&|k| copies.soon(k));
        self.copies.cleared();
        while let Some(d) = self.tracks.take_dropped_copy() {
            self.send(ToWorker::DropCopy(d));
        }
        self.copies.report(&self.tracks);
        n
    }

    /// What the copies have done.
    pub fn copy_counts(&self) -> CopyCounts {
        self.copies.counts
    }

    /// Whether the copy with `key` is about to play: wanted now for a
    /// launch waiting for it, a region drifting on Keep pitch, or a song
    /// region playing or within 2 bars. Clear copies keeps it.
    pub fn copy_soon(&self, key: &crate::copies::CopyKey) -> bool {
        self.copies.soon(key)
    }

    /// How many of the copies held a region plays now (System status's
    /// "12 copies · 3 playing", 5c.4).
    pub fn copies_playing(&self) -> usize {
        self.tracks.copies().filter(|c| c.playing).count()
    }

    /// The recordings of the copies being read or made, one call each
    /// (System status names whose they are).
    pub fn copies_making(&self, f: &mut dyn FnMut(RecordingId)) {
        self.copies.making_recs(f);
    }

    /// Memory the copies hold, how many there are, and how many are being
    /// read or made.
    pub fn copy_memory(&self) -> (usize, usize, usize) {
        let (bytes, n) = self.tracks.copy_bytes();
        (bytes, n, self.copies.busy())
    }

    /// How far each copy being made has got (0 to 1), for the app to see
    /// when the clip cards' progress moves. Doesn't allocate.
    pub fn copy_progress(&self, each: &mut dyn FnMut(f32)) {
        self.copies.progress(each);
    }

    /// Where clip `c`'s tempo copies for a steady `bpm` stand, over its
    /// regions on Stretch (5c: the clip card's foot, the cells' words):
    /// [`CopyState::None`] if it needs none.
    pub fn clip_copy_state(&self, c: &crate::clips::ClipPlay, bpm: f64) -> CopyState {
        let mut st = CopyState::None;
        self.tracks.clip_copy_keys(c, bpm, &mut |_, k| {
            st = st.with(self.copies.state(&self.tracks, &k));
        });
        st
    }

    /// Where song region `r`'s tempo copy along `map` stands (5c.3: its
    /// Tempo card's line): [`CopyState::None`] when it needs none.
    pub fn song_copy_state(&self, r: &crate::Region, map: &TempoMap) -> CopyState {
        self.tracks
            .song_copy_key(r, map)
            .map_or(CopyState::None, |k| self.copies.state(&self.tracks, &k))
    }

    /// Makes copies on `s` from now on, within its budget, not on the
    /// process's scheduler (`Sched::shared`): for a test that steps its
    /// own.
    pub fn copies_on(&mut self, s: Sched) {
        self.copies.budget = s.budget();
        self.copies.report(&self.tracks);
        self.send(ToWorker::Sched(s));
    }

    /// The copies' period: dropped ones to the worker, then what is
    /// wanted, room and the source being read ([`CopySide::tend`]).
    fn tend_copies(&mut self, block: &Block, map: &TempoMap) {
        while let Some(d) = self.tracks.take_dropped_copy() {
            self.send(ToWorker::DropCopy(d));
        }
        let Some(sp) = block.spans().last() else {
            return;
        };
        let now = sp.tick_at(sp.frame + u64::from(sp.len));
        let tx = self.tx.clone();
        let send = |m: ToWorker| {
            if let Some(tx) = &tx {
                let _ = tx.send(m);
            }
        };
        let gen = self.generation;
        self.copies.tend(
            &mut self.tracks,
            map,
            now,
            block.frames as usize,
            gen,
            &mut |a| send(ToWorker::Copy(a)),
            &mut |a| send(ToWorker::Drop(a)),
        );
        drop(tx);
        while let Some(copy) = self.arrived.pop() {
            if self.copies.made(&copy.key, &mut self.tracks, copy.bytes()) {
                match self.tracks.put_copy(copy) {
                    Ok(()) => self.copies.counts.made += 1,
                    Err(c) => {
                        self.copies.counts.no_room += 1;
                        self.send(ToWorker::DropCopy(c.data));
                    }
                }
            } else {
                self.send(ToWorker::DropCopy(copy.data));
            }
        }
        while let Some(d) = self.tracks.take_dropped_copy() {
            self.send(ToWorker::DropCopy(d));
        }
        // What they use now, held and being made, for the one budget.
        self.copies.report(&self.tracks);
    }

    /// Run the tracks for one period (see [`AudioTracks::process`]), then
    /// look after the save, the fit and the buffers.
    pub fn period(&mut self, block: &Block, map: &TempoMap, input: &[f32], out: &mut [f32]) {
        self.period_with(block, map, input, out, &[]);
    }

    /// [`AudioHost::period`] with what each track plays from the
    /// arranger's player ([`AudioTracks::process_with`]).
    pub fn period_with(
        &mut self,
        block: &Block,
        map: &TempoMap,
        input: &[f32],
        out: &mut [f32],
        answers: &[crate::Answer],
    ) {
        self.tracks.process_with(block, map, input, out, answers);
        self.after_period(block, map);
    }

    /// [`AudioHost::period_with`] with the tracks' loops (phase 4b:
    /// [`AudioTracks::process_with_loops`]).
    #[allow(clippy::too_many_arguments)]
    pub fn period_with_loops<'s>(
        &mut self,
        block: &Block,
        map: &TempoMap,
        input: &[f32],
        out: &mut [f32],
        answers: &[crate::Answer],
        loops: &crate::LoopPlayer,
        src: &dyn Fn(u128) -> Option<&'s [f32]>,
    ) {
        self.tracks
            .process_with_loops(block, map, input, out, answers, loops, src);
        self.after_period(block, map);
    }

    /// After the tracks' period: the save, the fit, the buffers, the
    /// player's problems, the loop and the heads.
    fn after_period(&mut self, block: &Block, map: &TempoMap) {
        self.tend_import(map);
        self.tend(block.frames as usize);
        self.tend_copies(block, map);
        self.periods += 1;
        if self.periods.is_multiple_of(PLAYER_LOOK_EVERY) {
            self.look_card_out();
            if let Some(st) = self.tracks.player_stats() {
                let (room, lost) = (&mut self.files.problems, &mut self.problems_lost);
                self.watch
                    .player(&st, &mut |r| keep_problem(room, lost, Problem::of(r)));
            }
        }
        for m in block.markers() {
            if let Mark::LoopSet(range) = m.mark {
                self.loop_range = range;
            }
        }
        if self.streaming && self.periods.is_multiple_of(HEADS_EVERY) {
            if let Some(sp) = block.spans().last() {
                self.tend_heads(sp.tick_at(sp.frame + u64::from(sp.len)) as Tick, map);
            }
        }
    }

    /// Every region of a take on its file has its start held in the cache
    /// (the stream thread's [`ToStream::Heads`]), and so has each track's
    /// frame at the loop's start, so a jump, a loop going round or a region
    /// after a gap never waits on the drive: once the regions, the takes on
    /// their files, the loop, the tempo there or the playhead's four bars
    /// change, the starts go to the stream thread, the ones ahead of the
    /// playhead first, nearest first (the loop's start by when the loop
    /// goes round), then the ones behind it. If the cache can't hold them
    /// all, the last are left out, the log says how many, and they go
    /// again every few seconds in case room came back. Does not allocate.
    fn tend_heads(&mut self, now: Tick, map: &TempoMap) {
        let Some(mut heads) = self.heads.take() else {
            // The stream thread has the room.
            return;
        };
        let tr = &self.tracks;
        let mix = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0100_0000_01b3);
        let mut from = mix(0xcbf2_9ce4_8422_2325, now.div_euclid(HEADS_SPAN) as u64);
        for t in 0..MAX_TRACKS {
            for r in tr.regions(t).iter().filter(|r| plays_from_file(tr, r.take)) {
                for v in [t as u64, r.start as u64, r.src as u64, u64::from(r.take)] {
                    from = mix(from, v);
                }
            }
            if let Some((take, at)) = self.loop_head(t, map) {
                from = mix(mix(from, u64::from(take)), at);
            }
        }
        for &(rec, at) in &self.clip_starts {
            if let Some(take) = tr.take_of(rec).filter(|&k| plays_from_file(tr, k)) {
                from = mix(mix(from, u64::from(take)), at);
            }
        }
        let retry =
            self.heads_held < self.heads_wanted && self.periods - self.heads_sent >= HEADS_RETRY;
        if from == self.heads_from && !retry {
            self.heads = Some(heads);
            return;
        }
        self.heads_from = from;
        self.heads_sent = self.periods;
        heads.clear();
        // The soonest first: ahead of the playhead by how far, then behind
        // it by how long ago.
        let order = |at: Tick, ahead: bool| {
            if ahead {
                (at - now).max(0)
            } else {
                Tick::MAX / 2 + (now - at).max(0)
            }
        };
        let head = |take: u16, at: u64, order: Tick| {
            let frames = tr.take(take)?.frames as u64;
            (at < frames).then(|| Head {
                order,
                take,
                from: at,
                to: (at + self.head_frames).min(frames),
            })
        };
        for t in 0..MAX_TRACKS {
            for r in tr.regions(t).iter().filter(|r| plays_from_file(tr, r.take)) {
                if let Some(h) = head(r.take, r.src.max(0) as u64, order(r.start, r.end > now)) {
                    heads.push(h);
                }
            }
            if let (Some((take, at)), Some((_, end))) = (self.loop_head(t, map), self.loop_range) {
                if let Some(h) = head(take, at, order(end, end > now)) {
                    heads.push(h);
                }
            }
        }
        // A clip may launch at any time: after what plays ahead, before
        // what played.
        for &(rec, at) in &self.clip_starts {
            if let Some(take) = tr.take_of(rec).filter(|&k| plays_from_file(tr, k)) {
                if let Some(h) = head(take, at, Tick::MAX / 4) {
                    heads.push(h);
                }
            }
        }
        heads.sort_unstable();
        self.heads_wanted = heads.len();
        self.send(ToWorker::Heads(heads));
    }

    /// Track `t`'s take and frame at the loop's start, if a region of a
    /// take on its file plays there other than from its own start (which
    /// is held anyway).
    fn loop_head(&self, t: usize, map: &TempoMap) -> Option<(u16, u64)> {
        let (start, _) = self.loop_range?;
        let tr = &self.tracks;
        let r = tr
            .regions(t)
            .iter()
            .find(|r| r.contains(start) && r.start != start && plays_from_file(tr, r.take))?;
        let rate = self.tracks.rate();
        let at = r.src + r.src_frames_to(map, r.start as f64, start as f64, rate);
        Some((r.take, at.max(0) as u64))
    }

    /// Start saving the audio tracks; the result comes in
    /// [`AudioHost::take_saved`]. False if a save is already under way or
    /// there is no worker.
    pub fn save(&mut self) -> bool {
        if self.saving.is_some() || self.tx.is_none() {
            return false;
        }
        self.send(ToWorker::Room);
        self.saving = Some(Saving::Asking);
        true
    }

    /// How far the save has got (0 to 1), while one is under way.
    pub fn saving(&self) -> Option<f32> {
        match &self.saving {
            None => None,
            Some(Saving::Asking | Saving::Waiting(_) | Saving::Sizing) => Some(0.0),
            Some(Saving::Reading { saved, i, .. }) => {
                Some(*i as f32 / saved.takes.len().max(1) as f32)
            }
        }
    }

    /// A finished save, with every take's audio.
    pub fn take_saved(&mut self) -> Option<Saved> {
        self.saved.take()
    }

    /// Stop the save under way, and drop one finished but not taken (a
    /// project opening over the one being saved). What it had read goes
    /// to the worker to drop. Does not allocate.
    pub fn cancel_save(&mut self) {
        match self.saving.take() {
            Some(Saving::Reading { saved, .. } | Saving::Waiting(saved)) => {
                self.send(ToWorker::DropSaved(saved))
            }
            // The worker's answer is dropped when it comes.
            Some(Saving::Asking | Saving::Sizing) => {
                self.cancelled = self.cancelled.saturating_add(1)
            }
            None => {}
        }
        if let Some(old) = self.saved.take() {
            self.send(ToWorker::DropSaved(old));
        }
    }

    /// Put a saved part back: every take (copying in over the next
    /// periods) and every track. Takes or tracks that could not be put
    /// back are listed in the error; the rest are in. A take's file in the
    /// project (its `file`, and its `path` when the opener found it) goes
    /// to the worker, so a save names it again and copies it from there.
    /// Any audio not taken goes to the worker to drop.
    ///
    /// While takes play from their files, a take with a file (and a
    /// lasting id of its own) opens as its file instead, its audio not
    /// needed: it comes in once the file is open ([`AudioHost::opening`]),
    /// and one whose file can't be opened is left out, said in the log.
    /// The tracks then go in once every take has come in or been left
    /// out, without the regions of a take left out (a track that can't go
    /// in is listed in the error at once). Refused while another
    /// project's takes are still opening ([`AudioHost::clear`] first).
    pub fn load(&mut self, mut saved: Saved) -> Result<(), String> {
        // Only a refusal allocates (its line); what is left of the part
        // goes to the worker to drop.
        if self.waiting.is_some() {
            self.send(ToWorker::DropSaved(saved));
            return Err("audio tracks: another project is still opening".into());
        }
        let mut bad = Vec::new();
        for mut k in saved.takes.drain(..) {
            let (id, frames, channels) = (k.id, k.frames, k.channels);
            let name = std::mem::take(&mut k.file);
            let path = k.path.take();
            // Kept with its file's name on the worker (dropped there too).
            let prov = k.prov.take();
            let as_file = self.streaming
                && path.is_some()
                && k.rec.is_given()
                && self.tracks.take_of(k.rec).is_none()
                && !self.opening.iter().any(|o| o.rec == k.rec || o.id == k.id)
                && self.opening.len() < self.opening.capacity();
            if as_file {
                self.tracks.reserve(id, true);
                self.send(ToWorker::Drop(std::mem::take(&mut k.audio)));
                self.send(ToWorker::Known {
                    rec: k.rec,
                    take: id,
                    name,
                    path,
                    frames,
                    channels,
                });
                self.keep_prov(k.rec, prov);
                self.opening.push(k);
                continue;
            }
            if path.is_none() && !k.audio.is_empty() && self.writes_files() {
                // A take no file holds (converted from another rate):
                // written to one and played from it, as a sound brought
                // in is.
                let audio = std::mem::take(&mut k.audio);
                k.prov = prov;
                self.write_take(k, audio);
                continue;
            }
            match self.tracks.load_take(k) {
                Ok(()) => {
                    if let Some(t) = self.tracks.take(id) {
                        if path.is_some() {
                            self.files.views[id as usize] = Some((t.rec, FileView::SAVED));
                        }
                        self.send(ToWorker::Known {
                            rec: t.rec,
                            take: id,
                            name,
                            path,
                            frames,
                            channels,
                        });
                        let rec = t.rec;
                        self.keep_prov(rec, prov);
                    }
                }
                Err(k) => {
                    bad.push(format!("take {}", k.id));
                    self.send(ToWorker::Drop(k.audio));
                    // Its name and path are dropped on the worker too.
                    self.send(ToWorker::Known {
                        rec: RecordingId::NONE,
                        take: id,
                        name,
                        path,
                        frames: 0,
                        channels,
                    });
                    self.keep_prov(RecordingId::NONE, prov);
                }
            }
        }
        if self.opening.is_empty() && self.to_write.is_empty() {
            for t in &saved.tracks {
                if !self.tracks.load_track(t) {
                    bad.push(format!("track {}", t.index + 1));
                }
            }
            self.send(ToWorker::DropSaved(saved));
        } else {
            for t in &saved.tracks {
                if !AudioTracks::track_fits(t) {
                    bad.push(format!("track {}", t.index + 1));
                }
            }
            self.waiting = Some(saved);
        }
        if bad.is_empty() {
            Ok(())
        } else {
            Err(format!("audio tracks: could not load {}", bad.join(", ")))
        }
    }

    /// Bring a sound in (the one Sample page's step 6, Put on Audio
    /// track): `audio`, interleaved, `channels` (1 or 2) at the tracks'
    /// rate, becomes a take of its own on `track`, its frame 0 at tick
    /// `at`, in a group of its own. It copies in over the next periods, as
    /// a loaded take does, then plays from `at` to its end over what was
    /// there, a region for each piece between its frames `cuts` (its
    /// slices): one undo step ([`AudioTracks::place`]). `bpm` is its tempo
    /// (0: not known), for Fit to tempo. Gives the take's id, or why not;
    /// a refused sound's audio goes to the worker to drop, so this never
    /// frees audio memory here (the cuts before are dropped here: call it
    /// between periods).
    pub fn import(
        &mut self,
        track: usize,
        at: Tick,
        channels: usize,
        audio: Vec<f32>,
        cuts: Vec<u64>,
        bpm: f64,
    ) -> Result<u16, ImportRefused> {
        let refuse = |h: &mut AudioHost, audio: Vec<f32>, why| {
            h.send(ToWorker::Drop(audio));
            Err(why)
        };
        if self.importing.is_some() {
            return refuse(self, audio, ImportRefused::Busy);
        }
        self.import_place = true;
        let Some(id) = self.tracks.free_take_id() else {
            return refuse(self, audio, ImportRefused::NoTake);
        };
        let group = self
            .tracks
            .takes()
            .map(|t| t.group)
            .max()
            .map_or(0, |g| g + 1);
        let frames = audio.len().checked_div(channels).unwrap_or(0);
        let mut file = TakeFile {
            prov: None,
            id,
            // A sound brought in is a new recording: the tracks give its id.
            rec: crate::RecordingId::NONE,
            track,
            group,
            pass: 0,
            bpm,
            varied: false,
            anchor: at as f64,
            frames,
            channels,
            file: String::new(),
            path: None,
            audio,
        };
        if self.writes_files() {
            if track >= MAX_TRACKS
                || !(1..=2).contains(&channels)
                || frames == 0
                || file.audio.len() != frames * channels
                || !file.anchor.is_finite()
            {
                return refuse(self, file.audio, ImportRefused::NoRoom);
            }
            // Written to its file and played from it: no memory taken.
            let audio = std::mem::take(&mut file.audio);
            self.write_take(file, audio);
            self.importing = Some(id);
            self.import_cuts = cuts;
            return Ok(id);
        }
        match self.tracks.load_take(file) {
            Ok(()) => {
                self.importing = Some(id);
                self.import_cuts = cuts;
                Ok(id)
            }
            Err(f) => refuse(self, f.audio, ImportRefused::NoRoom),
        }
    }

    /// Put in a cell (audio tracks phase 2): a sound brought in as for
    /// [`AudioHost::import`], uncut, as a take of its own on `track` that
    /// the song doesn't play: once it is in (copied in, or written to its
    /// file and open), [`AudioHost::imported`] gives it, for a clip to
    /// name. Refused as for [`AudioHost::import`].
    pub fn import_for_clip(
        &mut self,
        track: usize,
        channels: usize,
        audio: Vec<f32>,
        bpm: f64,
    ) -> Result<u16, ImportRefused> {
        let id = self.import(track, 0, channels, audio, Vec::new(), bpm)?;
        self.import_place = false;
        Ok(id)
    }

    /// The take of a sound brought in for a clip that has come in since
    /// the last ask ([`AudioHost::import_for_clip`]).
    pub fn imported(&mut self) -> Option<u16> {
        self.imported.take()
    }

    /// The take of a sound brought in that is still copying in.
    pub fn importing(&self) -> Option<u16> {
        self.importing
    }

    /// A sound brought in that has copied in (or opened as its file)
    /// goes on its track.
    fn tend_import(&mut self, map: &TempoMap) {
        let Some(id) = self.importing else {
            return;
        };
        if self.tracks.take(id).is_none() {
            if !self.coming(id) {
                // Gone (the tracks were cleared, or its file didn't open).
                self.importing = None;
                self.import_cuts.clear();
            }
        } else if self.tracks.loading() == 0 {
            self.importing = None;
            if !self.import_place {
                // For a clip: said, not placed.
                self.imported = Some(id);
                self.imports += 1;
            } else if self.tracks.place(id, &self.import_cuts, map) {
                self.imports += 1;
            }
            self.import_cuts.clear();
        }
    }

    /// Fit take `id` to `bpm` (the tempo now), keeping its pitch. False if
    /// a fit is under way, the take is not there or records, its tempo is
    /// not known or already `bpm`, or there is no worker.
    pub fn fit(&mut self, id: u16, bpm: f64) -> bool {
        let from = self.tracks.take(id).map_or(0.0, |t| t.bpm);
        self.start_fit(id, from, bpm, false)
    }

    /// Fit to tempo on an audio clip's card (audio tracks phase 2): a
    /// fitted copy of take `id`, its audio made at `from` BPM stretched to
    /// `bpm`, keeping its pitch, as a take of its own that the song does
    /// not play (the song keeps the original). Once it is in (copied in,
    /// or written to its file and open) [`AudioHost::fitted_copy`] says
    /// so, and the clip is pointed at it. False as for
    /// [`AudioHost::fit`], or if `from` is not a tempo.
    pub fn fit_copy(&mut self, id: u16, from: f64, bpm: f64) -> bool {
        self.start_fit(id, from, bpm, true)
    }

    /// The fitted copy that has come in since the last ask, if any.
    pub fn fitted_copy(&mut self) -> Option<FitCopy> {
        self.fitted.take()
    }

    /// How far the fit under way has got, 0 to 1, roughly: reading the
    /// take is the first half, stretching it takes it to 0.9 (the worker
    /// says nothing until it is done), copying it in the rest.
    pub fn fit_progress(&self) -> Option<f32> {
        Some(match self.fitting.as_ref()? {
            Fitting::Sizing { .. } => 0.0,
            Fitting::Reading { take, frame, .. } => {
                0.5 * (*frame as f32 / take.frames.max(1) as f32).min(1.0)
            }
            Fitting::Stretching { .. } => 0.5,
            Fitting::Loading { .. } => 0.9,
        })
    }

    fn start_fit(&mut self, id: u16, from: f64, bpm: f64, copy: bool) -> bool {
        if self.fitting.is_some() || self.tx.is_none() {
            return false;
        }
        let Some(take) = self.tracks.take(id).filter(|t| !t.recording) else {
            return false;
        };
        let Some(ratio) = fit_ratio(from, bpm).filter(|r| (r - 1.0).abs() > 1e-9) else {
            return false;
        };
        self.fit_copy = copy;
        if self.plays_from_file(id) {
            self.fit_from_file(take, ratio, bpm);
            return true;
        }
        self.send(ToWorker::Buffer {
            n: take.frames * take.channels,
            gen: self.generation,
        });
        self.fitting = Some(Fitting::Sizing { take, ratio, bpm });
        true
    }

    /// Fit `take` from its file, read and stretched on the worker.
    fn fit_from_file(&mut self, take: TakeInfo, ratio: f64, bpm: f64) {
        self.send(ToWorker::FitFile {
            take,
            ratio,
            bpm,
            gen: self.generation,
        });
        self.fitting = Some(Fitting::Stretching { take });
    }

    /// Where a render made recording `rec` from goes to the worker, which
    /// keeps it for a save (dropped there when `rec` is none). Does not
    /// allocate: `prov` is moved.
    fn keep_prov(&self, rec: RecordingId, prov: Option<Provenance>) {
        if let Some(prov) = prov {
            self.send(ToWorker::Prov { rec, prov });
        }
    }

    /// Whether `take` is still there, the same recording, now on its file
    /// (it moved while it was read from memory).
    fn moved_to_file(&self, take: u16, rec: RecordingId) -> bool {
        self.plays_from_file(take) && self.tracks.take(take).is_some_and(|t| t.rec == rec)
    }

    /// Whether some of take `id` plays from its file (all of it, or the
    /// start of a long recording whose memory went there as it recorded):
    /// a fit or a waveform reads it from there, not memory.
    fn plays_from_file(&self, id: u16) -> bool {
        plays_from_file(&self.tracks, id)
    }

    /// The take being fitted, while a fit is under way.
    pub fn fitting(&self) -> Option<u16> {
        match self.fitting.as_ref()? {
            Fitting::Sizing { take, .. }
            | Fitting::Reading { take, .. }
            | Fitting::Stretching { take } => Some(take.id),
            Fitting::Loading { swap, .. } => match swap {
                Command::Swap { old, .. } => Some(*old),
                _ => None,
            },
        }
    }

    /// Start reading take `id`'s waveform into about `columns` columns
    /// (1 to [`MAX_COLUMNS`]; fewer for a short take, as a column covers
    /// whole blocks of the peak table). The result comes in
    /// [`AudioHost::take_wave`]. Asking again replaces a wave not yet
    /// done. False if the take is not there or there is no worker.
    pub fn wave(&mut self, id: u16, columns: usize) -> bool {
        if self.tx.is_none() || self.tracks.take(id).is_none() {
            return false;
        }
        if let Some(Waving::Reading { wave, .. }) = self.waving.take() {
            self.send(ToWorker::DropWave(wave));
        }
        let columns = columns.clamp(1, MAX_COLUMNS);
        match self.tracks.take(id).filter(|_| self.plays_from_file(id)) {
            Some(take) => self.wave_from_file(take, columns),
            None => {
                self.send(ToWorker::Wave(columns));
                self.waving = Some(Waving::Sizing { id, columns });
            }
        }
        true
    }

    /// Read `take`'s waveform from its file on the worker.
    fn wave_from_file(&mut self, take: TakeInfo, columns: usize) {
        self.send(ToWorker::WaveFile { take, columns });
        self.waving = Some(Waving::File { id: take.id });
    }

    /// A finished waveform.
    pub fn take_wave(&mut self) -> Option<TakeWave> {
        self.wave.take()
    }

    /// Give back a part not loaded (a project opened over another still
    /// waiting), to be dropped on the worker rather than here.
    pub fn drop_saved(&self, s: Saved) {
        self.send(ToWorker::DropSaved(s));
    }

    /// The tracks' generation: one more each time they are emptied
    /// ([`AudioHost::clear`], New project or a project opening). It is
    /// the revision of [`crate::render::Dep::Project`] (phase 6.0.2): a
    /// render begun before a project opened never holds after.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The revision of `dep` now, for those the audio tracks hold: a
    /// recording ([`AudioTracks::rec_rev`]), a track by its engine number
    /// (`slot` maps the project's lasting number to it:
    /// [`AudioTracks::track_rev`]) and the project ([`Self::generation`]).
    /// None for one gone, and for a clip, a loop or the routes (the
    /// app's). Allocates nothing.
    pub fn rev(&self, dep: crate::render::Dep, slot: impl Fn(u32) -> Option<usize>) -> Option<u64> {
        use crate::render::Dep;
        match dep {
            Dep::Recording(rec) => self.tracks.rec_rev(rec),
            Dep::Track(id) => self.tracks.track_rev(slot(id)?),
            Dep::Project => Some(u64::from(self.generation)),
            Dep::Clip(_) | Dep::Loop(_) | Dep::Routes => None,
        }
    }

    /// Every track as new and every take forgotten
    /// ([`AudioTracks::clear`]: refused, false, while a track records, a
    /// take copies in or a take is still being copied out to its file),
    /// for New project or a project opening. The background jobs asked
    /// for until now are the old project's: a fit under way stops, and
    /// whatever the worker still sends for it is dropped as it comes
    /// ([`AudioHost::stale`]), so it can never reach the new project's
    /// takes (a take id comes back after a clear). A sound still being
    /// brought in goes with the rest. The take files stay where they are.
    /// Does not allocate.
    pub fn clear(&mut self) -> bool {
        if !self.files.outs.is_empty() || !self.tracks.clear() {
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        if let Some(a) = self.copies.clear() {
            self.send(ToWorker::Drop(a));
        }
        self.send(ToWorker::Copy(Ask::CancelAll));
        while let Some(c) = self.arrived.pop() {
            self.copies.counts.stale += 1;
            self.send(ToWorker::DropCopy(c.data));
        }
        while let Some(d) = self.tracks.take_dropped_copy() {
            self.send(ToWorker::DropCopy(d));
        }
        self.copies.report(&self.tracks);
        self.clear_render();
        match self.fitting.take() {
            Some(Fitting::Reading { audio, .. }) => self.send(ToWorker::Drop(audio)),
            // Sizing and Stretching: the worker's answer carries the old
            // generation; Loading: its take went with the rest.
            Some(Fitting::Sizing { .. } | Fitting::Stretching { .. } | Fitting::Loading { .. })
            | None => {}
        }
        self.importing = None;
        self.files.views.fill(None);
        // Takes still waiting for their files hold nothing to free; their
        // tracks go to the worker.
        self.opening.clear();
        // Being written: their files stay in the work folder (moved into
        // Recordings at the next start), and their answers are dropped.
        self.to_write.clear();
        if let Some(w) = self.waiting.take() {
            self.send(ToWorker::DropSaved(w));
        }
        self.send(ToWorker::Clear {
            gen: self.generation,
        });
        if self.streaming {
            self.send(ToWorker::Project);
            // A new project's shortfall is news.
            self.heads_said = (0, 0);
        }
        true
    }

    /// Stop the waveform being read and drop one finished but not taken
    /// (the takes are going: a project opening). Does not allocate.
    pub fn cancel_wave(&mut self) {
        if let Some(Waving::Reading { wave, .. }) = self.waving.take() {
            self.send(ToWorker::DropWave(wave));
        }
        if let Some(w) = self.wave.take() {
            self.send(ToWorker::DropWave(w));
        }
    }

    /// Audio done with, dropped on the worker (not here, on the audio
    /// thread): a sound that could not be brought in, say.
    pub fn drop_audio(&self, audio: Vec<f32>) {
        self.send(ToWorker::Drop(audio));
    }

    /// Give back a waveform the screen is done with, to be dropped on the
    /// worker rather than here (the audio thread never frees). Without a
    /// worker it is dropped here.
    pub fn drop_wave(&self, w: TakeWave) {
        self.send(ToWorker::DropWave(w));
    }

    fn tend(&mut self, frames: usize) {
        while let Some(v) = self.tracks.take_returned() {
            self.send(ToWorker::Drop(v));
        }
        while let Some(m) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                FromWorker::CopyBuffer { audio, gen, .. } if gen != self.generation => {
                    self.send(ToWorker::Drop(audio));
                }
                FromWorker::CopyBuffer { key, audio, .. } => {
                    if let Err(a) = self.copies.buffer(key, audio) {
                        self.send(ToWorker::Drop(a));
                    }
                }
                FromWorker::Copied { copy, gen } if gen != self.generation => {
                    self.copies.counts.stale += 1;
                    self.send(ToWorker::DropCopy(copy.data));
                }
                FromWorker::Copied { copy, .. } => {
                    // Taken in once what is wanted has been looked at
                    // again (`tend_copies`): a region edited while it was
                    // made doesn't get it.
                    if self.arrived.len() < self.arrived.capacity() {
                        self.copies.look_soon();
                        self.arrived.push(copy);
                    } else {
                        self.copies.gone(&copy.key, false);
                        self.send(ToWorker::DropCopy(copy.data));
                    }
                }
                FromWorker::CopyGone { gen, .. } if gen != self.generation => {}
                FromWorker::CopyGone { key, why, .. } => {
                    match why {
                        Gone::Cancelled => {}
                        Gone::Refused => self.copies.counts.refused += 1,
                        Gone::Failed => self.copies.counts.failed += 1,
                    }
                    self.copies.gone(&key, why != Gone::Cancelled);
                }
                FromWorker::Room(saved) | FromWorker::Sized(saved, _) if self.cancelled > 0 => {
                    self.cancelled -= 1;
                    self.send(ToWorker::DropSaved(saved));
                }
                FromWorker::Room(room) => match self.saving.take() {
                    Some(Saving::Asking) => self.saving = Some(Saving::Waiting(room)),
                    other => {
                        self.saving = other;
                        self.send(ToWorker::DropSaved(room));
                    }
                },
                FromWorker::Sized(saved, said) => {
                    for e in said {
                        self.event(e);
                    }
                    self.saving = Some(Saving::Reading {
                        saved,
                        i: 0,
                        frame: 0,
                    });
                }
                FromWorker::Spare(buf) => self.files.spare.push(buf),
                FromWorker::LayerSpare(buf) => self.files.layer_spare.push(buf),
                FromWorker::LayerWritten { rec, why } => {
                    self.layer_closed(rec);
                    if self.files.layers_done.len() < LAYERS_DONE {
                        self.files.layers_done.push((rec, why));
                    }
                }
                FromWorker::LayerRead { rec, audio } => {
                    if self.files.layers_read.len() < LAYERS_DONE {
                        self.files.layers_read.push((rec, audio));
                    } else if let Ok(a) = audio {
                        self.drop_audio(a);
                    }
                }
                FromWorker::Files(said) => self.heard(said),
                FromWorker::Problem(p) => self.problem(p),
                FromWorker::NoFile { rec, why } => {
                    if let Some(t) = self.tracks.take_of(rec).and_then(|id| self.tracks.take(id)) {
                        self.event(FileEvent::NoFile {
                            track: t.track,
                            why,
                        });
                    }
                }
                FromWorker::Player(p) => {
                    if let Some(old) = self.tracks.stream(Some(p)) {
                        self.send(ToWorker::DropPlayer(old));
                    }
                }
                FromWorker::StreamFailed(why) => {
                    self.streaming = false;
                    // A project's takes waiting for their files won't
                    // open: left out, said, and its tracks go in.
                    while let Some(k) = self.opening.pop() {
                        self.tracks.reserve(k.id, false);
                        self.event(FileEvent::Unopened {
                            track: k.track,
                            why: why.clone(),
                            kept: false,
                        });
                    }
                    self.tracks_in();
                    self.event(FileEvent::NoStream { why });
                }
                FromWorker::Stream(o) => self.stream_said(o),
                FromWorker::TakeWritten { rec, gen, out } => self.take_written(rec, gen, out),
                FromWorker::RenderBuffer { audio, gen } => self.render_buffer(audio, gen),
                FromWorker::Rendered {
                    rec,
                    name,
                    path,
                    audio,
                    desc,
                    prov,
                    gen,
                } => self.rendered(rec, name, path, audio, desc, prov, gen),
                FromWorker::RenderGone {
                    rec,
                    why,
                    leftovers,
                    gen,
                } => self.render_gone(rec, why, leftovers, gen),
                FromWorker::WaveFile(w) => match self.waving {
                    Some(Waving::File { id }) if id == w.id => {
                        self.waving = None;
                        if let Some(old) = self.wave.replace(w) {
                            self.send(ToWorker::DropWave(old));
                        }
                    }
                    _ => self.send(ToWorker::DropWave(w)),
                },
                FromWorker::FilesFailed(why) => {
                    // The tracks go on recording into memory only.
                    self.tracks.files(false);
                    self.event(FileEvent::ArmFailed { track: 0, why });
                }
                FromWorker::CarryFailed(why) => {
                    self.tracks.files(false);
                    self.carry_failed = true;
                    self.event(FileEvent::ArmFailed { track: 0, why });
                    self.look_card_out();
                }
                // For tracks emptied since it was asked for (a project
                // opened): dropped, never let near the new project's takes.
                FromWorker::Buffer { audio, gen } if gen != self.generation => {
                    self.stale += 1;
                    self.send(ToWorker::Drop(audio));
                }
                FromWorker::Stretched { out, gen } if gen != self.generation => {
                    self.stale += 1;
                    if let Some((file, _)) = out {
                        self.send(ToWorker::Drop(file.audio));
                    }
                }
                FromWorker::Buffer { audio, .. } => match self.fitting.take() {
                    Some(Fitting::Sizing { take, ratio, bpm }) => {
                        self.fitting = Some(Fitting::Reading {
                            take,
                            ratio,
                            bpm,
                            audio,
                            frame: 0,
                        });
                    }
                    other => {
                        self.fitting = other;
                        self.send(ToWorker::Drop(audio));
                    }
                },
                FromWorker::Wave(columns) => match self.waving.take() {
                    Some(Waving::Sizing { id, columns: n }) if columns.capacity() >= n => {
                        match self.start_wave(id, n, columns) {
                            Some(w) => self.waving = Some(w),
                            None => self.waving = None,
                        }
                    }
                    other => {
                        // Room for a wave asked for before the last one.
                        self.waving = other;
                        self.send(ToWorker::DropWave(TakeWave {
                            id: 0,
                            frames: 0,
                            channels: 0,
                            frames_per_column: 0,
                            columns,
                        }));
                    }
                },
                FromWorker::Stretched { out, .. } => {
                    self.fitting = None;
                    let Some((mut file, swap)) = out else {
                        self.fits_failed += 1;
                        continue;
                    };
                    // A recording may have taken the id while it stretched.
                    let (Some(new), Command::Swap { old, ratio, .. }) =
                        (self.tracks.free_take_id(), swap)
                    else {
                        self.fits_failed += 1;
                        self.send(ToWorker::Drop(file.audio));
                        continue;
                    };
                    file.id = new;
                    let swap = Command::Swap { old, new, ratio };
                    if self.writes_files() {
                        let audio = std::mem::take(&mut file.audio);
                        self.write_take(file, audio);
                        self.fitting = Some(Fitting::Loading { new, swap });
                        continue;
                    }
                    match self.tracks.load_take(file) {
                        Ok(()) => self.fitting = Some(Fitting::Loading { new, swap }),
                        Err(f) => {
                            self.fits_failed += 1;
                            self.send(ToWorker::Drop(f.audio));
                        }
                    }
                }
            }
        }
        self.tend_files();
        let budget = READ_SPEED * frames.max(1);
        self.tend_save(budget);
        self.tend_fit(budget);
        self.tend_render(budget);
        self.tend_wave(WAVE_SPEED * frames.max(1));
    }

    /// The log hears when the cache holds fewer of the regions' starts
    /// than wanted: once for each number wanted, and again only if fewer
    /// still are held (so a cache that keeps giving room back and taking
    /// it again doesn't fill the log).
    fn heads_short(&mut self) {
        let (held, of) = (self.heads_held, self.heads_wanted);
        let (said_held, said_of) = self.heads_said;
        if held < of && (of != said_of || held < said_held) {
            self.heads_said = (held, of);
            self.event(FileEvent::HeadsShort { held, of });
        }
    }

    /// What the stream thread answers: a project's take comes in as its
    /// file, a recorded one moves to it, a file no take wants any more is
    /// closed; and how many regions' starts the cache holds.
    fn stream_said(&mut self, o: FromStream) {
        match o {
            FromStream::Problem(r) => self.problem(Problem::of(r)),
            // For tracks since emptied: its source goes.
            FromStream::Open { era, take, gen, .. } if era != self.generation => {
                self.send(ToWorker::Close { take, gen });
            }
            FromStream::Failed { era, .. } if era != self.generation => {}
            FromStream::Heads { heads, held } => {
                self.heads = Some(heads);
                self.heads_held = held;
                self.heads_short();
            }
            FromStream::HeadsGivenBack { held } => {
                self.heads_held = held;
                self.heads_short();
            }
            FromStream::CueHeld { cue } => self.cue_said(cue, CueAudio::Held),
            FromStream::CueShort { cue } => self.cue_said(cue, CueAudio::Short),
            FromStream::CueGivenBack { cue } => self.cue_said(cue, CueAudio::GivenBack),
            FromStream::Open {
                take,
                rec,
                gen,
                growing,
                ..
            } => {
                let ours = self.tracks.take(take).is_some_and(|t| t.rec == rec);
                if growing {
                    // A recording's file as it is written: its memory goes
                    // back as the file holds it. One already on a file
                    // that went (its card): it carried on in another
                    // folder, and plays from its file there from now on.
                    if ours && self.tracks.file_gen(take).is_some_and(|g| g != gen) {
                        self.tracks.command(Command::Refile { take, rec, gen });
                    } else if ours && self.tracks.file_frames(take) == Some(0) {
                        self.tracks.command(Command::OnFile {
                            take,
                            rec,
                            gen,
                            growing: true,
                        });
                    } else {
                        self.send(ToWorker::Close { take, gen });
                    }
                } else if let Some(i) = self
                    .opening
                    .iter()
                    .position(|k| k.id == take && k.rec == rec)
                {
                    let k = self.opening.swap_remove(i);
                    if !self.tracks.add_file_take(&k, gen) {
                        self.tracks.reserve(take, false);
                        self.send(ToWorker::Close { take, gen });
                        self.event(FileEvent::Unopened {
                            track: k.track,
                            why: "its place was taken".into(),
                            kept: false,
                        });
                    }
                    self.tracks_in();
                } else if self.tracks.take(take).is_some_and(|t| t.rec == rec)
                    && self.tracks.file_gen(take).is_some_and(|g| g != gen)
                {
                    // Its file opened again (its card came back): the
                    // source it had is gone, so the take moves to this one.
                    self.tracks.command(Command::Refile { take, rec, gen });
                } else if self
                    .tracks
                    .take(take)
                    .is_some_and(|t| t.rec == rec && !t.recording)
                    && !self.tracks.on_file(take)
                {
                    self.tracks.command(Command::OnFile {
                        take,
                        rec,
                        gen,
                        growing: false,
                    });
                } else {
                    self.send(ToWorker::Close { take, gen });
                }
            }
            FromStream::Failed { take, rec, why, .. } => {
                if let Some(i) = self
                    .opening
                    .iter()
                    .position(|k| k.id == take && k.rec == rec)
                {
                    let k = self.opening.swap_remove(i);
                    self.tracks.reserve(take, false);
                    self.event(FileEvent::Unopened {
                        track: k.track,
                        why,
                        kept: false,
                    });
                    self.tracks_in();
                } else if let Some(t) = self.tracks.take(take).filter(|t| t.rec == rec) {
                    self.event(FileEvent::Unopened {
                        track: t.track,
                        why,
                        kept: true,
                    });
                }
            }
        }
    }

    /// A take written to its file (a sound brought in, or a stretched
    /// copy) opens as a project's take does; one that couldn't be written
    /// comes into memory instead, as it would have with takes in memory,
    /// and the log says why.
    fn take_written(
        &mut self,
        rec: RecordingId,
        gen: u32,
        out: Result<(String, PathBuf), (String, Vec<f32>)>,
    ) {
        let i = self.to_write.iter().position(|k| k.rec == rec);
        match (i.filter(|_| gen == self.generation), out) {
            (Some(i), Ok((name, path))) => {
                let mut k = self.to_write.swap_remove(i);
                self.send(ToWorker::Known {
                    rec,
                    take: k.id,
                    name,
                    path: Some(path),
                    frames: k.frames,
                    channels: k.channels,
                });
                let prov = k.prov.take();
                self.keep_prov(rec, prov);
                self.opening.push(k);
            }
            (Some(i), Err((why, audio))) => {
                let mut k = self.to_write.swap_remove(i);
                self.tracks.reserve(k.id, false);
                let (track, id) = (k.track, k.id);
                k.audio = audio;
                let prov = k.prov.take();
                let kept = self.tracks.load_take(k);
                let rec = match kept {
                    Ok(()) => self.tracks.take(id).map_or(RecordingId::NONE, |t| t.rec),
                    Err(_) => RecordingId::NONE,
                };
                self.keep_prov(rec, prov);
                match kept {
                    Ok(()) => self.event(FileEvent::Unwritten { track, why }),
                    Err(k) => {
                        self.send(ToWorker::Drop(k.audio));
                        self.event(FileEvent::Unopened {
                            track,
                            why: format!("{why}; nor was there room in memory"),
                            kept: false,
                        });
                    }
                }
                // A project's tracks may have waited for it.
                self.tracks_in();
            }
            // For tracks since emptied: the file stays in the work folder.
            (None, Ok(_)) => {}
            (None, Err((_, audio))) => self.send(ToWorker::Drop(audio)),
        }
    }

    /// A project's tracks go in once none of its takes is still opening:
    /// without the regions of the takes left out.
    fn tracks_in(&mut self) {
        if !self.opening.is_empty() || !self.to_write.is_empty() {
            return;
        }
        let Some(mut saved) = self.waiting.take() else {
            return;
        };
        for t in &mut saved.tracks {
            if !AudioTracks::track_fits(t) {
                // Listed by the load.
                continue;
            }
            let tracks = &self.tracks;
            t.regions.retain(|r| tracks.take(r.take).is_some());
            if !self.tracks.load_track(t) {
                self.event(FileEvent::TrackLeftOut { track: t.index });
            }
        }
        self.send(ToWorker::DropSaved(saved));
    }

    /// What the worker said about a file.
    fn heard(&mut self, said: Said) {
        match said {
            Said::CarriedOn { rec, hole } => {
                if let Some(id) = self.tracks.take_of(rec).filter(|_| hole > 0) {
                    if let Some(p) = self.card_parts.get_mut(usize::from(id)) {
                        *p = Some(rec);
                    }
                }
                self.look_card_out();
                if let Some(t) = self.tracks.take_of(rec).and_then(|id| self.tracks.take(id)) {
                    let hole_s = hole as f32 / self.tracks.rate() as f32;
                    let track = t.track;
                    self.event(FileEvent::CarriedOn { track, hole_s });
                }
            }
            Said::Joined { rec, frames } => {
                if let Some(p) = self.card_parts.iter_mut().find(|p| **p == Some(rec)) {
                    *p = None;
                }
                self.look_card_out();
                if let Some(t) = self.tracks.take_of(rec).and_then(|id| self.tracks.take(id)) {
                    let joined_s = frames as f32 / self.tracks.rate() as f32;
                    let track = t.track;
                    self.event(FileEvent::Joined { track, joined_s });
                }
            }
            Said::Unjoined { rec, why } => {
                if let Some(t) = self.tracks.take_of(rec).and_then(|id| self.tracks.take(id)) {
                    let track = t.track;
                    self.event(FileEvent::Unjoined { track, why });
                }
            }
            Said::Ready(rec) => {
                for a in self.files.armed.iter_mut().flatten() {
                    if a.0 == rec {
                        a.1 = true;
                    }
                }
            }
            Said::Failed { rec, why } => {
                // The files for a track's next take: the track doesn't arm.
                if let Some(t) = (0..MAX_TRACKS)
                    .find(|&t| self.tracks.next_recording(t).map(|(r, _)| r) == Some(rec))
                {
                    self.tracks.command(Command::Arm(t, false));
                    self.files.armed[t] = None;
                    self.event(FileEvent::ArmFailed { track: t, why });
                } else if let Some(t) = self.tracks.take_of(rec).and_then(|id| self.tracks.take(id))
                {
                    // A take already recording into them: memory only.
                    self.event(FileEvent::NoFile {
                        track: t.track,
                        why,
                    });
                }
            }
            Said::View { rec, view } => {
                let Some(id) = self.tracks.take_of(rec) else {
                    return;
                };
                let before = self.files.views[id as usize].map(|(_, v)| v.state);
                self.files.views[id as usize] = Some((rec, view));
                let worth = match view.state {
                    files::TakeState::AtRisk(_) | files::TakeState::Stopped(_) => true,
                    files::TakeState::Saved => {
                        matches!(before, Some(files::TakeState::AtRisk(_)))
                    }
                    _ => false,
                };
                if worth && before != Some(view.state) {
                    let track = self.tracks.take(id).map_or(0, |t| t.track);
                    self.event(FileEvent::State {
                        track,
                        state: view.state,
                    });
                }
            }
        }
    }

    /// The tracks' requests about files, to the worker in order (each
    /// take starting to record gets its copying out).
    fn send_asks(&mut self) {
        while let Some(io) = self.tracks.take_io() {
            match io {
                Io::Arm {
                    track,
                    rec,
                    channels,
                } => {
                    let name = self.files.names[usize::from(track) % MAX_TRACKS];
                    if let Some(a) = self.files.armed.get_mut(usize::from(track)) {
                        *a = Some((rec, false));
                    }
                    self.send(ToWorker::Arm {
                        track,
                        rec,
                        channels,
                        name,
                    });
                }
                Io::Disarm { rec } => {
                    for a in &mut self.files.armed {
                        if a.is_some_and(|(r, _)| r == rec) {
                            *a = None;
                        }
                    }
                    self.send(ToWorker::Disarm(rec));
                }
                Io::Start { take } => {
                    let Some(t) = self.tracks.take(take) else {
                        continue;
                    };
                    if let Some(a) = self.files.armed.get_mut(t.track) {
                        if a.is_some_and(|(r, _)| r == t.rec) {
                            *a = None;
                        }
                    }
                    self.files.views[take as usize] = Some((
                        t.rec,
                        FileView {
                            state: files::TakeState::Recording,
                            ..FileView::SAVED
                        },
                    ));
                    self.send(ToWorker::Start { rec: t.rec, take });
                    if self.files.outs.len() < self.files.outs.capacity() {
                        self.files.outs.push(Out {
                            take,
                            rec: t.rec,
                            channels: t.channels,
                            sent: 0,
                            buf: None,
                            fill: 0,
                        });
                    } else {
                        // Never: a take writing can't be forgotten, so
                        // there are never more than MAX_TAKES.
                        self.tracks.written(take);
                    }
                }
                Io::Close { take, gen } => self.send(ToWorker::Close { take, gen }),
            }
        }
    }

    /// Recording to files: the tracks' requests to the worker, and the
    /// takes recording copied out to it, at most [`COPY_SAMPLES`] samples
    /// a period, a buffer at a time. A take copied out in full once it has
    /// stopped is finished, and can be forgotten again.
    fn tend_files(&mut self) {
        self.send_asks();
        let mut budget = COPY_SAMPLES;
        let mut i = 0;
        while i < self.files.outs.len() {
            let FileSide { spare, outs, .. } = &mut self.files;
            let o = &mut outs[i];
            let Some(t) = self.tracks.take(o.take).filter(|t| t.rec == o.rec) else {
                // Never: a take writing can't be forgotten.
                if let Some(b) = o.buf.take() {
                    spare.push(b);
                }
                outs.swap_remove(i);
                continue;
            };
            let ch = o.channels.max(1);
            let full = CHUNK - CHUNK % ch;
            while o.sent < t.frames && budget >= ch {
                if o.buf.is_none() {
                    match spare.pop() {
                        Some(b) => o.buf = Some(b),
                        None => break,
                    }
                }
                let buf = o.buf.as_mut().expect("just made sure");
                let n = (t.frames - o.sent)
                    .min((full - o.fill) / ch)
                    .min(budget / ch);
                let got = self
                    .tracks
                    .read_take(o.take, o.sent, &mut buf[o.fill..o.fill + n * ch])
                    .unwrap_or(0);
                o.sent += got;
                o.fill += got * ch;
                budget -= got * ch;
                if o.fill == full {
                    let buf = o.buf.take().expect("filled");
                    if let Some(tx) = &self.tx {
                        let _ = tx.send(ToWorker::Push {
                            rec: o.rec,
                            buf,
                            len: full,
                        });
                    }
                    o.fill = 0;
                }
                if got == 0 {
                    break;
                }
            }
            if !t.recording && o.sent >= t.frames {
                let o = outs.swap_remove(i);
                if let Some(buf) = o.buf {
                    if o.fill > 0 {
                        self.send(ToWorker::Push {
                            rec: o.rec,
                            buf,
                            len: o.fill,
                        });
                    } else {
                        self.files.spare.push(buf);
                    }
                }
                self.send(ToWorker::Stop(o.rec));
                self.tracks.written(o.take);
                continue;
            }
            i += 1;
        }
    }

    /// The wave for take `id` in `n` columns, into `columns` (empty, with
    /// room for `n`); None (the room going to the worker) if the take has
    /// gone.
    fn start_wave(&mut self, id: u16, n: usize, columns: Vec<(f32, f32)>) -> Option<Waving> {
        let wave = |frames, channels, fpc| TakeWave {
            id,
            frames,
            channels,
            frames_per_column: fpc,
            columns,
        };
        let Some(t) = self.tracks.take(id) else {
            self.send(ToWorker::DropWave(wave(0, 0, 0)));
            return None;
        };
        if self.plays_from_file(id) {
            // It moved to its file while the room was made.
            self.send(ToWorker::DropWave(wave(0, 0, 0)));
            self.wave_from_file(t, n);
            return self.waving.take();
        }
        // Whole blocks of the table to a column.
        let step = (PEAK_BLOCK / t.channels.max(1)).max(1);
        let fpc = t.frames.div_ceil(n).div_ceil(step).max(1) * step;
        Some(Waving::Reading {
            wave: wave(t.frames, t.channels, fpc),
            at: 0,
        })
    }

    fn tend_wave(&mut self, budget: usize) {
        if !matches!(self.waving, Some(Waving::Reading { .. })) {
            return;
        }
        let Some(Waving::Reading { mut wave, mut at }) = self.waving.take() else {
            return;
        };
        let mut left = budget;
        while at < wave.frames && left > 0 {
            let to = (at + wave.frames_per_column).min(wave.frames);
            match self.tracks.peaks(wave.id, at, to) {
                Some(p) => wave.columns.push(p),
                None => {
                    // Forgotten while it was read, or moved to its file
                    // (read from there instead).
                    let n = wave.columns.capacity();
                    let moved = self.tracks.take(wave.id).filter(|t| {
                        self.plays_from_file(t.id)
                            && t.frames == wave.frames
                            && t.channels == wave.channels
                    });
                    self.send(ToWorker::DropWave(wave));
                    if let Some(t) = moved {
                        self.wave_from_file(t, n);
                    }
                    return;
                }
            }
            left = left.saturating_sub(to - at);
            at = to;
        }
        if at >= wave.frames {
            if let Some(old) = self.wave.replace(wave) {
                self.send(ToWorker::DropWave(old));
            }
        } else {
            self.waving = Some(Waving::Reading { wave, at });
        }
    }

    /// Whether no take is still on its way into its file: none being
    /// copied out, and none the writer last said was Recording or Saving.
    fn files_settled(&self) -> bool {
        self.files.outs.is_empty()
            && !self.files.views.iter().flatten().any(|(_, v)| {
                matches!(
                    v.state,
                    files::TakeState::Recording | files::TakeState::Saving
                )
            })
    }

    /// Whether the writer has done what it can with every take: none
    /// being copied out, and none it made a file for still Recording or
    /// Saving (a take since deleted isn't heard of again, so its last
    /// word doesn't count). A take At risk or Stopped after its stop may
    /// never finish (its drive full or gone: the writer retries for
    /// ever), so it doesn't hold recording in the folder before: it is
    /// left there as a recording for the next start's recovery, played
    /// and saved from what it committed there and what is in memory.
    fn files_finished(&self) -> bool {
        self.files.outs.is_empty()
            && self.files.views.iter().enumerate().all(|(id, v)| {
                v.is_none_or(|(rec, v)| {
                    !matches!(
                        v.state,
                        files::TakeState::Recording | files::TakeState::Saving
                    ) || self.tracks.take(id as u16).is_none_or(|t| t.rec != rec)
                })
            })
    }

    fn tend_save(&mut self, budget: usize) {
        self.save_step(budget);
        // While the takes are named and read, what each has in memory
        // stays there, so the worker and the reading agree on where each
        // frame is.
        let reading = matches!(self.saving, Some(Saving::Sizing | Saving::Reading { .. }));
        self.tracks.keep_memory(reading);
    }

    fn save_step(&mut self, budget: usize) {
        match self.saving.take() {
            None => {}
            Some(Saving::Asking) => self.saving = Some(Saving::Asking),
            Some(Saving::Waiting(mut room)) => {
                // Settled first, and only then filled, into the worker's
                // room: nothing is made here, nor while a take records,
                // loads, or is still on its way into its file (a save
                // names only finished files: it waits while one is
                // Saving), nor while a project's take waits for its file
                // to open (it isn't in the tracks yet).
                let filled = if self.files_settled()
                    && self.opening.is_empty()
                    && self.to_write.is_empty()
                {
                    self.tracks.save_into(&mut room)
                } else {
                    None
                };
                match filled {
                    None => self.saving = Some(Saving::Waiting(room)),
                    Some((tracks, takes)) => {
                        self.send(ToWorker::Size {
                            saved: room,
                            tracks,
                            takes,
                        });
                        self.saving = Some(Saving::Sizing);
                    }
                }
            }
            Some(Saving::Sizing) => self.saving = Some(Saving::Sizing),
            Some(Saving::Reading {
                mut saved,
                mut i,
                mut frame,
            }) => {
                let mut left = budget;
                while left > 0 && i < saved.takes.len() {
                    let k = &mut saved.takes[i];
                    if k.path.is_some() {
                        // Copied whole from its file by the save.
                        i += 1;
                        continue;
                    }
                    let ch = k.channels;
                    if frame == 0 {
                        // A take whose memory went to its file as it
                        // recorded: the worker read that part from there.
                        frame = self
                            .tracks
                            .file_frames(k.id)
                            .filter(|&n| n < k.frames)
                            .unwrap_or(0);
                    }
                    let to = (frame + left).min(k.frames);
                    match self
                        .tracks
                        .read_take(k.id, frame, &mut k.audio[frame * ch..to * ch])
                    {
                        Some(n) => {
                            frame += n;
                            left -= n.max(1).min(left);
                        }
                        None => {
                            // Forgotten while it was read: leave it out.
                            let gone = saved.takes.remove(i);
                            saved
                                .tracks
                                .iter_mut()
                                .for_each(|t| t.regions.retain(|r| r.take != gone.id));
                            self.send(ToWorker::Drop(gone.audio));
                            frame = 0;
                            continue;
                        }
                    }
                    if frame >= saved.takes[i].frames {
                        i += 1;
                        frame = 0;
                    }
                }
                if i >= saved.takes.len() {
                    if let Some(old) = self.saved.replace(saved) {
                        self.send(ToWorker::DropSaved(old));
                    }
                } else {
                    self.saving = Some(Saving::Reading { saved, i, frame });
                }
            }
        }
    }

    fn tend_fit(&mut self, budget: usize) {
        match self.fitting.take() {
            Some(Fitting::Reading {
                take,
                ratio,
                bpm,
                mut audio,
                mut frame,
            }) => {
                let ch = take.channels;
                let to = (frame + budget).min(take.frames);
                match self
                    .tracks
                    .read_take(take.id, frame, &mut audio[frame * ch..to * ch])
                {
                    None if self.moved_to_file(take.id, take.rec) => {
                        // Read from its file instead.
                        self.send(ToWorker::Drop(audio));
                        self.fit_from_file(take, ratio, bpm);
                    }
                    None => {
                        self.fits_failed += 1;
                        self.send(ToWorker::Drop(audio));
                    }
                    Some(n) => {
                        frame += n;
                        if frame >= take.frames {
                            match self.tracks.free_take_id() {
                                Some(id) => {
                                    self.send(ToWorker::Stretch {
                                        take: Box::new(take),
                                        audio,
                                        ratio,
                                        bpm,
                                        id,
                                        gen: self.generation,
                                    });
                                    self.fitting = Some(Fitting::Stretching { take });
                                }
                                None => {
                                    self.fits_failed += 1;
                                    self.send(ToWorker::Drop(audio));
                                }
                            }
                        } else {
                            self.fitting = Some(Fitting::Reading {
                                take,
                                ratio,
                                bpm,
                                audio,
                                frame,
                            });
                        }
                    }
                }
            }
            Some(Fitting::Loading { new, swap }) => {
                if self.tracks.take(new).is_some() && self.tracks.loading() == 0 {
                    // A copy for a clip is said, and the song keeps the
                    // take it plays.
                    match (self.fit_copy, swap) {
                        (true, Command::Swap { old, new, ratio }) => {
                            let rec = |id| self.tracks.take(id).map(|t| t.rec);
                            if let (Some(from), Some(to)) = (rec(old), rec(new)) {
                                self.fitted = Some(FitCopy { from, to, ratio });
                            }
                        }
                        _ => {
                            self.tracks.command(swap);
                        }
                    }
                    self.fits += 1;
                } else if self.tracks.take(new).is_some() || self.coming(new) {
                    self.fitting = Some(Fitting::Loading { new, swap });
                } else {
                    // Its file didn't open, nor could it come into memory.
                    self.fits_failed += 1;
                }
            }
            other => self.fitting = other,
        }
    }
}

/// `p` kept for System status if there is room, else counted.
fn keep_problem(room: &mut Vec<Problem>, lost: &mut u64, p: Problem) {
    if room.len() < PROBLEMS {
        room.push(p);
    } else {
        *lost += 1;
    }
}

/// The worker thread: every big buffer made and dropped here, stretches,
/// and the files (see the module notes). It wakes at least every 20 ms
/// while the writer runs, to see how its takes stand. `back` is its own
/// channel, for the stream thread's whole-file reads.
/// The scheduler's kind for the tracks' copies.
const COPY_KIND: &str = "audio tracks copy";

/// The next host's number among the scheduler's copy jobs.
static NEXT_OWNER: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(1);

/// A copy's job on the process's one scheduler, for host `owner`: its
/// number in the id's low 16 bits, so two hosts making the same copy (two
/// projects' engines, tests side by side) never replace each other's job,
/// and Clear copies stops only its own ([`ours`]).
fn copy_key(owner: u16, key: &crate::copies::CopyKey) -> sched::Key {
    sched::Key::new(COPY_KIND, (key.id() & !0xffff) | u128::from(owner))
}

/// Whether job `k` is one of host `owner`'s copies.
fn ours(owner: u16, k: sched::Key) -> bool {
    k.kind == COPY_KIND && k.id as u16 == owner
}

/// A copy being made on the scheduler: its job starts (finding the hits)
/// in its first step, and what comes of it goes back to the host.
struct CopyWork {
    spec: Option<crate::copies::CopySpec>,
    job: Option<crate::copies::CopyJob>,
    done: Sender<FromWorker>,
    gen: u32,
    /// Where it says how far it has got, for the host to read (5c).
    progress: Arc<crate::copying::Progress>,
    slot: u8,
    tag: u32,
}

impl CopyWork {
    fn key(&self) -> crate::copies::CopyKey {
        match (&self.job, &self.spec) {
            (Some(j), _) => *j.key(),
            (None, Some(s)) => s.key,
            (None, None) => unreachable!("a copy's work has its spec or its job"),
        }
    }
}

impl sched::Work for CopyWork {
    fn step(&mut self, frames: usize) -> bool {
        if let Some(spec) = self.spec.take() {
            let key = spec.key;
            match spec.start() {
                Ok(j) => self.job = Some(j),
                Err(_) => {
                    let _ = self.done.send(FromWorker::CopyGone {
                        key,
                        why: Gone::Failed,
                        gen: self.gen,
                    });
                    return true;
                }
            }
            return false;
        }
        let done = self.job.as_mut().is_none_or(|j| j.run(frames));
        let p = self.job.as_ref().map_or(0.0, |j| j.progress());
        self.progress.set(self.slot, self.tag, p);
        done
    }

    fn progress(&self) -> f32 {
        self.job.as_ref().map_or(0.0, |j| j.progress())
    }

    fn finish(self: Box<Self>) {
        let gen = self.gen;
        // A job that couldn't start has said so already.
        let Some(job) = self.job else {
            return;
        };
        let key = *job.key();
        let _ = self.done.send(match job.finish() {
            Some(copy) => FromWorker::Copied { copy, gen },
            None => FromWorker::CopyGone {
                key,
                why: Gone::Failed,
                gen,
            },
        });
    }

    fn cancelled(self: Box<Self>) {
        let _ = self.done.send(FromWorker::CopyGone {
            key: self.key(),
            why: Gone::Cancelled,
            gen: self.gen,
        });
    }

    /// Refused for want of memory: counted ([`CopyCounts::refused`]); the
    /// reason stays here, as a string would be freed on the audio thread.
    fn refused(self: Box<Self>, _why: String) {
        let _ = self.done.send(FromWorker::CopyGone {
            key: self.key(),
            why: Gone::Refused,
            gen: self.gen,
        });
    }
}

fn work(
    rate: u32,
    jobs: &Receiver<ToWorker>,
    done: &Sender<FromWorker>,
    back: &Sender<ToWorker>,
    board: Board,
) {
    let mut w = Worker {
        rate,
        board,
        files: Files::new(rate),
        stream: None,
        cards: Vec::new(),
        work_gate: None,
        wants: Vec::new(),
        next_job: 1,
        era: 0,
        drive: None,
        drive_stop: None,
        layers: Vec::new(),
        sched: None,
        owner: NEXT_OWNER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    };
    loop {
        let m = if w.files.writing() || !w.layers.is_empty() {
            match jobs.recv_timeout(Duration::from_millis(20)) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match jobs.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            }
        };
        let reply = m.and_then(|m| w.serve(m, done, back));
        if let Some(r) = reply {
            if done.send(r).is_err() {
                break;
            }
        }
        for said in w.files.poll(false) {
            if done.send(FromWorker::Files(said)).is_err() {
                return;
            }
        }
        for p in w.files.take_problems() {
            if done.send(FromWorker::Problem(p)).is_err() {
                return;
            }
        }
        w.note_syncs();
        w.open_finished();
        if !w.tend_layers(done) {
            return;
        }
    }
}

/// What the worker waits for a take's file for.
enum Want {
    Fit {
        take: TakeInfo,
        ratio: f64,
        bpm: f64,
        gen: u32,
    },
    Wave {
        take: TakeInfo,
        columns: usize,
    },
}

/// The worker's state.
struct Worker {
    rate: u32,
    files: Files,
    /// The stream layer, once started.
    stream: Option<Stream>,
    /// The card last opened at each folder ([`ToWorker::Device`]), given
    /// to the stream layer once it starts.
    cards: Vec<Arc<dyn CardHandle>>,
    /// Where the recording's sync times go, with the stream thread's
    /// reads.
    board: Board,
    /// Playback's cushion on the drive takes record to.
    work_gate: Option<Arc<Gate>>,
    /// Whole-file reads asked of the stream thread, and what each is for.
    wants: Vec<(u32, Want)>,
    next_job: u32,
    /// The tracks' generation (as [`ToWorker::Clear`] last said): each
    /// file opened carries it, so an answer for tracks since emptied is
    /// told apart.
    era: u32,
    /// How to look at the drive and where its looks go, and the end of
    /// the thread looking at it (dropped, it stops).
    drive: Option<(DriveProbe, Arc<DriveSeen>)>,
    /// Where the drive's thread is told the takes' folder moved (dropped,
    /// it stops).
    drive_stop: Option<Sender<(PathBuf, bool)>>,
    /// Loop layers' files being written (phase 4c.3).
    layers: Vec<LayerWrite>,
    /// Where copies are made: the process's scheduler unless a test gave
    /// its own ([`ToWorker::Sched`]).
    sched: Option<Sched>,
    /// Its number among the scheduler's copy jobs ([`copy_key`]).
    owner: u16,
}

impl Worker {
    /// Write what the loop layers' files have waiting, as playback's
    /// cushion allows; each finished, or failed, is said and its buffers
    /// go back. False once the audio thread has gone.
    fn tend_layers(&mut self, done: &Sender<FromWorker>) -> bool {
        let cushion = self
            .stream
            .as_ref()
            .map(|st| self.work_gate.as_ref().unwrap_or(&st.gate).cushion());
        let mut spent = Vec::new();
        let mut i = 0;
        while i < self.layers.len() {
            let l = &mut self.layers[i];
            let rec = l.rec;
            let out = match l.tend(cushion, &mut spent) {
                Err(e) => Some(Err(e)),
                Ok(()) if l.done() => Some(Ok(())),
                Ok(()) => None,
            };
            let Some(out) = out else {
                i += 1;
                continue;
            };
            let l = self.layers.swap_remove(i);
            let why = match out {
                Ok(()) => match l.finish() {
                    Ok((name, path)) => {
                        self.files.layer_written(name, path);
                        None
                    }
                    Err(e) => Some(e.to_string()),
                },
                Err(e) => {
                    l.abandon(&mut spent);
                    Some(e.to_string())
                }
            };
            if done.send(FromWorker::LayerWritten { rec, why }).is_err() {
                return false;
            }
        }
        for b in spent {
            if done.send(FromWorker::LayerSpare(b)).is_err() {
                return false;
            }
        }
        true
    }

    /// Look at the drive the takes record to, once there is a folder and
    /// a way to look: a thread of its own, told when the folder moves (to
    /// the SD card and back), so one watch says each drive's problems.
    fn look_at_drive(&mut self, done: &Sender<FromWorker>) {
        let (Some((probe, seen)), Some(dir)) = (self.drive.as_ref(), self.files.work_dir()) else {
            return;
        };
        // A folder on a drive of its own is on the SD card.
        let card = self.cards.iter().any(|c| dir.starts_with(c.root()));
        if let Some(moved) = &self.drive_stop {
            if moved.send((dir.to_path_buf(), card)).is_ok() {
                return;
            }
        }
        let (moved, heard) = channel();
        let done = done.clone();
        let started = crate::files::watch_drive(
            dir.to_path_buf(),
            card,
            *probe,
            self.files.armed(),
            Arc::clone(seen),
            heard,
            move |p| done.send(FromWorker::Problem(p)).is_ok(),
        );
        self.drive_stop = started.is_ok().then_some(moved);
    }

    /// The writer's sync times onto the board, when there are new ones.
    fn note_syncs(&mut self) {
        let now = self.files.syncs();
        let Ok(mut t) = self.board.lock() else {
            return;
        };
        let same = match (&t.syncs, &now) {
            (Some(s), Some((dir, times))) => s.dir == *dir && s.times == *times,
            (None, None) => true,
            _ => false,
        };
        if !same {
            t.syncs = now.map(|(dir, times)| Syncs {
                on_card: self.cards.iter().any(|c| dir.starts_with(c.root())),
                dir: dir.to_path_buf(),
                times,
            });
        }
    }

    /// Take files finished (recorded, or a project's) go to the stream
    /// thread to open, and recordings' files to open or grow as they are
    /// written.
    fn open_finished(&mut self) {
        let Some(st) = &self.stream else {
            return;
        };
        let era = self.era;
        for (take, rec, path, channels, frames) in self.files.take_opens() {
            let _ = st.tx.send(ToStream::Open {
                era,
                take,
                rec,
                path,
                channels,
                frames,
            });
        }
        for g in self.files.take_grows() {
            let _ = st.tx.send(match g {
                Growing::Open {
                    take,
                    rec,
                    path,
                    channels,
                    frames,
                } => ToStream::OpenGrowing {
                    era,
                    take,
                    rec,
                    path,
                    channels,
                    frames,
                },
                Growing::Grow { take, rec, frames } => ToStream::Grow {
                    era,
                    take,
                    rec,
                    frames,
                },
                Growing::Finished {
                    take,
                    rec,
                    path,
                    channels,
                    frames,
                } => ToStream::Finished {
                    era,
                    take,
                    rec,
                    path,
                    channels,
                    frames,
                },
            });
        }
    }

    fn to_stream(&self, m: ToStream) {
        if let Some(st) = &self.stream {
            let _ = st.tx.send(m);
        }
    }

    /// Read recording `rec`'s file behind playback, for `want`; None if it
    /// is being read, else the answer now (it has no file, or nothing
    /// streams).
    fn read_for(&mut self, rec: RecordingId, want: Want) -> Option<FromWorker> {
        match (self.files.path_of(rec), &self.stream) {
            (Some(path), Some(_)) => {
                let job = self.next_job;
                self.next_job = self.next_job.wrapping_add(1);
                self.to_stream(ToStream::Read { job, path });
                self.wants.push((job, want));
                None
            }
            _ => Some(self.answer(want, Err("it has no file to read".into()))),
        }
    }

    /// What a read of a take's file was for, done with its bytes.
    fn answer(&self, want: Want, bytes: Result<Vec<u8>, String>) -> FromWorker {
        let rate = self.rate;
        let audio = |take: &TakeInfo| -> Option<Vec<f32>> {
            let (audio, ch, r) = crate::save::from_wav(bytes.as_ref().ok()?).ok()?;
            (ch == take.channels && r == rate && audio.len() == take.frames * ch).then_some(audio)
        };
        match want {
            Want::Fit {
                take,
                ratio,
                bpm,
                gen,
            } => {
                let out = audio(&take).and_then(|a| {
                    let mut j = StretchJob::new(take, a, rate, ratio, bpm, take.id).ok()?;
                    while !j.run(1 << 15).finished {}
                    j.finish()
                });
                FromWorker::Stretched { out, gen }
            }
            Want::Wave { take, columns } => {
                let a = audio(&take).unwrap_or_default();
                FromWorker::WaveFile(wave_of(&take, &a, columns))
            }
        }
    }

    /// One message to the worker, and its answer if it has one.
    fn serve(
        &mut self,
        m: ToWorker,
        done: &Sender<FromWorker>,
        back: &Sender<ToWorker>,
    ) -> Option<FromWorker> {
        let (rate, files) = (self.rate, &mut self.files);
        Some(match m {
            ToWorker::Copy(Ask::Buffer { n, key, gen }) => FromWorker::CopyBuffer {
                key,
                audio: vec![0.0; n],
                gen,
            },
            ToWorker::Copy(Ask::Make {
                spec,
                priority,
                need,
                gen,
                progress,
                slot,
            }) => {
                let key = spec.key;
                let s = self.sched.get_or_insert_with(Sched::shared);
                let work = CopyWork {
                    tag: crate::copying::Progress::tag(&key),
                    spec: Some(spec),
                    job: None,
                    done: done.clone(),
                    gen,
                    progress,
                    slot,
                };
                s.submit(sched::Job::new(copy_key(self.owner, &key), priority, work).needs(need));
                return None;
            }
            ToWorker::Copy(Ask::Cancel(key)) => {
                if let Some(s) = &self.sched {
                    s.cancel(copy_key(self.owner, &key));
                }
                return None;
            }
            ToWorker::Copy(Ask::CancelAll) => {
                if let Some(s) = &self.sched {
                    let owner = self.owner;
                    s.retain(|k| !ours(owner, k));
                }
                return None;
            }
            ToWorker::DropCopy(d) => {
                drop(d);
                return None;
            }
            ToWorker::DropClips(c) => {
                drop(c);
                return None;
            }
            ToWorker::Sched(s) => {
                self.sched = Some(s);
                return None;
            }
            ToWorker::RenderBuffer { n, gen } => FromWorker::RenderBuffer {
                audio: vec![0.0; n],
                gen,
            },
            ToWorker::Render {
                desc,
                audio,
                channels,
                frames,
                rec,
                at,
                gen,
                progress,
            } => {
                let prov = Provenance::of(&desc, at);
                let Some(dir) = files.work_dir().map(std::path::Path::to_path_buf) else {
                    return Some(FromWorker::RenderGone {
                        rec,
                        why: RenderOutcome::Failed(RenderFailure::NoWorkFolder),
                        leftovers: Leftovers {
                            desc: Some(desc),
                            prov: Some(prov),
                            name: None,
                            path: None,
                            audio,
                            remove: false,
                        },
                        gen,
                    });
                };
                let work = RenderWork {
                    desc: Some(desc),
                    prov: Some(prov),
                    audio,
                    channels,
                    frames,
                    rate,
                    rec,
                    dir,
                    file: None,
                    put: 0,
                    failed: None,
                    done: done.clone(),
                    gen,
                    progress,
                };
                let s = self.sched.get_or_insert_with(Sched::shared);
                s.submit(sched::Job::new(
                    render_key(self.owner, rec),
                    sched::Priority::Prepared,
                    work,
                ));
                return None;
            }
            ToWorker::CancelRender(rec) => {
                if let Some(s) = &self.sched {
                    s.cancel(render_key(self.owner, rec));
                }
                return None;
            }
            ToWorker::RenderLeftovers(l) => {
                l.drop_on_worker(files.work_dir());
                return None;
            }
            ToWorker::Room => FromWorker::Room(Saved::room(rate)),
            ToWorker::Size {
                mut saved,
                tracks,
                takes,
            } => {
                saved.tracks.truncate(tracks);
                saved.takes.truncate(takes);
                // The writer's latest word first, so a take just Saved is
                // copied from its file.
                let _ = files.poll(true);
                let said = files.name_save(&mut saved.takes);
                FromWorker::Sized(saved, said)
            }
            ToWorker::Buffer { n, gen } => FromWorker::Buffer {
                audio: vec![0.0; n],
                gen,
            },
            ToWorker::Stretch {
                take,
                audio,
                ratio,
                bpm,
                id,
                gen,
            } => {
                let out = StretchJob::new(*take, audio, rate, ratio, bpm, id)
                    .ok()
                    .and_then(|mut j| {
                        while !j.run(1 << 15).finished {}
                        j.finish()
                    });
                FromWorker::Stretched { out, gen }
            }
            ToWorker::Wave(n) => FromWorker::Wave(Vec::with_capacity(n)),
            ToWorker::Drop(v) => {
                drop(v);
                return None;
            }
            ToWorker::DropSaved(s) => {
                drop(s);
                return None;
            }
            ToWorker::DropWave(w) => {
                drop(w);
                return None;
            }
            ToWorker::DropPlayer(p) => {
                drop(p);
                return None;
            }
            ToWorker::LayerRead(rec) => {
                let name = files::layer_file_name(rec);
                let audio = match files.layer_path(&name) {
                    None => Err(format!("{name} isn't a file this run wrote")),
                    Some(path) => std::fs::read(path)
                        .map_err(|e| e.to_string())
                        .and_then(|b| crate::save::from_wav(&b))
                        .and_then(|(audio, channels, _)| match channels {
                            2 => Ok(audio),
                            n => Err(format!("{n} channels, not 2")),
                        })
                        .map_err(|why| format!("{name}: {why}")),
                };
                FromWorker::LayerRead { rec, audio }
            }
            ToWorker::Device(card) => {
                if let Some(st) = &mut self.stream {
                    // Without its own thread, its files read on the
                    // first drive's, as before.
                    let _ = st.card(card.clone());
                }
                self.cards.retain(|c| c.root() != card.root());
                self.cards.push(card);
                return None;
            }
            ToWorker::CarryOn(dir) => match files.carry_on(dir, {
                // Which card went: the one the folder was on.
                let was = files.work_dir();
                let card = self
                    .cards
                    .iter()
                    .find(|c| was.is_some_and(|d| d.starts_with(c.root())));
                card.and_then(|c| c.mark())
            }) {
                Ok(_) => {
                    if let Some(st) = &self.stream {
                        if let Some(d) = files.work_dir() {
                            self.work_gate = Some(st.gate_for(d));
                        }
                    }
                    self.look_at_drive(done);
                    return None;
                }
                Err(e) => FromWorker::CarryFailed(e.to_string()),
            },
            ToWorker::Join { mark, places } => {
                let card = self
                    .cards
                    .iter()
                    .find(|c| c.valid() && c.mark() == Some(mark))
                    .cloned();
                if let Some(card) = card {
                    // Behind playback on the stick, where the takes are.
                    let gate = match (&self.stream, files.hole_dir().or(files.work_dir())) {
                        (Some(st), Some(d)) => st.gate_for(d),
                        _ => Arc::new(Gate::new()),
                    };
                    files.join_card(mark, places, card, gate);
                }
                return None;
            }
            ToWorker::Files { dir, cfg } => {
                let gate = self
                    .stream
                    .as_ref()
                    .map_or_else(|| Arc::new(Gate::new()), |st| st.gate_for(&dir));
                self.work_gate = Some(gate.clone());
                match files.start(dir, cfg, gate) {
                    Ok(()) => {
                        self.look_at_drive(done);
                        return None;
                    }
                    Err(e) => FromWorker::FilesFailed(e.to_string()),
                }
            }
            ToWorker::WatchDrive(probe, seen) => {
                self.drive = Some((probe, seen));
                self.look_at_drive(done);
                return None;
            }
            ToWorker::Arm {
                track,
                rec,
                channels,
                name,
            } => FromWorker::Files(files.arm(track, rec, channels, name)?),
            ToWorker::Disarm(rec) => {
                files.disarm(rec);
                return None;
            }
            ToWorker::Start { rec, take } => match files.start_take(rec, take) {
                Ok(()) => return None,
                Err(why) => FromWorker::NoFile { rec, why },
            },
            ToWorker::Push { rec, buf, len } => {
                files.push(rec, &buf[..len.min(buf.len())]);
                FromWorker::Spare(buf)
            }
            ToWorker::Stop(rec) => {
                files.stop(rec);
                return None;
            }
            ToWorker::Known {
                rec,
                take,
                name,
                path,
                frames,
                channels,
            } => {
                if rec.is_given() {
                    files.known(rec, take, name, path, (channels, frames));
                }
                return None;
            }
            ToWorker::Prov { rec, prov } => {
                if rec.is_given() {
                    files.set_prov(rec, prov);
                }
                return None;
            }
            ToWorker::Clear { gen } => {
                files.clear();
                self.era = gen;
                return None;
            }
            ToWorker::SavedInto(dir) => {
                files.saved_into(&dir);
                return None;
            }
            ToWorker::Stream(cfg) => {
                if self.stream.is_some() {
                    return None;
                }
                let answer = {
                    let done = done.clone();
                    move |o| done.send(FromWorker::Stream(o)).is_ok()
                };
                let read = {
                    let back = back.clone();
                    move |r| back.send(ToWorker::Read(r)).is_ok()
                };
                match streaming::start(rate, *cfg, answer, read, self.board.clone()) {
                    Ok((player, mut st)) => {
                        files.stream();
                        for card in &self.cards {
                            let _ = st.card(card.clone());
                        }
                        self.stream = Some(st);
                        FromWorker::Player(player)
                    }
                    Err(e) => FromWorker::StreamFailed(e.to_string()),
                }
            }
            ToWorker::Heads(heads) => {
                self.to_stream(ToStream::Heads(heads));
                return None;
            }
            ToWorker::Cue { cue, parts } => {
                self.to_stream(ToStream::Cue { cue, parts });
                return None;
            }
            ToWorker::LetCueGo { cue } => {
                self.to_stream(ToStream::LetCueGo { cue });
                return None;
            }
            ToWorker::WriteTake {
                rec,
                channels,
                audio,
                gen,
            } => {
                let written = match (files.work_dir(), &self.stream) {
                    (Some(dir), Some(st)) => {
                        files::write_take(dir, rec, channels, rate, &audio, &st.gate_for(dir))
                            .map_err(|e| e.to_string())
                    }
                    (None, _) => Err("there is no folder to record into".into()),
                    (_, None) => Err("takes don't play from their files".into()),
                };
                let out = match written {
                    Ok(file) => {
                        drop(audio);
                        Ok(file)
                    }
                    Err(why) => Err((why, audio)),
                };
                FromWorker::TakeWritten { rec, gen, out }
            }
            ToWorker::LayerBegin {
                rec,
                frames,
                channels,
            } => {
                let begun = match files.work_dir() {
                    Some(dir) => LayerWrite::begin(dir, rec, frames, channels, rate)
                        .map_err(|e| e.to_string()),
                    None => Err("there is no folder to record into".into()),
                };
                match begun {
                    Ok(w) => {
                        self.layers.retain(|l| l.rec != rec);
                        self.layers.push(w);
                        return None;
                    }
                    Err(why) => FromWorker::LayerWritten {
                        rec,
                        why: Some(why),
                    },
                }
            }
            ToWorker::LayerPiece { rec, buf, len } => {
                match self.layers.iter_mut().find(|l| l.rec == rec) {
                    Some(l) => {
                        l.give(buf, len);
                        return None;
                    }
                    // Its file failed, or was dropped: the buffer goes back.
                    None => FromWorker::LayerSpare(buf),
                }
            }
            ToWorker::LayerEnd(rec) => {
                if let Some(l) = self.layers.iter_mut().find(|l| l.rec == rec) {
                    l.end();
                }
                return None;
            }
            ToWorker::LayerDrop(rec) => {
                let mut spent = Vec::new();
                if let Some(i) = self.layers.iter().position(|l| l.rec == rec) {
                    self.layers.swap_remove(i).abandon(&mut spent);
                }
                for b in spent {
                    let _ = done.send(FromWorker::LayerSpare(b));
                }
                return None;
            }
            ToWorker::Close { take, gen } => {
                self.to_stream(ToStream::Close { take, gen });
                return None;
            }
            ToWorker::Project => {
                self.to_stream(ToStream::Project);
                return None;
            }
            ToWorker::FitFile {
                take,
                ratio,
                bpm,
                gen,
            } => {
                let want = Want::Fit {
                    take,
                    ratio,
                    bpm,
                    gen,
                };
                return self.read_for(take.rec, want);
            }
            ToWorker::WaveFile { take, columns } => {
                // Recorded here: from the peaks of what went to its file.
                if let Some(w) = self.files.wave_of(&take, columns) {
                    return Some(FromWorker::WaveFile(w));
                }
                return self.read_for(take.rec, Want::Wave { take, columns });
            }
            ToWorker::Read(Read { job, bytes }) => {
                let i = self.wants.iter().position(|(j, _)| *j == job)?;
                let (_, want) = self.wants.swap_remove(i);
                self.answer(want, bytes)
            }
        })
    }
}

/// Whether some of take `id` plays from its file (see
/// [`AudioHost::plays_from_file`]).
fn plays_from_file(tr: &AudioTracks, id: u16) -> bool {
    tr.file_frames(id).is_some_and(|n| n > 0)
}

/// A take's waveform in about `columns` columns from its frames `audio`
/// (empty if they couldn't be read: no columns), as
/// [`AudioHost::wave`] reads it from the pool: a column covers whole
/// blocks of [`PEAK_BLOCK`] samples.
fn wave_of(take: &TakeInfo, audio: &[f32], columns: usize) -> TakeWave {
    let ch = take.channels.max(1);
    let frames = if audio.len() == take.frames * ch {
        take.frames
    } else {
        0
    };
    let step = (PEAK_BLOCK / ch).max(1);
    let fpc = take.frames.div_ceil(columns.max(1)).div_ceil(step).max(1) * step;
    let mut out = Vec::with_capacity(frames.div_ceil(fpc));
    let mut at = 0;
    while at < frames {
        let to = (at + fpc).min(frames);
        out.push(crate::min_max(&audio[at * ch..to * ch]));
        at = to;
    }
    TakeWave {
        id: take.id,
        frames,
        channels: take.channels,
        frames_per_column: fpc,
        columns: out,
    }
}
