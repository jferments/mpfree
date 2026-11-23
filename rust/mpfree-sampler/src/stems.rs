//! Stems as the sampler makes them (the stems plan's step 4): a sound split
//! into vocals, drums, bass and other by `mpfree-stems`, run on the job
//! service's sound lane ([`crate::jobs`]).
//!
//! A split of a minute's sound takes about a minute on the MPC, and the
//! sound lane has one worker, so a split run as one job would hold up
//! every other sound's view, hits and octave copies for that long. So
//! [`Split`] runs it **one step a job**: one segment's STFT, one stem's
//! network (about 1.6 s on the MPC) or the segment's masks. Each job ends
//! by putting the next at the back of the queue, the separation moving
//! with it, so a job for the sound being shown ([`crate::jobs::Order::First`])
//! waits at most one step. Every step has the split's [`Key`], so a second
//! split of the same sound takes the first one's place.
//!
//! The memory is promised once, for the whole split, by the caller (the
//! app's shared memory check), and held until the split ends however it
//! ends: the steps themselves ask the service for nothing, so a split
//! never runs out of memory halfway.
//!
//! [`Split::stop`] stops it between steps; nothing it made is kept. The
//! stems come back as floats at the sound's 44.1 kHz, or, for a 16-bit
//! sound, rounded to 16 bits with triangular dither ([`to_16_bits`]), so
//! [`crate::model::Pcm::new`] holds them as 16 bits, at half the memory:
//! a stem can't be finer than its sound.
//!
//! While it runs, [`Split::peaks`] is each stem's picture so far, a column
//! at a time, for the Stems page's lanes to fill as the stems come.
//!
//! The app hands it the sound and where the model file is
//! ([`Input::Sound`], [`ModelFrom::File`]): the split's first step reads
//! the model (about 2.6 s from the stick, by the plan's estimate) and the
//! part of the sound asked for, converted to 44.1 kHz, so neither holds up
//! the screen. [`Split::model`] then has the model, for the next split.

use crate::jobs::{Job, Jobs, Key, Lane, Running};
use crate::model::Source;
use mpfree_stems::{Model, Separator, Settings, Sink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use mpfree_stems::{segments, steps, Stem, RATE, SEGMENT};

/// The job kind of every step of a split.
pub const KIND: &str = "stems";

/// What a split made: the stems kept, in [`Stem::ALL`]'s order (None for
/// one switched off), each interleaved stereo at 44.1 kHz, as long as the
/// sound.
#[derive(Debug)]
pub struct Made {
    pub stems: [Option<Vec<f32>>; 4],
    /// Its time from the first step starting to the last ending, the
    /// other jobs between them included.
    pub took: Duration,
}

/// Where a split's model comes from.
pub enum ModelFrom {
    /// Read already (by an earlier split, [`Split::model`]).
    Loaded(Arc<Model>),
    /// Read from this file by the split's first step.
    File(PathBuf),
}

/// What a split works through.
pub enum Input {
    /// Samples read already, interleaved, at 44.1 kHz, `channels` (1 or 2)
    /// a frame.
    Samples {
        samples: Arc<[f32]>,
        channels: usize,
    },
    /// Frames `span` of `source`, read and converted to 44.1 kHz by the
    /// split's first step ([`read`]).
    Sound { source: Source, span: (u64, u64) },
}

/// What to split and how.
pub struct Ask {
    /// The sound (its id), for the split's key.
    pub id: u128,
    pub model: ModelFrom,
    pub input: Input,
    pub settings: Settings,
    /// Which stems are kept, in [`Stem::ALL`]'s order. All four are worked
    /// out whatever this says, since each mask needs all four networks.
    pub keep: [bool; 4],
    /// Round the stems to 16 bits, with dither: the sound is 16-bit.
    pub sixteen: bool,
    /// The columns [`Split::peaks`] has across the whole sound (the Stems
    /// page's lanes' width).
    pub columns: usize,
}

/// The stems' picture as they come: each stem's columns so far, each the
/// lowest and the highest sample of both sides over `per` frames, in
/// [`Stem::ALL`]'s order (all four, kept or not).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Peaks {
    pub per: usize,
    /// The columns across the whole sound.
    pub columns: usize,
    pub stems: [Vec<(f32, f32)>; 4],
}

