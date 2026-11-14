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
//! - [`setup`]: a project's MIDI, sync and CV settings (what the mixer's
//!   I/O view edits), saved as the project's `midi` part.
//! - [`cv_wire`]: the bytes Akai's app sends to the controller to set the
//!   CV jacks (channel 16 CC pairs on cable 1), as captured.
//! - [`smf`]: Standard MIDI Files for the export: the notes a pass played
//!   ([`smf::NoteLog`], fixed room) and a type 1 file of them.
//! - [`monitor`]: the MIDI monitor's model: every message in and out of
//!   the ports as values (a lock-free feed where the bytes are, the list
//!   and the channel lights the screen shows).
//!
//! MIDI clock bytes are `mpfree-midi-clock`'s; how the transport follows
//! another device's clock is the time engine's. Which ALSA device each
//! port is, and opening it, is the app's, after the owner's go. The owner's
//! read-only captures showed both DIN jacks on USB-MIDI cable 2 (In and
//! Out) and the CV jacks driven by CCs on cable 1
//! (docs/dev/design/midi-io.md).
#![forbid(unsafe_code)]

pub mod cv;
pub mod cv_wire;
pub mod input;
pub mod learn;
pub mod monitor;
pub mod msg;
pub mod output;
pub mod port;
pub mod setup;
pub mod smf;

pub use msg::{Channel, Msg};
pub use port::{PortId, DIN, MAX_PORTS};
