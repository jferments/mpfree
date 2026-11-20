//! What a parameter of an instrument does under the sequencer's locks, for
//! the instruments the sequencer locks: the shared synth and the tracks'
//! own instruments (`synth.rs`, `instruments.rs`). A kit pad's sound keeps
//! its locks in the sampler engine (`Event::PadLock`), and the mixer's in
//! the mod engine (`modmix.rs`).
//!
//! - **The value under the locks.** The first lock on a parameter keeps
//!   the value it had under it until the last lock on it ends, so a run of
//!   locked steps comes back to the sound's own value, not to an earlier
//!   step's.
//! - **One owner at a time** (knob owners, instrument redesign step 4b).
//!   Several tracks can play one instrument (step 4a), so two tracks'
//!   clips can lock one knob at once. Each track's lock is kept with its
//!   value, and one track owns the knob, by a fixed priority, never by
//!   timing: the lowest lasting track id ([`Under::set_ranks`], step 2c),
//!   which doesn't change as tracks move; a track without one comes last.
//!   The owner's locks play with their exact value and timing; the
//!   others' are kept, not played.
//! - **When the owner stops**, the next one takes over the way the knob
//!   allows: a short glide ([`GLIDE_S`]) on a smooth knob (cutoff, a
//!   level), at once on a stepped one (a waveform, on or off, a mode).
//!   When the last lock ends, the value under them comes back on the
//!   end's frame, as before.
//! - **The hand wins.** A turn by hand (the Synth tab, a learned control,
//!   UNDO of a turn) is the value under the locks from then on, and it
//!   holds the knob while it moves and for [`HAND_S`] after: the locks
//!   are kept, not played. Then the owner takes the knob back the same
//!   way, gliding or at once.
//! - **An ended lock** stays known until the period has been played
//!   ([`Under::settle`]): an end and a new lock on the same frame (a clip
//!   switch) still find the sound's own value, though the instrument has
//!   not played the end yet.
//!
//! Fixed tables: nothing allocates after [`Under::new`].

/// The most parameters held under locks (or by the hand) at once, per
/// table.
pub const MAX: usize = 64;

/// The most locks at once, per table: a parameter locked by two tracks
/// counts two.
pub const HOLDS: usize = 128;

// Each lock's value put back has a place in its instrument's queue: two
// unlocks a period for each lock held (`mpfree_node::room`).
const _: () = assert!(mpfree_track::ROOM.unlocks >= 2 * HOLDS);

/// The tracks a lock can come from (each sequencer track).
pub const TRACKS: usize = mpfree_seq::MAX_TRACKS;

/// How long the next owner's glide to its value takes, in seconds.
pub const GLIDE_S: f32 = 0.03;

/// How long the hand keeps a knob after its last move, in seconds.
pub const HAND_S: f32 = 0.5;

/// No owner.
const NONE: u8 = u8::MAX;

/// Whether a parameter glides to a new owner's value (a smooth knob) or
/// switches at once (a stepped one: steps, a choice, on or off).
pub fn glides(p: &mpfree_node::ParamInfo) -> bool {
    !matches!(
        p.curve,
        mpfree_node::Curve::Stepped | mpfree_node::Curve::Choice(_) | mpfree_node::Curve::Toggle
    )
}

const _: () = assert!(TRACKS < NONE as usize);

#[derive(Clone, Copy, Debug, Default)]
struct Glide {
    from: f32,
    to: f32,
    /// Frames still to go (0: none).
    left: u32,
    total: u32,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    key: u32,
    under: f32,
    /// A smooth knob glides to a new owner's value; a stepped one switches.
    smooth: bool,
    /// The track whose locks play ([`NONE`]: no lock holds it).
    owner: u8,
    /// The value it was last set to here (a lock, the hand, a glide).
    last: f32,
    /// Frames the hand still holds it.
    hand: u32,
    glide: Glide,
}

impl Default for Entry {
    fn default() -> Entry {
        Entry {
            key: 0,
            under: 0.0,
            smooth: true,
            owner: NONE,
            last: 0.0,
            hand: 0,
            glide: Glide::default(),
        }
    }
}

/// One track's lock on a parameter, with the value it locks it to.
#[derive(Clone, Copy, Debug, Default)]
struct Hold {
    key: u32,
    src: u8,
    value: f32,
}

