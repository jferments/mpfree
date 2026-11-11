//! Block timing statistics: fixed memory, no allocation, safe on the audio
//! thread.

use crate::Span;

/// Histogram resolution: sub-buckets per doubling (12.5 % wide buckets).
const SUB: u32 = 8;
const SUB_BITS: u32 = 3;
/// The first octave starts at 2^10 ns (about 1 µs); everything below is one
/// bucket.
const LOW_BITS: u32 = 10;
/// Octaves covered: 2^10 to 2^26 ns (about 67 ms). Above is one bucket.
const OCTAVES: u32 = 16;
/// Buckets in a histogram: below, the log range, above.
pub const BUCKETS: usize = (2 + OCTAVES * SUB) as usize;

#[inline]
fn bucket(ns: u64) -> usize {
    if ns < 1 << LOW_BITS { return 0; }
    let e = 63 - ns.leading_zeros();
    if e >= LOW_BITS + OCTAVES { return BUCKETS - 1; }
    let sub = ((ns >> (e - SUB_BITS)) & (SUB as u64 - 1)) as u32;
    (1 + (e - LOW_BITS) * SUB + sub) as usize
}

/// The largest value bucket `i` holds (a percentile reports this, so it
/// never understates).
fn bucket_top(i: usize) -> u64 {
    if i == 0 { return (1 << LOW_BITS) - 1; }
    if i >= BUCKETS - 1 { return u64::MAX; }
    let k = (i - 1) as u32;
    let e = LOW_BITS + k / SUB;
    let sub = (k % SUB) as u64;
    ((SUB as u64 + sub + 1) << (e - SUB_BITS)) - 1
}

/// Running totals for a stream of blocks.
#[derive(Clone, Debug)]
pub struct Stats {
    pub blocks: u64,
    pub frames: u64,
    pub total_ns: u64,
    pub max_ns: u64,
    /// Blocks that took longer than the probe's limit.
    pub over: u64,
    /// Allocator calls seen inside measured blocks (needs
    /// [`crate::alloc::CountingAlloc`]).
    pub allocs: u64,
    /// Blocks with at least one allocator call.
    pub alloc_blocks: u64,
    hist: [u32; BUCKETS],
}

impl Default for Stats {
    fn default() -> Stats {
        Stats { blocks: 0, frames: 0, total_ns: 0, max_ns: 0, over: 0, allocs: 0, alloc_blocks: 0, hist: [0; BUCKETS] }
    }
}

impl Stats {
    #[inline]
    pub fn record(&mut self, ns: u64, frames: u32, limit_ns: u64, allocs: u64) {
        self.blocks += 1;
        self.frames += u64::from(frames);
        self.total_ns = self.total_ns.saturating_add(ns);
        self.max_ns = self.max_ns.max(ns);
        if ns > limit_ns { self.over += 1; }
        if allocs > 0 {
            self.allocs += allocs;
            self.alloc_blocks += 1;
        }
        let b = &mut self.hist[bucket(ns)];
        *b = b.saturating_add(1);
    }

    /// Adds another stream's totals (for example several voices or slots).
    pub fn merge(&mut self, o: &Stats) {
        self.blocks += o.blocks;
        self.frames += o.frames;
        self.total_ns = self.total_ns.saturating_add(o.total_ns);
        self.max_ns = self.max_ns.max(o.max_ns);
        self.over += o.over;
        self.allocs += o.allocs;
        self.alloc_blocks += o.alloc_blocks;
        for (a, b) in self.hist.iter_mut().zip(o.hist.iter()) { *a = a.saturating_add(*b); }
    }

    /// The value at or below which `q` (0 to 1) of the blocks fall, rounded
    /// up to the bucket's top (at most 12.5 % high) and never above the
    /// worst block. 0 with no blocks.
    pub fn quantile_ns(&self, q: f64) -> u64 {
        let n: u64 = self.hist.iter().map(|&c| u64::from(c)).sum();
        if n == 0 { return 0; }
        let rank = ((q.clamp(0.0, 1.0) * n as f64).ceil() as u64).max(1);
        let mut seen = 0u64;
        for (i, &c) in self.hist.iter().enumerate() {
            seen += u64::from(c);
            if seen >= rank { return bucket_top(i).min(self.max_ns); }
        }
        self.max_ns
    }

