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
//!
//! The channels to the worker are `std::sync::mpsc`: a message now and
//! then can allocate a block of the channel, as in the looper's host. A
//! period with nothing under way sends nothing.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use mpfree_transport::time::TempoMap;
use mpfree_transport::{Block, Tick};

pub use crate::disk::DiskStats;
use crate::disk::{Disk, FromDisk, ToDisk};
use crate::save::{Saved, TakeFile};
use crate::stretch::{fit_ratio, StretchJob};
use crate::{
    AudioTracks, Command, Io, SaveState, TakeInfo, CHUNK, IO_QUEUE, MAX_TAKES, PEAK_BLOCK,
};

/// Streaming: samples copied into or out of the tracks' memory in one
/// period at most (a chunk takes 4 periods): about 94 chunks a second at
/// 128-frame periods, against about 60 for 8 stereo tracks recording while
/// 8 more play, and a small, steady cost on the audio thread.
pub const COPY_SAMPLES: usize = 8192;
/// Streaming: chunk buffers kept for the disk thread (made once).
const SPARE: usize = 32;
/// Streaming: a fit holds the whole take in memory while it stretches, so
/// takes longer than this many samples (about 6 minutes of stereo at
/// 44.1 kHz) are refused.
pub const FIT_MAX_SAMPLES: usize = 1 << 25;

/// A chunk copying into the tracks (read from the disk) or out of them
/// (to be written), a part a period.
enum Job {
    Fill {
        take: u16,
        chunk: u32,
        buf: Vec<f32>,
        len: usize,
        done: usize,
    },
    Out {
        take: u16,
        chunk: u32,
        channels: usize,
        buf: Vec<f32>,
        len: usize,
        done: usize,
    },
}