/// Each locked parameter's own value and its locks, by a key the
/// instrument makes (an engine or a track, and the parameter's id).
#[derive(Clone, Debug)]
pub struct Under {
    e: [Entry; MAX],
    n: usize,
    h: [Hold; HOLDS],
    m: usize,
    /// Each track's lasting id (0: none yet), its place in line for a knob.
    rank: [u32; TRACKS],
    glide_frames: u32,
    hand_frames: u32,
    /// Locks that found a table full (they don't play).
    pub full: u64,
    /// Locks kept but not played: another track owned the knob, or the
    /// hand held it.
    pub waited: u64,
}

impl Under {
    /// Empty, for a sound at `rate` frames a second. Allocates nothing on
    /// the heap.
    pub fn new(rate: u32) -> Under {
        let frames = |s: f32| ((rate as f32 * s) as u32).max(1);
        Under {
            e: [Entry::default(); MAX],
            n: 0,
            h: [Hold::default(); HOLDS],
            m: 0,
            rank: [0; TRACKS],
            glide_frames: frames(GLIDE_S),
            hand_frames: frames(HAND_S),
            full: 0,
            waited: 0,
        }
    }

    /// Each track's lasting id (`mpfree_seq::TrackId`, 0 for none), from
    /// the time engine's tracks in order: the lowest owns a knob two
    /// tracks lock. A knob's owner follows at its next lock or end.
    pub fn set_ranks(&mut self, ids: &[u32; TRACKS]) {
        self.rank = *ids;
    }

    /// The status line's words for the locks that waited or were lost,
    /// when any were. Allocates (for the log, not the sound).
    pub fn describe(&self) -> Option<String> {
        (self.waited + self.full > 0).then(|| {
            format!(
                "knob locks: {} waited for another track or the hand, {} lost to a full table",
                self.waited, self.full
            )
        })
    }

    fn find(&self, key: u32) -> Option<usize> {
        self.e[..self.n].iter().position(|e| e.key == key)
    }

    fn hold(&self, key: u32, src: u8) -> Option<usize> {
        self.h[..self.m]
            .iter()
            .position(|h| h.key == key && h.src == src)
    }

    /// Track `src`'s place in line: its lasting id, one without last.
    fn place(&self, src: u8) -> (u32, u8) {
        let id = self.rank.get(usize::from(src)).copied().unwrap_or(0);
        (if id == 0 { u32::MAX } else { id }, src)
    }

    /// The track that owns `key` now, and its value: the first in line of
    /// the tracks that lock it.
    fn first(&self, key: u32) -> Option<(u8, f32)> {
        self.h[..self.m]
            .iter()
            .filter(|h| h.key == key)
            .min_by_key(|h| self.place(h.src))
            .map(|h| (h.src, h.value))
    }

    /// A lock from track `src` on `key`, to `value`; `now` is the
    /// parameter's value before it (kept as the value under the locks
    /// from the first lock on), and `smooth` whether it can glide. Some
    /// true: it plays now, on its frame (its track owns the knob). Some
    /// false: kept, not played (another track owns it, or the hand holds
    /// it). None when a table is full: it doesn't play, as nothing could
    /// put the value back (counted).
    pub fn lock(
        &mut self,
        key: u32,
        src: usize,
        value: f32,
        now: f32,
        smooth: bool,
    ) -> Option<bool> {
        let src = u8::try_from(src)
            .ok()
            .filter(|&s| usize::from(s) < TRACKS)?;
        let i = match self.find(key) {
            Some(i) => i,
            None if self.n == MAX => {
                self.full += 1;
                return None;
            }
            None => {
                self.e[self.n] = Entry {
                    key,
                    under: now,
                    smooth,
                    last: now,
                    ..Entry::default()
                };
                self.n += 1;
                self.n - 1
            }
        };
        match self.hold(key, src) {
            Some(j) => self.h[j].value = value,
            None if self.m == HOLDS => {
                self.full += 1;
                return None;
            }
            None => {
                self.h[self.m] = Hold { key, src, value };
                self.m += 1;
            }
        }
        let owner = self.first(key).map_or(src, |(o, _)| o);
        let e = &mut self.e[i];
        e.owner = owner;
        e.smooth = smooth;
        if e.hand > 0 || owner != src {
            self.waited += 1;
            return Some(false);
        }
        e.glide = Glide::default();
        e.last = value;
        Some(true)
    }