/// A split under way.
pub struct Split {
    key: Key,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicU32>,
    peaks: Arc<Mutex<Peaks>>,
    model: Arc<Mutex<Option<Arc<Model>>>>,
    rx: Receiver<Result<Made, String>>,
}

/// The separation and where its stems go, once the model and the sound
/// are read.
struct Work {
    sep: Separator<Arc<Model>, Arc<[f32]>>,
    sink: Gather,
}

/// What moves from one step's job to the next.
struct State {
    jobs: Jobs,
    key: Key,
    /// Until the first step has read them: the model and the sound.
    read: Option<(ModelFrom, Input)>,
    settings: Settings,
    keep: [bool; 4],
    columns: usize,
    work: Option<Work>,
    peaks: Arc<Mutex<Peaks>>,
    model: Arc<Mutex<Option<Arc<Model>>>>,
    sixteen: bool,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicU32>,
    tx: Sender<Result<Made, String>>,
    started: Option<Instant>,
    /// The memory promised for the split, let go when it ends.
    _hold: Box<dyn Send>,
}

impl Split {
    /// Starts splitting `ask` on `jobs`' sound lane, holding `hold` (the
    /// memory promised for it) until it ends. With the model read and the
    /// samples given, the separation starts here, and Err, with nothing
    /// started, says why it can't take the sound (its channels); else the
    /// first step reads them, and [`Split::poll`] says if that fails.
    pub fn start(jobs: &Jobs, ask: Ask, hold: impl Send + 'static) -> Result<Split, String> {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicU32::new(0));
        let key = Key::new(KIND, ask.id);
        let peaks = Arc::new(Mutex::new(Peaks {
            columns: ask.columns.max(1),
            ..Peaks::default()
        }));
        let model = Arc::new(Mutex::new(None));
        let mut state = State {
            jobs: jobs.clone(),
            key,
            read: None,
            settings: ask.settings,
            keep: ask.keep,
            columns: ask.columns,
            work: None,
            peaks: peaks.clone(),
            model: model.clone(),
            sixteen: ask.sixteen,
            stop: stop.clone(),
            done: done.clone(),
            tx,
            started: None,
            _hold: Box::new(hold),
        };
        match (ask.model, ask.input) {
            (ModelFrom::Loaded(m), Input::Samples { samples, channels }) => {
                state.begin(m, samples, channels)?;
            }
            read => state.read = Some(read),
        }
        post(Box::new(state), true);
        Ok(Split {
            key,
            stop,
            done,
            peaks,
            model,
            rx,
        })
    }

    /// The model, once the split has read it (or was given it): kept for
    /// the next split, so it is read once.
    pub fn model(&self) -> Option<Arc<Model>> {
        self.model.lock().ok().and_then(|m| m.clone())
    }

    /// The split's key on the job service.
    pub fn key(&self) -> Key {
        self.key
    }

    /// How far it has got, 0 to 1, a step at a time.
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.done.load(Ordering::Relaxed))
    }

    /// Each stem's picture so far ([`Peaks`]): whole once it has
    /// finished.
    pub fn peaks(&self) -> Peaks {
        self.peaks.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// Stops it before its next step (a step under way finishes first);
    /// nothing it made is kept, and [`Split::poll`] then says "stopped".
    pub fn stop(&self, jobs: &Jobs) {
        self.stop.store(true, Ordering::Relaxed);
        jobs.cancel(self.key);
    }

    /// The stems once it has finished, or why it didn't; None while it
    /// runs.
    pub fn poll(&self) -> Option<Result<Made, String>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            // Its step was dropped from the queue (stopped, or the service
            // ended) without saying so.
            Err(TryRecvError::Disconnected) => Some(Err("stopped".to_string())),
        }
    }
}

