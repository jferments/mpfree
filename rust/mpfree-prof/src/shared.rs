//! Handing summaries from the audio thread to the screen without a lock.

use std::sync::atomic::{fence, AtomicU64, Ordering};

use crate::Summary;

const FIELDS: usize = 11;

/// One [`Summary`] the audio thread publishes and any thread reads, with no
/// lock and no allocation (a sequence lock over atomics). The writer never
/// waits. A reader that catches a write in progress tries again, a few
/// times, then reports nothing for this look.
///
/// Share it with an `Arc` made at setup. 64-bit atomics are native on
/// ARMv7 (`ldrexd`/`strexd`) and x86-64.
#[derive(Debug, Default)]
pub struct SharedSummary {
    seq: AtomicU64,
    f: [AtomicU64; FIELDS],
}

fn pack(s: &Summary) -> [u64; FIELDS] {
    [
        s.blocks,
        s.frames,
        s.total_ns,
        s.mean_ns.to_bits(),
        s.p50_ns,
        s.p99_ns,
        s.p999_ns,
        s.max_ns,
        s.over,
        s.allocs,
        s.alloc_blocks,
    ]
}

fn unpack(v: [u64; FIELDS]) -> Summary {
    Summary {
        blocks: v[0],
        frames: v[1],
        total_ns: v[2],
        mean_ns: f64::from_bits(v[3]),
        p50_ns: v[4],
        p99_ns: v[5],
        p999_ns: v[6],
        max_ns: v[7],
        over: v[8],
        allocs: v[9],
        alloc_blocks: v[10],
    }
}

impl SharedSummary {
    pub fn new() -> SharedSummary {
        SharedSummary::default()
    }

    /// Publishes a summary. One writer only (the audio thread).
    #[inline]
    pub fn publish(&self, s: &Summary) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        for (a, v) in self.f.iter().zip(pack(s)) {
            a.store(v, Ordering::Relaxed);
        }
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    /// The latest summary and how many have been published (0: none yet,
    /// so the summary is all zeros). `None` if a write kept getting in the
    /// way; look again next frame.
    pub fn read(&self) -> Option<(Summary, u64)> {
        for _ in 0..8 {
            let a = self.seq.load(Ordering::Acquire);
            if a & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let mut v = [0u64; FIELDS];
            for (x, f) in v.iter_mut().zip(&self.f) {
                *x = f.load(Ordering::Relaxed);
            }
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == a {
                return Some((unpack(v), a / 2));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn publish_and_read() {
        let s = SharedSummary::new();
        assert_eq!(s.read().unwrap().1, 0);
        let sum = Summary {
            blocks: 3,
            frames: 384,
            total_ns: 9,
            mean_ns: 3.0,
            p50_ns: 2,
            p99_ns: 4,
            p999_ns: 4,
            max_ns: 4,
            over: 1,
            allocs: 0,
            alloc_blocks: 0,
        };
        s.publish(&sum);
        assert_eq!(s.read(), Some((sum, 1)));
    }

    #[test]
    fn a_reader_never_sees_a_torn_summary() {
        let s = Arc::new(SharedSummary::new());
        let w = Arc::clone(&s);
        let writer = std::thread::spawn(move || {
            for i in 1..=200_000u64 {
                // Every field equal, so a torn read shows as a mix.
                let sum = Summary {
                    blocks: i,
                    frames: i,
                    total_ns: i,
                    mean_ns: i as f64,
                    p50_ns: i,
                    p99_ns: i,
                    p999_ns: i,
                    max_ns: i,
                    over: i,
                    allocs: i,
                    alloc_blocks: i,
                };
                w.publish(&sum);
            }
        });
        let mut seen = 0u64;
        while !writer.is_finished() {
            if let Some((r, n)) = s.read() {
                let v = pack(&r);
                assert!(
                    v.iter().enumerate().all(|(k, x)| if k == 3 {
                        f64::from_bits(*x) == r.blocks as f64
                    } else {
                        *x == r.blocks
                    }),
                    "torn: {r:?}"
                );
                assert!(r.blocks >= seen && n == r.blocks);
                seen = r.blocks;
            }
        }
        writer.join().unwrap();
        assert_eq!(s.read().unwrap().0.blocks, 200_000);
    }
}
