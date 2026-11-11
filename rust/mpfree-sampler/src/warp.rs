//! Warp: regions that play at the project tempo.
//!
//! A warped region says what tempo it was played at ([`Warp::bpm`]). With
//! [`WarpMethod::Repitch`] it simply plays faster or slower, like tape. With
//! [`WarpMethod::Stretch`] it plays a stretched copy (made by
//! `mpfree-stretch` on a worker) whose length fits the project tempo and
//! whose pitch is the region's own, tune included. Until the copy for the
//! tempo is ready the region repitches, so it is always on the beat; a hit
//! after the copy arrives plays the copy.
//!
//! [`Warps`] holds the project tempo and the copies made for it, within a
//! memory budget. The control side asks it what is [`Warps::missing`],
//! runs a [`Warps::job`] for each on a worker, hands the result back with
//! [`Warps::insert`] and builds a new engine view with
//! [`crate::engine::EngineView::build_warped`]. The audio thread only
//! hears of the tempo ([`crate::engine::Engine::set_tempo`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mpfree_stretch::{Job, Settings, StretchError, TimeMap};

use crate::model::{ModelError, Project, Region, Source, SourceId, Warp, WarpMethod, PADS};

/// Memory the stretched copies may use, all told: about ten minutes of
/// stereo audio at 44.1 kHz.
pub const DEFAULT_BUDGET: usize = 200 << 20;

/// One stretched copy: a region's frames made `frames` long and moved by
/// `cents`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CopyKey {
    pub source: SourceId,
    pub start: u64,
    pub end: u64,
    pub frames: u64,
    pub cents: i32,
}

impl CopyKey {
    /// The copy a region warped with `warp` needs at `tempo`, with `cents`
    /// of tune (the region's, layer's and pad's together).
    pub fn of(region: &Region, warp: &Warp, cents: i32, tempo: f64) -> CopyKey {
        let len = region.frames() as f64;
        let frames = (len * f64::from(warp.bpm) / tempo).round().max(1.0) as u64;
        CopyKey {
            source: region.source,
            start: region.start,
            end: region.end,
            frames,
            cents,
        }
    }

    /// Output frames per input frame.
    pub fn scale(&self) -> f64 {
        self.frames as f64 / (self.end - self.start).max(1) as f64
    }
}

struct Copy {
    key: CopyKey,
    data: Arc<[f32]>,
    used: u64,
}

/// The project tempo and the stretched copies made for it.
pub struct Warps {
    tempo: f64,
    budget: usize,
    copies: Vec<Copy>,
    clock: u64,
}

impl Default for Warps {
    fn default() -> Warps {
        Warps::new(120.0)
    }
}

/// Tempo is kept within this, whatever the transport says.
const TEMPO: std::ops::RangeInclusive<f64> = 20.0..=999.0;

