//! MPC import, stage 1: MPC programs (`.xpm`) made into MPFree kits.
//! Keygroup programs come first ([`kit::keygroup_kit`]); drum programs
//! and whole projects follow ([MPC import](../../docs/dev/design/mpc-import.md)).
//!
//! It only ever reads MPC files and writes a new MPFree project: the
//! `mpfree-mpc-import` tool turns a program folder (the `.xpm` with its
//! samples beside it, as the MPC saves one) into a project in the
//! library, with a report of what came across exactly, what was
//! approximated and what was left out.
//!
//! - [`xml`]: a small, bounded XML reader.
//! - [`xpm`]: a keygroup program's keygroups and layers, as written.
//! - [`kit`]: the program as a kit of keygroup pads.
//! - [`import`]: a program folder made into a project in the library.

pub mod import;
pub mod kit;
pub mod xml;
pub mod xpm;
