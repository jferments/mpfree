//! MPFree's Squelch acid bass as an MPC instrument plugin: `cargo build -p
//! mpfree-mpcplug --example mpfree_squelch` makes `libmpfree_squelch.so`.

#[path = "common/plugins.rs"]
mod plugins;

mpfree_mpcplug::export_mpc_plugin!(plugins::SQUELCH);
