//! The Sample tab's views of its sounds ([`SourceView`]: peaks, hits, tempo
//! and root note), worked out on the sampler's job service
//! ([`mpfree_sampler::jobs`], its sound lane) so a long sound never holds
//! up the screen.
//!
//! A sound shown for the first time gets a flat line of its length at
//! once, then its peaks (about a seventh of the work), then the rest; each
//! step lands on a later [`Views::take_done`], and the page shows it then.
//! The sound shown last is worked out first. Views are kept by the sound's
//! id (a hash of its audio), so the same sound in another kit, or a kit
//! loaded again, is never analysed twice while it stays loaded.
//!
//! A sound's pitch over time (for the Slice tool's Pitch method) is followed
//! only when asked for ([`Views::follow_pitch`]), on the same lane, and
//! lands in the sound's view the same way.
//!
//! Each job says the memory it holds: a view, its peaks and its sound mixed
//! to mono for a moment; a pitch, its sound mixed to mono and brought down
//! to 8 kHz. A view keeps no mono copy (the sampler plan's step 8c). One
//! the floor can't spare is refused: the sound keeps its flat line, the
//! log says why once, and it is asked for again every [`RETRY`] while it
//! is loaded. Nothing is ever worked out on the screen instead, not even
//! when the service's workers can't start: the jobs wait for them.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_sampler::jobs::{Job, Jobs, Key, Lane, Order};
use mpfree_sampler::model::{Source, SourceId};
use mpfree_sampler_ui::source::{PitchTrack, PyramidPeaks, SourceView};

/// Where a sound's loudness summary is ([`Views::summary`]).
#[derive(Clone)]
pub enum Summary {
    Ready(Arc<PyramidPeaks>),
    /// Still being worked out, or not asked for yet.
    Pending,
    /// Worked out without one: a sound the summary can't hold.
    None,
}

/// A view's jobs on the service.
const VIEW: &str = "view";
/// A pitch's jobs on the service.
const PITCH: &str = "pitch";

/// How long a sound refused for memory waits before it is asked for again.
pub const RETRY: Duration = Duration::from_secs(5);

/// Memory a job holds beyond what is worked out from the sound's length.
const SLACK: u64 = 1_000_000;

/// What working out a sound's view holds, in bytes: its peaks (three
/// 16-bit numbers a channel every 64 frames, and the coarser levels), kept,
/// and its mono mix as floats, made to find its hits and note and gone
/// after.
pub fn view_need(frames: u64, channels: u8) -> u64 {
    frames
        .saturating_mul(4)
        .saturating_add(frames.saturating_mul(u64::from(channels)) / 8)
        .saturating_add(SLACK)
}

/// What following a sound's pitch holds: its mono mix as floats, and that
/// brought down to about 8 kHz.
pub fn pitch_need(frames: u64) -> u64 {
    frames.saturating_mul(5).saturating_add(SLACK)
}

fn view_key(id: SourceId) -> Key {
    Key::new(VIEW, id.0)
}

fn pitch_key(id: SourceId) -> Key {
    Key::new(PITCH, id.0)
}

/// What a job sends back.
enum Done {
    View(Arc<SourceView>),
    Pitch(SourceId, Option<Arc<PitchTrack>>),
    /// Refused for memory: why, in a musician's words, and the sound's name.
    Refused(SourceId, String, String),
}

/// Boxed, so the page (and every `Player` the screen tests build in one
/// debug-build stack frame) stays as small as with the plain map before.
pub struct Views(Box<Inner>);

pub struct Inner {
    made: HashMap<SourceId, Arc<SourceView>>,
    /// Sounds whose pitch is asked for and not yet followed.
    pitch_asked: HashSet<SourceId>,
    /// Sounds whose view was refused for memory, and when to ask again.
    refused: HashMap<SourceId, Instant>,
    /// Sounds whose refusal is logged already: logged once each.
    told: HashSet<SourceId>,
    /// The sounds themselves, to ask for them again.
    waiting: HashMap<SourceId, Source>,
    jobs: Jobs,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    /// A view came from a cache file since the last [`Views::take_done`].
    fresh: bool,
    /// Log lines for the page to write ([`Views::take_lines`]).
    lines: Vec<String>,
    /// How long a refused sound waits ([`RETRY`]; tests shorten it).
    retry: Duration,
}

