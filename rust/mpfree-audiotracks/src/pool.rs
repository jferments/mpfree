//! The memory every take shares: equal chunks of [`CHUNK`] samples, asked
//! for once. A take's audio is a run of chunks (chunk `c` holds its samples
//! `c * CHUNK..`), each in any free slot, found through a small table that
//! never grows. Slots are:
//!
//! - **Dirty:** recorded (or loaded) here and not yet on the disk; never
//!   given to anything else.
//! - **Clean:** the same as the disk has; can go to another chunk when a
//!   slot is needed, the one wanted longest ago first.
//! - **Loading:** waiting for the disk thread to fill it; plays nothing yet.
//!
//! With no disk (no streaming) every slot stays dirty, so the memory is the
//! whole limit, as a tape that ends.

use crate::PEAK_BLOCK;

/// Samples in one chunk (128 KiB of 32-bit floats): the unit the memory is
/// shared out in, and the unit read from and written to the disk.
pub const CHUNK: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Free,
    Dirty,
    Clean,
    Loading,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Slot {
    pub take: u16,
    pub chunk: u32,
    pub state: State,
    /// The planning round that last wanted it.
    pub last: u64,
}

const EMPTY: u64 = u64::MAX;

fn key(take: u16, chunk: u32) -> u64 {
    (u64::from(take) << 32) | u64::from(chunk)
}

fn hash(k: u64) -> usize {
    (k.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 29) as usize
}

pub(crate) struct Pool {
    pub data: Vec<f32>,
    pub slots: Vec<Slot>,
    /// (take, chunk) to slot: open addressing, at most half full.
    keys: Vec<u64>,
    vals: Vec<u32>,
    mask: usize,
    free: Vec<u32>,
    /// Each block's lowest and highest sample, kept as audio is written.
    pub peaks: Vec<(f32, f32)>,
    /// The planning round now.
    pub clock: u64,
}

