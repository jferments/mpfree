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
//! The channels to the worker are `std::sync::mpsc`: a message now and
//! then can allocate a block of the channel, as in the looper's host. A
//! period with nothing under way sends nothing.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use mpfree_stream::player::Player;
use mpfree_stream::writer::Gate;

use mpfree_transport::time::TempoMap;
use mpfree_transport::{Block, Tick};

use crate::files::{self, FileEvent, FileView, Files, Name, Said, WriterConfig};
use crate::save::{Saved, TakeFile};
use crate::streaming::{self, Opened, Read, Stream, StreamConfig, ToStream};
use crate::stretch::{fit_ratio, StretchJob};
use crate::{
    AudioTracks, Command, Io, RecordingId, TakeInfo, CHUNK, MAX_TAKES, MAX_TRACKS, PEAK_BLOCK,
};

/// Recording to files: samples copied out of the tracks' memory in one
/// period at most: 64 times what 8 stereo tracks record in a 128-frame
/// period would need a quarter of, a small, steady cost on the audio
/// thread.
pub const COPY_SAMPLES: usize = 8192;
/// Recording to files: buffers of [`CHUNK`] samples kept for copying takes
/// out to the worker (made once; each comes back once the writer has its
/// frames).
const SPARE: usize = 32;
/// Events for the log waiting to be taken, at most.
const EVENTS: usize = 64;

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

enum ToWorker {
    /// Room for a save (`Saved::room`), for the audio thread to fill.
    Room,
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
        take: TakeInfo,
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
    /// Record into `dir` from now on.
    Files {
        dir: PathBuf,
        cfg: WriterConfig,
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
}

enum FromWorker {
    Room(Saved),
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
    Files(Said),
    /// A take started with no file: why.
    NoFile {
        rec: RecordingId,
        why: String,
    },
    /// Recording to files couldn't start: why.
    FilesFailed(String),
    /// The stream layer's player, for the tracks.
    Player(Box<Player>),
    /// The stream layer couldn't start: why.
    StreamFailed(String),
    /// What came of opening a take's file (from the stream thread).
    Opened(Opened),
    /// A waveform read from a take's file.
    WaveFile(TakeWave),
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

/// The audio tracks with their worker; see the module notes.
pub struct AudioHost {
    pub tracks: AudioTracks,
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
    /// A sound brought in ([`AudioHost::import`]) copying in: its take,
    /// placed once it is in, and where it is cut (kept, not freed, here).
    importing: Option<u16>,
    import_cuts: Vec<u64>,
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
    /// take, made once; a take here holds no audio, name or path).
    opening: Vec<TakeFile>,
    /// That project's tracks (its takes taken out), waiting until every
    /// take has come in or been left out: then they go in, without the
    /// regions of a take left out, so no region ever plays a take that
    /// won't come.
    waiting: Option<Saved>,
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
        let spawned = std::thread::Builder::new()
            .name("mpfree-audiotracks".into())
            .spawn(move || work(rate, &jobs, &done, &back))
            .is_ok();
        AudioHost {
            tracks: AudioTracks::new(rate, seconds),
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
            },
            fits: 0,
            fits_failed: 0,
            importing: None,
            import_cuts: Vec::new(),
            imports: 0,
            generation: 0,
            stale: 0,
            streaming: false,
            opening: Vec::with_capacity(MAX_TAKES),
            waiting: None,
        }
    }

    /// Play takes from their files from now on (see the module notes and
    /// [`crate::streaming`]): the worker starts the stream layer with
    /// `cfg`, and the tracks get its player a moment later. Call it once,
    /// at start, before [`AudioHost::record_to`], so the writer waits for
    /// playback's reads. False (nothing changed) once files are on, if it
    /// is on already, or with no worker. Does not allocate.
    pub fn stream(&mut self, cfg: StreamConfig) -> bool {
        if self.tx.is_none() || self.streaming || self.tracks.records_to_files() {
            return false;
        }
        self.streaming = true;
        self.send(ToWorker::Stream(Box::new(cfg)));
        true
    }

    /// Whether takes play from their files (asked for: the player may be
    /// on its way).
    pub fn streams(&self) -> bool {
        self.streaming
    }

