//! The shared stream layer (docs/dev/design/disk-streaming.md): every long
//! sound played from disk, through a RAM cache the sound thread owns.
//!
//! Built a piece at a time (phase P1 of the plan):
//!
//! - [`format`]: where a WAV or AIFF file's audio is and how it is stored,
//!   so it is read in its own format and turned into float as it arrives.
//! - [`msg`]: what goes between the sound thread and the coordinator, and
//!   who owns a chunk.
//! - [`player`]: the sound thread's side: the cache it owns and reads.
//! - [`cursor`]: one voice's place in a source: seeks, speed, loops.
//! - `testkit` (feature `testkit`): marked sounds and the frame checker,
//!   for this crate's tests and engines built on the layer.
//! - [`coord`]: the coordinator: sources and their files, read planning
//!   by deadline, and the I/O thread that only reads.
//! - [`rec`] and [`recover`]: recording takes to files that survive a
//!   power cut, and finishing them after one (the recording file
//!   protocol), through the small file layer [`fsio`]; checksums in
//!   [`crc32c`].
//! - [`writer`]: the takes' writes and checkpoint rounds on a thread of
//!   their own, waiting for playback's reads, spread across the interval.
//! - [`policy`]: what a sound is apart from what it is doing (class, access,
//!   the musician's rule, device), and what follows: memory or disk, how
//!   much of its start stays in memory, and why admission says a workload
//!   isn't covered (the later phases, P3).
//! - [`ledger`]: the memory streaming owns or manages, for the app's shared
//!   memory check ([`coord::Coordinator::ledger`]).
//! - [`resample`]: files at another rate played at the engine's,
//!   resampled ahead into engine-rate units
//!   ([`coord::Coordinator::open_at_engine_rate`]).

#![forbid(unsafe_code)]

pub mod coord;
pub mod crc32c;
pub mod cursor;
pub mod format;
pub mod fsio;
pub mod ledger;
pub mod msg;
pub mod player;
pub mod policy;
pub mod rec;
pub mod recover;
pub mod resample;
#[cfg(feature = "testkit")]
pub mod testkit;
pub mod writer;
