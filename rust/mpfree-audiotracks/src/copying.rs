//! The host making Stretch's copies (the audio tracks rethink, phase 5b).
//!
//! The track player plays a copy only if it has one ([`crate::copies`]);
//! this is how it gets them. On the audio thread, with the host:
//!
//! 1. **What is wanted.** Every few periods, and at the next after a
//!    command (it may have edited a region) or a copy coming back, once
//!    the tempo map has held for half a second (a tempo being turned asks
//!    for nothing; the map that comes with the tracks, as a project or a
//!    set's next song opens, counts as held at once, 5d.3): the keys of every song region on Stretch (over one
//!    steady tempo, or following the map where it changes over the
//!    region, 5d.1, the map and its place going with the want to the
//!    worker: [`Over`]) and of the clips' regions on Stretch the app lists
//!    ([`ClipWant`], at the tempo at the playhead, and at each tempo a
//!    tempo point in the next eight bars brings, 5d.2), most urgent first:
//!    song regions from the playhead to two bars on and the clips the app
//!    says play, and the copies the regions drifting on Keep pitch wait
//!    for ([`Priority::Protect`]), then what comes in the next eight
//!    bars and the clips set up to come ([`Priority::Prepared`]), then the
//!    rest ([`Priority::Speculative`]).
//! 2. **Room.** The copies share the one budget for derived audio with
//!    the sampler's Warps and the rest (`mpfree_stretch::budget`, 5b.3):
//!    the host says what they use, those held and those being made, and a
//!    copy is asked for only if it fits within the limit for its priority
//!    beside the others. One that wouldn't fit makes room by letting go of
//!    copies nothing wants, the one played longest ago first, never one
//!    playing; if there still isn't room it isn't made, and its region
//!    plays Repitch until room comes back.
//! 3. **Its source.** The recording's frames it is made from are read
//!    from memory into a buffer the worker makes, at [`READ_SPEED`]
//!    frames a frame played, one copy at a time.
//! 4. **Made on the scheduler.** The host's worker hands the frames to
//!    the stretch scheduler (`mpfree_stretch::sched`) at the copy's
//!    priority, keyed by the copy's key; the scheduler's worker makes it a
//!    step at a time and sends it back. A key no longer wanted is
//!    cancelled there.
//! 5. **In.** A copy that comes back still wanted goes into the track
//!    player; one for tracks since emptied, or no longer wanted, is
//!    dropped, never played.
//!
//! Not yet: copies of a take on its file (it plays Repitch; streaming
//! stays off on the MPC for now).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use mpfree_stretch::budget::{Budget, Cache};
use mpfree_stretch::sched::Priority;
use mpfree_transport::time::{TempoMap, Tick, PPQ};

use crate::clips::{ClipPlay, ClipRegion, MAX_CLIP_REGIONS};
use crate::copies::{CopyKey, CopySpec, Over};
use crate::{AudioTracks, FollowTempo, RecordingId, MAX_TRACKS};

/// Copies wanted at once, at most: past it the least urgent are left
/// out (counted).
pub const MAX_WANTED: usize = 256;
/// Copies with the scheduler at once, at most.
pub(crate) const MAX_MAKING: usize = 8;
/// Copies that couldn't be made, not asked for again until what is
/// wanted changes.
const MAX_FAILED: usize = 64;
/// Periods between looks at what is wanted (about 93 ms at 128 frames).
const WANT_EVERY: u64 = 32;
/// Recording frames read a frame played, while a copy's source is read.
pub const READ_SPEED: usize = 64;

/// An audio clip with regions on Stretch, as the app lists them
/// ([`crate::host::AudioHost::clip_copies`]): its regions (the first `n`),
/// its pass length, the frame its last region rings to, and how soon it
/// may play.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipWant {
    pub regions: [ClipRegion; MAX_CLIP_REGIONS],
    pub n: usize,
    pub len: Tick,
    pub last_frame: i64,
    pub priority: Priority,
    /// The tempo its copies are for: a launch's, the tempo at its line
    /// (5b.4); 0 for the tempo at the playhead.
    pub bpm: f64,
}