    /// Takes of a project opening still waiting for their files to open.
    pub fn opening(&self) -> usize {
        self.opening.len()
    }

    /// Record every take to a file of its own in `dir` (made if missing;
    /// the library's work folder for the audio tracks), from the next
    /// armed track on (see [`crate::files`]). Call it once, at start,
    /// after anything a cut-off run left there is recovered
    /// ([`crate::files::recover_into`]). Does not allocate (the message
    /// carries `dir`). Refused, with `dir` given back to try again later,
    /// while a track records or a take is still being written, or with no
    /// worker.
    pub fn record_to(&mut self, dir: PathBuf, cfg: WriterConfig) -> Result<(), PathBuf> {
        if self.tx.is_none() || !self.tracks.files(true) {
            return Err(dir);
        }
        self.send(ToWorker::Files { dir, cfg });
        Ok(())
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

    fn send(&self, m: ToWorker) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
    }

    pub fn command(&mut self, c: Command) -> bool {
        self.tracks.command(c)
    }

    /// Run the tracks for one period (see [`AudioTracks::process`]), then
    /// look after the save, the fit and the buffers.
    pub fn period(&mut self, block: &Block, map: &TempoMap, input: &[f32], out: &mut [f32]) {
        self.tracks.process(block, map, input, out);
        self.tend_import(map);
        self.tend(block.frames as usize);
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
                self.opening.push(k);
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
                }
            }
        }
        if self.opening.is_empty() {
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
        let file = TakeFile {
            id,
            // A sound brought in is a new recording: the tracks give its id.
            rec: crate::RecordingId::NONE,
            track,
            group,
            pass: 0,
            bpm,
            anchor: at as f64,
            frames,
            channels,
            file: String::new(),
            path: None,
            audio,
        };
        match self.tracks.load_take(file) {
            Ok(()) => {
                self.importing = Some(id);
                self.import_cuts = cuts;
                Ok(id)
            }
            Err(f) => refuse(self, f.audio, ImportRefused::NoRoom),
        }
    }

    /// The take of a sound brought in that is still copying in.
    pub fn importing(&self) -> Option<u16> {
        self.importing
    }

    /// A sound brought in that has copied in goes on its track.
    fn tend_import(&mut self, map: &TempoMap) {
        let Some(id) = self.importing else {
            return;
        };
        if self.tracks.take(id).is_none() {
            // Gone (the tracks were cleared meanwhile).
            self.importing = None;
        } else if self.tracks.loading() == 0 {
            self.importing = None;
            if self.tracks.place(id, &self.import_cuts, map) {
                self.imports += 1;
            }
            self.import_cuts.clear();
        }
    }

    /// Fit take `id` to `bpm` (the tempo now), keeping its pitch. False if
    /// a fit is under way, the take is not there or records, its tempo is
    /// not known or already `bpm`, or there is no worker.
    pub fn fit(&mut self, id: u16, bpm: f64) -> bool {
        if self.fitting.is_some() || self.tx.is_none() {
            return false;
        }
        let Some(take) = self.tracks.take(id).filter(|t| !t.recording) else {
            return false;
        };
        let Some(ratio) = fit_ratio(take.bpm, bpm).filter(|r| (r - 1.0).abs() > 1e-9) else {
            return false;
        };
        if self.tracks.on_file(id) {
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

    /// Whether `take` is still there, the same recording, now on its file
    /// (it moved while it was read from memory).
    fn moved_to_file(&self, take: u16, rec: RecordingId) -> bool {
        self.tracks.on_file(take) && self.tracks.take(take).is_some_and(|t| t.rec == rec)
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
        match self.tracks.take(id).filter(|_| self.tracks.on_file(id)) {
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
        if let Some(w) = self.waiting.take() {
            self.send(ToWorker::DropSaved(w));
        }
        self.send(ToWorker::Clear {
            gen: self.generation,
        });
        if self.streaming {
            self.send(ToWorker::Project);
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
                FromWorker::Files(said) => self.heard(said),
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
                FromWorker::Opened(o) => self.opened(o),
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
        self.tend_wave(WAVE_SPEED * frames.max(1));
    }

    /// What came of opening a take's file: a project's take comes in as
    /// its file, a recorded one moves to it; a file no take wants any
    /// more is closed.
    fn opened(&mut self, o: Opened) {
        match o {
            // For tracks since emptied: its source goes.
            Opened::Open { era, take, gen, .. } if era != self.generation => {
                self.send(ToWorker::Close { take, gen });
            }
            Opened::Failed { era, .. } if era != self.generation => {}
            Opened::Open { take, rec, gen, .. } => {
                if let Some(i) = self
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
                } else if self
                    .tracks
                    .take(take)
                    .is_some_and(|t| t.rec == rec && !t.recording)
                    && !self.tracks.on_file(take)
                {
                    self.tracks.command(Command::OnFile { take, rec, gen });
                } else {
                    self.send(ToWorker::Close { take, gen });
                }
            }
            Opened::Failed { take, rec, why, .. } => {
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

    /// A project's tracks go in once none of its takes is still opening:
    /// without the regions of the takes left out.
    fn tracks_in(&mut self) {
        if !self.opening.is_empty() {
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

    /// Recording to files: the tracks' requests to the worker, and the
    /// takes recording copied out to it, at most [`COPY_SAMPLES`] samples
    /// a period, a buffer at a time. A take copied out in full once it has
    /// stopped is finished, and can be forgotten again.
    fn tend_files(&mut self) {
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
                    let _ = self.tx.as_ref().map(|tx| {
                        tx.send(ToWorker::Push {
                            rec: o.rec,
                            buf,
                            len: full,
                        })
                    });
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
        if self.tracks.on_file(id) {
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
                        self.tracks.on_file(t.id)
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

    fn tend_save(&mut self, budget: usize) {
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
                let filled = if self.files_settled() && self.opening.is_empty() {
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
                                        take,
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
                    self.tracks.command(swap);
                    self.fits += 1;
                } else {
                    self.fitting = Some(Fitting::Loading { new, swap });
                }
            }
            other => self.fitting = other,
        }
    }
}

/// The worker thread: every big buffer made and dropped here, stretches,
/// and the files (see the module notes). It wakes at least every 20 ms
/// while the writer runs, to see how its takes stand. `back` is its own
/// channel, for the stream thread's whole-file reads.
fn work(rate: u32, jobs: &Receiver<ToWorker>, done: &Sender<FromWorker>, back: &Sender<ToWorker>) {
    let mut w = Worker {
        rate,
        files: Files::new(rate),
        stream: None,
        wants: Vec::new(),
        next_job: 1,
        era: 0,
    };
    loop {
        let m = if w.files.writing() {
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
        w.open_finished();
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
    /// Whole-file reads asked of the stream thread, and what each is for.
    wants: Vec<(u32, Want)>,
    next_job: u32,
    /// The tracks' generation (as [`ToWorker::Clear`] last said): each
    /// file opened carries it, so an answer for tracks since emptied is
    /// told apart.
    era: u32,
}

impl Worker {
    /// Take files finished (recorded, or a project's) go to the stream
    /// thread to open.
    fn open_finished(&mut self) {
        let Some(st) = &self.stream else {
            return;
        };
        for (take, rec, path, channels, frames) in self.files.take_opens() {
            let _ = st.tx.send(ToStream::Open {
                era: self.era,
                take,
                rec,
                path,
                channels,
                frames,
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
                let out = StretchJob::new(take, audio, rate, ratio, bpm, id)
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
            ToWorker::Files { dir, cfg } => {
                let gate = self
                    .stream
                    .as_ref()
                    .map_or_else(|| Arc::new(Gate::new()), |st| st.gate.clone());
                match files.start(dir, cfg, gate) {
                    Ok(()) => return None,
                    Err(e) => FromWorker::FilesFailed(e.to_string()),
                }
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
                let opened = {
                    let done = done.clone();
                    move |o| done.send(FromWorker::Opened(o)).is_ok()
                };
                let read = {
                    let back = back.clone();
                    move |r| back.send(ToWorker::Read(r)).is_ok()
                };
                match streaming::start(rate, *cfg, opened, read) {
                    Ok((player, st)) => {
                        files.stream();
                        self.stream = Some(st);
                        FromWorker::Player(player)
                    }
                    Err(e) => FromWorker::StreamFailed(e.to_string()),
                }
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
