//! MPC-format plugins in MPFree.
//!
//! MPC OS loads third-party instruments and effects that are Linux shared
//! libraries speaking the VST 2.4 binary interface (one exported symbol,
//! `VSTPluginMain`), each in a plugin folder with its skins. The plugins in
//! the Open MPC catalog (<https://open-mpc.github.io>) are built that way.
//! This crate loads them as [`mpfree_node::Node`]s, so MPFree's plugin
//! sandbox (`mpfree-plughost`) runs them the way it runs CLAP plugins: each
//! in a process of its own.
//!
//! - [`folder`] finds plugins and reads their folders (`plugin.json`,
//!   `plugin-meta.xml`, the catalog's `mpc-plugin.json`).
//! - [`host`] loads the library, makes an instance in the MPC host's call
//!   order and runs it ([`MpcNode`], [`MpcControl`]).
//!
//! The plugin's code runs in the process that loads it, with no sandbox of
//! its own: load these only in the plugin process, never in the app.
//! MPFree never bundles these plugins; people install the ones they want.

pub mod folder;
pub mod host;
pub mod json;

pub use folder::{find, is_mpc_plugin, Folder};
pub use host::{plugin_id, Library, MpcControl, MpcNode, MpcStats};
