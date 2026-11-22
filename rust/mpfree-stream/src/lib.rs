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
//! - [`coord`]: the coordinator: sources and their files, read planning
//!   by deadline, and the I/O thread that only reads.

#![forbid(unsafe_code)]

pub mod coord;
pub mod cursor;
pub mod format;
pub mod msg;
pub mod player;
