//! The Sample tab's views of its sounds ([`SourceView`]: peaks, mono mix,
//! hits, tempo and root note), worked out on a thread of their own so a
//! long sound never holds up the screen.
//!
//! A sound shown for the first time gets a flat line of its length at
//! once, then its peaks (about a seventh of the work), then the rest; each
//! step lands on a later [`Views::take_done`], and the page shows it then.
//! The sound shown last is worked out first. Views are kept by the sound's
//! id (a hash of its audio), so the same sound in another kit, or a kit
//! loaded again, is never analysed twice while it stays loaded.
//!
//! A sound's pitch over time (for the Slice tool's Pitch method) is followed
//! only when asked for ([`Views::follow_pitch`]), on the same worker, and
//! lands in the sound's view the same way.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use mpfree_sampler::model::{Source, SourceId};
use mpfree_sampler_ui::source::{PitchTrack, SourceView};

/// What the worker does for a sound.
enum Job {
    /// Its view: the peaks, then the rest.
    View(Source),
    /// Its pitch over time, in its view's mono mix at its rate.
    Pitch(Arc<SourceView>, u32),
}

impl Job {
    fn id(&self) -> SourceId {
        match self {
            Job::View(s) => s.id(),
            Job::Pitch(v, _) => v.id,
        }
    }
}

/// What the worker sends back.
enum Done {
    View(Arc<SourceView>),
    Pitch(SourceId, Option<Arc<PitchTrack>>),
}

/// The sounds waiting for the worker, the first next.
#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    stop: bool,
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

/// Boxed, so the page (and every `Player` the screen tests build in one
/// debug-build stack frame) stays as small as with the plain map before.
pub struct Views(Box<Inner>);

pub struct Inner {
    made: HashMap<SourceId, Arc<SourceView>>,
    /// Sounds whose pitch is asked for and not yet followed.
    pitch_asked: HashSet<SourceId>,
    queue: Shared,
    rx: Receiver<Done>,
    /// False when the worker couldn't start: views are then worked out
    /// in [`Views::get`], as before.
    worker: bool,
    /// A view came from a cache file since the last [`Views::take_done`].
    fresh: bool,
}

impl Views {
    /// Starts the worker (`mpfree-analyse`), off the sound's cores and at
    /// the background priority the loaders use.
    pub fn new() -> Views {
        let queue: Shared = Arc::default();
        let (tx, rx) = mpsc::channel();
        let q = queue.clone();
        let worker = std::thread::Builder::new()
            .name("mpfree-analyse".into())
            .spawn(move || {
                let _ = crate::cores::background(); // off the sound's cores and priority
                work(&q, &tx);
            })
            .is_ok();
        Views(Box::new(Inner {
            made: HashMap::new(),
            pitch_asked: HashSet::new(),
            queue,
            rx,
            worker,
            fresh: false,
        }))
    }
}

impl std::ops::Deref for Views {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl std::ops::DerefMut for Views {
    fn deref_mut(&mut self) -> &mut Inner {
        &mut self.0
    }
}

impl Inner {
    /// The view of `s` as far as it is worked out (a flat line at first),
    /// and the rest asked for, ahead of anything else waiting.
    pub fn get(&mut self, s: &Source) -> Arc<SourceView> {
        if let Some(v) = self.made.get(&s.id()) {
            if !v.ready {
                self.ask(s);
            }
            return v.clone();
        }
        if !self.worker {
            let v = Arc::new(SourceView::analyse(s));
            self.made.insert(s.id(), v.clone());
            return v;
        }
        let v = Arc::new(SourceView::pending(s));
        self.made.insert(s.id(), v.clone());
        self.ask(s);
        v
    }

    /// The whole view of `s` now, worked out here if the worker hasn't
    /// finished it: for a step that needs the result at once (Map to keys
    /// needs the note heard).
    pub fn full(&mut self, s: &Source) -> Arc<SourceView> {
        match self.made.get(&s.id()) {
            Some(v) if v.ready => v.clone(),
            _ => {
                let v = Arc::new(SourceView::analyse(s));
                self.made.insert(s.id(), v.clone());
                v
            }
        }
    }

