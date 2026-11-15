//! MPFree's Delay as an MPC plugin: `cargo build -p mpfree-mpcplug
//! --example mpfree_delay` makes `libmpfree_delay.so`. A plugin folder for
//! the MPC holds it beside a `plugin.json` whose name, manufacturer and uid
//! match the [`Spec`] below (see the inMusic SDK's plugin folder format).

use mpfree_mpcplug::{export_mpc_plugin, Spec};
use mpfree_node::Node;

fn make() -> Box<dyn Node> {
    mpfree_fx::registry::make("mpfree.fx.delay").expect("delay is in the registry")
}

export_mpc_plugin!(Spec {
    name: "Delay",
    vendor: "MPFree",
    uid: *b"MFdl",
    version: 1,
    instrument: false,
    make,
});
