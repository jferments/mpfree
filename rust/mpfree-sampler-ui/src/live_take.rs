//! The take being recorded, drawn into the Sample page's wave as it comes
//! in (the one Sample page's step 2f, part 3: the Rec page's half-minute
//! input view goes, and the wave shows the take itself).
//!
//! [`LiveTake`] reads what the recording has copied since the last call
//! (stereo interleaved, as [`mpfree_sampler::record::Recording::data`]
//! gives it) into columns of a fixed number of frames, so a tick costs
//! only the frames that came in. A long take halves its columns' count
//! (doubling the frames each holds) rather than growing without end.
//! [`LiveTake::peaks`] hands the wave a copy of the columns as
//! [`Peaks`] spanning [`LiveTake::span`]: the take fills it from the left
//! and the rest stays empty until the head gets there, when the span
//! doubles.

use std::sync::Arc;

use mpfree_ui::wave::{Peaks, WaveColumn};

/// The most columns a take keeps; past it, pairs merge.
pub const MAX_COLUMNS: usize = 4096;
/// Frames a column holds to begin with (1.5 ms at 44.1 kHz).
pub const FIRST_PER: u64 = 64;

/// One column, both channels: min, max and the sum of squares.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Column {
    min: [f32; 2],
    max: [f32; 2],
    sq: [f32; 2],
    frames: u64,
}

impl Column {
    const EMPTY: Column = Column {
        min: [f32::MAX; 2],
        max: [f32::MIN; 2],
        sq: [0.0; 2],
        frames: 0,
    };

    fn add(&mut self, l: f32, r: f32) {
        for (ch, x) in [l, r].into_iter().enumerate() {
            self.min[ch] = self.min[ch].min(x);
            self.max[ch] = self.max[ch].max(x);
            self.sq[ch] += x * x;
        }
        self.frames += 1;
    }

    fn merge(&mut self, o: &Column) {
        for ch in 0..2 {
            self.min[ch] = self.min[ch].min(o.min[ch]);
            self.max[ch] = self.max[ch].max(o.max[ch]);
            self.sq[ch] += o.sq[ch];
        }
        self.frames += o.frames;
    }
}

/// The take so far, in columns.
#[derive(Clone, Debug)]
pub struct LiveTake {
    per: u64,
    cols: Vec<Column>,
    /// The column being filled.
    open: Column,
    /// Samples of the take's data read so far.
    read: usize,
}

impl Default for LiveTake {
    fn default() -> Self {
        LiveTake::new()
    }
}

impl LiveTake {
    pub fn new() -> LiveTake {
        LiveTake {
            per: FIRST_PER,
            cols: Vec::new(),
            open: Column::EMPTY,
            read: 0,
        }
    }

    /// Reads what `data` (the take's audio so far, stereo interleaved) has
    /// had since the last call. Data shorter than what was read is a new
    /// take: it starts again.
    pub fn update(&mut self, data: &[f32]) {
        if data.len() < self.read {
            *self = LiveTake::new();
        }
        let whole = data.len() - data.len() % 2;
        for f in data[self.read..whole].chunks_exact(2) {
            self.open.add(f[0], f[1]);
            if self.open.frames == self.per {
                self.cols.push(self.open);
                self.open = Column::EMPTY;
                if self.cols.len() >= MAX_COLUMNS {
                    self.halve();
                }
            }
        }
        self.read = whole;
    }

    /// Pairs of columns merge: each holds twice the frames.
    fn halve(&mut self) {
        let merged: Vec<Column> = self
            .cols
            .chunks(2)
            .map(|p| {
                let mut c = p[0];
                if let Some(o) = p.get(1) {
                    c.merge(o);
                }
                c
            })
            .collect();
        // Only full columns are kept and MAX_COLUMNS is even, so every
        // merged column is full too, and the open one has just started.
        self.cols = merged;
        self.per *= 2;
    }

    /// Frames read so far.
    pub fn frames(&self) -> u64 {
        (self.read / 2) as u64
    }

    /// Frames a column holds now.
    pub fn per(&self) -> u64 {
        self.per
    }

    /// Columns kept (the one being filled not counted).
    pub fn columns(&self) -> usize {
        self.cols.len()
    }

