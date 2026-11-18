//! The recorder's input in columns: the [`Strip`] the Sample page's meter
//! reads from the sampler's [`Input`] (only the frames that came since the
//! last frame, so it costs next to nothing), and the recording colour.
//! (The Rec tab's half-minute picture of it went with the tab in the one
//! Sample page's step 2f, part 4; the take now draws into the wave,
//! [`crate::live_take`].)

use std::collections::VecDeque;

use mpfree_sampler::record::Input;

/// The recording colour (the take being recorded, the record button's dot).
pub const REC_COLOR: u32 = 0xFF4D5E;

/// The input level the meters and the strip draw from (dBFS) up to 0.
pub const FLOOR_DB: f32 = -60.0;

/// Linear peak to dBFS (−∞ for silence).
pub fn db(x: f32) -> f32 {
    if x > 0.0 {
        20.0 * x.log10()
    } else {
        f32::NEG_INFINITY
    }
}

// ---------------------------------------------------------------- strip

/// The input's peaks in columns of a fixed number of frames, aligned on
/// the input's absolute frames, the latest `len` of them.
pub struct Strip {
    per: u64,
    len: usize,
    /// (column index, [left min, left max, right min, right max]).
    cols: VecDeque<(u64, [f32; 4])>,
    read_to: u64,
    scratch: Vec<f32>,
    peak: [f32; 2],
}

impl Strip {
    /// `len` columns of `per` frames each.
    pub fn new(per: u64, len: usize) -> Strip {
        Strip {
            per: per.max(1),
            len: len.max(1),
            cols: VecDeque::with_capacity(len.max(1)),
            read_to: 0,
            scratch: Vec::with_capacity(2 * per.max(1) as usize),
            peak: [0.0; 2],
        }
    }

    /// Frames per column.
    pub fn per(&self) -> u64 {
        self.per
    }

    /// Frames the strip spans.
    pub fn span(&self) -> u64 {
        self.per * self.len as u64
    }

    /// Forgets everything: the next update reads a new input from its start.
    pub fn reset(&mut self) {
        self.cols.clear();
        self.read_to = 0;
        self.peak = [0.0; 2];
    }

    /// Reads what the input has had since the last update (at first, as
    /// much of the strip's span as the input holds), a column at a time.
    pub fn update(&mut self, input: &Input) {
        let now = input.now();
        if now < self.read_to {
            // Another input, started since.
            self.reset();
        }
        let from = self
            .read_to
            .max(now.saturating_sub(self.span()))
            .max(input.audio.oldest());
        let mut at = from;
        while at < now {
            // To the end of this column, or now.
            let end = ((at / self.per + 1) * self.per).min(now);
            self.scratch.clear();
            if input.audio.copy(at, end, &mut self.scratch).is_err() {
                // Overwritten while we read (we fell more than the buffer
                // behind): start again from what it holds now.
                at = input.audio.oldest().max(end);
                continue;
            }
            let mut c = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
            for f in self.scratch.chunks_exact(2) {
                c[0] = c[0].min(f[0]);
                c[1] = c[1].max(f[0]);
                c[2] = c[2].min(f[1]);
                c[3] = c[3].max(f[1]);
                self.peak[0] = self.peak[0].max(f[0].abs());
                self.peak[1] = self.peak[1].max(f[1].abs());
            }
            let index = at / self.per;
            match self.cols.back_mut() {
                Some((i, last)) if *i == index => {
                    last[0] = last[0].min(c[0]);
                    last[1] = last[1].max(c[1]);
                    last[2] = last[2].min(c[2]);
                    last[3] = last[3].max(c[3]);
                }
                _ => {
                    if self.cols.len() == self.len {
                        self.cols.pop_front();
                    }
                    self.cols.push_back((index, c));
                }
            }
            at = end;
        }
        self.read_to = now.max(self.read_to);
    }

    /// The loudest sample on each side since the last call (linear).
    pub fn take_peak(&mut self) -> [f32; 2] {
        std::mem::take(&mut self.peak)
    }

    /// Column `index` (absolute frame / [`Strip::per`]), if kept.
    pub fn column(&self, index: u64) -> Option<[f32; 4]> {
        let first = self.cols.front()?.0;
        let (i, c) = self.cols.get(index.checked_sub(first)? as usize)?;
        (*i == index).then_some(*c)
    }

    /// The frame read up to.
    pub fn read_to(&self) -> u64 {
        self.read_to
    }
}
