//! Drum kits and samples for the pad player.
//!
//! - [`drums`]: MPFree's own kits, rendered from the Drums synth (the USB
//!   creator puts them on the stick's kits partition).
//! - [`freekits`]: free drum kits from public-domain (CC0) sample
//!   libraries, made from a pinned list by the `mpfree-free-kits` tool.
//! - [`import`]: the owner's Akai factory kits and samples, read from their
//!   own eMMC backup by the first-start wizard, on the MPC, for that MPC.
//! - [`kitfile`]: the kits file the pad player loads; [`wav`]: WAV
//!   conversion; [`xpm`] and [`xml`]: Akai's drum programs.

#![forbid(unsafe_code)]

pub mod drums;
pub mod freekits;
pub mod import;
pub mod kitfile;
pub mod wav;
pub mod xml;
pub mod xpm;
