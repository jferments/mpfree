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
//! - [`arps`]: one arp per track, and the per-track "Clips through the
//!   arp" switch: a [`Sink`] adapter between the player and the host.
//! - [`record`]: the clip recorder: notes played while the transport
//!   records go into the armed track's clip at their exact ticks, knob
//!   moves as parameter locks and Perform FX gestures as moves.
//! - [`takes`]: take undo on the control thread: what each take recorded
//!   and erased, to undo and redo it as a whole.
//! - [`recorded`]: recorded changes numbered, made to the view in place
//!   and to the project alike, and kept to catch up a view compiled a
//!   little late, for a host that compiles on another thread.
//!
//! The same session gives the same events for any period size, and the
//! audio-thread call never allocates; `tests/` proves both. Host-only for
//! now: nothing here touches a device.
#![forbid(unsafe_code)]

pub mod arp;
pub mod arps;
pub mod player;
pub mod record;
pub mod recorded;
pub mod takes;
pub mod view;

pub use arp::ArpHost;
pub use arps::{Through, TrackArps};
pub use mpfree_param;
pub use player::{Erased, ErasedLock, Event, Out, Player, Sink, Stats, ALL, SONG_TRACK};
pub use record::{
    Recorder, Taken, TakenLock, TakenMove, MAX_TOUCH_INPUT, MOVE_POINTS, TOUCH_BUFS, TOUCH_POINTS,
    TOUCH_ROOM,
};
pub use recorded::{Behind, Includes, Rec, Recent, RECENT};
pub use takes::{MovePut, MoveWrite, Recorded, Take, Takes, TAKE_ROOM};
pub use view::{anchored, CompileError, Cue, Launch, NowPlaying, PlayView, Started, EARLIEST};
