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

use crate::jobs::{Job, Jobs, Key, Lane, Running};
use mpfree_stems::{Model, Separator, Settings, Sink, Stem};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// What to split and how.
pub struct Ask {
    /// The sound (its id), for the split's key.
    pub id: u128,
    pub model: Arc<Model>,
    /// The samples read, interleaved, at 44.1 kHz.
    pub input: Arc<[f32]>,
    /// 1 or 2.
    pub channels: usize,
    pub settings: Settings,
    /// Which stems are kept, in [`Stem::ALL`]'s order. All four are worked
    /// out whatever this says, since each mask needs all four networks.
    pub keep: [bool; 4],
    /// Round the stems to 16 bits, with dither: the sound is 16-bit.
    pub sixteen: bool,
}

/// A split under way.
pub struct Split {
    key: Key,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicU32>,
    rx: Receiver<Result<Made, String>>,
}

/// What moves from one step's job to the next.
struct State {
    jobs: Jobs,
    key: Key,
    sep: Separator<Arc<Model>, Arc<[f32]>>,
    sink: Gather,
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
    /// memory promised for it) until it ends. Err, with nothing started,
    /// if the separation can't take the sound (its channels).
    pub fn start(jobs: &Jobs, ask: Ask, hold: impl Send + 'static) -> Result<Split, String> {
        let frames = ask.input.len() / ask.channels.max(1);
        let sep = Separator::new(
            ask.model,
            ask.input,
            ask.channels,
            mpfree_stems::RATE,
            &ask.settings,
        )
        .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicU32::new(0));
        let key = Key::new(KIND, ask.id);
        let state = State {
            jobs: jobs.clone(),
            key,
            sep,
            sink: Gather::new(ask.keep, frames),
            sixteen: ask.sixteen,
            stop: stop.clone(),
            done: done.clone(),
            tx,
            started: None,
            _hold: Box::new(hold),
        };
        post(Box::new(state), true);
        Ok(Split {
            key,
            stop,
            done,
            rx,
        })
    }

    /// The split's key on the job service.
    pub fn key(&self) -> Key {
        self.key
    }

    /// How far it has got, 0 to 1, a step at a time.
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.done.load(Ordering::Relaxed))
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

/// One step, then the next posted, or the stems sent.
fn step(mut st: Box<State>, run: &Running) {
    if st.stop.load(Ordering::Relaxed) || run.cancelled() {
        let _ = st.tx.send(Err("stopped".to_string()));
        return;
    }
    let started = *st.started.get_or_insert_with(Instant::now);
    let State { sep, sink, .. } = &mut *st;
    match sep.step(sink) {
        Ok(true) => {
            st.done
                .store(st.sep.progress().to_bits(), Ordering::Relaxed);
            // Stopped during the step: no next one.
            if st.stop.load(Ordering::Relaxed) || run.cancelled() {
                let _ = st.tx.send(Err("stopped".to_string()));
                return;
            }
            post(st, false);
        }
        Ok(false) => {
            st.done.store(1f32.to_bits(), Ordering::Relaxed);
            let State {
                sink, sixteen, tx, ..
            } = *st;
            let mut stems = sink.stems;
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
/// from the start, so nothing grows as they come.
struct Gather {
    stems: [Option<Vec<f32>>; 4],
}

impl Gather {
    fn new(keep: [bool; 4], frames: usize) -> Gather {
        Gather {
            stems: keep.map(|k| k.then(|| Vec::with_capacity(frames * 2))),
        }
    }
}

impl Sink for Gather {
    fn put(&mut self, stem: Stem, stereo: &[f32]) -> Result<(), String> {
        let k = Stem::ALL.iter().position(|&s| s == stem).unwrap_or(0);
        if let Some(s) = self.stems[k].as_mut() {
            s.extend_from_slice(stereo);
        }
        Ok(())
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
            model: model.clone(),
            input: input.clone(),
            channels: 2,
            settings: Settings::default(),
            keep,
            sixteen,
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
        assert!(!jobs.has(split.key()));
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