/// A copy wanted: its key, its take and channels, how urgent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Want {
    pub key: CopyKey,
    pub take: u16,
    pub channels: u8,
    pub priority: Priority,
    /// For a song region over a tempo change, the map and place its copy
    /// follows (5d.1).
    pub over: Option<Over>,
}

/// What the copies have done, for the debug lines and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyCounts {
    /// Copies made and taken in.
    pub made: u64,
    /// Copies that came back no longer wanted, or for tracks since
    /// emptied: dropped, never played.
    pub stale: u64,
    /// Copies left unmade for want of room in their share, or of a free
    /// place in the track player's table.
    pub no_room: u64,
    /// Copies the scheduler refused (its memory check) or that failed.
    pub refused: u64,
    pub failed: u64,
    /// Copies held let go to make room for others.
    pub let_go: u64,
    /// Copies wanted past [`MAX_WANTED`], left out.
    pub too_many: u64,
}

/// How far each copy with the scheduler has got, a slot for each of the
/// [`MAX_MAKING`] at most: the scheduler's worker writes it after each
/// step, the host reads it without a lock (for the clip card's "Making
/// its copy: 40 %", 5c). Each slot holds the copy's tag (the low 32 bits
/// of its key's id, never 0) over its progress's bits, so a slot taken
/// over by another copy is never read as this one's.
pub(crate) struct Progress([AtomicU64; MAX_MAKING]);

impl Progress {
    pub(crate) fn new() -> Progress {
        Progress(std::array::from_fn(|_| AtomicU64::new(0)))
    }

    /// The tag slot entries carry for `key`.
    pub(crate) fn tag(key: &CopyKey) -> u32 {
        (key.id() as u32).max(1)
    }

    /// Slot `slot` now says `key`'s copy is `p` done (0 to 1).
    pub(crate) fn set(&self, slot: u8, tag: u32, p: f32) {
        if let Some(a) = self.0.get(usize::from(slot)) {
            a.store(
                (u64::from(tag) << 32) | u64::from(p.clamp(0.0, 1.0).to_bits()),
                Ordering::Relaxed,
            );
        }
    }

    /// How far the copy tagged `tag` in slot `slot` has got: 0 until its
    /// worker has said.
    fn get(&self, slot: u8, tag: u32) -> f32 {
        let v = self
            .0
            .get(usize::from(slot))
            .map_or(0, |a| a.load(Ordering::Relaxed));
        if (v >> 32) as u32 == tag {
            f32::from_bits(v as u32)
        } else {
            0.0
        }
    }
}

/// Where an audio clip's tempo copies for a tempo stand (5c: the clip
/// card's foot, the cells' words), over its regions on Stretch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum CopyState {
    /// It needs none: no region on Stretch, at its own tempo, or past the
    /// speeds a copy is made at.
    #[default]
    None,
    /// Every copy it needs is in.
    Ready,
    /// One is being made, the least done this far (0 to 1).
    Making(f32),
    /// One waits its turn to be made.
    Waiting,
    /// One couldn't be made: no room in the budget for derived audio, its
    /// take on its file, or the stretch refused it. It is asked for again
    /// when what is wanted changes or room comes back.
    Unmade,
}

impl CopyState {
    /// Two regions' states as the clip's: the furthest from ready wins
    /// (couldn't be made, then waiting, then the least made).
    pub fn with(self, other: CopyState) -> CopyState {
        use CopyState::*;
        match (self, other) {
            (None, x) | (x, None) => x,
            (Unmade, _) | (_, Unmade) => Unmade,
            (Waiting, _) | (_, Waiting) => Waiting,
            (Making(a), Making(b)) => Making(a.min(b)),
            (Making(a), Ready) | (Ready, Making(a)) => Making(a),
            (Ready, Ready) => Ready,
        }
    }
}

