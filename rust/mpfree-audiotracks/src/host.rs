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

use std::sync::mpsc::{channel, Receiver, Sender};

use mpfree_transport::time::TempoMap;
use mpfree_transport::Block;

use crate::save::Saved;
use crate::stretch::{fit_ratio, StretchJob};
use crate::{AudioTracks, Command, TakeInfo, PEAK_BLOCK};

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
    fitting: Option<Fitting>,
    waving: Option<Waving>,
    wave: Option<TakeWave>,
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
            fitting: None,
            waving: None,
            wave: None,
            fits: 0,
            fits_failed: 0,
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

    /// Put a saved part back: every take (copying in over the next
    /// periods) and every track. Takes or tracks that could not be put
    /// back are listed in the error; the rest are in. Any audio not taken
    /// goes to the worker to drop.
    pub fn load(&mut self, mut saved: Saved) -> Result<(), String> {
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
        self.send(ToWorker::Buffer(take.frames * take.channels));
        self.fitting = Some(Fitting::Sizing { take, ratio, bpm });
        true
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
        self.send(ToWorker::Wave(columns));
        self.waving = Some(Waving::Sizing { id, columns });
        true
    }

    /// A finished waveform.
    pub fn take_wave(&mut self) -> Option<TakeWave> {
        self.wave.take()
    }

    fn tend(&mut self, frames: usize) {
        while let Some(v) = self.tracks.take_returned() {
            self.send(ToWorker::Drop(v));
        }
        while let Some(m) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
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