    /// Track `src`'s lock on `key` ends: the value to set on the end's
    /// frame, or None for nothing now. The last lock's end gives the value
    /// under them; the owner's end with another track still locking gives
    /// that track's value on a stepped knob, and starts a glide to it on a
    /// smooth one ([`Under::tick`]). Nothing while another track owns the
    /// knob, while the hand holds it, or when `src` held no lock there.
    pub fn unlock(&mut self, key: u32, src: usize) -> Option<f32> {
        let src = u8::try_from(src).ok()?;
        let j = self.hold(key, src)?;
        self.m -= 1;
        self.h[j] = self.h[self.m];
        let i = self.find(key)?;
        if self.e[i].owner != src {
            return None;
        }
        let next = self.first(key);
        let glide_frames = self.glide_frames;
        let e = &mut self.e[i];
        e.owner = next.map_or(NONE, |(o, _)| o);
        if e.hand > 0 {
            return None;
        }
        match next {
            None => {
                e.glide = Glide::default();
                e.last = e.under;
                Some(e.under)
            }
            Some((_, v)) if e.smooth => {
                e.glide = Glide {
                    from: e.last,
                    to: v,
                    left: glide_frames,
                    total: glide_frames,
                };
                None
            }
            Some((_, v)) => {
                e.glide = Glide::default();
                e.last = v;
                Some(v)
            }
        }
    }

    /// Whether track `src` holds a lock on `key` now: a lock from it there
    /// changes a value, and a new one adds a hold (whose end may put a
    /// value back).
    pub fn held_by(&self, key: u32, src: usize) -> bool {
        u8::try_from(src).is_ok_and(|s| self.hold(key, s).is_some())
    }

    /// Whether a lock holds `key` now.
    #[cfg(test)]
    pub fn holds(&self, key: u32) -> bool {
        self.h[..self.m].iter().any(|h| h.key == key)
    }

    /// The track whose lock plays on `key` now.
    #[cfg(test)]
    pub fn owner(&self, key: u32) -> Option<usize> {
        let e = self.e[self.find(key)?];
        (e.owner != NONE).then_some(usize::from(e.owner))
    }

    /// A turn by hand of `key` to `value`, which the caller sets: under a
    /// lock it is the value the locks' end comes back to, and it holds the
    /// knob for [`HAND_S`] (from this turn), the locks kept, not played.
    pub fn hand(&mut self, key: u32, value: f32) {
        let i = match self.find(key) {
            Some(i) => i,
            // A table full of locks: the turn plays, unheld.
            None if self.n == MAX => return,
            None => {
                self.e[self.n] = Entry {
                    key,
                    ..Entry::default()
                };
                self.n += 1;
                self.n - 1
            }
        };
        let e = &mut self.e[i];
        e.under = value;
        e.last = value;
        e.hand = self.hand_frames;
        e.glide = Glide::default();
    }

    /// Once a block, before the instrument plays its `frames` frames: a
    /// hand that let go gives the knob back to its owner (a glide, or at
    /// once on a stepped knob), and glides move on. `apply(key, value)`
    /// sets each that moves, from the block's first frame. Never
    /// allocates.
    pub fn tick(&mut self, frames: u32, mut apply: impl FnMut(u32, f32)) {
        for i in 0..self.n {
            let e = self.e[i];
            let mut e2 = e;
            if e.hand > 0 {
                e2.hand = e.hand.saturating_sub(frames);
                if e2.hand == 0 && e.owner != NONE {
                    if let Some(j) = self.hold(e.key, e.owner) {
                        let to = self.h[j].value;
                        if e.smooth {
                            e2.glide = Glide {
                                from: e.last,
                                to,
                                left: self.glide_frames,
                                total: self.glide_frames,
                            };
                        } else {
                            e2.last = to;
                            apply(e.key, to);
                        }
                    }
                }
            }
            if e2.glide.left > 0 {
                let g = &mut e2.glide;
                g.left = g.left.saturating_sub(frames);
                let done = 1.0 - g.left as f32 / g.total.max(1) as f32;
                let v = if g.left == 0 {
                    g.to
                } else {
                    g.from + (g.to - g.from) * done
                };
                e2.last = v;
                apply(e.key, v);
            }
            self.e[i] = e2;
        }
    }

