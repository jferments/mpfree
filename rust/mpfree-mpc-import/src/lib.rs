//! MPC import, stage 1: MPC programs (`.xpm`) made into MPFree kits.
//! Keygroup programs ([`kit::keygroup_kit`]) and drum programs
//! ([`drum::drum_kit`]). Whole projects are not imported: patterns come
//! across as MIDI files exported from the MPC
//! ([MPC import](../../docs/dev/design/mpc-import.md)).
//!
//! It only ever reads MPC files and writes a new MPFree project: the
//! `mpfree-mpc-import` tool turns a program folder (the `.xpm` with its
//! samples beside it, as the MPC saves one) into a project in the
//! library, with a report of what came across exactly, what was
//! approximated and what was left out.
//!
//! - [`xml`]: a small, bounded XML reader.
//! - [`xpm`]: a program's keygroups or pads and their layers, as written.
//! - [`kit`]: a keygroup program as a kit of keygroup pads.
//! - [`drum`]: a drum program as a kit of pads.
//! - [`import`]: a program folder made into a project in the library.

pub mod drum;
pub mod import;
pub mod kit;
pub mod xml;
pub mod xpm;
