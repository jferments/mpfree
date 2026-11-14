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
use mpfree_transport::Block;

pub use crate::disk::DiskStats;
use crate::disk::{Disk, FromDisk, ToDisk};
use crate::save::Saved;
use crate::stretch::{fit_ratio, StretchJob};
use crate::{AudioTracks, Command, Io, TakeInfo, CHUNK, IO_QUEUE, MAX_TAKES, PEAK_BLOCK};

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
    /// Room for every take's audio in a save.
    Size(Saved),
    /// A zeroed buffer of this many samples (a take to fit).
    Buffer(usize),
    Stretch {
        take: TakeInfo,
        audio: Vec<f32>,
        ratio: f64,
        bpm: f64,
        id: u16,
    },
    /// Room for a waveform of this many columns.
    Wave(usize),
    Drop(Vec<f32>),
    DropSaved(Saved),
    DropWave(TakeWave),
}

enum FromWorker {
    Sized(Saved),
    Buffer(Vec<f32>),
    Stretched(Option<(crate::save::TakeFile, Command)>),
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
    /// Until nothing records or loads.
    Waiting,
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
                        ToWorker::Size(mut saved) => {
                            for k in &mut saved.takes {
                                k.audio = vec![0.0; k.frames * k.channels];
                            }
                            FromWorker::Sized(saved)
                        }
                        ToWorker::Buffer(n) => FromWorker::Buffer(vec![0.0; n]),
                        ToWorker::Stretch {
                            take,
                            audio,
                            ratio,
                            bpm,
                            id,
                        } => {
                            let out = StretchJob::new(take, audio, rate, ratio, bpm, id)
                                .ok()
                                .and_then(|mut j| {
                                    while !j.run(1 << 15).finished {}
                                    j.finish()
                                });
                            FromWorker::Stretched(out)
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
        self.tend(block.frames as usize);
    }

    /// Start saving the audio tracks; the result comes in
    /// [`AudioHost::take_saved`]. False if a save is already under way or
    /// there is no worker.
    pub fn save(&mut self) -> bool {
        if self.saving.is_some() || self.tx.is_none() {
            return false;
        }
        self.saving = Some(Saving::Waiting);
        true
    }

    /// How far the save has got (0 to 1), while one is under way.
    pub fn saving(&self) -> Option<f32> {
        match &self.saving {
            None => None,
            Some(Saving::Waiting | Saving::Sizing) => Some(0.0),
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
            Some(Saving::Reading { saved, .. }) => self.send(ToWorker::DropSaved(saved)),
            Some(Saving::Sizing) => self.cancelled = self.cancelled.saturating_add(1),
            Some(Saving::Waiting) | None => {}
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
        if let Some(st) = &self.stream {
            if take.frames * take.channels > FIT_MAX_SAMPLES {
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
        self.send(ToWorker::Buffer(take.frames * take.channels));
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
                FromWorker::Sized(saved) if self.cancelled > 0 => {
                    self.cancelled -= 1;
                    self.send(ToWorker::DropSaved(saved));
                }
                FromWorker::Sized(saved) => {
                    self.saving = Some(Saving::Reading {
                        saved,
                        i: 0,
                        frame: 0,
                    });
                }
                FromWorker::Buffer(audio) => match self.fitting.take() {
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
                FromWorker::Stretched(out) => {
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
                FromDisk::Finished { ok } => {
                    st.in_flight -= 1;
                    if !ok {
                        self.tracks.write_failed(u16::MAX, 0);
                    }
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
                        Some(Io::Write { take, chunk }) => {
                            if budget == 0 || st.spare.is_empty() {
                                break;
                            }
                            st.waiting.pop_front();
                            if chunk == 0 {
                                // A new take: a new file, never one a save
                                // names.
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
                        _ => break,
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
            Some(Saving::Waiting) if self.stream.is_some() => {
                let st = self.stream.as_ref().expect("streaming");
                let settled = st.waiting.is_empty()
                    && st.job.is_none()
                    && st.in_flight == 0
                    && st.opening == 0
                    && self.tracks.unwritten() == 0
                    && self.tracks.loading() == 0;
                match self.tracks.save_file().filter(|_| settled) {
                    Some(mut s) => {
                        // The audio is in the take files already.
                        for k in &mut s.takes {
                            k.file = st.names[k.id as usize].clone().unwrap_or_default();
                        }
                        if let Some(old) = self.saved.replace(s) {
                            self.send(ToWorker::DropSaved(old));
                        }
                    }
                    None => self.saving = Some(Saving::Waiting),
                }
            }
            Some(Saving::Waiting) => match self.tracks.save_file() {
                Some(s) => {
                    self.send(ToWorker::Size(s));
                    self.saving = Some(Saving::Sizing);
                }
                None => self.saving = Some(Saving::Waiting),
            },
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