    /// After the period played: a parameter no lock, hand or glide holds
    /// any more is forgotten.
    pub fn settle(&mut self) {
        let mut k = 0;
        for i in 0..self.n {
            let e = self.e[i];
            let held =
                e.hand > 0 || e.glide.left > 0 || self.h[..self.m].iter().any(|h| h.key == e.key);
            if held {
                self.e[k] = e;
                k += 1;
            }
        }
        self.n = k;
    }

    /// Forgets every entry `pick` names, and its locks, giving each one a
    /// lock still holds to `back` with its value under it (to put back
    /// now).
    pub fn forget(&mut self, pick: impl Fn(u32) -> bool, mut back: impl FnMut(u32, f32)) {
        let mut k = 0;
        for i in 0..self.n {
            let e = self.e[i];
            if pick(e.key) {
                if self.h[..self.m].iter().any(|h| h.key == e.key) {
                    back(e.key, e.under);
                }
            } else {
                self.e[k] = e;
                k += 1;
            }
        }
        self.n = k;
        let mut k = 0;
        for j in 0..self.m {
            if !pick(self.h[j].key) {
                self.h[k] = self.h[j];
                k += 1;
            }
        }
        self.m = k;
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

    /// How many parameters a lock holds now.
    #[cfg(test)]
    pub fn holding(&self) -> usize {
        self.e[..self.n]
            .iter()
            .filter(|e| self.h[..self.m].iter().any(|h| h.key == e.key))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1000;

    fn table() -> Under {
        Under::new(RATE)
    }

    #[test]
    fn a_run_of_locks_comes_back_to_the_value_under_the_first() {
        let mut u = table();
        assert_eq!(u.lock(7, 0, 0.6, 0.25, true), Some(true));
        assert_eq!(
            u.lock(7, 0, 0.7, 0.9, true),
            Some(true),
            "a second lock keeps the first's value"
        );
        assert!(u.holds(7));
        assert_eq!(u.unlock(7, 0), Some(0.25));
        assert_eq!(u.unlock(7, 0), None, "once");
        // An end and a lock in the same period: still the sound's own.
        assert_eq!(u.lock(7, 0, 0.6, 0.9, true), Some(true));
        assert_eq!(u.unlock(7, 0), Some(0.25));
        u.settle();
        assert_eq!(u.holding(), 0);
        assert_eq!(u.lock(7, 0, 0.6, 0.5, true), Some(true));
        assert_eq!(u.unlock(7, 0), Some(0.5), "new after the period");
    }

    #[test]
    fn a_turn_under_a_lock_is_what_its_end_comes_back_to() {
        let mut u = table();
        u.hand(3, 0.1);
        assert!(!u.holds(3), "no lock: nothing kept");
        u.tick(RATE, |_, _| {});
        u.settle();
        assert_eq!(u.lock(3, 0, 0.9, 0.2, true), Some(true));
        u.hand(3, 0.7);
        assert_eq!(u.unlock(3, 0), None, "the hand holds it");
        u.tick(RATE, |_, _| {});
        u.settle();
        assert_eq!(u.lock(3, 0, 0.9, 0.7, true), Some(true));
        assert_eq!(u.unlock(3, 0), Some(0.7));
    }

    #[test]
    fn a_full_table_refuses_and_forget_gives_back_what_holds() {
        let mut u = table();
        for k in 0..MAX as u32 {
            assert_eq!(u.lock(k, 0, 1.0, k as f32, true), Some(true));
        }
        assert_eq!(u.lock(999, 0, 1.0, 0.0, true), None);
        assert_eq!(u.full, 1);
        assert_eq!(u.unlock(1, 0), Some(1.0));
        let mut back = Vec::new();
        u.forget(|k| k < 3, |k, v| back.push((k, v)));
        assert_eq!(
            back,
            vec![(0, 0.0), (2, 2.0)],
            "the ended one isn't given back"
        );
        assert_eq!(u.holding(), MAX - 3);
        u.reread(|k| k == 5, |_| Some(0.5));
        assert_eq!(u.unlock(5, 0), Some(0.5));
    }

    /// Step 4b: two tracks lock one knob. The lowest lasting id owns it,
    /// whichever locked first; the other's locks are kept, not played.
    /// When the owner stops, the next takes over: at once on a stepped
    /// knob, by a glide on a smooth one. The last end gives the value
    /// under them back.
    #[test]
    fn two_tracks_on_one_knob_take_turns_by_their_lasting_ids() {
        let mut u = table();
        // Track 3 (index 2) has id 1, the oldest; track 1 (index 0) id 5.
        let mut ids = [0; TRACKS];
        (ids[0], ids[2]) = (5, 1);
        u.set_ranks(&ids);
        // A stepped knob: track 1 locks first and plays, then track 3,
        // first in line, takes it on its lock's frame.
        assert_eq!(u.lock(1, 0, 2.0, 0.0, false), Some(true));
        assert_eq!(u.lock(1, 2, 3.0, 2.0, false), Some(true));
        assert_eq!(u.owner(1), Some(2));
        assert_eq!(
            u.lock(1, 0, 4.0, 3.0, false),
            Some(false),
            "kept, not played"
        );
        assert_eq!(u.waited, 1);
        assert_eq!(
            u.describe().as_deref(),
            Some("knob locks: 1 waited for another track or the hand, 0 lost to a full table")
        );
        // Track 3 stops: track 1's last value, at once.
        assert_eq!(u.unlock(1, 2), Some(4.0));
        assert_eq!(u.owner(1), Some(0));
        assert_eq!(u.unlock(1, 0), Some(0.0), "the value under them");
        // A smooth knob: the next owner glides there over GLIDE_S.
        assert_eq!(u.lock(9, 2, 1.0, 0.5, true), Some(true));
        assert_eq!(u.lock(9, 0, 0.0, 1.0, true), Some(false));
        assert_eq!(u.unlock(9, 2), None, "a glide, not a jump");
        let mut moved = Vec::new();
        let block = (RATE as f32 * GLIDE_S / 3.0) as u32;
        for _ in 0..3 {
            u.tick(block, |k, v| moved.push((k, v)));
        }
        assert_eq!(moved.len(), 3);
        assert!(moved[0].1 < 1.0 && moved[0].1 > moved[1].1, "{moved:?}");
        assert_eq!(moved[2], (9, 0.0), "there at the end");
        u.tick(block, |_, _| panic!("done"));
        // The last end while a glide runs: the value under them, at once,
        // and the glide stops.
        assert_eq!(u.lock(8, 2, 1.0, 0.5, true), Some(true));
        assert_eq!(u.lock(8, 0, 0.0, 1.0, true), Some(false));
        assert_eq!(u.unlock(8, 2), None);
        assert_eq!(u.unlock(8, 0), Some(0.5));
        u.tick(block, |_, _| panic!("no glide left"));
        // A track without an id comes after every track with one.
        ids[2] = 0;
        u.set_ranks(&ids);
        assert_eq!(u.lock(4, 2, 1.0, 0.0, false), Some(true));
        assert_eq!(
            u.lock(4, 0, 2.0, 1.0, false),
            Some(true),
            "track 1 has an id"
        );
        assert_eq!(u.owner(4), Some(0));
    }

    /// The hand wins while it turns and for HAND_S after: a lock then is
    /// kept, not played; then the owner takes the knob back (at once on a
    /// stepped knob), and the turn is the value under the locks.
    #[test]
    fn the_hand_holds_a_knob_then_gives_it_back_to_its_owner() {
        let mut u = table();
        u.hand(5, 0.3);
        assert_eq!(
            u.lock(5, 1, 0.9, 0.3, false),
            Some(false),
            "the hand has it"
        );
        let mut moved = Vec::new();
        u.tick(RATE / 4, |k, v| moved.push((k, v)));
        assert!(moved.is_empty(), "still held");
        u.hand(5, 0.4);
        u.tick(RATE / 4, |k, v| moved.push((k, v)));
        assert!(moved.is_empty(), "a new turn holds it again");
        u.tick(RATE / 4, |k, v| moved.push((k, v)));
        assert_eq!(moved, [(5, 0.9)], "back to the lock, at once");
        assert_eq!(u.unlock(5, 1), Some(0.4), "the turn is under it");
        u.settle();
        assert_eq!(u.holding(), 0);
    }
}