impl Warps {
    pub fn new(tempo: f64) -> Warps {
        Warps {
            tempo: tempo_ok(tempo),
            budget: DEFAULT_BUDGET,
            copies: Vec::new(),
            clock: 0,
        }
    }

    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    /// A new project tempo. The copies for the old one stay (within the
    /// budget), so going back is free. Returns whether it changed.
    pub fn set_tempo(&mut self, bpm: f64) -> bool {
        let bpm = tempo_ok(bpm);
        let changed = (bpm - self.tempo).abs() > 1e-9;
        self.tempo = bpm;
        changed
    }

    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
    }

    /// Memory the copies use now.
    pub fn bytes(&self) -> usize {
        self.copies.iter().map(|c| c.data.len() * 4).sum()
    }

    /// Every copy the project's warped regions need at the tempo, once
    /// each, in pad order.
    pub fn wanted(&self, project: &Project) -> Vec<CopyKey> {
        let mut out: Vec<CopyKey> = Vec::new();
        for i in 0..PADS {
            let Some(p) = project.pad(i) else { continue };
            for l in &p.layers {
                if let Some(k) = self.key_for(&l.region, l.tune_cents + p.tune_cents) {
                    if !out.contains(&k) {
                        out.push(k);
                    }
                }
            }
        }
        out
    }

    /// The copies the project needs that aren't made yet.
    pub fn missing(&self, project: &Project) -> Vec<CopyKey> {
        let mut w = self.wanted(project);
        w.retain(|k| self.copies.iter().all(|c| c.key != *k));
        w
    }

    /// The copy `region` needs at the tempo (`extra_cents` is the layer's
    /// and pad's tune), if it is warped to stretch.
    pub fn key_for(&self, region: &Region, extra_cents: i32) -> Option<CopyKey> {
        let w = region.warp?;
        if w.method != WarpMethod::Stretch || region.mode == crate::model::PlayMode::Granular {
            return None;
        }
        // A copy moves the pitch two octaves at most.
        let cents = (region.tune_cents + extra_cents).clamp(-2400, 2400);
        Some(CopyKey::of(region, &w, cents, self.tempo))
    }

    /// The copy for `key`, if it has been made. Marks it used.
    pub fn copy(&mut self, key: &CopyKey) -> Option<Arc<[f32]>> {
        self.clock += 1;
        let now = self.clock;
        self.copies.iter_mut().find(|c| c.key == *key).map(|c| {
            c.used = now;
            c.data.clone()
        })
    }

    /// The copy for `key` without marking it used.
    pub fn peek(&self, key: &CopyKey) -> Option<&Arc<[f32]>> {
        self.copies.iter().find(|c| c.key == *key).map(|c| &c.data)
    }

    /// Keeps a finished copy. Past the budget, the least recently used
    /// copies the project doesn't want at this tempo go first; the ones it
    /// wants are never dropped.
    pub fn insert(&mut self, key: CopyKey, data: Vec<f32>, project: &Project) {
        self.insert_for(key, data, std::slice::from_ref(project));
    }

    /// As [`Warps::insert`], keeping what any of `projects` wants.
    pub fn insert_for(&mut self, key: CopyKey, data: Vec<f32>, projects: &[Project]) {
        self.clock += 1;
        self.copies.retain(|c| c.key != key);
        self.copies.push(Copy {
            key,
            data: data.into(),
            used: self.clock,
        });
        let wanted: Vec<CopyKey> = projects.iter().flat_map(|p| self.wanted(p)).collect();
        while self.bytes() > self.budget {
            let old = self
                .copies
                .iter()
                .enumerate()
                .filter(|(_, c)| !wanted.contains(&c.key))
                .min_by_key(|(_, c)| c.used)
                .map(|(i, _)| i);
            match old {
                Some(i) => {
                    self.copies.remove(i);
                }
                None => break,
            }
        }
    }

    /// Forgets every copy (a new project).
    pub fn clear(&mut self) {
        self.copies.clear();
    }

    /// The stretch job that makes the copy for `key` from the project's
    /// source. It finds the hits as it starts, so make it on a worker
    /// ([`Warps::spec`] is the cheap part to take there).
    pub fn job(project: &Project, key: &CopyKey) -> Result<Job, StretchError> {
        Warps::spec(project, key)?.start()
    }

    /// What the job for `key` needs, copied out of the project (the
    /// region's frames once): quick, for the control thread to hand to a
    /// [`Worker`].
    pub fn spec(project: &Project, key: &CopyKey) -> Result<JobSpec, StretchError> {
        let s = project.source(key.source).ok_or(StretchError::Empty)?;
        let ch = usize::from(s.channels());
        let (a, b) = (key.start as usize * ch, key.end as usize * ch);
        let data = s.data().get(a..b).ok_or(StretchError::Empty)?;
        Ok(JobSpec {
            data: data.into(),
            channels: s.channels(),
            rate: s.rate(),
            map: TimeMap::to_length(key.end - key.start, key.frames)?,
            settings: Settings {
                semitones: (key.cents as f32 / 100.0).clamp(-24.0, 24.0),
                ..Settings::default()
            },
        })
    }
}

/// Everything a stretch job needs, owned, so it can go to a worker.
pub struct JobSpec {
    pub data: Arc<[f32]>,
    pub channels: u8,
    pub rate: u32,
    pub map: TimeMap,
    pub settings: Settings,
}

impl JobSpec {
    /// The job (finding the hits first).
    pub fn start(self) -> Result<Job, StretchError> {
        Job::new(self.data, self.channels, self.rate, self.map, self.settings)
    }
}

/// What a [`Worker`] hands back.
#[derive(Debug)]
pub enum Done {
    /// The copy for the key, and how long it took.
    Copy(CopyKey, Vec<f32>, Duration),
    Failed(CopyKey, StretchError),
    /// Dropped by [`Worker::cancel`] before it finished.
    Cancelled(CopyKey),
}

/// Output frames the worker does between looks at whether it was
/// cancelled: a few ms of work on the MPC.
const WORKER_STEP: usize = 8192;

