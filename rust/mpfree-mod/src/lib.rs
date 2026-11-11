//! Macros, a mod matrix, A/B morph, parameter-lock resolving and Q-Link
//! bindings for MPFree.
//!
//! Every parameter is named with an `mpfree_param::ParamAddr`. A
//! [`ModEngine`] looks after the parameters something moves (its
//! *targets*), works out each one's value from its layers, and hands the
//! host a sample-timed `NodeEvent::Param` for each value that changed.
//! The host sends it on: an instrument's to the track host, an insert's
//! to `Chain::queue`, a strip's to `Mixer::queue`. Nothing about the
//! nodes changes: to a node, modulation is automation.
//!
//! # Layers
//!
//! A target's value is worked out as a knob position (0 to 1, through
//! its `ParamInfo`'s curve, so a sweep on a cutoff moves evenly in
//! pitch), in this order:
//!
//! 1. **Base:** the value the user set ([`ModEvent::Set`]).
//! 2. **Morph:** once scenes A and B both hold the parameter, the morph
//!    position glides it between them, and a turn of its knob edits the
//!    scene the morph is nearer. Until then the base stands.
//! 3. **Macro:** a macro mapped to it puts it at `from + (to - from) ×
//!    curve(macro)`. One macro per parameter.
//! 4. **Lock:** a parameter lock replaces all of the above until it is
//!    unlocked.
//! 5. **Touch:** a hand on the control ([`ModEvent::Touch`]) replaces all
//!    of the above until it lets go ([`ModEvent::Untouch`]); locks keep
//!    landing underneath, so it lets go to what they hold by then.
//! 6. **Matrix:** every row aimed at it adds `source × depth (× via)`,
//!    and the sum is clamped to the knob's travel.
//!
//! Macros and the morph position are targets too (`Owner::Macro`,
//! `Owner::Morph`): they can be locked on a step, swept by an LFO, and
//! (macros) stored in the scenes.
//!
//! # Timing and cost
//!
//! The engine runs at control rate: every [`DEFAULT_TICK`] (32) frames of
//! absolute time, not of the block, so it gives the same events however
//! the host cuts its blocks. Sets, locks and touches take effect on their
//! exact frame, before any tick on that frame. A target is only worked out when
//! something moved it, and only sent when its value changed. Nodes smooth
//! their own parameters, so a 32-frame step never zippers.
//!
//! [`ModEngine::process`] never allocates, locks or panics, and nor do
//! the changes ([`ModEngine::add_target`], [`ModEngine::map`],
//! [`ModEngine::add_row`] ...) while there is room, so the host can make
//! them on the audio thread from a queue. The cost is measured by the
//! "Mod: ..." cases of `mpfree-bench`.
//!
//! # What stays elsewhere
//!
//! LFOs and envelopes inside a synth or an effect, and per-voice
//! modulation (velocity, pressure, key tracking per note), stay inside
//! the node. This matrix is per track or per bus. Switches (toggles and
//! choices) can be moved by a macro, a lock, a touch or the morph but not
//! swept by the matrix. A synth's voice mode is never a target.

#![forbid(unsafe_code)]

pub mod bind;
pub mod engine;
pub mod follow;
pub mod lfo;
pub mod text;

pub use bind::{Bank, Binding, Bindings, QLinkCard, Scope, Write, KNOBS};
pub use engine::{
    source_info, MacroMap, MapCurve, ModEngine, ModError, ModEvent, ModStats, ModTimed, Row,
    Source, DEFAULT_TICK, FOLLOWERS, HOST_SOURCES, LFOS, MACROS, MACRO_TARGETS, MAX_ROWS,
    MAX_TARGETS,
};
pub use follow::Follower;
pub use lfo::{Lfo, Shape};
pub use text::value_text;

/// A macro's description, for a knob card.
pub fn macro_info(m: usize) -> mpfree_node::ParamInfo {
    engine::macro_info(m)
}
