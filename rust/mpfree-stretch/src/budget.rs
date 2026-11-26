//! One memory budget for the audio MPFree derives (the audio tracks
//! rethink, phase 5b.3).
//!
//! Stretched and reversed copies are made from recordings MPFree already
//! has, so they can always be made again: the audio tracks' tempo copies,
//! the sampler's Warps, and in time the looper's tempo copies and phase
//! 6's reverse copies. Each cache keeps its own code and copies, and lets
//! go of its own, but all of them together stay within one number
//! ([`Budget::total`], [`DEFAULT_TOTAL`] to start with, the sampler's
//! Warps budget before: 200 MiB in all, not each).
//!
//! - **What a cache uses** is what it holds ([`Budget::set_held`]) plus
//!   what it has asked for and not yet had back, the jobs under way whose
//!   copies it will hold ([`Budget::set_making`]): it says each whenever
//!   it changes. (The two can be said from different threads: the
//!   sampler's Warps hold their copies on the control thread and their
//!   worker knows what is under way.)
//! - **Room goes by priority** ([`Budget::limit`]): what is needed now
//!   ([`Priority::Urgent`], [`Priority::Protect`]) may fill the whole
//!   budget, what is set up to come ([`Priority::Prepared`]) three
//!   quarters of it, and the rest ([`Priority::Speculative`]) half, so
//!   copies made just in case never leave a launch without room.
//! - **Two checks.** A cache asks before it asks for a copy
//!   ([`Budget::fits`]), letting go of its own copies nothing wants if it
//!   must; and the scheduler checks again as each job starts
//!   ([`Budget::check`]), so two caches asking at once can't pass it
//!   between them. A job that doesn't fit then is refused and told why.
//! - **The memory check is apart.** The budget is a cap on derived audio;
//!   whether the memory is there at all is the program's own check, the
//!   promise each job takes as it starts ([`crate::sched::Config::promise`]),
//!   in the same account as every other load's.
//!
//! Every call is a handful of atomic loads and stores: the tracks' host
//! asks from the audio thread, which never locks or allocates.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use crate::sched::Priority;

/// The budget to start with: 200 MiB for all derived audio together.
pub const DEFAULT_TOTAL: u64 = 200 << 20;

/// How a refusal for want of room in the budget starts
/// ([`Budget::check`]), so a client can tell it from the memory check's
/// and ask again once room comes back.
pub const NO_ROOM: &str = "no room for more copies";

/// A cache of derived audio, each with its share of the one budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cache {
    /// The audio tracks' tempo copies (regions on Stretch).
    Tracks,
    /// The sampler's Warps (regions warped to stretch).
    Warps,
    /// The looper's tempo copies (phase 5d).
    Looper,
    /// Reverse copies (phase 6).
    Reverse,
}

impl Cache {
    pub const ALL: [Cache; 4] = [Cache::Tracks, Cache::Warps, Cache::Looper, Cache::Reverse];

    /// What the memory gauge calls it.
    pub fn name(self) -> &'static str {
        match self {
            Cache::Tracks => "Tempo copies",
            Cache::Warps => "Warps",
            Cache::Looper => "Loop copies",
            Cache::Reverse => "Reverse copies",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The one budget for derived audio.
#[derive(Debug)]
pub struct Budget {
    total: AtomicU64,
    held: [AtomicU64; 4],
    making: [AtomicU64; 4],
}

impl Default for Budget {
    fn default() -> Budget {
        Budget::new(DEFAULT_TOTAL)
    }
}

impl Budget {
    /// A budget of `total` bytes, nothing used. The process has one
    /// ([`Budget::shared`]); a test makes its own.
    pub fn new(total: u64) -> Budget {
        Budget {
            total: AtomicU64::new(total),
            held: Default::default(),
            making: Default::default(),
        }
    }

    /// The process's budget, the one [`crate::sched::Sched::shared`]
    /// checks.
    pub fn shared() -> Arc<Budget> {
        static SHARED: OnceLock<Arc<Budget>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(Arc::default))
    }

    /// The bytes all derived audio may use together.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    pub fn set_total(&self, bytes: u64) {
        self.total.store(bytes, Ordering::Relaxed);
    }

    /// The copies `cache` holds now, in bytes.
    pub fn set_held(&self, cache: Cache, bytes: u64) {
        self.held[cache.index()].store(bytes, Ordering::Relaxed);
    }

