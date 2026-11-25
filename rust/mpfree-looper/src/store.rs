//! The loops' shared memory: blocks of audio, each layer slot's table
//! of them, and reading a layer's frames through its view.

use super::*;

/// The shared memory: blocks of BLOCK stereo frames, and for every layer
/// slot of every loop a table of its blocks. A slot's blocks are always
/// the first entries of its table.
pub(crate) struct Store {
    pub(crate) pool: Vec<f32>,
    /// MAX_LOOPS * MAX_LAYERS tables of `n_blocks` entries.
    pub(crate) tables: Vec<u16>,
    /// Blocks in each slot's table.
    pub(crate) held: [[u16; MAX_LAYERS]; MAX_LOOPS],
    /// Free blocks, a stack.
    pub(crate) free: Vec<u16>,
    n_free: usize,
    pub(crate) n_blocks: usize,
    /// Each loop as it was when cleared, for Undo ([`Command::Clear`]):
    /// the slots whose blocks it keeps (none: nothing kept), and when it
    /// was kept (the oldest goes first when the memory is needed).
    pub(crate) kept: Vec<Loop>,
    pub(crate) kept_mask: [u16; MAX_LOOPS],
    kept_at: [u64; MAX_LOOPS],
    kept_seq: u64,
    /// Each kept loop's Clear as a step of its own, by its transaction
    /// (audio tracks phase 4d: [`Looper::loop_steps`]).
    pub(crate) kept_txn: [TransactionId; MAX_LOOPS],
    /// The last layer number given ([`Store::born`]).
    born: u64,
    /// Each slot's layer the app says is written whole to its file and
    /// synced, by its number (0: none; audio tracks phase 4d.4:
    /// [`Looper::set_synced`]).
    pub(crate) synced: [[u64; MAX_LAYERS]; MAX_LOOPS],
    /// Slots sent away: their blocks went back while only history holds
    /// them, their audio in their file, and Undo or Redo that would bring
    /// them back waits until it is brought back ([`Looper::away_for`]).
    pub(crate) away: [u16; MAX_LOOPS],
    /// The blocks each slot held when it was sent away.
    pub(crate) away_blocks: [[u16; MAX_LAYERS]; MAX_LOOPS],
    /// A slot being brought back from its file ([`Looper::begin_back`]).
    pub(crate) back: Option<Back>,
}

/// A slot being brought back ([`Looper::begin_back`]): its loop, its
/// slot, the frames in so far and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Back {
    pub(crate) lp: usize,
    pub(crate) slot: usize,
    pub(crate) done: usize,
    pub(crate) size: usize,
}

impl Store {
    pub(crate) fn new(n_blocks: usize) -> Store {
        Store {
            pool: vec![0.0; 2 * BLOCK * n_blocks],
            tables: vec![NONE; MAX_LOOPS * MAX_LAYERS * n_blocks],
            held: [[0; MAX_LAYERS]; MAX_LOOPS],
            free: (0..n_blocks as u16).rev().collect(),
            n_free: n_blocks,
            n_blocks,
            kept: vec![Loop::NEW; MAX_LOOPS],
            kept_mask: [0; MAX_LOOPS],
            kept_at: [0; MAX_LOOPS],
            kept_seq: 0,
            kept_txn: [TransactionId::NONE; MAX_LOOPS],
            born: 0,
            synced: [[0; MAX_LAYERS]; MAX_LOOPS],
            away: [0; MAX_LOOPS],
            away_blocks: [[0; MAX_LAYERS]; MAX_LOOPS],
            back: None,
        }
    }

    /// Slot `k` of loop `lp` is sent away ([`Store::away`]).
    pub(crate) fn is_away(&self, lp: usize, k: usize) -> bool {
        self.away[lp] & (1 << k) != 0
    }

    /// A new layer's number ([`LayerInfo::born`]): never given twice in
    /// this looper, and kept here, apart from the loops, so that a loop
    /// Undo brings back can't give one again.
    pub(crate) fn born(&mut self) -> u64 {
        self.born += 1;
        self.born
    }

    #[inline]
    fn table(&self, lp: usize, slot: usize) -> usize {
        (lp * MAX_LAYERS + slot) * self.n_blocks
    }

    /// Where frame `q` of a slot is in the pool, if its block is there.
    #[inline]
    fn at(&self, lp: usize, slot: usize, q: usize) -> Option<usize> {
        let b = q / BLOCK;
        if b >= self.held[lp][slot] as usize {
            return None;
        }
        Some(2 * (self.tables[self.table(lp, slot) + b] as usize * BLOCK + q % BLOCK))
    }

    pub(crate) fn sample(&self, lp: usize, slot: usize, q: usize, c: usize) -> Option<&f32> {
        self.at(lp, slot, q).map(|i| &self.pool[i + c])
    }

    /// The blocks a slot holds, in order.
    #[inline]
    pub(crate) fn blocks_of(&self, lp: usize, slot: usize) -> &[u16] {
        let t = self.table(lp, slot);
        &self.tables[t..t + self.held[lp][slot] as usize]
    }

