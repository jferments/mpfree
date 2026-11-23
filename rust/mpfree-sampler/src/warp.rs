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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mpfree_stretch::{Job, Settings, StretchError, TimeMap};

use crate::jobs::{self, Jobs, Key, Lane};

use crate::model::{
    through, ModelError, Pin, Project, Region, Source, SourceId, Warp, WarpMethod, PADS,
};

/// Memory the stretched copies may use, all told: about ten minutes of
/// stereo audio at 44.1 kHz.
pub const DEFAULT_BUDGET: usize = 200 << 20;

/// One stretched copy: a region's frames made `frames` long and moved by
/// `cents`, through its pins (`pins` tells pin sets apart; 0 when there are
/// none).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CopyKey {
    pub source: SourceId,
    pub start: u64,
    pub end: u64,
    pub frames: u64,
    pub cents: i32,
    pub pins: u64,
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
            pins: if warp.pinned() {
                pins_id(&warp.pins)
            } else {
                0
            },
        }
    }

    /// Output frames per input frame, all told.
    pub fn scale(&self) -> f64 {
        self.frames as f64 / (self.end - self.start).max(1) as f64
    }
}

/// Tells pin sets apart: 0 for none, else a hash of every pin.
pub fn pins_id(pins: &[Pin]) -> u64 {
    use std::hash::{Hash, Hasher};
    if pins.is_empty() {
        return 0;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in pins {
        p.frame.hash(&mut h);
        p.beat.to_bits().hash(&mut h);
    }
    h.finish().max(1)
}

/// How a region's frames land in its stretched copy: points of (frames
/// from the region's start, copy frame) with straight lines between, the
/// start on 0 and the end on the copy's last frame. Without pins it is one
/// straight line.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyMap {
    /// The source frame the copy starts from.
    pub start: u64,
    points: Arc<[(f64, f64)]>,
}

impl CopyMap {
    /// The map for the copy `key` of a region warped with `warp`, from a
    /// source at `rate`. Pins count only when the warp stretches.
    pub fn new(warp: &Warp, rate: u32, key: &CopyKey) -> CopyMap {
        let pts = if warp.pinned() {
            warp.points(key.start, key.end, rate)
        } else {
            let len = key.end.saturating_sub(key.start);
            vec![(0.0, 0.0), (len as f64, warp.beats(len, rate))]
        };
        let total = pts[pts.len() - 1].1.max(f64::MIN_POSITIVE);
        let k = key.frames as f64 / total;
        CopyMap {
            start: key.start,
            points: pts.iter().map(|&(f, b)| (f, b * k)).collect(),
        }
    }

    /// The (input, output) points, for a stretch's time map.
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// The copy frame source frame `x` turns into.
    pub fn at(&self, x: f64) -> f64 {
        through(&self.points, x - self.start as f64, |p| p, self.scale())
    }

    /// The source frame copy frame `y` was made from.
    pub fn source(&self, y: f64) -> f64 {
        let back = |(x, y): (f64, f64)| (y, x);
        self.start as f64 + through(&self.points, y, back, 1.0 / self.scale())
    }