/// Puts the split's next step at the back of the sound lane's queue: the
/// first in place of any split of the sound waiting or running, the rest
/// as the running step's follower ([`Jobs::follow`]).
fn post(state: Box<State>, first: bool) {
    let (jobs, key) = (state.jobs.clone(), state.key);
    let job = Job::new(Lane::Sound, key, move |run| step(state, run));
    if first {
        jobs.submit(job);
    } else {
        jobs.follow(job);
    }
}

impl State {
    /// The separation of `samples` on `model`, and where its stems go.
    fn begin(
        &mut self,
        model: Arc<Model>,
        samples: Arc<[f32]>,
        channels: usize,
    ) -> Result<(), String> {
        let frames = samples.len() / channels.max(1);
        if let Ok(mut m) = self.model.lock() {
            *m = Some(model.clone());
        }
        let sep = Separator::new(model, samples, channels, RATE, &self.settings)
            .map_err(|e| e.to_string())?;
        if let Ok(mut p) = self.peaks.lock() {
            p.per = frames.div_ceil(self.columns.max(1)).max(1);
        }
        self.work = Some(Work {
            sep,
            sink: Gather::new(self.keep, frames, self.peaks.clone()),
        });
        Ok(())
    }

    /// The first step, when it has the model and the sound to read.
    fn read_in(&mut self, (model, input): (ModelFrom, Input)) -> Result<(), String> {
        let model = match model {
            ModelFrom::Loaded(m) => m,
            ModelFrom::File(path) => Arc::new(Model::open(&path).map_err(|e| e.to_string())?),
        };
        let (samples, channels) = match input {
            Input::Samples { samples, channels } => (samples, channels),
            Input::Sound { source, span } => read(&source, span)?,
        };
        self.begin(model, samples, channels)
    }
}

/// Frames `span` (from, to) of `source`, interleaved, converted to 44.1 kHz
/// when it is at another rate, and its channels a frame. Heavy for a long
/// sound: call it off the audio and screen threads.
pub fn read(source: &Source, (from, to): (u64, u64)) -> Result<(Arc<[f32]>, usize), String> {
    let ch = usize::from(source.channels());
    let to = to.min(source.frames());
    if from >= to {
        return Err("Start and End are together: nothing to split".into());
    }
    let samples: Vec<f32> = source
        .data()
        .iter()
        .skip(from as usize * ch)
        .take((to - from) as usize * ch)
        .collect();
    if source.rate() == RATE {
        return Ok((samples.into(), ch));
    }
    let part = Source::new(&source.name, source.rate(), source.channels(), samples)
        .map_err(|e| e.to_string())?;
    let (at, _) = part.at_rate(RATE).map_err(|e| e.to_string())?;
    Ok((at.data().to_vec().into(), ch))
}

/// One step, then the next posted, or the stems sent.
fn step(mut st: Box<State>, run: &Running) {
    if st.stop.load(Ordering::Relaxed) || run.cancelled() {
        let _ = st.tx.send(Err("stopped".to_string()));
        return;
    }
    let started = *st.started.get_or_insert_with(Instant::now);
    if let Some(read) = st.read.take() {
        // The first step reads the model and the sound; the separation's
        // own steps follow.
        match st.read_in(read) {
            Ok(()) if st.stop.load(Ordering::Relaxed) || run.cancelled() => {
                let _ = st.tx.send(Err("stopped".to_string()));
            }
            Ok(()) => post(st, false),
            Err(e) => {
                let _ = st.tx.send(Err(e));
            }
        }
        return;
    }
    let Some(Work { sep, sink }) = st.work.as_mut() else {
        let _ = st.tx.send(Err("nothing to split".to_string()));
        return;
    };
    match sep.step(sink) {
        Ok(true) => {
            let p = sep.progress();
            st.done.store(p.to_bits(), Ordering::Relaxed);
            // Stopped during the step: no next one.
            if st.stop.load(Ordering::Relaxed) || run.cancelled() {
                let _ = st.tx.send(Err("stopped".to_string()));
                return;
            }
            post(st, false);
        }
        Ok(false) => {
            sink.finish();
            st.done.store(1f32.to_bits(), Ordering::Relaxed);
            let State {
                work, sixteen, tx, ..
            } = *st;
            let mut stems = work.map(|w| w.sink.stems).unwrap_or_default();
            if sixteen {
                let mut seed = 0x5eed_u32;
                for s in stems.iter_mut().flatten() {
                    to_16_bits(s, &mut seed);
                }
            }
            let _ = tx.send(Ok(Made {
                stems,
                took: started.elapsed(),
            }));
        }
        Err(e) => {
            let _ = st.tx.send(Err(e.to_string()));
        }
    }
}

