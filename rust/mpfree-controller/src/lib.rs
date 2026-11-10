//! MPC One internal controller protocol, as observed on firmware 3.6.0
//! (`docs/hardware/MPC_ONE_3_6_0_CONTROLLER_PROTOCOL_20261109.md`).
//!
//! Two pure layers, no system calls and no heap allocation:
//! - [`midi`]: a MIDI 1.0 byte-stream parser (running status, real-time
//!   bytes, SysEx into a fixed buffer). This is what an ALSA RawMIDI read of
//!   the "Private" substream delivers.
//! - [`mpc_one`]: maps parsed messages to [`mpc_one::ControlEvent`]s and
//!   builds the output messages (button LEDs, pad RGB, keep-alive ping).
//!
//! Opening RawMIDI devices is deliberately out of scope for this crate.
#![forbid(unsafe_code)]
#![no_std]

pub mod midi;
pub mod mpc_one;
