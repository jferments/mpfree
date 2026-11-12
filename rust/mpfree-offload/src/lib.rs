//! Render partitions on another computer (the performance plan's offload,
//! step 2: desktop only).
//!
//! A partition of the audio graph (a send effect first, a sequenced
//! instrument later) runs on a computer on the network. The MPC sends each
//! block's events and audio as one UDP datagram numbered by its block, the
//! computer renders it with the same Rust node the MPC would run, and the
//! answer is played **L periods later** (2 by default: 5.8 ms at 128 frames
//! and 44.1 kHz). So:
//!
//! - **The MPC stays the only clock.** The server never plays anything; it
//!   answers numbered blocks. There is no drift between two sound cards and
//!   no sample-rate conversion.
//! - **A late or lost answer is a miss**, the same contract as the plugin
//!   sandbox and the lanes: that block is silent and the partition carries
//!   on. Nothing waits.
//! - **The audio thread never touches a socket.** [`client::Remote`] hands
//!   requests to a ring and takes answers from a table, lock-free and
//!   without allocating; the [`client::Link`]'s own threads do the network.
//!
//! [`wire`] is the datagram, [`server`] the other computer's side and
//! [`client`] the MPC's. `mpfree-offload measure` runs both on one machine
//! over loopback and reports the round trip's tail, the misses and what a
//! partition costs the audio thread.
//!
//! Not here yet: pairing and a control channel (the server takes its
//! partitions from its command line and only listens on loopback unless
//! told otherwise), a local fallback with a fade when the link drops, many
//! partitions on one socket, and anything on the MPC.

pub mod client;
pub mod server;
pub mod wire;

use mpfree_fx::{ids, Cloud};
use mpfree_node::{Ctx, Kind, Node, NodeInfo, ParamInfo, Timed};

/// A partition's node, by name: `echo` (sends its input back: the network
/// alone), `cloud-full` or `cloud-eco` (Cloud reverb, Plate, in that
/// quality).
pub fn node_by_name(name: &str) -> Option<Box<dyn Node>> {
    match name {
        "echo" => Some(Box::new(Echo)),
        "cloud-full" | "cloud-eco" => {
            let mut c = Cloud::new();
            let q = if name == "cloud-full" { 0.0 } else { 2.0 };
            c.set_param(ids::cloud::QUALITY, q);
            Some(Box::new(c))
        }
        _ => None,
    }
}

/// The names [`node_by_name`] knows.
pub const NODE_NAMES: &[&str] = &["echo", "cloud-full", "cloud-eco"];

/// An effect that changes nothing: what comes back is what went out, so a
/// link's round trip is the network's and the copies' alone.
pub struct Echo;

static ECHO_INFO: NodeInfo = NodeInfo {
    id: "mpfree.offload.echo",
    name: "Echo (no change)",
    kind: Kind::Effect,
};

impl Node for Echo {
    fn info(&self) -> &'static NodeInfo {
        &ECHO_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        0.0
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], _left: &mut [f32], _right: &mut [f32]) {}
}