    /// Copy frames per source frame, all told.
    fn scale(&self) -> f64 {
        let (x, y) = self.points[self.points.len() - 1];
        if x > 0.0 {
            y / x
        } else {
            1.0
        }
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
    /// each, in pad order, then the takes kept on the Sample page.
    pub fn wanted(&self, project: &Project) -> Vec<CopyKey> {
        let mut out: Vec<CopyKey> = Vec::new();
        let pads = (0..PADS).filter_map(|i| project.pad(i));
        for p in pads.chain(project.takes()) {
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
        let w = region.warp.as_ref()?;
        if w.method != WarpMethod::Stretch || region.mode == crate::model::PlayMode::Granular {
            return None;
        }
        // A copy moves the pitch two octaves at most.
        let cents = (region.tune_cents + extra_cents).clamp(-2400, 2400);
        Some(CopyKey::of(region, w, cents, self.tempo))
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
    pub fn insert(&mut self, key: CopyKey, data: impl Into<Arc<[f32]>>, project: &Project) {
        self.insert_for(key, data, std::slice::from_ref(project));
    }

    /// As [`Warps::insert`], keeping what any of `projects` wants.
    pub fn insert_for(&mut self, key: CopyKey, data: impl Into<Arc<[f32]>>, projects: &[Project]) {
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

    /// What the job for `key` needs, taken from the project: the sound
    /// itself, shared, and the region's span, whose frames are copied out
    /// only as the job starts, on its worker (sampler plan phase 2 step
    /// 7h): quick, for the control thread to hand to a [`Worker`].
    pub fn spec(project: &Project, key: &CopyKey) -> Result<JobSpec, StretchError> {
        let s = project.source(key.source).ok_or(StretchError::Empty)?;
        if key.start >= key.end || key.end > s.frames() {
            return Err(StretchError::Empty);
        }
        let map = match Warps::pinned_warp(project, key) {
            Some(w) => TimeMap::new(CopyMap::new(&w, s.rate(), key).points().to_vec())?,
            None => TimeMap::to_length(key.end - key.start, key.frames)?,
        };
        Ok(JobSpec {
            source: s.clone(),
            span: key.start..key.end,
            map,
            settings: Settings {
                semitones: (key.cents as f32 / 100.0).clamp(-24.0, 24.0),
                ..Settings::default()
            },
        })
    }
}

impl Warps {
    /// The warp, pins and all, of a region in `project` that wants the
    /// copy `key`, when the copy goes through pins.
    fn pinned_warp(project: &Project, key: &CopyKey) -> Option<Warp> {
        if key.pins == 0 {
            return None;
        }
        (0..PADS)
            .filter_map(|i| project.pad(i))
            .chain(project.takes())
            .flat_map(|p| &p.layers)
            .map(|l| &l.region)
            .filter(|r| (r.source, r.start, r.end) == (key.source, key.start, key.end))
            .filter_map(|r| r.warp.as_ref())
            .find(|w| w.pinned() && pins_id(&w.pins) == key.pins)
            .cloned()
    }
}

/// Everything a stretch job needs, owned, so it can go to a worker.
pub struct JobSpec {
    /// The sound, shared (a sound never changes once made).
    pub source: Source,
    /// The frames of it the job stretches: copied out as floats only when
    /// it starts ([`JobSpec::start`]), on its worker, never on the screen.
    pub span: std::ops::Range<u64>,
    pub map: TimeMap,
    pub settings: Settings,
}

impl JobSpec {
    /// The job: its span copied out of the sound, then the hits found.
    pub fn start(self) -> Result<Job, StretchError> {
        let ch = usize::from(self.source.channels());
        let (a, b) = (self.span.start as usize * ch, self.span.end as usize * ch);
        let data = self.source.data().f32s(a..b).ok_or(StretchError::Empty)?;
        Job::new(
            data.into(),
            self.source.channels(),
            self.source.rate(),
            self.map,
            self.settings,
        )
    }
}

/// What a [`Worker`] hands back.
#[derive(Debug)]
pub enum Done {
    /// The copy for the key, and how long it took. Made shareable on the
    /// worker, inside its job's memory promise: turning a `Vec` into an
    /// `Arc<[f32]>` copies it, and on the screen's thread that copy came
    /// after the promise ended, beside the next job's (train 147's CI held
    /// a copy's 21 MB over the memory test's limit that way).
    Copy(CopyKey, Arc<[f32]>, Duration),
    Failed(CopyKey, StretchError),
    /// Dropped before it finished: by [`Worker::cancel`], or for a newer
    /// copy of the same region (another tempo, other pins).
    Cancelled(CopyKey),
    /// Not started: the memory floor couldn't spare it. Why, in a
    /// musician's words.
    Refused(CopyKey, String),
}

/// Output frames the worker does between looks at whether it was
/// cancelled: a few ms of work on the MPC.
const WORKER_STEP: usize = 8192;

/// The job service's key for a copy: its region and tune, whatever the
/// tempo and pins, so a copy asked for at a newer tempo (or after the pins
/// moved) takes the place of the one still waiting or running.
fn service_key(k: &CopyKey) -> Key {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (k.source.0, k.start, k.end, k.cents).hash(&mut h);
    Key::new("stretch", u128::from(h.finish()))
}

/// The copies on the job service, by its key: the newest asked for each.
type Asked = Arc<Mutex<HashMap<Key, CopyKey>>>;

/// Makes stretched copies on the sampler's job service ([`crate::jobs`],
/// its sound lane), in the order asked. A copy of a region asked for again
/// (at a newer tempo, or with its pins moved) takes the place of the one
/// still waiting or running, which comes back [`Done::Cancelled`] at once:
/// a run of tempo changes keeps one job a region, and only the last is
/// made. Dropping it cancels what is left.
pub struct Worker {
    jobs: Jobs,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    generation: Arc<AtomicU64>,
    asked: Asked,
}

impl Worker {
    /// A worker on `jobs` (the app's: [`Jobs::shared`]).
    pub fn on(jobs: Jobs) -> Worker {
        let (tx, rx) = channel();
        Worker {
            jobs,
            tx,
            rx,
            generation: Arc::new(AtomicU64::new(0)),
            asked: Arc::default(),
        }
    }

    /// Asks for the copy for `key`, which holds about `need` bytes while it
    /// is made (the source's part, the stretch's working copy and the
    /// copy). False if the worker is going.
    pub fn send(&self, key: CopyKey, spec: JobSpec, need: u64) -> bool {
        let id = service_key(&key);
        {
            let mut asked = self.asked.lock().unwrap_or_else(|e| e.into_inner());
            match asked.insert(id, key) {
                // Asked for already, and still on its way.
                Some(old) if old == key && self.jobs.has(id) => return true,
                Some(old) if old != key && self.tx.send(Done::Cancelled(old)).is_err() => {
                    return false;
                }
                _ => {}
            }
        }
        let g = self.generation.load(Ordering::Relaxed);
        let (current, done, asked) = (self.generation.clone(), self.tx.clone(), self.asked.clone());
        let (refused, told) = (self.tx.clone(), self.asked.clone());
        self.jobs.submit(
            jobs::Job::new(Lane::Sound, id, move |run| {
                let stale = || current.load(Ordering::Relaxed) != g || run.cancelled();
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
                                // The job (and the source's part it held)
                                // goes first, so the copy into an Arc fits
                                // in what the job promised.
                                let out = job.into_output().unwrap_or_default();
                                break Done::Copy(key, Arc::from(out), t.elapsed());
                            }
                        },
                    }
                };
                // Said only while it is still the newest copy asked for its
                // region: one taken over was said cancelled already.
                if forget(&asked, id, key) {
                    let _ = done.send(result);
                }
            })
            .needs(need, move |why| {
                if forget(&told, id, key) {
                    let _ = refused.send(Done::Refused(key, why));
                }
            }),
        );
        true
    }