    pub fn summary(&self) -> Summary {
        Summary {
            blocks: self.blocks,
            frames: self.frames,
            total_ns: self.total_ns,
            mean_ns: if self.blocks == 0 { 0.0 } else { self.total_ns as f64 / self.blocks as f64 },
            p50_ns: self.quantile_ns(0.5),
            p99_ns: self.quantile_ns(0.99),
            p999_ns: self.quantile_ns(0.999),
            max_ns: self.max_ns,
            over: self.over,
            allocs: self.allocs,
            alloc_blocks: self.alloc_blocks,
        }
    }
}

/// A copyable digest of [`Stats`], small enough to send from the audio
/// thread to the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    pub blocks: u64,
    pub frames: u64,
    pub total_ns: u64,
    pub mean_ns: f64,
    pub p50_ns: u64,
    pub p99_ns: u64,
    pub p999_ns: u64,
    pub max_ns: u64,
    pub over: u64,
    pub allocs: u64,
    pub alloc_blocks: u64,
}

impl Summary {
    /// Share of real time used, in per cent: time spent over the audio the
    /// blocks covered. 0 with no frames.
    pub fn realtime_pct(&self, rate: u32) -> f64 {
        if self.frames == 0 || rate == 0 { return 0.0; }
        let audio_ns = self.frames as f64 * 1e9 / f64::from(rate);
        100.0 * self.total_ns as f64 / audio_ns
    }

    /// One log line: mean, p99, worst in µs and as a share of `period_ns`.
    pub fn line(&self, name: &str, period_ns: f64) -> String {
        let us = |ns: f64| ns / 1000.0;
        let pct = |ns: f64| if period_ns > 0.0 { 100.0 * ns / period_ns } else { 0.0 };
        let mut s = format!(
            "{name}: mean {:.0} us ({:.1} %), p99 {:.0} us ({:.1} %), max {:.0} us ({:.1} %) over {} blocks, {} over the limit",
            us(self.mean_ns), pct(self.mean_ns), us(self.p99_ns as f64), pct(self.p99_ns as f64),
            us(self.max_ns as f64), pct(self.max_ns as f64), self.blocks, self.over);
        if self.allocs > 0 {
            s.push_str(&format!(", {} allocator calls in {} blocks", self.allocs, self.alloc_blocks));
        }
        s
    }
}

/// Timing for one stream of blocks: a lifetime total and a window the
/// screen thread takes and restarts.
#[derive(Clone, Debug)]
pub struct Probe {
    limit_ns: u64,
    life: Stats,
    window: Stats,
}

impl Probe {
    /// `limit_ns`: a block longer than this counts as over (usually the
    /// period, or a module's budget).
    pub fn new(limit_ns: u64) -> Probe { Probe { limit_ns, life: Stats::default(), window: Stats::default() } }

    pub fn limit_ns(&self) -> u64 { self.limit_ns }
    pub fn set_limit_ns(&mut self, ns: u64) { self.limit_ns = ns; }

    #[inline]
    pub fn record(&mut self, ns: u64, frames: u32, allocs: u64) {
        self.life.record(ns, frames, self.limit_ns, allocs);
        self.window.record(ns, frames, self.limit_ns, allocs);
    }

    /// Ends a span: records its time and allocator calls. Returns the time.
    #[inline]
    pub fn stop(&mut self, span: Span, frames: u32) -> u64 {
        let ns = span.elapsed_ns();
        self.record(ns, frames, span.allocs());
        ns
    }

    pub fn lifetime(&self) -> Summary { self.life.summary() }
    pub fn stats(&self) -> &Stats { &self.life }