    /// A view read from its sound's cache file (`SourceView::from_cache`),
    /// ready: kept for its sound in place of one still being worked out
    /// (never of a ready one), and the sound no longer waits for the
    /// worker. The next [`Views::take_done`] says a view changed.
    pub fn known(&mut self, v: Arc<SourceView>) {
        if !v.ready || self.made.get(&v.id).is_some_and(|old| old.ready) {
            return;
        }
        let (lock, _) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.jobs
                .retain(|j| !matches!(j, Job::View(s) if s.id() == v.id));
        }
        self.made.insert(v.id, v);
        self.fresh = true;
    }

    /// The view of the sound `id`, if it is worked out: for the save, which
    /// keeps it beside the sound.
    pub fn ready(&self, id: SourceId) -> Option<Arc<SourceView>> {
        self.made.get(&id).filter(|v| v.ready).cloned()
    }

    /// Follows the pitch of `s` (rate and mono mix) on the worker, once,
    /// for the Slice tool's Pitch method: the view with it lands on a later
    /// [`Views::take_done`]. Nothing while the sound has no mono mix yet
    /// (asked again when it has), or once it is followed or asked for.
    pub fn follow_pitch(&mut self, s: &Source) {
        let Some(v) = self.made.get(&s.id()).cloned() else {
            return;
        };
        if v.pitch.is_some() || v.mono.is_empty() || self.pitch_asked.contains(&s.id()) {
            return;
        }
        if !self.worker {
            if let Some(t) = v.follow_pitch(s.rate()) {
                self.made.insert(s.id(), Arc::new(v.with_pitch(t)));
                self.fresh = true;
            }
            return;
        }
        let (lock, cv) = &*self.queue;
        let Ok(mut q) = lock.lock() else {
            return;
        };
        self.pitch_asked.insert(s.id());
        // After the sound's own view, if that is waiting: else first.
        let at = q
            .jobs
            .iter()
            .position(|j| matches!(j, Job::View(w) if w.id() == s.id()))
            .map_or(0, |i| i + 1);
        q.jobs.insert(at, Job::Pitch(v, s.rate()));
        cv.notify_one();
    }

