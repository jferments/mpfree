//! The device under test: anything that turns a stereo signal into
//! another, with parameters. MPFree's own effects plug in as [`NodeDut`];
//! a captured reference effect is measured from its recordings instead
//! (see [`crate::programme`]).

use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, ParamInfo, Timed};

use crate::wav::Stereo;

/// A parameter change at a frame of the input being processed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub frame: usize,
    pub id: u16,
    pub value: f64,
}

pub trait Dut {
    fn name(&self) -> String;
    fn rate(&self) -> u32;
    /// Frames of delay the device reports (look-ahead), 0 if it reports none.
    fn latency(&self) -> usize;
    /// Silences every state, keeps the parameters.
    fn reset(&mut self);
    fn params(&self) -> Vec<ParamInfo>;
    fn set(&mut self, id: u16, value: f64);
    /// Processes `input`, continuing from where the last call stopped, with
    /// `changes` (sorted by frame) applied on their frames.
    fn process(&mut self, input: &Stereo, changes: &[Change]) -> Stereo;

    /// `input` followed by `tail` frames of silence, from a reset.
    fn render(&mut self, input: &Stereo, tail: usize) -> Stereo {
        self.reset();
        let mut padded = input.clone();
        padded.left.resize(input.len() + tail, 0.0);
        padded.right.resize(input.len() + tail, 0.0);
        self.process(&padded, &[])
    }
}

/// One of MPFree's nodes, run in 128-frame blocks (the MPC's period) at a
/// steady 120 BPM with the transport playing, so synced effects run.
pub struct NodeDut {
    node: Box<dyn Node>,
    rate: u32,
    frame: u64,
    block: usize,
}

impl NodeDut {
    pub fn new(mut node: Box<dyn Node>, rate: u32) -> NodeDut {
        node.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
        NodeDut {
            node,
            rate,
            frame: 0,
            block: 128,
        }
    }

    /// A registry effect by node id, at its defaults.
    pub fn from_registry(id: &str, rate: u32) -> Option<NodeDut> {
        mpfree_fx::registry::make(id).map(|n| NodeDut::new(n, rate))
    }

    pub fn node(&self) -> &dyn Node {
        self.node.as_ref()
    }
}

impl Dut for NodeDut {
    fn name(&self) -> String {
        self.node.info().id.to_string()
    }

    fn rate(&self) -> u32 {
        self.rate
    }

    fn latency(&self) -> usize {
        self.node.latency() as usize
    }

    fn reset(&mut self) {
        self.node.reset();
        self.frame = 0;
    }

    fn params(&self) -> Vec<ParamInfo> {
        self.node.params().to_vec()
    }

    fn set(&mut self, id: u16, value: f64) {
        self.node.set_param(id, value as f32);
    }

    fn process(&mut self, input: &Stereo, changes: &[Change]) -> Stereo {
        let n = input.len();
        let mut left: Vec<f32> = input.left.iter().map(|x| *x as f32).collect();
        let mut right: Vec<f32> = input.right.iter().map(|x| *x as f32).collect();
        let mut next = 0;
        let mut events: Vec<Timed> = Vec::new();
        let mut at = 0;
        while at < n {
            let len = self.block.min(n - at);
            events.clear();
            while next < changes.len() && changes[next].frame < at + len {
                let c = changes[next];
                let offset = c.frame.saturating_sub(at) as u32;
                events.push(Timed::new(
                    offset,
                    NodeEvent::Param {
                        id: c.id,
                        value: c.value as f32,
                    },
                ));
                next += 1;
            }
            let beats = self.frame as f64 / self.rate as f64 * 2.0;
            let ctx = Ctx {
                rate: self.rate,
                frame: self.frame,
                bpm: 120.0,
                beats,
                playing: true,
                sidechain: None,
            };
            self.node.process(
                &ctx,
                &events,
                &mut left[at..at + len],
                &mut right[at..at + len],
            );
            self.frame += len as u64;
            at += len;
        }
        Stereo::new(
            left.into_iter().map(f64::from).collect(),
            right.into_iter().map(f64::from).collect(),
            self.rate,
        )
    }
}

/// A wire: output equals input. The reference every measurement is
/// checked against.
pub struct Wire {
    pub rate: u32,
}

impl Dut for Wire {
    fn name(&self) -> String {
        "wire".into()
    }
    fn rate(&self) -> u32 {
        self.rate
    }
    fn latency(&self) -> usize {
        0
    }
    fn reset(&mut self) {}
    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }
    fn set(&mut self, _: u16, _: f64) {}
    fn process(&mut self, input: &Stereo, _: &[Change]) -> Stereo {
        input.clone()
    }
}

/// A memoryless curve on each sample (for testing the measurements).
pub struct Shaper<F: FnMut(f64) -> f64> {
    pub rate: u32,
    pub curve: F,
}

impl<F: FnMut(f64) -> f64> Dut for Shaper<F> {
    fn name(&self) -> String {
        "shaper".into()
    }
    fn rate(&self) -> u32 {
        self.rate
    }
    fn latency(&self) -> usize {
        0
    }
    fn reset(&mut self) {}
    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }
    fn set(&mut self, _: u16, _: f64) {}
    fn process(&mut self, input: &Stereo, _: &[Change]) -> Stereo {
        let l = input.left.iter().map(|x| (self.curve)(*x)).collect();
        let r = input.right.iter().map(|x| (self.curve)(*x)).collect();
        Stereo::new(l, r, input.rate)
    }
}