/// What the host tells its worker about copies. Sent from the audio
/// thread, so [`Ask::Make`] isn't boxed, though a copy over a tempo change
/// carries the song's map (5d.1).
#[allow(clippy::large_enum_variant)]
pub(crate) enum Ask {
    /// A zeroed buffer of this many samples, for `key`'s source.
    Buffer { n: usize, key: CopyKey, gen: u32 },
    /// Make this copy on the scheduler, holding about `need` bytes,
    /// telling `progress`'s slot `slot` how far it has got.
    Make {
        spec: CopySpec,
        priority: Priority,
        need: u64,
        gen: u32,
        progress: Arc<Progress>,
        slot: u8,
    },
    /// Stop making the copy with this key.
    Cancel(CopyKey),
    /// Stop making every copy.
    CancelAll,
}

/// A copy's source being read.
enum Reading {
    /// Waiting for the worker's buffer.
    Sizing(Want),
    Reading {
        want: Want,
        audio: Vec<f32>,
        done: usize,
    },
}

/// The host's side of the copies: see the module notes.
pub(crate) struct CopySide {
    wanted: Vec<Want>,
    /// Where the next look builds what is wanted, made once.
    scratch: Vec<Want>,
    clips: Vec<ClipWant>,
    /// The clips launched and waiting for their copies (5b.4), at Urgent.
    launches: Vec<ClipWant>,
    map: Option<TempoMap>,
    /// Frames the map has held for.
    held_for: u64,
    reading: Option<Reading>,
    /// Copies with the scheduler, the memory each will take, and its
    /// slot in [`CopySide::progress`].
    making: Vec<(CopyKey, u64, u8)>,
    /// How far each copy with the scheduler has got.
    progress: Arc<Progress>,
    failed: Vec<CopyKey>,
    /// The one budget for derived audio: the process's, or the one of the
    /// scheduler a test gave the host.
    pub(crate) budget: Arc<Budget>,
    pub counts: CopyCounts,
    periods: u64,
    /// Look at the next steady period, not wait for the next
    /// [`WANT_EVERY`]: something was edited, or a copy came.
    relook: bool,
    /// What all derived audio used at the last look: once it is less,
    /// copies that found no room are asked for again.
    used_at_look: u64,
    /// The takes still copying in at the last period: one done may have
    /// copies to make, so it is looked at again (5d.3).
    loading: usize,
}

impl CopySide {
    pub(crate) fn new() -> CopySide {
        CopySide {
            wanted: Vec::with_capacity(MAX_WANTED),
            scratch: Vec::with_capacity(MAX_WANTED),
            clips: Vec::new(),
            launches: Vec::new(),
            map: None,
            held_for: 0,
            reading: None,
            making: Vec::with_capacity(MAX_MAKING),
            progress: Arc::new(Progress::new()),
            failed: Vec::with_capacity(MAX_FAILED),
            budget: Budget::shared(),
            counts: CopyCounts::default(),
            periods: 0,
            relook: false,
            used_at_look: 0,
            loading: 0,
        }
    }

    /// The clips on Stretch, from the app. Gives back the old list, for
    /// the host to drop off the audio thread.
    pub(crate) fn set_clips(&mut self, clips: Vec<ClipWant>) -> Vec<ClipWant> {
        std::mem::replace(&mut self.clips, clips)
    }

    /// The clips launched and waiting for their copies, from the app
    /// (5b.4); looked at again at the next steady period. Gives back the
    /// old list, for the host to drop off the audio thread.
    pub(crate) fn set_launches(&mut self, clips: Vec<ClipWant>) -> Vec<ClipWant> {
        self.relook = true;
        std::mem::replace(&mut self.launches, clips)
    }

    /// Forgets everything under way: the tracks were emptied. The buffer
    /// being read, if any, for the host to drop.
    pub(crate) fn clear(&mut self) -> Option<Vec<f32>> {
        self.wanted.clear();
        self.making.clear();
        self.failed.clear();
        self.map = None;
        self.held_for = 0;
        match self.reading.take() {
            Some(Reading::Reading { audio, .. }) => Some(audio),
            _ => None,
        }
    }

