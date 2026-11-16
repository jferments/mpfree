//! Installs MPC plugins from the Open MPC catalog
//! (<https://open-mpc.github.io>) into MPFree's plugin folder. MPFree never
//! ships these plugins: the musician picks them, and this crate checks and
//! unpacks the release zips they download.
//!
//! - [`catalog`]: reads `catalog.json`, picks each plugin's 32-bit ARM
//!   release and says whether MPFree on an MPC One can run it.
//! - [`zip`]: reads a zip held in memory (stored and deflated entries).
//! - [`sha256`]: checks a zip against the catalog's sum.
//! - [`install`]: finds the plugin folders in a zip and unpacks them safely.
//! - [`fetch`]: downloads from the catalog's sites with the system's libcurl.

pub mod catalog;
pub mod fetch;
pub mod install;
pub mod sha256;
pub mod zip;