    /// Frame `o` of block `b` of a slot whose blocks are `blocks`
    /// (silence if it does not hold that block).
    #[inline]
    pub(crate) fn frame(&self, blocks: &[u16], b: usize, o: usize) -> [f32; 2] {
        match blocks.get(b) {
            Some(&n) => {
                let i = 2 * (n as usize * BLOCK + o);
                let p = &self.pool[i..i + 2];
                [p[0], p[1]]
            }
            None => [0.0; 2],
        }
    }

    #[inline]
    pub(crate) fn get(&self, lp: usize, slot: usize, q: usize) -> [f32; 2] {
        self.at(lp, slot, q)
            .map_or([0.0; 2], |i| [self.pool[i], self.pool[i + 1]])
    }

    #[inline]
    pub(crate) fn set(&mut self, lp: usize, slot: usize, q: usize, x: [f32; 2]) {
        if let Some(i) = self.at(lp, slot, q) {
            self.pool[i] = x[0];
            self.pool[i + 1] = x[1];
        }
    }

    #[inline]
    pub(crate) fn add(&mut self, lp: usize, slot: usize, q: usize, x: [f32; 2]) {
        if let Some(i) = self.at(lp, slot, q) {
            self.pool[i] += x[0];
            self.pool[i + 1] += x[1];
        }
    }

    /// Give a slot blocks until it holds `n`. False, with what it got, if
    /// the memory ran out.
    /// A loop kept for Undo gives its blocks up first, the oldest kept
    /// first. Inline, since a first pass asks on every frame and nearly
    /// always holds them already.
    #[inline]
    pub(crate) fn grow(&mut self, lp: usize, slot: usize, n: usize) -> bool {
        (self.held[lp][slot] as usize) >= n || self.grow_to(lp, slot, n)
    }

    #[inline(never)]
    fn grow_to(&mut self, lp: usize, slot: usize, n: usize) -> bool {
        // A slot sent away (phase 4d.4) taking blocks for a new layer:
        // the layer that was away is gone. One being brought back stays
        // away until it is all in.
        let back = self.back.is_some_and(|b| b.lp == lp && b.slot == slot);
        if self.held[lp][slot] == 0 && !back {
            self.away[lp] &= !(1 << slot);
        }
        let t = self.table(lp, slot);
        while (self.held[lp][slot] as usize) < n {
            if self.n_free == 0 && !self.drop_oldest_kept(lp) {
                return false;
            }
            self.n_free -= 1;
            let h = self.held[lp][slot] as usize;
            self.tables[t + h] = self.free[self.n_free];
            self.held[lp][slot] += 1;
        }
        true
    }

    /// Give back a slot's blocks after its first `n`.
    pub(crate) fn shrink(&mut self, lp: usize, slot: usize, n: usize) {
        let t = self.table(lp, slot);
        while self.held[lp][slot] as usize > n {
            self.held[lp][slot] -= 1;
            let h = self.held[lp][slot] as usize;
            self.free[self.n_free] = self.tables[t + h];
            self.tables[t + h] = NONE;
            self.n_free += 1;
        }
    }

    /// Give back every block of a slot. A slot being brought back
    /// (phase 4d.4) is called off: its step was let go, or its loop
    /// forgotten.
    pub(crate) fn release(&mut self, lp: usize, slot: usize) {
        if self.back.is_some_and(|b| b.lp == lp && b.slot == slot) {
            self.back = None;
        }
        let t = self.table(lp, slot);
        for h in 0..self.held[lp][slot] as usize {
            self.free[self.n_free] = self.tables[t + h];
            self.tables[t + h] = NONE;
            self.n_free += 1;
        }
        self.held[lp][slot] = 0;
    }

    pub(crate) fn blocks(&self, lp: usize) -> usize {
        self.held[lp].iter().map(|&h| h as usize).sum()
    }

    /// Keep loop `lp` as it is (cleared, faded out) for Undo, with the
    /// blocks its slots hold. Copies, never allocates.
    pub(crate) fn keep(&mut self, lp: usize, l: &Loop) {
        self.drop_kept(lp);
        self.kept[lp].clone_from(l);
        // A slot sent away (phase 4d.4) is the kept loop's too, so the
        // loop recorded next can't take it.
        let mut mask = self.away[lp];
        for k in 0..MAX_LAYERS {
            if self.held[lp][k] > 0 {
                mask |= 1 << k;
            }
        }
        self.kept_seq += 1;
        self.kept_mask[lp] = mask;
        self.kept_at[lp] = self.kept_seq;
        self.kept_txn[lp] = TransactionId::next();
    }

    /// Loop `lp` is kept for Undo.
    pub(crate) fn has_kept(&self, lp: usize) -> bool {
        self.kept_mask[lp] != 0
    }

    /// Slot `k` of loop `lp` holds blocks the loop kept for Undo needs.
    pub(crate) fn kept_holds(&self, lp: usize, k: usize) -> bool {
        self.kept_mask[lp] & (1 << k) != 0
    }

