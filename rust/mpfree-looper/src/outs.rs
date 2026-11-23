//! Each loop's sound on its own (audio tracks phase 4, step 4a).
//!
//! The looper writes what each loop plays in a period to that loop's own
//! buffer, and what Listen plays to one more, so that a host can give each
//! loop a channel of its own. [`LoopOuts::mix_into`] adds them up in the
//! order the looper always summed them, so their sum is the looper's mix
//! sample for sample, bit for bit: [`crate::Looper::process`] is
//! [`crate::Looper::process_each`] followed by it.
//!
//! Why the bits match: in a period a loop adds to each of its frames once
//! (its run adds one value a frame, frame by frame or in one go once it is
//! still), so its buffer, which starts at zero, holds exactly that value.
//! The mix used to add the loops' values into one zeroed run buffer in
//! loop order and then that buffer into the mix; [`LoopOuts::mix_into`]
//! adds the loops' buffers into a zeroed sum in the same order, then the
//! sum into the mix. A loop resampling hears the sum of the loops before
//! it, made the same way.
//!
//! The buffers are allocated once ([`LoopOuts::new`], from
//! [`crate::Looper::new`]); a period clears only the buffers that sounded
//! in the period before, so a host can skip a loop that didn't sound
//! ([`LoopOuts::sounded`]).

use crate::MAX_LOOPS;

/// One period of each loop's sound, kept apart: what
/// [`crate::Looper::process_each`] wrote. Every buffer is interleaved
/// stereo and [`LoopOuts::frames`] frames long.
#[derive(Default)]
pub struct LoopOuts {
    /// Each loop's frames, [`LoopOuts::capacity`] frames a loop, loop
    /// after loop.
    bufs: Box<[f32]>,
    /// What Listen played.
    listen: Box<[f32]>,
    cap: usize,
    /// Frames in the period.
    frames: usize,
    /// The loops that may have sounded in the period (bit k: loop k).
    /// Every other loop's buffer is all zero.
    sounded: u32,
    /// The loops that resampled in the period: they heard the sum of
    /// the others, and are added after them.
    resampling: u32,
    /// The loop Listen played in the period. Without one, the listen
    /// buffer is all zero.
    listen_of: Option<usize>,
}

impl LoopOuts {
    /// Room for periods of up to `cap` frames. Allocates.
    pub(crate) fn new(cap: usize) -> LoopOuts {
        LoopOuts {
            bufs: vec![0.0; MAX_LOOPS * 2 * cap].into_boxed_slice(),
            listen: vec![0.0; 2 * cap].into_boxed_slice(),
            cap,
            frames: 0,
            sounded: 0,
            resampling: 0,
            listen_of: None,
        }
    }

    /// The longest period the buffers hold, in frames.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Frames in the last period.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Loop `k`'s sound in the last period, as it went into the mix:
    /// after its level, mute, solo, launch and fades. All zero where it
    /// didn't sound; empty for a loop past [`MAX_LOOPS`].
    pub fn loop_out(&self, k: usize) -> &[f32] {
        if k >= MAX_LOOPS {
            return &[];
        }
        let at = k * 2 * self.cap;
        &self.bufs[at..at + 2 * self.frames]
    }

    /// Loop `k` may have sounded in the last period. When false its
    /// frames are all zero, and a host can skip them.
    pub fn sounded(&self, k: usize) -> bool {
        k < MAX_LOOPS && self.sounded & (1 << k) != 0
    }

    /// Loop `k` resampled in the last period: what it recorded, if it
    /// recorded, was what the loops not resampling played.
    pub fn resampled(&self, k: usize) -> bool {
        k < MAX_LOOPS && self.resampling & (1 << k) != 0
    }

    /// What Listen played in the last period: the loop it played and its
    /// frames.
    pub fn listen(&self) -> Option<(usize, &[f32])> {
        self.listen_of.map(|k| (k, &self.listen[..2 * self.frames]))
    }

