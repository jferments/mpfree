//! The time engine's playback engine (roadmap T2).
//!
//! - [`view`]: the control thread compiles the project's playing clips into
//!   a read-only [`PlayView`] (notes sorted, quantise and speed applied,
//!   swing resolved) and swaps it in. A launch made while the music plays
//!   is a [`Cue`]: it waits for the next bar line.
//! - [`player`]: each period, the [`Player`] walks the transport's spans and
//!   plays every clip's notes on their exact frames into a [`Sink`], which
//!   the host maps to the sampler, the arp or MIDI out.
//! - [`arp`]: a track through the arpeggiator: the player's notes hold and
//!   release notes on an `mpfree-arp` engine that follows the transport.
//! - [`record`]: the clip recorder: notes played while the transport
//!   records go into the armed track's clip at their exact ticks.
//! - [`takes`]: take undo on the control thread: what each take recorded
//!   and erased, to undo and redo it as a whole.
//!
//! The same session gives the same events for any period size, and the
//! audio-thread call never allocates; `tests/` proves both. Host-only for
//! now: nothing here touches a device.
#![forbid(unsafe_code)]

pub mod arp;
pub mod player;
pub mod record;
pub mod takes;
pub mod view;

pub use arp::ArpHost;
pub use mpfree_param;
pub use player::{Erased, ErasedLock, Event, Out, Player, Sink, Stats, ALL};
pub use record::{Recorder, Taken, TakenLock};
pub use takes::{Take, Takes, TAKE_ROOM};
pub use view::{anchored, CompileError, Cue, Launch, PlayView, Started, EARLIEST};