    /// The window since the last take, and a fresh window.
    pub fn take_window(&mut self) -> Summary {
        let s = self.window.summary();
        self.window = Stats::default();
        s
    }

    /// Forgets everything (a new test run).
    pub fn clear(&mut self) {
        self.life = Stats::default();
        self.window = Stats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_cover_the_range_in_order() {
        let mut last = 0;
        for ns in [0u64, 1, 1023, 1024, 1100, 2047, 2048, 5_000, 100_000, 2_902_494, 60_000_000] {
            let b = bucket(ns);
            assert!(b >= last, "{ns}");
            assert!(bucket_top(b) >= ns, "{ns} in bucket {b} with top {}", bucket_top(b));
            if b > 0 && b < BUCKETS - 1 {
                assert!(bucket_top(b - 1) < ns, "{ns} would fit the bucket below");
                assert!((bucket_top(b) - ns) as f64 <= ns as f64 * 0.125 + 1.0, "{ns}: bucket wider than 12.5 %");
            }
            last = b;
        }
        assert_eq!(bucket(u64::MAX), BUCKETS - 1);
        assert_eq!(bucket(1 << 26), BUCKETS - 1);
        assert_eq!(bucket((1 << 26) - 1), BUCKETS - 2);
    }

    #[test]
    fn percentiles_mean_and_worst() {
        let mut s = Stats::default();
        // 990 blocks at 50 µs, 9 at 400 µs, one at 2 ms.
        for _ in 0..990 { s.record(50_000, 128, 2_902_494, 0); }
        for _ in 0..9 { s.record(400_000, 128, 2_902_494, 0); }
        s.record(2_000_000, 128, 1_000_000, 0);
        let m = s.summary();
        assert_eq!(m.blocks, 1000);
        assert_eq!(m.max_ns, 2_000_000);
        assert_eq!(m.over, 1);
        assert!((50_000..=56_250).contains(&m.p50_ns), "{}", m.p50_ns);
        assert!((50_000..=56_250).contains(&m.p99_ns), "{}", m.p99_ns);
        assert!((400_000..=450_000).contains(&m.p999_ns), "{}", m.p999_ns);
        assert_eq!(s.quantile_ns(1.0), 2_000_000);
        assert!((m.mean_ns - (990.0 * 50e3 + 9.0 * 400e3 + 2e6) / 1000.0).abs() < 1e-6);
        assert_eq!(Stats::default().summary().p99_ns, 0);
    }

    #[test]
    fn realtime_share_and_line() {
        let mut p = Probe::new(2_902_494);
        // 100 blocks of 128 frames at 44.1 kHz taking 290.2 µs each: 10 %.
        for _ in 0..100 { p.record(290_249, 128, 0); }
        p.record(10, 128, 3);
        let s = p.lifetime();
        let want = 100.0 * (100.0 * 290_249.0 + 10.0) / (101.0 * 128.0 * 1e9 / 44_100.0);
        assert!((s.realtime_pct(44_100) - want).abs() < 1e-9 && (want - 9.9).abs() < 0.05, "{want}");
        let line = s.line("synth", 2_902_494.0);
        assert!(line.starts_with("synth: mean 287 us (9.9 %), p99 ") && line.contains("max 290 us (10.0 %) over 101 blocks, 0 over the limit, 3 allocator calls in 1 blocks"), "{line}");
        assert_eq!(p.take_window().blocks, 101);
        assert_eq!(p.take_window().blocks, 0);
        assert_eq!(p.lifetime().blocks, 101);
    }

    #[test]
    fn merging_adds_streams() {
        let mut a = Stats::default();
        let mut b = Stats::default();
        a.record(1_000, 64, 10, 0);
        b.record(9_000, 64, 1_000_000, 2);
        a.merge(&b);
        let s = a.summary();
        assert_eq!((s.blocks, s.frames, s.max_ns, s.over, s.allocs, s.alloc_blocks), (2, 128, 9_000, 1, 2, 1));
    }
}
