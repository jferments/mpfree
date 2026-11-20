//! The Synth tab for the MPC One, built on `mpfree-ui`.
//!
//! - [`view`]: the panel. It adds its widgets and page to the app's
//!   [`mpfree_ui::Ui`], shows a [`view::SynthState`] taken from any
//!   [`mpfree_node::Node`], and turns touches, Q-Links and the wheel into
//!   [`view::SynthAction`]s for the app to apply to the node.
//! - [`pages`]: which parameters go on which page. Native engines name
//!   their pages (Poly: Osc, Filter, Env, LFO, Voice); any other node, such
//!   as a hosted plugin or an effect, gets pages from its parameter groups.
//! - [`qsets`]: the Q-Link sets a sound carries (Poly: Tone, Env, Filter
//!   env, Motion, Mix, Osc), saved with its preset.
//! - [`sections`]: how a page groups its controls: sections by job
//!   (sources, tone, shape, motion, voice), each in its job's colour, its
//!   controls sized by how much they matter. [`section_plans`] holds the
//!   native engines' plans; any other page gets a section per group.
//! - [`section_view`]: the widget that draws a page's sections (knobs,
//!   faders, drawbars, button rows, steppers, pills, routing rows, and
//!   the engine's drawings inside their sections).
//! - [`widgets`]: the title bar with page tabs, the drawn cards, and the
//!   tile grid and Q-Link strip the effect pages still use.
//! - [`visual`]: Poly's drawn visuals (oscillators, filter response,
//!   envelopes, LFO), worked out from its values.
//! - [`fm_visual`]: FM's: the algorithm diagram, operator envelopes,
//!   keyboard scaling, the pitch envelope and the LFO.
//! - [`kit_visual`]: Drums' (the kit, a hit's parts, its sweep, the pads'
//!   levels) and Pluck's (the pick or the mallet, the harmonics or modes,
//!   their decays).
//! - [`wt_visual`]: the Wavetable synth's: each table as a 3D stack of
//!   frames with the one its Position reads drawn bright and warped.
//! - [`inst_visual`]: the instrument engines': Keys (the pickup, the
//!   hammer, decays, tremolo), Drawbar (the drawbars, the wave, the
//!   percussion, the rotor), Squelch (the wave, the filter, the sweep) and
//!   Analog (the oscillators, the filter, envelopes, LFO, the voice cards).
//! - [`format`]: value text (260 Hz, 2.5 kHz, +3.2 oct).
//! - [`draw`]: anti-aliased lines and arcs through `Canvas::blend_mask`.
//! - [`sample`]: sample screens for the workstation preview and the tests.
//!
//! No device I/O: the app draws the screens and feeds them touches.

#![forbid(unsafe_code)]

pub mod draw;
pub mod fm_visual;
pub mod format;
pub mod inst_visual;
pub mod kit_visual;
pub mod pages;
pub mod qsets;
pub mod sample;
pub mod section_plans;
pub mod section_view;
pub mod sections;
pub mod view;
pub mod visual;
pub mod widgets;
pub mod wt_visual;

pub use view::{SynthAction, SynthPanel, SynthState};