    /// Forget loop `lp`'s kept loop: its blocks go back, and its slots
    /// sent away are gone for good.
    pub(crate) fn drop_kept(&mut self, lp: usize) {
        let mask = std::mem::take(&mut self.kept_mask[lp]);
        for k in 0..MAX_LAYERS {
            if mask & (1 << k) != 0 {
                self.release(lp, k);
            }
        }
        self.away[lp] &= !mask;
    }

    /// Forget the oldest kept loop but `lp`'s. False if there is none.
    #[cold]
    fn drop_oldest_kept(&mut self, lp: usize) -> bool {
        let oldest = (0..MAX_LOOPS)
            .filter(|&n| n != lp && self.kept_mask[n] != 0)
            .min_by_key(|&n| self.kept_at[n]);
        match oldest {
            Some(n) => {
                self.drop_kept(n);
                true
            }
            None => false,
        }
    }

    /// Blocks loop `lp` keeps for Undo.
    pub(crate) fn kept_blocks_of(&self, lp: usize) -> usize {
        (0..MAX_LAYERS)
            .filter(|&k| self.kept_holds(lp, k))
            .map(|k| self.held[lp][k] as usize)
            .sum()
    }

    /// Blocks free, or free once the kept loops give theirs up.
    pub(crate) fn available(&self) -> usize {
        self.n_free
            + (0..MAX_LOOPS)
                .map(|lp| self.kept_blocks_of(lp))
                .sum::<usize>()
    }
}

/// Blocks for `frames` frames.
pub(crate) fn blocks_for(frames: usize) -> usize {
    frames.div_ceil(BLOCK)
}

/// The frames of a layer's `size` up to its last one written, `valid`
/// being (first written, count): all of them once it went round.
pub(crate) fn written_to(size: usize, (start, count): (usize, usize)) -> usize {
    if count >= size || start + count > size {
        size
    } else {
        start + count
    }
}

/// The blocks [`Looper::new`] makes for `seconds` at `rate`: the seconds,
/// one more per layer, and never more than a block number can name
/// (65535 blocks, about 25 minutes at 44.1 kHz).
pub(crate) fn n_blocks(rate: u32, seconds: u32) -> usize {
    let frames = (rate as usize) * (seconds as usize);
    (blocks_for(frames) + MAX_LAYERS).min(NONE as usize)
}

/// The memory [`Looper::new`] sets aside for `seconds` at `rate`, in
/// bytes: the sound itself and its tables of blocks, and the last
/// [`HEARD_SECONDS`] of input that Capture takes from. For showing what a
/// size costs before it is made.
pub fn bytes_for(rate: u32, seconds: u32) -> usize {
    let per_block = 2 * BLOCK * std::mem::size_of::<f32>()
        + (MAX_LOOPS * MAX_LAYERS + 1) * std::mem::size_of::<u16>();
    n_blocks(rate, seconds) * per_block
        + 2 * heard_frames(rate, seconds) * std::mem::size_of::<f32>()
}

/// The frames of input kept for Capture: [`HEARD_SECONDS`], with any
/// memory for loops at all.
pub(crate) fn heard_frames(rate: u32, seconds: u32) -> usize {
    if seconds > 0 {
        rate as usize * HEARD_SECONDS as usize
    } else {
        0
    }
}

/// Playback frame `p` of a layer through its view: the audio frame the
/// view plays there, crossfaded into the next piece over `seam` frames
/// before each cut. Only the audio frames the layer has written count.
#[inline]
pub(crate) fn layer_frame(
    store: &Store,
    blocks: &[u16],
    valid: (usize, usize),
    v: &View,
    seam: usize,
    p: usize,
) -> [f32; 2] {
    if v.size() == 0 {
        return [0.0; 2];
    }
    let (i, off) = v.locate(p);
    piece_frame(store, blocks, valid, v, seam, i, off)
}

/// [`layer_frame`] at frame `off` of piece `i` (where `View::locate`
/// puts it), for a view of some audio.
#[inline(always)]
pub(crate) fn piece_frame(
    store: &Store,
    blocks: &[u16],
    valid: (usize, usize),
    v: &View,
    seam: usize,
    i: usize,
    off: usize,
) -> [f32; 2] {
    let size = v.size();
    let (start, count) = valid;
    let fetch = |a: usize| {
        // `(a + size - start) % size`: frames from the first one written.
        let d = a + size - start;
        let d = if d < size { d } else { d - size };
        if count >= size || d < count {
            store.frame(blocks, a / BLOCK, a % BLOCK)
        } else {
            [0.0; 2]
        }
    };
    let pieces = v.pieces();
    let piece = &pieces[i];
    let x = fetch(v.frame(piece, off as i64));
    let f = seam.min(piece.len as usize);
    // Frames to the piece's end, 1 on its last.
    let left = piece.len as usize - off;
    if f > 0 && left <= f && v.cut_after(i) {
        // Into what comes before the next piece, which flows on into it.
        let w = ((f - left) as f32 + 0.5) / f as f32;
        let next = &pieces[(i + 1) % pieces.len()];
        let y = fetch(v.frame(next, -(left as i64)));
        return [x[0] * (1.0 - w) + y[0] * w, x[1] * (1.0 - w) + y[1] * w];
    }
    x
}