    /// Whether the copy with `key` is about to play: wanted now for a
    /// launch waiting for it, a region drifting on Keep pitch, or a song
    /// region playing or within 2 bars ([`Priority::Protect`] or more
    /// urgent). Clear copies keeps it (5c.4).
    pub(crate) fn soon(&self, key: &CopyKey) -> bool {
        self.wanted
            .iter()
            .any(|w| w.key == *key && w.priority <= Priority::Protect)
    }

    /// The recordings of the copies being read or made, one call each.
    pub(crate) fn making_recs(&self, f: &mut dyn FnMut(RecordingId)) {
        if let Some(Reading::Sizing(w) | Reading::Reading { want: w, .. }) = &self.reading {
            f(w.key.rec);
        }
        for (k, _, _) in &self.making {
            f(k.rec);
        }
    }

    /// Copies being read or made.
    pub(crate) fn busy(&self) -> usize {
        self.making.len() + usize::from(self.reading.is_some())
    }

    /// Where the copy with `key` stands: in, being made (how far), waiting
    /// its turn (wanted, or being read), couldn't be made, or not wanted
    /// (waiting too: it is wanted at the next look).
    pub(crate) fn state(&self, tracks: &AudioTracks, key: &CopyKey) -> CopyState {
        if tracks.has_copy(key) {
            return CopyState::Ready;
        }
        if let Some(m) = self.making.iter().find(|m| m.0 == *key) {
            return CopyState::Making(self.progress.get(m.2, Progress::tag(key)));
        }
        if self.failed.contains(key) {
            return CopyState::Unmade;
        }
        CopyState::Waiting
    }

    /// How far each copy with the scheduler has got (0 to 1), in the
    /// order they were asked for.
    pub(crate) fn progress(&self, each: &mut dyn FnMut(f32)) {
        for m in &self.making {
            each(self.progress.get(m.2, Progress::tag(&m.0)));
        }
    }

    /// What is wanted is worked out again at the next steady period: a
    /// command may have edited a region, or a copy came and is taken in
    /// only if it is still wanted.
    pub(crate) fn look_soon(&mut self) {
        self.relook = true;
    }

    /// Whether `key` is wanted now.
    fn wants(&self, key: &CopyKey) -> bool {
        self.wanted.iter().any(|w| w.key == *key)
    }