    /// Drops every job asked for so far, the one running included (it
    /// stops within a step).
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A finished (or failed, cancelled or refused) job, if one is waiting.
    pub fn try_recv(&self) -> Option<Done> {
        self.rx.try_recv().ok()
    }

    /// Waits up to `timeout` for the next one (for tests and tools).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Done> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// Copies asked for and not yet back.
    pub fn asked(&self) -> usize {
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Takes `key` off the copies asked for if it is still the newest for its
/// region `id`: true then, and its result is said.
fn forget(asked: &Asked, id: Key, key: CopyKey) -> bool {
    let mut asked = asked.lock().unwrap_or_else(|e| e.into_inner());
    if asked.get(&id) == Some(&key) {
        asked.remove(&id);
        true
    } else {
        false
    }
}

impl Drop for Worker {
    /// What is left is cancelled: the job running stops within a step, and
    /// the ones waiting end as soon as they start.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// A stretch or pitch shift of one layer's region that makes a new source
/// (the Process menu): `ratio` times as long and `semitones` higher. Make
/// the [`Process::job`], run it on a worker, then [`Process::finish`] puts
/// the result in the layer's place as one undo step. A stretched region's
/// pins are made part of the new audio: its hits land on their beats, and
/// the new region has no pins.
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
        self.spec(project)?.start()
    }

    /// What the job needs, owned, for a worker to start it there (copying
    /// the region's frames out and finding the hits are part of the work).
    pub fn spec(&self, project: &Project) -> Result<JobSpec, StretchError> {
        let r = self.region(project)?;
        let s = project.source(r.source).ok_or(StretchError::Empty)?;
        if r.start >= r.end || r.end > s.frames() {
            return Err(StretchError::Empty);
        }
        // Pins a stretch would follow are made part of the new audio.
        let map = match r.warp.as_ref().filter(|w| w.pinned()) {
            Some(w) => {
                let pts = w.points(r.start, r.end, s.rate());
                let k = r.frames() as f64 * self.ratio / pts[pts.len() - 1].1;
                TimeMap::new(pts.iter().map(|&(f, b)| (f, b * k)).collect())?
            }
            None => TimeMap::ratio(r.frames(), self.ratio)?,
        };
        let settings = Settings {
            semitones: self.semitones,
            ..Settings::default()
        };
        Ok(JobSpec {
            source: s.clone(),
            span: r.start..r.end,
            map,
            settings,
        })
    }

    /// Puts the finished job's audio in the layer's place (one undo step)
    /// and returns the new source's id. The region it was made from must
    /// still be there.
    pub fn finish(&self, project: &mut Project, audio: Vec<f32>) -> Result<SourceId, ModelError> {
        let (new, scale) = self.sound(project, audio)?;
        project.use_processed(self.pad, self.layer, new, scale)
    }

    /// The finished job's audio as the new sound, named for what was done
    /// ("Break x0.77 +3st"), and how many of its frames each of the
    /// region's became: what [`Project::use_processed`] takes. The region
    /// it was made from must still be there.
    pub fn sound(&self, project: &Project, audio: Vec<f32>) -> Result<(Source, f64), ModelError> {
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
        Ok((new, scale))
    }
}

fn tempo_ok(bpm: f64) -> f64 {
    if bpm.is_finite() {
        bpm.clamp(*TEMPO.start(), *TEMPO.end())
    } else {
        120.0
    }
}