/// The separation's sink: the kept stems, in buffers as long as the sound
/// from the start, so nothing grows as they come, and every stem's
/// picture.
struct Gather {
    stems: [Option<Vec<f32>>; 4],
    peaks: Arc<Mutex<Peaks>>,
    /// Each stem's column under way: its lowest, its highest, its frames.
    column: [(f32, f32, usize); 4],
    per: usize,
}

impl Gather {
    fn new(keep: [bool; 4], frames: usize, peaks: Arc<Mutex<Peaks>>) -> Gather {
        let per = peaks.lock().map_or(1, |p| p.per);
        Gather {
            stems: keep.map(|k| k.then(|| Vec::with_capacity(frames * 2))),
            peaks,
            column: [(0.0, 0.0, 0); 4],
            per,
        }
    }
}

impl Sink for Gather {
    fn put(&mut self, stem: Stem, stereo: &[f32]) -> Result<(), String> {
        let k = Stem::ALL.iter().position(|&s| s == stem).unwrap_or(0);
        if let Some(s) = self.stems[k].as_mut() {
            s.extend_from_slice(stereo);
        }
        let mut done = Vec::new();
        let (lo, hi, n) = &mut self.column[k];
        for f in stereo.chunks_exact(2) {
            *lo = lo.min(f[0]).min(f[1]);
            *hi = hi.max(f[0]).max(f[1]);
            *n += 1;
            if *n == self.per {
                done.push((*lo, *hi));
                (*lo, *hi, *n) = (0.0, 0.0, 0);
            }
        }
        if let Ok(mut p) = self.peaks.lock() {
            p.stems[k].extend(done);
        }
        Ok(())
    }
}

impl Gather {
    /// The last columns, short ones, once the stems are all out.
    fn finish(&mut self) {
        if let Ok(mut p) = self.peaks.lock() {
            for (k, c) in self.column.iter_mut().enumerate() {
                if c.2 > 0 {
                    p.stems[k].push((c.0, c.1));
                    *c = (0.0, 0.0, 0);
                }
            }
        }
    }
}

/// Rounds each sample to the nearest 16-bit step (`k / 32768`, as
/// [`crate::dsp::as_i16`] reads them) after triangular dither of up to one
/// step each way, clipped at full scale: every sample is then 16-bit
/// exactly, and the quantising noise is a steady hiss, not distortion that
/// follows the music.
pub fn to_16_bits(samples: &mut [f32], seed: &mut u32) {
    use crate::dsp::Sample;
    for x in samples {
        let d = uniform(seed) + uniform(seed);
        let k = (f64::from(*x) * 32_768.0 + d)
            .round()
            .clamp(-32_768.0, 32_767.0) as i16;
        // Through the integer, so a zero has no sign (-0.0 isn't 16-bit).
        *x = k.f();
    }
}

