//! MPFree startup screen: a spinning spiral galaxy with "MPCFree OS v0.1" in
//! block letters at its centre, drawn with ratatui.
//!
//! The same scene runs in a desktop terminal (crossterm) and on the MPC One's
//! framebuffer through [`fb::FramebufferBackend`], a ratatui backend that
//! draws cells as pixels. [`png`] writes preview frames without a device.
#![forbid(unsafe_code)]

pub mod fb;
pub mod galaxy;
pub mod png;
pub mod title;
