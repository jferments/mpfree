//! What a parameter goes back to when a step's lock on it ends, for the
//! instruments the sequencer locks: the synth on the pads and a track's own
//! instrument (`synth.rs`, `instruments.rs`). A kit pad's sound keeps its
//! locks in the sampler engine (`Event::PadLock`), and the mixer's in the
//! mod engine (`modmix.rs`).
//!
//! - **A lock** on a parameter keeps the value it had under it, from its
//!   first lock until the lock ends, so a run of locked steps comes back to
//!   the sound's own value, not to an earlier step's.
//! - **A turn by hand** (the Synth tab) while a lock holds becomes the
//!   value under it, so the lock's end comes back to the turn, as the
//!   mixer's mod engine does.
//! - **An ended lock** stays known until the period has been played
//!   ([`Under::settle`]): an end and a new lock on the same frame (a clip
//!   switch) still find the sound's own value, though the instrument has
//!   not played the end yet.
//!
//! A fixed table: nothing allocates after [`Under::new`].

/// The most parameters held under locks at once, per table.
pub const MAX: usize = 64;

#[derive(Clone, Copy, Debug, Default)]
struct Entry {
    key: u32,
    under: f32,
    /// The lock still holds; false once it ended this period.
    on: bool,
}

/// Each locked parameter's own value, by a key the owner makes (an engine
/// or a track, and the parameter's id).
#[derive(Clone, Debug)]
pub struct Under {
    e: [Entry; MAX],
    n: usize,
    /// Locks that found the table full (they don't play).
    pub full: u64,
}

impl Default for Under {
    fn default() -> Self {
        Under::new()
    }
}

impl Under {
    pub fn new() -> Under {
        Under {
            e: [Entry::default(); MAX],
            n: 0,
            full: 0,
        }
    }

    fn find(&self, key: u32) -> Option<usize> {
        self.e[..self.n].iter().position(|e| e.key == key)
    }

    /// A lock on `key`, whose value is `now` before it: the value under it
    /// is kept from its first lock. False when the table is full (the lock
    /// should not play, as nothing could put the value back).
    pub fn lock(&mut self, key: u32, now: f32) -> bool {
        if let Some(i) = self.find(key) {
            self.e[i].on = true;
            return true;
        }
        if self.n == MAX {
            self.full += 1;
            return false;
        }
        self.e[self.n] = Entry {
            key,
            under: now,
            on: true,
        };
        self.n += 1;
        true
    }

    /// The lock on `key` ends: the value to put back, or None when no lock
    /// holds it.
    pub fn unlock(&mut self, key: u32) -> Option<f32> {
        let e = &mut self.e[self.find(key)?];
        if !e.on {
            return None;
        }
        e.on = false;
        Some(e.under)
    }

    /// Whether a lock holds `key` now.
    #[cfg(test)]
    pub fn holds(&self, key: u32) -> bool {
        self.find(key).is_some_and(|i| self.e[i].on)
    }

    /// A hand turned `key` to `value`: under a lock, the value its end
    /// comes back to.
    pub fn set(&mut self, key: u32, value: f32) {
        if let Some(i) = self.find(key) {
            self.e[i].under = value;
        }
    }

    /// After the period played: ended locks are forgotten.
    pub fn settle(&mut self) {
        let mut k = 0;
        for i in 0..self.n {
            if self.e[i].on {
                self.e[k] = self.e[i];
                k += 1;
            }
        }
        self.n = k;
    }

    /// Forgets every entry `pick` names, giving each lock still holding to
    /// `back` with its value under it (to put back now).
    pub fn forget(&mut self, pick: impl Fn(u32) -> bool, mut back: impl FnMut(u32, f32)) {
        let mut k = 0;
        for i in 0..self.n {
            let e = self.e[i];
            if pick(e.key) {
                if e.on {
                    back(e.key, e.under);
                }
            } else {
                self.e[k] = e;
                k += 1;
            }
        }
        self.n = k;
    }

    /// Takes the values under the entries `pick` names from `now` again
    /// (a preset went in: its values are the sound's own now).
    pub fn reread(&mut self, pick: impl Fn(u32) -> bool, now: impl Fn(u32) -> Option<f32>) {
        for e in &mut self.e[..self.n] {
            if pick(e.key) {
                if let Some(v) = now(e.key) {
                    e.under = v;
                }
            }
        }
    }

    /// How many locks hold now.
    #[cfg(test)]
    pub fn holding(&self) -> usize {
        self.e[..self.n].iter().filter(|e| e.on).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_locks_comes_back_to_the_value_under_the_first() {
        let mut u = Under::new();
        assert!(u.lock(7, 0.25));
        assert!(u.lock(7, 0.9), "a second lock keeps the first's value");
        assert!(u.holds(7));
        assert_eq!(u.unlock(7), Some(0.25));
        assert_eq!(u.unlock(7), None, "once");
        // An end and a lock in the same period: still the sound's own.
        assert!(u.lock(7, 0.9));
        assert_eq!(u.unlock(7), Some(0.25));
        u.settle();
        assert_eq!(u.holding(), 0);
        assert!(u.lock(7, 0.5));
        assert_eq!(u.unlock(7), Some(0.5), "new after the period");
    }

    #[test]
    fn a_turn_under_a_lock_is_what_its_end_comes_back_to() {
        let mut u = Under::new();
        u.set(3, 0.1);
        assert!(!u.holds(3), "no lock: nothing kept");
        assert!(u.lock(3, 0.2));
        u.set(3, 0.7);
        assert_eq!(u.unlock(3), Some(0.7));
    }

    #[test]
    fn a_full_table_refuses_and_forget_gives_back_what_holds() {
        let mut u = Under::new();
        for k in 0..MAX as u32 {
            assert!(u.lock(k, k as f32));
        }
        assert!(!u.lock(999, 0.0));
        assert_eq!(u.full, 1);
        assert_eq!(u.unlock(1), Some(1.0));
        let mut back = Vec::new();
        u.forget(|k| k < 3, |k, v| back.push((k, v)));
        assert_eq!(
            back,
            vec![(0, 0.0), (2, 2.0)],
            "the ended one isn't given back"
        );
        assert_eq!(u.holding(), MAX - 3);
        u.reread(|k| k == 5, |_| Some(0.5));
        assert_eq!(u.unlock(5), Some(0.5));
    }
}
