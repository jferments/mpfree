//! The time engine's clock: the transport that the sequencer, looper, arp
//! and sampler all follow.
//!
//! - [`time`]: ticks (960 per quarter note), the tempo map, the meter,
//!   bar.beat.tick positions and swing. Plain values.
//! - [`transport`]: the [`Transport`] owns the audio thread's frame counter.
//!   Each period it fills a [`Block`] of [`Span`]s (one state and one tempo
//!   each) and [`Marker`]s (start, stop, loop jump, record on ...).
//!
//! Host-only for now; nothing here touches a device. The design is the
//! time engine plan (transport section); the roadmap step is T1.
#![forbid(unsafe_code)]

pub mod time;
pub mod transport;

pub use time::{Meter, Position, Segment, Swing, TempoMap, Tick, PPQ};
pub use transport::{Block, Command, Mark, Marker, Span, State, Stats, Transport};
