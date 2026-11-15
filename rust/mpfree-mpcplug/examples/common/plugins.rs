// The MPFree plugins built for the MPC: one Spec each. Included by the
// plugin examples (each exports one) and by make_folders (which writes
// their folders), so the names and ids are written once.

#[allow(unused_imports)]
use mpfree_mpcplug::Spec;
#[allow(unused_imports)]
use mpfree_node::Node;

#[allow(dead_code)]
fn make_delay() -> Box<dyn Node> {
    mpfree_fx::registry::make("mpfree.fx.delay").expect("delay is in the registry")
}

#[allow(dead_code)]
fn make_squelch() -> Box<dyn Node> {
    Box::new(mpfree_synth::Squelch::new(44_100))
}

#[allow(dead_code)]
pub const DELAY: Spec = Spec {
    name: "Delay",
    vendor: "MPFree",
    uid: *b"MFdl",
    version: 1,
    instrument: false,
    make: make_delay,
};

#[allow(dead_code)]
pub const SQUELCH: Spec = Spec {
    name: "Squelch",
    vendor: "MPFree",
    uid: *b"MFsq",
    version: 1,
    instrument: true,
    make: make_squelch,
};

/// Every plugin, with its library's crate name (lib<crate>.so) and category.
#[allow(dead_code)]
pub const ALL: &[(&Spec, &str, &str)] = &[
    (&DELAY, "mpfree_delay", "Delay"),
    (&SQUELCH, "mpfree_squelch", "Synth"),
];