    /// Takes in what the worker finished since the last call. True when a
    /// view changed, for the page to show it.
    pub fn take_done(&mut self) -> bool {
        let mut changed = std::mem::take(&mut self.fresh);
        while let Ok(d) = self.rx.try_recv() {
            match d {
                Done::View(v) => {
                    // A view forgotten since (its sound gone) stays
                    // forgotten, and a step never replaces a later one. A
                    // pitch followed meanwhile stays with it.
                    if let Some(old) = self.made.get_mut(&v.id) {
                        if !old.ready {
                            *old = match (&old.pitch, &v.pitch) {
                                (Some(p), None) => Arc::new(v.with_pitch(p.clone())),
                                _ => v,
                            };
                            changed = true;
                        }
                    }
                }
                Done::Pitch(id, t) => {
                    self.pitch_asked.remove(&id);
                    if let (Some(old), Some(t)) = (self.made.get_mut(&id), t) {
                        if old.pitch.is_none() {
                            *old = Arc::new(old.with_pitch(t));
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }

    /// Forgets the views of sounds not in `keep`, and stops waiting for
    /// them.
    pub fn keep_only(&mut self, keep: &HashSet<SourceId>) {
        self.made.retain(|id, _| keep.contains(id));
        self.pitch_asked.retain(|id| keep.contains(id));
        let (lock, _) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.jobs.retain(|j| keep.contains(&j.id()));
        }
    }

    /// Whether any view asked for (or pitch) is still being worked out.
    #[allow(dead_code)] // the tests wait on it
    pub fn busy(&self) -> bool {
        self.made.values().any(|v| !v.ready) || !self.pitch_asked.is_empty()
    }

    /// Puts `s` first in the worker's queue (once).
    fn ask(&self, s: &Source) {
        let (lock, cv) = &*self.queue;
        let Ok(mut q) = lock.lock() else {
            return;
        };
        if let Some(i) = q
            .jobs
            .iter()
            .position(|j| matches!(j, Job::View(w) if w.id() == s.id()))
        {
            if i == 0 {
                return;
            }
            q.jobs.remove(i);
        }
        q.jobs.push_front(Job::View(s.clone()));
        cv.notify_one();
    }
}

impl Drop for Inner {
    /// The worker stops after the sound it is on.
    fn drop(&mut self) {
        let (lock, cv) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.stop = true;
            q.jobs.clear();
        }
        cv.notify_all();
    }
}

/// The worker: each sound's peaks, then the rest, sent as each is done;
/// or a sound's pitch.
fn work(queue: &Shared, tx: &Sender<Done>) {
    let (lock, cv) = &**queue;
    loop {
        let job = {
            let Ok(mut q) = lock.lock() else { return };
            loop {
                if q.stop {
                    return;
                }
                if let Some(j) = q.jobs.pop_front() {
                    break j;
                }
                q = match cv.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        let s = match job {
            Job::View(s) => s,
            Job::Pitch(v, rate) => {
                if tx.send(Done::Pitch(v.id, v.follow_pitch(rate))).is_err() {
                    return;
                }
                continue;
            }
        };
        let quick = Arc::new(SourceView::quick(&s));
        if tx.send(Done::View(quick.clone())).is_err() {
            return;
        }
        if tx.send(Done::View(Arc::new(quick.finish(&s)))).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// `secs` of a click every half second, stereo, at 44.1 kHz.
    fn clicks(name: &str, secs: usize, seed: f32) -> Source {
        let n = secs * 44_100;
        let data = (0..n * 2)
            .map(|i| {
                let f = i / 2;
                let k = f % 22_050;
                seed * (k as f32 * 0.3).sin() * (-(k as f32) / 800.0).exp()
            })
            .collect();
        Source::new(name, 44_100, 2, data).unwrap()
    }

    fn settle(v: &mut Views) {
        let end = Instant::now() + Duration::from_secs(20);
        while v.busy() {
            assert!(Instant::now() < end, "the worker never finished");
            std::thread::sleep(Duration::from_millis(2));
            v.take_done();
        }
    }

    #[test]
    fn a_sound_shows_a_flat_line_at_once_then_its_peaks_then_the_rest() {
        let s = clicks("Break", 8, 0.8);
        let mut v = Views::new();
        let first = v.get(&s);
        assert!(!first.ready);
        assert_eq!(first.peaks.frames(), s.frames());
        assert!(first.mono.is_empty() && first.onsets.is_none());
        let mut steps = Vec::new();
        let end = Instant::now() + Duration::from_secs(20);
        while v.busy() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
            if v.take_done() {
                let now = v.get(&s);
                steps.push((now.ready, now.mono.len()));
            }
        }
        // The peaks and mono mix first, the hits and tempo after (the two
        // may land together).
        assert_eq!(steps.last(), Some(&(true, s.frames() as usize)));
        let done = v.get(&s);
        let whole = SourceView::analyse(&s);
        assert_eq!(done.tempo, whole.tempo);
        assert_eq!(done.root_note, whole.root_note);
        assert_eq!(
            done.onsets.as_ref().map(|o| o.pick(0.5)),
            whole.onsets.as_ref().map(|o| o.pick(0.5))
        );
        // Asked again, it is the same view: nothing is worked out twice.
        assert!(Arc::ptr_eq(&done, &v.get(&s)));
        assert!(!v.busy());
    }

    #[test]
    fn a_sounds_pitch_is_followed_on_the_worker_once_asked_and_kept_with_its_view() {
        // A tone at A3 for a second, then A4.
        let tone = |hz: f32, n: usize| {
            (0..n).map(move |i| 0.4 * (i as f32 * hz / 44_100.0 * std::f32::consts::TAU).sin())
        };
        let data: Vec<f32> = tone(220.0, 44_100).chain(tone(440.0, 44_100)).collect();
        let s = Source::new("Line", 44_100, 1, data).unwrap();
        let mut v = Views::new();
        // Asked before the view is made, nothing is queued.
        v.follow_pitch(&s);
        assert!(!v.busy());
        v.get(&s);
        // Asked while the view is worked out: followed after it, and kept
        // when the view's last step lands.
        let end = Instant::now() + Duration::from_secs(20);
        while v.get(&s).mono.is_empty() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
            v.take_done();
        }
        v.follow_pitch(&s);
        assert!(v.busy());
        settle(&mut v);
        let done = v.get(&s);
        assert!(done.ready);
        let t = done.pitch.clone().expect("the pitch, followed");
        let cuts = t.changes(1.0, 2_205);
        assert_eq!(cuts.len(), 1, "{cuts:?}");
        assert!(cuts[0].abs_diff(44_100) < 441, "{cuts:?}");
        // Asked again: nothing more to do, the same view.
        v.follow_pitch(&s);
        assert!(!v.busy());
        assert!(Arc::ptr_eq(&done, &v.get(&s)));
    }

    #[test]
    fn the_sound_shown_last_is_worked_out_first() {
        let mut v = Views::new();
        let long: Vec<Source> = (0..4)
            .map(|i| clicks(&format!("Long {i}"), 20, 0.5 + 0.1 * i as f32))
            .collect();
        for s in &long {
            v.get(s);
        }
        let short = clicks("Hit", 1, 0.9);
        v.get(&short);
        let end = Instant::now() + Duration::from_secs(20);
        while !v.get(&short).ready {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
            v.take_done();
        }
        // At most the long sound the worker was already on came first.
        let ready = long.iter().filter(|s| v.get(s).ready).count();
        assert!(ready <= 1, "{ready} long sounds before the one shown");
        settle(&mut v);
    }

    #[test]
    fn sounds_no_longer_loaded_are_forgotten() {
        let (a, b) = (clicks("A", 1, 0.7), clicks("B", 1, 0.6));
        let mut v = Views::new();
        v.get(&a);
        v.get(&b);
        settle(&mut v);
        let kept = v.get(&a);
        v.keep_only(&HashSet::from([a.id()]));
        assert!(
            Arc::ptr_eq(&kept, &v.get(&a)),
            "a sound still here keeps its view"
        );
        assert!(!v.get(&b).ready, "a forgotten one is worked out again");
        settle(&mut v);
    }

    #[test]
    fn a_view_from_a_cache_file_is_ready_at_once_and_never_worked_out_again() {
        let s = clicks("Loop", 4, 0.8);
        let bytes = SourceView::analyse(&s).to_cache().unwrap();
        let mut v = Views::new();
        // Shown first (the worker is asked), then its cache comes in.
        assert!(!v.get(&s).ready);
        let cached = Arc::new(SourceView::from_cache(&s, &bytes).unwrap());
        v.known(cached.clone());
        assert!(v.take_done(), "the page is told");
        assert!(Arc::ptr_eq(&cached, &v.get(&s)));
        assert!(Arc::ptr_eq(&cached, &v.ready(s.id()).unwrap()));
        // What the worker sends for it later doesn't replace it.
        std::thread::sleep(Duration::from_millis(100));
        v.take_done();
        assert!(Arc::ptr_eq(&cached, &v.get(&s)));
        assert!(!v.busy());
        // A cache never replaces a view already worked out.
        let other = clicks("Other", 1, 0.5);
        let made = v.full(&other);
        let again = SourceView::from_cache(&other, &made.to_cache().unwrap()).unwrap();
        v.known(Arc::new(again));
        assert!(Arc::ptr_eq(&made, &v.get(&other)));
        // A sound not worked out yet has no view for the save.
        let new = clicks("New", 1, 0.4);
        v.get(&new);
        assert!(v.ready(new.id()).is_none());
        settle(&mut v);
    }

    #[test]
    fn the_whole_view_can_be_had_at_once() {
        let s = clicks("Keys", 2, 0.8);
        let mut v = Views::new();
        let whole = v.full(&s);
        assert!(whole.ready);
        assert!(Arc::ptr_eq(&whole, &v.get(&s)));
        // The worker's copy, if it comes later, doesn't replace it.
        std::thread::sleep(Duration::from_millis(50));
        v.take_done();
        assert!(Arc::ptr_eq(&whole, &v.get(&s)));
    }
}