/// A thread that makes stretched copies one at a time, in the order asked.
/// Dropping it cancels what is left and waits for the thread.
pub struct Worker {
    tx: Option<Sender<(u64, CopyKey, JobSpec)>>,
    rx: Receiver<Done>,
    generation: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts the thread (at normal priority: it should never compete
    /// with the audio thread).
    pub fn spawn() -> std::io::Result<Worker> {
        let (tx, jobs) = channel::<(u64, CopyKey, JobSpec)>();
        let (done, rx) = channel();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let handle = std::thread::Builder::new()
            .name("mpfree-stretch".into())
            .spawn(move || {
                for (g, key, spec) in jobs {
                    let stale = || current.load(Ordering::Relaxed) != g;
                    let t = Instant::now();
                    let result = if stale() {
                        Done::Cancelled(key)
                    } else {
                        match spec.start() {
                            Err(e) => Done::Failed(key, e),
                            Ok(mut job) => loop {
                                if stale() {
                                    break Done::Cancelled(key);
                                }
                                if job.run(WORKER_STEP).finished {
                                    let out = job.into_output().unwrap_or_default();
                                    break Done::Copy(key, out, t.elapsed());
                                }
                            },
                        }
                    };
                    if done.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Worker {
            tx: Some(tx),
            rx,
            generation,
            handle: Some(handle),
        })
    }

    /// Asks for the copy for `key`.
    pub fn send(&self, key: CopyKey, spec: JobSpec) -> bool {
        let g = self.generation.load(Ordering::Relaxed);
        self.tx
            .as_ref()
            .is_some_and(|tx| tx.send((g, key, spec)).is_ok())
    }

    /// Drops every job asked for so far, the one running included (it
    /// stops within a step).
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A finished (or failed, or cancelled) job, if one is waiting.
    pub fn try_recv(&self) -> Option<Done> {
        self.rx.try_recv().ok()
    }

    /// Waits up to `timeout` for the next one (for tests and tools).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Done> {
        self.rx.recv_timeout(timeout).ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// A stretch or pitch shift of one layer's region that makes a new source
/// (the Process menu): `ratio` times as long and `semitones` higher. Make
/// the [`Process::job`], run it on a worker, then [`Process::finish`] puts
/// the result in the layer's place as one undo step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Process {
    pub pad: usize,
    pub layer: usize,
    pub ratio: f64,
    pub semitones: f32,
}

impl Process {
    /// Stretched to `to_bpm` from the tempo the region is at.
    pub fn to_tempo(pad: usize, layer: usize, from_bpm: f64, to_bpm: f64) -> Process {
        Process {
            pad,
            layer,
            ratio: from_bpm / to_bpm,
            semitones: 0.0,
        }
    }

    fn region<'a>(&self, project: &'a Project) -> Result<&'a Region, StretchError> {
        project
            .pad(self.pad)
            .and_then(|p| p.layers.get(self.layer))
            .map(|l| &l.region)
            .ok_or(StretchError::Empty)
    }

    /// The job that makes the new audio. Run it on a worker.
    pub fn job(&self, project: &Project) -> Result<Job, StretchError> {
        let r = self.region(project)?;
        let s = project.source(r.source).ok_or(StretchError::Empty)?;
        let ch = usize::from(s.channels());
        let data = s
            .data()
            .get(r.start as usize * ch..r.end as usize * ch)
            .ok_or(StretchError::Empty)?;
        let map = TimeMap::ratio(r.frames(), self.ratio)?;
        let settings = Settings {
            semitones: self.semitones,
            ..Settings::default()
        };
        Job::new(data.into(), s.channels(), s.rate(), map, settings)
    }

    /// Puts the finished job's audio in the layer's place (one undo step)
    /// and returns the new source's id. The region it was made from must
    /// still be there.
    pub fn finish(&self, project: &mut Project, audio: Vec<f32>) -> Result<SourceId, ModelError> {
        let r = self.region(project).map_err(|_| ModelError::BadPad)?;
        let s = project.source(r.source).ok_or(ModelError::UnknownSource)?;
        let frames_in = r.frames() as f64;
        let mut name = s.name.clone();
        if (self.ratio - 1.0).abs() > 1e-9 {
            name.push_str(&format!(" x{:.2}", self.ratio));
        }
        if self.semitones != 0.0 {
            name.push_str(&format!(" {:+}st", self.semitones));
        }
        let new = Source::new(&name, s.rate(), s.channels(), audio)?;
        let scale = new.frames() as f64 / frames_in;
        project.use_processed(self.pad, self.layer, new, scale)
    }
}

fn tempo_ok(bpm: f64) -> f64 {
    if bpm.is_finite() {
        bpm.clamp(*TEMPO.start(), *TEMPO.end())
    } else {
        120.0
    }
}
