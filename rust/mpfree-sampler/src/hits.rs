//! A sound's hits, for a granular Stretch that keeps them sharp.
//!
//! A pad in the Stretch grain mode gives its grains the sound's hits
//! ([`mpfree_grain::GrainSource::next_hit`]): where its drums and notes
//! start, as `mpfree-stretch` finds them for a stretch that keeps its
//! attacks ([`mpfree_stretch::hits`], at [`SENSITIVITY`]). They are found
//! once a sound, the first time such a pad is built into the engine's view,
//! and shared by every clone of the sound, like its octave copies
//! ([`crate::bandlimit::OctaveCell`]): at once for a short sound, on the
//! sampler's job service ([`crate::jobs`], its sound lane) for a longer one. A voice takes them if they are found by the
//! time it starts, else plays without them for its life, so what it plays
//! never hangs on a thread's timing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use crate::jobs::{Job, Jobs, Key, Lane, Order};
use crate::model::Pcm;

/// How readily a change counts as a hit: the stretch's own default.
pub const SENSITIVITY: f32 = 0.6;

/// Sounds up to this many frames (1.5 s at 44.1 kHz) are searched at once
/// when a view is built; longer ones on the job service.
pub const SYNC_FRAMES: usize = 1 << 16;

/// Sounds longer than this (6.3 minutes at 44.1 kHz) are not searched.
pub const MAX_FRAMES: usize = 1 << 24;

/// Where a sound's hits are kept once found, shared by every clone of the
/// sound, and whether they have been asked for.
#[derive(Default)]
pub struct HitCell {
    found: OnceLock<Vec<u64>>,
    asked: AtomicBool,
}

impl std::fmt::Debug for HitCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HitCell")
            .field("found", &self.found.get().map(Vec::len))
            .field("asked", &self.asked.load(Ordering::Relaxed))
            .finish()
    }
}

impl HitCell {
    /// The hits (frames of the sound, in order), once found. Never blocks
    /// or allocates: safe on the audio thread.
    #[inline]
    pub fn get(&self) -> Option<&[u64]> {
        self.found.get().map(Vec::as_slice)
    }
}

fn find(data: &Pcm, channels: usize, rate: u32) -> Vec<u64> {
    mpfree_stretch::hits(&data.all_f32(), channels, rate, SENSITIVITY)
}

/// What searching a sound of `frames` frames and `channels` channels holds:
/// its samples as floats, and their mono mix for the hit finder.
pub fn need(frames: usize, channels: usize) -> u64 {
    let f = frames as u64;
    f.saturating_mul(4 * channels as u64)
        .saturating_add(f.saturating_mul(4))
        .saturating_add(1_000_000)
}

/// The service's key for a sound's hits: its cell, which the job holds.
fn key(cell: &Arc<HitCell>) -> Key {
    Key::new("hits", Arc::as_ptr(cell) as usize as u64)
}

/// Makes sure a sound's hits are found or on their way: at once for a
/// sound of up to [`SYNC_FRAMES`] frames, on the process's job service
/// ([`Jobs::shared`]) for a longer one, never for one longer than
/// [`MAX_FRAMES`]. Does nothing if they are found or asked for already. A
/// search the memory floor can't spare is refused (and counted by the
/// service), and a later call asks again. Off the audio thread only (it may
/// allocate, and briefly wait for the service's lock).
pub fn prepare(cell: &Arc<HitCell>, data: &Pcm, channels: u8, rate: u32) {
    prepare_on(&Jobs::shared(), cell, data, channels, rate);
}

/// [`prepare`] on `jobs`.
fn prepare_on(jobs: &Jobs, cell: &Arc<HitCell>, data: &Pcm, channels: u8, rate: u32) {
    if cell.found.get().is_some() {
        return;
    }
    let channels = usize::from(channels.max(1));
    let frames = data.len() / channels;
    if frames > MAX_FRAMES {
        return;
    }
    if frames <= SYNC_FRAMES {
        let _ = cell.found.set(find(data, channels, rate));
        return;
    }
    if cell.asked.swap(true, Ordering::AcqRel) {
        return;
    }
    let (found, refused) = (cell.clone(), cell.clone());
    let data = data.clone();
    jobs.submit(
        Job::new(Lane::Sound, key(cell), move |_| {
            let _ = found.found.set(find(&data, channels, rate));
        })
        .order(Order::First)
        .needs(need(frames, channels), move |_| {
            // Left without hits for now: the next view built asks again.
            refused.asked.store(false, Ordering::Release);
        }),
    );
}

