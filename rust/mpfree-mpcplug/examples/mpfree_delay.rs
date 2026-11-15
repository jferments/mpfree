//! MPFree's Delay as an MPC plugin: `cargo build -p mpfree-mpcplug
//! --example mpfree_delay` makes `libmpfree_delay.so`. Its Spec is in
//! `common/plugins.rs`; `tools/mpc-plugins/make-folders.sh` builds the
//! folder the MPC loads.

#[path = "common/plugins.rs"]
mod plugins;

mpfree_mpcplug::export_mpc_plugin!(plugins::DELAY);