impl Pool {
    pub fn new(samples: usize) -> Pool {
        let n = samples / CHUNK;
        let size = (2 * n).next_power_of_two().max(16);
        let mut data = vec![0.0; n * CHUNK];
        // Touch every page now, so the audio thread never waits for the
        // system to map one in the middle of a recording (a zeroed
        // allocation is mapped lazily).
        for i in (0..data.len()).step_by(1024) {
            data[i] = std::hint::black_box(0.0);
        }
        let mut free = Vec::with_capacity(n);
        free.extend((0..n as u32).rev());
        Pool {
            data,
            slots: vec![
                Slot {
                    take: 0,
                    chunk: 0,
                    state: State::Free,
                    last: 0,
                };
                n
            ],
            keys: vec![EMPTY; size],
            vals: vec![0; size],
            mask: size - 1,
            free,
            peaks: vec![(0.0, 0.0); n * CHUNK / PEAK_BLOCK],
            clock: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn used(&self) -> usize {
        (self.slots.len() - self.free.len()) * CHUNK
    }

    pub fn capacity(&self) -> usize {
        self.slots.len() * CHUNK
    }

    pub fn find(&self, take: u16, chunk: u32) -> Option<u32> {
        let k = key(take, chunk);
        let mut i = hash(k) & self.mask;
        loop {
            let s = self.keys[i];
            if s == k {
                return Some(self.vals[i]);
            }
            if s == EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
        }
    }

    /// The slot holding chunk `chunk` of take `take`, if it can be read.
    pub fn readable(&self, take: u16, chunk: u32) -> Option<usize> {
        self.find(take, chunk)
            .map(|s| s as usize)
            .filter(|&s| matches!(self.slots[s].state, State::Dirty | State::Clean))
    }

    fn insert(&mut self, k: u64, v: u32) {
        let mut i = hash(k) & self.mask;
        while self.keys[i] != EMPTY {
            i = (i + 1) & self.mask;
        }
        self.keys[i] = k;
        self.vals[i] = v;
    }

    fn remove(&mut self, k: u64) {
        let mut i = hash(k) & self.mask;
        loop {
            if self.keys[i] == k {
                break;
            }
            if self.keys[i] == EMPTY {
                return;
            }
            i = (i + 1) & self.mask;
        }
        // Shift the run after it back, so lookups never meet a hole.
        let mut j = i;
        loop {
            j = (j + 1) & self.mask;
            if self.keys[j] == EMPTY {
                break;
            }
            let h = hash(self.keys[j]) & self.mask;
            let stays = if i <= j {
                i < h && h <= j
            } else {
                i < h || h <= j
            };
            if !stays {
                self.keys[i] = self.keys[j];
                self.vals[i] = self.vals[j];
                i = j;
            }
        }
        self.keys[i] = EMPTY;
    }

    /// A slot for chunk `chunk` of take `take`, in `state`: a free one, or
    /// else the clean one wanted longest ago. With `any` false, only a slot
    /// not wanted in this planning round can go. None if there is none.
    pub fn alloc(&mut self, take: u16, chunk: u32, state: State, any: bool) -> Option<usize> {
        let s = match self.free.pop() {
            Some(s) => s,
            None => {
                let mut best: Option<(u32, u64)> = None;
                for (i, sl) in self.slots.iter().enumerate() {
                    if sl.state == State::Clean
                        && (any || sl.last < self.clock)
                        && best.is_none_or(|b| sl.last < b.1)
                    {
                        best = Some((i as u32, sl.last));
                    }
                }
                let (s, _) = best?;
                let old = self.slots[s as usize];
                self.remove(key(old.take, old.chunk));
                s
            }
        };
        self.slots[s as usize] = Slot {
            take,
            chunk,
            state,
            last: self.clock,
        };
        self.insert(key(take, chunk), s);
        Some(s as usize)
    }

    pub fn release(&mut self, s: usize) {
        let sl = self.slots[s];
        if sl.state == State::Free {
            return;
        }
        self.remove(key(sl.take, sl.chunk));
        self.slots[s].state = State::Free;
        if self.free.len() < self.free.capacity() {
            self.free.push(s as u32);
        }
    }

    /// Give back every slot of take `take`.
    pub fn release_take(&mut self, take: u16) {
        for s in 0..self.slots.len() {
            if self.slots[s].state != State::Free && self.slots[s].take == take {
                self.release(s);
            }
        }
    }

    /// Samples `start..start + len` were just written: their blocks' peaks
    /// take them in. A chunk is written in order from its start, so a
    /// write that begins on a block's edge is that block's first and
    /// replaces what was there before.
    pub fn note(&mut self, start: usize, len: usize) {
        let mut p = start;
        let end = start + len;
        while p < end {
            let b = p / PEAK_BLOCK;
            let to = end.min((b + 1) * PEAK_BLOCK);
            let (lo, hi) = crate::min_max(&self.data[p..to]);
            let pk = &mut self.peaks[b];
            *pk = if p.is_multiple_of(PEAK_BLOCK) {
                (lo, hi)
            } else {
                (pk.0.min(lo), pk.1.max(hi))
            };
            p = to;
        }
    }

    /// The lowest and highest sample of `start..start + len`, from the
    /// table for whole blocks.
    pub fn range_peaks(&self, start: usize, len: usize) -> (f32, f32) {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut p = start;
        let end = start + len;
        while p < end {
            let b = p / PEAK_BLOCK;
            let to = end.min((b + 1) * PEAK_BLOCK);
            let (l, h) = if p.is_multiple_of(PEAK_BLOCK) && to == (b + 1) * PEAK_BLOCK {
                self.peaks[b]
            } else {
                crate::min_max(&self.data[p..to])
            };
            lo = lo.min(l);
            hi = hi.max(h);
            p = to;
        }
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_finds_what_was_put_and_forgets_what_was_given_back() {
        let mut p = Pool::new(40 * CHUNK);
        let mut held = Vec::new();
        for i in 0..40u32 {
            let s = p.alloc((i % 7) as u16, i, State::Dirty, true).unwrap();
            held.push(((i % 7) as u16, i, s));
        }
        assert!(
            p.alloc(9, 0, State::Dirty, true).is_none(),
            "every slot dirty"
        );
        for &(t, c, s) in &held {
            assert_eq!(p.find(t, c), Some(s as u32));
        }
        // Give back every third, out of order; the rest stay findable.
        for &(t, c, s) in held.iter().step_by(3).rev() {
            assert_eq!(p.find(t, c), Some(s as u32));
            p.release(s);
            assert_eq!(p.find(t, c), None);
        }
        for (i, &(t, c, s)) in held.iter().enumerate() {
            assert_eq!(p.find(t, c), (i % 3 != 0).then_some(s as u32));
        }
        p.release_take(2);
        assert!(held
            .iter()
            .filter(|h| h.0 == 2)
            .all(|h| p.find(h.0, h.1).is_none()));
    }

    #[test]
    fn a_clean_slot_wanted_longest_ago_goes_first_and_wanted_ones_stay() {
        let mut p = Pool::new(3 * CHUNK);
        for c in 0..3 {
            let s = p.alloc(1, c, State::Clean, true).unwrap();
            p.slots[s].last = 10 + u64::from(c);
        }
        p.clock = 12;
        // Chunk 2 was wanted this round: only 0 and 1 may go, 0 first.
        let s = p.alloc(2, 0, State::Loading, false).unwrap();
        assert_eq!(p.find(1, 0), None);
        assert_eq!(p.find(2, 0), Some(s as u32));
        let s1 = p.find(1, 1).unwrap() as usize;
        p.slots[s1].last = 12;
        assert!(p.alloc(2, 1, State::Loading, false).is_none());
        assert!(
            p.alloc(2, 1, State::Dirty, true).is_some(),
            "a recording may take a wanted one"
        );
    }
}
