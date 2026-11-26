//! The host making Stretch's copies (the audio tracks rethink, phase 5b).
//!
//! The track player plays a copy only if it has one ([`crate::copies`]);
//! this is how it gets them. On the audio thread, with the host:
//!
//! 1. **What is wanted.** Every few periods, and at the next after a
//!    command (it may have edited a region) or a copy coming back, once
//!    the tempo map has held for half a second (a tempo being turned asks
//!    for nothing): the keys of every song region on Stretch over one
//!    steady tempo and of the clips' regions on Stretch the app lists
//!    ([`ClipWant`], at the tempo at the playhead), most urgent first:
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
//! stays off on the MPC for now), and ramps and steps (5d).

use std::sync::Arc;

use mpfree_stretch::budget::{Budget, Cache};
use mpfree_stretch::sched::Priority;
use mpfree_transport::time::{TempoMap, Tick, PPQ};

use crate::clips::{ClipPlay, ClipRegion, MAX_CLIP_REGIONS};
use crate::copies::{CopyKey, CopySpec};
use crate::{AudioTracks, FollowTempo, MAX_TRACKS};

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
}

/// A copy wanted: its key, its take and channels, how urgent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Want {
    pub key: CopyKey,
    pub take: u16,
    pub channels: u8,
    pub priority: Priority,
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

/// What the host tells its worker about copies.
pub(crate) enum Ask {
    /// A zeroed buffer of this many samples, for `key`'s source.
    Buffer { n: usize, key: CopyKey, gen: u32 },
    /// Make this copy on the scheduler, holding about `need` bytes.
    Make {
        spec: CopySpec,
        priority: Priority,
        need: u64,
        gen: u32,
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
    map: Option<TempoMap>,
    /// Frames the map has held for.
    held_for: u64,
    reading: Option<Reading>,
    /// Copies with the scheduler, and the memory each will take.
    making: Vec<(CopyKey, u64)>,
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
}

impl CopySide {
    pub(crate) fn new() -> CopySide {
        CopySide {
            wanted: Vec::with_capacity(MAX_WANTED),
            scratch: Vec::with_capacity(MAX_WANTED),
            clips: Vec::new(),
            map: None,
            held_for: 0,
            reading: None,
            making: Vec::with_capacity(MAX_MAKING),
            failed: Vec::with_capacity(MAX_FAILED),
            budget: Budget::shared(),
            counts: CopyCounts::default(),
            periods: 0,
            relook: false,
            used_at_look: 0,
        }
    }

    /// The clips on Stretch, from the app. Gives back the old list, for
    /// the host to drop off the audio thread.
    pub(crate) fn set_clips(&mut self, clips: Vec<ClipWant>) -> Vec<ClipWant> {
        std::mem::replace(&mut self.clips, clips)
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

    /// Copies being read or made.
    pub(crate) fn busy(&self) -> usize {
        self.making.len() + usize::from(self.reading.is_some())
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
        } else {
            self.map = Some(*map);
            self.held_for = 0;
        }
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
        // The regions drifting on Keep pitch first: they rejoin the bars
        // once their copy is here.
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
                let Some(info) = tracks.take(r.take) else {
                    continue;
                };
                let (s, e) = (r.start as f64, r.end as f64);
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
                };
                push(&mut out, w);
            }
        }
        let bpm = map.bpm_at(now.max(0.0) as Tick);
        for c in &self.clips {
            let play = ClipPlay {
                regions: &c.regions[..c.n.min(MAX_CLIP_REGIONS)],
                origin: 0.0,
                len: c.len,
                last_frame: c.last_frame,
                gain: 1.0,
                clip: 0,
                stamp: 0,
            };
            tracks.clip_copy_keys(&play, bpm, &mut |_, key| {
                let Some(take) = tracks.take_of(key.rec) else {
                    return;
                };
                let channels = tracks.take(take).map_or(1, |i| i.channels as u8);
                let w = Want {
                    key,
                    take,
                    channels,
                    priority: c.priority,
                };
                push(&mut out, w);
            });
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
            for &(k, _) in &self.making {
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
        let Some(Reading::Reading {
            want,
            mut audio,
            mut done,
        }) = self.reading.take()
        else {
            return;
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
        if self.making.len() < self.making.capacity() {
            self.making.push((key, key.bytes(ch)));
        }
        ask(Ask::Make {
            spec: CopySpec {
                key,
                channels: want.channels,
                audio,
            },
            priority: want.priority,
            need,
            gen,
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
