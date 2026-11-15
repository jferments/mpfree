//! MPFree's Squelch acid bass as an MPC instrument plugin: `cargo build -p
//! mpfree-mpcplug --example mpfree_squelch` makes `libmpfree_squelch.so`.

use mpfree_mpcplug::{export_mpc_plugin, Spec};
use mpfree_node::Node;

fn make() -> Box<dyn Node> {
    Box::new(mpfree_synth::squelch::Squelch::new(44_100))
}

export_mpc_plugin!(Spec {
    name: "Squelch",
    vendor: "MPFree",
    uid: *b"MFsq",
    version: 1,
    instrument: true,
    make,
});