/// The disk thread and what goes between it and the tracks.
struct Streaming {
    tx: Sender<ToDisk>,
    rx: Receiver<FromDisk>,
    spare: Vec<Vec<f32>>,
    /// Writes and finishes, in order; reads to send; read chunks to copy
    /// in; the copy under way.
    waiting: VecDeque<Io>,
    reads: VecDeque<(u16, u32)>,
    fills: VecDeque<(u16, u32, Vec<f32>, usize)>,
    job: Option<Job>,
    /// Each take's file name.
    names: Vec<Option<String>>,
    stamp: u64,
    next: u32,
    /// Writes and finishes not answered yet; saved takes being opened.
    in_flight: usize,
    opening: usize,
    stats: Arc<DiskStats>,
    dir: PathBuf,
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
    /// A filled room: trimmed to its first `tracks` and `takes`, and, with
    /// `audio`, given room for every take's audio.
    Size {
        saved: Saved,
        tracks: usize,
        takes: usize,
        audio: bool,
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
}

enum FromWorker {
    Room(Saved),
    Sized(Saved),
    Buffer {
        audio: Vec<f32>,
        gen: u32,
    },
    Stretched {
        out: Option<(crate::save::TakeFile, Command)>,
        gen: u32,
    },
    Wave(Vec<(f32, f32)>),
}

enum Waving {
    /// Waiting for room for `columns` columns.
    Sizing { id: u16, columns: usize },
    /// Column `at` is next.
    Reading { wave: TakeWave, at: usize },
    /// Streaming: the disk thread makes it from its table.
    Disk { id: u16 },
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
    /// Streaming: the take's audio is coming from its file.
    Fetching {
        take: TakeInfo,
        ratio: f64,
        bpm: f64,
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
    stream: Option<Streaming>,
    /// Streaming: saved takes that could not be opened (why, one a take).
    pub open_errors: Vec<String>,
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
    /// Streaming: whole takes asked of the disk for a fit before the
    /// tracks were emptied, whose answers are dropped as they come (the
    /// disk answers each, in order).
    stale_reads: u32,
    /// Background results dropped because the tracks were emptied after
    /// they were asked for.
    pub stale: u64,
}

/// Why [`AudioHost::import`] could not take a sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRefused {
    /// Another sound is still copying in.
    Busy,
    /// Streaming from take files: a sound would need a file of its own.
    Streaming,
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
        let spawned = std::thread::Builder::new()
            .name("mpfree-audiotracks".into())
            .spawn(move || {
                for m in jobs {
                    let reply = match m {
                        ToWorker::Room => FromWorker::Room(Saved::room(rate)),
                        ToWorker::Size {
                            mut saved,
                            tracks,
                            takes,
                            audio,
                        } => {
                            saved.tracks.truncate(tracks);
                            saved.takes.truncate(takes);
                            if audio {
                                for k in &mut saved.takes {
                                    k.audio = vec![0.0; k.frames * k.channels];
                                }
                            }
                            FromWorker::Sized(saved)
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
                            continue;
                        }
                        ToWorker::DropSaved(s) => {
                            drop(s);
                            continue;
                        }
                        ToWorker::DropWave(w) => {
                            drop(w);
                            continue;
                        }
                    };
                    if done.send(reply).is_err() {
                        break;
                    }
                }
            })
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
            stream: None,
            open_errors: Vec::new(),
            fits: 0,
            fits_failed: 0,
            importing: None,
            import_cuts: Vec::new(),
            imports: 0,
            generation: 0,
            stale_reads: 0,
            stale: 0,
        }
    }

    /// The tracks streaming from take files in `dir` (made if missing; on
    /// the MPC, a folder on the stick's data partition), with `seconds` of
    /// stereo memory in front of it (see the module notes of
    /// [`crate`]), the worker thread and the disk thread
    /// (`mpfree-audiotracks-disk`).
    pub fn streaming(rate: u32, seconds: u32, dir: PathBuf) -> std::io::Result<AudioHost> {
        std::fs::create_dir_all(&dir)?;
        let mut h = AudioHost::new(rate, seconds);
        h.tracks.stream(true);
        let stats = Arc::new(DiskStats::default());
        let (tx, jobs) = channel::<ToDisk>();
        let (done, rx) = channel();
        let disk = Disk::new(dir.clone(), rate, stats.clone());
        std::thread::Builder::new()
            .name("mpfree-audiotracks-disk".into())
            .spawn(move || disk.serve(jobs, done))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        h.stream = Some(Streaming {
            tx,
            rx,
            spare: (0..SPARE).map(|_| vec![0.0; CHUNK]).collect(),
            waiting: VecDeque::with_capacity(IO_QUEUE),
            reads: VecDeque::with_capacity(IO_QUEUE),
            fills: VecDeque::with_capacity(SPARE),
            job: None,
            names: vec![None; MAX_TAKES],
            stamp,
            next: 0,
            in_flight: 0,
            opening: 0,
            stats,
            dir,
        });
        Ok(h)
    }

    /// Streaming: the disk thread's counters (reads, writes, errors, the
    /// longest call).
    pub fn disk_stats(&self) -> Option<&DiskStats> {
        self.stream.as_ref().map(|s| &*s.stats)
    }

    /// Streaming: the folder the take files are in, and take `id`'s file.
    pub fn take_file(&self, id: u16) -> Option<PathBuf> {
        let st = self.stream.as_ref()?;
        Some(st.dir.join(st.names.get(id as usize)?.as_ref()?))
    }

    /// Streaming: saved takes still being opened.
    pub fn opening(&self) -> usize {
        self.stream.as_ref().map_or(0, |s| s.opening)
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
    /// back are listed in the error; the rest are in. Any audio not taken
    /// goes to the worker to drop.
    pub fn load(&mut self, mut saved: Saved) -> Result<(), String> {
        if let Some(st) = &mut self.stream {
            // Streaming: each take's file is opened and checked on the disk
            // thread, then the take is put back (see `opening`;
            // failures land in `open_errors`).
            for mut k in saved.takes.drain(..) {
                k.audio = Vec::new();
                if let Some(n) = st.names.get_mut(k.id as usize) {
                    *n = Some(k.file.clone());
                }
                st.opening += 1;
                let _ = st.tx.send(ToDisk::Open { file: k });
            }
            let mut bad = Vec::new();
            for t in &saved.tracks {
                if !self.tracks.load_track(t) {
                    bad.push(format!("track {}", t.index + 1));
                }
            }
            self.send(ToWorker::DropSaved(saved));
            return if bad.is_empty() {
                Ok(())
            } else {
                Err(format!("audio tracks: could not load {}", bad.join(", ")))
            };
        }
        // Only a refusal allocates (its line); what is left of the part
        // goes to the worker to drop.
        let mut bad = Vec::new();
        for k in saved.takes.drain(..) {
            if let Err(k) = self.tracks.load_take(k) {
                bad.push(format!("take {}", k.id));
                self.send(ToWorker::Drop(k.audio));
            }
        }
        for t in &saved.tracks {
            if !self.tracks.load_track(t) {
                bad.push(format!("track {}", t.index + 1));
            }
        }
        self.send(ToWorker::DropSaved(saved));
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
        if self.stream.is_some() {
            return refuse(self, audio, ImportRefused::Streaming);
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
    /// not known or already `bpm`, or there is no worker; streaming, also
    /// until all of it is on the disk ([`SaveState::Saved`]), as the fit
    /// reads it from there.
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
        if let Some(st) = &self.stream {
            if take.frames * take.channels > FIT_MAX_SAMPLES
                || self.tracks.save_state(id) != Some(SaveState::Saved)
            {
                return false;
            }
            let _ = st.tx.send(ToDisk::ReadAll {
                take: id,
                frames: take.frames,
                channels: take.channels,
            });
            self.fitting = Some(Fitting::Fetching { take, ratio, bpm });
            return true;
        }
        self.send(ToWorker::Buffer {
            n: take.frames * take.channels,
            gen: self.generation,
        });
        self.fitting = Some(Fitting::Sizing { take, ratio, bpm });
        true
    }

    /// The take being fitted, while a fit is under way.
    pub fn fitting(&self) -> Option<u16> {
        match self.fitting.as_ref()? {
            Fitting::Sizing { take, .. }
            | Fitting::Reading { take, .. }
            | Fitting::Stretching { take }
            | Fitting::Fetching { take, .. } => Some(take.id),
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
        if let Some(st) = &self.stream {
            let t = self.tracks.take(id).expect("checked above");
            let _ = st.tx.send(ToDisk::Wave {
                take: id,
                frames: t.frames,
                channels: t.channels,
                columns,
            });
            self.waving = Some(Waving::Disk { id });
            return true;
        }
        self.send(ToWorker::Wave(columns));
        self.waving = Some(Waving::Sizing { id, columns });
        true
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
    /// take copies in or audio is still going to the disk), for New
    /// project or a project opening. The background jobs asked for until
    /// now are the old project's: a fit under way stops, and whatever the
    /// worker or the disk still sends for it is dropped as it comes
    /// ([`AudioHost::stale`]), so it can never reach the new project's
    /// takes (a take id comes back after a clear). A sound still being
    /// brought in goes with the rest. Does not allocate.
    pub fn clear(&mut self) -> bool {
        if !self.tracks.clear() {
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        match self.fitting.take() {
            Some(Fitting::Reading { audio, .. }) => self.send(ToWorker::Drop(audio)),
            Some(Fitting::Fetching { .. }) => self.stale_reads += 1,
            // Sizing and Stretching: the worker's answer carries the old
            // generation; Loading: its take went with the rest.
            Some(Fitting::Sizing { .. } | Fitting::Stretching { .. } | Fitting::Loading { .. })
            | None => {}
        }
        self.importing = None;
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
        self.tend_disk();
        while let Some(m) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                FromWorker::Room(saved) | FromWorker::Sized(saved) if self.cancelled > 0 => {
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
                // Streaming: the audio is in the take files already, and
                // the save is done.
                FromWorker::Sized(saved) if self.stream.is_some() => {
                    self.saving = None;
                    if let Some(old) = self.saved.replace(saved) {
                        self.send(ToWorker::DropSaved(old));
                    }
                }
                FromWorker::Sized(saved) => {
                    self.saving = Some(Saving::Reading {
                        saved,
                        i: 0,
                        frame: 0,
                    });
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
        let budget = READ_SPEED * frames.max(1);
        self.tend_save(budget);
        self.tend_fit(budget);
        self.tend_wave(WAVE_SPEED * frames.max(1));
    }

    /// Streaming: answers from the disk thread into the tracks, and the
    /// tracks' requests out to it, copying at most [`COPY_SAMPLES`] samples
    /// a period.
    fn tend_disk(&mut self) {
        let Some(st) = self.stream.as_mut() else {
            return;
        };
        let mut audio = None;
        let mut disk_wave = None;
        while let Ok(m) = st.rx.try_recv() {
            match m {
                FromDisk::Written {
                    take,
                    chunk,
                    buf,
                    ok,
                } => {
                    st.in_flight -= 1;
                    if ok {
                        self.tracks.written(take, chunk);
                    } else {
                        self.tracks.write_failed(take, chunk);
                    }
                    st.spare.push(buf);
                }
                FromDisk::Read {
                    take,
                    chunk,
                    buf,
                    got,
                } => match got {
                    Some(len) if st.fills.len() < st.fills.capacity() => {
                        st.fills.push_back((take, chunk, buf, len));
                    }
                    _ => {
                        self.tracks.fill_failed(take, chunk);
                        st.spare.push(buf);
                    }
                },
                FromDisk::Finished { take, ok } => {
                    st.in_flight -= 1;
                    self.tracks.finished(take, ok);
                }
                FromDisk::Opened { file, ok } => {
                    st.opening -= 1;
                    let id = file.id;
                    let r = ok.and_then(|()| {
                        self.tracks
                            .add_take(file)
                            .map_err(|f| format!("take {}: could not be put back", f.id))
                    });
                    if let Err(e) = r {
                        if let Some(n) = st.names.get_mut(id as usize) {
                            *n = None;
                        }
                        self.open_errors.push(e);
                    }
                }
                FromDisk::Audio { take, audio: a } => audio = Some((take, a)),
                FromDisk::Wave(w) => disk_wave = Some(w),
            }
        }
        // The tracks' requests: reads go out at once (as buffers allow);
        // writes and finishes keep their order.
        while st.waiting.len() < st.waiting.capacity() && st.reads.len() < st.reads.capacity() {
            match self.tracks.take_io() {
                Some(Io::Read { take, chunk }) => st.reads.push_back((take, chunk)),
                Some(r) => st.waiting.push_back(r),
                None => break,
            }
        }
        while !st.reads.is_empty() && !st.spare.is_empty() {
            let (take, chunk) = st.reads.pop_front().expect("checked");
            let buf = st.spare.pop().expect("checked");
            let _ = st.tx.send(ToDisk::Read { take, chunk, buf });
        }
        // Copies, read chunks first.
        let mut budget = COPY_SAMPLES;
        loop {
            if st.job.is_none() {
                if let Some((take, chunk, buf, len)) = st.fills.pop_front() {
                    st.job = Some(Job::Fill {
                        take,
                        chunk,
                        buf,
                        len,
                        done: 0,
                    });
                } else {
                    match st.waiting.front().copied() {
                        Some(io @ (Io::Write { take, chunk } | Io::Rewrite { take, chunk })) => {
                            if budget == 0 || st.spare.is_empty() {
                                break;
                            }
                            st.waiting.pop_front();
                            if matches!(io, Io::Write { chunk: 0, .. }) {
                                // A new take: a new file, never one a save
                                // names. (A write tried again goes to the
                                // file the take has.)
                                let name = format!("{}-{}.wav", st.stamp, st.next);
                                st.next += 1;
                                st.names[take as usize] = Some(name.clone());
                                let _ = st.tx.send(ToDisk::Name { take, name });
                            }
                            let Some(len) = self.tracks.chunk(take, chunk).map(<[f32]>::len) else {
                                continue;
                            };
                            st.job = Some(Job::Out {
                                take,
                                chunk,
                                channels: self.tracks.take(take).map_or(1, |t| t.channels),
                                buf: st.spare.pop().expect("checked"),
                                len,
                                done: 0,
                            });
                        }
                        Some(Io::Finish { take }) => {
                            st.waiting.pop_front();
                            if let Some(t) = self.tracks.take(take) {
                                st.in_flight += 1;
                                let _ = st.tx.send(ToDisk::Finish {
                                    take,
                                    frames: t.frames,
                                    channels: t.channels,
                                });
                            }
                            continue;
                        }
                        Some(Io::Read { .. }) | None => break,
                    }
                }
            }
            if budget == 0 {
                break;
            }
            match st.job.take() {
                Some(Job::Fill {
                    take,
                    chunk,
                    buf,
                    len,
                    done,
                }) => {
                    let k = budget.min(len - done);
                    let last = done + k == len;
                    let kept = self
                        .tracks
                        .fill_part(take, chunk, done, &buf[done..done + k], last);
                    budget -= k;
                    if kept && !last {
                        st.job = Some(Job::Fill {
                            take,
                            chunk,
                            buf,
                            len,
                            done: done + k,
                        });
                    } else {
                        st.spare.push(buf);
                    }
                }
                Some(Job::Out {
                    take,
                    chunk,
                    channels,
                    mut buf,
                    len,
                    done,
                }) => {
                    let Some(data) = self.tracks.chunk(take, chunk) else {
                        st.spare.push(buf);
                        continue;
                    };
                    let k = budget.min(len - done);
                    buf[done..done + k].copy_from_slice(&data[done..done + k]);
                    budget -= k;
                    if done + k == len {
                        st.in_flight += 1;
                        let _ = st.tx.send(ToDisk::Write {
                            take,
                            chunk,
                            channels,
                            len,
                            data: buf,
                        });
                    } else {
                        st.job = Some(Job::Out {
                            take,
                            chunk,
                            channels,
                            buf,
                            len,
                            done: done + k,
                        });
                    }
                }
                None => break,
            }
        }
        if let Some((_, a)) = audio.as_mut().filter(|_| self.stale_reads > 0) {
            // A fit's read asked for before the tracks were emptied.
            self.stale_reads -= 1;
            self.stale += 1;
            if let Some(a) = a.take() {
                self.send(ToWorker::Drop(a));
            }
            audio = None;
        }
        if let Some((take, a)) = audio {
            match (self.fitting.take(), a) {
                (
                    Some(Fitting::Fetching {
                        take: t,
                        ratio,
                        bpm,
                    }),
                    Some(audio),
                ) if t.id == take => match self.tracks.free_take_id() {
                    Some(id) => {
                        self.send(ToWorker::Stretch {
                            take: t,
                            audio,
                            ratio,
                            bpm,
                            id,
                            gen: self.generation,
                        });
                        self.fitting = Some(Fitting::Stretching { take: t });
                    }
                    None => {
                        self.fits_failed += 1;
                        self.send(ToWorker::Drop(audio));
                    }
                },
                (Some(Fitting::Fetching { .. }), _) => self.fits_failed += 1,
                (other, a) => {
                    self.fitting = other;
                    if let Some(a) = a {
                        self.send(ToWorker::Drop(a));
                    }
                }
            }
        }
        if let Some(w) = disk_wave {
            if let Some(Waving::Disk { id }) = self.waving.take() {
                if let Some(mut w) = w.filter(|w| w.id == id) {
                    // The part not on the disk yet (a recording's
                    // end) from the tracks' own table.
                    let fpc = w.frames_per_column;
                    let mut at = w.columns.len() * fpc;
                    while at < w.frames && w.columns.len() < w.columns.capacity() {
                        let to = (at + fpc).min(w.frames);
                        match self.tracks.peaks(id, at, to) {
                            Some(p) => w.columns.push(p),
                            None => break,
                        }
                        at = to;
                    }
                    if let Some(old) = self.wave.replace(w) {
                        self.send(ToWorker::DropWave(old));
                    }
                }
            }
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
                    // Forgotten while it was read.
                    self.send(ToWorker::DropWave(wave));
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

    fn tend_save(&mut self, budget: usize) {
        match self.saving.take() {
            None => {}
            Some(Saving::Asking) => self.saving = Some(Saving::Asking),
            Some(Saving::Waiting(mut room)) => {
                // Settled first, and only then filled, into the worker's
                // room: nothing is made here, nor while a take records,
                // loads or is still being written to its file.
                let written = self.stream.as_ref().is_none_or(|st| {
                    st.waiting.is_empty()
                        && st.job.is_none()
                        && st.in_flight == 0
                        && st.opening == 0
                        && self.tracks.unwritten() == 0
                        && self.tracks.loading() == 0
                });
                let filled = if written {
                    self.tracks.save_into(&mut room)
                } else {
                    None
                };
                match filled {
                    None => self.saving = Some(Saving::Waiting(room)),
                    Some((tracks, takes)) => {
                        // Streaming: each take's file, into its room.
                        if let Some(st) = &self.stream {
                            for k in &mut room.takes[..takes] {
                                if let Some(Some(name)) = st.names.get(k.id as usize) {
                                    k.file.push_str(name);
                                }
                            }
                        }
                        self.send(ToWorker::Size {
                            saved: room,
                            tracks,
                            takes,
                            audio: self.stream.is_none(),
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