    /// The copies `cache` has asked for and not yet had back, in bytes.
    pub fn set_making(&self, cache: Cache, bytes: u64) {
        self.making[cache.index()].store(bytes, Ordering::Relaxed);
    }

    /// What `cache` holds and has under way: its share now.
    pub fn uses(&self, cache: Cache) -> u64 {
        let i = cache.index();
        self.held[i]
            .load(Ordering::Relaxed)
            .saturating_add(self.making[i].load(Ordering::Relaxed))
    }

    /// What `cache` holds, and has under way, apart.
    pub fn share(&self, cache: Cache) -> (u64, u64) {
        let i = cache.index();
        (
            self.held[i].load(Ordering::Relaxed),
            self.making[i].load(Ordering::Relaxed),
        )
    }

    /// What every cache uses together.
    pub fn used(&self) -> u64 {
        Cache::ALL
            .iter()
            .map(|&c| self.uses(c))
            .fold(0, u64::saturating_add)
    }

    /// The most all derived audio may use together for a copy of
    /// `priority`: the whole budget for what is needed now, three quarters
    /// for what is set up to come, half for the rest.
    pub fn limit(&self, priority: Priority) -> u64 {
        let total = self.total();
        match priority {
            Priority::Urgent | Priority::Protect => total,
            Priority::Prepared => total / 4 * 3,
            Priority::Speculative => total / 2,
        }
    }

    /// Whether `cache`, using `own` bytes in all (the copy it is about to
    /// ask for included), stays within the limit for `priority` beside
    /// what the other caches use.
    pub fn fits(&self, cache: Cache, own: u64, priority: Priority) -> bool {
        let others = self.used().saturating_sub(self.uses(cache));
        others.saturating_add(own) <= self.limit(priority)
    }

    /// The scheduler's check as a job of `priority` starts, its cache
    /// having counted it already: whether all derived audio is within the
    /// limit for its priority. Err says why, in a musician's words.
    pub fn check(&self, priority: Priority) -> Result<(), String> {
        let (used, limit) = (self.used(), self.limit(priority));
        if used <= limit {
            return Ok(());
        }
        Err(format!(
            "{NO_ROOM}: {} MB of the {} MB they may use is used",
            mb(used),
            mb(limit)
        ))
    }
}

/// Bytes as whole megabytes (1 000 000 bytes, as the app says them),
/// rounded up.
fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_goes_by_priority_and_counts_every_cache_together() {
        let b = Budget::new(1000);
        assert_eq!(b.limit(Priority::Urgent), 1000);
        assert_eq!(b.limit(Priority::Protect), 1000);
        assert_eq!(b.limit(Priority::Prepared), 750);
        assert_eq!(b.limit(Priority::Speculative), 500);
        b.set_held(Cache::Warps, 300);
        b.set_making(Cache::Warps, 100);
        // The tracks may take 100 more just in case, 350 for what is set
        // up to come, 600 for what is needed now.
        assert!(b.fits(Cache::Tracks, 100, Priority::Speculative));
        assert!(!b.fits(Cache::Tracks, 101, Priority::Speculative));
        assert!(b.fits(Cache::Tracks, 350, Priority::Prepared));
        assert!(b.fits(Cache::Tracks, 600, Priority::Protect));
        assert!(!b.fits(Cache::Tracks, 601, Priority::Urgent));
        // What a cache uses is what it says now, not added up.
        b.set_held(Cache::Tracks, 300);
        b.set_held(Cache::Tracks, 150);
        b.set_making(Cache::Tracks, 50);
        assert_eq!(b.used(), 600);
        assert_eq!(b.share(Cache::Warps), (300, 100));
        // Its own use doesn't count twice when it asks again.
        assert!(b.fits(Cache::Tracks, 600, Priority::Protect));
        assert!(b.check(Priority::Protect).is_ok());
        assert!(b.check(Priority::Prepared).is_ok());
        let why = b.check(Priority::Speculative).unwrap_err();
        assert!(why.starts_with("no room for more copies"), "{why}");
        b.set_total(2000);
        assert!(b.check(Priority::Speculative).is_ok());
        assert_eq!(Cache::ALL.map(|c| b.uses(c)), [200, 400, 0, 0]);
    }
}