impl Views {
    /// Views worked out on `jobs`, the page's job service.
    pub fn new(jobs: Jobs) -> Views {
        let (tx, rx) = mpsc::channel();
        Views(Box::new(Inner {
            made: HashMap::new(),
            pitch_asked: HashSet::new(),
            refused: HashMap::new(),
            told: HashSet::new(),
            waiting: HashMap::new(),
            jobs,
            tx,
            rx,
            fresh: false,
            lines: Vec::new(),
            retry: RETRY,
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
            let v = v.clone();
            if !v.ready {
                self.ask(s);
            }
            return v;
        }
        let v = Arc::new(SourceView::pending(s));
        self.made.insert(s.id(), v.clone());
        self.ask(s);
        v
    }

    /// A view read from its sound's cache file (`SourceView::from_cache`),
    /// ready: kept for its sound in place of one still being worked out
    /// (never of a ready one), and the sound no longer waits for the
    /// worker. The next [`Views::take_done`] says a view changed.
    pub fn known(&mut self, v: Arc<SourceView>) {
        if !v.ready || self.made.get(&v.id).is_some_and(|old| old.ready) {
            return;
        }
        self.settled(v.id);
        self.jobs.cancel(view_key(v.id));
        self.made.insert(v.id, v);
        self.fresh = true;
    }

    /// Where the loudness summary (the peak pyramid) of sound `id` is,
    /// without asking for it: the first thing the worker works out, so it
    /// comes long before the rest of the view.
    pub fn summary(&self, id: SourceId) -> Summary {
        match self.made.get(&id) {
            Some(v) => match &v.pyramid {
                Some(p) => Summary::Ready(p.clone()),
                None if v.ready => Summary::None,
                None => Summary::Pending,
            },
            None => Summary::Pending,
        }
    }

    /// The view of the sound `id`, if it is worked out: for the save, which
    /// keeps it beside the sound.
    pub fn ready(&self, id: SourceId) -> Option<Arc<SourceView>> {
        self.made.get(&id).filter(|v| v.ready).cloned()
    }

    /// Follows the pitch of `s` on the service, once, for the Slice tool's
    /// Pitch method: the view with it lands on a later
    /// [`Views::take_done`]. Nothing before its peaks are worked out (asked
    /// again when they are), or once it is followed or asked for.
    pub fn follow_pitch(&mut self, s: &Source) {
        let Some(v) = self.made.get(&s.id()).cloned() else {
            return;
        };
        if v.pitch.is_some() || v.mono.is_none() || self.pitch_asked.contains(&s.id()) {
            return;
        }
        self.pitch_asked.insert(s.id());
        let (id, name, sound) = (s.id(), s.name.clone(), s.clone());
        let (tx, why) = (self.tx.clone(), self.tx.clone());
        // After the sound's own view, if that is waiting: else first.
        self.jobs.submit(
            Job::new(Lane::Sound, pitch_key(id), move |_| {
                let _ = tx.send(Done::Pitch(id, v.follow_pitch(&sound)));
            })
            .order(Order::After(view_key(id)))
            .needs(pitch_need(s.frames()), move |w| {
                let _ = why.send(Done::Refused(id, w, format!("{name}'s pitch")));
            }),
        );
    }

    /// Takes in what the jobs finished since the last call, and asks again
    /// for sounds refused for memory a while ago. True when a view changed,
    /// for the page to show it.
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
                            let ready = v.ready;
                            *old = match (&old.pitch, &v.pitch) {
                                (Some(p), None) => Arc::new(v.with_pitch(p.clone())),
                                _ => v,
                            };
                            if ready {
                                let id = old.id;
                                self.settled(id);
                            }
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
                Done::Refused(id, why, what) => self.refused_for(id, &why, &what),
            }
        }
        self.ask_again();
        changed
    }

    /// The log lines since the last call, for the page to write.
    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    /// Forgets the views of sounds not in `keep`, and stops waiting for
    /// them.
    pub fn keep_only(&mut self, keep: &HashSet<SourceId>) {
        self.made.retain(|id, _| keep.contains(id));
        self.pitch_asked.retain(|id| keep.contains(id));
        self.refused.retain(|id, _| keep.contains(id));
        self.told.retain(|id| keep.contains(id));
        self.waiting.retain(|id, _| keep.contains(id));
        self.jobs
            .retain(|k| (k.kind != VIEW && k.kind != PITCH) || keep.contains(&SourceId(k.id)));
    }

    /// Holds the service's workers (`true`) before their next job, or lets
    /// them go on.
    #[cfg(test)]
    pub fn hold(&self, held: bool) {
        self.jobs.hold(held);
    }

    /// Whether any view asked for (or pitch) is still being worked out: a
    /// sound refused for memory isn't, until it is asked for again.
    #[allow(dead_code)] // the tests wait on it
    pub fn busy(&self) -> bool {
        self.made
            .values()
            .any(|v| !v.ready && !self.refused.contains_key(&v.id))
            || !self.pitch_asked.is_empty()
    }

    /// Whether sound `id`'s view was refused for memory (and isn't worked
    /// out since): it waits for its next try.
    pub fn is_refused(&self, id: SourceId) -> bool {
        self.told.contains(&id)
    }

    /// How many sounds wait for memory to work out their view.
    #[cfg(test)]
    pub fn refused_now(&self) -> usize {
        self.refused.len()
    }

    /// Shortens [`RETRY`], for a test.
    #[cfg(test)]
    pub fn set_retry(&mut self, d: Duration) {
        self.retry = d;
    }

    /// Puts `s` first on the service (once): a job already waiting moves to
    /// the front, and one running is left to finish. A sound refused for
    /// memory waits for its turn to be asked again.
    fn ask(&mut self, s: &Source) {
        if self.refused.contains_key(&s.id()) {
            return;
        }
        self.waiting.insert(s.id(), s.clone());
        if self.jobs.promote(view_key(s.id())) {
            return;
        }
        let (tx, why) = (self.tx.clone(), self.tx.clone());
        let (id, name) = (s.id(), s.name.clone());
        let sound = s.clone();
        self.jobs.submit(
            Job::new(Lane::Sound, view_key(id), move |run| {
                let quick = Arc::new(SourceView::quick(&sound));
                if tx.send(Done::View(quick.clone())).is_err() || run.cancelled() {
                    return;
                }
                let _ = tx.send(Done::View(Arc::new(quick.finish(&sound))));
            })
            .order(Order::First)
            .needs(view_need(s.frames(), s.channels()), move |w| {
                let _ = why.send(Done::Refused(id, w, format!("{name}'s waveform")));
            }),
        );
    }

    /// A refusal from the service: logged the first time for the sound,
    /// then asked for again after [`RETRY`] while it is loaded.
    fn refused_for(&mut self, id: SourceId, why: &str, what: &str) {
        if what.ends_with("'s pitch") {
            self.pitch_asked.remove(&id);
            self.lines.push(format!(
                "sample: {what} not followed: {why}; asked for again when the Slice tool next needs it"
            ));
            return;
        }
        if !self.made.contains_key(&id) {
            return;
        }
        self.refused.insert(id, Instant::now() + self.retry);
        if self.told.insert(id) {
            self.lines.push(format!(
                "sample: {what} waits: {why}; asked for again every {} s while it is loaded",
                self.retry.as_secs().max(1)
            ));
        }
    }

    /// Asks again for each refused sound whose wait is over.
    fn ask_again(&mut self) {
        if self.refused.is_empty() {
            return;
        }
        let now = Instant::now();
        let due: Vec<SourceId> = self
            .refused
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            // Not logged again if the service refuses it once more: the
            // wait starts over then.
            self.refused.remove(&id);
            if let Some(s) = self.waiting.get(&id).cloned() {
                self.ask(&s);
            }
        }
    }

    /// The sound's view is worked out: it waits for nothing more.
    fn settled(&mut self, id: SourceId) {
        self.refused.remove(&id);
        self.told.remove(&id);
        self.waiting.remove(&id);
    }
}