    /// Add the last period to `out` (interleaved stereo, as many frames
    /// as both have) as the looper's mix: what Listen played first, then
    /// each frame's loops summed in the looper's order, the loops not
    /// resampling by number and then the ones resampling by number. The
    /// same bits as [`crate::Looper::process`] adds.
    pub fn mix_into(&self, out: &mut [f32]) {
        let n = 2 * self.frames.min(out.len() / 2);
        let out = &mut out[..n];
        if self.listen_of.is_some() {
            for (o, v) in out.iter_mut().zip(&self.listen[..n]) {
                *o += v;
            }
        }
        let mut order = [0usize; MAX_LOOPS];
        let mut m = 0;
        for last in [false, true] {
            for k in 0..MAX_LOOPS {
                if self.sounded(k) && self.resampled(k) == last {
                    order[m] = k;
                    m += 1;
                }
            }
        }
        let buf = |k: usize| &self.bufs[k * 2 * self.cap..][..n];
        // The run buffer started at zero and each loop added its value:
        // the first loop's buffer is that sum so far (0 + v is v, and is
        // never -0), so it is where the sum starts.
        match order[..m] {
            [] => {}
            [k] => {
                for (o, v) in out.iter_mut().zip(buf(k)) {
                    *o += v;
                }
            }
            [first, ref rest @ ..] => {
                // A chunk at a time: each sample's loops are added in
                // order, as the run buffer added them, and the sum then
                // goes into the mix.
                const CHUNK: usize = 256;
                let mut at = 0;
                while at < n {
                    let len = CHUNK.min(n - at);
                    let mut s = [0.0f32; CHUNK];
                    let s = &mut s[..len];
                    s.copy_from_slice(&buf(first)[at..at + len]);
                    for &k in rest {
                        for (s, v) in s.iter_mut().zip(&buf(k)[at..at + len]) {
                            *s += v;
                        }
                    }
                    for (o, s) in out[at..at + len].iter_mut().zip(&*s) {
                        *o += s;
                    }
                    at += len;
                }
            }
        }
    }

    /// A new period of `frames` frames (at most the capacity): the buffers
    /// that sounded in the last one go back to zero.
    pub(crate) fn begin(&mut self, frames: usize) {
        let was = 2 * self.frames;
        for k in 0..MAX_LOOPS {
            if self.sounded & (1 << k) != 0 {
                let at = k * 2 * self.cap;
                self.bufs[at..at + was].fill(0.0);
            }
        }
        if self.listen_of.take().is_some() {
            self.listen[..was].fill(0.0);
        }
        self.frames = frames.min(self.cap);
        self.sounded = 0;
        self.resampling = 0;
    }

    /// The loops resampling in this period.
    pub(crate) fn set_resampling(&mut self, mask: u32) {
        self.resampling = mask;
    }

    /// Loop `k`'s `frames` frames from frame `off`, for its run to add to
    /// (they start at zero). The loop may write them, so they are cleared
    /// next period; a loop that isn't run (empty, with nothing coming in)
    /// leaves its buffer as it is, all zero.
    pub(crate) fn run_mut(&mut self, k: usize, off: usize, frames: usize) -> &mut [[f32; 2]] {
        self.sounded |= 1 << k;
        let at = k * 2 * self.cap + 2 * off;
        self.bufs[at..at + 2 * frames].as_chunks_mut::<2>().0
    }

    /// Add the frames from `off` of the loops in `mask` that sounded, by
    /// number, to `to` (interleaved, as long as it is): what a loop
    /// resampling hears.
    pub(crate) fn sum_into(&self, mask: u32, off: usize, to: &mut [f32]) {
        for k in 0..MAX_LOOPS {
            if mask & self.sounded & (1 << k) != 0 {
                let b = &self.bufs[k * 2 * self.cap + 2 * off..][..to.len()];
                for (t, v) in to.iter_mut().zip(b) {
                    *t += v;
                }
            }
        }
    }

    /// The listen buffer for this period, all zero, for Listen to add to;
    /// `of` is the loop it plays.
    pub(crate) fn listen_mut(&mut self, of: usize) -> &mut [f32] {
        self.listen_of = Some(of);
        &mut self.listen[..2 * self.frames]
    }
}
