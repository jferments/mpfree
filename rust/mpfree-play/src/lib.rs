//! The time engine's playback engine (roadmap T2).
//!
//! - [`view`]: the control thread compiles the project's playing clips into
//!   a read-only [`PlayView`] (notes sorted, quantise and speed applied,
//!   swing resolved) and swaps it in.
//! - [`player`]: each period, the [`Player`] walks the transport's spans and
//!   plays every clip's notes on their exact frames into a [`Sink`], which
//!   the host maps to the sampler, the arp or MIDI out.
//!
//! The same session gives the same events for any period size, and the
//! audio-thread call never allocates; `tests/` proves both. Host-only for
//! now: nothing here touches a device.
#![forbid(unsafe_code)]

pub mod player;
pub mod view;

pub use player::{Event, Out, Player, Sink, Stats, ALL};
pub use view::{CompileError, Launch, PlayView};