    /// One period of `frames` frames, the song at tick `now` along `map`:
    /// what is wanted, room, the source read, what to tell the worker
    /// (`ask`) and the buffers to drop (`drop`). Does not allocate.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tend(
        &mut self,
        tracks: &mut AudioTracks,
        map: &TempoMap,
        now: f64,
        frames: usize,
        gen: u32,
        ask: &mut dyn FnMut(Ask),
        drop: &mut dyn FnMut(Vec<f32>),
    ) {
        self.periods += 1;
        if self.map.as_ref() == Some(map) {
            self.held_for += frames as u64;
        } else if self.map.is_none() {
            // The map that came with the tracks (a project or a set's song
            // opening, or the first): nobody is turning it, so it counts
            // as held, and the copies are asked for at once (5d.3).
            self.map = Some(*map);
            self.held_for = u64::from(tracks.rate()) / 2;
            self.relook = true;
        } else {
            self.map = Some(*map);
            self.held_for = 0;
        }
        let loading = tracks.loading();
        if loading < self.loading {
            self.relook = true;
        }
        self.loading = loading;
        let steady = self.held_for >= u64::from(tracks.rate()) / 2;
        if steady && (self.relook || self.periods.is_multiple_of(WANT_EVERY)) {
            self.relook = false;
            self.look(tracks, map, now, ask, drop);
        }
        self.read(tracks, frames, gen, ask, drop);
        if self.reading.is_none() && self.making.len() < MAX_MAKING && steady {
            self.start(tracks, gen, ask);
        }
    }

    /// Works out what is wanted now; what is under way and no longer
    /// wanted stops.
    fn look(
        &mut self,
        tracks: &AudioTracks,
        map: &TempoMap,
        now: f64,
        ask: &mut dyn FnMut(Ask),
        drop: &mut dyn FnMut(Vec<f32>),
    ) {
        // Room came back (a cache let copies go, or the budget grew): what
        // couldn't be made may be now.
        let used = self.budget.used();
        if used < self.used_at_look {
            self.failed.clear();
        }
        self.used_at_look = used;
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        let bar = 4.0 * f64::from(PPQ);
        let mut too_many = 0;
        let mut push = |out: &mut Vec<Want>, w: Want| {
            if out.iter().any(|o| o.key == w.key) {
                return;
            }
            if out.len() < out.capacity() {
                out.push(w);
            } else {
                too_many += 1;
            }
        };
        let bpm = map.bpm_at(now.max(0.0) as Tick);
        let clip =
            |out: &mut Vec<Want>, c: &ClipWant, push: &mut dyn FnMut(&mut Vec<Want>, Want)| {
                let play = ClipPlay {
                    regions: &c.regions[..c.n.min(MAX_CLIP_REGIONS)],
                    origin: 0.0,
                    len: c.len,
                    last_frame: c.last_frame,
                    gain: 1.0,
                    clip: 0,
                    stamp: 0,
                };
                let at = if c.bpm > 0.0 { c.bpm } else { bpm };
                let mut want = |out: &mut Vec<Want>, bpm: f64, priority: Priority| {
                    tracks.clip_copy_keys(&play, bpm, &mut |_, key| {
                        let Some(take) = tracks.take_of(key.rec) else {
                            return;
                        };
                        let channels = tracks.take(take).map_or(1, |i| i.channels as u8);
                        let w = Want {
                            key,
                            take,
                            channels,
                            priority,
                            over: None,
                        };
                        push(out, w);
                    });
                };
                want(out, at, c.priority);
                // Each tempo the song's map plays steadily at from a tempo
                // point in the next eight bars (a ramp's end tempo where it
                // ends), so a clip playing switches to its copy there
                // (5d.2).
                let segs = map.segments();
                for (i, s) in segs.iter().enumerate().skip(1) {
                    let st = s.start as f64;
                    if st > now + 8.0 * bar {
                        break;
                    }
                    if st > now && !map.is_ramp(i) && s.bpm != at {
                        // Within two bars as needed next, as a song
                        // region's; never more urgent than the clip.
                        let soon = if st <= now + 2.0 * bar {
                            Priority::Protect
                        } else {
                            Priority::Prepared
                        };
                        want(out, s.bpm, c.priority.max(soon));
                    }
                }
            };
        // The clips launched and waiting for their copies (5b.4), then
        // the regions drifting on Keep pitch: they rejoin the bars once
        // their copy is here.
        for c in &self.launches {
            clip(&mut out, c, &mut push);
        }
        tracks.drift_wants(&mut |key| {
            let Some(take) = tracks.take_of(key.rec) else {
                return;
            };
            let channels = tracks.take(take).map_or(1, |i| i.channels as u8);
            let w = Want {
                key,
                take,
                channels,
                priority: Priority::Protect,
                over: None,
            };
            push(&mut out, w);
        });
        for t in 0..MAX_TRACKS {
            for r in tracks.regions(t) {
                if r.follow != FollowTempo::Stretch || (r.end as f64) < now {
                    continue;
                }
                let Some(key) = tracks.song_copy_key(r, map) else {
                    continue;
                };
                let (s, e) = (r.start as f64, r.end as f64);
                let over = (key.shape != 0).then_some(Over {
                    map: *map,
                    at: s,
                    len: e - s,
                });
                let Some(info) = tracks.take(r.take) else {
                    continue;
                };
                let priority = if s <= now + 2.0 * bar && e > now {
                    Priority::Protect
                } else if s <= now + 8.0 * bar {
                    Priority::Prepared
                } else {
                    Priority::Speculative
                };
                let w = Want {
                    key,
                    take: r.take,
                    channels: info.channels as u8,
                    priority,
                    over,
                };
                push(&mut out, w);
            }
        }
        for c in &self.clips {
            clip(&mut out, c, &mut push);
        }
        self.counts.too_many += too_many;
        // Most urgent first, in the order found within each (an insertion
        // sort: stable, and it doesn't allocate).
        for i in 1..out.len() {
            let mut j = i;
            while j > 0 && out[j - 1].priority > out[j].priority {
                out.swap(j - 1, j);
                j -= 1;
            }
        }
        if out != self.wanted {
            // What isn't wanted any more stops; what failed may be asked
            // again.
            for &(k, _, _) in &self.making {
                if !out.iter().any(|w| w.key == k) {
                    ask(Ask::Cancel(k));
                }
            }
            self.failed.clear();
            let stale = match &self.reading {
                Some(Reading::Sizing(w)) | Some(Reading::Reading { want: w, .. }) => {
                    !out.iter().any(|o| o.key == w.key)
                }
                None => false,
            };
            if stale {
                if let Some(Reading::Reading { audio, .. }) = self.reading.take() {
                    drop(audio);
                }
            }
            std::mem::swap(&mut self.wanted, &mut out);
        }
        self.scratch = out;
    }

    /// The next copy wanted that isn't held, under way or failed, if it
    /// has room: its buffer asked for.
    fn start(&mut self, tracks: &mut AudioTracks, gen: u32, ask: &mut dyn FnMut(Ask)) {
        let next = self.wanted.iter().copied().find(|w| {
            !tracks.has_copy(&w.key)
                && !self.making.iter().any(|m| m.0 == w.key)
                && !self.failed.contains(&w.key)
        });
        let Some(w) = next else {
            return;
        };
        let need = w.key.bytes(usize::from(w.channels));
        if !self.room(tracks, need, w.priority) {
            self.counts.no_room += 1;
            self.fail(w.key);
            return;
        }
        let n = CopySpec::frames_in(&w.key) * usize::from(w.channels);
        self.reading = Some(Reading::Sizing(w));
        ask(Ask::Buffer { n, key: w.key, gen });
    }

    /// What the copies use now: those held and those being made.
    fn uses(&self, tracks: &AudioTracks) -> u64 {
        let making: u64 = self.making.iter().map(|m| m.1).sum();
        tracks.copy_bytes().0 as u64 + making
    }

    /// Tells the budget what the copies hold and have under way now.
    pub(crate) fn report(&self, tracks: &AudioTracks) {
        let making: u64 = self.making.iter().map(|m| m.1).sum();
        self.budget
            .set_held(Cache::Tracks, tracks.copy_bytes().0 as u64);
        self.budget.set_making(Cache::Tracks, making);
    }

    /// Room for a copy of `need` bytes for `priority` in the budget,
    /// letting go of copies nothing wants (the one played longest ago
    /// first, never one playing) if it must. Whether there is.
    fn room(&mut self, tracks: &mut AudioTracks, need: u64, priority: Priority) -> bool {
        loop {
            let own = self.uses(tracks) + need;
            if self.budget.fits(Cache::Tracks, own, priority)
                && tracks.copy_bytes().1 < crate::copies::MAX_COPIES
            {
                return true;
            }
            let old = tracks
                .copies()
                .filter(|v| !v.playing && !self.wants(&v.key))
                .min_by_key(|v| v.heard)
                .map(|v| v.key);
            match old {
                Some(k) if tracks.drop_copy(&k) => self.counts.let_go += 1,
                _ => return false,
            }
        }
    }

    fn fail(&mut self, key: CopyKey) {
        if self.failed.len() < self.failed.capacity() {
            self.failed.push(key);
        }
    }

    /// The buffer for `key` came: its source is read from here on. False
    /// if it isn't the one being waited for (drop it).
    pub(crate) fn buffer(&mut self, key: CopyKey, audio: Vec<f32>) -> Result<(), Vec<f32>> {
        match self.reading.take() {
            Some(Reading::Sizing(w)) if w.key == key => {
                self.reading = Some(Reading::Reading {
                    want: w,
                    audio,
                    done: 0,
                });
                Ok(())
            }
            other => {
                self.reading = other;
                Err(audio)
            }
        }
    }

    /// Reads the source of the copy under way, [`READ_SPEED`] frames a
    /// frame played; once read, it goes to the worker to be made.
    fn read(
        &mut self,
        tracks: &AudioTracks,
        frames: usize,
        gen: u32,
        ask: &mut dyn FnMut(Ask),
        drop: &mut dyn FnMut(Vec<f32>),
    ) {
        // Only a source being read: one still waiting for its buffer stays
        // as it is. (Before, taking it dropped the wait, so `start` asked
        // for the same buffer again every period until the worker's
        // answer came.)
        let (want, mut audio, mut done) = match self.reading.take() {
            Some(Reading::Reading { want, audio, done }) => (want, audio, done),
            other => {
                self.reading = other;
                return;
            }
        };
        let key = want.key;
        let ch = usize::from(want.channels.max(1));
        let total = CopySpec::frames_in(&key);
        let from = CopySpec::frames_from(&key);
        let len = tracks.take(want.take).map_or(0, |i| i.frames);
        let mut budget = READ_SPEED * frames.max(1);
        while done < total && budget > 0 {
            let g = from + done as i64;
            if g < 0 || g as usize >= len {
                // Silence before and after the recording: the buffer is
                // zeroed.
                let skip = if g < 0 {
                    ((-g) as usize).min(total - done)
                } else {
                    total - done
                };
                done += skip;
                continue;
            }
            let n = (total - done).min(budget).min(len - g as usize);
            match tracks.read_take(
                want.take,
                g as usize,
                &mut audio[done * ch..(done + n) * ch],
            ) {
                Some(got) if got > 0 => {
                    done += got;
                    budget = budget.saturating_sub(got);
                }
                _ => {
                    // Gone, or on its file: not made in 5b.
                    self.counts.failed += 1;
                    self.fail(key);
                    drop(audio);
                    return;
                }
            }
        }
        if done < total {
            self.reading = Some(Reading::Reading { want, audio, done });
            return;
        }
        let need = CopySpec::need(&key, want.channels);
        // The first slot no copy under way has (there is one: at most
        // MAX_MAKING are, and this one isn't yet).
        let slot = (0..MAX_MAKING as u8)
            .find(|s| !self.making.iter().any(|m| m.2 == *s))
            .unwrap_or(0);
        self.progress.set(slot, Progress::tag(&key), 0.0);
        if self.making.len() < self.making.capacity() {
            self.making.push((key, key.bytes(ch), slot));
        }
        ask(Ask::Make {
            spec: CopySpec {
                key,
                channels: want.channels,
                audio,
                over: want.over,
            },
            priority: want.priority,
            need,
            gen,
            progress: Arc::clone(&self.progress),
            slot,
        });
    }

    /// A copy came back from the scheduler: whether to take it in (still
    /// wanted, and room for it).
    pub(crate) fn made(&mut self, key: &CopyKey, tracks: &mut AudioTracks, bytes: usize) -> bool {
        self.making.retain(|m| m.0 != *key);
        let Some(priority) = self
            .wanted
            .iter()
            .find(|w| w.key == *key)
            .map(|w| w.priority)
        else {
            self.counts.stale += 1;
            return false;
        };
        if !self.room(tracks, bytes as u64, priority) {
            self.counts.no_room += 1;
            self.fail(*key);
            return false;
        }
        true
    }

    /// The copy with `key` won't come: cancelled (`failed` false), or
    /// refused or failed, not asked for again until what is wanted
    /// changes.
    pub(crate) fn gone(&mut self, key: &CopyKey, failed: bool) {
        self.making.retain(|m| m.0 != *key);
        if failed {
            self.fail(*key);
        }
    }

    /// Clear copies: everything wanted may be made again.
    pub(crate) fn cleared(&mut self) {
        self.failed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{TakeFile, TrackFile};
    use crate::{Input, Monitor, Region};
    use mpfree_stretch::sched::{Config, Sched};
    use mpfree_transport::{Block, Transport};

    /// A copy waiting for the worker's buffer is asked for once, however
    /// many periods the worker takes to answer, and its source is then
    /// read and sent to be made once. Before, every period dropped the
    /// wait and asked again: a busy worker made a zeroed buffer for each
    /// period it was late, each came back to be dropped, and the audio
    /// thread's channel sends took blocks of the channels now and then
    /// (the host's no-allocation test failed under load).
    #[test]
    fn a_copy_waiting_for_its_buffer_is_asked_for_once_however_late_it_comes() {
        const RATE: u32 = 22_050;
        const P: usize = 256;
        let bar = 4 * i64::from(PPQ);
        // Two bars at 120 BPM.
        let frames = 4 * RATE as usize;
        let rec = RecordingId(7);
        let take = TakeFile {
            id: 0,
            rec,
            track: 0,
            group: 0,
            pass: 0,
            bpm: 120.0,
            varied: false,
            anchor: 0.0,
            frames,
            channels: 1,
            file: String::new(),
            path: None,
            audio: (0..frames)
                .map(|j| ((j * 31) % 97) as f32 / 97.0 - 0.5)
                .collect(),
        };
        let mut tracks = AudioTracks::new(RATE, 20);
        assert!(tracks.load_take(take).is_ok());
        let region = Region {
            follow: FollowTempo::Stretch,
            ..Region::of(0, 2 * bar, 0, (0, 0.0), 120.0)
        };
        let track = TrackFile {
            index: 0,
            id: 0,
            input: Input::Left,
            monitor: Monitor::Auto,
            from: String::new(),
            regions: vec![region],
        };
        assert!(tracks.load_track(&track));
        let mut side = CopySide::new();
        side.budget = Sched::manual(Config::default()).budget();
        // At 90 BPM the region wants a copy.
        let mut t = Transport::new(RATE, 90.0).unwrap();
        let mut b = Block::new();
        let input = vec![0.0f32; 2 * P];
        let mut out = vec![0.0f32; 2 * P];
        let mut period =
            |tracks: &mut AudioTracks, side: &mut CopySide, ask: &mut dyn FnMut(Ask)| {
                assert!(t.process(P as u32, &mut b));
                let map = *t.tempo_map();
                tracks.process(&b, &map, &input, &mut out);
                side.tend(tracks, &map, 0.0, P, 0, ask, &mut |_| {});
            };
        // About 4.6 s with the buffer never coming: asked for once.
        let mut buffers = Vec::new();
        for _ in 0..400 {
            period(&mut tracks, &mut side, &mut |a| {
                if let Ask::Buffer { n, key, .. } = a {
                    buffers.push((n, key));
                }
            });
        }
        assert_eq!(buffers.len(), 1, "{buffers:?}");
        let (n, key) = buffers[0];
        assert_eq!(side.busy(), 1);
        // It comes: the source is read and sent to be made, once.
        assert!(side.buffer(key, vec![0.0; n]).is_ok());
        let (mut makes, mut more) = (0, 0);
        for _ in 0..400 {
            period(&mut tracks, &mut side, &mut |a| match a {
                Ask::Make { spec, .. } => {
                    assert_eq!(spec.key, key);
                    makes += 1;
                }
                Ask::Buffer { .. } => more += 1,
                _ => {}
            });
        }
        assert_eq!((makes, more), (1, 0));
    }
}
