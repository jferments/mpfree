//! A library stays started while any plugin made from it lives, and its
//! entry's init and deinit balance. Alone in its own test binary, so no
//! other test changes the count meanwhile.

use mpfree_clap::testplug::{self, SYNTH_ID};
use mpfree_clap::Library;
use mpfree_node::Node;
use std::sync::atomic::Ordering;

#[test]
fn init_and_deinit_balance() {
    assert_eq!(testplug::LIVE_INITS.load(Ordering::SeqCst), 0);
    let l = Library::from_entry(&testplug::ENTRY, "linked").expect("entry");
    assert_eq!(testplug::LIVE_INITS.load(Ordering::SeqCst), 1);
    let mut n = l.create(SYNTH_ID).expect("synth");
    n.prepare(44_100, 128);
    let c = n.control();
    drop(l);
    assert_eq!(testplug::LIVE_INITS.load(Ordering::SeqCst), 1, "the plugin keeps its file started");
    drop(n);
    assert_eq!(testplug::LIVE_INITS.load(Ordering::SeqCst), 1, "so does its control handle");
    drop(c);
    assert_eq!(testplug::LIVE_INITS.load(Ordering::SeqCst), 0);
}
