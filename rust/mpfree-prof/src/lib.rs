//! Real-time profiling for MPFree.
//!
//! MPFree is built on a fast desktop and runs on the MPC One: four 32-bit
//! Cortex-A17 cores at 1.608 GHz, with the audio thread due every 128 frames
//! at 44.1 kHz (2.902 ms). This crate is how every engine says what it costs
//! there, in the same units, so the numbers can be added up against one
//! budget:
//!
//! - [`Probe`] records how long each audio block took: count, mean,
//!   percentiles (p50, p99, p99.9 from a fixed log histogram), worst, and how
//!   many blocks went over a limit. Recording never allocates, locks or
//!   panics, so it is safe on the audio thread. A probe keeps a lifetime
//!   total and a window the screen thread takes every few hundred ms.
//! - [`Profiled`] wraps any [`mpfree_node::Node`] and times its `process`,
//!   and counts allocations inside it when the program installs
//!   `alloc::CountingAlloc` (the `counting-alloc` feature).
//! - [`Profiler`] is a fixed set of named probes for a host's audio loop
//!   (sampler, synth, FX, looper, the whole period), built before the stream
//!   starts.
//! - [`budget`] holds the MPC One's period and the per-module budget table
//!   every engine is measured against.
//! - [`model`] holds the reference kernels that calibrate desktop numbers to
//!   the MPC: run them on both machines, and the ratio turns a desktop
//!   measurement into an estimate for the device.
//!
//! The clock is [`std::time::Instant`] (`clock_gettime(CLOCK_MONOTONIC)`
//! through the vDSO on Linux). It measures wall time, so a block that was
//! preempted counts the time it lost: that is what an underrun cares about.
//! The cost of reading the clock is measured by `mpfree-bench --calibrate`.

#![deny(unsafe_code)]

pub mod alloc;
pub mod budget;
pub mod model;
pub mod procs;
pub mod report;
mod node;
mod shared;
mod stats;

pub use node::Profiled;
pub use shared::SharedSummary;
pub use stats::{Probe, Stats, Summary, BUCKETS};

use std::time::Instant;

/// A started measurement. `Span::start()` before the work, then
/// [`Probe::stop`] after it.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    at: Instant,
    allocs: u64,
}

impl Span {
    #[inline]
    pub fn start() -> Span { Span { allocs: alloc::thread_count(), at: Instant::now() } }

    /// Nanoseconds since the start.
    #[inline]
    pub fn elapsed_ns(&self) -> u64 { self.at.elapsed().as_nanos().min(u64::MAX as u128) as u64 }

    /// Allocator calls on this thread since the start (0 when the counting
    /// allocator is not installed).
    #[inline]
    pub fn allocs(&self) -> u64 { alloc::thread_count().wrapping_sub(self.allocs) }
}

/// A fixed set of named probes, made before the audio stream starts. The
/// audio thread records into them by index; the screen thread takes their
/// windows.
#[derive(Clone, Debug)]
pub struct Profiler {
    names: Vec<&'static str>,
    probes: Vec<Probe>,
}

impl Profiler {
    /// One probe per name, each with the same over-limit threshold (usually
    /// the period: see [`budget::Period::ns`]).
    pub fn new(names: &[&'static str], limit_ns: u64) -> Profiler {
        Profiler { names: names.to_vec(), probes: names.iter().map(|_| Probe::new(limit_ns)).collect() }
    }

    /// The index of a name, looked up once at setup.
    pub fn index(&self, name: &str) -> Option<usize> { self.names.iter().position(|n| *n == name) }

    pub fn names(&self) -> &[&'static str] { &self.names }

    /// Records a finished span on probe `i`. Out-of-range indices are
    /// ignored (never a panic on the audio thread).
    #[inline]
    pub fn stop(&mut self, i: usize, span: Span, frames: u32) {
        if let Some(p) = self.probes.get_mut(i) { p.stop(span, frames); }
    }

    pub fn probe(&self, i: usize) -> Option<&Probe> { self.probes.get(i) }
    pub fn probe_mut(&mut self, i: usize) -> Option<&mut Probe> { self.probes.get_mut(i) }

    /// Every probe's window as `(name, summary)`, and the windows restart.
    /// Fills `out` (cleared first) so a host can reuse one buffer.
    pub fn take_windows(&mut self, out: &mut Vec<(&'static str, Summary)>) {
        out.clear();
        for (n, p) in self.names.iter().zip(self.probes.iter_mut()) {
            out.push((n, p.take_window()));
        }
    }

    /// Every probe's lifetime summary.
    pub fn lifetime(&self) -> Vec<(&'static str, Summary)> {
        self.names.iter().zip(&self.probes).map(|(n, p)| (*n, p.lifetime())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profiler_keeps_its_probes_apart() {
        let mut p = Profiler::new(&["synth", "sampler"], 2_902_494);
        let s = p.index("sampler").unwrap();
        assert_eq!(p.index("nope"), None);
        p.probe_mut(s).unwrap().record(500_000, 128, 0);
        p.probe_mut(0).unwrap().record(100_000, 128, 0);
        p.probe_mut(0).unwrap().record(3_000_000, 128, 0);
        p.stop(99, Span::start(), 128);
        let mut w = Vec::new();
        p.take_windows(&mut w);
        assert_eq!(w[0].0, "synth");
        assert_eq!(w[0].1.blocks, 2);
        assert_eq!(w[0].1.over, 1);
        assert_eq!(w[1].1.max_ns, 500_000);
        p.take_windows(&mut w);
        assert_eq!(w[0].1.blocks, 0, "windows restart");
        assert_eq!(p.lifetime()[0].1.blocks, 2, "the lifetime keeps everything");
    }
}
