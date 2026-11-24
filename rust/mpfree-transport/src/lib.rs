//! The time engine's clock: the transport that the sequencer, looper, arp
//! and sampler all follow.
//!
//! - [`time`]: ticks (960 per quarter note), the tempo map, the meter,
//!   bar.beat.tick positions and swing. Plain values.
//! - [`transport`]: the [`Transport`] owns the audio thread's frame counter.
//!   Each period it fills a [`Block`] of [`Span`]s (one state and one tempo
//!   each) and [`Marker`]s (start, stop, loop jump, record on ...).
//! - [`TransactionId`]: what ties one change's parts in every engine
//!   together for UNDO (the audio tracks rethink, phase 4b).
//!
//! Host-only for now; nothing here touches a device. The design is the
//! time engine plan (transport section); the roadmap step is T1.
#![forbid(unsafe_code)]

pub mod time;
pub mod transport;

pub use time::{Meter, Position, Segment, Swing, TempoMap, Tick, PPQ};
pub use transport::{Block, Command, Mark, Marker, Span, State, Stats, Transport};

use std::sync::atomic::{AtomicU64, Ordering};

/// One change to the project, in every engine it touched (the audio
/// tracks rethink, phase 4b: `docs/dev/design/loops-on-audio-tracks.md`,
/// "Transactions: one UNDO"). The time engine's control half, which owns
/// UNDO, takes one ([`TransactionId::next`]) for each step it keeps
/// (`mpfree_play::Take::txn`) before anything changes, and an engine that
/// keeps its own part of the step names it by this id. Given from 1, only
/// ever up, never twice in a run (one count for every project the run
/// opens, so a step of an earlier project never answers to a new one's).
/// Not saved: the history isn't. 0 is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId(pub u64);

/// The last [`TransactionId`] given.
static LAST_TRANSACTION: AtomicU64 = AtomicU64::new(0);

impl TransactionId {
    /// No transaction.
    pub const NONE: TransactionId = TransactionId(0);

    /// A new id, after every one given before in this run. Does not
    /// allocate or lock.
    pub fn next() -> TransactionId {
        TransactionId(LAST_TRANSACTION.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// Whether this names a transaction (not [`TransactionId::NONE`]).
    pub fn is_some(self) -> bool {
        self.0 != 0
    }
}

impl std::fmt::Display for TransactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::TransactionId;

    #[test]
    fn transaction_ids_only_go_up_and_are_never_none() {
        let a = TransactionId::next();
        let b = TransactionId::next();
        assert!(a.is_some() && b > a);
        let all: Vec<TransactionId> = (0..4)
            .map(|_| {
                std::thread::spawn(|| (0..1000).map(|_| TransactionId::next()).collect::<Vec<_>>())
            })
            .flat_map(|h| h.join().unwrap())
            .collect();
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "never given twice");
        assert!(!TransactionId::NONE.is_some());
    }
}
