//! MIDI and CV/gate for MPFree, host first: bytes and volts, no ports.
//!
//! - [`port`]: the external ports (the DIN jacks, USB devices on the
//!   USB-A socket), with ids that survive unplugging.
//! - [`msg`]: channel messages as values.
//! - [`input`]: keyboards into tracks (the selected track, or each track's
//!   own port and channel), the clock source into the time engine.
//! - [`output`]: MIDI tracks out to a port and channel, the period's
//!   bytes, and DIN pacing (31250 bit/s) with how late each message and
//!   clock pulse leaves.
//! - [`learn`]: MIDI Learn onto any `mpfree_param::ParamAddr`.
//! - [`cv`]: the eight CV/gate outputs as voltages.
//!
//! MIDI clock bytes are `mpfree-midi-clock`'s; how the transport follows
//! another device's clock is the time engine's. Which ALSA device each
//! port is, and opening it, is the app's, after the owner's go: the DIN
//! jacks' USB-MIDI cable and the way the CV jacks are driven are still to
//! be confirmed by a read-only capture (docs/dev/design/midi-io.md).
#![forbid(unsafe_code)]

pub mod cv;
pub mod input;
pub mod learn;
pub mod msg;
pub mod output;
pub mod port;

pub use msg::{Channel, Msg};
pub use port::{PortId, DIN, MAX_PORTS};
