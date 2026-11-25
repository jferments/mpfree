//! How long a drive's reads and syncs take, kept as counts in fixed
//! buckets so nothing is allocated after it is made, and summed up as the
//! SD card plan's B2a asks: the median, the slowest 1 in 100 and 1 in
//! 1,000, and the worst, for Dev and the device rounds' logs.

use std::fmt;

/// Buckets per doubling: each is about 19% wider than the one before.
const PER_DOUBLING: usize = 4;
/// The first bucket's top, in seconds: 100 microseconds.
const FIRST: f64 = 1e-4;
/// From 100 microseconds up to about 100 s; longer goes in the last.
const BUCKETS: usize = 20 * PER_DOUBLING + 1;

/// Timings, counted.
#[derive(Clone, Debug)]
pub struct Times {
    counts: [u32; BUCKETS],
    count: u64,
    worst: f64,
}

impl Default for Times {
    fn default() -> Times {
        Times {
            counts: [0; BUCKETS],
            count: 0,
            worst: 0.0,
        }
    }
}

/// The top of bucket `b`, in seconds.
fn top(b: usize) -> f64 {
    FIRST * 2f64.powf(b as f64 / PER_DOUBLING as f64)
}

impl Times {
    /// One more, `s` seconds long.
    pub fn add(&mut self, s: f64) {
        let s = if s.is_finite() { s.max(0.0) } else { 0.0 };
        let b = if s <= FIRST {
            0
        } else {
            ((s / FIRST).log2() * PER_DOUBLING as f64).ceil() as usize
        };
        let b = b.min(BUCKETS - 1);
        self.counts[b] = self.counts[b].saturating_add(1);
        self.count += 1;
        self.worst = self.worst.max(s);
    }

    /// How many there have been.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The time `q` of them (0 to 1) took at most: the top of the bucket
    /// it falls in, never more than the worst. 0 with none yet.
    pub fn quantile(&self, q: f64) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let want = ((q.clamp(0.0, 1.0) * self.count as f64).ceil() as u64).max(1);
        let mut seen = 0u64;
        for (b, &c) in self.counts.iter().enumerate() {
            seen += u64::from(c);
            if seen >= want {
                return top(b).min(self.worst);
            }
        }
        self.worst
    }

    pub fn summary(&self) -> Summary {
        Summary {
            count: self.count,
            median: self.quantile(0.5),
            p99: self.quantile(0.99),
            p999: self.quantile(0.999),
            worst: self.worst,
        }
    }
}

/// [`Times`] summed up, in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    pub count: u64,
    pub median: f64,
    /// The slowest 1 in 100 took at least this; 1 in 1,000, `p999`.
    pub p99: f64,
    pub p999: f64,
    pub worst: f64,
}

/// "1.2 ms", "40 ms", "1.20 s".
fn secs(s: f64) -> String {
    if s < 0.01 {
        format!("{:.1} ms", s * 1e3)
    } else if s < 1.0 {
        format!("{:.0} ms", s * 1e3)
    } else {
        format!("{s:.2} s")
    }
}

/// "median 2.0 ms, 1 in 100 40 ms, 1 in 1,000 300 ms, worst 1.20 s, of
/// 1234"; "none yet" with none.
impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.count == 0 {
            return write!(f, "none yet");
        }
        write!(
            f,
            "median {}, 1 in 100 {}, 1 in 1,000 {}, worst {}, of {}",
            secs(self.median),
            secs(self.p99),
            secs(self.p999),
            secs(self.worst),
            self.count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_median_and_the_slowest_fall_in_their_buckets() {
        let mut t = Times::default();
        assert_eq!(t.summary(), Summary::default());
        assert_eq!(t.summary().to_string(), "none yet");
        // 980 reads of 2 ms, 15 of 50 ms, 4 of 300 ms and one of 1.5 s.
        for _ in 0..980 {
            t.add(0.002);
        }
        for _ in 0..15 {
            t.add(0.05);
        }
        for _ in 0..4 {
            t.add(0.3);
        }
        t.add(1.5);
        let s = t.summary();
        assert_eq!(s.count, 1000);
        let near = |got: f64, want: f64| got >= want && got < want * 1.2;
        assert!(near(s.median, 0.002), "{s:?}");
        assert!(near(s.p99, 0.05), "{s:?}");
        assert!(near(s.p999, 0.3), "{s:?}");
        assert_eq!(s.worst, 1.5);
        assert_eq!(
            s.to_string(),
            format!(
                "median {:.1} ms, 1 in 100 {:.0} ms, 1 in 1,000 {:.0} ms, worst 1.50 s, of 1000",
                s.median * 1e3,
                s.p99 * 1e3,
                s.p999 * 1e3
            )
        );
    }

    #[test]
    fn odd_times_land_at_the_ends() {
        let mut t = Times::default();
        t.add(0.0);
        t.add(-1.0);
        t.add(f64::NAN);
        t.add(1e6);
        assert_eq!(t.count(), 4);
        assert!(t.quantile(0.5) <= 1e-4);
        assert_eq!(t.summary().worst, 1e6);
        assert!(t.quantile(1.0) <= 1e6);
    }
}