impl Drop for Inner {
    /// The views' jobs still waiting are dropped, and the one running is
    /// told to stop.
    fn drop(&mut self) {
        self.jobs.retain(|k| k.kind != VIEW && k.kind != PITCH);
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

    fn views() -> Views {
        Views::new(Jobs::new(mpfree_sampler::jobs::Config::default()))
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
        let mut v = views();
        let first = v.get(&s);
        assert!(!first.ready);
        assert_eq!(first.peaks.frames(), s.frames());
        assert!(first.mono.is_none() && first.onsets.is_none());
        let mut steps = Vec::new();
        let end = Instant::now() + Duration::from_secs(20);
        while v.busy() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
            if v.take_done() {
                let now = v.get(&s);
                steps.push((now.ready, now.pyramid.is_some()));
            }
        }
        // The peaks first, the hits and tempo after (the two may land
        // together).
        assert_eq!(steps.last(), Some(&(true, true)));
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
        let mut v = views();
        // Asked before the view is made, nothing is queued.
        v.follow_pitch(&s);
        assert!(!v.busy());
        v.get(&s);
        // Asked while the view is worked out: followed after it, and kept
        // when the view's last step lands.
        let end = Instant::now() + Duration::from_secs(20);
        while v.get(&s).mono.is_none() {
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
        let mut v = views();
        let long: Vec<Source> = (0..4)
            .map(|i| clicks(&format!("Long {i}"), 20, 0.5 + 0.1 * i as f32))
            .collect();
        for s in &long {
            v.get(s);
        }
        let short = clicks("Hit", 1, 0.9);
        v.get(&short);
        // Watched without asking again (which would move it first anyway).
        let end = Instant::now() + Duration::from_secs(20);
        while v.ready(short.id()).is_none() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
            v.take_done();
        }
        // At most the long sound the worker was already on came first.
        let ready = long.iter().filter(|s| v.ready(s.id()).is_some()).count();
        assert!(ready <= 1, "{ready} long sounds before the one shown");
        settle(&mut v);
    }