    /// The span the wave shows: `first` (the take's planned length, or a
    /// few bars), doubled until the take so far fits inside it.
    pub fn span(&self, first: u64) -> u64 {
        let mut s = first.max(1);
        while s < self.frames() {
            s = s.saturating_mul(2);
        }
        s
    }

    /// The take so far as peaks spanning `span` frames (the rest silent).
    pub fn peaks(&self, span: u64) -> Arc<dyn Peaks> {
        let mut cols = self.cols.clone();
        if self.open.frames > 0 {
            cols.push(self.open);
        }
        Arc::new(LivePeaks {
            per: self.per,
            cols: cols.into(),
            recorded: self.frames(),
            span: span.max(self.frames()).max(1),
        })
    }
}

/// [`LiveTake`]'s columns as the wave reads them.
struct LivePeaks {
    per: u64,
    cols: Arc<[Column]>,
    /// Frames recorded; the rest of the span is silent.
    recorded: u64,
    span: u64,
}

impl Peaks for LivePeaks {
    fn frames(&self) -> u64 {
        self.span
    }

    fn channels(&self) -> usize {
        2
    }

    fn column(&self, ch: usize, f0: f64, f1: f64) -> WaveColumn {
        let frames = self.recorded as f64;
        if ch >= 2 || f1 <= 0.0 || f0 >= frames || self.cols.is_empty() {
            return WaveColumn::default();
        }
        let per = self.per as f64;
        let a = (f0.max(0.0) / per).floor() as usize;
        let b = ((f1.min(frames) / per).ceil() as usize)
            .max(a + 1)
            .min(self.cols.len());
        if a >= b {
            return WaveColumn::default();
        }
        let mut c = Column::EMPTY;
        for col in &self.cols[a..b] {
            c.merge(col);
        }
        WaveColumn {
            min: c.min[ch],
            max: c.max[ch],
            rms: (c.sq[ch] / c.frames.max(1) as f32).sqrt(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` frames: left a ramp up to 1, right half of it negated.
    fn ramp(n: usize) -> Vec<f32> {
        (0..n)
            .flat_map(|i| {
                let x = i as f32 / n as f32;
                [x, -x / 2.0]
            })
            .collect()
    }

    #[test]
    fn it_reads_only_what_came_in_and_draws_it_from_the_left() {
        let data = ramp(1000);
        let mut t = LiveTake::new();
        t.update(&data[..2 * 300]);
        assert_eq!(t.frames(), 300);
        assert_eq!(t.columns(), 300 / FIRST_PER as usize);
        t.update(&data);
        assert_eq!(t.frames(), 1000);
        // A half frame waits for its other half.
        let mut odd = data.clone();
        odd.push(0.5);
        t.update(&odd);
        assert_eq!(t.frames(), 1000);

        let p = t.peaks(4000);
        assert_eq!((p.frames(), p.channels()), (4000, 2));
        let c = p.column(0, 0.0, 1000.0);
        assert!(c.min == 0.0 && (c.max - 0.999).abs() < 1e-3, "{c:?}");
        let r = p.column(1, 0.0, 1000.0);
        assert!((r.min + 0.4995).abs() < 1e-3 && r.max == 0.0, "{r:?}");
        // Past the head, nothing yet.
        assert_eq!(p.column(0, 1000.0, 2000.0), WaveColumn::default());
        // A new take (shorter data) starts again.
        t.update(&data[..20]);
        assert_eq!(t.frames(), 10);
    }

    #[test]
    fn a_long_take_keeps_a_bounded_number_of_columns_and_its_span_doubles() {
        let mut t = LiveTake::new();
        let mut data = Vec::new();
        for _ in 0..60 {
            data.resize(data.len() + 2 * 44_100, 0.25f32);
            t.update(&data);
        }
        assert_eq!(t.frames(), 60 * 44_100);
        assert!(t.columns() < MAX_COLUMNS, "{}", t.columns());
        assert!(t.per() > FIRST_PER);
        // Every frame is in a column or the open one, counted once.
        let p = t.peaks(1);
        let c = p.column(0, 0.0, t.frames() as f64);
        assert!((c.rms - 0.25).abs() < 1e-3, "{c:?}");
        let bars4 = 4 * 88_200;
        assert_eq!(t.span(bars4), 8 * bars4, "60 s in 8 s spans: 64 s");
        assert_eq!(LiveTake::new().span(bars4), bars4);
    }
}