/// -0.5 to 0.5.
fn uniform(seed: &mut u32) -> f64 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    f64::from(*seed >> 8) / f64::from(1u32 << 24) - 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{Config, Order};
    use crate::model::Pcm;

    fn noise(n: usize, seed: u32, level: f32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * level
            })
            .collect()
    }

    fn ask(model: &Arc<Model>, input: &Arc<[f32]>, keep: [bool; 4], sixteen: bool) -> Ask {
        Ask {
            id: 7,
            model: ModelFrom::Loaded(model.clone()),
            input: Input::Samples {
                samples: input.clone(),
                channels: 2,
            },
            settings: Settings::default(),
            keep,
            sixteen,
            columns: 100,
        }
    }

    fn wait(split: &Split) -> Result<Made, String> {
        let t = Instant::now();
        loop {
            if let Some(r) = split.poll() {
                return r;
            }
            assert!(
                t.elapsed() < Duration::from_secs(120),
                "the split never ended"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The same stems as one separation run straight through.
    fn straight(model: &Model, input: &[f32]) -> [Vec<f32>; 4] {
        struct All([Vec<f32>; 4]);
        impl Sink for All {
            fn put(&mut self, stem: Stem, stereo: &[f32]) -> Result<(), String> {
                let k = Stem::ALL.iter().position(|&s| s == stem).unwrap();
                self.0[k].extend_from_slice(stereo);
                Ok(())
            }
        }
        let mut sep =
            Separator::new(model, input, 2, mpfree_stems::RATE, &Settings::default()).unwrap();
        let mut all = All(Default::default());
        while sep.step(&mut all).unwrap() {}
        all.0
    }

    #[test]
    fn a_split_runs_a_step_a_job_and_keeps_the_stems_asked_for() {
        let model = Arc::new(Model::small(3));
        // Two seconds: two 64-frame segments of the small network.
        let input: Arc<[f32]> = noise(2 * 44_100 * 2, 5, 0.4).into();
        let jobs = Jobs::new(Config::default());
        let split = Split::start(
            &jobs,
            ask(&model, &input, [true, false, true, true], false),
            (),
        )
        .unwrap();
        let made = wait(&split).unwrap();
        assert_eq!(split.progress(), 1.0);
        let want = straight(&model, &input);
        assert_eq!(made.stems[0].as_deref(), Some(&want[0][..]));
        assert!(made.stems[1].is_none(), "drums switched off");
        assert_eq!(made.stems[2].as_deref(), Some(&want[2][..]));
        assert_eq!(made.stems[3].as_deref(), Some(&want[3][..]));
        assert_eq!(want[0].len(), input.len());
        // Every step was a job of its own: 2 segments × 6 steps, none of
        // them counted replaced by the next.
        let c = jobs.counts();
        assert_eq!((c.done, c.replaced), (12, 0));
        // The picture: every stem's, kept or not, a column a hundredth of
        // the sound, each its lowest and highest sample.
        let peaks = split.peaks();
        assert_eq!((peaks.per, peaks.columns), (882, 100));
        for (k, stem) in want.iter().enumerate() {
            assert_eq!(peaks.stems[k].len(), 100, "stem {k}");
            for (c, &(lo, hi)) in peaks.stems[k].iter().enumerate() {
                let col = &stem[c * 882 * 2..((c + 1) * 882 * 2).min(stem.len())];
                let want_lo = col.iter().copied().fold(0.0f32, f32::min);
                let want_hi = col.iter().copied().fold(0.0f32, f32::max);
                assert_eq!((lo, hi), (want_lo, want_hi), "stem {k} column {c}");
            }
        }
    }

    #[test]
    fn its_first_step_reads_the_part_asked_for_at_44_1_khz() {
        let model = Arc::new(Model::small(3));
        // Two seconds at 48 kHz; the second from 0.5 s.
        let source = Source::new("Break", 48_000, 2, noise(2 * 48_000 * 2, 9, 0.4)).unwrap();
        let jobs = Jobs::new(Config::default());
        let ask = Ask {
            input: Input::Sound {
                source: source.clone(),
                span: (24_000, 72_000),
            },
            ..ask(&model, &Arc::from(Vec::new()), [true; 4], false)
        };
        let split = Split::start(&jobs, ask, ()).unwrap();
        let made = wait(&split).unwrap();
        let (read, channels) = read(&source, (24_000, 72_000)).unwrap();
        assert_eq!(channels, 2);
        assert!(
            (read.len() as i64 / 2 - 44_100).abs() <= 2,
            "{} frames",
            read.len() / 2
        );
        assert_eq!(made.stems[0].as_ref().map(Vec::len), Some(read.len()));
        // The same stems as a split of what it read.
        assert_eq!(
            made.stems[2].as_deref(),
            Some(&straight(&model, &read)[2][..])
        );
        assert!(Arc::ptr_eq(&split.model().unwrap(), &model));
        // Its first job read the sound; one segment of six steps followed.
        assert_eq!(jobs.counts().done, 7);
        assert!(read_of_nothing_is_refused(&source));
    }

    fn read_of_nothing_is_refused(source: &Source) -> bool {
        read(source, (500, 500)).is_err() && read(source, (500, 400)).is_err()
    }

    #[test]
    fn a_model_file_that_isnt_there_says_why_and_nothing_is_kept() {
        let jobs = Jobs::new(Config::default());
        let held = Arc::new(());
        let model = Arc::new(Model::small(3));
        let input: Arc<[f32]> = noise(44_100 * 2, 5, 0.4).into();
        let ask = Ask {
            model: ModelFrom::File(PathBuf::from("/nonexistent/spleeter-4stems.mpfstem")),
            ..ask(&model, &input, [true; 4], false)
        };
        let split = Split::start(&jobs, ask, held.clone()).unwrap();
        let why = wait(&split).unwrap_err();
        assert!(why.starts_with("stem model:"), "{why}");
        assert!(split.model().is_none());
        let t = Instant::now();
        while Arc::strong_count(&held) > 1 {
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "the promise was kept"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_picture_fills_a_segment_at_a_time() {
        let model = Arc::new(Model::small(3));
        let input: Arc<[f32]> = noise(2 * 44_100 * 2, 5, 0.4).into();
        let jobs = Jobs::new(Config::default());
        jobs.hold(true);
        let split = Split::start(&jobs, ask(&model, &input, [true; 4], false), ()).unwrap();
        assert!(split.peaks().stems.iter().all(Vec::is_empty));
        jobs.hold(false);
        let mut seen = Vec::new();
        while split.poll().is_none() {
            let n = split.peaks().stems[0].len();
            if seen.last() != Some(&n) {
                seen.push(n);
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        let n = split.peaks().stems[0].len();
        if seen.last() != Some(&n) {
            seen.push(n);
        }
        // Nothing, the first segment's columns, then all of them.
        assert_eq!(seen.first(), Some(&0));
        assert_eq!(seen.last(), Some(&100));
        assert!(seen.len() >= 3, "{seen:?}");
        assert!(
            seen.windows(2).all(|w| w[0] < w[1]),
            "it only grows: {seen:?}"
        );
    }

    #[test]
    fn other_sounds_jobs_run_between_its_steps() {
        let model = Arc::new(Model::small(3));
        let input: Arc<[f32]> = noise(2 * 44_100 * 2, 5, 0.4).into();
        let jobs = Jobs::new(Config::default());
        jobs.hold(true);
        let split = Split::start(&jobs, ask(&model, &input, [true; 4], false), ()).unwrap();
        // A view of another sound, posted while the split's first step
        // waits, goes first; one posted at the back still runs before the
        // split's second step, which is posted behind it.
        let (tx, rx) = mpsc::channel();
        let t2 = tx.clone();
        jobs.submit(
            Job::new(Lane::Sound, Key::new("view", 1), move |_| {
                let _ = tx.send("view");
            })
            .order(Order::First),
        );
        let at_back = Arc::new(AtomicU32::new(u32::MAX));
        let seen = at_back.clone();
        let done = split.done.clone();
        jobs.submit(Job::new(Lane::Sound, Key::new("hits", 2), move |_| {
            seen.store(done.load(Ordering::Relaxed), Ordering::Relaxed);
            let _ = t2.send("hits");
        }));
        jobs.hold(false);
        wait(&split).unwrap();
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), ["view", "hits"]);
        // The hits ran after the split's first step and before its last.
        let p = f32::from_bits(at_back.load(Ordering::Relaxed));
        assert!(p > 0.0 && p < 1.0, "the hits ran at {p}");
    }

    #[test]
    fn stop_ends_it_between_steps_and_lets_its_memory_go() {
        let model = Arc::new(Model::small(3));
        let input: Arc<[f32]> = noise(4 * 44_100 * 2, 5, 0.4).into();
        let jobs = Jobs::new(Config::default());
        let held = Arc::new(());
        let split =
            Split::start(&jobs, ask(&model, &input, [true; 4], false), held.clone()).unwrap();
        while split.progress() == 0.0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        split.stop(&jobs);
        assert_eq!(wait(&split).unwrap_err(), "stopped");
        assert!(split.progress() < 1.0);
        // The promise goes with the split's last job.
        let t = Instant::now();
        while Arc::strong_count(&held) > 1 {
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "the promise was kept"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        // And the job itself, once its worker has put it down.
        while jobs.has(split.key()) {
            assert!(t.elapsed() < Duration::from_secs(10), "the job stayed");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_second_split_of_the_sound_takes_the_firsts_place() {
        let model = Arc::new(Model::small(3));
        let input: Arc<[f32]> = noise(2 * 44_100 * 2, 5, 0.4).into();
        let jobs = Jobs::new(Config::default());
        jobs.hold(true);
        let first = Split::start(&jobs, ask(&model, &input, [true; 4], false), ()).unwrap();
        let second = Split::start(&jobs, ask(&model, &input, [true; 4], false), ()).unwrap();
        jobs.hold(false);
        assert_eq!(wait(&first).unwrap_err(), "stopped");
        assert!(wait(&second).is_ok());
    }

    #[test]
    fn a_16_bit_sounds_stems_are_held_as_16_bits() {
        let model = Arc::new(Model::small(3));
        let input: Arc<[f32]> = noise(44_100 * 2, 5, 0.4).into();
        let jobs = Jobs::new(Config::default());
        let split = Split::start(&jobs, ask(&model, &input, [true; 4], true), ()).unwrap();
        let made = wait(&split).unwrap();
        let want = straight(&model, &input);
        for (got, want) in made.stems.iter().zip(&want) {
            let got = got.as_ref().unwrap();
            assert!(Pcm::new(got.clone()).is_16_bit());
            // Within a step and a half of the floats.
            let worst = got
                .iter()
                .zip(want)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(worst <= 1.5 / 32_768.0 + 1e-7, "{worst}");
        }
    }

    #[test]
    fn dither_spreads_the_rounding_and_clips_at_full_scale() {
        let mut seed = 1;
        // A level between two steps comes out as both, on average where it
        // was: the dither keeps what is below a step.
        let level = 0.3 / 32_768.0;
        let mut x = vec![level; 100_000];
        to_16_bits(&mut x, &mut seed);
        let mean = x.iter().map(|&v| f64::from(v)).sum::<f64>() / x.len() as f64;
        assert!((mean - f64::from(level)).abs() < 0.02 / 32_768.0, "{mean}");
        assert!(x.iter().all(|&v| crate::dsp::as_i16(v).is_some()));
        let mut loud = vec![1.5, -1.5, 1.0];
        to_16_bits(&mut loud, &mut seed);
        assert_eq!(loud[..2], [32_767.0 / 32_768.0, -1.0]);
        assert!(loud[2] >= 32_766.0 / 32_768.0);
    }
}