    #[test]
    fn sounds_no_longer_loaded_are_forgotten() {
        let (a, b) = (clicks("A", 1, 0.7), clicks("B", 1, 0.6));
        let mut v = views();
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
        let mut v = views();
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
        v.get(&other);
        settle(&mut v);
        let made = v.get(&other);
        let again = SourceView::from_cache(&other, &made.to_cache().unwrap()).unwrap();
        v.known(Arc::new(again));
        assert!(Arc::ptr_eq(&made, &v.get(&other)));
        // A sound not worked out yet has no view for the save.
        let new = clicks("New", 1, 0.4);
        v.get(&new);
        assert!(v.ready(new.id()).is_none());
        settle(&mut v);
    }

    /// Memory just short: a job that needs more than a megabyte is refused.
    fn short_service() -> Jobs {
        Jobs::new(mpfree_sampler::jobs::Config {
            free: || Some(257_000_000),
            room: crate::memory::room_in,
            ..mpfree_sampler::jobs::Config::default()
        })
    }

    #[test]
    fn a_view_the_floor_cant_spare_keeps_its_flat_line_says_why_once_and_is_asked_again() {
        // 257 MB free, 256 MB kept spare: a 2-second take's view needs
        // about 1.7 MB.
        let s = clicks("Take", 2, 0.8);
        let jobs = short_service();
        let mut v = Views::new(jobs.clone());
        v.set_retry(Duration::from_millis(20));
        assert!(!v.get(&s).ready);
        let end = Instant::now() + Duration::from_secs(20);
        while v.refused_now() == 0 {
            assert!(Instant::now() < end, "never refused");
            std::thread::sleep(Duration::from_millis(1));
            v.take_done();
        }
        let lines = v.take_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("sample: Take's waveform waits: not enough memory: needs about"),
            "{}",
            lines[0]
        );
        assert!(!v.busy(), "a refused sound isn't being worked out");
        assert!(matches!(v.summary(s.id()), Summary::Pending));
        assert!(!v.get(&s).ready, "asked again only after the wait");
        // After the wait it is asked again, refused again, and not logged
        // again; nothing is ever worked out on the screen.
        let end = Instant::now() + Duration::from_secs(20);
        while jobs.counts().refused < 3 {
            assert!(Instant::now() < end, "never asked again");
            std::thread::sleep(Duration::from_millis(5));
            v.take_done();
        }
        assert!(v.take_lines().is_empty());
        assert!(!v.get(&s).ready);
        assert_eq!(jobs.counts().done, 0);
        // Once gone from the kits, it isn't asked for any more.
        v.keep_only(&HashSet::new());
        assert_eq!(v.refused_now(), 0);
    }

    /// Whether [`free_then_more`] says memory is short.
    static SHORT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

    fn free_then_more() -> Option<u64> {
        Some(if SHORT.load(std::sync::atomic::Ordering::Relaxed) {
            100_000_000
        } else {
            4_000_000_000
        })
    }

    #[test]
    fn a_refused_view_comes_once_memory_is_free_again() {
        let s = clicks("Loop", 2, 0.8);
        let jobs = Jobs::new(mpfree_sampler::jobs::Config {
            free: free_then_more,
            room: crate::memory::room_in,
            ..mpfree_sampler::jobs::Config::default()
        });
        let mut v = Views::new(jobs.clone());
        v.set_retry(Duration::from_millis(10));
        v.get(&s);
        let end = Instant::now() + Duration::from_secs(20);
        while v.refused_now() == 0 {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
            v.take_done();
        }
        SHORT.store(false, std::sync::atomic::Ordering::Relaxed);
        let end = Instant::now() + Duration::from_secs(20);
        while !v.get(&s).ready {
            assert!(Instant::now() < end, "never worked out");
            std::thread::sleep(Duration::from_millis(2));
            v.take_done();
        }
        assert_eq!(v.refused_now(), 0);
        assert_eq!(v.take_lines().len(), 1, "the one refusal, logged once");
        let whole = SourceView::analyse(&s);
        assert_eq!(v.get(&s).tempo, whole.tempo);
    }

    #[test]
    fn a_views_need_is_its_mono_mix_once_and_its_peaks() {
        // 25 minutes of stereo at 44.1 kHz: 265 MB of floats for a moment
        // (twice that before step 8c, which kept one in the view), 16.5 MB
        // of peaks.
        let frames = 66_150_000;
        assert_eq!(view_need(frames, 2), 264_600_000 + 16_537_500 + SLACK);
        assert_eq!(view_need(frames, 1), 264_600_000 + 8_268_750 + SLACK);
        assert_eq!(pitch_need(frames), 264_600_000 + 66_150_000 + SLACK);
    }

    #[test]
    fn views_are_worked_out_on_the_services_sound_lane() {
        let jobs = Jobs::new(mpfree_sampler::jobs::Config::default());
        let mut v = Views::new(jobs.clone());
        jobs.hold(true);
        let s = clicks("Hit", 1, 0.9);
        v.get(&s);
        assert!(jobs.has(Key::new(VIEW, s.id().0)));
        // Shown again: still one job.
        v.get(&s);
        assert_eq!(jobs.waiting(), 1);
        jobs.hold(false);
        settle(&mut v);
        assert!(v.get(&s).ready);
        assert_eq!(jobs.counts().done, 1);
        // The views go: their jobs go with them.
        let t = clicks("Other", 1, 0.5);
        jobs.hold(true);
        v.get(&t);
        drop(v);
        assert!(!jobs.has(Key::new(VIEW, t.id().0)));
        jobs.hold(false);
    }
}