/// Finds a sound's hits here and now (any length up to [`MAX_FRAMES`]), if
/// they aren't found already: for offline renders and tests, which mustn't
/// depend on a thread's timing.
pub fn prepare_now(cell: &HitCell, data: &Pcm, channels: u8, rate: u32) {
    let channels = usize::from(channels.max(1));
    if cell.found.get().is_none() && data.len() / channels <= MAX_FRAMES {
        let _ = cell.found.set(find(data, channels, rate));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noise bursts every quarter second, each from silence.
    fn bursts(rate: u32, secs: f32) -> (Vec<f32>, Vec<u64>) {
        let frames = (rate as f32 * secs) as usize;
        let every = rate as usize / 4;
        let mut seed = 0x1234_5678u32;
        let mut out = vec![0.0f32; frames];
        let mut at = Vec::new();
        for (i, x) in out.iter_mut().enumerate() {
            let k = i % every;
            if k == 0 {
                at.push(i as u64);
            }
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            *x = 0.5 * noise * (-(k as f32) / (0.03 * rate as f32)).exp();
        }
        (out, at)
    }

    #[test]
    fn finds_each_burst_near_its_start() {
        let (data, at) = bursts(44_100, 2.0);
        let cell = Arc::new(HitCell::default());
        prepare(&cell, &Pcm::F32(data.into()), 1, 44_100);
        // 2 s is more than SYNC_FRAMES: wait for the job service.
        let t = std::time::Instant::now();
        while cell.get().is_none() && t.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let hits = cell.get().expect("found");
        // The first burst starts the sound: nothing before it to jump from.
        for want in &at[1..] {
            assert!(
                hits.iter().any(|&h| h.abs_diff(*want) <= 64),
                "no hit near {want}: {hits:?}"
            );
        }
        assert!(hits.len() <= at.len(), "{hits:?}");
    }

    /// A service with 1 MB more than the floor free.
    fn short() -> Jobs {
        Jobs::new(crate::jobs::Config {
            free: || Some(257_000_000),
            room: |free, need| match free {
                Some(f) if f.saturating_sub(need) >= 256_000_000 => Ok(()),
                _ => Err("not enough memory".into()),
            },
            ..crate::jobs::Config::default()
        })
    }

    #[test]
    fn a_long_sounds_search_runs_on_the_job_service_and_a_refused_one_is_asked_again() {
        let (data, _) = bursts(44_100, 2.0);
        let data = Pcm::F32(data.into());
        // 2 s of mono needs about 1.7 MB: refused, and asked for again.
        assert!(need(88_200, 1) > 1_000_000);
        let jobs = short();
        let cell = Arc::new(HitCell::default());
        prepare_on(&jobs, &cell, &data, 1, 44_100);
        let t = std::time::Instant::now();
        while jobs.counts().refused == 0 {
            assert!(t.elapsed().as_secs() < 30, "never refused");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(cell.get().is_none());
        assert!(
            !cell.asked.load(Ordering::Acquire),
            "a later call asks again"
        );
        // With room, it is found on the service.
        let jobs = Jobs::new(crate::jobs::Config::default());
        prepare_on(&jobs, &cell, &data, 1, 44_100);
        assert!(jobs.has(key(&cell)) || cell.get().is_some());
        let t = std::time::Instant::now();
        while cell.get().is_none() {
            assert!(t.elapsed().as_secs() < 30, "never found");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(jobs.counts().done, 1);
        // Found: never asked for again.
        prepare_on(&jobs, &cell, &data, 1, 44_100);
        assert!(!jobs.busy());
        assert_eq!(jobs.counts().done, 1);
    }

    #[test]
    fn a_steady_tone_has_none_after_its_start() {
        let rate = 44_100;
        let data: Vec<f32> = (0..rate)
            .map(|i| 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
            .collect();
        let cell = HitCell::default();
        prepare_now(&cell, &Pcm::F32(data.into()), 1, rate);
        // Its start may count, as nothing comes before it.
        let hits = cell.get().expect("found");
        assert!(hits.iter().all(|&h| h < 2_048), "{hits:?}");
    }
}
