//! The streaming ledger (disk streaming's P3, step 2): the memory streaming
//! owns or manages, counted the way the shared memory check needs it.
//!
//! Every allocation has one owner who charges it, however many engines read
//! it. Streaming charges its cache, its buffers and its whole-file reads. A
//! sound another engine holds and lends to streaming (`open_memory`) is
//! charged to that engine: the ledger shows it, apart, and never counts it.
//! Capture memory belongs to the recording writer, which reports it beside
//! this ([`crate::writer::Writer::memory`]).

/// One pool's memory, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    /// Allocated: what the pool takes from the system, used or not.
    pub allocated: u64,
    /// Holding something useful now.
    pub in_use: u64,
    /// Promised to work under way (recordings' reserve, holds).
    pub reserved: u64,
    /// What could really be given back to the system now. Emptying cache
    /// units gives nothing back: only remaking the cache does, so this is 0
    /// while the cache exists.
    pub reclaimable: u64,
}

/// What streaming holds, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    /// The sound thread's cache, made whole at start: empty units are room
    /// for streaming, not free memory for another engine.
    pub cache: Pool,
    /// The buffers fills travel in, and the read buffers.
    pub buffers: Pool,
    /// Whole-file reads under way or waiting to be taken.
    pub jobs: u64,
    /// Sounds lent by other engines (`open_memory`): read here, charged to
    /// their owner. Shown, not counted in [`Ledger::charged`].
    pub lent: u64,
}

impl Ledger {
    /// The memory streaming charges against the app's: what it has
    /// allocated, used or not.
    pub fn charged(&self) -> u64 {
        self.cache
            .allocated
            .saturating_add(self.buffers.allocated)
            .saturating_add(self.jobs)
    }
}
